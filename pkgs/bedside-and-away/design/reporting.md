# Reporting, receipts, defaults and household decisions

Implements a bounded component of #9359. Existing layout and versioned store event endpoint remain the integration boundary. This component is not an OCR service, market-research runner, account system or completed live release.

## Decisions and alternatives

- Preserve the current JSON/event store rather than introduce a competing accounting database. Pure policy functions yield a new state; existing `apply` persists atomically with expected store version and an audit event. Server authorization must precede calls and every returned record must be filtered to the current principal.
- Use integer two-decimal minor units rather than binary floating arithmetic for monetary operations. Existing `total` records are read as USD; new records retain `totalMinor` plus legacy `total` and explicit currency. Do not combine currencies or guess exchange rates. Receipt staging supports USD/EUR/GBP/CAD/AUD only; extending minor-unit exponents needs a migration.
- Explicit category allocations sum exactly to the receipt line total. Otherwise split equally across unique sorted category IDs, allocating remainder cents deterministically. A transaction is not counted twice just because it belongs to multiple categories. Report labels can coincide; aggregation identity still uses category IDs.
- A blank amount is unknown, not zero. Negative stock is shown as reconciliation needed, not silently clamped. Stock keys are item ID plus unit, not names.
- Receipt interpretation begins with reviewed transcription (`name | full line total | quantity`). Exact unique-name matches are suggestions; ambiguous or missing matches must be chosen. Save only checked rows matched to current items. No product creation, OCR, remote fetch, contextual inference, or deletion is implied. Imports are idempotent within a receipt ID and staged line ID. Re-staging the same real receipt with a new ID is not automatically detected.
- Dates without times never invent an hour from the timestamp at which someone recorded the entry. Timestamp reports use an explicit IANA zone, initially America/Chicago. Holiday/busy/slow classification remains human supplied.
- Defaults remain preview-only absent explicit enabled mode. Ancestor disabled/off is conservative and sticky. Custom values, cancelled proposals and archived scopes survive refresh. Accept validates captured revision and current policy, rejecting stale proposals. Explicit refresh creates a new proposal revision but does not undo cancellation. Globs support literal text, `*` and `?` over labels/sections; raw file/content scanning is not implemented.
- Household decision aggregation uses equal weights across configured person labels, not authenticated voting. All active criteria must be rated for all people to display a combined score. Required constraints remain a separate eligible/failed/unknown result; a high score cannot override failed compatibility. Editor author and time accompany each assessment.

## API and integration

Load `public/reporting.js` and `public/reporting-ui.js` with existing assets. Both are plain browser scripts. Helpers are also executable under Node's VM without DOM dependencies. UI uses established `apply`, `dlg`, `current`, `version`, `hub`, `byId`, `decisionContext`, `decisionBoard`, and HTML escaping helpers.

Required parent-owned integration edits:
1. Build/embed/serve both scripts in the same manner as decision.js; load before app startup rendering.
2. Render map `report: reportingView` instead of `report`.
3. Inventory action calls `reportingInventory(id)`.
4. Defaults `refresh(ids)` delegates to `reportingRefresh(ids)`; `accept(ids)` delegates to `reportingAccept(ids)`. The wrapper captures `version+1` during preview because saving its event advances revision.
5. Add `reportingReceiptButton()` to Quick submit. Receipt button is also in reporting view.
6. Add `reportingDecisionButton()` and `reportingGroupSummary(s)` to Decision room.
7. Include both new tests in aggregate checks. Preserve native cf deployment; no Wrangler or dependency additions.

Core functions on `BedawayReports`:
- `moneyToMinor(value) -> integer|null` (throws on invalid precision/negative values).
- `allocateExpense(minor, categoryIds, explicitAllocations?) -> {categoryId,minor}[]`.
- `stageReceipt(text,{id,store,at,items,currency}) -> pending receipt`.
- `applyReceipt(state,stage,{selected,revision,expectedRevision,author}) -> new state`.
- `buildReport(state,{groupBy,timeZone}) -> {totals,groups,stock,unknownAmountCount,timeZone}`.
- `previewDefaults(state,{scopes,sections,revision}) -> new state`.
- `acceptDefaults(state,{scopes,revision,ids?}) -> new state`.
- `groupDecision(criteria,people) -> {people,score,eligible}`.

Scope descriptors are `{id,label,ancestors,archived}`. Monetary allocations are `{categoryId,minor}`. Receipt format and records are versioned. Keep editor authorization and server validation at the existing mutation boundary; these helpers alone are not an authorization service.

## Ledger coverage and remaining gaps

|IDs|This component|Still needed|
|---|---|---|
|BA-048, BA-092|Manual multi-person ratings, separate required constraints, author/time, unknown handling|Authenticated independent ratings/identity and E2E UI review|
|BA-052, BA-055|Inventory records and staged receipt transcription, item matching, no-change cancellation, retry dedupe|OCR, external source ingestion, receipt identity across separately staged drafts, unit conversion|
|BA-053, BA-054|Explicit structured receipt entry only|Contextual phrase inference remains unimplemented; do not claim AI understanding|
|BA-056, BA-057|Existing budget UX retained, currency-separated bars, explicit exact-cent allocation, author/category grouping|Budget periods/multi-currency budget conversion not chosen|
|BA-058|IANA time-zone grouping, date-only unknown hour, manual context labels|Holiday calendar integration, inferred busy/slow absent|
|BA-059–067|Opt-in default application, hierarchy, additive/custom/disabled protection, stale preview guards, uncancelled subset/bulk acceptance|Actual research content generation, file/content pattern scanning, arbitrary graph/text scope UI beyond current scopes|
|BA-069–073|No change to existing saved-source summaries|Automatic research execution/context derivation and remote parser absent|

## Verification

- `node tests/reporting.mjs`: monetary precision, deterministic allocations, exact sum validation, receipt staging/no mutation, selected-match apply, duplicate and stale guards, date/time/unknown handling, item-ID stock separation, defaults custom/disabled/cancelled/archived/stale paths, multi-person unknown/constraint logic.
- `node tests/reporting-ui.mjs`: staged UI versus commit, selected mapping, stale open dialog, revision capture, decimal validation, render smoke.
- Existing `node scripts/check.mjs`, decision/evidence/steering/variants tests passed on copied prototype.
- Browser interaction, authenticated backend validation, deployed data migration and live read/write have not been run by this component worker.

Rollback: remove feature hooks/scripts without transforming old records. Newly written inventory retains legacy fields for older rendering, but old reports will not honor allocation/currency/unknown semantics; rollback must warn and prefer read-only views rather than silently misreport. Database snapshot/restore remains release owner responsibility.
