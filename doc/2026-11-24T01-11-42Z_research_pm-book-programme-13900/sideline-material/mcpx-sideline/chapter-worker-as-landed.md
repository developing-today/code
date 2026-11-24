# The door Worker at mcpx.security.cab

mcpx normally lives on one machine: the daemon listens on loopback and a
unix socket, and every local client reaches it there. The door Worker is
the one public way in. It is a Cloudflare Worker, served only at
`mcpx.security.cab`, that stands in front of the laptop's mcpx daemon,
checks who is calling, decides how much of mcpx that caller may see —
that decision is the `ns` selector, below — and proxies what survives
to the bridge on the laptop. It is the self-hosted replacement for a
vendor's metered remote relay, built for
[#13657](https://github.com/developing-today/code/issues/13657) on the
shape of the Desktop Commander door
([#13404](https://github.com/developing-today/code/issues/13404)).

The Worker is not part of the Go module. Its source is
`pkgs/mcpx/worker/`:

| File | What it is |
|---|---|
| `src/index.ts` | the router, the `/mcp` bearer check and the proxy to the bridge door |
| `src/oauth.ts` | the OAuth 2.1 authorization server: discovery, registration, `/authorize`, `/token` |
| `src/filter.ts` | the `ns` selector: request gating and response filtering at the JSON-RPC layer |
| `wrangler.toml` | the Worker definition: name `mcpx`, compatibility date `2025-09-15`, `workers_dev` off, the plain variables and the `OAUTH_KV` binding |
| `config.json` | the `cf` tool's view of the same deployment: worker `mcpx`, the single route `mcpx.security.cab`, `workersDev: false` |
| `test/` | vitest suites for the filter, the OAuth server, the proxy and the promote step |
| `README.md` | the Worker's own operational readme; this chapter is its book form |

## The door chain

```text
AI client
  -> https://mcpx.security.cab/mcp        (the bare / is the same door)
     this Worker: OAuth bearer or machine bearer checked here;
     the ns selector is computed here, once, and nowhere else
  -> https://mcpx-link.security.cab/mcp   (the bridge door, BRIDGE_BASE)
     a Cloudflare Access application with a service-token-only policy;
     the Worker presents its Access service token on this leg
  -> the sandbox-link Cloudflare Tunnel
  -> http://localhost:8932 on the laptop
  -> the mcpx bridge proxy on 127.0.0.1:8932
  -> the mcpx daemon's /mcp endpoint, found through the daemon's
     state descriptor
```

Three boundaries, three different credentials, and no credential crosses
a boundary it was not issued for:

- The caller's bearer — an OAuth access token this Worker issued, or the
  machine bearer — is checked at the public door and then *removed*.
  The bridge leg carries the Worker's own Access service token
  (`CF-Access-Client-Id` / `CF-Access-Client-Secret`) instead, and any
  Access headers the caller sent are stripped.
- The daemon itself is loopback-only and unauthenticated; it trusts its
  only caller, the local bridge proxy. This door is therefore the
  daemon's security boundary, which is why the filtering below lives
  here and not in the daemon — until daemon-side authentication lands
  ([#13659](https://github.com/developing-today/code/issues/13659)).
- Only `/mcp` (and its bare-path twin `/`) is proxied. The daemon's
  `/v1/*` API is never proxied and has no public path.

One gap in the chain as recorded: the Worker's own readme and source
describe the bridge proxy on `127.0.0.1:8932` and the `mcpx-link` tunnel
ingress, but no unit or ingress rule defining either exists in this
repository's home configuration — the only bridge defined there is the
Desktop Commander bridge on `127.0.0.1:8931`. Treat the laptop half of
the chain as asserted by the Worker, not yet as described by the
deployment configuration.

## Routes

Every route the router in `src/index.ts` serves, and nothing else —
any other path is a 404 with a JSON `not found` body:

| Route | Method | What answers |
|---|---|---|
| `/healthz` | any | `{"ok": true, "service": "mcpx"}`, no authentication |
| `/.well-known/oauth-protected-resource` | any | the RFC 9728 protected-resource document: resource `https://mcpx.security.cab/mcp`, this Worker as its authorization server, scope `mcpx` |
| `/.well-known/oauth-protected-resource/mcp` | any | the same document |
| `/.well-known/oauth-authorization-server` | any | the RFC 8414 authorization-server document: the `/authorize`, `/token` and `/register` endpoints, `code` response type, `authorization_code` and `refresh_token` grants, PKCE `S256` only |
| `/register` | POST (405 otherwise) | dynamic client registration, RFC 7591 |
| `/authorize` | GET and POST (405 otherwise) | the approval page and the approval decision; the one path behind Cloudflare Access |
| `/token` | POST (405 otherwise) | authorization-code exchange and refresh-token rotation |
| `/mcp` | any | the gated, filtered proxy described below |
| `/` | any | exactly `/mcp`: same gate, same proxying; the upstream path is always `/mcp` |

## Who may call: the authentication model

Hosted MCP clients — Claude, ChatGPT — connect from their own servers
and cannot pass a Cloudflare Access login at `/mcp`. So `/mcp` itself
takes a bearer token, and Access guards only the human leg of the
OAuth flow:

- **OAuth access tokens.** A client registers itself at `/register`
  (public clients only: `token_endpoint_auth_method` must be `none`,
  and every redirect URI must be https, or http on a loopback host).
  It sends its user to `/authorize` with a PKCE `S256` challenge.
  `/authorize` sits behind a Cloudflare Access application scoped to
  `mcpx.security.cab/authorize` whose policy admits the owner's Google
  identity only — and the Worker does not take Access's word for it:
  before rendering the approval page it verifies the Access JWT itself
  (RS256 against the team JWKS from `ACCESS_TEAM_DOMAIN`, the audience
  `ACCESS_AUD`, the issuer, the expiry, and the owner email held in the
  `OWNER_EMAIL` constant in `src/oauth.ts`). Approving mints a
  single-use authorization code, good for 10 minutes; `/token`
  exchanges it — code verifier checked, code consumed on *any* attempt,
  successful or not — for an opaque access token good for 1 hour and a
  refresh token good for 30 days. Refreshing rotates: the presented
  refresh token dies whatever happens next, and a fresh pair is issued.
- **The machine bearer.** The owner's own tooling skips OAuth and sends
  a static bearer instead (fleet secret `mcpx.machine-bearer`, staged
  into the Worker as the `MACHINE_BEARER` secret). The comparison is
  constant-time (`timingSafeEqual` in `src/oauth.ts`).
- **State.** Everything OAuth lives in the `OAUTH_KV` namespace.
  Clients are stored in the clear under `client:<id>` (they hold
  no secret); pending approvals under `pending:<nonce>` for 10
  minutes; codes and tokens are stored only under the SHA-256 of the
  presented value — `code:<hash>`, `at:<hash>`,
  `rt:<hash>` — never in the clear. Revoking a client is an
  operation on that namespace: delete its `at:` and `rt:` entries, and
  its `client:` record.
- **Refusal.** `/mcp` without a valid bearer is a 401 whose
  `WWW-Authenticate` header points at the protected-resource document —
  that header is what sends a hosted client into the OAuth flow. The
  single scope is `mcpx`; there are no per-client scopes, which is
  exactly the gap the `ns` selector fills, next.

## The `ns` selector

The upstream daemon offers this door no per-client hooks, so the Worker
polices exposure itself, on requests *and* responses. Enforcement is a
pure function of the request's selector and the message: nothing is
keyed to `Mcp-Session-Id`, and a client cannot widen its own exposure
mid-session except by choosing a wider selector on the next request.

The client picks the selector with the `ns` query parameter on `/mcp`
(or `/`). The router computes it exactly once, in
`resolveFilterSelector` (`src/filter.ts`), strips `ns` before
proxying — the bridge and the daemon never see it — and hands the
selector to the gate and the filter. Any caller `codemode` parameter is
dropped the same way: only the code-mode surface is offered.

| `ns` | Profile | Selected set | What it means |
|---|---|---|---|
| absent | `conservative` | empty | discovery only: see the catalogue, call nothing |
| `a,b` | `selected` | `{a, b}` | discovery, plus calls into exactly namespaces `a` and `b` |
| `*` | `full` | — | no filtering at all: the whole daemon, as a local client would see it |
| empty, or only commas | `selected` | empty | still enforced as an empty set; an empty selector never silently widens to the discovery profile |

### What each profile may do

The discovery tools are `mcpx_namespaces`, `mcpx_catalog`,
`mcpx_types` and `mcpx_search`. The selected profile adds exactly two
dispatch tools, `mcpx_call` and `mcpx_batch`. `mcpx_exec`,
`mcpx_tool_invoke` and every other `mcpx_*` extra are never exposed by
the conservative or selected profiles — not in listings, and not by
calling them directly.

- **`tools/list`** responses are filtered to the profile's tool set.
- **`tools/call`** requests are gated *before* anything reaches the
  bridge; a refused call never touches the daemon:
  - `mcpx_namespaces`, `mcpx_catalog` and `mcpx_search` are allowed in
    both profiles. The `mcpx_namespaces` *response* is filtered to the
    selected set (empty under conservative); `mcpx_catalog` and
    `mcpx_search` responses are metadata and pass unfiltered — a known
    v1 limitation, below.
  - `mcpx_types` is allowed only when the caller names a namespace
    list and every entry is in the selected set. With no list the
    upstream would list every namespace's types, so the call is
    refused — which means the conservative profile, whose set is
    empty, refuses `mcpx_types` even though it lists it.
  - `mcpx_call` (selected only) resolves its `namespace` / `tool`
    arguments with mcpx's own dotted-name rule — a dotted name splits
    on the first dot, from either argument — and is allowed only when
    the resolved namespace is in the set.
  - `mcpx_batch` and its alias `batch_call` (selected only) are
    all-or-nothing *as a tool call*: every item must resolve to a
    namespace in the set, and no item may target a gateway tool (any
    `mcpx_*` name, or `batch_call`). One bad item refuses the whole
    batch call.
  - Everything else is refused, in both profiles.
- **JSON-RPC batches** — an array body, as distinct from the
  `mcpx_batch` tool — are gated element by element. Allowed elements
  go upstream; each refused element gets its own error object, and the
  two halves are merged back into one response (as appended SSE events
  when the upstream answer is a stream). If *every* element is refused,
  the door answers itself and nothing is called upstream at all.
- **Prompts and resources.** `prompts/list` and `resources/list`
  responses drop entries whose `mcpx://<ns>/…` URI is outside
  the selected set, and `prompts/get` / `resources/read` for such a URI
  are refused. URIs that are not `mcpx://` URIs pass.
- **Everything else passes**: `initialize`, `ping`, notifications and
  any other method are proxied untouched; list-shaped answers are
  filtered on the way back instead.
- **Responses** are filtered in whichever framing the upstream used —
  plain JSON or SSE `data:` lines — and re-emitted in the same framing.
  The full profile is the exception: its response streams through
  untouched.

### Refusal shape

A refused call is not an HTTP error. The door answers HTTP 200 with a
JSON-RPC error object:

```json
{"jsonrpc": "2.0", "id": 1, "error": {"code": -32602, "message": "not exposed by this door's selector"}}
```

(`-32602` and that message are the `NOT_EXPOSED_ERROR` constant in
`src/filter.ts`; the `id` is the refused message's own id.) A body that
is not JSON-RPC the gate can police — unparseable, or carrying no
`method` at all — is forwarded untouched; the gate fails open on shape
and closed on meaning.

## Configuration and secret reference

Every load-bearing name below is spelled as it appears in
`wrangler.toml`, `src/index.ts` or `src/oauth.ts`. Secret *values*
appear nowhere in the repository; only their names do.

Plain variables (`[vars]` in `wrangler.toml`):

| Name | Value in the repository | What it is |
|---|---|---|
| `BRIDGE_BASE` | `https://mcpx-link.security.cab` | the bridge door; the proxy's upstream, always at path `/mcp` |
| `ACCESS_TEAM_DOMAIN` | `snowy-sun-58a9.cloudflareaccess.com` | the Access team domain hosting the JWKS the Worker verifies `/authorize` assertions against |
| `ACCESS_AUD` | the audience string in `wrangler.toml` | the audience of the Access application scoped to `/authorize`; an assertion for any other audience is refused |

Bindings and secrets:

| Name | Kind | What it is |
|---|---|---|
| `OAUTH_KV` | KV namespace binding (id in `wrangler.toml`) | all OAuth state: clients, pending approvals, hashed codes and tokens |
| `ACCESS_CLIENT_ID` | per-version Worker secret | the Access service token's client id for the bridge door |
| `ACCESS_CLIENT_SECRET` | per-version Worker secret | its secret; with the id, presented as `CF-Access-Client-Id` / `CF-Access-Client-Secret` on the bridge leg |
| `MACHINE_BEARER` | per-version Worker secret | the static bearer the owner's machine clients present at `/mcp` |

Where the values live, by name only:

| Store | Names |
|---|---|
| fleet secrets | `cloudflare.access.mcpx-link-client-id`, `cloudflare.access.mcpx-link-client-secret` (the bridge-door token), `mcpx.machine-bearer`, `cloudflare.api-token-mcpx-worker` (the scoped deploy token), `cloudflare.tunnel.sandbox-link-token` (the tunnel the bridge door rides) |
| GitHub environment `mcpx-production` | `MCPX_WORKER_CF_TOKEN`, `BRIDGE_ACCESS_CLIENT_ID`, `BRIDGE_ACCESS_CLIENT_SECRET`, `MACHINE_BEARER` |
| Worker version secrets | `ACCESS_CLIENT_ID`, `ACCESS_CLIENT_SECRET`, `MACHINE_BEARER` — uploaded per version by the publish workflow, from the environment secrets |

Fixed in code, not configuration: the issuer and resource
(`https://mcpx.security.cab` and `…/mcp`), the scope `mcpx`, the owner
email (`OWNER_EMAIL`), and the lifetimes — code and pending approval
600 seconds, access token 3600 seconds, refresh token 30 days.

## Failure shapes

| Symptom | Where it comes from | Meaning |
|---|---|---|
| 401 `{"error": "unauthorized"}` with a `WWW-Authenticate` resource pointer | `/mcp` gate | no bearer, or neither an unexpired OAuth token nor the machine bearer |
| 404 `{"error": "not found"}` | the router | the path is not one of the routes above |
| 405 `{"error": "method not allowed"}` | `/register`, `/authorize`, `/token` | wrong method for the endpoint |
| 500 `{"error": "bridge credentials are not configured"}` | `/mcp`, after the bearer check | the deployed version carries no `ACCESS_CLIENT_ID` / `ACCESS_CLIENT_SECRET`; the door refuses rather than proxying unauthenticated to the bridge |
| HTTP 200 with JSON-RPC `-32602` | the filter gate | the selector does not expose that call; nothing reached the daemon |
| OAuth `invalid_grant` from `/token` | the token endpoint | unknown, expired or already-consumed code; a code whose client, redirect URI or PKCE verifier does not match; an unknown, expired or rotated-out refresh token. Codes and refresh tokens are consumed on the attempt, so a failed exchange cannot be retried with the same material |
| OAuth `invalid_client_metadata` / `invalid_redirect_uri` from `/register` | registration | a non-JSON body, a client asking for a token-endpoint auth method other than `none`, or redirect URIs that are not https / loopback http |
| OAuth `unsupported_grant_type` from `/token` | the token endpoint | a grant other than `authorization_code` or `refresh_token` |
| an HTML error page from `/authorize` (403, or 400) | the authorization endpoint | 403: no verifiable Access assertion for the owner — the request did not come through the Access application as the owner. 400: unknown client, unregistered redirect URI, a response type other than `code`, a missing or non-`S256` PKCE challenge, or a scope outside `mcpx`; on POST, an expired or already-used approval nonce |
| a redirect to the client with `error=access_denied` | `/authorize` POST | the owner pressed Deny — a completed flow, not a failure |
| the upstream's own status and body | the proxy | once a request passes the gate, upstream answers — including its errors, its `Mcp-Session-Id` header and its SSE streams — pass back untouched, subject to response filtering |

## Operations

**Tests.** The Worker has its own suite, independent of the Go module:
`npm test` (vitest) and `npm run typecheck` in `pkgs/mcpx/worker/`. The
`mcpx-worker-tests.yml` workflow runs them on pushes and pull requests
that touch the directory.

**Publishing** is a two-step versions flow, modelled on the
desktop-commander and id-mail publisher–promoter pairs, and uploading
never moves traffic by itself:

1. *Publish* — `mcpx-worker-publish.yml`, manual dispatch on `main`
   with `confirm_upload`: tests, a credential-free dry run, then
   `upload-version.mjs` creates a Worker version with the version
   secrets file. The run emits a provenance artifact
   (`mcpx-worker-upload-<run_id>-<run_attempt>`) naming
   the version, its tag (`mcpx-<sha12>-run-<id>-attempt-<n>`)
   and the baseline it was built against.
2. *Promote* — `mcpx-worker-promote.yml`, manual dispatch naming the
   publish run and attempt, with `confirm_promotion`:
   `verify-upload-run.mjs` vouches that the run was a successful manual
   publish on `main`; `promote.mjs` checks the live deployment still
   matches the manifest baseline (or already serves exactly this
   version, an idempotent no-op) and deploys that one version at 100%.

As of this chapter's writing the publish step is the known break in
the chain: the workflow has failed on `main`
([#13754](https://github.com/developing-today/code/issues/13754))
because the Worker does not yet exist on the Cloudflare account —
a versions upload cannot be a Worker's first deployment
([#13770](https://github.com/developing-today/code/issues/13770)).
Until a first full deployment exists, treat every "live" statement in
this chapter as describing the code's behaviour, verified by its test
suite, not a probed endpoint.

**Revocation and rotation.** Revoke an OAuth client by deleting its
`at:` / `rt:` entries (and its `client:` record) from `OAUTH_KV`.
Rotate the machine bearer or the bridge service token in fleet
secrets, update the `mcpx-production` environment from them, and
publish + promote a new version — the Worker-side values are
per-version secrets, so rotation is a deployment, not an edit.

**Client setup.** Hosted clients add the custom connector
`https://mcpx.security.cab/mcp` and are walked through registration,
the Access sign-in and the approval page on first connection.
Header-capable clients (Claude Code, Cursor, other mcpx clients) send
`Authorization: Bearer <machine bearer>` and choose their
exposure with `?ns=` on the URL.

## The related work, in order

The door's own items, in the order they should be worked — the same
shape as the programme's issue map, narrowed to this Worker:

| Order | Issue | What it is | Depends on |
|---|---|---|---|
| 1 | [#13770](https://github.com/developing-today/code/issues/13770) — mcpx Worker does not exist on the Cloudflare account | the publish workflow cannot upload a first version; a first full deployment has to happen before the versions flow can run | nothing in code; it is an account/deployment step |
| 2 | [#13754](https://github.com/developing-today/code/issues/13754) — main CI failure: mcpx worker publish workflow | the recorded `main` failure of that publish step | #13770 resolved, then a green publish + promote run |
| 3 | [#13657](https://github.com/developing-today/code/issues/13657) — public endpoint at mcpx.security.cab with OAuth/Google-SSO front door and machine bearer | the door's specification and acceptance list: discovery documents, registration, authorize/token behaviour, machine-bearer access from outside, `/` ≡ `/mcp`, the 401 shape, the service-token-only bridge door, the pipeline proven end to end | the implementation (#13697, closed) and a live deployment (order 1–2) to check the acceptance list against |
| 4 | [#13659](https://github.com/developing-today/code/issues/13659) — per-user/per-key authentication in the daemon | moves authentication into the Go daemon and resolves each identity to a CapabilityCeiling / PolicySession; the `ns` selector then *maps onto* a ceiling instead of being enforced by Worker-side filtering | #13657's door as the interim boundary; its own design record first. Does not block the door |
| — | [#13697](https://github.com/developing-today/code/issues/13697) — public front-door Worker with OAuth and selector enforcement | the implementation of #13657, including this Worker's source and tests — **closed** | — |
| — | [#13404](https://github.com/developing-today/code/issues/13404) — self-hosted Desktop Commander MCP endpoint | the proven pattern this door mirrors: Worker front door, Access gate, laptop bridge (covered in the fleet book) | — |

Dependency map, in words: #13404 proved the shape; #13697 built the
mcpx door from it for #13657; #13770 blocks #13754's workflow, and
both block the live half of #13657's acceptance; #13659 is the
follow-on that retires this chapter's central assumption — that
exposure policy lives in the proxy — once the daemon can hold it
itself. The Google identity behind the `/authorize` Access application
rests on the Access Google OAuth client recorded in
[#12417](https://github.com/developing-today/code/issues/12417).

## Limitations, stated once

- `mcpx_catalog` and `mcpx_search` responses are unfiltered metadata:
  under the conservative and selected profiles a caller can learn that
  namespaces outside its set exist, though it cannot call into them.
- The flat (non-code-mode) surface is not offered; a caller's
  `codemode` parameter is dropped.
- The selector is chosen by the caller, not assigned to the caller:
  any holder of a valid bearer can ask for `?ns=*`. The selector is an
  exposure control for well-behaved clients, not an authorization
  boundary between callers — that boundary is #13659's work.
- The daemon behind the bridge is unauthenticated by design; the door
  is the boundary, and nothing else may be given a path to the bridge.
