# PRI-07 — Read-only priority shadow report

Issue: [#9485](https://github.com/developing-today/code/issues/9485)
(slice `PRI-07` of [#9199](https://github.com/developing-today/code/issues/9199),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement.**
This record elaborates the shadow slice of the merged design
package (`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
PR #9225) into the comparison, report, read-only and rollback
contract a separately scoped implementation issue can be filed
against. The component row says only: "Shadow diff uses pinned
snapshot/version; live values unchanged; rollback tests pass; no
production rollout in this issue." This record _decides_ what is
compared with what, how a difference is classified, what the
report carries, how "read-only" is proven rather than asserted,
what rollback means for a slice that writes nothing, and what
evidence a later cutover proposal has to bring. It authorizes no
code, schema, migration, deployment, live acceptance or cutover;
those remain separately gated stages (see §10).

Governing decisions consumed, not re-opened:

- **Component #9199**, the `PRI-07` slice row and "Verification /
  migration boundary": migration compares without mutating live
  priorities, shadow-evaluates first, preserves current numeric
  values and proves rollback; a mock passing is not live queue
  evidence; no rollout, schema change or queue-semantic change is
  authorized by the design.
- **Drew's ruling on the priority model (2026-11-24, recorded on
  #9199 comment of 08:11Z)**: selection is a comparator chain —
  eligibility gate, effective pressure, peer rank, importance as a
  tie-break inside equal-urgency peer groups only, bounded
  exploration — and "the live integer remains the operational sort
  key while the temporal fields land and importance is collected
  null-first — claim order tonight is unchanged." The shadow is the
  instrument that shows what the chain _would_ do while that
  sentence stays true.
- **FDR-PMO-002** (priority profiles, ordering and provenance):
  intrinsic values are canonical, effective values are derived, and
  #7030's ratified `expedite = 15` / `next-up = 25` bands are
  consumed, never renumbered or snapped.
- **The package `architecture.md`**, "Cross-target selection order":
  `fleet.selection-order/v1` with `queue_snapshot_ref`,
  `graph_revision`, `policy_ref`, `comparator_ref`, `as_of_utc`,
  `lane_context`, `complete_candidate_set` and `ordered_items
{ work_id, work_version, effective_urgency, peer_rank, created_at }`.
  The shadow's candidate order _is_ such an object; this record adds
  no key to it.
- **PRI-01 / PRI-02 / PRI-05a–d, landed** (PR #14733,
  `pkgs/fleet/crates/fleet-queue/src/work/priority.rs`):
  `PriorityProfile::queue_v1`, `validate_value` (off-profile is a
  typed error, never snapped), `CandidateKey` / `compare_candidates`
  / `order_candidates` (urgency ascending, untriaged last, peer rank
  with `None = 0`, creation time, canonical `(authority_id, work_id)`
  identity), `effective_priority` (opportunity boosts),
  `explain_priority` and `PinnedInputs`. PRI-07 is a consumer of
  those, not a second implementation of any of them.
- **PRI-03 record** (`doc/2026-11-23T15-34-52Z_design_pri03_peer_ordering_rank_rebalance/`)
  owns peer rank and comparator parity in `turso_queue.rs`; **PRI-04
  record** (`doc/2026-11-25T01-11-48Z_design_pri04_dependency_pressure/`)
  owns inherited urgency and says, in its §9, "Wiring the evaluator
  into the live claim order is not part of the implementation issue:
  it is PRI-07's shadow report first, then a separately approved
  cutover." This record is that shadow.

Verified state at writing (2026-11-25, origin/main `25f58546c`):

- No dated PRI-07 record exists under `doc/`; the only PRI-07 text
  on main is the #9225 package (component slice row, census,
  preserved issue body). This record is the first.
- `work/priority.rs` exists (PRI-01, PRI-02, PRI-05a–d). It contains
  no shadow, no report type and no test named
  `pmo_priority_shadow_preserves_live_values`. Its module comment
  says "later slices (PRI-02 … PRI-07) consume it."
- **The live claim order** is the SQL `ORDER BY` of the production
  pick in `turso_queue/lifecycle.rs`:
  `ORDER BY (priority IS NULL), priority ASC, created_at ASC LIMIT 1`.
  It has **no identity tie-break**: two eligible rows with equal
  `priority` and equal `created_at` are ordered however SQLite
  returns them. The test harness pick in `turso_queue.rs`, the admin
  listing in `turso_queue/admin.rs` and the replica pick in
  `work/sqlite_replica.rs` all add `id ASC` / `task_id ASC`; the
  production pick does not.
- `tasks.priority` is `INTEGER` (nullable); `tasks.created_at` is
  `TEXT`, so the live order sorts creation time **lexically**, while
  `CandidateKey.created_at` is an integer clock.
- No peer-rank column exists on the queue (`peer_rank` occurs nowhere
  in `turso_queue*`), so every live candidate's rank reads as the
  legacy `None = 0`.
- No dependency-pressure evaluator exists (PRI-04 #9480 is a record
  only; `work/relations.rs` does not exist).
- `work/contracts.rs` registers the digest domain
  `fleet.selection-order/v1`.
- Gate state: PR #9225 MERGED. PRI-01 #9478, PRI-02 #9479 and
  PRI-05a–d #9481–#9484 CLOSED. PRI-03 #9511, PRI-04 #9480,
  DEP-02 #9503, DEP-03 #9443, #9178 and #14170 OPEN. The slice
  body's shared-file edge names PRI-04 as this slice's predecessor
  on `work/priority.rs`. Per the sibling pattern (PRI-03, PRI-04,
  SEL-02, REL-01 all landed as records while their implementation
  prerequisites stood open), these gate the implementation issue
  (§10), not this record.

## 1. Intent

Before the claim order is ever switched from "the integer, then
creation time" to the comparator chain, Drew and the lanes should be
able to see, on the real queue, which item would be claimed next
under each, how far the two orders differ, and _why_ each difference
exists — while nothing about the live queue changes. That is all
this slice is:

1. a pure function that takes a pinned snapshot of the eligible set,
   the live order that the production pick produced over it, and the
   pinned policy inputs, and returns a **shadow report**: the
   comparator-chain order, the live order, and every difference
   between them with the reason that explains it (§3, §4);
2. a proof, by test, that producing the report writes nothing and
   changes no live value (§5);
3. a definition of rollback for a capability that writes nothing,
   and a test that exercises it (§7);
4. the evidence a later cutover proposal must bring, so that the
   shadow is not mistaken for authorization (§8).

## 2. Ownership: who writes what

| Concern                                                                 | Owner                     | PRI-07's relation                                                               |
| ----------------------------------------------------------------------- | ------------------------- | ------------------------------------------------------------------------------- |
| Profile, value validation, comparator, explanation, boosts              | PRI-01/02/05 (landed)     | Calls them; re-implements none.                                                 |
| Peer rank and comparator parity in `turso_queue.rs`                     | PRI-03 (#9511)            | Reads rank when it exists; `None = 0` until then.                               |
| Inherited (dependency) urgency                                          | PRI-04 (#9480)            | Reads it when it exists; reports `unavailable` until then (§3.4).               |
| Eligibility (status, deps satisfied, holds, manual-only, budget, scope) | Live pick / #9178 / SEL-* | Consumes the live pick's eligible set as given; computes no eligibility.        |
| Reading the queue store into a snapshot                                 | CLI-13 registration slice | Defines the snapshot type; the store adapter that fills it is not this slice's. |
| Bounded exploration / sampling                                          | #9204                     | Out of the shadow (§3.5).                                                       |
| **Shadow comparison, divergence classes, report type, read-only proof** | **PRI-07 (this)**         | Owns them, in `work/priority.rs`.                                               |

The eligibility row matters most. The shadow compares two
**orderings of the same eligible set**; it never asks "is this item
eligible?" itself. If it did, every difference would be ambiguous
between "ordered differently" and "admitted differently", and the
report could not tell a comparator change from a gate change. A
future change to eligibility is SEL-_/REL-_'s to shadow, not this
slice's.

## 3. What is compared

### 3.1 The two orders

For a pinned snapshot `σ` (§6):

- **L — the live order.** The eligible rows of `σ` in the order the
  production pick's `ORDER BY` produces. L is an **input**, read by
  running that `ORDER BY` over the snapshot, never re-derived in
  Rust: a Rust model of the SQL would compare two models and prove
  nothing about the queue. Where the store yields equal sort keys,
  L records them as one **unordered tie group** rather than inventing
  an order SQLite did not promise.
- **S — the shadow order.** The same rows ordered by
  `order_candidates` over `CandidateKey`s built as follows:
  - `work` — the row's canonical `(authority_id, work_id)`;
  - `urgency` — the row's **effective** value for the pinned
    `as_of`: intrinsic from `profile.validate_value(priority)`
    (`NULL` → `None`, untriaged), then the strongest active
    opportunity boost via `effective_priority`, then, once PRI-04
    exists, its inherited component combined by PRI-04 §3.5–§3.6;
  - `peer_rank` — the stored rank once PRI-03 exists, else `None`;
  - `created_at` — the row's creation time parsed to the integer
    clock (§3.3).

S is emitted as a `fleet.selection-order/v1` object (no new key)
with `comparator_ref` naming the V2 comparator and `policy_ref`
naming the pinned profile, policies and the pressure state of
§3.4. Its digest uses the already-registered domain.

### 3.2 Divergence classes

Every item whose position in S differs from its position in L (or
whose L position is inside a tie group S resolved) carries exactly
one **primary class**, the first in this table that explains it, and
may list others that also apply. Classes are a closed set; a new one
is a schema version bump of the report.

| Class                 | Meaning                                                                                                                      | Expected today                    |
| --------------------- | ---------------------------------------------------------------------------------------------------------------------------- | --------------------------------- |
| `off_profile`         | The stored integer is not a member of the pinned profile. L sorts it numerically; S cannot place it and holds it out (§3.6). | Yes — any non-member live value   |
| `live_tie_unordered`  | L had a tie group (equal priority and creation text); S resolves it by identity. Not a reorder: L promised no order.         | Yes — the missing `id ASC`        |
| `clock_text_order`    | L's lexical `created_at` and S's parsed clock disagree (mixed formats, offsets, precision).                                  | Possible                          |
| `unreadable_clock`    | `created_at` does not parse to the integer clock. Held out like `off_profile`.                                               | Possible                          |
| `opportunity_boost`   | An active boost moved the effective value (PRI-05b).                                                                         | Only where boosts are recorded    |
| `peer_rank`           | A non-zero peer rank reordered an equal-urgency group (PRI-03).                                                              | No — no rank column yet           |
| `dependency_pressure` | Inherited urgency moved the effective value (PRI-04).                                                                        | No — not built; state unavailable |
| `unexplained`         | A position differs and no class above accounts for it.                                                                       | **Never** — a defect              |

`unexplained` is the slice's alarm. It means S and L disagree for a
reason the comparator chain does not name: a parity bug in one of
them, a snapshot read from two different moments, or a sort key the
SQL has that the design does not (or the reverse). The report still
returns, with the count, and the oracle of §9 treats any
`unexplained` entry in a fixture run as a failure.

### 3.3 Creation time

`created_at` is `TEXT` in the store and an integer in `CandidateKey`.
The shadow parses it with the one parser the queue already uses for
its own timestamps; if no single parser exists in `fleet-queue`, the
implementation issue names one and tests it, and the shadow never
guesses a format. A value that does not parse is `unreadable_clock`
and is held out (§3.6). Where the parsed order and the lexical order
of two items disagree, the item is `clock_text_order`: that is a
real, current way the live order can differ from the designed one,
and cutover must not change it silently.

### 3.4 Derived components that do not exist yet

PRI-03's rank and PRI-04's pressure are not built. The report must
not read "no difference" as "the chain agrees": for each derived
component it states one of

- `applied` — the component exists at the pinned policy and was
  evaluated;
- `not_present` — no implementation exists at this build (rank,
  pressure today);
- `unavailable` — the component exists but returned no values for
  this snapshot (PRI-04's `graph_evaluation_incomplete`, a missing
  policy).

The states go into `policy_ref`, so two reports with different
component states never share a fingerprint, and the summary line of
the report names every component that was not `applied`.

### 3.5 What is deliberately not in S

- **Importance.** Drew's ruling puts importance inside equal-urgency
  peer groups only, and PRI-08 has not chosen how. Until a slice
  defines the tie-break, S does not use importance, and the report
  says `importance: not_present`. When that slice lands it adds its
  own class here, by schema bump.
- **Bounded exploration (#9204).** Sampling is deliberately
  non-deterministic per claim; a shadow of it would be a
  distribution, not an order. Out of scope.
- **Directives (`do_now`).** Not urgency (C-R045). If a directive
  bypasses order in the live pick, it does so before `ORDER BY`, as
  eligibility; the shadow inherits that from L's eligible set.
- **Lane scope and platform limits.** These narrow the eligible set
  (`scope.sql`, `PLATFORM_LIMIT_PICK_CLAUSE`). The shadow reports one
  order per `lane_context` it is given; with no scope it is the
  default pick's whole eligible set.

### 3.6 Items S cannot place

An `off_profile` or `unreadable_clock` row has no valid
`CandidateKey`. S **holds it out** — it appears in the report's
`held_out` list with its class and its L position, and is absent
from S's `ordered_items`, with `complete_candidate_set: false`. The
shadow does not snap the value to a neighbour (the PRI-01 defect it
exists to prevent), does not sort it "where the integer would go",
and does not drop it silently. A cutover is impossible while any
live row is held out, because the V2 comparator would refuse the set
(§8).

### 3.7 Measures

The report carries, for the whole eligible set and for the first
`k` positions (`k` from the request, default 10):

- **`next_claim_agrees`** — whether L's first item (or L's first tie
  group) contains S's first item. This is the one number that says
  whether tonight's claim would differ.
- **`moved`** — count of items whose position differs, per primary
  class.
- **`first_divergence`** — the first position at which the two
  orders differ, with that item's class.
- **`inversions`** — the number of item pairs ordered oppositely by
  L and S (pairs inside an L tie group are not counted), so a large
  structural change is not mistaken for a few swaps.

No measure is turned into a pass/fail threshold by this slice; the
thresholds for cutover are the cutover proposal's to set (§8).

## 4. The report

Type `PriorityShadowReport`, schema `fleet.priority-shadow-report/v1`,
pure and serialisable (canonical JSON), carrying:

- `pinned` — the existing `PinnedInputs` (profile id and version,
  policies, `snapshot_ref`, `as_of`) plus `graph_revision` when
  PRI-04 is `applied`;
- `lane_context` — as given;
- `live_order` — L as work refs, tie groups explicit;
- `shadow_order` — S as the `fleet.selection-order/v1` object, and
  its digest;
- `held_out` — §3.6 entries `{ work, class, live_position,
stored_value_or_text }`;
- `differences` — one entry per moved or tie-resolved item:
  `{ work, live_position, shadow_position, primary_class,
other_classes, explanation }`, where `explanation` is the
  `PriorityExplanation` PRI-02 already produces for that item;
- `components` — the §3.4 state of every derived component;
- `measures` — §3.7;
- `writes: 0` — a constant the type cannot set otherwise; see §5.

The report is **an observation**. It is not a selection-order object
that any claim path accepts (its `policy_ref` names the shadow, so a
claim that presented it would fail revalidation), not a queue write,
not a reason-coded event, and it registers no reason code. It may be
printed, attached to an issue, or stored as a file by whatever runs
it; the slice persists nothing (§9).

## 5. Read-only, proven

"Live values unchanged" is the slice's acceptance, so it is tested
against the store, not inferred from the function's signature:

1. The shadow function is pure: its inputs are a snapshot value, L,
   the profile and policies; it takes no connection, no client, no
   writer, and returns a value. That alone is the design-level
   guarantee.
2. The acceptance test (§9, `pmo_priority_shadow_preserves_live_values`)
   additionally seeds an in-memory SQLite queue from a fixture,
   records a byte-level fingerprint of every row of every table
   (values, `priority`, `created_at`, labels/progress text, row
   count), reads the snapshot and L through the live `ORDER BY`,
   builds the report, and asserts the fingerprint is identical
   afterwards and that the next real pick over the same store
   returns L's first item — the live order, not S's.
3. The report type has no field or method that converts it into a
   write, a claim argument or a `PriorityWrite`; the test asserts
   that `check_priority_write` is never reached by constructing the
   report under a gate that panics if called.

## 6. Pinning and staleness

A report is a pure function of `(snapshot_ref, L, profile key,
policies, graph_revision if any, as_of, lane_context)`. Two rules:

- **L and S come from one snapshot.** L must be read from the same
  `snapshot_ref` S is built from. If the adapter cannot guarantee
  that (it read L and the rows at different moments), it does not
  call the shadow; the shadow itself rejects a call whose L names a
  row not in the snapshot, or omits one that is, with a typed
  `StaleSnapshot` error and no report. This is the slice's "stale
  input" negative case, and the reason `unexplained` should never be
  produced by a well-formed call.
- **No implicit now.** `as_of` is an input; boosts and any time-based
  component are evaluated at it. Re-running with the same inputs
  yields the same report byte for byte, including its digest — the
  "duplicate/replay" negative case.

## 7. Rollback

The shadow writes nothing, so rollback cannot mean undoing a write.
It means two things, both tested:

1. **Disable leaves no trace.** With the shadow disabled (its
   caller not invoked, or the implementation's feature toggle off),
   the claim path, the pick SQL, every stored value and every other
   module behave identically to a build without the shadow. The test
   runs the same seeded store through N picks with and without a
   report being produced between picks and asserts identical claim
   sequences.
2. **Nothing depends on it.** No claim, selection-order object,
   priority write, explanation or journal entry references a shadow
   report. The implementation issue's review checklist names this;
   the test asserts that the claim path accepts and rejects exactly
   the same inputs with the shadow module compiled out of the
   candidate order (the order type has no shadow field).

## 8. What a cutover would need (not authorized here)

A cutover is the separately approved change that makes the claim
order use S. This record does not propose one. It fixes the evidence
a cutover proposal must carry, so the shadow cannot be cited as if
it were approval:

1. Shadow reports over the live queue, taken at distinct times
   chosen by the proposal, each with `unexplained = 0` and
   `held_out` empty — or a migration plan for every held-out row
   that preserves its value (no snapping) and has its own approval.
2. For every primary class present, a statement of whether the
   change in behaviour is intended, with the item that shows it.
   `live_tie_unordered` and `clock_text_order` in particular change
   tonight's claim order and need an explicit yes.
3. Every derived component the cutover relies on reported as
   `applied`; a component reported `not_present` or `unavailable`
   cannot be part of a cutover.
4. PRI-03's comparator parity work in `turso_queue.rs` landed, so
   that S and the SQL can agree by construction after cutover.
5. Drew's explicit approval of the cutover itself, which the
   ruling of 2026-11-24 does not give: "claim order tonight is
   unchanged."

## 9. Persisted fields and acceptance

**Persisted fields: none.** The report is recomputed from a pinned
snapshot; storing reports, or a "last shadow" column, would be a
second source of truth. CEN-04 §5's field-admission gate is not
engaged, and no Queue Table Improvements / #9074 schema disposition
is needed. A later slice that wants to keep reports for audit brings
its own admission entry.

All tests live in `work/priority.rs`'s test module and run with the
slice body's command form. Each test name must first appear in the
`-- --list` output of its exact command; a filter that matches zero
tests is not acceptance. Builds and test runs go through remote
dispatch, per the repository's remote-build rule; no local compile is
an acceptance record.

| Test                                                     | Oracle                                                                                                                                                                                                                                  |
| -------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `pmo_priority_shadow_preserves_live_values` (first, red) | §5.2: seeded in-memory queue; full-table fingerprint identical before and after building the report; the next real pick returns L's first item.                                                                                         |
| `pmo_priority_shadow_live_order_from_production_sql`     | L is produced by the production pick's `ORDER BY` text, read from `turso_queue/lifecycle.rs` via `include_str!` and asserted present there, not re-typed; a guard fails if that text changes without this test changing.                |
| `pmo_priority_shadow_classifies_every_difference`        | Generated fixtures up to 40 rows mixing profile members, ratified bands, `NULL`, equal creation texts, mixed timestamp formats and recorded boosts: every moved item has exactly one primary class from §3.2 and `unexplained` is zero. |
| `pmo_priority_shadow_tie_group_not_a_reorder`            | Two rows with equal `priority` and `created_at`: L reports one tie group; S orders them by identity; the item is `live_tie_unordered`, `inversions` is 0, `next_claim_agrees` is true when the group is first.                          |
| `pmo_priority_shadow_holds_out_off_profile`              | Stored values 12, 450 and `i64::MIN`: each is `held_out` as `off_profile` with its L position; none is snapped; `complete_candidate_set` is false; the rest of S is the order of the remaining rows.                                    |
| `pmo_priority_shadow_ratified_bands_pass_through`        | 15 and 25 are placed between 10/20 and 20/30 respectively in S, never moved to 10 or 20; with no other factor they produce no difference.                                                                                               |
| `pmo_priority_shadow_components_not_present_are_named`   | With no rank and no pressure implementation, `components` reports both `not_present`, the summary names them, and the digest differs from a run where a fixture marks pressure `unavailable`.                                           |
| `pmo_priority_shadow_rejects_mismatched_snapshot`        | L naming a row absent from the snapshot, or omitting one present, returns `StaleSnapshot` and no report.                                                                                                                                |
| `pmo_priority_shadow_replay_identical`                   | Same inputs twice: byte-identical canonical JSON and digest; a different `as_of` that crosses a boost expiry changes the report and the digest.                                                                                         |
| `pmo_priority_shadow_disable_changes_nothing`            | §7.1: N picks over the same seeded store with and without reports produced between them yield identical claim sequences.                                                                                                                |
| `pmo_priority_shadow_never_reaches_write_gate`           | §5.3: report construction under a priority-write gate that panics if called completes.                                                                                                                                                  |

Negative coverage from the slice body, mapped: stale input →
`rejects_mismatched_snapshot`; duplicate/replay →
`replay_identical`; ambiguous resource → canonical
`(authority_id, work_id)` identity in S, equal numbers in different
repos never conflated (fixture in `classifies_every_difference`);
unauthorized actor → there is no write path to authorize
(`never_reaches_write_gate`); partial remote effect → not
applicable, the slice has no remote effects; unknown version → an
unknown profile or policy version in `pinned` fails with the
existing typed `UnsupportedVersion`/`ProfileMismatch` and no
report; no false verified/completed result → `held_out`,
`components` and `unexplained` keep an incomplete comparison from
reading as agreement.

## 10. Gates and sequencing

Implementation against this record starts only when **all** of the
following hold:

1. This record is merged.
2. A separately scoped implementation issue is filed against it,
   carrying §9's oracles as its acceptance. #9485 itself authorizes
   no code; it stays open as the slice tracker.
3. The shared-file edge from the slice body is honoured: **PRI-04**
   (#9480) lands its `work/priority.rs` changes first. PRI-03
   (#9511) also writes `work/priority.rs`; whichever of PRI-03 and
   PRI-07 is implemented second rebases onto the first. Neither is a
   semantic prerequisite: §3.4 makes the shadow correct with rank
   and pressure absent.
4. The live pick's `ORDER BY` is still the single text §9's
   `live_order_from_production_sql` reads. If #9178 or PRI-03
   changes it first, the implementation reads the new text; it does
   not shadow a stale one.

The store adapter that fills the snapshot from the live queue and the
command that prints a report belong to the CLI-13 registration slice
(the slice body: "Shared CLI glue remains owned by the single CLI-13
registration slice"). Running the shadow against the live queue is a
read; running it on a schedule would be a new schedule and needs its
own approval.

**Schedules:** this record creates, changes, and retires no schedule
on any surface.

## 11. Requirement trace (design-level)

Full per-occurrence text for every allocated ID is in
`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/census/requirements-crosswalk.jsonl`;
this table records which part of this design speaks to each
allocated ID — design coverage, not implementation.

| ID     | Where addressed                                                                                                      |
| ------ | -------------------------------------------------------------------------------------------------------------------- |
| C-R047 | §3.1, §3.6, `holds_out_off_profile`, `ratified_bands_pass_through` — validate against live values, never reinterpret |
| H-R046 | §5, §7, §9 — acceptance, migration-evidence (§8) and rollback tests in the handoff                                   |
| C-R162 | §2, §8, §10 — current architecture map, gaps (no id tie, lexical clock), go/no-go evidence for cutover               |

## 12. Non-goals

- No cutover, no change to the pick SQL or any claim path, no
  production rollout, and no claim that the comparator chain is
  ready because a shadow ran (§8).
- No eligibility computation (§2), no exploration (§3.5), no
  importance tie-break (§3.5).
- No peer rank, rank storage or comparator parity (PRI-03); no
  inherited urgency (PRI-04); no profile, aging, boost, hysteresis
  or clump logic (PRI-01/02/05) — the shadow calls what exists.
- No store adapter, CLI command, schedule or PM projection
  (CLI-13, PRI-06).
- No new digest domain, reason code, queue, database, service,
  resolver, executable or GitHub client, and no persisted field.
- No change to #6533 or its live acceptance.
