## MCPX-SIDELINE result — mcpx book coverage of the door Worker

**PR:** [#13906](https://github.com/developing-today/code/pull/13906) — docs(mcpx): book chapter for the door Worker at mcpx.security.cab
**Merged:** merge commit [c4e2e729](https://github.com/developing-today/code/commit/c4e2e7294b3e30cfc3d0f8af1139c748d02e42ba) (branch commit a0c78e1259a)

**Chapter:** `pkgs/mcpx/docs/worker.md` — "The door Worker at mcpx.security.cab" (350 lines). Covers: what the Worker is (`pkgs/mcpx/worker/`, src/index.ts + oauth.ts + filter.ts), the door chain (client → Worker OAuth/bearer gate → Access service-token bridge door at mcpx-link.security.cab → sandbox-link tunnel → bridge proxy on 127.0.0.1:8932 → daemon), every route, the auth model (OAuth 2.1 + PKCE S256, Access-verified /authorize, machine bearer, hashed KV state, TTLs), the `?ns=` enforcement semantics at the JSON-RPC layer (absent = conservative/discovery-only; `?ns=a,b` = selected set's call/batch; `?ns=*` = full pass-through; refusals are HTTP 200 with JSON-RPC -32602), the config/env/secret-name reference (all names grepped from wrangler.toml, the sources and the home config — names only, no values), failure shapes, publish/promote operations, and a prioritised order list + dependency map for the door's items (#13770 → #13754 → #13657 → #13659; #13697 closed; #13404 the mirrored pattern).

**Registration (both navigation systems):** `docs/SUMMARY.md` entry (H1 matches entry text) and `docs/compendium.manifest.json` (chapter-source in `designs`). The manifest also gains the previously missing entry for `worker/README.md` (not-compendium; its book chapter is the new file) — without it the manifest gate could not pass.

**Verification actually run (laptop worktree, fast local checks):**

- `go test ./internal/settings/ -count=1` — ok (includes TestEveryDocumentHasAChapter)
- `go test ./internal/refdoc/ -run TestCommittedRegionsAreCurrent -count=1` — ok
- `mdbook build` with docs-history built from tools/docs-history — builds; only warning is the pre-existing search-index-size warning; worker.html renders with the correct title, and a double-escaping bug in placeholders was caught in the render and fixed before merge
- Not run: the full Nix `.#mcpx-docs` build (laptop under heavy load; fast gates used per the brief). Post-merge, the "mcpx docs" workflow run 45299935945 at the merge SHA was still queued at last check (runner backlog) — not yet concluded.

**Disagreements found (numbered, for grouped filing):**

1. `worker/README.md` had no compendium manifest entry, so TestEveryDocumentHasAChapter failed on it — fixed in this PR.
2. The Worker's docs/source assert a bridge proxy on 127.0.0.1:8932 and an mcpx-link tunnel ingress, but no unit or ingress defining either exists in the repo's home configuration (only the Desktop Commander bridge on 127.0.0.1:8931 is defined there). The chapter states this openly.
3. #13657 calls the Worker a "transparent proxy"; only the full (`?ns=*`) profile is transparent — the other profiles buffer and filter responses.
4. #13659 and the Worker README cite #8299 for the CapabilityCeiling/PolicySession alignment; #8299's own title is an id phase-14 role-configuration item, so the citation's target is at best indirect.

Raw survey material for the programme's dated record is deposited in the lane's workspace deposit (pm-book/sideline-material/mcpx-sideline/): survey notes, verbatim config listings, the issue list, the disagreements list, and the chapter as landed.
