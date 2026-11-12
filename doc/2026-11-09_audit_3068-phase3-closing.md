# Phase 3 closing audit — parsed-shape contract (#3068 / #3069)

Audit run: queue task 4465, lane code-o, 2026-11-09. Code state:
`origin/main` 2b53f81b0e plus branch `queue/4465-3069-closing-audit`
(contract statement commit eb7c5044dc).

## Gate verification (acceptance: phase must not start before Phases 1–2 landed)

All nine gate issues verified CLOSED via the GitHub API on 2026-11-09:
Phase 1 — #2938, #2816, #2821, #2974; Phase 2 — #2832, #2738, #2800,
#2827, #2618. The gate, unmet at the last check on 2026-11-02 (five
still open), is now met, so this audit ran.

## Method

Grep inventory over the scoped surfaces (`bin/`, `scripts/`, `pkgs/`,
`pm-stack`) for `JSON.parse(`, response `.json()`, Python
`json.loads(`/`json.load(`, and MessagePack decode (`msgpack` /
`unpackb`):

- 698 raw hits: 590 JS/TS, 108 Python.
- 354 JS/TS hits are in tests, fixtures, smoke scripts, docs examples,
  and worker-poc code — disposition (c) as a class: code-constructed
  input, never a trust boundary.
- 302 production hits remain: 236 JS/TS across 87 files, 66 Python.
- MessagePack: zero decode sites in scope.

Dispositions: (a) covered by #3068 Phases 1–3 / a named origin issue,
(b) a boundary the phases missed and the same rule covers — filed as
a follow-up (below), (c) not a trust boundary, with the reason.

## Dispositions

### (a) Covered

- **pm-stack/aggregator production hits** (render.ts, sources/live.ts,
  snapshots.ts, events.ts, aa.ts, scripts/collect-fleet-feed.ts,
  gather-quota.ts, produce-charge-candidates.ts, and the rest of
  `pm-stack/aggregator/**`): the aggregator is the surface #3068's
  phases and the #2576 contract program already worked; this phase's
  scope is pm-stack's _non_-aggregator handlers. The contract
  statement now lands in the aggregator README (this branch).
- **pkgs/lab-ops-hub**: already compliant as-built — `lib.ts` routes
  crawl results through `parseCrawlResult` and its comment at line 454
  records the rule for inbound `req.json()` (guarded parse, required
  fields checked before binding).
- **pm-stack/projector/src/github.ts** (3 of 4 sites): parses to
  `unknown` first, then validates — the compliant pattern.
- **pkgs/id/web/src/image-browser.ts / image-upload.ts**: named shape
  parsers (`parseImageEntries`, `parseUploadResponse`) — compliant.
- **#2608 (cloudflare-export GraphQL `errors`)**: see the decision
  below; fixed, with the as-built normalisation at
  `cloudflare-quota-export.mjs:519-533`.

### (b) Missed boundaries — one follow-up issue filed and linked on #3069

1. `bin/sync-models.mjs:127` — models.dev response read as
   `(await res.json()).response?.models || {}`: absence-as-emptiness
   (corollary 3). A shape drift reads as "zero models" instead of an
   error.
2. `pkgs/id/web` response casts: `src/main.ts` (8 `.json()` sites,
   several `as {…}` casts), `src/world.ts:433,566`, `src/home.ts:105`
   — first-party server responses dereferenced by cast with no shape
   parser, in the package whose frame shapes are the #2824/#2802
   origins.
3. `pm-stack/dispatch`: `src/github.ts` casts (`{id}`,
   `{token, expires_at}`, `{number, html_url}`), `src/queue.ts:119,136`
   `TaskRow[]` casts, `src/auth.ts:109` response deref.
4. `pm-stack/projector/src/github.ts:191` — generic helper returns
   `(await res.json()) as T` (contrast its own unknown-first sites);
   `src/server.ts:252,346` inbound `req.json()` bodies.
5. `pkgs/atlas-search/worker`: inbound casts `src/index.ts:75`,
   `src/cache_do.ts:82`; response casts `src/index.ts:92,113`.
6. `pkgs/cold-store-reaper/worker/src/index.js:108` inbound
   `request.json()` job deref; `:159` `resp.json()` payload
   (`dist/worker.js` is the built copy of the same two sites).
7. `pkgs/mcpx/plugin/opencode/mcpx/daemon.ts:463,476,537` —
   `(await res.json()) as T` / `as Resolution` on daemon responses.
8. `bin/codedb-bridge.mjs:250` response parse raced then dereferenced;
   `bin/provider-catalog-sync.mjs:525` `res.json()` returned to
   dereferencing callers.
9. Exporter scripts returning API responses unvalidated to
   dereferencing callers: `pm-stack/meta-exporter` (:195, :330),
   `pm-stack/supabase-export`, `pm-stack/opencode-usage-export`
   (cloudflare-export's `errors` shape is the #2608 exception, above).
10. `pm-stack/opencode-claimant` (`claimant.py`, `poll.py`): queue-
    broker responses parsed and acted on (session launches) without a
    shape check. Loopback, token-authed, first-party — lowest risk in
    this list, same rule.

### (c) Not trust boundaries

- **Local config / state file reads** in `bin/` tools
  (provider-catalog-sync, opencode-route-proxy, proxy-supervise's
  `readJson`, sync-models' policy files, record.mjs): operator-owned
  files on the same host; a malformed file fails the tool loudly
  before any external side effect.
- **Python tooling** (`scripts/**`, `pkgs/mcpx/scripts/**`,
  `pkgs/cold-store-reaper/cold_store.py`, deploy and research
  scripts): parses local files and its own subprocesses' output —
  code-constructed input.
- **`?? []` / `|| {}` / `?? {}` defaults at parse boundaries**:
  inventoried explicitly per the issue. Almost all sit in the bin/
  local-config mergers above — optional-by-design: merging partial
  operator configs is the tools' purpose, and an absent section is
  the normal case, treated identically to an empty one by
  construction. `cloudflare-quota-export.mjs:526`
  (`gql.body?.errors ?? []`) is optional-by-design: absent `errors`
  _is_ no errors. The one absence-as-emptiness case is the network
  response at `bin/sync-models.mjs:127` — item (b)1.

## #2608 ownership decision

#2608 was **not** fixed under #2576's Phase 3 and is **not** fixed
under this phase: it was fixed under its own small_fix (queue
task 5394, PR #3463, merged at 48eb4d314 on 2026-11-01; issue closed
as completed). Recorded here and on #2608 per the issue's explicit
disposition requirement; do not double-file.

## Contract statement

Landed on this branch (commit eb7c5044dc): the #3068 rule — validate
the shape you deref, at the boundary, before any side effect; exact
plain-object vocabulary; failure is the surface's own error; absence
is not emptiness; the shared probe matrix (`null`, array, scalar,
`{}`, each relied-on field wrong-typed) — stated in
`pm-stack/aggregator/README.md` § "Parsed-shape contract (#3068)" and
in the `bin/sync-models.mjs` header.

## Remaining acceptance items after this evidence

- The (b) follow-up issue is filed and linked on #3069.
- The PR carrying the contract statement + this audit merges.
- #3068 closes with a link to this evidence once the PR is merged.
