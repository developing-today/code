# Design: Draft approval queues, outcomes, and rejected-draft forks

- **Date:** 2026-11-08
- **Status:** design decision record for review; no implementation is included.
- **Issue:** [#5835](https://github.com/developing-today/code/issues/5835), child of [#5677](https://github.com/developing-today/code/issues/5677).
- **Parent decisions:** #5677 owns the note↔board reference model, merge model, sovereignty rule, and broad opt-in draft-approval flow. This record fills in workflow mechanics without revising those decisions.
- **Baseline:** `main` at `e1b77bba0dfa1a0d975887dee6cf0053c9190eb7` (2026-11-08). The current note event model has immutable `note.create` / `note.revise` events and preserves concurrent revision heads (`pkgs/id/src/notes.rs:1-17,39-57,150-177`); it does not define approval state or an approval queue.
- **Inputs:** #5677's “Draft → approval flow” and open questions; #2048's stable note/revision model; #5835's acceptance; the knowledge-sharing boundary in [#5840](https://github.com/developing-today/code/issues/5840) where applicable.

## Decision in brief

For an object whose effective policy requires approval, a submission is an immutable proposal for one exact revision, not a mutable flag on the live object. Keep one review queue per governed object; reviewer and author views are filtered projections over those submissions. Reviewers resolve a submission with **approve**, **reject**, or **request changes**. Each action is attributed and append-only. A rejected submission remains the submitter's fork and can be resubmitted as a new, linked submission; rejection never deletes or transfers ownership of the work.

| Question                   | Decision                                                                                                                                                                                                        |
| -------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Queue scope and visibility | One logical queue per object; reviewers see only objects they are authorized to review, and authors see their own submissions. Combined inboxes are filtered views, not a new authority or visibility boundary. |
| Ordering                   | Oldest actionable submission first (`submitted_at`, then stable submission ID); no urgency or arrival-order policy may silently change the order.                                                               |
| Delegation                 | No per-request forwarding or ad hoc delegation in v1. Decision authority is the object's owner/admin set as #5677 settles it. Changing that role set is governed there, not by a queue action.                  |
| Decision target            | A decision names the exact submitted revision and its base revision(s). Editing creates a new revision and requires a new submission.                                                                           |
| Outcomes                   | Approve authorizes that snapshot to become effective under #5677's merge model; reject preserves it as the author's fork; request-changes returns it to the author without making it effective.                 |
| Resubmission               | A changed or rejected fork may be submitted again as a new immutable submission linked to the prior one. Prior decisions remain visible and unchanged.                                                          |
| Silence and concurrency    | No timeout means approval. A pending item remains pending. Only one valid terminal decision wins an atomic transition for a submission; stale competing actions are reported, not overwritten.                  |

## 1. Scope and parent boundary

This record decides the queue, submission, decision, and rejected-fork mechanics requested by #5835. It does not choose which objects use approval, who can change that setting, or the parent project's editing/merge/sovereignty rules. Submissions identify the governed object by its stable identity and the exact revision, not by a mutable name or copied board rendering. If a board post refers to a note, the post↔note relationship remains exactly the one selected by #5677; this record adds no new snapshot/live-reference semantics to that relationship.

The parent already states that approval is opt-in, edits can be drafts pending owner/admin approval, and a rejected draft remains its author's fork. Those are constraints here, not options reopened for debate. The exact setting scope (object, collection, board, or a parent-selected combination), its defaults and inheritance, and the final definition of “owner/admin” remain **provisional pending #5677**. This mechanism begins only after an object has an effective approval policy and an authorized reviewer set.

The current `NoteOp` has only create and revise operations (`pkgs/id/src/notes.rs:39-57`); its history fold retains revisions and concurrent heads rather than selecting a winner (`pkgs/id/src/notes.rs:150-177`). Approval is therefore an additional state/decision layer, not a mutation of a revision or a reason to overwrite the revision graph. This record specifies semantics, not event names, wire format, storage, or UI toolkit.

## 2. Queue and submission model

### 2.1 Queue shape and access

- A queue belongs to one governed object and contains its pending submissions. A dashboard may combine queues for convenience, but every row remains scoped to its object and is filtered by that object's current review authority.
- An authorized reviewer can inspect the submitted snapshot, its base revision(s), relevant discussion, and prior linked submissions needed to decide. The author can inspect their own submissions and outcomes. Other readers see only the status and decision facts that the object's settled visibility and history rules permit.
- Queue membership never grants read, edit, or administrative authority. A URL, notification, or queue index is a locator, not a capability. Denied viewers must not learn restricted object content or metadata merely because a submission exists; apply the existing object read boundary to queue, search, history, and notification surfaces.
- Sort actionable entries by submission time ascending, breaking equal timestamps by stable submission ID. “Request changes” items leave the actionable queue until the author resubmits; approved, rejected, withdrawn, and superseded submissions are history, not actionable work.
- No separate delegate field exists in v1. A reviewer cannot forward a decision to an arbitrary person. The owner/admin authority set, whether named admins are supported, and whether one reviewer or a quorum is required are **provisional pending #5677**. If #5677 later adopts delegation, it must name the grantor, scope, expiry/revocation behavior, and audit record; it must not be inferred from a notification recipient.

### 2.2 Submission identity and states

Submitting captures an immutable snapshot revision, the stable object ID, the base revision ID(s), submitter, submission time, and an optional note to reviewers. A later edit creates a distinct revision; it never changes the bytes or meaning of an existing submission.

```text
editing fork --submit--> pending
pending --approve--> approved
pending --reject--> rejected (fork retained by submitter)
pending --request changes--> changes-requested
pending --submitter withdraws--> withdrawn
changes-requested --edit + resubmit--> new pending submission linked to prior
rejected --edit + resubmit--> new pending submission linked to prior
pending --base no longer usable--> stale (resubmit after rebase/merge)
```

`approved`, `rejected`, `withdrawn`, and `stale` are terminal for that submission ID. A correction or new attempt is another linked record, never an edit/reopen of the old decision. A stale base is not silently rebased: the author must create the appropriate revision against the current base and submit that snapshot for review again.

The `stale` transition and how a proposal is reconciled with changed heads depend on #5677's merge model and are **provisional pending #5677**. Regardless of that model, an approval must not discard a concurrent head or cause an arrival-order/last-writer-wins selection.

## 3. Outcomes and visibility

### Approve

Approval authorizes only the exact submitted snapshot and base named by the decision. If the base is still usable, the approved revision becomes eligible to become the object's effective version through the explicit merge/promotion rule settled by #5677. If the base has changed or the parent's merge rule requires a join, approval alone does not overwrite a newer or concurrent head: produce the required explicit merge/rebase revision and review that exact result before it becomes effective. The precise promotion and conflict path is **provisional pending #5677**.

The author and authorized reviewers can see who approved, when, and which revision was approved. Approval does not grant anyone a new read/edit role, broaden the object's audience, or imply approval of later revisions.

### Reject

Reject records a plain-language reason visible to the author and authorized reviewers. The rejected snapshot stays an attributable fork under the submitter's control; it is not moved into the reviewer's namespace, overwritten, or deleted. The reviewer cannot make it the effective version through the rejection action.

The fork's default visibility is no broader than the source object. The submitter may keep it within their own authorized/private scope or deliberately share it using the normal visibility mechanism; rejection itself grants no new audience. Exact fork ACL inheritance, public-history visibility, and how that composes with #5677's sovereignty rule are **provisional pending #5677**. No implementation may promise remote erasure of bytes already copied by an authorized reader.

### Request changes

Request-changes records the requested changes and returns control of the working fork to its author. The snapshot remains non-effective and is removed from the actionable queue until resubmitted. The author may edit, preserve the old snapshot, or withdraw. Resubmission is a new immutable submission linked to the original, so the reviewer can see what changed without rewriting history.

### Withdraw, pending, and competing actions

The submitter may withdraw a pending submission; withdrawal closes queue work but preserves the submission and event trail. There is no auto-approval or silent rejection on timeout. A pending item remains visible to the author and authorized reviewers, with age surfaced so an abandoned queue is detectable.

Decision writes are conditional on the submission still being pending and on the revision/base IDs matching the reviewed snapshot. After the reviewer threshold that #5677 selects is met, only one terminal transition commits; later actions receive the current state and cannot replace it. A reviewer who loses a race must reload before acting on a replacement submission.

## 4. Resubmission of rejected work

1. The author retains the rejected revision as a fork with its stable ancestry and attribution.
2. To try again, the author edits that fork (or creates a new fork from it), chooses a valid current base, and submits a new immutable snapshot.
3. The new submission gets a new ID and timestamp, links to the rejected submission it follows, and enters the normal queue at its new position. It does not erase or reopen the rejection.
4. Reviewers may inspect the prior decision and the author's changes, subject to the same object visibility boundary. A materially unchanged resubmission is allowed; the system may show the relationship but does not silently suppress the author's attempt.

This path preserves the author's work and the parent's “rejected draft remains the author's fork” rule while keeping each review decision auditable.

## 5. Decision record and invariants

Every submission and decision is an attributable, append-only fact. At minimum, the logical record carries: object ID; submission ID; submitter; submitted revision ID and base revision ID(s); submission time; outcome; reviewer/actor; decision time; reason or requested changes when supplied; and links to any predecessor/resubmission. The current queue is a deterministic projection of these facts. Storage, event kind, and replication encoding are implementation choices; changing a prior decision in place is not.

Implementation invariants:

1. A submission pins one immutable revision and its base; later edits cannot alter what a reviewer approved or rejected.
2. Only the authority set settled by #5677 can decide. Queue membership, links, notifications, and convenience delegation never grant authority.
3. Queue reads and metadata obey the object's existing visibility boundary; queue state never leaks restricted content or existence.
4. Approval applies only to the named revision. It cannot silently replace a newer or concurrent head, widen visibility, or approve later edits.
5. Rejection and request-changes never delete the author's work or transfer its ownership. Rejected work remains an attributable fork and can be resubmitted as a new linked submission.
6. Submission and decision history is append-only and attributed. Resubmission, correction, reversal, and withdrawal add facts; they do not rewrite the earlier record.
7. A pending submission is never approved by timeout, silence, or a queue failure. A decision race has one conditional winner and a visible stale-action response.
8. Review outcomes are distinct: approval authorizes promotion under the parent merge model; rejection closes the attempt while retaining the fork; request-changes keeps the work editable but non-effective.

## 6. Alternatives rejected

| Alternative                                           | Why it is not v1                                                                                                                          |
| ----------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- |
| Mutable “draft” flag on the live object               | It does not pin what was reviewed, makes later edits appear approved, and cannot preserve a rejected fork reliably.                       |
| One global queue visible to every owner/admin         | It mixes authority scopes and risks disclosing restricted object titles, content, or existence. Combined inboxes are filtered views only. |
| Last-write-wins or automatic approval after a timeout | Silence is not consent, and arrival order cannot resolve concurrent revisions safely.                                                     |
| Reviewer deletes or takes ownership of rejected work  | It contradicts #5677's rejected-fork rule and erases the submitter's work and provenance.                                                 |
| Reopen a rejected submission in place                 | It hides which snapshot was rejected and makes the audit trail mutable; a linked new submission is unambiguous.                           |
| Forward a particular decision to any user             | It bypasses object authority and creates an unreviewed delegation system. Any future delegation must be explicit in #5677 and auditable.  |

## 7. Provisional boundary and coordination

The following decisions remain with #5677 and must not be treated as settled by this child:

- whether the approval setting attaches to an object, collection, board, or more than one; defaults, inheritance, and who may enable/disable it;
- the exact owner/admin principal set, whether one reviewer or a quorum is required, and whether a separately delegated reviewer role is allowed;
- the parent merge model, effective-version projection, and exact stale-base/rebase/merge mechanics;
- the full sovereignty and visibility rule for a rejected fork and its history, including how it composes with the normal object ACL;
- storage/wire events, notification delivery, and which UI, CLI, API, and agent surfaces expose the queue.

This record proposes no changes to a parent decision and creates no separate issue: the queue and fork mechanics form one coherent child design. If a parent decision later conflicts, update #5677 first; this record's dependent choices remain provisional until reconciled there.

## Acceptance map (#5835)

| Requirement                                                    | Where                                                |
| -------------------------------------------------------------- | ---------------------------------------------------- |
| Queue shape, visibility, ordering, and delegation              | §2.1                                                 |
| Approve/reject/request-changes outcomes and visibility         | §3                                                   |
| Rejected fork ownership, visibility boundary, and resubmission | §3, §4                                               |
| Parent dependencies marked provisional                         | §1, §2.2, §3, §7                                     |
| Alternatives and implementation invariants                     | §5, §6                                               |
| Split decision                                                 | No split: §7 records why this is one coherent design |
