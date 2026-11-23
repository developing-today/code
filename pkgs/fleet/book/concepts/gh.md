# The gh passthrough

`fleet gh` runs the real `gh` CLI (GitHub's command-line tool) under a
rotating fleet token. Every argument passes to the real `gh` binary
unchanged (`FLEET_GH_BIN` selects the binary, default `gh`) — except the
four _shadow subcommands_ (`tiers`, `cache-serve`, `cache-status`,
`closures`), which fleet implements itself. The domain help's status line
still claims the implementation "lands in a later step" of the multicall
epic; that wording is stale — the source is a complete implementation, and
this chapter documents the source.

Source: https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet/src/gh.rs

## The passthrough contract

Before each attempt, one token is resolved by name through the fleet
secrets chain and exported as `GH_TOKEN` **for that attempt only**.
Attempts are _captured_ (piped), not streamed, because the failure text is
what classifies a failover; the caller sees the final attempt's stdout,
stderr, and exit code **byte-exact** as `gh` produced them. Attempts are
silent by default; `FLEET_GH_TRACE=1` names each attempt's token _source_
(never a value) on stderr. Tokens are never printed, logged, or written to
a file; provider errors name source and stage only. The audit log redacts
secret-carrying arguments (`gh secret set` bodies, `Authorization` headers)
in its copy only.

**Raw mode** — `fleet gh --raw <args…>` or `FLEET_GH_RAW=1` — is the
first-class bypass: the real `gh` execs directly with the caller's ambient
credentials; no token is resolved or injected, and there is no rotation or
failover. It exists so callers that must pin one identity — and the
`gh` → `fleet gh` PATH shim, whose escape hatch this is — never have to
choose between the rotation and bypassing fleet entirely.

## Token rotation: three Apps, then personal

The rotation order in source (the top-level help understates it as one
App):

1. **Fleet App installations** — the "developing.today fleet" App, App ID
   **5612056** (`FLEET_GH_APP_ID` may override). Private key by secret name
   `github-app.developing-today-fleet.private-key`. Default owners, in
   order: `developing-today`, `developing-today-forks`, `dezren39`.
2. **developing.today App installations** — App ID and key both by name
   (`kaneo.github-app-id`, `kaneo.github-app-private-key`). Default owners:
   `developing-today`, `dezren39`.
3. **developing-today-backup App installations** — by name
   (`github-app.backup-app-id`, `github-app.backup-private-key`). Default
   owner: `developing-today`.
4. **Personal** — the user's `gh` OAuth token, parsed out of the broker
   secret named `github-cli.hosts-yml` (the hosts.yml file itself, stored
   by name; the parser extracts its `oauth_token`). Tried **last** (Drew,
   2026-11-23): the personal credential is the final fallback, not the
   first identity used.

Per App leg: the private key is resolved by name; an RS256 JSON Web Token
is signed with the system `openssl`, the key passed over an inherited file
descriptor — never written to disk, never in argv; the owner's installation
is resolved live from `GET /app/installations`, paginated at 100 per page
(the App has 100+ installations).

**Hint-first.** When the call names a repo or org, that owner's
installation moves to the front of _each_ App leg, because an installation
token only sees its own installation's scope.

Controls: `FLEET_GH_ORDER` (a comma list fixing the rotation exactly:
`personal`, `app:<owner>`, `devtoday:<owner>`, `backup:<owner>`);
`FLEET_GH_APP_OWNERS` / `FLEET_GH_DEVTODAY_OWNERS` / `FLEET_GH_BACKUP_OWNERS`
(comma lists replacing an App's default owners).

## Failover triggers: the retry-safety rule

A failed attempt's combined stdout+stderr is classified:

| Class          | Trigger strings (case-insensitive)                                                                                                                                                                                                       | Retry?                   |
| -------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------ |
| RateLimit      | `rate limit`, `rate_limit`, `too many requests`, `http 429`                                                                                                                                                                              | Yes — any call           |
| Permission     | `missing required scopes`, `insufficient_scopes`, `resource not accessible`, `not accessible by integration`, `not accessible by personal access token`, `this endpoint requires you to be authenticated`, `bad credentials`, `http 401` | Yes — any call           |
| NodeResolution | `could not resolve to a node`                                                                                                                                                                                                            | **Read-only calls only** |
| Other          | anything else                                                                                                                                                                                                                            | **Read-only calls only** |

The rule: **a write is only repeated under a fresh token when the failure
proves GitHub refused the request before processing it.** Rate-limit
rejections (the limiter answers before the mutation runs) and
permission/scope rejections are both pre-execution refusals.
Node-resolution is explicitly _not_ one: GraphQL executes mutation fields
serially, so a batched mutation may already have run an earlier field
before a later field's node fails to resolve; replaying would duplicate
the part that landed. Any other write failure is returned as-is — the
request may have landed, and repeating it could double-post. **Reads repeat
on any failure class.**

A token _provider_ failure (a source that cannot resolve or mint) skips
that source silently (named under trace) and the rotation continues; if no
attempt produced output, the caller gets the provider errors and exit 3.
If the last failover has no successor, the caller is shown the real `gh`
failure, not a summary.

Two wrappers sit in front of the rotation:

- **Close guard.** `issue close` and `issue edit --add-label status:done`
  are refused (exit 2, never reaching the rotation) when the queue broker
  has a claimed or parked task bound to that repo#issue, unless the bound
  task's completion record (a `fleet-closure/v1` record carrying its task
  id) is presented with the wrapper-only flag `--completion-record <path>`,
  stripped before `gh` runs. The queue query **fails open** — a broker
  outage must not block every close.
- **Metadata create path.** `issue create` / `pr create` bodies are stamped
  with the fleet metadata block before the rotation runs, and the full
  record is inserted into the metadata store after a successful create; a
  malformed caller block refuses the create before any write; a failed
  insert never loses the item (reported on stderr; ingestion heals it).

## The 4802 cache

`fleet gh cache-serve [--bind <addr>]` runs the credential/token cache
daemon in the foreground. It caches four things, **memory-only, with
single-flight minting** (concurrent requests for one key block on the
in-flight mint):

| Entry kind                 | TTL                                                                         |
| -------------------------- | --------------------------------------------------------------------------- |
| Secret resolutions         | 60 s                                                                        |
| App JWTs                   | within their expiry, minus a 60 s skew                                      |
| Installation tokens        | until shortly before `expires_at`, minus the same 60 s skew, floored at 1 s |
| Per-App installation lists | 300 s                                                                       |

Nothing is written to disk; a restart is a cold cache, never a stale one.
The wire is newline-delimited JSON over TCP, protocol version 1 — a daemon
that does not know a request's version answers `{ok:false, …}` rather than
guessing. Bind: `--bind` beats `FLEET_GH_CACHE_BIND` beats the default
**`127.0.0.1:4802`**; a bind without a port gets 4802. Every normal
`fleet gh` call resolves through the localhost daemon first, then the
laptop daemon (the proxy's reach target at the same port); with no daemon
answering it resolves in-process over the _same_ cache implementation and
says so on stderr ("resolving in-process").

`fleet gh cache-status` probes the configured cache daemons and prints each
one's snapshot — entry counts by kind, hit/miss counters, soonest expiry.
Key names and values never leave the daemon; counts only. It exits 3 when
no daemon answered.

## The shadow verbs: tiers and closures

**`fleet gh tiers`** probes _every_ rotation source and prints its core and
GraphQL rate-limit headroom — labels and numbers only; token values never
leave the provider. Exit 0 when every source answers, **3 when any source
is down**. Source behaviour: `fleet gh tiers --help` prints the verb's own
usage and exits 0; any other argument errors.

**`fleet gh closures [<item>]`** reads back the closure log. The policy
layer behind it: `issue|pr close|reopen` through the rotation must carry a
`--reason` from the taxonomy — `implemented`, `superseded`, `cancelled`,
`started-incorrectly` — enforced with the policy error "Please give a
reason. This is required by policy." The reason is written into the close
comment, and every successful close or reopen appends one `fleet-closure/v1`
JSON-line record to `closure-records.jsonl` in the fleet state directory
(backed up with it); `closures` prints the log, optionally filtered to one
item. Source behaviour: the verb produces **no help output at all** — its
behaviour is documented only in source, and here.
