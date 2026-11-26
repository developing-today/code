# SEL-04 — Durable exploration run and disposition state machine

Issue: #9494 (slice SEL-04 of #9204, program #9196). Status: design
record. It decides how an exploration draw becomes a durable run, how
each drawn candidate gets a disposition before any long work starts,
and how at most one of them reaches the ordinary claim. It writes no
code, schema or migration; those follow the gates in §9.

The two rules the slice body puts first are kept as hard invariants:

- **No early leases.** A draw reserves nothing. Every drawn candidate
  stays claimable by everyone else until the one selected candidate
  goes through SEL-02's ordinary revalidated claim.
- **No automatic score edits.** A disposition can carry a triage hint.
  Nothing in this slice ever writes a priority, urgency, rank or band.

## 0. Governing decisions consumed

| Source                                        | What this record takes from it                                                                                                                                                                         |
| --------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| #9494 slice text                              | Owned file `work/selection.rs` plus integration in `turso_queue`; red test `pmo_selection_disposition_replay_no_claim`; crash/replay, duplicate no-op, explicit conflicts, one claim by one candidate. |
| #9204 V2, "Durable draw/disposition"          | The `ExplorationRun` contents, the five dispositions, `drawn → disposed → selected → claim_attempt → claimed or stale or rejected`, idempotent by run, event and candidate.                            |
| #9204 V2, "Urgent-protection gate"            | Exploration only when no qualifying urgent work for this lane would be delayed, rechecked before the draw and before the claim.                                                                        |
| #9204 V2, "Performance and complexity"        | `O(k)` candidate and disposition rows per run, ids and versions only, typed overflow and no silent truncation.                                                                                         |
| #9204 "Rollback / safety boundary"            | The number of candidates sampled never implies a claim count; disabling exploration stops disposition writes without rewriting priorities or breaking claims.                                          |
| SEL-02 record (`2026-11-25T01-18-23Z`)        | Shape B `ClaimIntent` with source `exploration`, its typed outcomes, the urgent-gate query at commit, one `BEGIN IMMEDIATE` per claim, no remote effect inside it. SEL-02 §7 assigns runs here.        |
| REL-03 record (`2026-11-25T20-47-34Z`)        | Scope tuple `(principal_ref, authority_id, idempotency_key)`, digest over `intent_dto()`, replay and conflict rows, stored `operation_id`, a replay writes nothing.                                    |
| SEL-07 record (`2026-11-25T21-38-00Z`) §5     | `exploration = disabled` is the default and the kill; there is no value that enables exploration; §5.4's four obligations on this slice's writers.                                                     |
| CEN-04 record (`2026-11-23T13-51-45Z`) §3, §5 | A persisted field with no §3 row may not be written; `events` is the per-task timeline and not a journal; a new database or service is never admitted.                                                 |
| SEL-01, SEL-03 (#9491, #9493)                 | SEL-01's eligibility verdict and urgent predicate; SEL-03's versioned profile and fixed-mass sampler. This record persists their outputs and decides neither.                                          |
| Drew's ruling, 2026-11-24 (on #9196)          | Design-only gates without a hard blocker are void. SEL-04 keeps its real hard edges (§9).                                                                                                              |

## 1. Verified state on main `95a2bd0c`

- `pkgs/fleet/crates/fleet-queue/src/work/selection.rs` is 297 lines and
  holds SEL-06 only: `FairCandidate`, `FairnessState`, the
  `LaneFairnessPolicy` trait, `NoFairness`, `BoundedShareFairness` and
  their tests. It has no selector, sampler, run or disposition.
- No `ExplorationRun`, exploration table or `queue explore` command
  exists anywhere under `pkgs/fleet/`. The census row `queue.explore`
  says "explore absent from client.rs".
- The broker schema in `turso_queue/core.rs` has `events` (`id`,
  `worker_id`, `task_id`, `kind`, `detail`, `created_at`). `/events-prune`
  and `/sweep` call `prune_events`, which runs
  `DELETE FROM events WHERE created_at < ?` with a 30-day default. An
  `events` row is therefore not a durable home for anything that must
  outlive that window.
- `task_history` records status and priority transitions per task. It
  is an audit of task changes, not a run.
- `work/contracts.rs` `Command` already carries `operation_id`,
  `idempotency_key`, `request_digest`, `work`, `kind`, `arguments`,
  `actor_ref`, `expected_work_version` and `lease_generation`.
- No SEL-01, SEL-02, SEL-03 or REL-03 code is on main. Their records
  for SEL-02 and REL-03 are merged; SEL-03 has a queued build task
  under Drew's 2026-11-24 ruling.

## 2. Intent

An exploration draw is a question, not a reservation. The broker draws
up to `k` candidates from one pinned snapshot, records the draw so it
can be reproduced, and hands the candidates back. The lane decides
what to do with every one of them and records that decision before it
starts any long work. At most one candidate is selected, and that one
goes through the same revalidated claim every other chosen item uses.
If the world moved, the claim says so, the run closes, and the lane
goes back to normal selection. Whatever the lane learned about the
other candidates survives as a hint for triage, never as a score
change. Crashes and retries at any point leave either the old state or
the next state, never a second draw, a second claim or a lost
disposition.

## 3. Ownership

- **Owned:** the run and disposition model in
  `pkgs/fleet/crates/fleet-queue/src/work/selection.rs`, as pure types
  and functions with tests in its `mod tests`, and the broker
  integration in `turso_queue` (schema rows, verbs, and the one write
  inside SEL-02's claim transaction that closes the run).
- **Read-only:** SEL-01's eligibility verdict, reason codes and urgent
  predicate; SEL-03's profile, sampler and draw output; SEL-02's claim
  outcomes; REL-03's scope and receipt rules; SEL-07's controls.
- **Not owned:** CLI registration (CLI-13), the parser (CLI-02),
  authorization grants (AUTH-01), the work version and fence columns
  (REL-02), and applying any hint (#9199 triage).
- **Added nowhere:** no new queue, database, service, scheduler,
  executable or GitHub client. The tables in D10 live in the existing
  broker database and enter through CEN-04 §5.

## 4. Decisions

### D1. Starting a run

A run starts only on an explicit request (`queue explore`, D12). This
slice adds no automatic trigger to the pick path: an automatic 4% draw
is activation, which SEL-07 leaves without any control value. The
broker runs these steps in one `BEGIN IMMEDIATE`, in this order, and
each failing step returns a typed result and writes nothing:

1. **Control.** SEL-07's effective `exploration` value. Anything but
   an explicitly allowed value returns `exploration_disabled`.
2. **Parse, key and digest** (CLI-02, REL-03 §3.2 and §3.3).
3. **Authorization** of the authenticated principal for this lane at
   this authority (AUTH-01). `actor_ref` is attribution only.
4. **Idempotency lookup** (REL-03 §4). A replay returns the stored
   run, its candidates and their current dispositions, with
   `duplicate: true`. It never draws again.
5. **One open run per lane.** If the lane already has a run that is
   not closed and not past its deadline, the result is `run_open` with
   that run's id. A past-deadline run is closed first as D8 says, then
   the start continues.
6. **Snapshot and eligibility.** SEL-01's eligible set for this lane
   from one pinned snapshot. An empty set returns `nothing_to_explore`
   and writes nothing ("0 candidates emits no event", #9204).
7. **Urgent gate.** SEL-01's urgent predicate for this lane on the same
   snapshot. Any qualifying claimable item returns `preempted` with its
   ref and writes nothing. The caller does a normal pick.
8. **Profile.** SEL-03's profile by version. Unknown or invalid returns
   SEL-03's typed error. There is no fallback to a uniform draw.
9. **Draw.** SEL-03's sampler with the seed from D3 draws
   `min(count, eligible)` candidates without replacement.
10. **Persist** the run and its candidates (D2) in state `drawn`, and
    store the REL-03 receipt for the start operation.

Nothing in these steps writes a lease, a fence generation, a claim
event, a priority or a `task_history` row. A drawn candidate stays
claimable by every other lane and by normal selection.

### D2. What a run records

All integers and versions cross the wire as canonical decimal strings,
as `contracts.rs` already requires. Probabilities are stored as
canonical decimal strings with enough digits to round-trip the
sampler's `f64`; the oracle recomputes them from the pinned inputs.

**Run** (one row per run):

| Field                                                        | Meaning                                                                              |
| ------------------------------------------------------------ | ------------------------------------------------------------------------------------ |
| `run_id`                                                     | broker-assigned, monotonic per authority; never chosen by the caller                 |
| `authority_id`, `lane_id`, `principal_ref`                   | who explored, for which lane; `principal_ref` from the auth layer, never `actor_ref` |
| `start_operation_id`                                         | the REL-03 operation that created the run                                            |
| `state`, `outcome`, `run_revision`                           | D4; `run_revision` increments on every run write                                     |
| `queue_snapshot_revision`, `candidate_set_hash`, `as_of_utc` | the pinned eligible population (#9204 `SelectionInput`)                              |
| `policy_ref`, `profile_version`, `comparator_ref`            | the policy, SEL-03 profile and comparator in force at the draw                       |
| `graph_revision`                                             | recorded for explanation; SEL-02 keeps the graph live at claim                       |
| `urgent_gate`                                                | `clear` with its `checked_at`; a run never exists with any other value               |
| `rng_algorithm`, `seed`                                      | D3                                                                                   |
| `eligible_count`, `requested_count`, `drawn_count`           | `drawn_count < requested_count` is explicit, never padded                            |
| `started_at`, `dispose_by`, `claim_by`, `closed_at`          | store clock; the two deadlines are pinned at start (D8)                              |
| `selected_index`, `claim_operation_id`                       | set by D5 and D6                                                                     |

**Candidate** (one row per drawn candidate):

| Field                                     | Meaning                                                                        |
| ----------------------------------------- | ------------------------------------------------------------------------------ |
| `run_id`, `candidate_index`               | draw order, `0` to `drawn_count - 1`                                           |
| `work`                                    | `(authority_id, work_id)` pair; equal numbers at two authorities stay distinct |
| `work_version_at_draw`                    | the REL-02 work version in the snapshot; becomes `expected_work_version` (D6)  |
| `first_draw_mass`, `conditional_mass`     | `q_i` and the probability it had at its own draw step, from SEL-03             |
| `state`, `claim_outcome`, `claim_reasons` | D4, D6                                                                         |

**Disposition revision** (append-only, one row per write):

| Field                                         | Meaning                                                             |
| --------------------------------------------- | ------------------------------------------------------------------- |
| `run_id`, `candidate_index`, `revision`       | revision `1` is the first disposition; each supersede adds the next |
| `kind`                                        | one of the five in D5                                               |
| `reason_code`, `note`                         | required for `no_action`; optional elsewhere; `note` is bounded     |
| `hint`                                        | only on `triage` and `defer_or_reprioritize` (D5)                   |
| `evidence_refs`                               | refs only, never copied context                                     |
| `principal_ref`, `operation_id`, `written_at` | who wrote it, under which REL-03 operation, by the store clock      |
| `supersedes_revision`                         | empty on revision `1`; otherwise the revision it replaced           |

The candidate count is bounded by the profile's maximum; `note`,
`evidence_refs` and the hint have bounds the implementation issue
sets. Over any bound the write fails with
`disposition_payload_overflow` or `candidate_payload_overflow`. Nothing
is truncated.

### D3. The seed

`seed` is the SHA-256 of JCS over
`{authority_id, lane_id, run_id, candidate_set_hash, profile_version}`
under the digest domain `fleet.exploration-seed/v1`, registered with
DST-01's domain list. `rng_algorithm` names SEL-03's pinned generator.

Because `run_id` is assigned by the broker and the candidate set is
fixed by the snapshot, a lane cannot pick its own draw. Retrying a
start with the same key replays the same run (D1 step 4). Starting a
fresh run needs the open one closed first (D1 step 5), and every
closed run stays readable with its outcome, so a lane that keeps
abandoning runs to fish for a candidate is visible in `show` and in
SEL-05's measures. Recomputing the draw from the persisted inputs must
reproduce the candidates and masses exactly (§6).

### D4. The state machine

**Run states**

| State      | Meaning                                                                                   |
| ---------- | ----------------------------------------------------------------------------------------- |
| `drawn`    | persisted with its candidates; at least one candidate has no disposition                  |
| `disposed` | every candidate has a current disposition and exactly one is selected                     |
| `closed`   | terminal, with `outcome` one of `claimed`, `stale`, `rejected`, `no_selection`, `expired` |

There is no persisted `started` state: D1 is one transaction, so a run
is either absent or `drawn`.

**Candidate states**

| State           | Meaning                                                                    |
| --------------- | -------------------------------------------------------------------------- |
| `drawn`         | no disposition yet                                                         |
| `disposed`      | has a current disposition; final for every kind but `select_for_execution` |
| `claim_attempt` | the selected candidate entering SEL-02's claim (D6)                        |
| `claimed`       | SEL-02 returned `claimed`                                                  |
| `stale`         | SEL-02 returned `stale_item`, `stale_policy` or `preempted`                |
| `rejected`      | SEL-02 returned `ineligible` or `held`                                     |

**Transitions** (every other transition is refused with
`invalid_transition` and writes nothing):

| From (run, candidate)             | Event                               | To (run, candidate)                         |
| --------------------------------- | ----------------------------------- | ------------------------------------------- |
| absent                            | start (D1)                          | `drawn`, every candidate `drawn`            |
| `drawn`, `drawn`                  | first disposition                   | `drawn`, `disposed`                         |
| `drawn`, last `drawn` candidate   | disposition, one candidate selected | `disposed`                                  |
| `drawn`, last `drawn` candidate   | disposition, none selected          | `closed` `no_selection`                     |
| `drawn` or `disposed`, `disposed` | supersede (D5)                      | recomputed by the two rows above            |
| `disposed`, selected `disposed`   | claim (D6)                          | selected `claim_attempt`, then D6's outcome |
| `drawn` or `disposed`             | deadline passed (D8)                | `closed` `expired`                          |

Once a run is `closed`, or its selected candidate is in
`claim_attempt` or later, every disposition in the run is frozen:
writes return `run_frozen`.

### D5. Dispositions

| Kind                    | Means                                           | Carries                |
| ----------------------- | ----------------------------------------------- | ---------------------- |
| `select_for_execution`  | the lane intends to claim and work this one     | nothing extra          |
| `triage`                | worth a triage look; the lane is not working it | optional hint          |
| `defer_or_reprioritize` | the lane thinks its urgency is wrong            | optional hint          |
| `release_unchanged`     | back to the pool as it was                      | nothing                |
| `no_action`             | none of the above, said explicitly              | required `reason_code` |

`no_action` reason codes: `not_actionable_now`, `needs_information`,
`duplicate_of_other`, `run_expired` (system only, D8) and `other`
(requires `note`).

**At most one selection.** A run has at most one
`select_for_execution`. A second one is refused with `already_selected`
and the index already selected.

**Hints are evidence, never edits.** A hint is
`{direction: more_urgent or less_urgent or retriage, suggested_delta?,
evidence_refs}`. `suggested_delta` is a number a person or the triage
agent may read; nothing reads it as an instruction. Applying a hint is
a separate, authorized `reprioritize` under #9199 that cites
`(run_id, candidate_index)` as evidence. No path in this slice,
including expiry, the kill, rollback or re-enabling, writes a
priority. The component's -20, -10 and +10 examples stay examples.

**Idempotent by run and candidate.** A disposition write names
`(run_id, candidate_index)`:

- identical to the current revision (same kind, reason, hint and
  evidence): `disposition_unchanged`, nothing written, the current
  revision returned;
- different, with no `supersedes_revision`: `disposition_conflict`,
  nothing written, the current revision returned;
- different, with `supersedes_revision` equal to the current revision,
  by an authorized principal (the run's principal, or an operator with
  an AUTH-01 grant for this authority), with a `note`: a new revision;
- different, with any other `supersedes_revision`: `VERSION_CONFLICT`
  with the current revision.

A write under a REL-03 key that was already used for a different
disposition body is REL-03's `IDEMPOTENCY_KEY_REUSED`. Revisions are
never deleted or rewritten; the history of a candidate is its revision
list.

### D6. The one claim

`claim` takes a run, not an item. It is refused with `run_not_disposed`
unless the run is `disposed`, with `no_selection` if no candidate is
selected, and with `run_closed` or `run_frozen` as D4 says. The claim
is SEL-02's Shape B with:

- `work` = the selected candidate's pair;
- `expected_work_version` = `work_version_at_draw`;
- `observed_snapshot_ref` = the run's snapshot, policy and profile refs;
- `reason.code` = `exploration`;
- `operation_id` and the REL-03 key from the claim request; the run
  stores `claim_operation_id` the first time.

The broker runs it inside SEL-02's one `BEGIN IMMEDIATE`. As the first
write of that transaction it sets the selected candidate to
`claim_attempt` with a run-revision check, and as the last write it
records the outcome below. Both commit with the claim or with its
refusal event, or neither does. A crash therefore leaves the run
`disposed` with no attempt, and a retry of the same key is a first
acceptance; a lost response after commit is a REL-03 replay that
returns the stored outcome. There is no window in which a claim
happened and the run does not know.

| SEL-02 outcome                                                             | Candidate                     | Run                        |
| -------------------------------------------------------------------------- | ----------------------------- | -------------------------- |
| `claimed`                                                                  | `claimed`                     | `closed` `claimed`         |
| `stale_item`, `stale_policy`, `preempted`                                  | `stale`, with SEL-02's refs   | `closed` `stale`           |
| `ineligible`, `held`                                                       | `rejected`, with reason codes | `closed` `rejected`        |
| `unauthorized`, `ambiguous_ref`, `wrong_authority`, `idempotency_conflict` | unchanged                     | unchanged; nothing written |
| `unsupported`                                                              | unchanged                     | unchanged; nothing written |

A `stale` or `rejected` run never tries another candidate. The caller
goes back to a normal Shape A pick, as #9204 and SEL-02 §5.3 require.
Exploring again needs a new run, with its own gate check and draw.

Remote effects of a successful claim (assignment, Project fields) go
through REL-04's outbox after commit, as SEL-02 §4.5 says. A run
outcome of `claimed` means the lease was granted, not that any remote
effect is verified.

### D7. Crash and replay

| Interrupted at                    | State after restart                   | What a retry does                                                                                                             |
| --------------------------------- | ------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| inside the start transaction      | no run                                | same key is a first acceptance; a new snapshot may draw differently                                                           |
| after start commit, response lost | `drawn`                               | same key replays the same run and draw                                                                                        |
| between dispositions              | `drawn`, some candidates `disposed`   | the lane reads the open run (`show`, or `run_open` on a new start) and continues; repeated writes are `disposition_unchanged` |
| after the last disposition        | `disposed` or `closed` `no_selection` | claim when ready, or nothing to do                                                                                            |
| inside the claim transaction      | `disposed`, no attempt                | same claim key is a first acceptance and claims at most once                                                                  |
| after claim commit, response lost | `closed` with the outcome             | same claim key replays the stored outcome; no second generation bump                                                          |
| the lane never returns            | `drawn` or `disposed`                 | D8 closes it as `expired`                                                                                                     |

### D8. Deadlines and expiry

`dispose_by` and `claim_by` are pinned in the run at start from this
slice's configuration. The implementation issue sets their values; a
missing or non-finite deadline is a typed configuration error, not an
open-ended run.

An expired run is closed by the broker, never by a timer this slice
adds. It happens at the next D1 step 5 for the same lane, or in the
existing `/sweep`. Closing writes, for each candidate still `drawn`, a
`no_action` disposition with reason `run_expired`, principal
`system:expiry`, so every drawn candidate ends with a durable,
explicit disposition. A selected candidate that was never claimed keeps
its `select_for_execution` revision; the run's `expired` outcome says
the claim did not happen. Expiry writes no lease, priority or hint.

### D9. Obligations under SEL-07's kill

With `exploration = disabled` (the default and the kill):

- D1 step 1 returns `exploration_disabled` before any read or write.
- Runs and dispositions already stored stay as they are and stay
  readable through `show`. Nothing deletes, rewrites or resolves them.
- Disposition writes on an open run are refused with
  `exploration_disabled`, so the kill also stops this slice's writes.
  D8 expiry still runs, because it only finishes records that already
  exist and changes nothing outside them.
- A claim already inside SEL-02's transaction finishes normally. A new
  `claim` on a `disposed` run is refused with `exploration_disabled`;
  the run later expires.
- No hint is applied by the kill, by rollback or by re-enabling.

These are the obligations SEL-07's
`pmo_selection_exploration_kill_keeps_history` checks against this
slice's fixture.

### D10. Where runs are stored

| Candidate home                             | Verdict and reason                                                                                     |
| ------------------------------------------ | ------------------------------------------------------------------------------------------------------ |
| `events` with a run `kind`                 | refused: `prune_events` deletes rows after 30 days by default; per-task, not run-shaped (CEN-04 §3.6)  |
| `task_history`                             | refused: an audit of task status and priority changes; a draw changes neither                          |
| new columns on `tasks`                     | refused: a run is not a fact about one task, and the slice must not touch task fields that QTI may own |
| a new database, queue or service           | refused by the slice body and CEN-04 §5                                                                |
| three additive tables in the broker schema | **proposed**: `exploration_runs`, `exploration_candidates`, `exploration_dispositions`, shaped as D2   |

The three tables enter only through a CEN-04 §5 admission row filed
with the implementation issue: the facts are D2's fields, the owner is
SEL-04 under #9204, the home is the existing broker database, the write
and read APIs are D12's verbs, the legacy mapping is the refusals
above, and rollback is disable-only (§7). They overlap no QTI field,
because no task column is touched. `prune_events` and every other
cleanup path leave them alone; their retention is a separate decision
and this slice deletes nothing.

Until the row is admitted, the model in `work/selection.rs` keeps runs
in memory, like DSP-01's acceptance ledger, and the broker writes
nothing.

### D11. Result codes

| Code                                                                         | When                                                                       |
| ---------------------------------------------------------------------------- | -------------------------------------------------------------------------- |
| `run_started`                                                                | D1 accepted                                                                |
| `run_open`                                                                   | the lane already has an open run                                           |
| `exploration_disabled`                                                       | SEL-07's control (D1, D9)                                                  |
| `nothing_to_explore`                                                         | no eligible candidate                                                      |
| `preempted`                                                                  | urgent work for this lane, at start or at claim                            |
| `disposition_recorded`, `disposition_unchanged`                              | D5                                                                         |
| `disposition_conflict`, `VERSION_CONFLICT`, `already_selected`               | D5                                                                         |
| `invalid_disposition`                                                        | missing `no_action` reason, hint on the wrong kind, `other` without a note |
| `run_not_disposed`, `no_selection`, `run_closed`, `run_frozen`               | D4, D6                                                                     |
| `invalid_transition`                                                         | any transition not in D4's table                                           |
| `unknown_run`, `unknown_candidate`                                           | the ref names nothing at this authority                                    |
| `unknown_version`                                                            | unknown run schema, policy or profile version                              |
| `disposition_payload_overflow`, `candidate_payload_overflow`                 | D2's bounds                                                                |
| `unsupported`                                                                | a store or hosted adapter that cannot do D1 or D6 atomically               |
| `unauthorized`, `wrong_authority`, `ambiguous_ref`, `IDEMPOTENCY_KEY_REUSED` | refused before any run state is read                                       |

Every code but `run_started`, `disposition_recorded` and D6's outcomes
writes nothing. None of them reports a claim that did not happen.

### D12. Command surface

The census gives `queue.explore` to SEL-03 as primary and SEL-04 as
supporting, and `flag.count` likewise. SEL-03 owns the draw options
(`--strategy`, the probabilities `--explain` prints). This slice owns
the run and its dispositions. CLI-13 registers the spellings and CLI-02
parses them; the semantics here are what they register:

```text
fleet queue explore [--count N] [--idempotency-key K] [--explain] [--json]
fleet queue explore show <run> [--json]
fleet queue explore dispose <run> <index> <kind> [--reason CODE] [--note TEXT]
                            [--hint DIRECTION] [--supersedes REV] [--idempotency-key K]
fleet queue explore claim <run> [--idempotency-key K]
```

- `--count` here is the candidate count, `1` up to the profile's
  maximum; outside that is CLI-02's usage error. `queue claim --count`
  keeps its existing batch meaning: the census row requires separate
  semantics per command.
- `show` is a read. It prints the run, every candidate with its masses
  and current disposition, and the revision history with `--explain`.
- The broker side is four additive verbs beside the existing ones
  (`/explore-start`, `/explore-show`, `/explore-dispose`,
  `/explore-claim`), reusing the broker's existing auth and transport.

## 5. Negative cases

| Negative from the slice body          | Where it is closed                                                                                                    |
| ------------------------------------- | --------------------------------------------------------------------------------------------------------------------- |
| stale input                           | D6: the claim compares `work_version_at_draw`, policy and profile, and rechecks the urgent gate; stale closes the run |
| duplicate or replay                   | D1 step 4 and D6 replay through REL-03; D5 identical writes are no-ops                                                |
| ambiguous resource                    | candidates are `(authority_id, work_id)` pairs; `ambiguous_ref` and `wrong_authority` write nothing                   |
| unauthorized actor                    | D1 step 3, D5 supersede grant, D6 via SEL-02; `actor_ref` is never proof                                              |
| partial remote effect                 | no remote effect in this slice; a claim's remote effects stay `pending` in REL-04's outbox                            |
| unknown version                       | `unknown_version` for run schema, policy or profile; no write                                                         |
| no false verified or completed result | `claimed` means a lease, nothing more; `expired`, `stale` and `rejected` are outcomes, never success                  |
| draw reserves items                   | D1 writes no lease; a drawn candidate stays claimable by others                                                       |
| score drift from random selection     | D5: hints only; no path writes a priority                                                                             |
| a second claim from one run           | D5 at most one selection; D6 closes the run on every claim outcome                                                    |

## 6. Oracles and tests

All are future tests in `work/selection.rs`'s test module, written
failing first, under
`cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker <name>`.
Each name must first appear in that command's `-- --list` output; a
filter that matches zero tests is not acceptance. Builds and tests run
through remote dispatch, not a local compile.

| Test                                                     | Oracle                                                                                                                                                                                                                                                                                       |
| -------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `pmo_selection_disposition_replay_no_claim` (first, red) | Start a 3-candidate run; dispose A select, B triage, C release; replay the start and every disposition. One run, one draw, every replay `disposition_unchanged`, no lease, no claim event, no priority change. Claim once: `claimed`; replay the claim: same result, generation bumped once. |
| `pmo_selection_draw_writes_no_lease`                     | After a start, no lease row or generation bump exists for any candidate, and another lane's Shape A pick can claim a drawn candidate.                                                                                                                                                        |
| `pmo_selection_draw_reproduces_from_seed`                | Recomputing SEL-03's draw from the stored seed, profile and snapshot gives the same candidates and masses, byte for byte.                                                                                                                                                                    |
| `pmo_selection_urgent_gate_blocks_start`                 | An urgent item claimable by this lane gives `preempted` and no run; an urgent item for another lane does not block.                                                                                                                                                                          |
| `pmo_selection_one_open_run_per_lane`                    | A second start for a lane with an open run returns `run_open` with its id; two lanes may hold runs over the same candidate.                                                                                                                                                                  |
| `pmo_selection_disposition_conflict_needs_supersede`     | A different disposition without `supersedes_revision` is `disposition_conflict`; with the wrong revision `VERSION_CONFLICT`; with the right one and a grant, revision 2, revision 1 kept.                                                                                                    |
| `pmo_selection_at_most_one_selected`                     | A second `select_for_execution` is `already_selected`.                                                                                                                                                                                                                                       |
| `pmo_selection_claim_requires_disposed_run`              | Claim on a `drawn` run is `run_not_disposed`; on a `no_selection` run `no_selection`; after close `run_closed`.                                                                                                                                                                              |
| `pmo_selection_stale_selection_closes_run`               | The selected item is edited after the draw: `stale`, run closed, no second candidate claimed, caller told to pick normally.                                                                                                                                                                  |
| `pmo_selection_crash_points_recover`                     | Each D7 row: crash injected at that point, then retry; one draw, at most one claim, no lost disposition.                                                                                                                                                                                     |
| `pmo_selection_hints_never_change_priority`              | Over every kind, supersede, expiry, kill and re-enable, `tasks.priority` and `task_history` are unchanged.                                                                                                                                                                                   |
| `pmo_selection_expiry_records_no_action`                 | A past-deadline run closes `expired` with `no_action`/`run_expired` for every undisposed candidate, and no other write.                                                                                                                                                                      |
| `pmo_selection_unauthorized_writes_nothing`              | Start, dispose, supersede and claim by an unauthorized principal or a forged `actor_ref` write nothing.                                                                                                                                                                                      |
| `pmo_selection_unknown_version_refused`                  | Unknown run schema, policy or profile version: `unknown_version`, no write.                                                                                                                                                                                                                  |
| `pmo_selection_adapter_without_txn_unsupported`          | A store that cannot run D6's run update inside the claim transaction returns `unsupported` and claims nothing.                                                                                                                                                                               |
| `pmo_selection_exploration_kill_keeps_history`           | SEL-07's test, against this slice's fixture: D9's four obligations.                                                                                                                                                                                                                          |

## 7. Compatibility, migration and reversal

- Additive only. No existing verb, table, column, spelling or claim
  path changes. `queue claim`, `work next` and the broker pick behave
  exactly as before; exploration never runs unless asked for and
  allowed.
- With SEL-07's default `exploration = disabled`, the new verbs refuse
  before reading or writing, so landing this slice changes no live
  behaviour.
- The three tables are created by the admitted migration only. Reversal
  is disable-only: turn exploration off. The tables, runs and
  dispositions stay; nothing is dropped, deleted or reset, and no
  priority is touched.
- Hosted parity (#9079, #9081): a target that cannot persist runs with
  revisions, or cannot close the run inside its claim transaction,
  returns `unsupported` and must not advertise exploration.

## 8. Requirement trace

| Requirement                                                    | How this record serves it                                                                                                                                        |
| -------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| C-R095, C-R097, H-R009, O-R015, S-R008                         | Exploration only when no urgent work for this lane is claimable, checked at start and again at claim (D1 step 7, D6).                                            |
| C-R098, C-R099                                                 | Every drawn candidate gets a durable disposition before the claim, and the draw reserves nothing (D1, D4, D5, D6).                                               |
| C-R003, C-R100, O-R038                                         | Hints persist as evidence; random selection never moves a priority; do-now and directives are untouched (D5, D9).                                                |
| C-R004, C-R101, C-R104, H-R010, H-R036, O-R016, S-R010, P-R014 | The formula and defaults stay SEL-03's candidates; this record pins the profile version, masses and seed so every draw is reproducible and explainable (D2, D3). |
| C-R005, C-R107, C-R108, P-R009, O-R023, H-R017                 | `queue explore` subcommands with `--count`, `--explain`, `--json` and `--idempotency-key`, registered by CLI-13, with no parallel dispatch (D12).                |
| C-R009                                                         | The exploration route still claims safely through SEL-02's revalidated claim (D6).                                                                               |
| H-R024, H-R031                                                 | Every state, error, receipt and failure case is in §4 to §6 so it stays in acceptance coverage.                                                                  |

## 9. Gates

Implementation starts when these hold:

1. This record is merged.
2. SEL-03 has landed its profile and sampler, and is the shared-file
   predecessor on `work/selection.rs`. The pure run model (D2 to D5,
   D7, D8) can then land, taking a draw as input.
3. SEL-02's Shape B claim and REL-03's ledger exist in the broker for
   D1 step 4 and D6. These are the slice's hard edges.
4. The CEN-04 §5 admission row for D10's tables is filed and accepted
   before any broker write.
5. CLI-13 registers D12's spellings.

Activation (any automatic draw, or an `exploration` control value that
allows a start in production) stays SEL-07 §8's separately approved
step. Applying hints stays #9199's.

**Schedules:** this record creates, changes and retires no schedule on
any surface.

## 10. Non-goals

- The selector, eligibility and urgent predicate (SEL-01), the claim
  and its revalidation (SEL-02), the sampler and profile values
  (SEL-03), simulation (SEL-05), fairness (SEL-06), shadow and
  controls (SEL-07).
- Any automatic exploration trigger in the pick path.
- Applying a hint, reprioritizing, or any priority write.
- Retention or cleanup of stored runs.
- Any remote write.
