# desktop-commander Worker — self-hosted MCP endpoint (#13404)

The self-hosted replacement for the vendor's metered Remote Desktop
Commander relay. The vendor's hosted plan meters tool calls (free tier
10,000/month, Pro $20/month); this endpoint has no meter and keeps the
whole path inside the fleet's own Cloudflare account and hardware.

## Path of a request

```
AI client
  -> POST https://commander.security.cab/mcp
     Guarded by this Worker's OAuth 2.1 authorization server
     (src/oauth.ts). Hosted clients (Claude, ChatGPT) connect from
     their own servers and cannot pass a Cloudflare Access login, so
     /mcp takes a Bearer access token instead — or the machine bearer
     for the owner's own tooling.
  -> this Worker (src/index.ts): checks the Bearer, then proxies the
     streamable-HTTP MCP request, swapping the caller's credentials
     for its own bridge egress token.
  -> https://commander-link.security.cab/mcp
     Access app "desktop-commander link (bridge)": service-token-only
     policy (the atlas-search pattern). This door did not change.
  -> sandbox-link Cloudflare Tunnel -> http://localhost:8931 on amd
  -> supergateway (systemd user unit desktop-commander-bridge,
     pkgs/supergateway) in streamable-HTTP mode
  -> the packaged Desktop Commander (pkgs/desktop-commander, stdio)
```

`workers_dev` is off; `commander.security.cab` (a Worker route) is
the only public door.

## OAuth front door (src/oauth.ts)

Standard MCP authorization shape, so Claude and ChatGPT can connect:

- `GET /.well-known/oauth-protected-resource` (and `.../mcp`) and
  `GET /.well-known/oauth-authorization-server` — public discovery.
- `POST /register` — dynamic client registration (RFC 7591); public
  clients only (`token_endpoint_auth_method: "none"`), redirect URIs
  must be https or loopback http. Clients live in the `OAUTH_KV`
  namespace under `client:<id>`.
- `GET /authorize` — the approval page, and the ONE path still behind
  Cloudflare Access: the "desktop-commander" Access app is scoped to
  `commander.security.cab/authorize` with the owner-email policy only.
  The Worker also verifies the Access JWT itself (RS256 against the
  team JWKS, audience, owner email) before rendering. Approving
  issues a single-use code (10 min, PKCE S256 bound) via the form's
  POST back to `/authorize`.
- `POST /token` — authorization-code exchange (PKCE verified, code
  consumed on any attempt) issuing an opaque access token (1 h) and
  a rotating refresh token (30 d). Codes and tokens are stored in KV
  under their SHA-256 (`code:<hash>`, `at:<hash>`, `rt:<hash>`),
  never in the clear. Revoke a client by deleting its `at:`/`rt:`
  entries (and its `client:` record) from the namespace.
- `/mcp` without a valid Bearer answers 401 with
  `WWW-Authenticate: Bearer resource_metadata="https://commander.security.cab/.well-known/oauth-protected-resource"`,
  which is what sends hosted clients into the OAuth flow.

The public Access service token that used to guard the whole hostname
is retired at this door (the token still exists; nothing accepts it
here anymore). The bridge door's service token is unchanged.

## Secrets — by name only, never values

| Where | Name |
| --- | --- |
| fleet secrets (Bitwarden) | `cloudflare.access.desktop-commander-link-client-id` / `-client-secret` — bridge-door token the Worker presents |
| fleet secrets (Bitwarden) | `cloudflare.access.desktop-commander-client-id` / `-client-secret` — the retired public-door token (kept, unused here) |
| fleet secrets (Bitwarden) | `cloudflare.api-token-desktop-commander-worker` — scoped deploy token |
| fleet secrets (Bitwarden) | `desktop-commander.machine-bearer` — the /mcp machine bearer |
| GitHub environment `desktop-commander-production` | `DESKTOP_COMMANDER_WORKER_CF_TOKEN` (same deploy token), `BRIDGE_ACCESS_CLIENT_ID`, `BRIDGE_ACCESS_CLIENT_SECRET`, `MACHINE_BEARER` |
| Worker version secrets | `ACCESS_CLIENT_ID`, `ACCESS_CLIENT_SECRET`, `MACHINE_BEARER` — uploaded per version by the publish workflow from the environment secrets |

## Deploys — cf version deployments via GitHub Actions

Modeled on the id-mail publisher/promoter pair:

1. **Publish** (`desktop-commander-worker-publish.yml`, manual dispatch
   on main, `confirm_upload`): tests, a credential-free dry run, then
   `upload-version.mjs` runs `cf workers versions create` with the
   Worker secrets file. Uploading changes no traffic. The run emits a
   provenance artifact naming the exact version, tag, and baseline.
2. **Promote** (`desktop-commander-worker-promote.yml`, manual dispatch
   with the publish run id + attempt, `confirm_promotion`):
   `verify-upload-run.mjs` vouches the run was a successful manual
   publish on main, then `promote.mjs` checks the live deployment
   still matches the manifest baseline and creates a deployment with
   exactly that version at 100%.

Tests (`desktop-commander-worker-tests.yml`) run on pushes and PRs
touching this directory.

## Laptop side

- `desktop-commander-bridge.service` (user unit; defined in
  `home/common/default.nix` next to `desktop-commander-remote`):
  supergateway wrapping the packaged Desktop Commander on
  `127.0.0.1:8931`, path `/mcp`, stateful sessions, `/healthz` for
  gateway health.
- The vendor device agent (`desktop-commander-remote.service`) stays
  running until this endpoint is verified end to end; standing it down
  is a separate recorded step.

## Client setup

- Hosted clients (Claude, ChatGPT): add the custom connector
  `https://commander.security.cab/mcp`; the client registers itself
  and sends the owner through the Access sign-in + approval page on
  first connection.
- Header-capable clients (Claude Code, Cursor, mcpx): send
  `Authorization: Bearer <machine bearer>` (fleet secrets:
  `desktop-commander.machine-bearer`).
