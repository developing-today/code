# fleet gh: credential lifecycle and current limitations

**Status:** CURRENT BEHAVIOUR, as implemented (read from source and
verified against live behaviour 2026-11-14 / 2026-11-16). **Source of
truth:** `pkgs/fleet/crates/fleet/src/gh.rs` (the `fleet gh` domain,
code#8887) and `pkgs/fleet/crates/fleet-secrets/src/lib.rs` (the
secrets resolver chain it uses). **Closes:** code#10744. **Tracked
fixes for the interim parts:** code#10311 (make the token rotation
configurable without code changes) and code#10742 (credential/token
caching daemon for sandbox and laptop). This page documents reality
until those land; it does not change any behaviour (changing
behaviour is code#10742's scope, not this page's).

## What `fleet gh` is

`fleet gh` is a passthrough wrapper around the real `gh` CLI. Every
argument passes through unchanged; the caller sees the final
attempt's stdout, stderr and exit code exactly as `gh` produced
them. What the wrapper adds is the identity: before each attempt it
resolves one token by name from the fleet secrets broker and exports
it as `GH_TOKEN` for that attempt (child process) only. On a failure
class that proves GitHub refused the request before execution — a
rate limit or a permission/scope rejection — or on any failure of a
read-only call, it fails over to the next token in the rotation
instead of failing the caller.

Tokens are never printed, logged, or written to a file. Provider
errors name the source and the stage only, never a value. Attempts
are silent by default; `FLEET_GH_TRACE=1` names each attempt's token
*source* (never the token) on stderr. Attempt output is captured
(piped), not streamed, because the failure text is what classifies a
failover.

## The rotation, in order

1. **Personal token.** Drew's `gh` OAuth token, parsed out of the
   broker secret named `github-cli.hosts-yml` — the `gh` hosts file
   itself, stored by name. The resolver extracts the `oauth_token:`
   line; the file's value never leaves the resolver.
2. **Installation tokens of the fleet's three GitHub Apps**, in App
   order, minted in-process per owner (see "Minting" below):

   | Leg (label prefix) | App | App ID source | Key secret name | Default owners |
   |---|---|---|---|---|
   | `app:<owner>` | "developing.today fleet" | compiled-in constant `5612056` (override: `FLEET_GH_APP_ID`) | `github-app.developing-today-fleet.private-key` | `developing-today`, `developing-today-forks`, `dezren39` |
   | `devtoday:<owner>` | developing.today | broker secret `kaneo.github-app-id` | `kaneo.github-app-private-key` | `developing-today`, `dezren39` |
   | `backup:<owner>` | developing-today-backup | broker secret `github-app.backup-app-id` | `github-app.backup-private-key` | `developing-today` |

   App IDs and keys are resolved **by name** through the secrets
   broker (next section); no key or token value is ever in argv, in
   the environment of the child `gh` (except the finished
   installation token as `GH_TOKEN`), or on disk.

## How a secret name becomes a value

Credentials resolve by name through the fleet secrets resolver
(`Resolver::default_chain` in `fleet-secrets`):

1. **Secrets broker first** — a framed request through the session
   broker to the secrets broker (`GET /secret/<name>`), with the
   token *file* named in the request (the broker reads the file; the
   token value does not travel in the request either). On the laptop
   a direct broker leg is probed first where the broker's own socket
   answers. A broker leg that cannot answer is retried under a
   bounded retry policy before the chain moves on (a broker restart
   during a switch must not degrade a whole consumer); a broker 404
   is a definitive answer about the name and is not retried as an
   outage.
2. **Bitwarden CLI (`bws`) fallback** — the same name-resolution
   shape the secrets broker itself uses to fill its cache, used when
   the broker leg cannot answer.

Operator consequence: a `fleet gh` failure that names a secret as
"unresolved" is a broker/Bitwarden problem (name missing or chain
down), not a GitHub problem — check the secrets broker before
debugging GitHub permissions.

## Minting an App installation token (in-process, per invocation)

For each App leg that is actually attempted:

1. Resolve the App's private key by name (table above) and its App
   ID (constant for the fleet App; broker secret for the other two).
2. Build an RS256 JWT in-process: header `{"alg":"RS256","typ":"JWT"}`,
   payload `iat` = now − 60 s, `exp` = now + 540 s, `iss` = App ID.
   Only the signature touches the private key, and that happens
   inside `openssl dgst -sha256 -sign` with the key handed over on
   an inherited file descriptor (`/dev/fd/<n>`) — never written to
   disk, never in argv.
3. Resolve the owner's installation **live** from
   `GET /app/installations`, paginated at 100 per page. (First-page-
   only reads silently lose installations — the fleet-paperwork
   #7930 break; the pagination is load-bearing.)
4. Exchange: `POST /app/installations/<id>/access_tokens` returns
   the installation token, which is exported as `GH_TOKEN` to the
   child `gh` for that attempt only.

The App leg's own HTTPS validates against the **platform** root
store (ureq's platform verifier), not webpki-roots alone: hosts
behind a TLS-intercepting egress proxy (the sandbox) present the
proxy's CA, which lives only in the platform store.

Within one process, a built JWT is reused per App and a minted
token is reused per (App, owner) pair. That is the *only* caching
that exists — see the next section.

## INTERIM: no cross-invocation caching

There is **no** caching across invocations. Every `fleet gh` call
starts a fresh process, so every call re-resolves its secrets,
re-signs its JWTs, re-lists installations, and re-mints installation
tokens for every leg it attempts. That is the current, deliberate
state of the code — and its cost: each App-leg attempt pays a full
secrets-broker round trip, an `openssl` sign, a paginated
installation list, and a token exchange before `gh` itself runs.

This limitation is owned, not tribal knowledge: code#10742 tracks
the credential/token caching daemon for sandbox and laptop that
removes the per-invocation cost. Until it lands, do not "fix" slow
`fleet gh` calls by wrapping them in local token caches — a cached
installation token held outside this lifecycle is an unrotated
credential.

## Leg lists are compiled in; the environment overrides

The rotation's leg lists are **compiled-in constants** in `gh.rs`
(the `DEFAULT_APP_OWNERS` / `DEVTODAY_DEFAULT_OWNERS` /
`BACKUP_DEFAULT_OWNERS` constants and `GhApp::ALL` order). They can
be replaced at runtime, per invocation, without a rebuild:

| Variable | Effect |
|---|---|
| `FLEET_GH_APP_OWNERS` | Comma list replacing the fleet App's owners |
| `FLEET_GH_DEVTODAY_OWNERS` | Comma list replacing the developing.today App's owners |
| `FLEET_GH_BACKUP_OWNERS` | Comma list replacing the backup App's owners |
| `FLEET_GH_APP_ID` | Overrides the fleet App's compiled-in App ID |
| `FLEET_GH_ORDER` | Comma list fixing the whole rotation exactly (grammar below) |
| `FLEET_GH_BIN` | The `gh` binary to run (default: `gh`) |
| `FLEET_GH_TRACE=1` | Name each attempt's token source on stderr |
| `FLEET_GH_RAW=1` | Raw mode (below); same as a leading `--raw` |

`FLEET_GH_ORDER` grammar: comma-separated entries, each either
`personal`, or `<prefix>:<owner>` where `<prefix>` is `app` (fleet
App), `devtoday`, or `backup`. Example:
`FLEET_GH_ORDER="backup:developing-today,personal"`. An entry that
matches no App prefix parses as `personal`. If the parsed list is
empty, the default rotation is used. Making this configuration
first-class (not env-only, not compiled-in) is code#10311.

## Owner hints: reordering, never adding

When the call names a repo or org — via `--repo`/`-R owner/repo`,
`--owner org`, an `api repos/<owner>/…` path, or a `github.com`
URL — that owner is the *hint*. Within **each** App leg, the hinted
owner's installation (if that owner is already in the leg's list)
moves to the front, because an installation token only sees its own
installation's scope.

A hint **never adds** an owner to a leg and never creates a new
leg. There is no dynamic minting for an unlisted org.

## Unknown and unlisted orgs: what actually happens

- `fleet gh` never mints a token dynamically for an org that is not
  in a leg's (possibly env-overridden) owner list.
- A call scoped to an unlisted org is therefore attempted under the
  personal token first, then under installation tokens scoped to
  *other* owners. An installation token cannot see the unlisted
  org, so a scoped call under a wrong-org leg fails with a
  permission/403-class refusal ("Resource not accessible by
  integration" and similar) — which is a pre-execution refusal, so
  the rotation fails over to the next leg rather than surfacing it
  immediately.
- The personal-first default **masks** this gap for an admin user:
  the personal token usually succeeds on the first attempt, so the
  missing App coverage for a new org is invisible until the personal
  leg is rate-limited or down. Adding a new org to the fleet means
  adding it to the relevant leg list (env override today; code
  change to the constants for a permanent addition) — nothing
  discovers it automatically.

## Retry safety (why a write is, or is not, repeated)

A **write** is repeated under a fresh token only when the failure
proves GitHub refused the request *before processing it*:
rate-limit rejections (the limiter answers before the mutation
runs) and permission/scope rejections (`INSUFFICIENT_SCOPES`,
"Resource not accessible by …", `Bad credentials`/401 shapes). Any
other failure of a write is returned as-is: the request may have
landed, and repeating it could double-post. **Reads** (the
whitelisted list/view/status shapes, and `gh api` calls with no
field flags and no non-GET method) repeat on any failure class.

## Exit codes

| Code | Meaning |
|---|---|
| `0` | The (final) `gh` attempt succeeded — or, for `fleet gh tiers`, every rotation source answered |
| *gh's own code* | The final attempt's exit code is propagated as-is |
| `2` | Usage error shape reserved in the domain (`EXIT_USAGE`) |
| `3` | **No token source could be resolved at all** — every source failed to resolve/mint and no `gh` attempt produced output. Also: `fleet gh tiers` exits 3 when *any* source is down, and a failure to spawn the `gh` binary at all returns 3. Exit 3 is a fleet-side credential/transport problem, never a GitHub answer about your request |

## Raw mode: the first-class bypass

`fleet gh --raw <gh args…>` (leading `--raw` only; later positions
belong to `gh`) or `FLEET_GH_RAW=1` execs the real `gh` directly
with the caller's ambient credentials and environment: no token is
resolved or injected, and there is no rotation or failover. Stdio
is inherited (not captured), so interactive `gh` behaves exactly as
unwrapped. Raw mode exists so callers that must pin one identity —
and the `gh` → `fleet gh` PATH shim, whose escape hatch this is —
never have to choose between the rotation and bypassing fleet
entirely.

## `fleet gh tiers` (shadow subcommand)

Real `gh` has no `tiers` verb, so `fleet gh tiers` is intercepted:
it probes every rotation source and prints each label with its core
and GraphQL rate-limit headroom (`<label>\tcore <remaining>/<limit>\tgql …`),
or `DOWN` with a value-free reason. Labels and numbers only — token
values never leave the provider. Exit 0 when every source answers,
3 when any source is down.

## Interim vs. permanent — the short list

Interim (owned by a tracked fix, do not treat as design):

- Per-invocation re-resolve / re-sign / re-list / re-mint cost —
  caching daemon: code#10742.
- Compiled-in leg lists with env-only overrides — configurable
  rotation: code#10311.

Permanent (the design, until a new decision replaces it):

- Secrets are referenced by name only and resolved through the
  broker-first, Bitwarden-fallback chain; values are never printed,
  logged, filed, or passed in argv.
- Minting is in-process; the private key reaches `openssl` only on
  an inherited fd.
- Writes repeat only on proven pre-execution refusals.
- Raw mode is a supported, first-class bypass, not a workaround.
