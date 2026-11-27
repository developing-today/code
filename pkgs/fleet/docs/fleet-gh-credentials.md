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
process**. Rotation policy can persist in the fleet config file,
but credential values are resolved by name and never written there.

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

Default rotation for one invocation (`gh.rs`, `rotation_with`):
each App leg in App order (fleet → developing.today → backup) with
that App's owners in list order, then the **personal leg last**
(the user, 2026-11-23, code#13310) — the user's own token is the final
fallback, not the first identity tried. Total: 3 + 2 + 1 + 1 =
**7 sources** when no additional Apps are configured.

Compiled-in owner lists (`gh.rs:104-110`):

- Fleet App: `developing-today`, `developing-today-forks`, `dezren39`
- developing.today App: `developing-today`, `dezren39`
- Backup App: `developing-today`

### Persistent rotation config (code#10311 ask (a))

The same two knobs live in the fleet config file, so the rotation survives
shells and machines. Env beats the file per process; the file
beats the compiled-in lists above.

```toml
[gh]
# Replaces the whole rotation exactly, same grammar as
# FLEET_GH_ORDER. Enumerating the entries also selects the
# enabled set: an App left out is not tried.
order = ["app:developing-today", "app:dezren39", "personal"]

[gh.owners]
# Replaces one App's compiled-in owners (empty list disables it).
app = ["developing-today", "dezren39"]
```

A blank order/owner entry and an unknown `[gh.owners]` slug are
loud config-load errors (the #5460/#6405 rules); an empty owner
list (`app = []`) is allowed and disables that App.

#### Environment overrides

Environment overrides are per-process and beat the persistent
`[gh]` rotation settings above.

### Additional Apps from fleet config (`code#10311`)

Declare additional GitHub Apps in the fleet TOML config; no binary
change is required. These entries **extend**, not replace, the three
built-ins. Each configured App follows the built-ins in declaration
order, with its owners in list order; personal remains the final
fallback. Normal calls and `fleet gh tiers` use the same configuration.

```toml
[[gh.apps]]
slug = "layercloud-app"
app_id = 123456
key_secret = "github-app.layercloud.private-key"
owners = ["layercloud", "another-org"]

[[gh.apps]]
slug = "another-app"
app_id_secret = "github-app.another.id"
key_secret = "github-app.another.private-key"
owners = ["layercloud"]
```

- Set exactly one of `app_id` (public integer ID) or `app_id_secret`
  (the name of a secret containing the ID). `key_secret` always names
  a secret containing the PEM private key. Resolution uses the existing
  fleet-secrets chain; never put private-key material in the config.
- Slugs must be unique, nonempty lowercase ASCII letters, digits or
  hyphens. The built-in slugs `app`, `devtoday` and `backup` are reserved.
  Secret names and owner entries must not be blank.
- `owners` defaults to an empty list: that App contributes no sources
  to the default rotation, but is still available in an explicit order.
- `FLEET_GH_ORDER=layercloud-app:layercloud,personal` selects a configured
  App exactly like a built-in. The whole-order override remains exact;
  it may name an owner outside the App's default owner list. Unknown
  slugs retain the legacy behavior of becoming a personal leg.
- Owner hints reorder configured owners case-insensitively. When no
  explicit `FLEET_GH_ORDER` is set, an owner absent from an App's
  configured list is also checked against that App's live installations;
  a match adds the App leg for that call. The built-in owner and App ID
  environment overrides remain unchanged and do not affect added Apps.

Cache requests identify Apps by slug, not by credentials. Configure the
same `[[gh.apps]]` definitions on the invoking host **and every cache
daemon host**, then restart running daemons after changing definitions.
The daemon resolves a slug against its own loaded config. An unknown
slug is an error; a daemon answering with an error is authoritative and
does not trigger local minting. Existing cache lifetimes, secret
resolution, signing and installation minting behavior are unchanged.

The environment variables `FLEET_GH_APP_OWNERS`,
`FLEET_GH_DEVTODAY_OWNERS` and `FLEET_GH_BACKUP_OWNERS` override
the matching built-in App owner list. Configured Apps use their
`owners` arrays in `[[gh.apps]]`.

| Variable                   | Effect (`gh.rs:74-86`, `gh.rs:352-378`)                                                                                                                                                                                                                                                   |
| -------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `FLEET_GH_ORDER`           | Comma list fixing the whole rotation exactly: entries `personal`, `app:<owner>` (fleet App), `devtoday:<owner>`, `backup:<owner>`, or a configured `<slug>:<owner>`. **Gotcha:** an entry matching no known `slug:` prefix silently becomes an extra `personal` leg — typos do not error. |
| `FLEET_GH_APP_OWNERS`      | Comma list replacing the fleet App's owners.                                                                                                                                                                                                                                              |
| `FLEET_GH_DEVTODAY_OWNERS` | Comma list replacing the developing.today App's owners.                                                                                                                                                                                                                                   |
| `FLEET_GH_BACKUP_OWNERS`   | Comma list replacing the backup App's owners.                                                                                                                                                                                                                                             |
| `FLEET_GH_APP_ID`          | Overrides the fleet App ID constant `5612056`.                                                                                                                                                                                                                                            |

### Owner hint: reorder and discover live installations

When the call names an owner — `--repo`/`-R owner/repo`,
`--owner org`, an `api repos/<owner>/…` path, or a `github.com/`
URL — that owner's leg moves to the **front of each App leg that
already lists it**. If an App does not list the owner, and no explicit
`FLEET_GH_ORDER` overrides the rotation, its cached live installation
list is checked; a match adds that App leg before its configured legs.
A list lookup error is reported and leaves that App's configured
rotation intact. Installation data uses the existing paginated cache
and lifetime. This lets private/scoped calls use an App installed in
an otherwise unconfigured org without weakening the personal-token-last
fallback.

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

When a call names an owner outside the configured lists, each App's
paginated installation list is checked. A live installation adds that
App's leg for the call; an App with no matching installation is not
added. An installation-list error is reported and does not discard the
configured rotation. This is the dynamic-hint choice for #10311(e): it
matches the observed `layercloud` case (all three Apps are installed
there, but none had a configured owner entry) while retaining the
personal token as the final fallback. Setting `FLEET_GH_ORDER` remains
an explicit override and disables discovery.

For an inventory of App installations beyond the configured owner
lists, use `fleet gh tiers --all`. It lists every installation of each
built-in and configured App (the App installation list is paginated),
then probes each installation's rate-limit headroom; plain
`fleet gh tiers` remains limited to the configured rotation. The
`--all` command may make many API calls and can take longer for Apps
with many installations.

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
