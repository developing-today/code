# fleet gh

`fleet gh` is the real `gh` CLI, passed through under a rotating fleet token. Four argument vectors are **shadow subcommands** (`tiers`, `cache-serve`, `cache-status`, `closures`); every other argument vector passes through to the real `gh` unchanged.

See also [The gh passthrough](../concepts/gh.md).

## The passthrough (any other arguments)

```
fleet gh <args...>
fleet gh --raw <args...>
```

| Flag               | Argument | Default                     | Meaning                                                                                                                                        |
| ------------------ | -------- | --------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| `--raw`            | —        | off (also `FLEET_GH_RAW=1`) | First-class bypass: exec the real `gh` directly with the caller's ambient credentials; no token resolved or injected, no rotation or failover. |
| _(all other args)_ | —        | —                           | Passed to the real `gh` binary unchanged. `FLEET_GH_BIN` selects the binary, default `gh`.                                                     |

### Behaviour

Before each attempt, one token is resolved by name through the fleet secrets chain and exported as `GH_TOKEN` **for that attempt only**. Attempts are _captured_ (piped), not streamed, because the failure text is what classifies a failover; the caller sees the final attempt's stdout, stderr, and exit code **byte-exact** as `gh` produced them. Attempts are silent by default; `FLEET_GH_TRACE=1` names each attempt's token _source_ (never a value) on stderr. Tokens are never printed, logged, or written to a file; provider errors name source and stage only. The audit log redacts secret-carrying arguments (`gh secret set` bodies, `Authorization` headers) in its copy only.

**Rotation order** (source behaviour):

1. **Fleet App installations** — "developing.today fleet", App ID **5612056** (`FLEET_GH_APP_ID` may override). Private key by secret name `github-app.developing-today-fleet.private-key`. Default owners, in order: `developing-today`, `developing-today-forks`, `dezren39`.
2. **developing.today App installations** — App ID and key both by name (`kaneo.github-app-id`, `kaneo.github-app-private-key`). Default owners: `developing-today`, `dezren39`.
3. **developing-today-backup App installations** — by name (`github-app.backup-app-id`, `github-app.backup-private-key`). Default owner: `developing-today`.
4. **Personal** — the `gh` OAuth token, parsed out of the broker secret named `github-cli.hosts-yml` (the hosts.yml file itself, stored by name; the parser extracts its `oauth_token`). Tried last: the final fallback, not the first identity used.

Known discrepancy: the help summary's "the developing.today App's installations, then personal last" understates the source — there are **three** Apps in a fixed order (fleet → developing.today → backup), each expanded over its owners, with the personal token last.

Mechanics per App leg: the private key is resolved by name; an RS256 JWT is signed with the system `openssl`, the key passed over an inherited fd — never written to disk, never in argv; the owner's installation is resolved live from `GET /app/installations`, paginated at 100/page. **Hint-first:** when the call names a repo or org, that owner's installation moves to the front of _each_ App leg, because an installation token only sees its own installation's scope. Controls: `FLEET_GH_ORDER` (comma list fixing the rotation exactly: `personal`, `app:<owner>`, `devtoday:<owner>`, `backup:<owner>`), `FLEET_GH_APP_OWNERS` / `FLEET_GH_DEVTODAY_OWNERS` / `FLEET_GH_BACKUP_OWNERS` (comma lists replacing an App's default owners).

**Failover triggers.** A failed attempt's combined stdout+stderr is classified:

| Class          | Trigger strings (case-insensitive)                                                                                                                                                                                                       | Retry?               |
| -------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------- |
| RateLimit      | `rate limit`, `rate_limit`, `too many requests`, `http 429`                                                                                                                                                                              | Yes — any call       |
| Permission     | `missing required scopes`, `insufficient_scopes`, `resource not accessible`, `not accessible by integration`, `not accessible by personal access token`, `this endpoint requires you to be authenticated`, `bad credentials`, `http 401` | Yes — any call       |
| NodeResolution | `could not resolve to a node`                                                                                                                                                                                                            | Read-only calls only |
| Other          | anything else                                                                                                                                                                                                                            | Read-only calls only |

The rule: a **write is only repeated under a fresh token when the failure proves GitHub refused the request before processing it** — rate-limit and permission/scope rejections are both pre-execution refusals. Node-resolution is explicitly not one: GraphQL executes mutation fields serially, so a batched mutation may already have run an earlier field before a later field's node fails to resolve; replaying would duplicate the part that landed. Any other write failure is returned as-is. Reads repeat on any failure class. A token _provider_ failure skips that source silently (named under trace) and the rotation continues; if no attempt produced output, the caller gets the provider errors and exit 3. If the last failover has no successor, the caller is shown the real `gh` failure, not a summary.

Two wrappers sit in front of the rotation:

- **Close guard:** `issue close` and `issue edit --add-label status:done` are refused (exit 2, never reaching the rotation) when the queue broker has a claimed or parked task bound to that repo#issue, unless the bound task's completion record (a `fleet-closure/v1` record carrying its task id) is presented with the wrapper-only flag `--completion-record <path>`, stripped before `gh` runs. The queue query **fails open** — a broker outage must not block every close.
- **Metadata create path:** `issue create` / `pr create` bodies are stamped with the fleet Metadata block before the rotation runs, and the full record is inserted into the metadata store after a successful create; a malformed caller block refuses the create before any write; a failed insert never loses the item (reported on stderr, ingestion heals it). Controls: `FLEET_METADATA_DB`, `FLEET_GH_STAMP_KEY_ID` (key material by name from the secrets broker).

### Exit codes

- The final attempt's `gh` exit code, byte-exact.
- `2` — close-guard refusal, or a malformed metadata block on a create.
- `3` — no token source produced an attempt (provider errors reported).

### Example

```console
$ fleet gh api rate_limit --jq '.rate.remaining'
```

Runs the real `gh` under the first rotation token that succeeds, failing over on a rate-limit or permission refusal.

## fleet gh tiers

```
fleet gh tiers
```

| Flag | Argument | Default | Meaning         |
| ---- | -------- | ------- | --------------- |
| —    | —        | —       | Takes no flags. |

### Behaviour

Probes _every_ rotation source and prints its core and GraphQL rate-limit headroom — labels and numbers only; token values never leave the provider.

Help: `fleet gh tiers --help` prints the verb's own usage and exits 0; any other argument is a usage error naming it.

### Exit codes

- `0` — every source answers.
- `3` — any source is down.

### Example

```console
$ fleet gh tiers
```

Prints per-source core and GraphQL headroom for all three Apps' installations and the personal token.

## fleet gh cache-serve

```
fleet gh cache-serve [--bind <addr>]
```

| Flag     | Argument | Default                                      | Meaning                                                                                                |
| -------- | -------- | -------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| `--bind` | `<addr>` | `FLEET_GH_CACHE_BIND`, else `127.0.0.1:4802` | Daemon bind address. `--bind` beats the env, which beats the default; a bind without a port gets 4802. |

### Behaviour

Runs the credential/token cache daemon in the foreground (port **4802** — the next port after the queue broker's 4801). It caches four things, **memory-only, single-flight minting** (concurrent requests for one key block on the in-flight mint):

| Entry kind                 | TTL                                                                         |
| -------------------------- | --------------------------------------------------------------------------- |
| Secret resolutions         | 60 s                                                                        |
| App JWTs                   | within their expiry, minus a 60 s skew                                      |
| Installation tokens        | until shortly before `expires_at`, minus the same 60 s skew, floored at 1 s |
| Per-App installation lists | 300 s                                                                       |

Nothing is written to disk; a restart is a cold cache, never a stale one. The wire is newline-delimited JSON over TCP, protocol version 1 — a daemon that does not know a request's version answers `{ok: false, …}` rather than guessing. Every normal `fleet gh` call resolves through the localhost daemon first, then the laptop daemon (the proxy's reach target at the same port; `[gh] cache_local` / `cache_laptop` in config, `FLEET_GH_CACHE` env beats both); with no daemon answering it resolves in-process over the _same_ cache implementation and says so on stderr ("resolving in-process").

### Exit codes

- `0` — clean exit.
- `2` — usage.
- `69` — capability unavailable.

### Example

```console
$ fleet gh cache-serve
```

Runs the cache daemon in the foreground on 127.0.0.1:4802.

## fleet gh cache-status

```
fleet gh cache-status
```

| Flag | Argument | Default | Meaning                          |
| ---- | -------- | ------- | -------------------------------- |
| —    | —        | —       | Takes no arguments and no flags. |

### Behaviour

Probes the configured cache daemons and prints each one's snapshot — entry counts by kind, hit/miss counters, soonest expiry. Key names and values never leave the daemon; counts only.

### Exit codes

- `0` — at least one daemon answered.
- `2` — usage (trailing argument).
- `3` — **no daemon answered**.

### Example

```console
$ fleet gh cache-status
```

Prints counts-only snapshots for each configured cache daemon.

## fleet gh closures

```
fleet gh closures [<item>]
```

| Flag                            | Argument       | Default | Meaning                                                       |
| ------------------------------- | -------------- | ------- | ------------------------------------------------------------- |
| `<item>` (positional, optional) | item reference | —       | Filter the closure log to one item. This verb takes no flags. |

### Behaviour

Reads back the closure log. The policy layer behind it: `issue|pr close|reopen` through the rotation must carry a `--reason` from the taxonomy — `implemented`, `superseded`, `cancelled`, `started-incorrectly` — enforced with the policy error "Please give a reason. This is required by policy." The reason is written into the close comment, and every successful close or reopen appends one `fleet-closure/v1` JSON-line record to `closure-records.jsonl` in the fleet state directory (backed up with it); `closures` prints the log, optionally filtered to one item.

Help: `fleet gh closures --help` prints the verb's own usage and exits 0.

### Exit codes

- `0` — ok.
- `2` — usage.

### Example

```console
$ fleet gh closures
```

Prints the full `fleet-closure/v1` closure log, one JSON record per line.
