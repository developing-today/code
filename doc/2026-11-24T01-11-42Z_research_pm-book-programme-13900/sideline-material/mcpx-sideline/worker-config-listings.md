# mcpx Worker — config listings (verbatim from pkgs/mcpx/worker, surveyed 2026-11-23)

Secret values appear nowhere below: wrangler.toml carries names and comments only.

## wrangler.toml

```toml
# mcpx Worker (#13657). The self-hosted replacement for the
# vendor's metered Remote mcpx relay: an OAuth-fronted MCP
# door that proxies to the mcpx bridge on the laptop.
#
# workers_dev is OFF: the only door is the custom domain
# mcpx.security.cab. Hosted MCP clients (Claude, ChatGPT) cannot
# pass a Cloudflare Access login at /mcp, so the Worker runs an OAuth
# 2.1 authorization server (src/oauth.ts): public discovery +
# registration, /token with PKCE, and /authorize — the one path still
# behind a Cloudflare Access app scoped to mcpx.security.cab/
# authorize (owner Google identity only). /mcp accepts the OAuth
# Bearer it issues, or the machine bearer the owner's
# tooling.
#
# Secrets (uploaded per version with --secrets-file at publish time;
# values never in this file):
#   ACCESS_CLIENT_ID      Cloudflare Access service token for the
#   ACCESS_CLIENT_SECRET  bridge door, mcpx-link.security.cab
#                         (fleet secrets:
#                          cloudflare.access.mcpx-link-client-id
#                          cloudflare.access.mcpx-link-client-secret)
#   MACHINE_BEARER        Bearer the owner's machine MCP clients
#                         (fleet secrets: mcpx.machine-bearer)
name = "mcpx"
main = "src/index.ts"
compatibility_date = "2025-09-15"
workers_dev = false

[vars]
BRIDGE_BASE = "https://mcpx-link.security.cab"
ACCESS_TEAM_DOMAIN = "snowy-sun-58a9.cloudflareaccess.com"
# Audience of the Access app scoped to /authorize (the Worker verifies
# it on the Access JWT before rendering the approval page).
ACCESS_AUD = "b34b9764fb317774bf5c0ff0ac9cbd8a2ab80ed346d06f08d74967011bac640e"

[[kv_namespaces]]
binding = "OAUTH_KV"
id = "11eb93dc9fa24087ae57b12483dc6172"
```

(Bearer-token redactions in the original survey output were display redactions
of the words in comments; the file itself contains no token values.)

## config.json

```json
{ "compatibilityDate": "2025-09-15", "routes": ["mcpx.security.cab"], "worker": "mcpx", "workersDev": false }
```

## package.json

```json
{
  "description": "Self-hosted mcpx MCP endpoint: Access-gated Worker front door proxying to the laptop bridge (developing-today/code#13657).",
  "devDependencies": {
    "@cloudflare/workers-types": "^5.20261019.1",
    "cf": "1.0.0-beta.20",
    "typescript": "^5.6.0",
    "vitest": "^4.1.0",
    "wrangler": "4.152.0"
  },
  "name": "mcpx-worker",
  "private": true,
  "scripts": {
    "deploy": "cf workers versions create",
    "test": "vitest run",
    "typecheck": "tsc --noEmit"
  },
  "type": "module",
  "version": "0.1.0"
}
```

## Env interface (src/index.ts)

```ts
export interface Env extends OAuthEnv {
  BRIDGE_BASE: string;
  ACCESS_CLIENT_ID: string;
  ACCESS_CLIENT_SECRET: string; // (display-redacted in survey output)
  MACHINE_BEARER: string; // Worker secret; canonical value in fleet
  // secrets as mcpx.machine-bearer; staged by
  // the publish workflow from the GitHub
  // environment secret of the same name
}
```

## OAuth env interface (src/oauth.ts)

```ts
export interface OAuthEnv {
  OAUTH_KV: KVNamespace;
  ACCESS_AUD: string; // AUD of the Access application scoped to /authorize
  ACCESS_TEAM_DOMAIN: string; // team domain hosting the JWKS
}
```
