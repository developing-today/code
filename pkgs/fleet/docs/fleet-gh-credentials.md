# `fleet gh` — credential lifecycle

Operator reference for how `fleet gh` gets its tokens, what it does
with them, and where the sharp edges are. Everything below is
verified against `crates/fleet/src/gh.rs` on current `main` plus the
secrets chain in `crates/fleet-secrets/src/lib.rs`; where live probes
established a fact the issue is cited. This page closes
developing-today/code#10744.

## The one-sentence model

Every `fleet gh` invocation resolves one token **by name** from the
fleet secrets broker, exports it as `GH_TOKEN` to **one child `gh`
process**, and throws everything away on exit. There is no cache, no
state file, and no cross-invocation reuse of anything (see
[No caching](#no-caching-across-invocations)).

## Step 1 — secret resolution by name

All credentials resolve through `fleet-secrets`
(`crates/fleet-secrets/src/lib.rs:1-46`), in this order, per secret
name (`crates/fleet/src/gh.rs:497-502`):

1. **Secrets-broker protocol** — a framed request through the
   session broker to port 4800, `GET /secret/<name>`. Fleet never
   opens the token file itself.
2. **`bws` CLI fallback** — `bws secret list` + `get`, the same
   shape the secrets broker uses to fill its own cache. It carries
   its own credential: `BWS_ACCESS_TOKEN` when non-empty, else the
   on-disk machine-account token file.

Nothing persists a value (no cache, no file, no log line), an empty
string counts as resolution failure, and errors name the secret and
the source — never a value (`lib.rs:15-28`).

### Exact secret names per leg (`gh.rs:101-110`)

| Leg                      | Secret name                                                        | Notes                                                                                                                                                                        |
| ------------------------ | ------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Personal                 | `github-cli.hosts-yml`                                             | The whole `gh` hosts file, stored by name; `fleet gh` parses the `oauth_token:` line out of it (`gh.rs:628-642`). The value never leaves the resolver except into the child. |
| Fleet App key            | `github-app.developing-today-fleet.private-key`                    | PEM private key of the "developing.today fleet" App.                                                                                                                         |
| Fleet App ID             | _(compiled-in constant `5612056`; `FLEET_GH_APP_ID` overrides it)_ | The only ID not carried in the broker (`gh.rs:483-487`).                                                                                                                     |
| developing.today App ID  | `kaneo.github-app-id`                                              |                                                                                                                                                                              |
| developing.today App key | `kaneo.github-app-private-key`                                     |                                                                                                                                                                              |
| Backup App ID            | `github-app.backup-app-id`                                         |                                                                                                                                                                              |
| Backup App key           | `github-app.backup-private-key`                                    |                                                                                                                                                                              |

## Step 2 — in-process minting (App legs only)

The personal leg uses its token directly. Each App leg mints an
installation token **inside the `fleet gh` process**
(`gh.rs:505-609`):

1. **RS256 JWT** — header `{"alg":"RS256","typ":"JWT"}`, claims
   `iat = now − 60s`, `exp = now + 540s` (a ~10-minute window), `iss`
   = the App ID. The signature is computed by the system `openssl`
   binary (`openssl dgst -sha256 -sign`); the private key travels on
   an inherited file descriptor (`/dev/fd/N`) — never written to a
   file, never placed in argv or the environment
   (`gh.rs:680-736`).
2. **Installation lookup** — live `GET /app/installations`
   (`per_page=100`, paginated; the first-page-only read once lost the
   fleet in #7930), matched on `account.login` against the leg's
   owner (`gh.rs:559-588`).
3. **Token exchange** — `POST /app/installations/{id}/access_tokens`,
   success is HTTP 201; the `token` field is the leg's credential
   (`gh.rs:592-608`).

The App leg's own HTTPS validates against the **platform root
store** (ureq `PlatformVerifier`), not webpki-roots alone: behind a
TLS-intercepting egress proxy the proxy CA lives only in the
platform store (`gh.rs:36-40`, `gh.rs:544-553`).

Within one process, each App's JWT is built once and each
owner-installation token is minted once (the `app_jwts` / `minted`
caches on the per-process provider, `gh.rs:459-464`) — so repeated
failovers to the same leg do not re-mint. The process exiting drops
all of it.

## Step 3 — the child gets `GH_TOKEN`, and only the child

`fleet gh` runs the real `gh` binary once per attempt with
`GH_TOKEN` set **on the child process environment only**
(`gh.rs:775-786`); the parent's environment is untouched. Attempts
are captured (piped), not streamed, because the failure text is what
classifies a failover — the final attempt's stdout, stderr and exit
code are then replayed byte-exact (`gh.rs:64-72`, `gh.rs:867-876`).
Stdin is inherited so prompts still behave.

## Rotation order and overrides

Default rotation for one invocation (`gh.rs:380-429`): **personal
first**, then each App leg in App order (fleet → developing.today →
backup) with that App's owners in list order. Total: 1 + 3 + 2 + 1 =
**7 sources**.

Compiled-in owner lists (`gh.rs:104-110`):

- Fleet App: `developing-today`, `developing-today-forks`, `dezren39`
- developing.today App: `developing-today`, `dezren39`
- Backup App: `developing-today`

Environment overrides (per-process, nothing persists):

| Variable                   | Effect (`gh.rs:74-86`, `gh.rs:352-378`)                                                                                                                                                                                                                             |
| -------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `FLEET_GH_ORDER`           | Comma list fixing the whole rotation exactly: entries `personal`, `app:<owner>` (fleet App), `devtoday:<owner>`, `backup:<owner>`. **Gotcha:** an entry matching no `slug:` prefix silently becomes an extra `personal` leg — typos do not error (`gh.rs:399-411`). |
| `FLEET_GH_APP_OWNERS`      | Comma list replacing the fleet App's owners.                                                                                                                                                                                                                        |
| `FLEET_GH_DEVTODAY_OWNERS` | Comma list replacing the developing.today App's owners.                                                                                                                                                                                                             |
| `FLEET_GH_BACKUP_OWNERS`   | Comma list replacing the backup App's owners.                                                                                                                                                                                                                       |
| `FLEET_GH_APP_ID`          | Overrides the fleet App ID constant `5612056`.                                                                                                                                                                                                                      |

### Owner hint: reorder only, never add

When the call names an owner — `--repo`/`-R owner/repo`,
`--owner org`, an `api repos/<owner>/…` path, or a `github.com/`
URL — that owner's leg moves to the **front of each App leg that
already lists it** (`gh.rs:302-341`, `gh.rs:418-427`). A hint for an
owner no leg lists changes nothing: `fleet gh` **never mints for an
unconfigured owner** (see [Unknown-org behavior](#unknown-org-behavior)).

## Retry-safety: what repeats under the next token

A failed attempt fails over only when the failure proves GitHub
refused the request **before processing it**
(`gh.rs:55-62`, `gh.rs:436-444`):

- **Rate limit** (rate-limit strings, `429`, `too many requests`)
  and **permission/scope refusals** (`INSUFFICIENT_SCOPES`,
  `Resource not accessible by …`, `bad credentials`, `401`) —
  **any** call repeats, including writes: nothing landed.
- **Anything else** — repeats **only if the call is read-only**.
  A write whose failure is unclassifiable is returned as-is; it may
  have landed and repeating it could double-post.

Read-only is a whitelist (`gh.rs:248-296`): `api` with GET/HEAD and
no field flags; `list`/`view`/`status`/`search`/`diff`/`checks`
verbs; `issue`/`pr`/`repo`/… subcommands only in their
`list|view|status|field-list|item-list|diff|checks` shapes.
**Anything unrecognized is a write** (the safe default).

Attempts are silent by default; `FLEET_GH_TRACE=1` names each
attempt's token _source_ (label only, never a value) on stderr
(`gh.rs:813-815`, `gh.rs:833-846`).

## Unknown-org behavior

Installation lookup runs only for owners already in the configured
lists, and the hint never adds one — so for an org no leg is
configured for, `fleet gh` runs the call under other orgs' legs.
A scoped/private call under a wrong-org leg fails
`403 Resource not accessible by integration`, which classifies as a
permission refusal and fails over to the next leg; if no leg covers
the org, every leg 403s in turn. This is the #10744 gap: live probes
(2026-11-14, org `layercloud`, all three Apps installed there but no
leg configured) confirmed the default rotation 403s while
`FLEET_GH_ORDER=…,app:layercloud,…` succeeds. In the default
rotation the **personal token, tried first, masks the gap** while it
has headroom; public reads also pass under any leg. Tracked fix:
configurable rotation without code changes, code#10311.

## No caching across invocations

Every call re-resolves secrets through the broker, re-signs the
JWTs, re-lists installations, and re-mints installation tokens; all
caching is the in-memory per-process provider above. For interactive
use this is seconds of overhead per call; for automation it
compounds (the closure watchdog on code#10138 could not clear a
~41h event backlog inside a 10-minute run on 2026-11-16).

**This is interim and owned, not tribal knowledge.** Tracked fix:
the credential/token caching daemon, code#10742 — memory-only
caching of secret results (short TTL), JWTs (within expiry),
installation tokens (until shortly before expiry) and installation
lists, with single-flight minting; daemon address as a fleet-config
setting with a per-invocation flag override; localhost-then-laptop
resolution order with a default port; and a **visible** fallback note
when no daemon answers (today's in-process path remains the
fallback). Until that lands, assume each `fleet gh` call pays the
full resolution cost.

## Exit codes and bypasses

- **Exit 3 is ours only**: "no token source could be resolved at
  all" — every provider error printed (source + stage, never
  values), then `fleet gh: no token source could be resolved`
  (`gh.rs:96-99`, `gh.rs:859-864`). The same 3 is returned when the
  `gh` binary cannot be spawned at all, and by `fleet gh tiers`
  when any source is down (see below). Usage errors exit 2; every
  other exit code is the final attempt's own, propagated unchanged.
- **`fleet gh tiers`** (a shadow subcommand; real `gh` has none):
  probes every configured rotation source with `gh api
rate_limit`, prints `<label>\tcore <rem>/<lim>\tgraphql
<rem>/<lim>`, exits 3 when any source is DOWN
  (`gh.rs:878-927`).
- **`FLEET_GH_BIN`** — the `gh` binary to run (default `gh`).
  With `gh` absent from PATH the call fails cleanly on stderr with
  exit 3 (`gh.rs:816-821`).
- **`FLEET_GH_RAW=1` / leading `--raw`** — first-class bypass: the
  real `gh` execs directly with the caller's ambient credentials
  and environment; **no token is resolved, injected, rotated, or
  failed over** (`gh.rs:929-964`). Only a _leading_ `--raw` is
  fleet's; `--raw` anywhere else belongs to `gh` (its
  `--raw-field` family lives further along the command line).
