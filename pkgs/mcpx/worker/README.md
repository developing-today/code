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

The mcpx upstream offers no per-client hooks (`X-Mcpx-Settings` is
ignored on `/mcp`, and there is no per-session state), so the Worker
enforces exposure itself, on both requests and responses. Enforcement
is a pure function of the request URL selector and the message; nothing
is keyed to `Mcp-Session-Id`.

The client picks a profile with the `ns` query parameter on `/mcp` (or
`/`, identical handling):

| `ns`   | Profile        | Selected set               |
| ------ | -------------- | -------------------------- |
| absent | `conservative` | empty                      |
| `a,b`  | `selected`     | `{a, b}`                   |
| `*`    | `full`         | pass-through, no filtering |

`ns` is consumed by the door and stripped before proxying, and any
caller `codemode` parameter is dropped: only the code-mode surface is
offered in v1.

Enforcement in the conservative and selected profiles:

- `tools/list` responses expose exactly `mcpx_namespaces`,
  `mcpx_catalog`, `mcpx_types`, `mcpx_search`; the selected profile
  adds `mcpx_call` and `mcpx_batch`. `mcpx_exec`,
  `mcpx_tool_invoke` and every other `mcpx_*` extra are never listed.
  Pages are filtered as they arrive with the upstream cursor passed
  through.
- `tools/call` requests are checked before anything reaches upstream; a
  disallowed call returns HTTP 200 with JSON-RPC error `-32602`
  ("not exposed by this door's selector").
  - `mcpx_namespaces` is allowed and its response is filtered to the
    selected set (empty in the conservative profile).
  - `mcpx_catalog` and `mcpx_search` are allowed with unfiltered
    responses (see limitations).
  - `mcpx_types` is allowed only when every requested namespace is in
    the selected set, so the conservative profile rejects it.
  - `mcpx_call` (selected only) resolves `namespace`/`tool` with mcpx's
    own dotted-name rule and allows only in-set namespaces.
  - `mcpx_batch` / `batch_call` (selected only) is allowed only when
    every item resolves in-set and none targets a gateway tool
    (`mcpx_*`, `batch_call`); one violation rejects the whole batch.
  - Everything else, including `mcpx_exec` and `mcpx_tool_invoke`, is
    rejected in both profiles.
- Batched JSON-RPC arrays are checked element by element; rejected
  elements get their own error object while allowed elements proceed.
- `prompts/list` and `resources/list` responses drop entries whose
  `mcpx://<ns>/` URI is out of set; `prompts/get` and `resources/read`
  for such a URI are rejected.
- `initialize`, notifications, `ping` and other methods pass through.

Responses are handled in both framings (JSON and SSE `data:` lines) and
re-emitted unchanged in kind.

v1 limitations: `mcpx_catalog` and `mcpx_search` responses are
metadata and stay unfiltered; the flat code-mode surface is not
offered. These profiles are the door-side stand-in for per-client
capability ceilings — once daemon-side auth lands (#13659, aligned
with #8299) the `ns` selector maps onto a ceiling instead of
Worker-side filtering.

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
