# Design decision record: curated-board submissions — intake queues, intake roles, notifications

Issue: [#5857](https://github.com/developing-today/code/issues/5857)
(follow-on design from parent [#5680](https://github.com/developing-today/code/issues/5680),
decomposition item 5). Programme: epic [#2056](https://github.com/developing-today/code/issues/2056).
Boards v1: [#2054](https://github.com/developing-today/code/issues/2054).
Status: decided (this record), 2026-11-08. Implementation is a separate,
later decision — this record settles the shape only, per the parent
issue's filing note.

Baseline: `origin/main` at `83b7a024b8`, worktree `~/git/fleet/12932-5857-curated-board-submissions`.
Verified at that baseline: `pkgs/id/src/boards.rs` implements the signed
board graph (board root, rules, forum, thread, post, attachment,
membership, role attestation, moderation action, fork, supersede,
tombstone, theme, board listing) and the three-layer view
(`BoardGraph::compose_view`, `ViewLayers`, `BoardView`). There is **no
intake, queue, submission or notification object of any kind**: the only
authority vocabulary is `BoardRole::Moderator`, granted and revoked by
owner-signed `RoleAttestation`, and the only per-object state changes are
append-only `ModerationAction` (`hide`/`restore`), `Supersede` and
`Tombstone`. `Post` carries `board`, `thread`, `parent_post`, `body`,
`attachments`, `embed_ref` only — no visibility or pending state. No
design record for curated-board submissions exists on main, and no open
PR implements one.

This record applies, and does not re-decide, the neighbouring records:
post semantics and the read-authority/posting-rights gates
([#5856](https://github.com/developing-today/code/issues/5856), PR
[#7605](https://github.com/developing-today/code/pull/7605)), the
subscription/feed/notification-mode model
([#5862](https://github.com/developing-today/code/issues/5862), PR
[#7586](https://github.com/developing-today/code/pull/7586)), and the
note→board sharing model ([#5830](https://github.com/developing-today/code/issues/5830),
PR [#7581](https://github.com/developing-today/code/pull/7581)).

## Scope and non-goals

This record settles the three questions the ticket names, for **curated**
boards only:

1. **Approval queues** — how a submission waits for a decision, and what
   a decision is.
2. **Board roles for intake** — who may decide, and how that authority
   exists without accounts.
3. **Notifications for curated boards** — who hears about what, and how
   it is muted.

Not settled here, and not re-decided:

- **Post semantics** (reference vs snapshot, pinning, presentation,
  widening, withdrawal): #5856. This record fixes *where* the intake
  decision hooks into that flow (§2.5 of that record) and nothing else.
- **Subscription, discovery and feed**, including `notify_mode` on
  subscriptions: #5862. This record defines only the *intake* events that
  #5862's notification machinery delivers.
- **Moderation roles, reports, audit and appeals** on published content:
  [#5861](https://github.com/developing-today/code/issues/5861) on
  [#835](https://github.com/developing-today/code/issues/835). Intake is
  a pre-publication decision; it is not moderation, and this record does
  not add a moderation power (§5).
- **Theme format and precedence** ([#5858](https://github.com/developing-today/code/issues/5858)),
  the live-object embed contract ([#5859](https://github.com/developing-today/code/issues/5859)),
  and the note→board sharing model (#5830, applied unchanged).
- **Closed/encrypted boards**: not v1 (#2054). Every rule here is stated
  for public boards, where "not yet published" is a projection state and
  not a confidentiality guarantee (§7).

This record changes no #2054 v1 object semantic, grants no board power
over objects it references, and introduces no second identity or
signature system.

## Decision 1 — A board declares its intake policy; the default stays open

- **Intake policy is a signed, versioned board object**, `IntakePolicy`,
  in the existing `BoardObject` vocabulary and the existing signed
  envelope. It is board-scoped like `Rules`, and like `Rules` it is
  versioned: a revision is a new immutable object, and the board pins the
  rules version a thread opened under, so a policy change never rewrites
  a decision already made.
- **Two modes only:** `open` and `curated`. There is no third mode in v1
  (no per-forum mode, no per-author exemption, no "trusted posters skip
  the queue").
- **`open` is the default and is the absence of a policy.** A board with
  no `IntakePolicy` object behaves exactly as it does today: a post
  inserted into the board graph is in the board's view. This keeps every
  existing board and every existing signed set unchanged — the same
  compatibility rule #2054 used for `Board::theme_ref` (a new
  `Option` field with `serde(default)`; old bodies decode as absent).
- **`curated` means: a post is not in the board view until an intake
  decision admits it** (§3). The policy object carries the board, the
  revision, the mode, and the intake-role requirement (§2); it carries no
  list of pending items — pending state is derived from the graph, never
  stored in the policy.
- **Who may set the policy:** the board owner, and only the board owner.
  Switching `open → curated` is an owner action recorded on the board
  (a new signed revision naming the prior one), visible to every reader
  of the board. Switching `curated → open` is the same action in reverse;
  it admits nothing retroactively (§3, "no silent admission"). Moderators
  cannot change the intake mode: a role that can admit content must not
  also be able to remove the queue.
- **An `open` board cannot be given a queue by a third party.** There is
  no object by which anyone but the owner changes a board's intake mode.

## Decision 2 — Intake authority is a board-scoped role, granted by the owner, revocable, and separate from moderation

- **One new role: `BoardRole::Intake`.** Like `Moderator`, it is
  board-scoped and exists only as an owner-signed `RoleAttestation`
  (`granted: true` / `granted: false`, with `supersedes` naming the prior
  attestation). There is no account, no registration, no board-local
  identity: the role subject is the canonical Ed25519 key, and the
  attestation is verifiable from the graph alone.
- **Who may decide an intake item:** the board owner (always — ownership
  is the board root's signer) and any key holding an active `Intake`
  grant. Nobody else. A `Moderator` grant does **not** imply `Intake`,
  and an `Intake` grant does **not** imply `Moderator`: admitting a post
  and moderating a published post are different powers with different
  audit consequences, and v1 refuses to bundle them implicitly. An owner
  who wants one person to do both grants both.
- **Revocation takes effect on the view, not on history.** A revoked
  intake role cannot decide new items; decisions it already signed stay
  in the graph and stay attributable. This is the same
  acceptance-vs-enforcement split `BoardGraph::has_role` already
  documents for moderators: the substrate records grants and revocations
  in any arrival order, and the view layer decides who may act *now*.
- **The owner's own submissions do not queue on their own board.** An
  owner posting to a curated board is admitted by the act (the owner is
  the admitting authority). This is not a bypass: the post and the
  implicit admission are both signed and both visible in the audit
  surface (§6).
- **Intake authority is not moderation authority over the target.** An
  intake decision says "this post may appear on this board". It never
  grants the decider any power over the referenced object, its author, or
  another board (#5856's no-widening rule).

## Decision 3 — A submission is a real signed post plus a signed intake decision; pending is a projection

- **The submission is a real post, signed at submission time.** A
  submission to a curated board is the same signed `Post` object #5856
  defines, inserted into the board graph immediately. Provenance,
  pinning, presentation, commentary and widening are fixed at submission,
  not at approval, so an approved post is exactly the post that was
  submitted — approval cannot rewrite it.
- **A submission is bound to the intake policy it was made under.** The
  board-share record that submits the post cites the `IntakePolicy`
  revision in force at submission time (`submitted_under_policy`). The
  citation is part of the signed submission, not metadata a peer infers
  from arrival order: two posts with no `IntakeDecision` are
  distinguishable by the policy revision each cites, forever. A
  submission made while the board's policy was `open` cites an
  open-mode revision; one made under `curated` cites a curated-mode
  revision. This binding is what lets the projection rules in this
  section and §4 survive mode switches (§3, "No silent admission")
  without consulting wall-clock order.
- **The intake decision is its own signed object**, `IntakeDecision`:
  `{board, submission, decision: admit | decline | unadmit, kind:
  initial | reversal, reverses: decision_id | null, reason,
  policy_revision}`. An `initial` decision has `reverses: null`. A
  `reversal` (§4) must cite, in `reverses`, the exact decision it
  supersedes; a decision that cites nothing is an initial decision
  whatever its author intended. The shape therefore distinguishes a
  deliberate reversal from a competing initial decision in the data,
  not in prose.
  It is append-only and attributable, in the same family as
  `ModerationAction`, and it cites the `IntakePolicy` revision it was
  made under. It is **not** a `ModerationAction`: nothing is being hidden
  from a public view, because nothing was ever in it.
- **Pending is derived, never stored.** On a `curated` board, a post is
  *pending* while it has no `IntakeDecision` naming it; *admitted* while
  its earliest `admit` decision stands; *declined* while its earliest
  `decline` decision stands. The board view (`compose_view`) includes
  admitted posts, excludes pending and declined posts, and reports both
  in the `hidden` list with the layer that excluded them — the same
  transparency rule the three-layer view already applies: hidden is not
  erased, and the reason is visible.
- **One resolution algorithm: earliest initial wins; only a citing
  reversal supersedes.** Two intake decisions for one submission are
  legal signed history (two deciders, or a retry). Projection is a
  single deterministic procedure every peer runs over the same signed
  set: (1) order all decisions for the submission by the same
  deterministic order the graph already uses for duplicate `Rules`
  revisions (content-hash order within the same cited frontier); (2)
  the earliest `initial` decision is the standing outcome — every
  later `initial` decision is recorded, not applied, exactly as
  before; (3) a `reversal` applies only when its `reverses` field
  cites the currently standing decision and its signer is the board
  owner or an active `Intake` holder at decision time; an applying
  reversal becomes the standing outcome, superseding its predecessor;
  (4) a reversal citing a decision that is not standing (a stale,
  competing, or never-applied one) is recorded, not applied; of two
  reversals citing the same standing decision, the earliest by the
  order in (1) applies and the other is recorded, not applied. No
  other decision ever changes the projection. "Latest decision wins"
  is therefore never the rule: the standing outcome changes only along
  an unbroken chain of citing reversals rooted at the earliest
  initial decision, so every peer holding the same set projects the
  same outcome.
- **A declined submission stays in the graph.** It is not deleted, not
  tombstoned, and not hidden from its author or from the board's audit
  surface. The author may submit a new post (a new share event, per
  #5856's rule that re-pointing is a new share, not an edit). The board
  holds no power over the submission's target.
- **No silent admission.** Switching a board from `curated` to `open`
  does not admit the backlog: posts pending at the switch remain pending
  until decided, and the policy revision records the switch. A board
  owner who wants to clear the queue does so by deciding, or by a
  separate, explicit bulk-admit act that names each submission it admits.
- **`open` boards have no pending state for posts submitted under
  an open policy.** A post with no `IntakeDecision` is published by
  the open-board projection only when its `submitted_under_policy`
  citation names an open-mode revision. A post submitted under a
  curated-mode revision keeps the pending projection of the policy it
  was submitted under — including after a `curated` → `open` switch —
  until an `IntakeDecision` or an explicit bulk-admit act (§3, "No
  silent admission") disposes of it. The same post object is valid on
  both modes; the projection follows the policy the submission cites,
  not the board's current mode and not arrival order.

## Decision 4 — Reversal is a new signed act: `unadmit`

- **An admitted post can be removed from the board view by an intake
  decision, not by a moderation action.** `IntakeDecision { decision:
  unadmit, kind: reversal, reverses: <the standing admit> }` by the
  owner or an active `Intake` holder removes the post
  from the board view from that point on, with a reason, and leaves the
  post, the admission, the unadmission and the chain in the graph.
- **Why not reuse `ModerationAction::Hide`:** a hide is a moderation
  statement about a published object and belongs to the moderation log
  and its appeals process (#5861). An unadmission is the intake role
  withdrawing its own earlier admission. Keeping them separate keeps
  each role's actions legible in the audit surface and prevents an
  intake grant from becoming a moderation grant by accident.
- **`unadmit` does not touch the target.** It removes the post from one
  board's view. The referenced object's author keeps their own
  withdrawal power (#5856 §4), which acts on the post's rendering of the
  target and propagates to reposts; the two powers are independent and
  neither erases the other's record.
- **Re-admitting a declined or unadmitted post is a reversal, not a
  second initial decision.** It takes a new `IntakeDecision` with
  `kind: reversal` citing the standing decline (or the standing
  `unadmit`) in `reverses`, per the single resolution algorithm in
  §3. A bare second `admit` — an initial decision citing nothing —
  is recorded, not applied, whoever signs it. The graph records the
  whole sequence; the view follows the standing decision produced by
  §3's algorithm, never simply the latest decision.

## Decision 5 — Notifications are intake events delivered by the subscription machinery, and nothing more

- **Intake emits events, it does not own a notification system.** The
  events are: `submitted` (a post is pending on the board), `admitted`,
  `declined`, `unadmitted`. Each names the board, the submission, the
  decider (absent for `submitted`), and the reason where one exists.
- **Delivery is #5862's.** Who receives an event is decided by that
  record's `notify_mode` on subscriptions (`off` by default, `digest`,
  `immediate`) and its mute rule. This record does not create a second
  notification channel, a second mute, or a second default. A board's
  followers who opted in hear about admissions; a board's followers who
  did not, do not.
- **Three audiences, three rules:**
  - **The submitter** is always notified of the decision on their own
    submission (`admitted`, `declined`, `unadmitted`) with the reason,
    regardless of any subscription — this is a direct answer to an act
    they took, not a feed item. It is silenced only by the submitter's
    own per-board intake mute (§ below).
  - **Board followers** hear about `admitted` and `unadmitted` under
    their `notify_mode`. They do **not** hear about `submitted` or
    `declined`: a pending item is not board content, and a declined item
    is the submitter's business, not the followers'.
  - **The deciders** (owner and active `Intake` holders) hear about
    `submitted`, so a queue can be worked. This is the one audience that
    is derived from a role rather than from a subscription, and it is
    opt-outable (§ below).
- **Muting:** a viewer mutes intake notifications per board with one
  state, `intake_notify: on | off`, which suppresses the role-derived
  `submitted` stream and the submitter's decision notices for that board.
  Muting is the viewer's own act, so it can suppress their own decision
  notices too — but a decision on a submission is always recorded on the
  submission itself and visible to its author in their submission list,
  so a mute never hides the outcome, only the ping. Muting intake does
  not mute the board's feed (#5862's mute is separate and unchanged).
- **No notification carries content the recipient may not read.** A
  `submitted` notice names the board and the submitter's key; it does not
  carry the post body, the target's title, or the target's metadata to a
  decider who lacks read authority on the target. The decider's decision
  surface applies #5856's render-time authority check like any other
  surface (§6).

## Invariants implementation must hold

1. **Default is open.** A board with no `IntakePolicy` publishes posts as
   today; adding a policy is an owner act, never a side effect.
2. **One queue per board, one mode.** `open | curated` only; no per-forum,
   per-author or per-post exceptions in v1.
3. **Intake is a role, not an account.** Decisions are signed by a key
   holding an active owner-signed `Intake` grant or by the owner; the
   grant is verifiable from the graph alone and revocable on the view.
4. **Submission is fixed at submission.** The approved post is the
   submitted post: approval cannot change body, commentary, pin,
   presentation or widening.
5. **Pending is a projection.** No stored queue, no mutable pending flag;
   the view derives pending/admitted/declined from the signed set, and
   every peer holding the same set derives the same outcome.
6. **Hidden is not erased.** Pending, declined and unadmitted posts stay
   in the graph, stay attributable, and are reported with their layer and
   reason in the composed view.
7. **No authority widening.** An intake decision never grants read
   authority on a target, and no intake surface renders target content to
   a reader who lacks it.
8. **Intake ≠ moderation.** Intake decisions and moderation actions are
   separate objects, separate roles, separate audit streams; neither
   implies the other.
9. **No silent admission.** A mode change admits nothing retroactively;
   bulk admission names each submission.
10. **Notifications are events plus the existing subscription model.** No
    second channel, no second mute, no second default; the submitter's
    decision notice is the one non-subscription delivery, and it is
    opt-outable. A mute never hides an outcome, only its notification.
11. **Parity.** Submitting, deciding and being notified behave identically
    on web, CLI/script and agent surfaces (the parent's graph-parity
    requirement).

## Alternatives considered and rejected

| Option | Why rejected |
|---|---|
| **A single `Moderator` role decides intake** | Bundles "may admit content" with "may hide published content" and their different audit consequences. An owner handing out queue duty would silently hand out moderation power, and #5861 owns moderation roles. |
| **Pending state as a field on `Post` (`pending: bool`)** | Makes pending mutable state on an immutable signed object (an edit would be needed to admit), breaks the "approval cannot rewrite the submission" property, and lets two peers disagree on a flag instead of deriving from decisions. |
| **Reuse `ModerationAction::Hide` for pending and decline** | A hide is a public moderation statement about published content with an appeals path; a pending item was never published. Conflating them makes the moderation log claim actions that never happened and gives intake grants moderation power by the back door. |
| **A stored queue object per board** | A mutable list is a second source of truth that can diverge from the signed set; the graph already has everything needed to derive the queue. |
| **Per-forum or per-author intake modes** | Multiplies the state space and the notification matrix before any board needs it; a board can get the same effect with a curated board and forums that carry no posts directly. |
| **"Trusted posters skip the queue"** | An implicit role with no attestation; it would need its own grants, its own revocation and its own audit. If the owner trusts a poster, the answer is to let them post to an open board. |
| **Approval rewrites the post (approve-with-edits)** | Turns the decider into an author of someone else's post, breaking attribution and #5856's derived-attribution rule. A decider who wants changes declines with a reason; the author resubmits. |
| **Notification system owned by intake** | A second channel with a second mute and a second default, which #5862 exists to prevent. |
| **Notify followers of every submission** | Turns every pending item into board content for readers who cannot see it, and leaks the existence and cadence of unadmitted material. |
| **Silent backlog admission on `curated → open`** | Publishes content nobody decided on, under a policy change that looked like an administrative switch. |

## Parent-decision check

Nothing here changes a decision in parent #5680. It answers the parent's
§1 bullet *"Inbox/requests model for curated boards: submit-to-board with
owner/moderator approval queue vs. open boards where anyone with access
posts immediately"* — answered as: **both modes exist, `open` is the
default, `curated` routes through a derived queue decided by an
owner-granted `Intake` role, and the approved post is the submitted post**
— and the parent's §1 bullet *"Notifications: board owner, board
subscribers, and object owner each notified of what, and how it can be
muted"* — answered in Decision 5, with delivery delegated to #5862's
`notify_mode` and mute.

It applies rather than forks the adjacent decided designs: #5856's
post-is-a-pinned-reference and two-authority-gates model (this record
adds a third gate — board intake — on top of read authority and posting
rights, without altering either), #5862's subscription/`notify_mode`/mute
model, #2054's append-only/supersede/three-layer-view mechanics, and
#2048's stable-note-identity model.

## Implementation notes and hazards (for the follow-on)

Recorded now because they shape the slice, not because this record
implements them:

1. **`BoardObject` is a closed enum with canonical re-serialization.**
   `BoardObject::from_artifact` re-serializes the decoded body and
   compares it to the stored body, so `IntakePolicy` and `IntakeDecision`
   are new enum variants with new kind names, not new fields on `Board`
   or `Post`. Old signed sets keep decoding; new variants are simply
   unknown to an old peer, which must reject rather than half-project
   them (the strict-decode path already does this).
2. **`BoardRole` is a closed enum today.** Adding `Intake` widens
   `RoleAttestation`'s accepted vocabulary; `has_role` already answers
   "does a grant of this role exist" in any arrival order, and
   `compose_view` already enforces the current-frontier rule for
   moderators. Intake decisions need the same frontier check at the view
   layer — a decision signed by a key whose grant is superseded by a
   revocation must not admit anything, while staying in the audit stream.
3. **The board view must learn a fourth exclusion reason.** `BoardView`
   reports `hidden` with a `ViewLayer` (`personal`/`board`/`host`). Pending
   and declined items are excluded by the board's own intake policy, so
   they belong to the board layer with the `IntakeDecision` (or its
   absence) named as the record — not a new layer, and never silently
   omitted.
4. **Ordering rule for competing decisions.** The "earliest decision
   wins, by content-hash order within a cited frontier" rule must be
   implemented exactly as `resolve_rules` implements duplicate
   `(board, revision)` objects, so two peers that received the decisions
   in opposite order still project the same outcome.
5. **Property tests** belong with the slice: a curated board never
   publishes an undecided post; a decision cannot change the submitted
   post; revocation removes the power but not the record; a mode switch
   admits nothing; the same signed set projects the same queue in any
   arrival order; and no intake surface renders target content without
   read authority.

## Follow-on

Implementation is a separate decision (per the parent, possibly months
out). When scheduled it splits as:

1. `IntakePolicy` + `BoardRole::Intake` — the policy object, the role
   grant/revocation path, and the frontier check for decisions.
2. `IntakeDecision` + derived queue — admit/decline/unadmit objects, the
   pending/admitted/declined projection in `compose_view`, the audit
   surface, and the deterministic competing-decision order.
3. Intake events and notification wiring — `submitted`/`admitted`/
   `declined`/`unadmitted` events delivered through #5862's
   `notify_mode` and mute, plus the per-board `intake_notify` state.
4. Submission surfaces — the submit-to-curated-board flow on web, CLI and
   agent surfaces, with #5856's authority checks applied at submission
   and at decision.

Each split is a separate issue when implementation is scheduled; none of
them may re-decide Decisions 1–5 without amending this record explicitly.

## Open questions for Drew

These do not block recording the design; the recommendations above are
the implementation defaults until ruled otherwise.

1. **Intake vs moderator role:** (a) separate `Intake` role as
   recommended, (b) `Moderator` implies intake, or (c) a single combined
   `Curator` role. **Recommend (a)**; the audit consequences differ.
2. **Submitter notification floor:** (a) always notify the submitter of
   their decision, opt-outable as recommended, (b) notify only under
   `notify_mode`, or (c) always, not opt-outable. **Recommend (a)**.
3. **Declined submissions:** (a) stay in the graph, visible to the author
   and the audit surface as recommended, (b) tombstoned on decline, or
   (c) hidden from everyone but the deciders. **Recommend (a)**; the
   board holds no power over the submission's target.
4. **Queue visibility to followers:** (a) followers never see pending
   items as recommended, (b) followers see an aggregate "N pending"
   count. **Recommend (a)** for v1; a count is a small later addition if
   a board asks for it.
