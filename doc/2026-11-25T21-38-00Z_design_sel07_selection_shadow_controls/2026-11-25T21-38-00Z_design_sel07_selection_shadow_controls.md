# SEL-07 — Read-only selection shadow, kill switch and rollback tests

Issue: [#9497](https://github.com/developing-today/code/issues/9497)
(slice `SEL-07` of [#9204](https://github.com/developing-today/code/issues/9204),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement.**
This record elaborates the last slice of the selection component in
the merged design package
(`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
PR #9225) into the comparison, control, report and rollback contract
a separately scoped implementation issue can be filed against. The
component row says only: "Shadow-vs-reference diff; kill switch;
read-only rollback; no priority changes; operational approval
recorded separately." This record _decides_ what the shadow compares,
what the controls are and which values they may take, what "kill"
does to every part of the selection program, what rollback means,
and what a later activation proposal has to bring. It authorizes no
code, schema, migration, deployment, live acceptance, cutover or
activation; those remain separately gated stages (§10).

Governing decisions consumed, not re-opened:

- **Component #9204**, "Decision frame", "Eligibility and
  deterministic normal selection" and "Rollback / safety boundary":
  normal selection stays deterministic highest effective urgency
  among eligible work; eligibility is a gate with typed exclusion
  reasons, not a weight; exploration samples without claiming;
  "Start with offline contract, then read-only shadow selection and
  comparison; keep production feature disabled until approved.
  Existing urgent selection and claim behavior remain fallback.
  Rollback must disable exploration/disposition writes without
  rewriting priorities or breaking claims."
- **Drew's ruling on the priority model (2026-11-24, on #9199)**:
  selection is a comparator chain with bounded exploration last, and
  "claim order tonight is unchanged." SEL-07 is the instrument that
  shows what the selection program would do while that stays true.
- **FDR-PMO-008** (exploration, fairness and calibration, proposed)
  and **FDR-PMO-002** (priority profiles): exploration constants are
  hypotheses; the ratified `expedite = 15` / `next-up = 25` bands are
  consumed, never renumbered.
- **`architecture.md`, "Cross-target selection order"**: the
  `fleet.selection-order/v1` object and its keys. SEL-07 adds no key
  to it.
- **SEL-06, landed** (#9496 closed;
  `pkgs/fleet/crates/fleet-queue/src/work/selection.rs`):
  `FairCandidate`, `FairnessState`, `LaneFairnessPolicy`,
  `NoFairness` ("the interface's representation of today's live
  behaviour") and `BoundedShareFairness`. The module comment: "no
  change to live selection: the broker keeps choosing by its
  existing path until a later slice wires a policy in."
- **SEL-02 record**
  (`doc/2026-11-25T01-18-23Z_design_sel02_claim_revalidation/`):
  the claim transaction is the consistency boundary; a selection
  never becomes a lease without revalidation; an exploration draw
  does not claim.
- **PRI-07 record**
  (`doc/2026-11-25T21-05-00Z_design_pri07_priority_shadow/`): the
  priority shadow compares two _orderings of one eligible set_, reads
  the live order from the production `ORDER BY` rather than
  re-modelling it, classifies every difference in a closed set with
  `unexplained` as the alarm, and proves read-only against a store
  fingerprint. SEL-07 reuses that report for ordering differences
  and does not re-implement it (§3.3).

Verified state at writing (2026-11-25, origin/main):

- No dated SEL-07 record exists under `doc/`. The only SEL-07 text
  on main is the #9225 package (component slice row, census and the
  preserved issue body). This record is the first.
- `work/selection.rs` holds SEL-06 only (fairness interface and two
  model policies, 297 lines). It has no reference selector, sampler,
  disposition state, shadow, control type or test named
  `pmo_selection_rollback_keeps_live_values`.
- **The live pick** is in `turso_queue/lifecycle.rs`. Per call it
  runs the `bundle_cap_gate` (ordinary picks only), then loops:
  `SELECT id FROM tasks WHERE 1=1 AND payload IS NOT NULL AND
payload <> '' AND payload <> '{}' AND ( {pick_predicate} )
{PLATFORM_LIMIT_PICK_CLAUSE}{scope.sql} ORDER BY (priority IS
NULL), priority ASC, created_at ASC LIMIT 1`, then claims that id
  with a conditional `UPDATE` that re-checks eligibility at the
  claim instant, skipping or quarantining a defective head (#9609
  budgets). `pick_predicate` is `default_pick_predicate()` or, for
  the claimant's sink collection, `handoff_pick_predicate()`. Per
  #8187 neither mode matches the claimer's platform against the
  item; per #8097 lane filters arrive as `scope.sql`.
- Gate state: PR #9225 MERGED. SEL-06 #9496 CLOSED. SEL-01 #9491,
  SEL-02 #9492, SEL-03 #9493, SEL-04 #9494 and SEL-05 #9495 OPEN.
  PRI-07 #9485 OPEN with its record merged (#15236). #6533 OPEN. Per
  the sibling pattern (PRI-07, SEL-02, HYD-04 landed as records while
  their prerequisites stood open), these gate the implementation
  issue (§10), not this record.

## 1. Intent

Every other selection slice builds a part that could one day choose
work differently from today: a reference selector with typed
eligibility (SEL-01), claim revalidation (SEL-02), an exploration
sampler (SEL-03), durable draw dispositions (SEL-04), a simulator
(SEL-05) and a fairness policy (SEL-06). None of them may change what
a lane claims tonight. SEL-07 is the slice that lets those parts run
**beside** the live pick, on the real queue, while guaranteeing they
cannot affect it:

1. a **shadow**: for a pinned snapshot and lane context, what the
   live pick would take next, what the reference selection program
   would take, and every difference between them with its reason
   (§3, §4);
2. **controls** with a closed set of values, the most restrictive of
   which (the kill switch) is the default and wins every conflict
   (§5);
3. a proof, by test, that the shadow and the controls write nothing
   and change no live value or claim (§6);
4. a definition of rollback, for the shadow and for the parts of the
   program that do write (SEL-04 dispositions), and tests for it
   (§7);
5. the evidence and approval an activation proposal must bring, so a
   shadow run is never read as permission (§8).

## 2. Ownership: who writes what

| Concern                                                             | Owner                     | SEL-07's relation                                                                    |
| ------------------------------------------------------------------- | ------------------------- | ------------------------------------------------------------------------------------ |
| Reference eligibility (both predicates, hard gates, reason codes)   | SEL-01 (#9491)            | Calls it; computes no eligibility of its own. `not_present` until it lands (§3.5).   |
| Claim revalidation and the claim transaction                        | SEL-02 (#9492), broker    | Never enters it. The shadow takes no claim and no lease.                             |
| Exploration profile and fixed-mass sampler                          | SEL-03 (#9493)            | Calls it in `shadow_draw` mode with a pinned seed; persists nothing (§3.4).          |
| Draw/disposition state machine and its writes                       | SEL-04 (#9494)            | Gates its writes through the control (§5.4); owns none of them.                      |
| Offline simulator and statistical oracles                           | SEL-05 (#9495)            | None. SEL-05 is offline evidence; SEL-07 is on-queue evidence. Both feed §8.         |
| Lane fairness policy                                                | SEL-06 (landed)           | Calls the configured policy; `NoFairness` is the live-equivalent baseline.           |
| Ordering differences within one eligible set                        | PRI-07 (#9485)            | Delegates to its report (§3.3); does not reclassify ordering.                        |
| Store adapter that fills the snapshot; command that prints a report | CLI-13 registration slice | Defines the snapshot and report types; the adapter and command are not this slice's. |
| **Shadow comparison, divergence classes, report, controls, kill**   | **SEL-07 (this)**         | Owns them, in `work/selection.rs`.                                                   |

The split with PRI-07 is the one that matters. PRI-07 takes the
eligible set as given and asks "in what order?". SEL-07 asks "which
item, from which set?": it compares the live pick's **eligible set
and head** with the reference program's, so it can say whether a
difference comes from admission (eligibility), from order, from
fairness, or from exploration. When the two sets agree and only the
order differs, SEL-07 hands that set to PRI-07 and embeds its
report's digest; one difference is classified exactly once.

## 3. What is compared

### 3.1 The live side

For a pinned snapshot `σ` and a lane context `λ` (the lane, its
`scope.sql` filter and its bundle-cap state):

- **`E_L` — live eligible set.** The rows `σ` returns for the live
  pick's `WHERE` clause for `λ`, in default (not handoff) mode. The
  clause is **read, not re-modelled**: the implementation runs the
  same `pick_sql` text the production pick formats, with `LIMIT 1`
  removed, inside a read-only transaction over `σ`. A Rust model of
  that SQL would compare two models and prove nothing about the
  queue.
- **`L` — live order** of `E_L` under the production `ORDER BY`,
  with equal sort keys recorded as unordered tie groups (as PRI-07
  §3.1).
- **`h_L` — live head.** The first row of `L`, or the first tie
  group, or `none`. If the existing packet check that the pick loop
  uses to skip or quarantine a defective head is a pure function,
  the shadow applies it read-only and walks past failing rows
  exactly as the loop would, recording each as
  `live_head_defective`; if it is not pure, `h_L` is the first row
  of `L` and the report says `packet_check: not_applied`. The shadow
  never quarantines, retires or skips a row in the store.
- **Bundle cap.** If `bundle_cap_gate` would refuse `λ`, the live
  side is `refused(bundle_cap)` and the report compares nothing
  further for that lane: the reference program has no different
  answer to a launch that may not pick at all.

Handoff picks (`handoff_pick_predicate`) are the claimant's sink
collection, governed by #6533 and #8234. They are **out of scope**:
the shadow reports only default-mode picks.

### 3.2 The reference side

Over the same `σ` and `λ`:

- **`E_R` — reference eligible set**, from SEL-01's selector with
  its typed exclusion reason for every row of `σ` it leaves out
  (closed/completed/cancelled, `manual_only`, unresolved hard
  dependency, missing capability, incompatible ownership, active
  lease or concurrency limit, policy-disabled, incomplete graph,
  task requirement not satisfied, claimer filter not matched).
- **`R` — reference order** of `E_R` by the shared comparator
  (PRI-01/02/05's `order_candidates`), then the configured SEL-06
  policy's `service_order` over the lane's candidates.
- **`h_R` — reference head**, the first item of that service order.

### 3.3 Divergence classes

Every row of `σ` that is in exactly one of `E_L`, `E_R`, or whose
head status differs, carries exactly one **primary class**, the
first in this table that explains it, and may list others. The set
is closed; a new class is a schema version bump.

| Class                 | Meaning                                                                                                                                                       | Expected today                      |
| --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------- |
| `reference_excluded`  | In `E_L`, not in `E_R`; carries SEL-01's reason code. The live pick would admit a row the design excludes (e.g. `manual_only`, an unresolved dependency).     | Yes, wherever SQL lacks a gate      |
| `live_excluded`       | In `E_R`, not in `E_L`; carries the live clause that excluded it (payload empty, timer not due, platform limit, `scope.sql`).                                 | Possible                            |
| `live_head_defective` | `h_L` would be skipped or quarantined by the pick loop's packet check (§3.1).                                                                                 | Only with defective packets         |
| `order` (delegated)   | `E_L = E_R` on the lane's candidates and the heads differ by order alone. The PRI-07 report over that set classifies it; SEL-07 records its digest and class. | Yes (PRI-07's `live_tie_unordered`) |
| `fairness`            | Sets and comparator order agree; the configured SEL-06 policy served a different lane's item first.                                                           | Only with a non-null policy         |
| `exploration`         | Informational only: in `shadow_draw` mode, the item SEL-03 would have drawn. Never counted as a head difference (§3.4).                                       | Only in `shadow_draw`               |
| `unexplained`         | A set or head differs and no class above accounts for it.                                                                                                     | **Never** — a defect                |

`unexplained` is the alarm, as in PRI-07: a parity bug in one side,
a snapshot read at two moments, or a live clause the reference does
not know (or the reverse). The report still returns with the count;
the oracles of §9 fail any fixture run that produces one.

`reference_excluded` is expected to be the largest class on the real
queue for as long as the live SQL lacks gates the design names (the
V1 baseline: no `manual_only` or dependency check in the pick
predicate). That is the point of the shadow: it measures exactly how
many claims the gates would have changed before anyone switches them
on.

### 3.4 Exploration in the shadow

SEL-03's sampler is feature-disabled. In `shadow_draw` mode (§5.1)
the shadow calls it once per report with the pinned profile and a
**seed derived deterministically** from the report's pinned inputs
(the snapshot digest, the lane and the profile version), so the same
inputs always draw the same candidates. It records the urgent-gate
result, the candidate ids and their first-draw probabilities. It
writes no `ExplorationRun`, no disposition and no hint; it starts no
claim. An exploration draw is a distribution, not a head, so the
`exploration` class never counts towards `heads_agree` or `moved`.

### 3.5 Parts that do not exist yet

Each reference part reports one of `applied`, `not_present` (no
implementation at this build) or `unavailable` (present, but
returned no answer for `σ`, such as `graph_evaluation_incomplete`).
The states go into the report's pinned inputs, so reports with
different part states never share a fingerprint, and the summary
names every part that was not `applied`. With SEL-01 `not_present`
the shadow cannot compute `E_R`; it then reports
`reference: not_present` and compares nothing, rather than treating
the live set as the reference. That is the honest state of the
program today.

### 3.6 Measures

Per lane context, and summed over all contexts in one request:

- **`heads_agree`** — `h_R` is `h_L` or inside `h_L`'s tie group.
  The one number that says whether tonight's next claim for that
  lane would differ.
- **`admission_delta`** — counts of `reference_excluded` and
  `live_excluded` rows, per reason code.
- **`moved`** — count of primary classes other than `exploration`.
- **`first_divergence`** — the first live position at which the
  sets or orders differ, with its class.

No measure is a pass/fail threshold here; thresholds are the
activation proposal's to set (§8).

## 4. The report

Type `SelectionShadowReport`, schema
`fleet.selection-shadow-report/v1`, pure and serialisable (canonical
JSON), carrying:

- `pinned` — snapshot ref and digest, `as_of_utc`, policy and
  profile versions, comparator ref, fairness policy id, exploration
  profile version when `shadow_draw`, and the §3.5 part states;
- `controls` — the effective control values that produced the
  report (§5);
- `lanes` — one entry per lane context: `live`
  (`E_L` digest, `L` as work refs with tie groups, `h_L`,
  packet-check state or `refused(bundle_cap)`), `reference`
  (`E_R` digest, exclusions with reason codes, `h_R`), `differences`
  (`{ work, primary_class, other_classes, reason_code_or_clause,
live_position, reference_position }`), the embedded PRI-07 report
  digest when the `order` class is present, the `exploration` draw
  when present, and `measures`;
- `writes: 0` — a constant the type cannot set otherwise (§6).

The report is **an observation**. It is not a `fleet.selection-order`
object any claim path accepts, not a `ClaimIntent`, not an
`ExplorationRun`, not a reason-coded event and not a queue write. It
registers no reason code and no digest domain beyond its own schema
name. Whoever runs it may print it or attach it to an issue; the
slice persists nothing (§9).

## 5. Controls and the kill switch

### 5.1 The two controls and their values

```text
SelectionControls = {
  selection:   live_only | shadow,
  exploration: disabled  | shadow_draw,
}
```

- `selection = live_only` — the reference program is not invoked at
  all on the pick path; this is today's behaviour.
- `selection = shadow` — the reference program runs beside the live
  pick as §3 describes; the live pick still decides.
- `exploration = disabled` — SEL-03 is not invoked and SEL-04 may
  start no run (§5.4).
- `exploration = shadow_draw` — §3.4. Only meaningful with
  `selection = shadow`; with `live_only` it is treated as `disabled`.

**There is no value that lets the reference program decide a claim
and no value that enables exploration.** Those are activation, which
needs Drew's recorded approval (§8). Adding such a value is a change
to this type in a separately approved issue, so a configuration
mistake, a stale file or a copied setting cannot activate anything:
there is nothing for it to set.

### 5.2 Where the values come from

The controls are an **input** to the selection entry point, pinned
into each report, like `as_of`. They are read from the claimant's
existing configuration surface; the CLI-13 registration slice owns
the glue that reads them, and the implementation issue names the
exact existing surface. This slice adds no table, column, queue
setting or service to hold them (§9).

### 5.3 Fail-closed rules

1. **Default is the kill.** Absent, empty, unparseable or unknown
   values resolve to `selection = live_only`,
   `exploration = disabled`, with a typed warning naming the value
   that was rejected. Nothing ever defaults to `shadow`.
2. **Most restrictive wins.** If more than one scope supplies a value
   (process, lane, repository), the effective value is the most
   restrictive (`live_only` over `shadow`, `disabled` over
   `shadow_draw`). A kill at any scope cannot be overridden by an
   enable at another.
3. **Next pick, no restart.** The effective value is read per pick
   request. A kill takes effect at the next pick; a shadow
   computation already running when the kill arrives is discarded
   unreported. It never affected the pick, so discarding it is safe.
4. **The shadow cannot delay or fail a claim.** The shadow runs in
   its own read-only transaction over `σ`, never inside the claim
   transaction, and under a time and row budget the implementation
   issue sets. Over budget, an error or a panic in the reference
   program ends that report as `shadow_incomplete` with the reason;
   the live pick's result, latency and error behaviour are
   unchanged. The live pick never waits for the shadow.

### 5.4 What the kill does to the parts that write

SEL-04 is the only part of the selection program that writes
(durable `ExplorationRun` records and dispositions). The component's
rollback clause requires that disabling exploration "disable[s]
exploration/disposition writes without rewriting priorities or
breaking claims." With `exploration = disabled` (the default and the
kill):

- **No new run starts.** SEL-04's entry point checks the effective
  control and returns a typed `exploration_disabled` result without
  writing.
- **Existing records stay.** Runs and dispositions already persisted
  remain durable and readable; nothing deletes, rewrites or
  "resolves" them. A pending run is left pending.
- **An in-flight claim finishes normally.** An item already in
  `claim_attempt` goes through SEL-02's ordinary revalidated claim;
  the kill neither cancels it nor lets it bypass revalidation.
- **No priority changes.** No disposition hint is ever applied to an
  urgency value by the kill, by rollback or by re-enabling. Applying
  a hint is a separate authorized triage decision under #9199.

These are obligations on SEL-04's implementation that this record
names so both slices test the same thing; SEL-07's tests (§9) check
them against a SEL-04 fixture once SEL-04 exists, and are marked
`not_present` until then.

## 6. Read-only, proven

"Live values unchanged" is the acceptance, so it is tested against a
store, not inferred from signatures:

1. The shadow function takes a snapshot value, the live clause text,
   the reference parts and the controls, and returns a report. It
   takes no writer and no claim handle.
2. `pmo_selection_rollback_keeps_live_values` (§9) seeds an
   in-memory SQLite queue from a fixture, records a byte-level
   fingerprint of every row of every table, runs N live picks with
   `selection = live_only`, rebuilds the store, runs the same N
   picks with `selection = shadow` and `exploration = shadow_draw`
   producing a report before each pick, and asserts: the claim
   sequences are identical; the store fingerprints after each run
   are identical; and every report has `writes = 0`.
3. Report construction under a priority-write gate, a disposition
   writer and a claim function that each panic if called completes.

## 7. Rollback

1. **Shadow off leaves no trace.** Going from `shadow` to
   `live_only` changes no stored value, no claim and no later pick:
   the same claim-sequence comparison as §6.2, with the switch made
   halfway through the N picks.
2. **Nothing depends on a report.** No claim, `ClaimIntent`,
   selection-order object, priority write, explanation, disposition
   or journal entry references a shadow report. The order and claim
   types have no shadow field; the test compiles the claim path with
   the shadow module excluded and asserts it accepts and rejects the
   same inputs.
3. **Exploration kill keeps history.** With a SEL-04 fixture: start
   runs under a test-only enabling path, kill, and assert that no
   new run starts, existing runs and dispositions are byte-identical,
   an in-flight `claim_attempt` completes through revalidation, and
   no urgency value changed.

## 8. What activation would need (not authorized here)

Activation is any separately approved change that lets the reference
program decide claims (turning on SEL-01's gates, a fairness policy,
or exploration). This record proposes none. It fixes the evidence an
activation proposal must carry, so a shadow run is never cited as
approval:

1. Shadow reports over the live queue at distinct times the proposal
   chooses, for every lane context the activation affects, each with
   `unexplained = 0`, and every part it relies on `applied`.
2. For every `reference_excluded` reason code present, the items
   that show it and a statement that excluding them is intended;
   for `live_excluded`, the same. These change who gets work tonight
   and each needs an explicit yes.
3. PRI-07's cutover evidence (its §8) wherever the activation
   changes order rather than admission.
4. For exploration, SEL-05's offline statistical evidence with its
   preregistered intervals, plus shadow draws showing the urgent
   gate held (zero urgent displacement) on the live queue.
5. **Operational approval recorded separately**: Drew's explicit
   approval of the activation, recorded on the activation issue,
   naming the control value being added to §5.1. The 2026-11-24
   ruling does not give it: "claim order tonight is unchanged."

## 9. Persisted fields and acceptance

**Persisted fields: none.** The report is recomputed from a pinned
snapshot; the controls are inputs. Storing reports or a control in
the queue would be a second source of truth and a new write path.
CEN-04 §5's field-admission gate is not engaged, and no Queue Table
Improvements or #9074 schema disposition is needed.

All tests live in `work/selection.rs`'s test module and run with the
slice body's command form,
`cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker <name>`.
Each name must first appear in that command's `-- --list` output; a
filter that matches zero tests is not acceptance. Builds and tests go
through remote dispatch, per the repository's remote-build rule; no
local compile is an acceptance record.

| Test                                                          | Oracle                                                                                                                                                                                                   |
| ------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `pmo_selection_rollback_keeps_live_values` (first, red)       | §6.2: identical claim sequences and store fingerprints with the shadow off and on; `writes = 0` on every report.                                                                                         |
| `pmo_selection_shadow_live_side_from_production_sql`          | `E_L` and `L` come from the production `pick_sql` text, read from `turso_queue/lifecycle.rs` via `include_str!` and asserted present there; a guard fails if that text changes without this test.        |
| `pmo_selection_shadow_classifies_every_difference`            | Generated fixtures mixing `manual_only`, unresolved dependencies, empty payloads, future timers, scoped lanes, equal sort keys and two lanes: every difference has one primary class, `unexplained = 0`. |
| `pmo_selection_shadow_order_delegates_to_pri07`               | Equal sets, different heads: the difference is `order`, carries the PRI-07 report digest and PRI-07's class, and is not counted twice.                                                                   |
| `pmo_selection_shadow_reference_not_present_compares_nothing` | With SEL-01 absent: `reference: not_present`, no differences, the summary names the missing part; the live set is never used as the reference.                                                           |
| `pmo_selection_shadow_draw_writes_nothing`                    | `shadow_draw`: the same pinned inputs yield the same draw; no run, disposition or claim is written; `exploration` never changes `heads_agree`.                                                           |
| `pmo_selection_controls_fail_closed`                          | Absent, empty, unknown and malformed values resolve to `live_only`/`disabled` with a typed warning; `live_only` at any scope beats `shadow` at another.                                                  |
| `pmo_selection_kill_takes_effect_next_pick`                   | A kill between two picks: the second pick runs no reference part; a shadow in flight at the kill is discarded unreported; neither pick's result changes.                                                 |
| `pmo_selection_shadow_cannot_delay_or_fail_claim`             | A reference part that panics, errors or exceeds budget: the report is `shadow_incomplete`; the live pick returns the same result and error as with the shadow off.                                       |
| `pmo_selection_exploration_kill_keeps_history`                | §7.3 against a SEL-04 fixture (`not_present` until SEL-04 lands, and then required).                                                                                                                     |
| `pmo_selection_shadow_never_reaches_writers`                  | §6.3: construction under panicking priority-write, disposition-write and claim functions completes.                                                                                                      |

Negative coverage from the slice body, mapped: stale input → a live
side and reference side that name different snapshots are rejected
with a typed `StaleSnapshot` and no report; duplicate/replay → same
pinned inputs yield a byte-identical report, draw included; ambiguous
resource → canonical `(authority_id, work_id)` identity on both
sides, equal numbers in different repositories never conflated (a
fixture in `classifies_every_difference`); unauthorized actor → the
controls have no value an actor could set to activate anything, and
there is no write path to authorize; partial remote effect → none, the
slice has no remote effect; unknown version → an unknown policy,
profile or report schema version fails with a typed error and no
report; no false verified/completed result → `not_present`,
`shadow_incomplete` and `unexplained` keep an incomplete comparison
from reading as agreement.

## 10. Gates and sequencing

Implementation against this record starts only when **all** of these
hold:

1. This record is merged.
2. A separately scoped implementation issue is filed against it,
   carrying §9's oracles as its acceptance. #9497 itself authorizes
   no code; it stays open as the slice tracker.
3. The slice body's hard edges are honoured: SEL-01 (#9491) lands the
   reference selector the shadow calls, and SEL-06 (landed) is the
   shared-file predecessor on `work/selection.rs`. SEL-02 through
   SEL-05 also write that file; whichever lands later rebases. The
   shadow is correct with SEL-03 and SEL-04 absent (§3.5), so they
   gate only `shadow_draw` and §7.3, not the first red test.
4. The live `pick_sql` is still the single text
   `live_side_from_production_sql` reads. If #9178, PRI-03 or SEL-02
   changes it first, the implementation reads the new text.
5. #6533's live launch-and-bind acceptance stays separate; nothing
   here duplicates or stands in for it.

Running a shadow against the live queue is a read. Running it on a
schedule would be a new schedule and needs its own approval.

**Schedules:** this record creates, changes, and retires no schedule
on any surface.

## 11. Requirement trace (design-level)

Full per-occurrence text for every allocated ID is in
`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/census/requirements-crosswalk.jsonl`;
this table records which part of this design speaks to each ID:
design coverage, not implementation.

| ID     | Where addressed                                                                                                                                                   |
| ------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| C-R102 | §3.2: the reference order is the shared comparator, lower sooner; §3.3 measures where the live head and the reference head disagree.                              |
| C-R103 | §3.3 `exploration`, §3.4, §8.4: bucket-corrected draws are observed in shadow only, so their remaining large-cluster advantage is measured before any activation. |
| C-R105 | §3.5: importance is not invented from urgency; an axis mixture needs its own data and slice, and a missing part reports `not_present`.                            |
| H-R001 | §2, §5.4: selection (priority track) and dispositions (lifecycle track) keep separate owners; the kill gates both without merging them.                           |
| O-R022 | §2: the broker keeps the claim, SEL-04 keeps disposition writes, CLI-13 keeps the adapter; the shadow is a read-only observer of each.                            |
| P-R008 | §2, §4: the Queue Broker's responsibility is unchanged; the report is an observation, not a journal, mutation or outbox entry.                                    |
| P-R027 | §6, §9: acceptance against a realistic seeded queue and multi-lane pick sequences, with a store fingerprint, not a mock-only claim.                               |
| S-R009 | §5.1, §8: the controls carry no activation value, so the mechanism stays open to change until evidence and approval choose it.                                    |

## 12. Non-goals

- No activation, cutover or rollout; no change to the pick SQL, the
  claim path or any stored value; no claim that the selection program
  is ready because a shadow ran (§8).
- No eligibility computation (SEL-01), claim revalidation (SEL-02),
  sampler (SEL-03), disposition state machine (SEL-04), simulator
  (SEL-05) or fairness policy (SEL-06): the shadow calls what exists.
- No ordering classification of its own (PRI-07).
- No handoff-pick shadow (#6533, #8234).
- No store adapter, CLI command, schedule or PM projection (CLI-13).
- No new queue, database, table, column, service, resolver,
  executable or GitHub client, and no persisted field.
- No change to #6533 or its live acceptance.
