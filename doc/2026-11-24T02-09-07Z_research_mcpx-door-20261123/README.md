# mcpx Worker — public MCP endpoint (#13657)

The public front door for mcpx at `mcpx.security.cab`, mirroring the
proven desktop-commander Worker build (#13404). Hosted MCP clients
(Claude, ChatGPT) and the owner's own tooling reach the laptop's mcpx
daemon through this Worker; nothing else on the laptop is exposed.

## Path of a request

```
AI client
  -> POST https://mcpx.security.cab/mcp   (bare https://mcpx.security.cab/ is identical)
     Guarded by this Worker's OAuth 2.1 authorization server
     (src/oauth.ts). Hosted clients connect from their own servers
     and cannot pass a Cloudflare Access login, so /mcp takes a
     Bearer token instead — or the machine bearer for the owner's
     own tooling.
  -> this Worker (src/index.ts): checks the Bearer, computes the
     filter selector (src/filter.ts), then proxies the streamable-HTTP
     MCP request, swapping the caller's credentials for its own
     bridge egress token.
  -> https://mcpx-link.security.cab/mcp
     Access app "mcpx link (bridge)": service-token-only policy.
  -> sandbox-link Cloudflare Tunnel -> http://localhost:8932 on amd
  -> the mcpx bridge proxy on 127.0.0.1:8932
  -> the mcpx daemon's /mcp endpoint, discovered via the daemon's
     state descriptor
```

`workers_dev` is off; `mcpx.security.cab` (a Worker route) is the
only public door.

## Origin lockdown

The mcpx daemon listens on loopback only and is unauthenticated —
it trusts its only caller, the local bridge proxy. This door is the
security boundary: it exposes `/mcp` (and its bare-path twin `/`)
only, behind OAuth/machine-bearer auth. The daemon's `/v1/*` API is
never proxied and has no public path.

## OAuth front door (src/oauth.ts)

Standard MCP authorization shape, so Claude and ChatGPT can connect:

- `GET /.well-known/oauth-protected-resource` (and `.../mcp`) and
  `GET /.well-known/oauth-authorization-server` — public discovery.
- `POST /register` — dynamic client registration (RFC 7591); public
  clients only (`token_endpoint_auth_method: "none"`), redirect URIs
  must be https or loopback http. Clients live in the `OAUTH_KV`
  namespace under `client:<id>`.
- `GET /authorize` — the approval page, and the ONE path still behind
  Cloudflare Access: the "mcpx" Access app is scoped to
  `mcpx.security.cab/authorize` with the owner-email policy only
  (Google SSO identity). The Worker also verifies the Access JWT
  itself (RS256 against the team JWKS, audience, owner email) before
  rendering. Approving issues a single-use code (10 min, PKCE S256
  bound) via the form's POST back to `/authorize`.
- `POST /token` — authorization-code exchange (PKCE verified, code
  consumed on any attempt) issuing an opaque access token (1 h) and
  a rotating refresh token (30 d). Codes and tokens are stored in KV
  under their SHA-256 (`code:<hash>`, `at:<hash>`, `rt:<hash>`),
  never in the clear. Revoke a client by deleting its `at:`/`rt:`
  entries (and its `client:` record) from the namespace.
- `/mcp` (and `/`) without a valid Bearer returns 401 with
  `WWW-Authenticate: Bearer resource_metadata="https://mcpx.security.cab/.well-known/oauth-protected-resource"`,
  which is what sends hosted clients into the OAuth flow.

## Filtering (src/filter.ts)

Architecture in place; enforcement spec pending on #13657.

- A per-request selector is parsed from the `ns` query parameter in
  exactly one place (`resolveFilterSelector` in src/filter.ts),
  computed by the router and passed to the proxy/filter layer.
- No selector means the named conservative default profile
  (`DEFAULT_FILTER_PROFILE = "conservative"`).
- `ns` is consumed by this door and stripped before proxying; all
  other query parameters pass through untouched.
- Until the enforcement spec lands, `filterMcpResponse` is a
  pass-through: the selector is threaded end to end so enforcement
  can be added in src/filter.ts without touching the router.

## Secrets — by name only, never values

| Where                                | Name                                                                                                                                     |
| ------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------- |
| fleet secrets (Bitwarden)            | `cloudflare.access.mcpx-link-client-id` / `-client-secret` — bridge-door token the Worker presents                                       |
| fleet secrets (Bitwarden)            | `cloudflare.api-token-mcpx-worker` — scoped deploy token                                                                                 |
| fleet secrets (Bitwarden)            | `mcpx.machine-bearer` — the /mcp machine bearer                                                                                          |
| fleet secrets (Bitwarden)            | `cloudflare.tunnel.sandbox-link-token` — the sandbox-link tunnel the bridge door rides                                                   |
| GitHub environment `mcpx-production` | `MCPX_WORKER_CF_TOKEN` (same deploy token), `BRIDGE_ACCESS_CLIENT_ID`, `BRIDGE_ACCESS_CLIENT_SECRET`, `MACHINE_BEARER`                   |
| Worker version secrets               | `ACCESS_CLIENT_ID`, `ACCESS_CLIENT_SECRET`, `MACHINE_BEARER` — uploaded per version by the publish workflow from the environment secrets |

## Deploys — cf version deployments via GitHub Actions

Modeled on the desktop-commander / id-mail publisher-promoter pair:

1. **Publish** (`mcpx-worker-publish.yml`, manual dispatch on main,
   `confirm_upload`): tests, a credential-free dry run, then
   `upload-version.mjs` runs `cf workers versions create` with the
   Worker secrets file. Uploading changes no traffic. The run emits
   a provenance artifact (`mcpx-worker-upload-<run_id>-<run_attempt>`)
   naming the exact version, tag (`mcpx-<sha12>-run-<id>-attempt-<n>`),
   and baseline.
2. **Promote** (`mcpx-worker-promote.yml`, manual dispatch with the
   publish run id + attempt, `confirm_promotion`):
   `verify-upload-run.mjs` vouches the run was a successful manual
   publish on main, then `promote.mjs` checks the live deployment
   still matches the manifest baseline (or already serves exactly
   this version — an idempotent no-op) and creates a deployment with
   exactly that version at 100%.

Tests (`mcpx-worker-tests.yml`) run on pushes and PRs touching this
directory.

## Client setup

- Hosted clients (Claude, ChatGPT): add the custom connector
  `https://mcpx.security.cab/mcp`; the client registers itself and
  sends the owner through the Access sign-in + approval page on
  first connection.
- Header-capable clients (Claude Code, Cursor, other mcpx clients):
  send `Authorization: Bearer <machine bearer>` (fleet secrets:
  `mcpx.machine-bearer`).
