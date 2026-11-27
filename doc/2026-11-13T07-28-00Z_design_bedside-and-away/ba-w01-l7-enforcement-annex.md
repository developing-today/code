# BA-W01 L7 annex: enforcement on history, media, exports, summaries and background jobs

Annex to [`ba-w01-identity-authorization-contract.md`](./ba-w01-identity-authorization-contract.md),
leaf L7 of section 10 (issue #11756, parent #9365). Ledger rows BA-086 and BA-088.

## 2026-11-26T22:50Z Plan: slice 1

L7 depends on L4. L4 (#11753) is closed as implemented by PR #11993, which added the section 4
evaluator, the query scope builder and per-surface shadow mode to
`pkgs/bedside-and-away/worker/access.mjs`. The dependency is therefore met.

Slice 1 puts every L7 surface behind one module, `pkgs/bedside-and-away/worker/surfaces.mjs`. Each
seam asks the evaluator, through `projectSnapshot` or `compileAccess`, as the principal that started
the work. No seam filters with logic of its own (section 5: "No surface may filter with its own
logic").

## Denial-shape table

Traced row by row to the section 5 table. "Absent" means the entity never existed; "forbidden" means
it exists and the evaluator denies it. The test column names the assertion that compares the two.

| §5 surface            | Seam                                               | Principal used                              | Forbidden answer                                                    | Same as absent? | Test                                                        |
| --------------------- | -------------------------------------------------- | ------------------------------------------- | ------------------------------------------------------------------- | --------------- | ----------------------------------------------------------- |
| History and audit     | `historyView` (wraps `projectEvents`)              | Request principal                           | Scoped history omitted entirely, never redacted to placeholders     | Yes             | `tests/surfaces.mjs` history block                          |
| Media                 | `mediaLookup(state, context, {kind, id})`          | Request principal                           | `null`, answered as `NOT_FOUND` (404 `Not found`)                   | Yes             | `tests/surfaces.mjs` media block                            |
| Exports               | `buildExport`; route `GET /api/stores/:id/export`  | Request principal                           | Row left out; export states `storeId` and `sourceRevision`          | Yes             | `tests/surfaces.mjs` export blocks; existence oracle route  |
| Summaries and reports | `summaryInput` (the report pipeline's only input)  | Request principal                           | Row left out before any aggregate is computed                      | Yes             | `tests/surfaces.mjs` summaries block                        |
| Background jobs       | `jobEnvelope`, `authorizeJobTargets`, `jobAuthorizer` | Initiator carried in the frozen job envelope | `NOT_FOUND`; the steering outbox refuses the job as revoked         | Yes             | `tests/surfaces.mjs` jobs and outbox blocks                 |

A store the caller cannot reach at all is answered by `access()` in `worker/api.mjs` with the same
404 as an absent store. `tests/existence-oracle.mjs` now probes `GET /export` alongside the other
store routes.

## What each seam guarantees

- **History.** A store with scoped policy has its history withheld, because an old snapshot or label
  can describe a record that is no longer readable. This was already enforced by L4's
  `projectEvents`; `historyView` is the named seam the history reader goes through.
- **Media.** No media is stored today. `mediaLookup` is the seam any media route must use: it returns
  the row the media belongs to only if the principal can read it, and the route answers 404 for
  `null`, whether the row is absent or forbidden.
- **Exports.** The server builds the export from the same projection as the store read, so it holds
  only authorized rows. Non-administrators never receive the access policy. The source revision is
  stated so an export traces to one snapshot.
- **Summaries and reports.** `public/reporting.js` and `public/summary.js` compute over the
  snapshot the server already projected. `summaryInput` is the server-side input for any report
  pipeline, so aggregates never count a hidden row.
- **Background jobs.** A job envelope carries the initiating principal and the store role they held,
  frozen at creation. Every target is decided as that principal; a target they cannot act on, or
  cannot see, is refused exactly like a missing target. `jobAuthorizer` adapts this to the steering
  outbox's `authorize` resolver, so the outbox's existing version predicates still close
  authorization races.

## Not in this slice

- Shadow mode (section 9) is not applied to these seams. Each is new or already enforced, so there is
  no earlier unenforced behaviour to compare against; they enforce from the start. The `SHADOWABLE`
  list in `access.mjs` is unchanged.
- The steering outbox is not yet wired into a live route. `jobAuthorizer` is the resolver that route
  must pass when it is.
- A media upload or fetch route does not exist. When one is added it must call `mediaLookup`.
