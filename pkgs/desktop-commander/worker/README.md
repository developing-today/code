# desktop-commander Worker — self-hosted MCP endpoint (#13404)

The self-hosted replacement for the vendor's metered Remote Desktop
Commander relay. The vendor's hosted plan meters tool calls (free tier
10,000/month, Pro $20/month); this endpoint has no meter and keeps the
whole path inside the fleet's own Cloudflare account and hardware.

## Path of a request

```
AI client
  -> POST https://commander.security.cab/mcp
     Access app "desktop-commander": owner Google identity (the IdP
     wired under #12412 with the client saved under #12417), or the
     "desktop-commander" service token for machine clients.
  -> this Worker (src/index.ts): refuses requests without an Access
     JWT assertion, then proxies the streamable-HTTP MCP request,
     swapping the caller's Access material for its own egress token.
  -> https://commander-link.security.cab/mcp
     Access app "desktop-commander link (bridge)": service-token-only
     policy (the atlas-search pattern).
  -> sandbox-link Cloudflare Tunnel -> http://localhost:8931 on amd
  -> supergateway (systemd user unit desktop-commander-bridge,
     pkgs/supergateway) in streamable-HTTP mode
  -> the packaged Desktop Commander (pkgs/desktop-commander, stdio)
```

`workers_dev` is off; `commander.security.cab` (a Worker custom
domain) is the only public door.

## Secrets — by name only, never values

| Where | Name |
| --- | --- |
| fleet secrets (Bitwarden) | `cloudflare.access.desktop-commander-link-client-id` / `-client-secret` — bridge-door token the Worker presents |
| fleet secrets (Bitwarden) | `cloudflare.access.desktop-commander-client-id` / `-client-secret` — public-door token for machine MCP clients |
| fleet secrets (Bitwarden) | `cloudflare.api-token-desktop-commander-worker` — scoped deploy token |
| GitHub environment `desktop-commander-production` | `DESKTOP_COMMANDER_WORKER_CF_TOKEN` (same deploy token), `BRIDGE_ACCESS_CLIENT_ID`, `BRIDGE_ACCESS_CLIENT_SECRET` |
| Worker version secrets | `ACCESS_CLIENT_ID`, `ACCESS_CLIENT_SECRET` — uploaded per version by the publish workflow from the environment secrets |

## Deploys — cf version deployments via GitHub Actions

Modeled on the id-mail publisher/promoter pair:

1. **Publish** (`desktop-commander-worker-publish.yml`, manual dispatch
   on main, `confirm_upload`): tests, a credential-free dry run, then
   `upload-version.mjs` runs `cf workers versions create` with the
   bridge secrets file. Uploading changes no traffic. The run emits a
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

- Header-capable clients (Claude Code, Cursor, mcpx): send
  `CF-Access-Client-Id` / `CF-Access-Client-Secret` with the
  public-door token values.
- Browser clients: sign in with the owner's Google identity at the
  Access prompt.
