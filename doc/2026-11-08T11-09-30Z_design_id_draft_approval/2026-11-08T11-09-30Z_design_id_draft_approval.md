# Design: ID draft approval queues and rejected forks

- Date: 2026-11-08
- Status: **design decision record for #5835**; decisions depending on the still-open parent #5677 are marked provisional below. No implementation is included.
- Issue: [#5835](https://github.com/developing-today/code/issues/5835)
- Parent: [#5677](https://github.com/developing-today/code/issues/5677), ID notes, message board, and multiplayer editing v1
- Related designs: [#2048](../2026-11-01_design_id2048.md) (stable note identity and revisions); [#5840](../2026-11-08T08-19-40Z_design_id_knowledge_sharing_permissions/2026-11-08T08-19-40Z_design_id_knowledge_sharing_permissions.md) (sharing and permissions; ready for issue-owner review); #5832 (multiplayer edit sessions); #5834 (talk pages and changelog)
- Baseline: laptop `origin/main` at `b15a9b3985f3` when this record was drafted.

## Decision in brief

Treat approval as an explicit review of an immutable candidate revision, not as an edit to the target. The target's effective version changes only when an eligible approver approves the exact submitted candidate and the parent merge model can apply it without silently replacing newer work. A rejection leaves the candidate in its author's fork, with its own provenance and read policy; it neither changes the target nor deletes the fork. A request for changes returns the candidate to its author for a new, separately recorded submission.

An approval queue is a private work view for the author and eligible approvers, ordered oldest-first within each target. It does not widen access to either the target or the candidate. The workflow remains opt-in: when approval is not enabled for a target, edits are immediate as #5677 requires.

This record fills in the workflow mechanics requested by #5835. It does not revise #5677's note↔board reference model, merge model, sovereignty rule, or high-level approval flow. Parent-level questions that affect these mechanics remain provisional pending #5677.

## Scope and evidence

The parent states that approval is an opt-in gate; edits are immediate when it is off; when on, drafts need owner/admin approval; outcomes include approve, reject, and request changes; and rejection leaves the author's fork intact. It also says anyone may fork an object they can see, owners may decline adoption but cannot prevent visible-object forks, and attribution/history are not erased. These are constraints, not choices reopened here.

The parent still lists the setting's scope and who can change it as open. It does not choose the exact approver roster or settle the workflow's revision/state model. Those parent-dependent choices are explicitly provisional below.

The #2048 note model establishes stable object identity separately from immutable revisions; concurrent revisions remain separate heads until an explicit merge. Current `pkgs/id/src/notes.rs` models signed `note.create` and `note.revise` events and revision ancestry, but does not define approval state. The #5840 sharing design says note read access is resolved by the note's own policy and configured defaults, collections do not grant or inherit access, and group `Admin` manages the group roster rather than a note ACL. This approval flow must compose with the reviewed access policy rather than inventing a second permission system.

## Terms

- **Target:** the existing object whose effective version is being proposed for change.
- **Candidate revision:** an immutable revision in the author's fork, including the exact content and parent revision(s) submitted for review.
- **Submission:** a request to consider one candidate revision for one target. A submission is not itself an edit to the target.
- **Eligible approver:** the target owner or another principal explicitly authorized by the settled parent approval policy. A group role alone does not imply approval authority.
- **Rejected fork:** the author's retained line of work after a submission is rejected. It remains a fork; it is not a copy adopted by the target and is not deleted.

## Decisions

### 1. Gate and setting

1. Approval is off unless explicitly enabled. An object with approval off retains the parent's friction-free immediate-edit behavior.
2. Enabling approval does not grant read, write, or approval authority. Access to the target and candidate follows the applicable object policy.
3. Turning the gate off does not silently approve, publish, reject, or delete submissions already pending. Each pending submission remains pending until an approver decides or its author withdraws it.

**Provisional pending #5677:** whether the setting is per-object, per-collection, or per-board; how an effective setting is inherited (if at all); and who may enable or disable it. The behavior in this section applies at whichever scope the parent settles. No collection-level permission inheritance is introduced by this record.

### 2. Queue shape, access, and ordering

1. Each pending submission appears once in the target's review queue. An eligible approver may also see it in a consolidated “My approvals” view. There is no separate global queue that reveals unrelated targets.
2. Queue rows show only the target reference, author, submitted time, current review state, and candidate reference/diff needed to make a decision. Queue contents and candidate text are returned only to the author and eligible approvers who can read the candidate under the settled access policy. If an approver cannot read the candidate, the submission cannot be decided by that principal; the system must not expose candidate content or restricted metadata to make the queue convenient.
3. Within a target, pending submissions are ordered by submission time, oldest first, with a stable submission-ID tie-break. Different targets have independent queues; there is no cross-target fairness or priority promise.
4. A submission freezes the candidate revision it names. Later author edits create another revision and do not change what an approver is reviewing.
5. V1 has no implicit delegation. A group `Admin` or `Write` role does not by itself confer approval authority. This follows the adjacent #5840 sharing design, where group administration is not note-ACL administration and editing authority is distinct from policy administration.

**Provisional pending #5677:** the exact approver roster, whether one eligible approver or a larger quorum is required, and whether the parent will define named delegates. The queue can render the settled roster without changing its ordering or access rules.

### 3. Review outcomes

| Outcome | Queue effect | Target effect | Candidate/fork effect |
|---|---|---|---|
| **Approve** | Terminal for this submission; remove it from pending review. | Authorizes publication of the exact reviewed candidate through the parent merge model. It must not silently replace a newer target head. | Preserve the reviewed revision and its attribution as history. Approval does not rewrite the candidate. |
| **Request changes** | Non-terminal for the design as a whole, but this submission leaves the actionable queue and returns to the author. | No change. | Keep the fork. The reviewer supplies actionable feedback visible to the author and eligible reviewers. The author may edit and submit a new candidate. |
| **Reject** | Terminal for this submission; remove it from pending review. | No change. | Keep the candidate as the author's rejected fork, with its ancestry and provenance. A rejection reason is required. |
| **Withdraw** (author) | Remove the pending submission from the queue. | No change. | Keep the fork and record the withdrawal; withdrawal is not rejection or deletion. |

An approval decision applies only to the frozen candidate revision. If the target has advanced since submission, the system must surface the changed base and follow the parent merge/conflict rules; it must not convert approval into last-writer-wins replacement. The candidate must be re-reviewed if resolving the conflict changes the content being adopted.

**Provisional pending #5677:** the exact publication/merge operation when the target has concurrent or newer heads. This record adopts, and does not choose, the parent's merge model. An approval must not bypass that model.

### 4. Rejected-draft-as-fork and resubmission

1. The submitter remains the author/owner of the rejected fork. Rejection does not transfer ownership to the target owner, erase the candidate, or change the target's effective version.
2. The fork keeps its stable lineage and immutable revisions. The rejection is an attributed outcome linked to the submission, not a rewrite of the candidate or its history.
3. A rejected fork keeps its own effective read policy. It does not inherit the target's audience, group grants, or a collection's audience merely because it was submitted there. Sharing it remains subject to the settled object access rules; a target reference is a locator, not a grant.
4. The author can revise the fork and resubmit it to the same target, or use it as the base for another fork. Resubmission creates a new submission ID naming the new immutable candidate revision and points back to the earlier submission/outcome. Previous rejection and feedback remain in history; they are not reopened or overwritten.
5. At most one active submission for a given candidate revision and target is allowed. The author must withdraw or resolve that submission before submitting that exact pair again. A new revision may be submitted while a different candidate revision is pending only if the parent review model permits multiple independent candidates; otherwise the system must make the active submission visible and require withdrawal first.

The last sentence in decision 5 depends on the parent's final queue/concurrency choice and is **provisional pending #5677**. The non-negotiable rule is that duplicate submissions of the same candidate-target pair cannot create ambiguous competing decisions.

### 5. Outcome visibility and record

The author receives the decision and its required rationale for request-changes/rejection. Eligible approvers can inspect the submission and its outcome. The target's audience must not gain candidate content or private review comments solely because a submission exists. When the parent changelog design records the event, it should retain actor, timestamp, target, candidate revision, outcome, and the prior submission reference; content and rationale are readable only under the relevant access policy.

**Provisional pending #5677 and #5834:** the exact event/changelog schema and which outcome metadata is visible in the target's public history. This record requires attributed, durable history consistent with the parent, but does not pre-empt the per-object changelog design.

## Alternatives rejected

| Alternative | Why it is not the default here |
|---|---|
| Require both owner and every admin to approve | Creates an implicit quorum, queue deadlocks, and makes “owner/admin approval” stronger than the parent currently says. Quorum remains for #5677 to settle. |
| Let any group `Admin` or `Write` member approve | Conflates roster administration or revision-writing with authority to adopt work; conflicts with the separation in the #5840 sharing design. |
| Publish a submitted draft to everyone who can read the target | Makes a review request widen visibility and can expose a candidate the author has not shared under its own policy. |
| Auto-publish on rejection or when the gate is disabled | Turns an administrative setting change into an implicit approval and may overwrite work without a review decision. |
| Replace the target with the approved candidate regardless of newer heads | Loses concurrent work and conflicts with #2048's explicit multi-head/merge model. |
| Delete or transfer a rejected candidate | Contradicts #5677's rejected-draft-as-author-fork rule and user sovereignty. |
| Keep rejected items in the actionable queue indefinitely | Makes queue state misleading and does not distinguish a new resubmission from the already-decided request. Preserve history instead. |

## Invariants for implementation

1. Approval is opt-in; when off, edits remain immediate.
2. A submission names one immutable candidate revision and target. Reviewers decide the revision they saw, never a mutable draft pointer.
3. Only an eligible approver can decide. The authority check is made at decision time, and concurrent terminal decisions cannot both win.
4. Approval does not bypass the settled merge model or silently discard a newer target head.
5. Request-changes, rejection, and withdrawal never alter the target's effective version.
6. Rejection never deletes the candidate, erases attribution, or transfers authorship; resubmission creates a new linked submission.
7. A queue or submission does not widen target or candidate read access. Restricted content and metadata remain unavailable to unauthorized readers.
8. Group administration, group edit authority, and approval authority remain distinct unless #5677 explicitly settles otherwise.
9. Changing or disabling the approval setting never turns a pending submission into an approval.
10. Submission identity, review outcome, actor, and time are durable and auditable under the final changelog model.

## Implementation order and acceptance

1. Settle the provisional roster, setting scope/authority, and quorum questions in #5677; reconcile this record if the parent chooses a different rule.
2. Define a durable submission record that binds target, exact candidate revision/base, author, submission time, state, and prior submission/outcome links.
3. Add authorization-gated queue projections and test ordering, access denial, missing candidate access, and deterministic tie-breaking.
4. Add decision transitions with a race oracle: at most one terminal outcome wins for a submission; request-changes and withdrawal preserve the target; approval cannot silently drop a newer head.
5. Test rejection and resubmission against the invariant that every candidate revision and outcome remains attributable and that the target is unchanged until an approved candidate is safely applied.
6. Integrate outcome events with #5834's settled changelog and #5840's settled read policy before enabling the gate for shared content.

This record does not split the approval workflow into further issues: the queue, decision, fork, and resubmission rules are one lifecycle. No change to a parent #5677 decision is proposed in this record.
