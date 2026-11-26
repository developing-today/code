# The Commander door: Desktop Commander over MCP

The Commander door is the fleet's public path to the Desktop Commander
MCP server running on the laptop. Desktop Commander gives an AI client
terminal and filesystem access to the machine it runs on; the vendor
sells that reach as a metered hosted relay. The fleet instead runs its
own unmetered path, end to end on its own Cloudflare account and its own
hardware: a Cloudflare Worker holds the public door, a second,
service-token-only door guards the bridge into the laptop, a Cloudflare
Tunnel carries the traffic, and a small gateway on the laptop turns the
HTTP stream back into the packaged server's stdio.

Tracking issue: developing-today/code#13404. The Worker source is
`pkgs/desktop-commander/worker/`; the packaged server is
`pkgs/desktop-commander`; the laptop unit is defined in
`home/common/default.nix`.

Source: https://github.com/developing-today/code/blob/main/pkgs/desktop-commander/worker/src/index.ts

## The chain, hop by hop

```text
AI client
  -> POST https://commander.security.cab/mcp          (the public door)
     the desktop-commander Worker: OAuth 2.1 / machine-bearer gate
  -> POST https://commander-link.security.cab/mcp     (the bridge door)
     Cloudflare Access app: service-token-only policy
  -> sandbox-link Cloudflare Tunnel
  -> http://127.0.0.1:8931/mcp on the laptop
     supergateway (desktop-commander-bridge user unit), streamable HTTP
  -> the packaged Desktop Commander (stdio), version 0.2.52
```

1. **The client** speaks streamable-HTTP MCP to
   `https://commander.security.cab/mcp`. Hosted clients (Claude,
   ChatGPT) connect from their own servers; header-capable clients
   (Claude Code, Cursor, mcpx, and the fleet's own `fleet commander`)
   connect directly with a bearer token.
2. **The public-door Worker** (`desktop-commander`,
   `pkgs/desktop-commander/worker/src/index.ts`) checks the bearer,
   then proxies the request transparently: method, path, query, headers
   and body pass through, and the response — including the
   `Mcp-Session-Id` header and any SSE stream — passes back untouched.
   Before proxying it deletes the caller's own credentials
   (`authorization`, `cf-access-jwt-assertion`,
   `cf-access-client-id`, `cf-access-client-secret`) and substitutes
   its own bridge-door service token. `workers_dev` is off: the custom
   domain is the only public door.
3. **The bridge door** at `commander-link.security.cab` is a Cloudflare
   Access application with a service-token-only policy (the atlas-search
   pattern). Only the Worker holds that token; nothing else can present
   it, so the bridge door accepts the Worker's egress and nothing else.
4. **The tunnel** is the existing sandbox-link Cloudflare Tunnel: it
   terminates the bridge door on the laptop at `http://localhost:8931`.
5. **supergateway** (the `desktop-commander-bridge` systemd user unit in
   `home/common/default.nix`, wrapping `pkgs/supergateway`) serves the
   packaged Desktop Commander in streamable-HTTP mode on
   `127.0.0.1:8931`, path `/mcp`, stateful sessions, with `/healthz` for
   gateway health.
6. **The packaged Desktop Commander** (`pkgs/desktop-commander`,
   `@wonderwhy-er/desktop-commander` version 0.2.52, built from the npm
   registry tarball) runs as a stdio MCP server under the gateway. It is
   the same package the vendor device agent runs; only the transport
   and the door in front of it are the fleet's.

The fleet binary's own client for this door — `fleet commander health`
and `fleet commander exec`, tier T1 of the laptop route forms — is
covered in [fleet laptop route forms](../reference/laptop-route-forms.md).
This chapter is about the door itself, not that client.

## Auth at each hop

| Hop                                | Who is checked     | How                                                                                                                                                                                                                                                                                                                              |
| ---------------------------------- | ------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Client → public door, `/mcp`       | The calling client | A Bearer token the Worker accepts: an OAuth 2.1 access token it issued, or the machine bearer (see below). Anything else gets a 401 (see Failure shapes).                                                                                                                                                                        |
| Client → public door, `/authorize` | The owner          | The one path still behind Cloudflare Access: the `desktop-commander` Access app is scoped to `commander.security.cab/authorize` with an owner-Google-identity policy only. The Worker also verifies the Access JWT itself — RS256 against the team JWKS, the app audience, the owner email — before rendering the approval page. |
| Worker → bridge door               | The Worker         | The Access service token for `commander-link.security.cab`, presented as `CF-Access-Client-Id` / `CF-Access-Client-Secret` headers, swapped in by the Worker in place of the caller's credentials.                                                                                                                               |
| Tunnel → laptop                    | Nobody new         | The tunnel and the gateway add no auth of their own; the gateway binds `127.0.0.1` only, so the tunnel is the only way in. Every earlier hop has already authenticated the request.                                                                                                                                              |

### The OAuth 2.1 front door

The Worker runs its own OAuth 2.1 authorization server
(`src/oauth.ts`), because hosted clients cannot pass a Cloudflare
Access login at `/mcp` — they arrive from their vendors' servers, not
from a browser the owner is signed into. It landed in PR #13488,
superseding the original #13404 design in which the whole hostname sat
behind Access and machine clients used a public-door service token;
that public-door token is retired at this door (the token still exists;
nothing accepts it here any more). The bridge door did not change.

- **Discovery (public):** `GET /.well-known/oauth-protected-resource`
  (also under `/mcp`) and `GET /.well-known/oauth-authorization-server`.
  Issuer `https://commander.security.cab`, resource
  `https://commander.security.cab/mcp`, scope `commander`.
- **Registration (public):** `POST /register` (RFC 7591). Public clients
  only (`token_endpoint_auth_method: "none"`); redirect URIs must be
  https or loopback http. Clients are stored in the `OAUTH_KV`
  namespace under `client:<id>`.
- **Authorize:** `GET /authorize` renders the approval page (behind
  Access, above); approving is a form POST back to `/authorize` that
  issues a single-use authorization code, valid 10 minutes and bound to
  the client's PKCE S256 challenge.
- **Token:** `POST /token` exchanges the code (PKCE verified; a code is
  consumed on any attempt, successful or not) for an opaque access
  token valid 1 hour and a rotating refresh token valid 30 days.
- **State:** codes, access tokens and refresh tokens are stored in
  `OAUTH_KV` under the SHA-256 of the presented value (`code:<hash>`,
  `at:<hash>`, `rt:<hash>`), never in the clear. Revoke a client by
  deleting its `at:` / `rt:` entries and its `client:` record from the
  namespace.

### The machine bearer

The owner's own tooling skips the OAuth flow with a single static
bearer token, compared in constant time. Its canonical value lives in
fleet secrets under the name `desktop-commander.machine-bearer`; the
Worker receives it as the per-version secret `MACHINE_BEARER`, staged
by the publish workflow from the GitHub environment secret of the same
name. `fleet commander` resolves it by that fleet-secret name at call
time and never stores it.

## Configuration and secrets reference

Every name below was verified by grepping the repository — the Worker
config, its source, and the home configuration that runs the laptop
half.

### Worker configuration (`pkgs/desktop-commander/worker/wrangler.toml`)

| Setting                         | Value                                                              | Meaning                                                                                |
| ------------------------------- | ------------------------------------------------------------------ | -------------------------------------------------------------------------------------- |
| Worker name                     | `desktop-commander`                                                | The Worker that holds the public door.                                                 |
| `workers_dev`                   | `false`                                                            | No `workers.dev` door; the custom domain is the only door.                             |
| Route                           | `commander.security.cab`                                           | From `worker/config.json`: the Worker's custom domain.                                 |
| `BRIDGE_BASE` (var)             | `https://commander-link.security.cab`                              | The bridge door the Worker proxies to.                                                 |
| `ACCESS_TEAM_DOMAIN` (var)      | `snowy-sun-58a9.cloudflareaccess.com`                              | The Access team domain hosting the JWKS the Worker verifies `/authorize` JWTs against. |
| `ACCESS_AUD` (var)              | `c6ddf0a5c0ba8742c0a3c74d12495bf5c4fe9bf6f01f604d62583cb2ed353879` | Audience of the Access app scoped to `/authorize`.                                     |
| `OAUTH_KV` (KV binding)         | namespace id `ea675d3c7d5b4f3b9f1cecae82e57d1b`                    | OAuth clients, codes and tokens, keyed by SHA-256 as described above.                  |
| `ACCESS_CLIENT_ID` (secret)     | —                                                                  | Bridge-door service token id, uploaded per version at publish time.                    |
| `ACCESS_CLIENT_SECRET` (secret) | —                                                                  | Bridge-door service token secret, uploaded per version at publish time.                |
| `MACHINE_BEARER` (secret)       | —                                                                  | The machine bearer for `/mcp`, uploaded per version at publish time.                   |

### Secrets, by name only

| Name                                                                                                            | Store                                             | Used by                                                                                                                                           |
| --------------------------------------------------------------------------------------------------------------- | ------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| `cloudflare.access.desktop-commander-link-client-id` / `cloudflare.access.desktop-commander-link-client-secret` | fleet secrets                                     | The bridge-door service token the Worker presents.                                                                                                |
| `desktop-commander.machine-bearer`                                                                              | fleet secrets                                     | The `/mcp` machine bearer; `fleet commander` resolves it by this name.                                                                            |
| `cloudflare.api-token-desktop-commander-worker`                                                                 | fleet secrets                                     | The scoped Cloudflare deploy token for the publish workflow.                                                                                      |
| `cloudflare.access.desktop-commander-client-id` / `cloudflare.access.desktop-commander-client-secret`           | fleet secrets                                     | The retired public-door service token pair. Kept in the vault; nothing accepts it at the public door any more.                                    |
| `DESKTOP_COMMANDER_WORKER_CF_TOKEN`, `BRIDGE_ACCESS_CLIENT_ID`, `BRIDGE_ACCESS_CLIENT_SECRET`, `MACHINE_BEARER` | GitHub environment `desktop-commander-production` | The publish workflow's copies of the deploy token, the bridge-door token, and the machine bearer, uploaded to each Worker version as its secrets. |

### Laptop configuration (`home/common/default.nix`)

- `desktop-commander-bridge.service` (systemd user unit): supergateway
  wrapping the packaged Desktop Commander —
  `--outputTransport streamableHttp --streamableHttpPath /mcp --port
8931 --host 127.0.0.1 --stateful --healthEndpoint /healthz`, restart
  on failure. Its start script exports the real user PATH first,
  because the wrapped server exists to run the user's commands.
- `desktop-commander-remote.service` (systemd user unit): the vendor
  device agent (`desktop-commander remote`). It stays running until the
  self-hosted endpoint is stood down as a separate recorded step; the
  two paths coexist deliberately.

## Failure shapes

| Symptom                                                                                                                                                                      | Where it stops                  | What it means                                                                                                                                                                                                                                                                                                                                            |
| ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `401 {"error":"unauthorized"}` from `/mcp`, with a `WWW-Authenticate: Bearer resource_metadata="https://commander.security.cab/.well-known/oauth-protected-resource"` header | Public door                     | No bearer, an expired OAuth access token (1 hour), a revoked token, or a wrong machine bearer. The header is the signal that sends hosted clients into the OAuth flow; header-capable clients should check the machine bearer's fleet-secret name resolves.                                                                                              |
| A redirect to a Cloudflare Access login at `/authorize`                                                                                                                      | Public door, approval path only | Expected for anyone who is not the owner: only the owner's Google identity passes, and the Worker re-verifies the Access JWT (signature, audience, owner email) before rendering the page. A login wall at `/mcp` itself is **not** this shape — `/mcp` is not behind Access any more; if a client reports one, it is holding a pre-OAuth configuration. |
| `500 {"error":"bridge credentials are not configured"}` from `/mcp`                                                                                                          | Public door                     | The live Worker version was uploaded without its `ACCESS_CLIENT_ID` / `ACCESS_CLIENT_SECRET` version secrets. Re-run the publish/promote pipeline so the version carries them.                                                                                                                                                                           |
| `404 {"error":"not found"}`                                                                                                                                                  | Public door                     | A path other than `/mcp`, `/healthz` and the OAuth surface. A wrong hostname (for example the `workers.dev` address) lands here or nowhere — `workers_dev` is off.                                                                                                                                                                                       |
| `405 {"error":"method not allowed"}`                                                                                                                                         | Public door                     | A non-POST to `/register` or `/token`, or a method other than GET/POST to `/authorize`.                                                                                                                                                                                                                                                                  |
| An Access refusal or 403 between the Worker and the bridge door                                                                                                              | Bridge door                     | The bridge-door service token is wrong, rotated without re-publishing the Worker version, or the Access app's service-token policy changed. The token's fleet-secret names are the `desktop-commander-link` pair above.                                                                                                                                  |
| The Worker answers, the bridge door passes, but calls time out or 502                                                                                                        | Tunnel or laptop                | The sandbox-link tunnel is down, or the laptop half is stopped: check `systemctl --user status desktop-commander-bridge` and the gateway's own `http://127.0.0.1:8931/healthz` on the laptop.                                                                                                                                                            |
| MCP session errors mid-stream (lost `Mcp-Session-Id`, cut SSE)                                                                                                               | Laptop gateway                  | The gateway runs stateful sessions; a unit restart drops every live session. Clients reconnect and re-initialize.                                                                                                                                                                                                                                        |
| Markdown-to-PDF tool calls fail on the laptop                                                                                                                                | Packaged server                 | A packaging limit, not a door failure: the package is built with `--ignore-scripts`, so the Chromium its PDF path downloads never arrives. Recorded in `pkgs/desktop-commander/default.nix`.                                                                                                                                                             |

## Operations

- **Health:** `GET https://commander.security.cab/healthz` answers
  `{"ok":true,"service":"desktop-commander"}` with no token — it proves
  the Worker answers, nothing about the hops behind it. End-to-end
  health is an MCP `initialize` + `tools/list` against `/mcp` with a
  valid bearer (`fleet commander health` does exactly this through the
  whole chain), and the laptop-local check is the gateway's
  `http://127.0.0.1:8931/healthz`.
- **Deploys** are Cloudflare version deployments from GitHub Actions,
  modeled on the id-mail publisher/promoter pair:
  1. **Publish** (`desktop-commander-worker-publish.yml`, manual
     dispatch on `main`, `confirm_upload`): runs the Worker's tests
     (`pkgs/desktop-commander/worker/test/` — proxy, OAuth and promote
     suites), a credential-free dry run, then uploads a new version
     with its secrets file. Uploading changes no traffic. The run emits
     a provenance artifact naming the exact version, tag and baseline.
  2. **Promote** (`desktop-commander-worker-promote.yml`, manual
     dispatch with the publish run id and attempt, `confirm_promotion`):
     verifies the run was a successful manual publish on `main`, checks
     the live deployment still matches the manifest baseline, then
     creates a deployment with exactly that version at 100%.
     Tests (`desktop-commander-worker-tests.yml`) also run on pushes and
     PRs touching the Worker directory.
- **Token rotation:** rotate the bridge-door service token or the
  machine bearer in fleet secrets / the `desktop-commander-production`
  environment, then publish and promote a new Worker version — version
  secrets are per-version, so a rotation only takes effect on a version
  uploaded with the new values. Rotating the machine bearer also means
  every machine client picks the new value up by name on its next call;
  nothing caches it.
- **Revoking a hosted client:** delete its `at:<hash>` / `rt:<hash>`
  entries (and its `client:` record) from the `OAUTH_KV` namespace.
  There is no admin UI; the namespace is the record.
- **The vendor agent stand-down** is a separate recorded step on
  #13404, deliberately not part of any deploy: `desktop-commander-remote`
  keeps running alongside the self-hosted door until it is taken.

## Related items: priorities and dependencies

Prioritised as of 2026-11-23, from the issue record. The door itself is
built and live; what remains open is bookkeeping and one deliberate
operational step.

| Priority | Item                                                                                          | State and dependency                                                                                                                                                                                                                                |
| -------- | --------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| P1       | #13404 — the tracking issue for this door                                                     | Open. Its body still describes the original all-Access design; the OAuth landing (next row) superseded the public-door half of it. The vendor-agent stand-down is recorded here as the remaining step.                                              |
| P1       | #13488 — OAuth 2.1 front door (PR)                                                            | Merged 2026-11-23, merge commit `4421467c3776`. The dependency everything hosted hangs on: without it, Claude and ChatGPT cannot connect at all.                                                                                                    |
| P2       | #13427 — the original endpoint build (PR)                                                     | Merged. Worker, bridge package and the publish/promote pipeline; the foundation #13488 extended.                                                                                                                                                    |
| P2       | #13654 — `fleet commander` subcommand (proposal)                                              | Open, but the subcommand exists: `fleet commander health` / `exec` ship as tier T1 of the route forms (#13717) and are documented in [fleet laptop route forms](../reference/laptop-route-forms.md). The issue reads as implemented-but-not-closed. |
| P3       | #13699 — restore string delimiters on `desktop-commander-bridge-start`                        | Open, but the defect as described is not present at current `main`: the block in `home/common/default.nix` carries both delimiters, and the sibling fix #13716 is closed. Reads as a stale duplicate.                                               |
| P3       | The publish/promote fix chain: #13429, #13441, #13443, #13447, #13453, #13473, #13478, #13479 | All closed. The versions-flow sharp edges (dev-server declaration, first-publish baseline, version resolution by tag, no-op promotes) are already paid down; the pipeline has run green end to end (#13404, 2026-11-23).                            |

Dependency shape, in one line: the packaged server and the laptop unit
(#13427) → the Worker door and its pipeline (#13427) → the OAuth front
(#13488) → hosted clients; the fleet client (#13654 via #13717) depends
only on the door and the machine bearer's secret name, not on OAuth.
