# REL-07 — Completion prepare/partial/recovery state machine

Issue: [#9517](https://github.com/developing-today/code/issues/9517)
(slice `REL-07` of [#9209](https://github.com/developing-today/code/issues/9209),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement.**
This record elaborates the REL-07 row of the merged design package
(`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
PR #9225): "explicit resource allowlist, policy effect set,
`completing` recovery, no-webhook normal success", with the acceptance
"directly close local canonical item only after all required effect
receipts; preserve partial effects". Component #9209 §5 already gives
the phases in prose. This record turns them into states, transitions
and finalize conditions that a pure model in `work/completion.rs` can
check, reconciles them with the `/complete` verb and the lifecycle
model on main, and names what the owner still has to decide. It
authorizes no code, schema, remote write or deployment (§9).

Governing decisions consumed, not re-opened:

- **Component #9209 §5.1–§5.3:** prepare in one short local
  transaction with no HTTP; final refresh; merge with the expected head
  SHA; a 202 stays `completing`; verify by merge method, never infer
  merged from closed; close only designated issues, never reopen to
  simulate rollback; finalize in one local transaction "after all
  required effect receipts are confirmed"; the completion operation may
  finalize after the initiating lease expires but cannot let a stale
  worker make a new mutation. "No later webhook/sweep is needed on
  normal success."
- **Component #9209 §3.2 and §4:** the fence protects local writes
  only; once a remote send may have happened the effect is `unknown`
  and needs readback; no cancellation fiction; per-target order.
- **REL-03** (`doc/2026-11-25T20-47-34Z_design_rel03_idempotency_identity_receipts/`):
  operation id, principal-scoped key and digest, stored receipt; a
  retry with the same key returns the receipt.
- **REL-09** (`work/completion.rs` on main): `evaluate_completion`
  pins an activity watermark and returns `merge_allowed: false` when
  invalidating activity landed after the pinned read.
- **CLI-01** (`work/lifecycle.rs` on main): the V2 transition table
  has `active → completing` and `completing → completed | cancelled |
failed`, and no edge back from `completing`.

## Verified state on main

Read at `origin/main` on 2026-11-25:

- **`/complete`** (`turso_queue/admin.rs` `verb_complete`) is one
  step: it checks the caller holds the claim (`claimed_by`), runs
  `gate_completion` and, when a disposition is supplied,
  `disposition::require_verified`, all before any mutation, then
  closes the task. A rejected completion leaves the task claimed. It
  makes no GitHub call and has no `completing` step.
- **Queue states** (`lifecycle.rs` `QueueState`): `queued`, `claimed`,
  `parked`, `blocked`, `handed_off`, `done`, `cancelled`. There is no
  stored `completing`. The V2 `WorkflowState::Completing` exists only
  in the pure model.
- **`work/completion.rs`** holds REL-09 only (`ActivityClass`,
  `ActivityWatermark`, `Freshness`, `CompletionReceipt`,
  `evaluate_completion`). No completion operation or effect model.
- **Fence:** REL-02 (#9487) is in review on PR #15281; REL-01's record
  is on main (`doc/2026-11-25T01-23-08Z_design_rel01_claim_lease_compat_9512/`).
- Merges and issue closes today happen through `fleet gh`, outside the
  queue; nothing ties them to a completion receipt.

## 1. Intent

When an agent finishes, one command should merge the right PR, close
the right issues and close the queue item, and say exactly which of
those happened. If the process dies or an answer is lost anywhere in
between, running the same command again must find out what already
happened on GitHub and finish from there, without merging twice,
closing the wrong thing, or waiting for a webhook.

## 2. Ownership

| Concern                                        | Owner                | This slice                           |
| ---------------------------------------------- | -------------------- | ------------------------------------ |
| Completion operation and effect state machines | `work/completion.rs` | **owns** (pure model)                |
| Freshness and late activity                    | REL-09, same file    | consumer                             |
| Operation identity, receipts                   | REL-03               | consumer                             |
| Durable intent, relay and ordering             | REL-04, REL-05       | consumer                             |
| Fence generation                               | REL-01, REL-02       | consumer                             |
| Effect preconditions and readback calls        | REL-06               | consumer                             |
| Lifecycle transition table                     | CLI-01 / #9206       | consumer; §8 asks one owner decision |
| CLI command and human/JSON output              | REL-08               | not here                             |
| Final refresh cursor                           | HYD-06 / #9207       | consumer                             |
| Authorization of each effect                   | AUTH-01, DSP-08      | consumer                             |

## 3. The effect set and the allowlist

A completion request names its effects. Nothing is added by inference.

| Effect          | Target                    | Required precondition                                   | Verified by (readback)                                             |
| --------------- | ------------------------- | ------------------------------------------------------- | ------------------------------------------------------------------ |
| `merge_pr`      | owner/repo + PR number    | expected head SHA; method (`merge`, `squash`, `rebase`) | PR `merged: true`, merge commit SHA, method-specific relation (§5) |
| `close_issue`   | owner/repo + issue number | the merge effect confirmed, when policy orders it after | issue `state: closed` and reason                                   |
| `finalize_task` | the queue task            | every required effect `confirmed`                       | the local finalize transaction committed                           |

Rules:

1. **Explicit only.** Every remote target is named in the request.
   A parent, epic, component or "all linked issues" is never added.
2. **Linked check.** Each named target must also be an active link of
   the work with a matching kind (`pr_for` for a merge, `implements`
   for a close; CLI-12). A named but unlinked target is refused with
   `completion_ref_not_linked`, unless an owner-confirmed override is
   recorded. This catches a typo'd number before it closes the wrong
   issue.
3. **Required or optional.** Policy marks each effect. A required
   effect that cannot be confirmed keeps the work in `completing`. An
   optional effect may end `skipped` or `failed` and is reported; it
   never blocks finalize.
4. **Order.** `merge_pr` before any `close_issue` that policy ties to
   it; issues close in request order; `finalize_task` last.
5. **Authorization** for each effect is checked at prepare and again
   before each send (DSP-08 §7).

## 4. State machines

**Effect status** (one per effect):

| From      | To                              | When                                                       |
| --------- | ------------------------------- | ---------------------------------------------------------- |
| `planned` | `sending`                       | relay takes the effect                                     |
| `planned` | `skipped`                       | optional effect not wanted after refresh                   |
| `sending` | `confirmed`                     | answer plus readback match                                 |
| `sending` | `pending`                       | 202 accepted / merge queued                                |
| `sending` | `unknown`                       | timeout, reset or 5xx after the request left               |
| `sending` | `conflict`                      | 409 head mismatch, review or check gate failed             |
| `sending` | `failed_terminal`               | 401/403/404/422                                            |
| `sending` | `planned`                       | refused before sending (rate limit); same operation        |
| `pending` | `confirmed` / `failed_terminal` | readback of the queued merge                               |
| `unknown` | `confirmed`                     | readback shows the intended result                         |
| `unknown` | `planned`                       | readback proves it did not apply; resend same operation    |
| `unknown` | `conflict`                      | readback shows another result (closed unmerged, other SHA) |

`confirmed` never moves again. A `conflict` or `failed_terminal`
effect moves only by a new, explicitly authorized operation.

**Operation status** (derived, never set by hand):

| Status        | Meaning                                                                |
| ------------- | ---------------------------------------------------------------------- |
| `prepared`    | local transaction committed, nothing sent                              |
| `in_progress` | some effect `sending`                                                  |
| `pending`     | some required effect `pending` or `unknown`; none failed               |
| `partial`     | some required effect `confirmed` and some `failed_terminal`/`conflict` |
| `conflict`    | refresh or a precondition stopped it before any required effect landed |
| `completed`   | finalize committed                                                     |

**Work lifecycle:** `active → completing` at prepare;
`completing → completed` at finalize. `partial`, `pending` and
`conflict` all leave the work in `completing` with every receipt kept.

## 5. Phases

1. **Prepare** (one local transaction, no HTTP). Check the
   authenticated principal, current fence generation, transition
   `active → completing`, no open hard blocker, the §3 allowlist and
   the REL-03 key. Write the operation, one effect row per target, the
   expected head SHA and the work version, and the journal event.
   Same key and digest returns the stored receipt instead.
2. **Final refresh.** Read activity since the checkpoint (HYD-06) and
   call REL-09 `evaluate_completion`. `merge_allowed: false` stops
   here with operation `conflict` and the late activity listed. A
   refresh is not a lock; the residual race is disclosed on the
   receipt as REL-09 already does.
3. **Merge.** Send with the expected head SHA. 409 → `conflict`.
   202 or merge queue → `pending`; poll within the command deadline,
   else return `pending`. Never report merged from acceptance.
4. **Verify merge** by method: for `merge`, the merge commit and the
   PR's `merged` record; for `squash` and `rebase`, the PR's `merged`
   record and the returned commit SHA. A closed, unmerged PR is not
   merged.
5. **Close issues**, each its own effect and receipt. One failing does
   not undo another, and nothing is reopened.
6. **Finalize** (one local transaction). Commit only when all hold:
   every required effect `confirmed`; the work is still `completing`
   under this operation; no newer cancel, hold or revocation; and the
   existing `/complete` gates (`gate_completion`, and
   `disposition::require_verified` when a disposition is given) pass.
   Then write the per-effect receipts, `completed`, the queue's `done`
   and release the lease. Normal success needs no webhook or sweep.
7. **Return** a `fleet.result/v1` with every effect's status and the
   next action. REL-08 renders it.

## 6. Recovery

Recovery runs when the same command is retried with the same key, or
when the reconciler finds a `completing` work with an open operation.

1. Load the operation; never prepare a second one for the same work.
2. For each effect in `sending`, `pending` or `unknown`, read back
   through REL-06 before anything else: the PR's merged state and
   commit SHA, the issue's state. Match → `confirmed`. Proven not
   applied → `planned` and resend under the same operation. Different
   result → `conflict`.
3. Never resend a merge or close whose outcome is not proven absent.
4. Then continue from the first effect that is not `confirmed`, and
   finalize when §5.6 holds.

**Lease and fence.** Finalize is scoped to the operation, so it may
commit after the initiating worker's lease expired, as long as §5.6
holds. A stale worker may not prepare a new operation, add an effect
or touch any other local state; its late answers are recorded on its
own operation as evidence only. A new holder of a `completing` work
must reconcile the open operation first.

**Webhooks** are hints. A delivery for a Fleet-made merge or close is
matched to the effect and deduplicated; it can move `unknown` to
`confirmed` after readback, never straight to `completed`.

## 7. Tests

| Test                                                                       | Proves                                                                                                     |
| -------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| `pmo_rel_completion_recovers_merge_after_lost_ack_without_webhook` (first) | crash after the remote merge, before the local receipt: retry reads back, confirms, finalizes, merges once |
| `pmo_rel_completion_partial_issue_close_stays_completing`                  | merge confirmed, one close fails: `partial`, merge receipt kept, no reopen                                 |
| `pmo_rel_completion_async_merge_pending_not_completed`                     | 202: `pending`; finalize refused until readback                                                            |
| `pmo_rel_completion_head_mismatch_conflict`                                | 409: `conflict`, no merge of the new head                                                                  |
| `pmo_rel_completion_closed_unmerged_is_not_merged`                         | closed PR without `merged` never confirms                                                                  |
| `pmo_rel_completion_squash_verified_without_ancestry`                      | squash verifies from the PR record, not ancestry                                                           |
| `pmo_rel_completion_only_named_linked_refs`                                | unnamed epic never closed; named but unlinked refused                                                      |
| `pmo_rel_completion_stale_worker_cannot_add_effects`                       | expired lease: no new operation or effect; evidence recorded only                                          |
| `pmo_rel_completion_finalize_after_lease_expiry_scoped`                    | operation-scoped finalize succeeds; a newer cancel blocks it                                               |
| `pmo_rel_completion_late_activity_holds_before_merge`                      | REL-09 `merge_allowed: false` stops at refresh                                                             |
| `pmo_rel_completion_duplicate_webhook_no_double_transition`                | duplicate delivery changes nothing                                                                         |
| `pmo_rel_completion_existing_gate_still_applies`                           | finalize refused when `gate_completion` would refuse today                                                 |
| `pmo_rel_completion_same_key_one_operation`                                | concurrent same-key requests yield one operation                                                           |

Command: list, then run
`cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_rel_completion_recovers_merge_after_lost_ack_without_webhook`.
A zero-test filter is not a pass. The remote side is a fake with an
exact request ledger supporting apply-then-timeout, 409, 202 then
merged or failed, the three merge methods and partial closes.

## 8. Persisted fields and gates

Persisted (inside REL-03/REL-04's operation store): the operation, its
effect rows with status, expected head SHA, method, returned commit
SHA, issue state, attempt history, and the work version and fence at
prepare. On the task: an additive `completing` status.

- **Hard blockers:** REL-01 to REL-06, HYD-06, AUTH-01.
- **Owner and storage decisions this record does not make:**
  - adding the stored `completing` status (#9178 and the storage owner
    must approve it as additive);
  - whether the lifecycle table gets `completing → active` for an
    abandoned completion where every effect is `planned` or proven not
    applied. Without it, an abandoned completion can only end
    `cancelled` or `failed`. This record recommends the edge under that
    condition;
  - which effects policy marks required.
- **Stages each needing their own evidence and approval:** storage
  change, relay wiring, authenticated merges and closes, deployment.
- **Compatibility:** `/complete` keeps working as the one-step close;
  its gates are reused at finalize, never bypassed.
- **Rollback:** stop preparing new completions. Open operations stay
  in `completing` with every receipt and remain inspectable and
  recoverable; nothing is reset or deleted.

## 9. Requirement trace (design-level)

| ID(s)                                                                                                          | Outcome (crosswalk, abridged)                                                  | Where     |
| -------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------ | --------- |
| C-R007, C-R143, C-R145, O-R034, P-R022, P-R033                                                                 | Direct verified completion: merge, close designated issues, close queue item   | §3, §5    |
| C-R144, P-R047, P-R062                                                                                         | Validate lease, auth, dependencies, late activity, head SHA before merge       | §5.1–§5.3 |
| C-R146, C-R147                                                                                                 | No webhook needed on normal success; webhook and sweeper for recovery          | §5.6, §6  |
| C-R148, P-R038, P-R045, P-R060                                                                                 | Merge succeeds, close fails: recoverable `completing`, precise partial report  | §4, §5.5  |
| C-R140, C-R141, P-R044, P-R059                                                                                 | Stable ids and expected versions; verify before retry                          | §4, §6    |
| H-R015, O-R032, P-R021, P-R053, P-R068                                                                         | Transactional intent, durable retry, remote verification, inspectable state    | §5.1, §6  |
| O-R027, P-R042, P-R057                                                                                         | Fencing; stale workers cannot write; recovery after termination                | §6        |
| P-R043, P-R058                                                                                                 | GitHub unavailable: durable intent, `pending`, not success                     | §4        |
| P-R007                                                                                                         | Execution state distinct from GitHub state; a merge alone is not completion    | §4, §5.6  |
| O-R035, P-R023, H-R016                                                                                         | Races, lost acknowledgements, partial completion, duplicate webhooks           | §6, §7    |
| P-R040                                                                                                         | No unsafe merge to beat a deadline                                             | §5.2      |
| C-R005, C-R009, C-R107, C-R108, C-R110, H-R017, O-R023, P-R009, O-R025, P-R011, P-R001, P-R029, P-R025, H-R001 | Composed, recoverable workflow from safe primitives; reuse existing mechanisms | §2, §8    |
| C-R150, O-R050, O-R052, P-R036                                                                                 | Resume and finish walkthroughs                                                 | §6        |
| H-R024, H-R031, P-R027                                                                                         | Original errors and failures stay in coverage; realistic acceptance            | §7        |
| P-R041, P-R056, P-R046, P-R061, P-R048, P-R063                                                                 | Claim races, managed content conflicts, cycles: consumed from their owners     | §2        |

## 10. Non-goals

- No CLI command or output format (REL-08).
- No new queue, outbox engine or relay (REL-04, REL-05).
- No Project field writes; the Project orchestrator stays the only writer.
- No automatic reopen, rollback or compensation of a confirmed effect.
