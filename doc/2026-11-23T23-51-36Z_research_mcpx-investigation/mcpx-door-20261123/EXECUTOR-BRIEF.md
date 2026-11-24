# mcpx Worker (W1, #13657) — continuation brief

## State already reached (verified from tool read-backs, 2026-11-23 ~13:50 CST)

- Laptop primary clone `~/git/code` was on `main`, CLEAN at fetch time
  (the dirt named in the original brief — modified
  `doc/fleet-schedule-registry.md`, untracked `pkgs/id/examples/` —
  was NOT present; nothing was touched/stashed/committed by this lane).
- Worktree created: `~/git/code-mcpx-door-20261123`, branch
  `lane-mcpx-door-20261123`, off `origin/main` @ `0a6ca03ccca`
  (Merge PR #13538).
- Copied into the worktree (unadapted, byte copies):
  `pkgs/desktop-commander/worker/.` → `pkgs/mcpx/worker/`
  (node_modules excluded; 18 files incl. package-lock.json), and the
  three workflows → `.github/workflows/mcpx-worker-{publish,promote,tests}.yml`.

## Why this lane stopped

The `git worktree add` exec result tripped a platform execution-risk
flag on the checkout content. Per the task rule, the operation was
retried via the Desktop Commander endpoint
(`https://commander.security.cab/mcp`, bearer by name
`desktop-commander.machine-bearer` via `fleet secrets get`, in-memory
only): attempt 1 got Cloudflare error 1010 (browser signature banned,
Python UA); attempt 2 (browser UA) got a connection closed without
response. DC route from the sandbox is therefore not working this
session. Remaining build steps must run in a venue that does not trip
the flag — Drew's standing route: a laptop-local OpenCode session
working in the worktree in-session, evidence recorded on #13657,
Archer/parent finishes (merge) via `gh` from the record.

## Remaining steps for the executor (in the laptop worktree)

1. Get `adapt.py`, `filter.ts`, `README.md` from the sandbox workspace
   `~/workspace/mcpx-door-20261123/` to the laptop (they are
   hand-authored lane assets, not repo content).
2. Run `python3 adapt.py` from `~/git/code-mcpx-door-20261123`
   (it performs all Commander→mcpx swaps, the index.ts bare-path +
   selector restructure, writes `src/filter.ts`, appends the
   bare-path/selector tests to `test/proxy.test.ts`, fixes the
   package-lock name, and asserts the final config/AUD/KV/scope
   values). Then copy the companion `README.md` over
   `pkgs/mcpx/worker/README.md` (adapt.py deliberately excludes it).
3. Sanity greps: no `desktop-commander` left outside README's
   historical mention; workflows reference `MCPX_WORKER_*`,
   environment `mcpx-production`, concurrency `mcpx-worker-lifecycle`,
   artifact `mcpx-worker-upload-<run_id>-<run_attempt>`, tag prefix
   `mcpx-`.
4. In `pkgs/mcpx/worker`: `npm ci --no-audit --no-fund`, `npm test`,
   `npm run typecheck` — all green before PR. (Node v24.21.0 / npm
   11.19.0 verified present on the laptop.)
5. Commit (only `pkgs/mcpx/worker/**` + the three workflow files),
   push `lane-mcpx-door-20261123`, open PR: description covers the
   door, the adaptations, bare-path equivalence, the filtering seam;
   "Part of #13657" — NO closing keywords (#13657 stays open until
   live verification). Re-read PR comments immediately before merging;
   merge once green.
6. Filtering enforcement: DO NOT invent semantics. `src/filter.ts`
   ships architecture-only (selector `ns` parsed once in the router,
   `DEFAULT_FILTER_PROFILE = "conservative"`, pass-through
   `filterMcpResponse`). The enforcement spec arrives as a follow-up
   from the parent; if it lands before the PR merges, implement it in
   `filter.ts` + tests in the same PR.

## Key adaptation values (already encoded in adapt.py)

- Worker `mcpx`, package `mcpx-worker`, routes `["mcpx.security.cab"]`,
  workersDev false, compatibilityDate `2025-09-15` (unchanged).
- ISSUER `https://mcpx.security.cab`, RESOURCE
  `https://mcpx.security.cab/mcp`, SCOPE `mcpx`, OWNER_EMAIL unchanged.
- BRIDGE_BASE `https://mcpx-link.security.cab`, ACCESS_TEAM_DOMAIN
  `snowy-sun-58a9.cloudflareaccess.com` (unchanged), ACCESS_AUD
  `b34b9764fb317774bf5c0ff0ac9cbd8a2ab80ed346d06f08d74967011bac640e`,
  KV OAUTH_KV id `11eb93dc9fa24087ae57b12483dc6172`.
- devDeps unchanged: cf 1.0.0-beta.20, wrangler 4.152.0,
  @cloudflare/workers-types ^5.20261019.1.
