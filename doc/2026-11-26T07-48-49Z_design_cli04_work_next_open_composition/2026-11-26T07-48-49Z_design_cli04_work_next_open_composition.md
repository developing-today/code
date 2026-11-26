# CLI-04 — Shared service composition for `work next` and `work open`

Issue: #9440 (slice CLI-04 of #9206, program #9196). Status: design
record. It decides how `fleet work next` and `fleet work open`
compose the selector, the shared claim path and hydration into one
command each, without a second ranking or eligibility check, and what
the composed command reports when a later step fails after an earlier
one succeeded. It writes no code; the build follows the gates in §9.

Two rules hold throughout:

- **Selection is not claim.** Showing the next item never claims it,
  and one invocation claims at most one item. Exploration sampling
  several candidates never turns into several claims.
- **No second judge.** Ranking lives in the broker pick and SEL-01;
  eligibility lives in SEL-02's one claim clause; hydration lives in
  HYD-04's entry point. CLI-04 only calls them in order and reports.

## 0. Governing decisions consumed

| Source                                 | What this record takes from it                                                                                                                       |
| -------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| #9440 slice text                       | Owned files `work/selection.rs`, `work/lifecycle.rs`; first red test `work_next_revalidates_eligibility_through_shared_claim_path`; partial receipt. |
| #9206 §5 composed operations           | `next`: selection, ranking, revalidation, atomic claim, optional bind, hydration, receipt. `open`: resolve, inspect, optional claim and hydrate.     |
| SEL-02 record (`2026-11-25T01-18-23Z`) | One claim clause for every path. `work next` is Shape A (broker pick). `work open` is Shape B (`operator_pick`). `--claim` on a read is Shape B.     |
| REL-01 record (`2026-11-25T01-23-08Z`) | The pure door model `claim_eligibility` in `work/lifecycle.rs`; automatic pick versus by-id doors.                                                   |
| HYD-04 record (`2026-11-25T21-40-00Z`) | One library entry point `hydrate(...) -> HydrationResult`; `partial` maps to `Outcome::Partial`; GET-only; writes only HYD-03 staging.               |
| CLI-02 record (`2026-11-23T12-33-39Z`) | One strict parser in `work/cli.rs`; parse failure exits 2 and creates nothing; every other result is a `fleet.result/v1` envelope.                   |
| CLI-13 record (`2026-11-26T03-04-43Z`) | The `work` domain and its registration; CLI-04 adds operations to `cli.rs` only and touches no shared registration file.                             |
| REL-03 record (`2026-11-25T20-47-34Z`) | `operation_id`, idempotency key and request digest; a replay returns the stored receipt.                                                             |
| Drew's ruling, 2026-11-24 (on #9196)   | Design-only gates without a hard blocker are void. The SEL-01 and REL-01 file edges gate the build only (§9).                                        |

## 1. Verified state on main `84d54f93`

- There is no `work` domain and no `work next` or `work open`
  anywhere. `fleet-queue/src/work/cli.rs` does not exist; CLI-02 and
  CLI-13 are records only.
- `queue claim` is `cmd_claim` in
  `fleet-queue/src/client/commands_lifecycle.rs:13`. It posts one
  claim body (worker, lease, optional queue, filter, pool, platform,
  model marks, `--task`, `--count`, `--handoff`, `--bind`) and the
  broker picks and claims in one call. There is no read-only pick:
  no broker verb returns the head without claiming it.
- The claim SQL lives in `fleet-queue/src/turso_queue/claim.rs`:
  `default_pick_predicate()` (:2273), `HANDOFF_SQL` (:761),
  `claim_by_id` (:1163), `claim_scope` (:1980). Source-level tests in
  `turso_queue.rs` assert the by-id path shares the pick's platform
  limit (`claim_by_id_enforces_platform_limit_like_the_pick_8217`).
- `work/lifecycle.rs` holds the REL-01 pure model:
  `ClaimContract`, `ClaimPath` (`Automatic`, `ById`, `TakeOver`),
  `ClaimRefusal`, `ClaimEligibility`, and `claim_eligibility`
  (:677). It also holds `WorkInspection` (:283) and
  `resolve_work_inspection_by_ref` (:340), which refuses an
  ambiguous or missing ref instead of returning a near match.
- `work/selection.rs` on main holds SEL-06 fairness only. The SEL-01
  pure selector (`select`, `hard_gate_exclusion`, `is_urgent`) is in
  open PR #15186, not on main.
- `work/contracts.rs` defines `fleet.command/v1`,
  `fleet.result/v1` and the `Outcome` vocabulary (:513):
  `rejected`, `accepted`, `running`, `pending_sync`, `partial`,
  `conflict`, `uncertain`, `verified_complete`.
- No hydration entry point exists in code (`work/context.rs` is
  absent); HYD-04 is a record.

## 2. Intent

An agent runs one command to get its next piece of work ready:

```
fleet work next --claim --hydrate full
```

It gets back one envelope that says which item it now holds, under
which lease and fence generation, and whether the context bundle is
complete, partial or failed. A person who already knows the item runs
`fleet work open <ref> --claim`, and the claim is checked against the
version the same command just read. Each step is the existing
primitive; nothing here re-ranks or re-checks eligibility.

## 3. Ownership

| Part                                           | Owner                                                                   |
| ---------------------------------------------- | ----------------------------------------------------------------------- |
| Composition service (`work_next`, `work_open`) | `work/lifecycle.rs` (this slice)                                        |
| Preview over the selector's explanation        | `work/selection.rs` (this slice), reading SEL-01's `Selection`          |
| Argv grammar for the two operations            | `work/cli.rs` (CLI-02's parser; CLI-04 adds two operation declarations) |
| Domain registration and help                   | CLI-13; no edit here                                                    |
| Claim clause, pick SQL, read-only pick         | SEL-02 / `turso_queue/claim.rs`; CLI-04 adds no SQL                     |
| Hydration                                      | HYD-04 entry point; CLI-04 calls it                                     |

## 4. Decisions

### D1. The service takes ports, not stores

`work_next` and `work_open` are plain functions in `work/lifecycle.rs`
over three narrow traits:

| Port          | Methods                                                                                                | Real implementation                              |
| ------------- | ------------------------------------------------------------------------------------------------------ | ------------------------------------------------ |
| `ClaimPort`   | `pick(request) -> ClaimOutcome`, `claim_intent(intent) -> ClaimOutcome`, `preview(request) -> Preview` | broker client (Shape A, Shape B, read-only pick) |
| `InspectPort` | `inspect(work_ref) -> Result<WorkInspection, WorkInspectionLookupError>`                               | the inspection read used by `work inspect`       |
| `HydratePort` | `hydrate(inspection, mode) -> HydrationResult`                                                         | HYD-04's `hydrate`                               |

The service owns ordering and the receipt and nothing else. Tests
drive it with fakes; the CLI drives it with the real ports. `queue
claim` and `work next --claim` build the same `pick` request, so the
sugar and the primitive go through one function.

### D2. `work next` without `--claim` is a preview

`fleet work next` with no `--claim` asks the broker for the item its
pick would choose now, through a read-only pick: the same predicate,
claim scope and order that the claiming pick uses, run as a `SELECT`
with no `UPDATE`. The broker builds it with the same builder SEL-02
§3.1 requires for every claim path; CLI-04 adds no SQL and no client
ranking. The result carries the item ref, its work version, the
snapshot ref and SEL-01's reasons, with outcome `accepted` and a
`claimed: false` field.

A preview is advice. A later `work next --claim` may claim a
different item, and nothing may claim the previewed item by id
(SEL-02 §3.2: no path claims a remembered head). A caller who wants
that exact item runs `work open <ref> --claim`.

### D3. `work next --claim` is one broker pick

`--claim` sends the Shape A pick, the same body `queue claim` sends,
with the caller's filter, pool and requirement marks. The broker
chooses and claims in one transaction, so revalidation is the claim
clause itself. CLI-04 never sends `--task` for `work next`, never
runs a preview first, and never retries with a different item: the
broker's own re-pick loop is the only retry. `nothing_claimable`
returns outcome `rejected` with that code and exit 0, because an
empty queue is an answer, not a failure.

When SEL-03 exploration is active, the exploration run (SEL-04)
chooses the candidate and the claim is Shape B with source
`exploration`. One invocation still makes one claim; a stale result
returns to normal selection once, as SEL-04 specifies, and never
claims a second sampled candidate in the same invocation.

### D4. `work open <ref>` resolves before it writes

`work open` first resolves the ref through `InspectPort`. An
ambiguous, missing or wrong-authority ref ends the command with
outcome `rejected` and the lookup's code before any claim or
hydration. Without `--claim` it returns the inspection and its work
version.

With `--claim`, it sends a Shape B `ClaimIntent` with source
`operator_pick` and `expected_work_version` set to the version this
command just read (SEL-02 §6: "the read's returned version becomes
`expected_work_version`"). An explicit `--expected-version` replaces
it; a mismatch with the read is outcome `conflict` before any claim
is sent. Every SEL-02 refusal (`stale_item`, `held`, `ineligible`
with reasons, `preempted`) passes through unchanged.

### D5. `--hydrate full|incremental`

After a successful claim, or after a resolved `work open` without a
claim, `--hydrate` calls `HydratePort::hydrate` with the inspection
and the mode. `work next --hydrate` without `--claim` is a usage
error: hydrating a head the caller does not hold spends budget on an
item that may go to someone else. `work open --hydrate` without
`--claim` is allowed, because the item is named and hydration is
read-only (HYD-04 issues only GETs).

### D6. `--ready`

The source inventory spells `work next --ready`, and #9206 keeps
every original option. The pick already applies the full claim
clause, including DEP-03's hard blockers, timers and requirements,
so every item `work next` can return is ready. `--ready` is accepted
by the parser and recorded in the result as `ready_only: true`; it
changes no query. A not-ready listing is `fleet dependency
blockers` and `work inspect`, not `work next`.

### D7. Bind

`--bind` passes through to the claim as it does on `queue claim`
today. The bind is part of the claim transaction, so it is not a
separate step in the receipt.

### D8. One receipt, steps in order

The composed command returns one `fleet.result/v1` envelope with a
`steps` array, one entry per step that ran: `resolve` (open only),
`preview` or `claim`, then `hydrate`. Each entry carries its own
code. The top-level outcome follows from the steps:

| Steps                                    | Top-level outcome                    | Lease after the command |
| ---------------------------------------- | ------------------------------------ | ----------------------- |
| preview only                             | `accepted`, `claimed: false`         | none taken              |
| claim refused or nothing claimable       | `rejected` (or `conflict` for stale) | none taken              |
| claim granted, no hydrate asked          | `accepted`                           | held                    |
| claim granted, hydrate complete          | `accepted`                           | held                    |
| claim granted, hydrate partial or failed | `partial`                            | held                    |
| claim outcome unknown (timeout)          | `uncertain`                          | unknown; see D9         |

A claim is never released because hydration failed. The agent holds
the work and can hydrate again with `work open <ref> --hydrate
incremental` under the same lease. The partial receipt names the
claimed item, its fence generation and the hydration generation that
is still current, so nothing has to be rediscovered.

### D9. Idempotency and replay

The composed command mints one `operation_id` (REL-03). The claim
step is keyed by it, so replaying `work next --claim` after a client
crash returns the stored claim receipt instead of claiming a second
item. A timeout after the claim was sent is `uncertain`; the next
invocation with the same `operation_id` reads the stored receipt. The
hydrate step is not keyed: HYD-03 publication is atomic and a repeat
publishes a newer generation or none, never a torn one.

### D10. Output

Human output renders the same envelope as `--json`. The first line
names the claimed item or says nothing was claimed. Lease tokens are
never printed in either form (CLI-02 §7).

## 5. Negative cases

- `work next` never sends a by-id claim and never sends two claims.
- A preview never writes a lease, a generation bump or a `claimed`
  event.
- `work open` with an ambiguous ref writes nothing and calls no
  hydration.
- `work open --claim` with a stale version returns `stale_item` and
  writes nothing.
- `work next --hydrate` without `--claim` is a usage error (exit 2,
  `conflicting_flags`) and creates no operation.
- Hydration failure after a claim never releases the claim and never
  reports `accepted`.
- A replay with the same `operation_id` never produces a second claim.

## 6. Oracles and tests

All in `work/lifecycle.rs` unit tests with fake ports, run by remote
dispatch (`cargo test --manifest-path pkgs/fleet/Cargo.toml -p
fleet-queue --lib --features client,queue-broker`), with `-- --list`
first to confirm each name exists.

| Test                                                           | Oracle                                                                                                                                                                           |
| -------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `work_next_revalidates_eligibility_through_shared_claim_path`  | Fake broker previews item A, then A becomes ineligible; `--claim` calls `pick` once, never `claim_intent` or by-id, and returns what `pick` returned (B or `nothing_claimable`). |
| `work_next_claim_and_queue_claim_send_identical_pick_requests` | The pick request built for `work next --claim` equals the one built for `queue claim` with the same flags.                                                                       |
| `work_next_preview_never_claims`                               | Preview calls only `preview`; the fake records no claim call.                                                                                                                    |
| `work_next_claims_at_most_once_under_exploration`              | A three-candidate exploration sample yields one `claim_intent`; a stale result falls back to one `pick`.                                                                         |
| `work_open_ambiguous_ref_writes_nothing`                       | Ambiguous lookup: no claim call, no hydrate call, outcome `rejected`.                                                                                                            |
| `work_open_claim_uses_inspected_version`                       | `claim_intent.expected_work_version` equals the inspection's version; an explicit mismatching version is `conflict` with no claim call.                                          |
| `work_next_partial_hydration_keeps_claim_and_reports_partial`  | Claim granted, hydrate `failed`: outcome `partial`, lease held, receipt names fence and current generation, no release call.                                                     |
| `work_next_replay_returns_stored_claim`                        | Same `operation_id` twice: one claim, identical receipts.                                                                                                                        |
| `work_next_hydrate_without_claim_is_usage_error`               | Parser refuses before any port call.                                                                                                                                             |

A broker-side test, owned by the read-only pick's build slice, asserts
that the preview `SELECT` and the claiming `UPDATE` are built from the
same predicate function, in the style of the existing source-level
`claim_by_id_enforces_platform_limit_like_the_pick_8217`.

## 7. Compatibility, migration and reversal

- `queue claim` keeps every flag and its output. `work next --claim`
  is new sugar over the same request; nothing about `queue claim`
  changes.
- The read-only pick is a new broker verb. An older broker answers it
  as unknown; `work next` without `--claim` then reports outcome
  `rejected` with code `unsupported` and never falls back to a
  client-side ranking.
- No schema change, no new table, no new queue field.
- Reversal is removing the two operation declarations from `cli.rs`
  and the read-only pick verb. Claims made through `work next` are
  ordinary claims and need no cleanup.

## 8. Requirement trace

| Requirement                                                                         | Where        |
| ----------------------------------------------------------------------------------- | ------------ |
| C-R009, C-R110, O-R025, P-R011: compose next and open from primitives               | D1 to D5, D8 |
| C-R107, C-R108, H-R017, O-R023, P-R009: cohesive CLI with source spellings          | D2 to D7, §7 |
| C-R150, O-R049: scenario A start (next, claim, hydrate full)                        | D3, D5, D8   |
| C-R151, C-R153: resume and near-timeout paths stay separate (`work resume`, `park`) | §10          |
| H-R014: atomic claims, durable hydration, attribution                               | D3, D8, D9   |
| H-R016: duplicate request, stale lease, conflict                                    | D4, D9, §5   |
| H-R024, H-R031: keep every original option                                          | D6, D7       |
| Slice done check: same service for selection and sugar, partial receipt             | D1, D8, §6   |

## 9. Gates

| Build step                                         | Hard edge                                                                       |
| -------------------------------------------------- | ------------------------------------------------------------------------------- |
| Service and ports in `work/lifecycle.rs`           | REL-01 owns the same file: serialize after its open edits land                  |
| Preview over SEL-01 reasons in `work/selection.rs` | SEL-01 (#9491, PR #15186) owns the same file: after it merges                   |
| Read-only pick verb                                | SEL-02's shared clause builder (the pick and preview must share it)             |
| Real `HydratePort`                                 | HYD-04's entry point exists; until then `--hydrate` is refused as `unsupported` |
| Argv declarations                                  | CLI-02's `cli.rs` exists; registration is CLI-13's                              |

The service with fake ports and the tests in §6 need only the first
two edges. No other gate applies.

## 10. Non-goals

- `work resume`, `work park`, `work checkpoint`, `work complete` and
  `work handoff`. Each has its own slice; `resume` is SEL-02 Shape B
  with source `resume`.
- Any change to ranking, the claim clause or exploration policy.
- A client-side ranking or eligibility check of any kind.
- Exit-code mapping beyond CLI-02's parse-failure row and D3's empty
  queue; the full matrix belongs to CLI-02's implementation issue.
