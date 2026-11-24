# MCPX-SIDELINE survey notes — the mcpx door Worker

Lane: MCPX-SIDELINE, programme #13900 (developing-today/code), coordinator PM-BOOK.
Surveyed 2026-11-23 (CST) from the existing laptop clone `~/git/code` (primary, main)
and the worktree `~/wt-mcpx-worker-book` (branch `book/mcpx-worker`, off origin/main
0ecb31eef33). All repo reads via `~/tools/laptop-ssh.sh`; GitHub reads via `fleet gh api`.
No fresh clone was made; the primary checkout was never modified.

## Where the Worker lives

`pkgs/mcpx/worker/` in developing-today/code:

```
cloudflare.config.ts, config.json, package.json, package-lock.json,
  promote.d.mts, promote.mjs, README.md, tsconfig.json, upload-version.mjs,
  verify-upload-run.mjs, vitest.config.ts, wrangler.toml
src/index.ts    router + /mcp bearer gate + proxy
src/oauth.ts    OAuth 2.1 authorization server
src/filter.ts   ns selector gate + response filter
test/filter.test.ts, test/oauth.test.ts, test/promote.test.ts, test/proxy.test.ts,
  test/memory-kv.ts
```

## Book structure findings (bookwright v5 gates)

- Book root: `pkgs/mcpx/` (book.toml: src = "docs", create-missing = false,
  preprocessor docs-history). Docs are flat markdown under `pkgs/mcpx/docs/`.
- TWO navigation systems:
  1. `docs/SUMMARY.md` (mdBook). H1 of a chapter must equal its SUMMARY entry text.
  2. `docs/compendium.manifest.json` (schema `mcpx-compendium/v1`), gated by
     `TestEveryDocumentHasAChapter` in `pkgs/mcpx/internal/settings/compendium_test.go`:
     every markdown file under pkgs/mcpx must have an entry (chapter + role
     chapter-source/chapter-index/symlink, or chapter null + role included/not-compendium,
     always with a note). Chapters list has 12 chapters (ids: features, designs,
     decision-records, protocol, build-and-packaging, proposed, catalogue,
     comparisons-and-history, research-and-findings, maintaining-these-docs,
     document-history, research-program).
- Generated-region currency gate: `TestCommittedRegionsAreCurrent` in
  `pkgs/mcpx/internal/refdoc/regions_test.go` (regenerate with
  `go test ./internal/refdoc -run TestCommittedRegionsAreCurrent -update`).
  The new chapter contains no generated regions.
- Pre-existing manifest gap found: `worker/README.md` had NO manifest entry
  (grep of the manifest for "worker" returned nothing; 156 entries, none for it),
  so the gate could not pass on the tree as found. Fixed in the PR by registering
  it as not-compendium (mirroring `daemon/README.md`'s entry pattern).
- `docs/remote.md` is chapter "designs" in the manifest — the new chapter
  `docs/worker.md` was registered in the same chapter, role chapter-source,
  lines 350.
- docs.nix builds the book via mdBook + docs-history + docs-twins (agent twins);
  local fast path used instead: `go build -o /tmp/docs-history ./tools/docs-history`
  then `PATH=/tmp:$PATH mdbook build`.

## Door chain (from worker/src/index.ts header comment + worker/README.md)

```
AI client -> https://mcpx.security.cab/mcp   (bare / identical)
  Worker: OAuth bearer or machine bearer; ns selector computed once in router
-> https://mcpx-link.security.cab/mcp   (BRIDGE_BASE)
  Access app "mcpx link (bridge)", service-token-only; Worker presents
  ACCESS_CLIENT_ID / ACCESS_CLIENT_SECRET as CF-Access-Client-Id/-Secret
-> sandbox-link Cloudflare Tunnel -> http://localhost:8932 on amd (the laptop)
-> mcpx bridge proxy on 127.0.0.1:8932
-> mcpx daemon's /mcp endpoint, discovered via the daemon state descriptor
Daemon is loopback-only, unauthenticated; /v1/* never proxied.
```

Grep result (disagreement #2): searching `home/`, `pkgs/mcpx/modules/`,
`pkgs/mcpx/daemon/` for `8932`, `mcpx-bridge`, `mcpx-link` returned NOTHING.
The home config (`home/common/default.nix` ~line 1079) defines only the
Desktop Commander bridge: supergateway on 127.0.0.1:8931/mcp, carried by the
sandbox-link tunnel to commander-link.security.cab (#13404). No mcpx bridge
unit or mcpx-link tunnel ingress exists in the repo's deployment config.

## Routes (src/index.ts, exhaustive)

- GET/any `/healthz` -> {"ok":true,"service":"mcpx"}
- `/.well-known/oauth-protected-resource` and `/.well-known/oauth-protected-resource/mcp`
  -> protectedResourceMetadata()
- `/.well-known/oauth-authorization-server` -> authorizationServerMetadata()
- POST `/register` (405 otherwise) -> registerClient
- GET/POST `/authorize` (405 otherwise) -> authorizeGet / authorizePost
- POST `/token` (405 otherwise) -> tokenEndpoint
- `/mcp` and `/` -> bearer gate, then proxyMcp with selector; upstream path always /mcp
- anything else -> 401 is only for /mcp|/ without valid bearer; other paths 404 {"error":"not found"}

401 shape: {"error":"unauthorized"} + header
`WWW-Authenticate: Bearer resource_metadata="https://mcpx.security.cab/.well-known/oauth-protected-resource"`.
500 shape when ACCESS_CLIENT_ID/SECRET missing (checked AFTER bearer):
{"error":"bridge credentials are not configured"}.

Proxy behaviour (proxyMcp): strips `ns` and `codemode` query params; deletes
host, authorization, cf-access-jwt-assertion, cf-access-client-id,
cf-access-client-secret from caller headers; sets CF-Access-Client-Id/-Secret;
redirect: "manual"; gates body via gateMcpRequestBody; if all elements rejected,
answers itself (200, JSON array for batch else single object); merges partial
batch rejections via mergeBatchRejections; otherwise filterMcpResponse.

## Filter semantics (src/filter.ts, exhaustive)

- `resolveFilterSelector(url)`: raw = `ns` param trimmed or null.
  - `*` -> mode full, profile "*"
  - non-null -> mode selected, namespaces = comma-split/trimmed/non-empty
    (empty or separators-only stays selected with empty set)
  - absent -> mode conservative, profile DEFAULT_FILTER_PROFILE = "conservative"
- DISCOVERY_TOOLS = mcpx_namespaces, mcpx_catalog, mcpx_types, mcpx_search
- SELECTED_DISPATCH_TOOLS = mcpx_call, mcpx_batch
- exposedToolNames: full -> [] (meaning no filtering), selected -> discovery+dispatch,
  conservative -> discovery only.
- gatewayCallAllowed:
  - mcpx_namespaces / mcpx_catalog / mcpx_search: allowed in conservative+selected
  - mcpx_types: selected only, requires non-empty args.namespaces, every entry's
    namespace part (before first dot) in set
  - mcpx_call: selected only; resolveTarget splits dotted namespace/tool on first
    dot either way; namespace must be in set
  - mcpx_batch / batch_call: selected only; calls non-empty; every call resolves,
    no gateway tool target (mcpx_* or batch_call as tool OR namespace), in set;
    one violation rejects the whole tool call
  - default: false (mcpx_exec, mcpx_tool_invoke, all else rejected)
- mcpRequestAllowed: full -> true; tools/call -> gatewayCallAllowed;
  prompts/get + resources/read -> uriAllowed (mcpx://<ns>/ URIs must be in set;
  non-mcpx:// URIs pass); everything else passes (lists filtered on return).
- NOT_EXPOSED_ERROR = {code: -32602, message: "not exposed by this door's selector"};
  jsonRpcError(id) wraps it with the message's own id.
- gateMcpRequestBody: null (forward untouched) when body empty, unparseable, or no
  element has a "method" key; otherwise splits allowed/rejected; bodyText is the
  allowed subset re-serialized.
- Response filtering: tools/list -> filterToolList; tools/call to mcpx_namespaces ->
  filterNamespaceResult (structuredContent + JSON-in-text content, namespaces/
  results/items keys recursed, bare namespace objects dropped when out of set);
  prompts/list + resources/list -> filterUriList; SSE data: lines transformed the
  same way; full mode returns the response untouched (streaming).
- mergeBatchRejections: filtered upstream body + rejected errors; for SSE appends
  `event: message` frames; else JSON array concatenation, content-type set to
  application/json.

README-only claims checked against code: "Pages are filtered as they arrive with
the upstream cursor passed through" — filterToolList spreads the result record,
so nextCursor survives; consistent. README limitation: mcpx_catalog and
mcpx_search responses unfiltered — consistent with transformResponseMessage
(only mcpx_namespaces gets result filtering).

## OAuth (src/oauth.ts, exhaustive)

Constants: ISSUER = https://mcpx.security.cab; RESOURCE = https://mcpx.security.cab/mcp;
SCOPE = "mcpx"; OWNER_EMAIL = the owner's Gmail address (constant in source; the
chapter deliberately does not print it). TTLs: CODE_TTL_S 600, PENDING_TTL_S 600,
ACCESS_TOKEN_TTL_S 3600, REFRESH_TOKEN_TTL_S 30*24*3600 = 2592000.
Metadata: response_types ["code"], grants ["authorization_code","refresh_token"],
code_challenge_methods ["S256"], token_endpoint_auth_methods ["none"],
service_documentation points at issue #13657.
Registration: redirect_uris must be non-empty, each https or http loopback
(localhost / 127.0.0.1 / [::1]); token_endpoint_auth_method if present must be
"none"; client_id = randomHex(16); stored at `client:<id>` with no expiry; 201.
Access verification (verifyAccessAssertion): header Cf-Access-Jwt-Assertion;
RS256 + kid required; JWKS from https://<ACCESS_TEAM_DOMAIN>/cdn-cgi/access/certs,
cached 300_000 ms in module scope; checks signature, exp, aud includes ACCESS_AUD,
iss === https://<ACCESS_TEAM_DOMAIN>, email === OWNER_EMAIL.
Authorize GET: 403 HTML page without valid claims; 400 for unknown client,
unregistered redirect_uri, response_type != code, missing/non-S256 challenge,
scope not containing "mcpx"; stores `pending:<nonce>` (nonce randomToken(24),
TTL 600) and renders approval form (says access token 1 hour, refresh 30 days,
revoke by deleting from KV).
Authorize POST: re-verifies claims; pending deleted on any outcome; deny ->
302 redirect with error=access_denied (+state); approve -> code randomToken(32)
stored at `code:<sha256hex>` TTL 600, 302 with code (+state).
Token endpoint: form-encoded only. authorization_code: requires code, client_id,
redirect_uri, code_verifier; code record deleted BEFORE checks complete
(consumed on any attempt); checks exp, client match, redirect match,
sha256Base64url(verifier) === code_challenge. refresh_token: requires
refresh_token + client_id; record deleted first (rotation); checks exp + client.
issueTokens: access randomToken(32) at `at:<sha256hex>` TTL 3600, refresh at
`rt:<sha256hex>` TTL 2592000; response {access_token, token_type Bearer,
expires_in 3600, refresh_token, scope}.
isValidAccessToken: `at:<sha256hex>` record exists and exp > now.
timingSafeEqual: constant-time except early return on length mismatch.
Machine bearer: index.ts compares Bearer token to env.MACHINE_BEARER with
timingSafeEqual; empty machine bearer never matches.

## Config listings

Verbatim wrangler.toml, config.json and package.json are in
`worker-config-listings.md` in this deposit. Headline values:
name "mcpx", main src/index.ts, compatibility_date 2025-09-15, workers_dev false;
vars BRIDGE_BASE=https://mcpx-link.security.cab,
ACCESS_TEAM_DOMAIN=snowy-sun-58a9.cloudflareaccess.com,
ACCESS_AUD=b34b9764fb317774bf5c0ff0ac9cbd8a2ab80ed346d06f08d74967011bac640e
(audience identifier, not a secret — it is committed in wrangler.toml);
KV binding OAUTH_KV id 11eb93dc9fa24087ae57b12483dc6172.
Note: wrangler.toml has NO routes section; the route lives in config.json
(`routes: ["mcpx.security.cab"]`, the cf tool's config) — disagreement-adjacent
observation recorded in the chapter's source table.

Secret names (values never read, names only — from wrangler.toml comments and
worker/README.md): Worker version secrets ACCESS_CLIENT_ID, ACCESS_CLIENT_SECRET,
MACHINE_BEARER. fleet secrets: cloudflare.access.mcpx-link-client-id,
cloudflare.access.mcpx-link-client-secret, mcpx.machine-bearer,
cloudflare.api-token-mcpx-worker, cloudflare.tunnel.sandbox-link-token.
GitHub environment mcpx-production: MCPX_WORKER_CF_TOKEN, BRIDGE_ACCESS_CLIENT_ID,
BRIDGE_ACCESS_CLIENT_SECRET, MACHINE_BEARER.

## Deployment / workflows (worker/README.md + .github/workflows listing)

Workflows present: mcpx-worker-publish.yml, mcpx-worker-promote.yml,
mcpx-worker-tests.yml (plus mcpx-docs.yml and other mcpx workflows).
Publish: manual dispatch on main, confirm_upload; tests; credential-free dry run;
upload-version.mjs runs `cf workers versions create` with secrets file; artifact
mcpx-worker-upload-<run_id>-<run_attempt>; tag mcpx-<sha12>-run-<id>-attempt-<n>.
Promote: manual dispatch with publish run id + attempt, confirm_promotion;
verify-upload-run.mjs vouches the run; promote.mjs checks live deployment vs
manifest baseline (or already-this-version no-op), deploys version at 100%.
package.json scripts: deploy = "cf workers versions create", test = "vitest run",
typecheck = "tsc --noEmit". Deps: cf 1.0.0-beta.20, wrangler 4.152.0,
typescript ^5.6.0, vitest ^4.1.0, @cloudflare/workers-types ^5.20261019.1.

## Verification actually run (worktree, laptop)

- `go test ./internal/settings/ -count=1` — ok (includes TestEveryDocumentHasAChapter,
  after both manifest entries were added).
- `go test ./internal/refdoc/ -run TestCommittedRegionsAreCurrent -count=1` — ok
  (first attempt with GOPROXY=off failed on module lookup for an unrelated dep;
  rerun with default proxy settings passed).
- `mdbook build` with docs-history built from tools/docs-history — succeeded;
  only warning "search index is very large" (pre-existing, book-wide);
  book/worker.html rendered, <title> "The door Worker at mcpx.security.cab - mcpx".
  First render caught double-escaped placeholders (&amp;lt;) from &lt; inside
  code spans; fixed by using literal < > inside code spans, rebuilt, verified
  single-escaped in HTML.
- NOT run: the full Nix `.#mcpx-docs` build (laptop under heavy load; the fast
  local mdbook + go gates were used per the brief), docs-twins gate (part of the
  Nix build only), worker vitest suite (chapter documents the Worker; its code
  was not changed).
- Post-merge: "mcpx docs" workflow run 45299935945 at merge SHA c4e2e7294 was
  still `queued` on last check (runner backlog; sibling runs for the same SHA
  also queued). Not yet concluded at deposit time.

## Transfer / process notes

- Draft written in the sandbox, transferred to the laptop as base64 chunks passed
  as command arguments, sha256-verified both sides
  (chapter sha256 c12f58d4e65977367e46f7a4d9f4762d5f845c9a1bd0f3940d73f5b6184d6afb).
  Appending chunks in combined shell calls duplicated two chunks once
  (42056 vs 30056 bytes); redone as one verified file per chunk, then concatenated.
- A heredoc through laptop-ssh was mangled by the channel (quotes stripped);
  the SUMMARY/manifest registration was instead done by a base64-transferred
  python script (deposited as registration-edit-script.py), sha256-verified.
- Commit a0c78e1259a on book/mcpx-worker; PR #13906; merged, merge commit
  c4e2e7294b3e30cfc3d0f8af1139c748d02e42ba.
