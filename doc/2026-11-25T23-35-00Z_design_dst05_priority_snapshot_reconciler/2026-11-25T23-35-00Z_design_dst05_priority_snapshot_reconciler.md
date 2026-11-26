# DST-05 — Versioned desired/observed priority snapshot reconciler

Issue: [#9455](https://github.com/developing-today/code/issues/9455)
(slice `DST-05` of [#9200](https://github.com/developing-today/code/issues/9200),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement.**
This record elaborates the DST-05 row of the merged design package
(`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
PR #9225): "versioned desired/observed snapshot reconciler with dirty
cursor + full audit", acceptance "reorder/duplicate/outage/rate-limit/
partial-effect fixtures converge or report conflict; full scan finds
omitted dirty marker; no second Project writer", first oracle
`pmo_distribution_projection_replay`. It decides what "desired" and
"observed" are, how they are versioned and hashed, where the dirty
set comes from, what the reconciler may do about a difference and
what it may never do. It also settles a file-ownership contradiction
in the slice header (§2). It authorizes no code, schema, route,
schedule or deployment (§10).

Governing decisions consumed, not re-opened:

- **Component #9200** ("Authority and precedence table", "Minimal
  event identity, revision, snapshot and receipt", "Cross-target
  order and reconciliation"): Fleet is the canonical priority owner;
  Project fields are written only by the orchestrator; outbound
  labels go through the authorized Fleet `gh` operation adapter; the
  Projector is a disposable read-only projection and transport intake
  and is never an outbound GitHub/Project mutator; projection events
  never feed back as new human intent (`projection_echo`); four
  distinct sequences (delivery id, operation/idempotency, per-work
  version, read snapshot/watermark); the reconciler "first drains an
  existing dirty-item set/cursor, then full reconciliation by an
  existing schedule interface; do not create a schedule/timer here";
  compare desired/observed hashes and revisions, compare-and-apply,
  detect echoes, verify remote effect before retry, keep conflicts
  visible; replay rules (unknown major version quarantined/read-only,
  same revision with a divergent payload is a terminal conflict, a
  missing revision is a gap, an older event never rolls state back,
  no last-write-wins, no webhook-arrival or Project-position
  precedence).
- **PRI-01** (`work/priority.rs`): ratified values are exact,
  `expedite = 15` and `next-up = 25` are profile members, never
  snapped, never renormalized; values from different profiles never
  compare.
- **REL-03** (`doc/2026-11-25T20-47-34Z_design_rel03_idempotency_identity_receipts/`):
  operation identity, `IDEMPOTENCY_KEY_REUSED`, receipts; acceptance
  before authorization lookup.
- **REL-04** (`doc/2026-11-25T22-30-00Z_design_rel04_lifecycle_mutation_outbox/`):
  every remote effect is an immutable intent row committed in the
  same local transaction as the state CAS; the status table
  (`pending → delivering → confirmed | retryable_failure | unknown |
conflict | terminal_failure`); `unknown` is resolved only by
  read-back; per-target order by `target_seq`; no new scheduler.
- **REL-06** (`doc/2026-11-23T12-20-33Z_design_rel06-github-preconditions-readback/`):
  effect preconditions, read-back verification, verify-before-retry.
- **DST-06** (`doc/2026-11-25T01-19-37Z_design_dst06_order_hash_fixture/`):
  digest framing via `work/contracts.rs` `digest`/`canonical_json`; a
  hash is a fingerprint, not authority.
- **DST-07** (`doc/2026-11-25T20-20-00Z_design_dst07_degraded_stale_payload_limits/`):
  the eight consumer states, `stale_snapshot`, `unsupported_version`,
  `invalid_schema`, `payload_too_large`, and "never invent a value".

## Verified state on main (`06230c30b`)

| Fact                                    | Where                                                                                                                                                                                     | What it means here                                                                                                                                                                                                                                                             |
| --------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Hot ledger + consumer checkpoints       | `pm-stack/projector/schema.sql` `pmp_hot_events`, `pmp_consumer_checkpoints`; `src/store.ts`                                                                                              | An existing monotonic cursor (`seq`) with per-consumer, forward-only, never-past-high-water acks. One handled sub-event per row, idempotent per `event_id`. This is the **existing dirty cursor** on the observed side.                                                        |
| Hot ledger `actor` for issue rows       | `src/projector.ts` `applyIssue` → `actor: issue.user.login ?? sender`                                                                                                                     | For an issue or PR event the recorded actor is the **item author**, not the sender who changed the label. The ledger cannot attribute a label change, so echo detection must not use it (§5.3).                                                                                |
| Item labels                             | `pmp_items.labels` (JSON array), monotonic guard on `source_updated_at`                                                                                                                   | The observed label set per item, already kept current by webhook and poll.                                                                                                                                                                                                     |
| Poll reconcile + its timer              | `src/reconcile.ts` (`reconcileRepos`), `POST /reconcile` single-flight in `src/server.ts`; registry row `pm-projector-reconcile.timer` (`*:00/15:00`) in `doc/fleet-schedule-registry.md` | The **existing schedule interface** the full pass rides on. No new timer.                                                                                                                                                                                                      |
| Projector writes to GitHub              | only `POST /app/installations/{id}/access_tokens` in `src/github.ts`                                                                                                                      | The Projector mutates nothing remote today. DST-05 keeps it that way.                                                                                                                                                                                                          |
| Projects v2 events                      | not handled by `src/projector.ts`                                                                                                                                                         | The Projector has no observed Project field state. Project observation belongs to the orchestrator (§5.5).                                                                                                                                                                     |
| Mirror labels                           | `pm-stack/aggregator/src/mobile-work-actions.ts` `PRIORITY_MIRROR_LABELS`                                                                                                                 | `priority/critical` (10), `expedite` (15), `priority/high` (20), `next-up` (25), `priority/normal` (40), `priority/low` (70). Scale-only values 30, 50, 90 have **no** mirror label. Other priority-looking labels (`priority/medium`, `priority/very-high`) are noncanonical. |
| Operation forwarder                     | `src/operation-forwarder.ts`                                                                                                                                                              | The injected-sink pattern: the Projector builds an envelope and hands it to Fleet authority; no sink is wired by default. DST-05's Projector side uses the same pattern.                                                                                                       |
| Reconcile / distribution module in Rust | none: `work/` has `authority`, `completion`, `contracts`, `degraded`, `journal`, `lifecycle`, `operations`, `priority`, `routing`, `selection`, `sqlite_replica`                          | `work/distribution.rs` is proposed by DST-06/DST-07 and does not exist yet. DST-05 adds its own section to that proposed file (§2).                                                                                                                                            |
| Intent outbox, read-back                | REL-04 and REL-06 records; no general outbox on main (`creation_outbox` is the isolated pilot)                                                                                            | The reconciler produces commands whose effects are REL-04 intents. Until REL-04 lands, there is nothing to apply through, so implementation is gated (§10).                                                                                                                    |

## 1. Intent

Priority has one owner (Fleet) and several mirrors (GitHub labels,
Project fields, PM Stack projections). Mirrors drift: a webhook is
lost, a send times out, someone edits a label by hand, a rollout
predates the mirror, two changes land out of order. The failures this
slice prevents:

1. A mirror stays wrong forever because the one event that would have
   fixed it was lost (the "omitted dirty marker").
2. Fleet's own label write comes back as a webhook and is accepted as
   a new human decision, so the system argues with itself.
3. A retry after a timeout sends a second label change on top of one
   that already landed, or overwrites a human edit made in between.
4. An old or reordered event rolls a newer value back.
5. A divergence is "fixed" silently, so nobody sees that a human and
   Fleet disagreed.
6. A second writer appears for Project fields, or the Projector
   becomes a GitHub mutator.

## 2. Ownership, and the file the slice header gets wrong

| Concern                                                                                     | Owner                                                      |
| ------------------------------------------------------------------------------------------- | ---------------------------------------------------------- |
| Canonical priority, per-work version, policy/profile version                                | Fleet broker (PRI-01, REL-04 CAS)                          |
| Authorized label input becoming a command                                                   | DST-04 (#9454)                                             |
| Remote effect intent, status table, per-target order                                        | REL-04                                                     |
| Preconditions and read-back                                                                 | REL-06                                                     |
| Project field writes                                                                        | the orchestrator (sole Project writer), unchanged          |
| **Desired/observed snapshot types, the decision table, replay rules, audit row shape**      | **DST-05, Rust, `work/distribution.rs` §DST-05**           |
| **Observed-side dirty drain and observed snapshot builder over Projector rows (read-only)** | **DST-05, `pm-stack/projector/src/priority-reconcile.ts`** |

The slice header lists `pkgs/fleet/crates/fleet-queue/test/priority-reconcile.test.ts`
as an owned file and `cargo test ... -p fleet-queue --lib ...
pmo_distribution_projection_replay` as the done command. Those cannot
both be right: `fleet-queue` has no `test/` directory, a `.ts` file
is not run by `cargo`, and a `--lib` filter only finds tests inside
`src/`. **Decision:**

- The decision core is Rust, so the done command is honest: the
  `pmo_distribution_*` tests live in a `#[cfg(test)] mod` of
  `pkgs/fleet/crates/fleet-queue/src/work/distribution.rs`, in a
  DST-05 section beside DST-06's and DST-07's.
- The TypeScript test is `pm-stack/projector/test/priority-reconcile.test.ts`,
  next to the 23 existing projector tests and run by
  `.github/workflows/pm-stack-tests.yml` (`bun test`).
- `pm-stack/projector/src/priority-reconcile.ts` is **read-only**: it
  drains the hot ledger, builds observed snapshots from `pmp_items`
  and hands them to Fleet through an injected sink, exactly as
  `operation-forwarder.ts` does. It holds no GitHub client, issues no
  write and stores no decision. That is the only reading of the slice
  that agrees with the component's "never an outbound GH/Project
  mutator".

## 3. The two snapshots

### 3.1 Desired

One desired snapshot per work item, read from the broker inside one
read transaction:

| Field               | Meaning                                                                                                              |
| ------------------- | -------------------------------------------------------------------------------------------------------------------- |
| `schema`            | `fleet.priority-projection/v1`                                                                                       |
| `work_ref`          | authority-qualified identity (`(authority_id, work_id)`)                                                             |
| `target_ref`        | `github:<owner>/<repo>#<n>`, the mirror the item maps to; none ⇒ not reconciled                                      |
| `work_version`      | the broker's per-work version at read                                                                                |
| `profile_ref`       | PRI-01 profile id and version (`fleet.queue@1`)                                                                      |
| `value`             | the exact profile member (`15`, never `10`)                                                                          |
| `mirror_labels`     | the priority-family label set the value maps to: zero or one label from the table above; `30`/`50`/`90` map to empty |
| `last_operation_id` | REL-04 `last_operation_id` on the row                                                                                |
| `desired_hash`      | `digest("fleet.priority-projection/v1", {schema, target_ref, profile_ref, mirror_labels})`                           |

`desired_hash` covers only what the mirror can show. A change of
`value` from 30 to 50 changes no label, so it changes no hash and
creates no work. The work version and operation id are carried for
compare-and-apply and audit, not hashed, so a version bump that
leaves labels unchanged is not drift.

### 3.2 Observed

One observed snapshot per target, built from the Projector's row:

| Field               | Meaning                                                                                                                                                                 |
| ------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `target_ref`        | as above                                                                                                                                                                |
| `family_labels`     | the item's labels intersected with the priority family: the six mirror labels **plus** any label matching `priority/*`, so noncanonical spellings are seen, not ignored |
| `source_updated_at` | `pmp_items.source_updated_at` (the GitHub `updated_at` the row reflects)                                                                                                |
| `observed_seq`      | the hot-ledger `seq` that last touched the ref, or the poll cursor when the row came from `reconcile.ts`                                                                |
| `observed_hash`     | `digest("fleet.priority-projection/v1", {schema, target_ref, family_labels})` with the same framing                                                                     |
| `projector_schema`  | the Projector schema version the row was read under                                                                                                                     |

Both hashes use DST-06's framing (`ASCII(domain) || 0x00 ||
UTF8(JCS(dto))`, SHA-256, `sha256:` + 64 hex). The domain
`fleet.priority-projection/v1` must be added to the
`architecture.md` domain registry before implementation (§10); this
record does not edit that file.

### 3.3 What equality means

`desired_hash == observed_hash` ⇒ **in sync**, nothing to do,
whatever the versions say. Otherwise the decision table (§5) runs.
A snapshot is never proof that a write was applied: "in sync" is a
statement about two reads at stated revisions, and the audit row
records both.

## 4. Dirty drain, then full pass

### 4.1 Dirty sources (all existing)

- **Observed side:** the hot ledger. The reconciler is one more
  consumer, `priority-reconcile`, in `pmp_consumer_checkpoints` (a
  row, not a table). It pins the high-water, pages `seq > checkpoint`
  in ascending order, and keeps refs whose event is `issues` or
  `pull_request` (any action: the ledger stores the sub-event, not
  the label delta, so every issue event on a ref marks it dirty). It
  acks at the pinned high-water **only after** every dirty ref in the
  page has an audit row (§6). A failed pass leaves the checkpoint
  where it was and replays; replay is safe because the decision is a
  pure function of the two snapshots.
- **Desired side:** REL-04 intents for the effect kind
  `label.priority_mirror` that are not terminal. They are already the
  record of "Fleet changed its mind and the mirror has not caught up";
  the reconciler reads them, it does not duplicate them.

The dirty pass is `O(D)` in dirty refs. It is bounded per run (the
existing `pageSize` convention, default 200) and stops on deadline
with the checkpoint unmoved past the last fully audited page.

### 4.2 Full pass

After the dirty drain, the same run compares every mapped work item:
`O(N)` reads, writes only where §5 says. It runs inside the existing
`POST /reconcile` pass, after `reconcileRepos` has refreshed the
Projector rows, under the same single-flight gate, on the existing
`pm-projector-reconcile.timer`. **No timer, cron, service or worker
is added**, so `doc/fleet-schedule-registry.md` gains no row; if an
implementation finds it needs one, that is a stop and a new decision,
not a quiet addition.

The full pass is what finds the omitted dirty marker: a label edited
while the webhook was down is in `pmp_items.labels` after the poll,
has no hot-ledger row, and still differs from desired.

Wiring the call into `server.ts`'s `/reconcile` handler is one line
in a file DST-05 does not own; it lands as its own change (§10).

## 5. The decision table

Inputs: the desired snapshot `D`, the observed snapshot `O`, the
non-terminal `label.priority_mirror` intents `I` for the target in
`target_seq` order, and the recorded refusals and acceptances DST-04
produced for that target.

| #   | Condition                                                                                                             | Outcome code           | Action                                                                                                                                                                                |
| --- | --------------------------------------------------------------------------------------------------------------------- | ---------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | `D` or `O` carries an unknown major schema, or `profile_ref` is unknown                                               | `unsupported_version`  | Quarantine the target read-only. No command. Audit. Never coerce.                                                                                                                     |
| 2   | `D.desired_hash == O.observed_hash`                                                                                   | `in_sync`              | None. Audit only when it closes a previous non-`in_sync` row.                                                                                                                         |
| 3   | `O.family_labels` equals the payload of a non-terminal intent in `I` (Fleet's own write landing before its read-back) | `projection_echo`      | Record the observation; advance read-back only. Never a command, never human intent.                                                                                                  |
| 4   | Some intent in `I` is `pending`, `delivering` or `retryable_failure`                                                  | `in_flight`            | None. REL-04 owns delivery; a second command would fight the first.                                                                                                                   |
| 5   | Some intent in `I` is `unknown`                                                                                       | `awaiting_readback`    | None. Ask REL-06 for read-back. Never resend.                                                                                                                                         |
| 6   | A DST-04 command for this target is accepted but its new version is not yet in `D`                                    | `awaiting_desired`     | None; the next pass reads the new `D`.                                                                                                                                                |
| 7   | `O` differs and the change is a DST-04 input that was **refused** (unauthorized actor, conflict)                      | `repair_after_refusal` | Issue one projection command (§5.2) back to `D`. The refusal is already visible on its own receipt; the audit row links it.                                                           |
| 8   | `O` differs and no delivery, command or refusal accounts for it                                                       | `unattributed_drift`   | **No overwrite.** Raise conflict: needs-owner alert deduplicated on `priority-drift:<work_ref>`, visible in `sync status`. Cleared by a new Fleet decision or a matched DST-04 input. |
| 9   | `O.family_labels` holds more than one priority-family label, or a noncanonical spelling                               | `ambiguous_observed`   | Same as 8. Never pick one.                                                                                                                                                            |
| 10  | `O` is missing (target deleted, `pmp_item_deletions` marker, 404)                                                     | `target_gone`          | No command. Audit. Fleet's lifecycle owns what a deleted target means.                                                                                                                |
| 11  | `O` differs, it carries none of 3–10, and `O` matches an **earlier** confirmed projection than `D`'s                  | `stale_mirror`         | Issue one projection command to `D` (the mirror missed a change).                                                                                                                     |
| 12  | `D` has no mirror labels and `O` has none (scale-only value)                                                          | `in_sync`              | Same as 2 (it hashes equal).                                                                                                                                                          |

Rows are checked in order; the first match wins, so every pair
selects exactly one outcome. Rows 8 and 9 are the "keep conflicts
visible" rule: the reconciler does not decide that a human was wrong.

### 5.1 Replay and order rules

- An observed snapshot older than one already audited for the target
  (`source_updated_at` lower, or equal with a lower `observed_seq`) is
  recorded and ignored: it never rolls a decision back.
- The same `(target_ref, observed_seq)` with a different
  `observed_hash` is `terminal_conflict`: the ledger contradicts
  itself. Quarantine and alert; never pick.
- A gap (the checkpoint is behind the ledger's lowest retained `seq`
  after a prune) forces a full pass for the affected repos before the
  checkpoint moves.
- Webhook arrival order, `received_at` and Project position confer no
  precedence anywhere in the table.

### 5.2 The projection command

When rows 7 or 11 fire, the reconciler issues one Fleet command, not
a write:

- verb `priority.project`, effect kind `label.priority_mirror`;
- idempotency key `priority-project:<work_ref>:<work_version>:<desired_hash>`,
  so a replayed or concurrent pass maps to the same operation and
  REL-03 returns the stored receipt (no second effect);
- `expected_work_version = D.work_version`: if the broker moved on,
  the CAS loses and the command is `conflict`, and the next pass reads
  the new `D`;
- REL-06 precondition `observed_hash == O.observed_hash`, read back
  immediately before the send: if a human changed the labels in
  between, the send is refused with `REMOTE_CONFLICT` and the target
  returns to the table on the next pass, instead of being clobbered;
- payload: add `D.mirror_labels`, remove the other members of the
  family that `O` showed. Bounded by DST-07's payload limits; an
  oversize or invalid payload returns `payload_too_large` or
  `invalid_schema` and keeps the intent recoverable.

Retry is REL-04's: verify before retry, `unknown` only via read-back,
per-target `target_seq` order. The reconciler never retries anything
itself; a later pass simply finds row 4 or 5.

### 5.3 Echo detection

Echo is decided by payload, not by actor: the hot ledger records the
item author, not who changed the label (Verified state), and GitHub
carries no operation id on a label event. Row 3 matches the observed
family-label set against the payloads Fleet itself queued for that
target and has not yet read back. A mirror that matches an older,
already confirmed projection is not an echo: Fleet has since changed
its mind and nothing is in flight, so it is row 11 (`stale_mirror`). A human who happens to set exactly the
label Fleet just set produces the same state, which is harmless: the
state is what Fleet wanted.

### 5.4 Ratified values

`expedite` (15) and `next-up` (25) map to their own labels, never to
`priority/critical` or `priority/high`, and an observed `expedite`
never reads as 10. A desired value with no mirror label (30, 50, 90)
desires the **absence** of every family label; it does not desire
the nearest neighbour.

### 5.5 Project fields

DST-05 never writes a Project field and never issues a command whose
effect is a Project write. The Projector sees no Projects v2 events.
Where the orchestrator exposes its observed Project value, the
reconciler compares it with `D` and records `project_drift` in the
audit and in `sync status`; convergence is the orchestrator's own
sync. The test `pmo_distribution_no_second_project_writer` (§7)
asserts no Project effect kind is ever emitted.

## 6. Audit

Every non-trivial decision (every row except a repeated row 2/12)
writes one audit fact through the existing journal/timeline path
(REL-04 §3.1 slot 4/5; no new table from this slice):

`{schema: "fleet.priority-reconcile-audit/v1", run_id, pass: dirty|full,
target_ref, work_ref, outcome, desired: {work_version, desired_hash,
profile_ref}, observed: {observed_hash, observed_seq,
source_updated_at}, operation_id?, refusal_ref?, at}`

The audit row is what lets an operator answer "why does this label
say that" without reading code. It carries no label text beyond the
family-label names and no issue body, comment or actor login.

`sync status` (CLI-02 vocabulary) shows counts per outcome for the
last run and lists every target in `unattributed_drift`,
`ambiguous_observed`, `terminal_conflict` or `unsupported_version`.
A run never reports `verified_complete`: convergence of a mirror is
`accepted` when a command was queued and `verified_complete` only
for the operation, after REL-06 read-back, through REL-03's receipt.

## 7. Tests

Rust, `work/distribution.rs` DST-05 section (pure, no I/O):

| Test                                                         | Oracle                                                                                                                                                                                     |
| ------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `pmo_distribution_projection_replay`                         | A recorded sequence of desired and observed snapshots, replayed twice and in shuffled delivery order, yields the same outcomes, the same commands (same idempotency keys) and no rollback. |
| `pmo_distribution_duplicate_delivery_one_command`            | The same dirty ref seen twice in one pass and across two passes produces one command key.                                                                                                  |
| `pmo_distribution_older_observation_never_rolls_back`        | An older `source_updated_at` after a newer one is recorded, not acted on.                                                                                                                  |
| `pmo_distribution_same_seq_divergent_hash_terminal_conflict` | Row 5.1 second bullet.                                                                                                                                                                     |
| `pmo_distribution_echo_is_not_intent`                        | Observed equals a queued intent's payload ⇒ `projection_echo`, no command.                                                                                                                 |
| `pmo_distribution_unknown_intent_never_resent`               | An `unknown` intent ⇒ `awaiting_readback`, no command (partial remote effect negative).                                                                                                    |
| `pmo_distribution_unattributed_drift_not_overwritten`        | Hand edit with no record ⇒ conflict, no command.                                                                                                                                           |
| `pmo_distribution_refused_input_repaired_with_link`          | Refused DST-04 input (unauthorized actor) ⇒ one command whose audit links the refusal.                                                                                                     |
| `pmo_distribution_ambiguous_labels_never_picked`             | Two family labels, or `priority/medium` ⇒ `ambiguous_observed`.                                                                                                                            |
| `pmo_distribution_unknown_version_quarantined`               | Unknown schema or profile ⇒ `unsupported_version`, no command.                                                                                                                             |
| `pmo_distribution_ratified_bands_exact`                      | 15 ↔ `expedite`, 25 ↔ `next-up`, 30/50/90 ⇒ empty family; never 10 or 20.                                                                                                                  |
| `pmo_distribution_full_pass_finds_omitted_marker`            | A drift with no ledger row is found by the full pass.                                                                                                                                      |
| `pmo_distribution_no_second_project_writer`                  | No outcome emits a Project effect kind.                                                                                                                                                    |
| `pmo_distribution_no_false_verified`                         | No run result reports `verified_complete`.                                                                                                                                                 |

TypeScript, `pm-stack/projector/test/priority-reconcile.test.ts`
(in-memory libSQL, as the existing tests): the `priority-reconcile`
checkpoint never acks past the pinned high-water or before every
dirty ref is handed to the sink; a sink failure leaves the checkpoint
unmoved; the observed builder includes noncanonical `priority/*`
labels; the module imports nothing from `github.ts` and issues no
`fetch` (rate-limit and outage negatives become "checkpoint
unmoved, retried next pass").

Runner: GitHub-hosted only. Rust via `.github/workflows/fleet-rust-tests.yml`
(`cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib
--features client,queue-broker pmo_distribution_`, after `-- --list`
shows each name); TypeScript via `.github/workflows/pm-stack-tests.yml`.
A zero-test filter passing is not acceptance.

## 8. Persisted fields

None owned by this slice. It uses one `pmp_consumer_checkpoints` row
(an existing table, a new consumer name), REL-04's intent and
operation rows, and the journal/timeline audit slot. If an
implementation wants a column for the last audited observed revision
per target, it is a CEN-04 §5 admission row (fact, owner, home,
legacy mapping, disable-only rollback) before any write.

## 9. Requirement trace (design-level)

| IDs    | Outcome                                                                                   | Where             |
| ------ | ----------------------------------------------------------------------------------------- | ----------------- |
| C-R066 | GitHub labels, Projects and PM Stack projections stay projections of Fleet's value        | §3, §5            |
| C-R068 | an authorized label addition is a command (DST-04); a refused one is repaired visibly     | §5 rows 6, 7      |
| C-R077 | Projector and board are projections, not owners; no second Project writer                 | §2, §5.5          |
| C-R079 | existing Projector, orchestrator, broker and timer boundaries reused; no merged databases | §2, §4, §8        |
| H-R005 | versioned payloads, stable ids, provenance, replay and reconciliation                     | §3, §5.1, §6      |
| H-R012 | idempotent webhooks and durable receipts; projector DB never canonical                    | §4.1, §5.2        |
| O-R001 | no drift, no oscillation (echo), no stale pivots; conflicts visible                       | §5.3, §5 rows 8–9 |
| O-R018 | Fleet canonical; mutations through the shared service; idempotent reconcile               | §5.2              |

## 10. Gates

- **Hard:** REL-04's outbox and REL-06's read-back must exist; the
  reconciler has nothing to apply through otherwise. REL-03 (record
  merged, #15240) supplies the receipt.
- **Soft:** DST-04 (#9454) supplies rows 6 and 7. Until it lands,
  every observed difference not explained by an intent is row 8
  (`unattributed_drift`): safe, just noisier.
- **Registry:** the `fleet.priority-projection/v1` and
  `fleet.priority-reconcile-audit/v1` domains are added to
  `architecture.md`'s registry before any hash is emitted.
- **Shared files:** `work/distribution.rs` is created by whichever of
  DST-06, DST-07 or DST-05 lands first; the others add sections. The
  one-line hook in `server.ts`'s `/reconcile` handler is a separate
  change in a file owned elsewhere.
- **Rollback:** stop calling the reconciler from `/reconcile`.
  Queued intents finish or stay inspectable under REL-04/REL-08;
  audit rows and the checkpoint row are kept; nothing is deleted.

## 11. Non-goals

- No code, route, table, column, migration or timer.
- No GitHub or Project write from the Projector, ever.
- No automatic overwrite of an unattributed human edit.
- No change to `reconcile.ts`'s poll, the hot ledger or its existing
  consumers.
- No re-ratification or renormalization of priority values.
