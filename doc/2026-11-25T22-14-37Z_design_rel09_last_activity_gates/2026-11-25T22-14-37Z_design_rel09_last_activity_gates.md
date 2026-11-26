# REL-09 — Last-activity gates, duplicate webhook and post-completion policy

Issue: [#9519](https://github.com/developing-today/code/issues/9519)
(slice `REL-09` of [#9209](https://github.com/developing-today/code/issues/9209),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement.**
The issue asks to "define invalidating activity classes, watermark,
refresh→merge residual race, duplicate delivery and post-completion
policy". The pure policy for all five already exists on main in
`work/completion.rs` (#10776), and the REL-07 and REL-08 records
consume it. This record writes down what that code decides, settles the
four questions it leaves open, and names the oracles that a follow-up
must add. It authorizes no code, schema, remote write or deployment.

Governing decisions consumed, not re-opened:

- **#9209 component:** direct completion with an expected head SHA; a
  202 from GitHub is accepted, not merged; partial effects stay
  recoverable.
- **REL-06** (`doc/2026-11-23T12-20-33Z_design_rel06-github-preconditions-readback/`):
  `pr.merge` requires the expected head SHA and verifies per method
  (`pmo_rel_merge_rejects_stale_head_sha`).
- **REL-07** (`doc/2026-11-25T22-26-00Z_design_rel07_completion_state_machine/`):
  the completion state machine calls `evaluate_completion`, and
  `merge_allowed: false` stops at refresh
  (`pmo_rel_completion_late_activity_holds_before_merge`).
- **REL-08** (`doc/2026-11-25T22-52-00Z_design_rel08_work_complete_cli/`):
  the `work complete` receipt states `"residual_race": "activity after
<watermark> is not seen"`.
- **#9203** (closed): event routing and dedupe own delivery. **#9207**
  owns incremental activity reads; HYD-06 reads activity since the
  checkpoint for REL-07's final refresh.

## Verified state on main

Read at `origin/main` on 2026-11-25:

- `work/completion.rs` defines `ActivityClass` (nine classes),
  `ActivityRevision`, `ActivityWatermark`, `Freshness`
  {`Current`, `Limited`}, `CompletionReceipt` (with `merge_allowed`),
  `PostTerminalOutcome` and `evaluate_completion`.
- Four tests cover it: a late review after refresh is reported, a
  comment race does not limit freshness, a duplicate delivery is
  idempotent, and a post-terminal event does not reopen the receipt.
- **No caller.** Nothing outside `completion.rs` uses
  `evaluate_completion` or `ActivityRevision`. The live complete path
  (`turso_queue/admin.rs` `verb_complete`) does not read activity.
- **No revision source.** Nothing assigns `ActivityRevision` values to
  real GitHub events yet.

## 1. What the code already decides

| Question                   | Decision on main                                                                                  |
| -------------------------- | ------------------------------------------------------------------------------------------------- |
| Invalidating classes       | review, review comment, explicit approval, explicit block, close, reopen                          |
| Not invalidating           | comment, push, label change                                                                       |
| Watermark                  | the highest revision the completion read observed; it travels in the receipt                      |
| Late invalidating activity | `Freshness::Limited` with the late classes; `merge_allowed = false`                               |
| Duplicate delivery         | de-duplicated by revision; revision 0 is never a real event                                       |
| Post-terminal event        | `AlreadyCovered` at or below the receipt's boundary, else `IgnoredAfterCompletion`; never reopens |
| Locks                      | none; the policy is a pure function                                                               |

These stand. The open points are where revisions come from, how
GitHub events map to classes, why a push is not invalidating, and what
a post-completion event does to the work item.

## 2. Where revisions come from

`ActivityRevision` is "monotonic per work item" and "assigned by the
event owner". This record fixes the owner and the rule:

- **One sequence per work item, assigned by Fleet at ingestion.** The
  revision is Fleet's own insertion order in its activity inbox for
  that work item. It is never a GitHub timestamp, a GitHub id or a
  delivery order.
- **Natural key.** Each event has a natural key: provider, repository
  id, event kind and the provider's own event id (review id, comment
  id, timeline event id). A webhook delivery and a later poll of the
  same event share the natural key.
- **Redelivery is the same revision.** Ingestion looks up the natural
  key first. A second delivery of the same event (same natural key,
  any delivery id) returns the existing revision and adds nothing.
  That is what makes the code's de-dup-by-revision correct for real
  webhooks.
- **Edits are new events.** An edited review or comment gets a new
  revision under a natural key that includes the edit's own id or
  updated-at value. Its class is the class of the thing edited.

The inbox and its ingestion belong to #9207 (incremental activity)
behind #9203 routing. REL-09 only compares revisions.

## 3. Mapping GitHub events to classes

| GitHub event                                         | Class              |
| ---------------------------------------------------- | ------------------ |
| `issue_comment` created or edited                    | `Comment`          |
| `pull_request_review` submitted, edited or dismissed | `Review`           |
| `pull_request_review_comment` created or edited      | `ReviewComment`    |
| `pull_request` `synchronize`                         | `Push`             |
| `labeled`, `unlabeled`                               | `LabelChange`      |
| `closed` (issue or PR), `merged` by someone else     | `Close`            |
| `reopened`                                           | `Reopen`           |
| Fleet approve directive (DSP-08)                     | `ExplicitApproval` |
| Fleet block directive (DSP-08)                       | `ExplicitBlock`    |
| any other event type                                 | **`Unclassified`** |

Two choices here change the code and need the follow-up:

- **Unclassified holds.** An event type nobody mapped must not slip
  through as harmless. The follow-up adds `ActivityClass::Unclassified`,
  which invalidates and is not terminal. The receipt names it, so the
  next reader can add a mapping.
- **Directives are Fleet's, not GitHub's.** `ExplicitApproval` and
  `ExplicitBlock` come from Fleet's own directive intake, which
  authenticates the author (DSP-08). A GitHub comment that only says
  "approved" is a `Comment`.
- **Priority labels** are inputs to PRI-01, and they keep that path. For
  REL-09 they are only a `LabelChange`.

## 4. Why a push does not invalidate

A push moves the PR head. The decision it could invalidate is "merge
this head", and that is already bound by REL-06: the merge carries the
expected head SHA, and a moved head fails the precondition
(`pmo_rel_merge_rejects_stale_head_sha`). Counting pushes in the
activity watermark as well would hold merges twice for one cause and
would hold them for pushes to branches the merge never uses. So the two
gates stay separate:

- **Activity gate (REL-09):** did people decide something after the
  read?
- **Head gate (REL-06):** is the code being merged the code that was
  read?

A completion is clean only when both pass. The receipt reports them
separately.

## 5. The refresh→merge residual race

The watermark is taken at the refresh read. The boundary is evaluated
again just before the merge is sent and once more before the receipt is
reported. Late invalidating activity at either point holds the merge:
REL-07 phase 2 ends the operation as `conflict` with the late activity
listed, and the work goes back to refresh.

Some activity is always invisible: what GitHub has accepted but Fleet
has not yet ingested. No read closes that gap. Therefore:

- The receipt states the watermark and says that activity after it is
  not seen (REL-08's `residual_race` field).
- A `Current` verdict means "nothing that can change the decision had
  been ingested", never "nothing happened".
- No receipt claims clean reconciliation past its watermark.

## 6. Post-completion policy

After the receipt is reported, the completion is final. Events still
arrive:

| Event after completion           | Effect                                                                |
| -------------------------------- | --------------------------------------------------------------------- |
| at or below the boundary         | `AlreadyCovered`; nothing                                             |
| `Comment`, `Push`, `LabelChange` | journal `Internal` entry only                                         |
| `Review`, `ReviewComment`        | journal entry and an advisory triage hint on the work item            |
| `ExplicitBlock`                  | journal entry and an advisory triage hint naming the directive author |
| `Reopen`                         | journal entry and a triage hint; **no** automatic re-queue            |
| `Unclassified`                   | journal entry and a triage hint                                       |

None of these reopens the receipt, re-runs an effect or changes the
completion. Re-queuing reopened work is a lifecycle decision (#9178),
made through its normal verb by someone with a grant. The hint uses the
existing advisory reason codes (`EffectClass::AdvisoryHint`); it
changes no base field.

## 7. Errors and receipt fields

REL-09 adds no error. A held merge is the REL-07 phase 2 result: the
operation is `conflict` with the late activity listed. REL-08's receipt
already carries `late_activity` and `residual_race`; this record asks
REL-08 to add two more fields, so a reader can see what the decision was
based on:

| Field           | Meaning                                                         | Status        |
| --------------- | --------------------------------------------------------------- | ------------- |
| `late_activity` | late events considered, by revision                             | in REL-08     |
| `residual_race` | the fixed sentence about activity after the watermark           | in REL-08     |
| `watermark`     | the pinned revision                                             | proposed here |
| `freshness`     | `current`, or `limited` with the late classes and current value | proposed here |

## 8. Tests and gates

Existing oracles stay: `pmo_rel_late_review_after_refresh_is_reported_at_completion_boundary`,
`pmo_rel_tiny_comment_race_does_not_limit_freshness`,
`pmo_rel_delayed_duplicate_webhook_delivery_is_idempotent`,
`pmo_rel_post_terminal_event_does_not_reopen_receipt`.

The follow-up adds, in `work/completion.rs`:

1. `pmo_rel_unclassified_activity_holds_completion`.
2. `pmo_rel_push_after_refresh_is_left_to_head_sha_gate` — the
   receipt is `Current`, and the test asserts that the REL-06 head gate
   is the one that refuses.
3. `pmo_rel_redelivery_maps_to_same_revision` — two deliveries of one
   natural key produce one revision (ingestion fixture).
4. `pmo_rel_post_completion_review_raises_hint_not_reopen`.
5. `pmo_rel_reopen_after_completion_never_requeues`.

**Done command:** `cargo test --manifest-path pkgs/fleet/Cargo.toml -p
fleet-queue --lib --features client,queue-broker pmo_rel_`.

**Gates.** Code for §2 waits on #9207's inbox. The `Unclassified`
variant and oracles 1, 2, 4 and 5 can land on their own, because they
are pure. Calling `evaluate_completion` from the live path is REL-07's
slice; it needs a remote test run before merge.

## 9. Requirement trace (design-level)

| Issue deliverable             | Where      |
| ----------------------------- | ---------- |
| Invalidating activity classes | §1, §3     |
| Watermark                     | §1, §2, §7 |
| Refresh→merge residual race   | §4, §5     |
| Duplicate delivery            | §2         |
| Post-completion policy        | §6         |

## 10. Non-goals

- No webhook receiver, inbox or poller (#9203, #9207).
- No change to merge preconditions (REL-06) or the completion state
  machine (REL-07).
- No automatic reopen or re-queue of completed work.
- No claim that a watermark sees activity Fleet has not ingested.
