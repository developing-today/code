# MOB-09 — Desired/observed reconciliation, freshness and rollback

Design record for
[#9476](https://github.com/developing-today/code/issues/9476)
(`pmo-9196:MOB-09`), slice MOB-09 of
`components/9201-pm-projection-mobile-control.md` in
`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`.

The slice's failing test: "fake Project writer accepts request but
remote readback remains old; UI presently must not call it verified.
Recovery test disables mutations yet leaves pending intent
inspectable/replayable." Named red test:
`work_verified_effect_requires_source_readback` in
`pm-stack/aggregator/test/work-operation-reconciliation.test.ts`.

This record fixes when an action may be called verified, what a
stale or partial read does, what "mutations off" does, and what
shadow mode compares. It authorizes no deploy, no Project write and
no live run.

## Governing decisions consumed

- **Slice oracle**, quoted: "fault-injected desired→pending→conflict/
  verified states; second independent read confirms every 'verified'
  fixture. Shadow mode compares live projection to legacy board with
  no writes; rollback config disables new mutation adapter and retains
  receipts/action intents for later review." Negatives: "stale input,
  duplicate/replay, ambiguous resource, unauthorized actor, partial
  remote effect, unknown version, and no false verified/completed
  result". "No new durable store."
- **C-R082**, the issue's one curated requirement: "Show operation
  accepted, running, pending external sync, blocked, conflicted, and
  verified complete separately".
- **MOB-01** (`doc/2026-11-25T01-21-26Z_design_mob01_work_read_model/`):
  the fact envelope with `revision`, `observed_at` and `freshness`
  (`live | stale | fixtures`); "A stale value is never zeroed, dropped
  or relabelled live"; an `observed_at` revision is the weakest kind
  and is `revision_unavailable` for consequential writes; the
  freshness roll-up and `mutation_blockers`.
- **MOB-05** (#9472, closed): `pm-stack/aggregator/src/work-action-contract.ts`,
  the envelope lifecycle and its declared edges.
- **MOB-06** (#9473, model merged in #15246):
  `pm-stack/aggregator/src/mobile-work-actions.ts`; "`Done` is never
  requestable because visible Done needs a verified receipt."
- **CEN-04 §3.14**: the aggregator action store is a request ledger
  owned by the aggregator, "not canonical applied state".
- **AUTH-01** (merged): the Project's single writer is
  `updateProjectV2ItemFieldValue` in `hosted-worker/src/github.ts`.

## Verified state on `origin/main` `510e50de`

- `src/work-action-contract.ts` (MOB-05) has the states `draft,
previewed, accepted, running, pending_sync, partial, conflict,
uncertain, rejected, verified_complete` and these edges out of the
  middle states:
  - `running → pending_sync | partial | conflict | uncertain | rejected | verified_complete`
  - `pending_sync → verified_complete | partial | uncertain`
  - `partial → verified_complete | conflict`
- `applyWorkAction(envelope, actor, authority, report)` takes the
  downstream's own `report.outcome` and moves `running` to it. A
  downstream that reports `verified_complete` on acceptance is taken at
  its word. Nothing on main moves an envelope out of `pending_sync` or
  `partial`. That is the gap the slice's failing test names.
- `src/mobile-work-actions.ts` (MOB-06) plans one verb into a MOB-05
  request; it does not report outcomes.
- `src/work-operation-reconciliation.ts`, its test, and MOB-01's
  `src/work-projection.ts` do not exist. `src/work-items.ts` and
  `src/render.ts` (the current board) do.
- The aggregator has no mutation on/off setting.
- #8580 (GitHub mirror via webhook Worker) is closed; #8581 (fleet
  verb contract v2) is open.

## 1. Intent

An accepted write is not an applied write. The Project writer can
answer 200 and the Project can still show the old value, through
eventual consistency, a later overwrite, or a write that never landed.
MOB-09 makes "verified" mean "read back from the source of record and
seen", keeps every other state visible as itself, and gives an
operator a switch that stops new writes without losing what is in
flight.

## 2. Ownership

| Concern                                                                                                               | Owner                                                         |
| --------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------- |
| `reconcileWorkAction`, readback rules, drift results, shadow compare, `work-operation-reconciliation.ts` and its test | **MOB-09 (this)**                                             |
| Freshness display of operation state in `render.ts`                                                                   | **MOB-09**                                                    |
| Envelope states and edges                                                                                             | MOB-05's `work-action-contract.ts`                            |
| Which verbs exist and their policy                                                                                    | MOB-06                                                        |
| The read model, facts and freshness                                                                                   | MOB-01                                                        |
| Writing to the Project                                                                                                | the single writer (AUTH-01)                                   |
| Where receipts persist                                                                                                | the aggregator action store's owner (CEN-04 §3.14)            |
| The mutation switch's home                                                                                            | the aggregator's existing config; named by the implementation |

## 3. Verified means read back

### 3.1 The rule

`reconcileWorkAction(envelope, desired, readbacks, opts)` is pure. It
moves an envelope in `running`, `pending_sync` or `partial` to
`verified_complete` only when, for every desired effect, there is a
readback that:

1. comes from the source of record for that field (the Project item
   for a Project field, the issue for a label or comment), named by
   MOB-01's `SourceKind`;
2. is not the write's own response. A readback carries a `read_id`
   from a separate read; the write response cannot be passed as one;
3. was observed after the effect was issued, and carries a revision
   of a known kind (`work_version`, `updated_at`, `etag`,
   `item_revision`) that is newer than the revision the preview read.
   A revision of kind `observed_at` alone is not enough (MOB-01 D4);
4. has freshness `live`. A `stale`, `fixtures` or `error` read never
   verifies;
5. names the same object ref as the action. A readback for another
   object, or one that matches two objects, is `ambiguous` and never
   verifies;
6. shows the desired value.

A write response that says "done" moves the envelope to
`pending_sync`, never further. The recommended call shape is that the
caller passes `pending_sync` to `applyWorkAction` for every remote
effect and lets the reconciler promote it. Because the reconciler
cannot stop a caller from passing `verified_complete` today,
`reconcileWorkAction` also re-checks an envelope already in
`verified_complete`: if it holds no qualifying readback, the result
is flagged `unbacked_verified` and the display treats it as
`pending_sync` (§5). Changing MOB-05's `applyWorkAction` to refuse a
direct `verified_complete` is proposed to that file's maintainer,
not made here.

### 3.2 Outcomes of one reconciliation pass

| Readback for an effect                                  | Result for that effect |
| ------------------------------------------------------- | ---------------------- |
| qualifies (§3.1) and shows the desired value            | `verified`             |
| qualifies and shows the value from before the write     | `not_yet`              |
| qualifies and shows a third value, newer than the write | `drift`                |
| stale, fixtures, error, `observed_at`-only, or none     | `unread`               |
| for another object, or ambiguous                        | `ambiguous`            |
| unknown revision kind or unknown schema version         | `unknown_version`      |

The envelope's next state, using only MOB-05's declared edges:

| Effects                                                         | From `running` / `pending_sync`             | From `partial`      |
| --------------------------------------------------------------- | ------------------------------------------- | ------------------- |
| all `verified`                                                  | `verified_complete`                         | `verified_complete` |
| some `verified`, the rest `not_yet` or `unread`                 | `partial`                                   | stays `partial`     |
| none `verified`, all `not_yet` or `unread`, before the deadline | stays `pending_sync`                        | —                   |
| any `drift`                                                     | `uncertain` (reason `drift`)                | `conflict`          |
| any `ambiguous` or `unknown_version`                            | `uncertain`                                 | stays `partial`     |
| `not_yet` or `unread` past `sync_deadline`                      | `uncertain` (reason `sync_deadline_passed`) | stays `partial`     |

`pending_sync → conflict` is not a declared edge, so a drift seen
from `pending_sync` is `uncertain` with reason `drift`, and the drift
result is kept. Adding that edge is a proposal for MOB-05's file; if
it lands, the table's `drift` row maps to `conflict` from both
columns.

`sync_deadline` is per source, from `opts`, with no default in code;
the implementation sets it from the source's observed lag in staging.
Passing it never produces `verified_complete`.

### 3.3 Drift results

Every pass returns, per effect, `{ operation_key, field, desired,
observed, observed_revision, observed_at, read_id, result }`. These
are the "readback/drift receipts" the goal names. They are values, not
a new store. Persisting them beside the action record is the action
store owner's decision under CEN-04 §3.14. Until then they are
recomputed on each pass from fresh reads, which is safe because a
pass is pure.

### 3.4 The second independent read

The oracle requires that every `verified` fixture is confirmed by a
second, independent read. In tests, every case that ends
`verified_complete` performs a second read through a separate fake
reader instance and asserts the same value and a revision no older
than the first. In production one qualifying read verifies; the
second read is a test oracle, not a runtime requirement.

## 4. Mutations off (rollback)

One setting, `mutations_enabled`, read at each gate:

- **Off:** `applyWorkAction` is not called. The caller returns the
  envelope as `rejected` with reason `mutations_disabled` before any
  effect. Preview and accept may still run, so an operator can see
  what would happen.
- Envelopes already `accepted` stay `accepted`, inspectable, and can be
  applied after the switch is turned back on, still subject to MOB-05's
  version and role re-reads.
- Envelopes in `pending_sync` or `partial` keep reconciling, because a
  readback is a read, not a mutation. They may become
  `verified_complete`, `conflict` or `uncertain` while off.
- Nothing is deleted: action records, receipts and intents stay for
  review.
- A replay of an applied operation key returns its original receipt
  (MOB-05) whether the switch is on or off.

## 5. Display (C-R082)

`render.ts` shows each state as itself, with the freshness of the last
read:

| State                                      | Shown as                                                           |
| ------------------------------------------ | ------------------------------------------------------------------ |
| `accepted`                                 | Accepted                                                           |
| `running`                                  | Running                                                            |
| `pending_sync`, or `unbacked_verified`     | Pending external sync, with the last read's age, or "not read yet" |
| `partial`                                  | Partly applied, listing what is verified and what is not           |
| `conflict`, or `uncertain` with `drift`    | Conflicted, with desired and observed values                       |
| `uncertain` (other reasons)                | Not confirmed, with the reason                                     |
| blocked (MOB-01 `mutation_blockers`)       | Blocked, with the blocking facet                                   |
| `verified_complete` with a qualifying read | Verified complete                                                  |

No state borrows another's label, and only the last row may show a
completion mark.

## 6. Shadow mode

`shadowCompare(view, legacyRow)` compares a MOB-01 work view with the
current board's row for the same item and returns named differences
(status, urgency, blockers, assignee, freshness). It writes nothing.
Shadow mode is the aggregator running with `mutations_enabled` off and
the comparison logged for review. A test wraps every writer in a
counting fake and asserts zero calls across a shadow run.

## 7. Tests

In `pm-stack/aggregator/test/work-operation-reconciliation.test.ts`,
run with `bun test test/work-operation-reconciliation.test.ts` from
`pm-stack/aggregator`, plus `bunx tsc --noEmit`.

| Test                                                             | Asserts                                                                                     |
| ---------------------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| `work_verified_effect_requires_source_readback` (first, red)     | writer accepts, readback shows the old value → `pending_sync`, not verified                 |
| `work_write_response_is_not_a_readback`                          | passing the write response as a readback is refused                                         |
| `work_readback_promotes_to_verified_with_second_read`            | qualifying readback → `verified_complete`; a second fake read agrees                        |
| `work_stale_error_or_fixture_read_never_verifies`                | each freshness other than `live` → `unread`                                                 |
| `work_observed_at_revision_never_verifies`                       | weak revision → `unread`                                                                    |
| `work_drift_from_pending_is_uncertain_and_from_partial_conflict` | §3.2's drift rows                                                                           |
| `work_partial_multi_effect_reports_partial`                      | one of two effects verified → `partial`, with both results                                  |
| `work_sync_deadline_passes_to_uncertain`                         | past deadline → `uncertain`, never verified                                                 |
| `work_ambiguous_resource_never_verifies`                         | readback for another or two objects → `ambiguous`                                           |
| `work_unknown_version_is_uncertain`                              | unknown revision kind → `unknown_version`                                                   |
| `work_direct_verified_without_readback_is_flagged`               | an envelope `applyWorkAction` set to `verified_complete` with no read → `unbacked_verified` |
| `work_mutations_off_refuses_apply_before_effect`                 | effect sink count 0                                                                         |
| `work_mutations_off_keeps_pending_inspectable_and_replayable`    | accepted stays accepted; replay returns the receipt; pending keeps reconciling              |
| `work_shadow_mode_makes_no_writes`                               | every writer fake counts 0                                                                  |
| `work_every_transition_is_a_declared_edge`                       | each reconciler transition is in `WORK_ACTION_EDGES`                                        |

Duplicate/replay and unauthorized-actor negatives are MOB-05's and
stay tested there; this file asserts only that reconciliation adds no
effect and needs no actor.

## 8. Gates

- Code: the pure module and its tests can land once this record
  merges, with MOB-05 and MOB-06 already on main. It changes no
  existing file except the `render.ts` display rows.
- The mutation switch's home is named by the implementation, in the
  aggregator's existing configuration.
- Persisting drift results waits for the action store owner.
- Live Project proof is separately recorded: a real write and a real
  readback, authorized on its own. Mocks never stand for deployment.

## 9. Requirement trace (design-level)

| Requirement | Where                                                                 |
| ----------- | --------------------------------------------------------------------- |
| C-R082      | §5: six operation states shown separately; §3 keeps "verified" honest |

## 10. Non-goals

- No new store, queue, writer or service.
- No change to MOB-05's edges; the two changes it would want are
  proposals (§3.1, §3.2).
- No deploy, no live Project write.

## 2026-11-25T22:40Z Correction:

§4's first bullet said that with mutations off, apply "returns the envelope
as `rejected` with reason `mutations_disabled`". That contradicts the same
section's promise that the accepted request can be applied once mutations
are back on: MOB-05's `rejected` is terminal, so a rejected envelope can
never be applied again.

The implementation (`pm-stack/aggregator/src/work-operation-reconciliation.ts`,
`applyIfMutationsEnabled`) does this instead:

- Off: no authority call, no effect, no transition. The envelope comes back
  unchanged in `accepted`, with the result
  `{ applied: false, refused: "mutations_disabled" }`.
- On: MOB-05's `applyWorkAction`, with every one of its gates.

The tests `mutations off refuses apply before any effect` and
`mutations off keeps the envelope accepted and replayable once back on`
are the oracles for this.
