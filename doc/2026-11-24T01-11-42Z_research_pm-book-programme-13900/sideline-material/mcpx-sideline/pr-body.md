## What

Adds the mcpx book's chapter for the **door Worker at mcpx.security.cab** (`pkgs/mcpx/worker/`, #13657) — the sideline assigned by programme #13900.

New file: `pkgs/mcpx/docs/worker.md` — what the Worker is, the door chain (client → Worker OAuth/bearer gate → Access service-token bridge door at `mcpx-link.security.cab` → sandbox-link tunnel → bridge proxy on `127.0.0.1:8932` → daemon), the `?ns=` enforcement semantics at the JSON-RPC layer (absent = discovery only; `?ns=a,b` = that set's call/batch; `?ns=*` = full pass-through; refusal = HTTP 200 with JSON-RPC `-32602`), the auth model (OAuth 2.1 + PKCE, Access-verified `/authorize`, machine bearer, KV state), the config/env/secret-name reference (all names grepped from `wrangler.toml`, `src/index.ts`, `src/oauth.ts` and the home config), failure shapes, publish/promote operations, and a prioritised order list + dependency map for the door's related issues (#13770 → #13754 → #13657 → #13659; #13697 closed; #13404 the mirrored pattern).

## Registration (both navigation systems)

- `docs/SUMMARY.md`: chapter entry after _Reaching a daemon on another machine_; the H1 matches the entry text exactly.
- `docs/compendium.manifest.json`: `docs/worker.md` as chapter-source in `designs`, plus the previously missing entry for `worker/README.md` (`not-compendium`, mirroring `daemon/README.md`; its book chapter is the new file). Without that second entry `TestEveryDocumentHasAChapter` cannot pass with the Worker in the tree.

## Verification (laptop worktree, fast local checks per bookwright v5)

- `go test ./internal/settings/ -count=1` — **ok** (includes `TestEveryDocumentHasAChapter`)
- `go test ./internal/refdoc/ -run TestCommittedRegionsAreCurrent -count=1` — **ok**
- `mdbook build` with the `docs-history` preprocessor built from `tools/docs-history` — **builds**; only warning is the pre-existing "search index is very large"; `book/worker.html` renders, title correct, escaped placeholders verified in the rendered HTML (an initial double-escaping of `<ns>`-style tokens was caught in the render and fixed)
- No generated regions were hand-edited; the chapter contains none.

## Disagreements found (for grouped filing by the coordinator)

1. `worker/README.md` had no compendium manifest entry — the manifest gate fails on it. Fixed in this PR (see above).
2. The Worker's readme/source assert a bridge proxy on `127.0.0.1:8932` and an `mcpx-link` tunnel ingress, but no unit or ingress defining either exists in the repo's home configuration (only the Desktop Commander bridge on `127.0.0.1:8931` is defined there). The chapter states this openly.
3. #13657's spec calls the Worker a "transparent proxy"; only the `full` (`?ns=*`) profile is transparent — the other profiles buffer and filter responses.
4. #13659 and the Worker readme cite #8299 for the CapabilityCeiling/PolicySession alignment; #8299's own title is an id phase-14 role-configuration item, so the citation's target is at best indirect.
