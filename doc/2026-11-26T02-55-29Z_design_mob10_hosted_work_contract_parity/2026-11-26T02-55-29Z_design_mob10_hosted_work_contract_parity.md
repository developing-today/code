# MOB-10 — Hosted parity of the work read and action contracts

Issue: #9477 (slice MOB-10 of #9201). Status: design record. It is not
an authorization to implement. Requirement IDs: H-R005, H-R023.

## 0. Governing decisions consumed

| Source                                         | What this record takes from it                                                                                                                                 |
| ---------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| #9201 MOB-10 slice text                        | Goal: the hosted editions serve the same read and action schemas with no semantic drift. Owned file `pm-stack/aggregator/test/hosted-mobile-contract.test.ts`. |
| #9079 design (`2026-11-23T03-22-03Z_...`) §1   | One Worker, route modules; `pm-stack/pm-service` cannot import the aggregator _app_.                                                                           |
| #9079 design §6, §7 invariant 1                | Divergences are named, never silent; same input in → same JSON out, pinned by a shared fixture suite (slice 8, #12833).                                        |
| #9081 (closed)                                 | The fleet tool's hosted edition. Nothing in this slice reads or writes its metadata DB.                                                                        |
| MOB-01 record D1, D2, D9                       | `projectWorkView` is pure; wire schema `pm_work_view/v1`; byte-identical output for the same input.                                                            |
| MOB-02 record D1                               | `work-lifecycle-policy.ts` is the one precedence implementation, pure.                                                                                         |
| MOB-05 (`src/work-action-contract.ts`, merged) | The action envelope, its states and declared edges, refusals, operation keys.                                                                                  |
| MOB-08 record §4                               | Relations, activity and outcomes sections and `pm_view_receipt/v1`; nothing persisted.                                                                         |
| MOB-09 record and code (#15321, merged)        | `reconcileWorkAction` and the mutation switch `applyIfMutationsEnabled`.                                                                                       |

## 1. Verified state on main `93483263`

- `pm-stack/aggregator/src/work-action-contract.ts` has **no imports**.
  It is a pure module with no I/O and no clock.
- `pm-stack/aggregator/src/work-operation-reconciliation.ts` imports
  only `work-action-contract.ts`.
- `work-projection.ts` (MOB-01) is on open PR #15188, not on main.
  `work-lifecycle-policy.ts` (MOB-02) and `work-relations-activity.ts`
  (MOB-08) do not exist yet.
- `outcome-view.ts` and `activity.ts` import `node:fs/promises`. They
  are laptop store code, not pure contract code.
- `pm-stack/pm-service` is the hosted edition (#9079). Its `src/`
  holds `collector/`, `read-api/`, `dashboard/`, `tenant-store/`,
  `front-door.ts` and `worker.ts`. It has no work view, no action
  route and no import from the aggregator. `collector/pool.ts` is a
  hand port of the aggregator's pool parser, pinned by slice 8.
- pm-service's `tsconfig.json` types are `@cloudflare/workers-types`
  only. Its suite runs in `pm-stack-tests.yml`; the aggregator suite
  runs only by dispatch in `pm-aggregator-tests.yml`.
- #12833 (slice 8, dual-edition conformance suite) is open and has no
  code on main. #12832 (identity wiring) is open.

## 2. Intent

The mobile work surfaces (the work view, its lifecycle column, its
relations, activity and outcomes, the view receipt, the action
envelope and its reconciliation) must mean the same thing in the
hosted edition as on the laptop. "The same" is checked, not promised:
for each golden input, the hosted code path returns the same bytes as
the laptop code path.

## 3. Ownership

- **Owned:** new `pm-stack/aggregator/test/hosted-mobile-contract.test.ts`
  and new `pm-stack/aggregator/test/fixtures/hosted-mobile-contract/`
  (the manifest of §4.4).
- **Read-only consumers:** the pure contract modules of §4.1, the
  MOB-01 fixtures under `test/fixtures/pm-work-view/`, the MOB-02
  fixtures under `test/fixtures/work-lifecycle/`, and pm-service.
- **Not touched here:** any pm-service route, the tenant store schema,
  `worker.ts`, `server.ts`, the projector, the Fleet crates. Wiring a
  hosted route that serves these schemas is a separately scoped slice
  under #9079 (§9).
- **Added nowhere:** no database, queue, store, cache, scheduler,
  service, GitHub client, writer or creation-outbox producer.

## 4. Decisions

### D1. One implementation, imported by both editions

The contract modules are pure, so the hosted edition imports them by
relative path instead of porting them:

| Module (all under `pm-stack/aggregator/src/`) | Owner  | Wire schema it produces                             |
| --------------------------------------------- | ------ | --------------------------------------------------- |
| `work-projection.ts`                          | MOB-01 | `pm_work_view/v1`                                   |
| `work-lifecycle-policy.ts`                    | MOB-02 | `projection.column`, reason codes, conflicts        |
| `work-relations-activity.ts`                  | MOB-08 | relations, activity, outcomes, `pm_view_receipt/v1` |
| `work-action-contract.ts`                     | MOB-05 | the action envelope and its receipts                |
| `work-operation-reconciliation.ts`            | MOB-09 | reconcile result, apply gate result                 |

The #9079 rule is that pm-service does not import the aggregator
_app_: its server, stores, feed readers and Node I/O. These five
modules are none of those. Wrangler bundles a relative import outside
the package like any other source file. A port would be a second
implementation whose only job is to agree with the first, which is
the "divergence factory" #9079 §3 rejects for D1.

The pool parser stays a port (#9079 slice 2) because the aggregator's
`projects.ts` imports the collector's I/O. D1 does not change that.

### D2. The pure-closure rule

A module in the D1 table may import only:

- another module in the D1 table, and
- `import type` from `model.ts`.

It may not import `node:*`, `bun:*`, any Worker or Bun global, any
store, feed, server or render module, or read `process.env`, `Date.now`
or `Math.random`. The rule is checked by the owned test (D5 test 2),
which walks the import graph from each module and fails on the first
edge outside the rule, naming the module and the import.

Effect: a later edit that makes a contract module impure fails CI in
the aggregator suite before pm-service's bundle or behavior changes.

### D3. Where the hosted edition calls the contracts

Each hosted route that serves a mobile schema is a thin adapter:

1. read the tenant-store rows it needs (pool, link records, Metadata
   copies, receipts),
2. map them to the contract input types (`WorkViewInput`,
   `RelationsInput`, `ActivityInput`, the action envelope),
3. call the D1 function with `opts.now` taken once per request,
4. serialize the result unchanged.

Step 2 is the only hosted-specific logic. Its mapping functions live
in pm-service (`src/work/adapters.ts`, owned by the route slice, §9)
and are themselves covered by D5 test 4.

### D4. Named divergences

| Element                        | Laptop                                | Hosted                                                             | Kind                                                                                                     |
| ------------------------------ | ------------------------------------- | ------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------- |
| Source of `WorkViewInput`      | collector snapshots on disk / Turso   | tenant-store rows                                                  | Store location only, same as #9079 §6 link records                                                       |
| Fleet broker read (`claim`)    | absent until CLI-13 or REL-02 lands   | absent; no broker reachable from a Worker in V1                    | Both report `absent/not_sourced`; none silent                                                            |
| Session binding (`session`)    | absent until MOB-07                   | absent; a hosted session is not a work session                     | Both report `absent/not_sourced`                                                                         |
| Action apply                   | MOB-05 apply behind the MOB-09 switch | refused: hosted `mutationsEnabled` is `false` until #12831's lease | **Capability not supported**: the hosted route returns the MOB-09 `refused: "mutations_disabled"` result |
| `freshness` of fixture sources | `fixtures`                            | `fixtures`                                                         | None; a hosted fixture store never reports `live`                                                        |

The unsupported capability is reported in the schema the client
already reads (MOB-09's refusal), not by a missing route or an error
page. A client cannot tell a hosted refusal from a laptop refusal with
the switch off, which is the parity property.

### D5. The conformance test and its cases

`hosted-mobile-contract.test.ts` imports each D1 module twice: once by
the aggregator path and once through a small pm-service entry
(`pm-stack/pm-service/src/work/contracts.ts`, added by the route
slice) that re-exports them. Until that entry exists, the test's
hosted half resolves the same files and the closure test still runs.
The named tests:

1. `hosted_mobile_contract_matches_shared_golden` (the red test). For
   every case in the §4.4 manifest, the hosted call returns JSON
   byte-identical to the case's committed golden output.
2. `hosted_contract_modules_are_pure`. The D2 walk passes for all five
   modules, and a planted fixture module importing `node:fs` fails it
   with a message naming that import.
3. `hosted_and_laptop_resolve_one_implementation`. The re-exported
   functions are the same function objects as the aggregator's
   (`toBe`), so no second copy exists in the bundle graph.
4. `hosted_adapters_map_tenant_rows_to_contract_inputs`. A tenant DB
   built with pm-service's own migrations (`bun:sqlite` adapter) and
   loaded with the `all-live-eligible` and `conflicting-facets` rows
   produces the same `WorkViewInput` as the fixture input file.
5. `hosted_apply_reports_mutations_disabled`. The hosted apply path
   with `mutationsEnabled=false` returns `refused: "mutations_disabled"`
   and an unchanged `accepted` envelope.
6. `hosted_never_reports_live_for_fixture_store`. A fixture-backed
   tenant DB yields `freshness: "fixtures"`.
7. `hosted_unknown_snapshot_version_is_invalid`. An input row with an
   unknown `snapshot_version` gives that facet's `invalid` state in
   both editions.
8. `hosted_big_ids_round_trip`. `work_id` and revisions above 2^53
   stay decimal strings through the tenant row mapping.

Negative cases required by the issue, and where each is covered:

| Negative case            | Covered by                                                                  |
| ------------------------ | --------------------------------------------------------------------------- |
| stale input              | golden `conflicting-facets` (project facet `stale`) in test 1               |
| duplicate / replay       | golden MOB-05 replay case (same operation key → original receipt) in test 1 |
| ambiguous resource       | golden `renamed-repo-same-number` and the MOB-09 `matchedObjects 2` case    |
| unauthorized actor       | golden MOB-05 lost-role refusal in test 1                                   |
| partial remote effect    | golden MOB-09 two-effect partial case in test 1                             |
| unknown version          | test 7 and the MOB-09 `lamport` revision kind case                          |
| no false verified result | golden MOB-09 unbacked `verified_complete` case; test 5                     |

The test lists its names with `bun test --list`-equivalent output
before running: a filter that matches zero tests is a failure, per the
issue.

### 4.4 Manifest

`test/fixtures/hosted-mobile-contract/manifest.json` lists cases as
`{name, contract, input, golden}` pointing at the owning slices'
fixture files. It copies no fixture. A case whose files do not exist
yet is listed with `"pending": "<owning slice>"`, and test 1 fails if
a pending case's files appear without the entry being updated, so the
manifest cannot silently lag the slices.

### D6. Relation to slice 8 (#12833)

#12833 pins the #9060 read API (projects, item, roadmap, releases).
This record pins the mobile work contracts. They share the rule (same
input, same bytes) and nothing else: different fixtures, different
test files, different owners. If #12833 lands a shared harness first,
test 1 may use it; this record does not depend on it.

## 5. Gates

1. This record merged.
2. MOB-01 code on main (#15188), so `work-projection.ts` exists.
   Tests 1, 4, 6, 7 and 8 need it. Tests 2, 3 and 5 can land first
   over MOB-05 and MOB-09 alone.
3. MOB-02 and MOB-08 code on main for their manifest cases; until then
   those cases stay `pending`.
4. Hosted routes (§9) and the hosted apply switch stay gated on #12832
   (identity) and #12831 (writer lease). This slice adds no route.

## 6. Requirement trace

| ID     | How this record serves it                                                                                       |
| ------ | --------------------------------------------------------------------------------------------------------------- |
| H-R005 | Hosted compatibility is a test over shared goldens (D5), with unsupported capabilities reported in-schema (D4). |
| H-R023 | The record, its oracles and its gates are committed; implementation stays open until the tests pass on main.    |

## 7. Compatibility and rollback

Additive. No existing export, route, schema or fixture changes. The
test reads fixtures owned by other slices and writes nothing. Removing
the test and manifest removes the check and nothing else. A hosted
route built on D3 is rolled back by removing the route; the contract
modules stay where they are.

## 8. Non-goals

- Any hosted route, deployment or live acceptance (each needs its own
  evidence and approval).
- Hosted mutations: apply stays refused in the hosted edition until
  the writer lease (#12831) and identity (#12832) land.
- Porting `activity.ts` or `outcome-view.ts` stores; only the pure
  MOB-08 module crosses.
- The #9060 read API parity (slice 8, #12833).
- Normalizing `expedite=15` / `next-up=25`; they stay source values.

## 9. Follow-up slice (to be filed when gates 2 and 4 clear)

`impl(pm-service): hosted work view and action routes over the shared
contracts` — owns `pm-stack/pm-service/src/work/contracts.ts`,
`src/work/adapters.ts`, the route wiring in `worker.ts`, and its own
pm-service tests. Acceptance: D5 tests 1 to 8 pass with the hosted
half resolving through `contracts.ts`, and the hosted apply route
returns the D4 refusal.
