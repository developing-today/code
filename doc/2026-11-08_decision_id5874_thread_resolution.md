# Decision: thread resolution and the discussion→edit loop (resolved-into links and change proposals)

- **Date:** 2026-11-08
- **Status:** decision record — no implementation is included in this change.
- **Issue:** [#5874](https://github.com/developing-today/code/issues/5874),
  "id: design: thread resolution & the discussion→edit loop (from #5682)".
  The issue's closing rule: "Close this design with a recorded decision
  (on this issue or in a `doc/` record linked from it): the chosen shape,
  the alternatives rejected and why, and the invariants implementation
  must hold." This record is that decision.
- **Parent design:** [#5682](https://github.com/developing-today/code/issues/5682)
  ("Chat next steps — conversation↔object linking, talk pages, terminal
  participation"). The parent's decisions are **not re-decided here**:
  anchoring is a first-class link (§1 of the parent, including the
  *resolved-into* link type this record gives mechanics to); a talk page
  *is* a chat conversation on the #2036 protocol with a permanent object
  anchor, not a second messaging system (§2); thread visibility never
  leaks an anchored object — or its existence — to non-readers (§1);
  full participation, including resolving threads, is required from a
  terminal (§3). §8 states the one refinement this record proposes, and
  it is additive.
- **Inputs cited:**
  - [#5870](https://github.com/developing-today/code/issues/5870)
    (chat anchor data model, sibling, still open) — owns the anchor
    representation and anchor-survival rules. This record consumes
    anchors; it does not define them (§9).
  - [#5871](https://github.com/developing-today/code/issues/5871)
    (talk-page semantics, sibling, still open) — owns the talk-page
    surface and lifecycle. Resolution markers appear on that surface;
    their semantics are defined here, their presentation there (§9).
  - [#2051](https://github.com/developing-today/code/issues/2051) /
    #2036 — chat v1 and the messaging/relay protocol this design stays
    inside: resolution and proposals are events on the same protocol,
    never a side channel.
  - In-tree evidence at the baseline below:
    - `pkgs/id/src/chat.rs` — the v1 chat message shape
      (`chat.message.v1`), its causal projection (`project`, with the
      header backlink as a causal parent), and `ChatLinkKind`, which in
      v1 is exactly `{Blob, Event}`: a message links content by hash and
      author; it does not carry workflow state.
    - `pkgs/id/src/note_links.rs` — the established pattern this record
      reuses: links as signed events (`note.link`), a derived
      projection (never separately mutable state), links resolved by
      stable ID and pinned revision, never by name, and access filtering
      as a query-time view that drops an edge whole rather than leaking
      an inaccessible target's existence.
    - `git grep resolved_into|resolved-into|ResolvedInto -- pkgs/`
      returns nothing: no resolution or proposal mechanics exist in the
      tree yet; this record is the first design for them.
- **Evidence baseline:** `origin/main` at
  `10da0c4ed2b59e53950f6dc47af35ea76f05e8ee`, worktree
  `~/git/fleet/task-13637`, 2026-11-08. Queue history: task 10276 and
  task 12360 triaged #5874 as real and current with the design
  unwritten; task 13637 is the fix-family follow-on those triages
  created, and this record is its deliverable.

## Decision summary

| Question | Decision |
| --- | --- |
| What "resolved" is | A signed event, `chat.resolution.v1`, folded by projection — never a mutable field on the thread (§2) |
| What a resolution names | The thread root, the resolver, a resolution kind (`edit` / `decision` / `no-action`), and — for `edit` — the object and the exact edit event/revision it produced (§2) |
| Navigation | Both ways, from the same derived edge set: thread → outcome edit; object revision → the resolution (and thread) that produced it (§2) |
| What a change proposal is | A signed event, `chat.proposal.v1`, carrying the proposed content by blob link, anchored to (object, base revision) (§3) |
| Proposal lifecycle | Derived by folding later events: `proposed` → `accepted` / `rejected` / `superseded` / `withdrawn`; acceptance is not a state write — the accepting edit event itself, citing the proposal, is the acceptance (§3) |
| Who may resolve / accept | Resolve: a thread participant or an object writer. Accept: only an author with write authority on the object, because acceptance *is* an edit (§4) |
| Audit | The signed event chain itself: edit cites proposal, proposal cites thread, thread is anchored to the object; verifiable by any reader (§4) |
| Visibility | The parent's rule, applied with the `note_links` mechanism: a resolution/proposal edge that would expose an object a viewer cannot read is dropped whole (§4) |
| v1 message shape | Unchanged: no new `ChatLinkKind`; resolution and proposal are their own event kinds (§7) |

## 1. Scope

This record settles exactly what #5874 asks: **resolved-into links and
change proposals from threads — how a discussion becomes an edit with
provenance.** It defines the two event kinds, their projections, the
proposal lifecycle, and the authorization and audit rules. It does not
define anchors (#5870), talk-page presentation (#5871), or moderation
(parent §2 points at #835), and it implements nothing: implementation
is split in §8 and its timing is a separate prioritisation decision,
as the issue itself notes.

## 2. The resolved-into relation: `chat.resolution.v1`

A thread is **resolved** when a `chat.resolution.v1` event exists for
its root. The event is signed by its author (the resolver) and names:

- `thread`: the root event ID of the thread being resolved;
- `kind`: `edit` (the discussion produced an edit on an object),
  `decision` (the discussion settled a question, recorded in the
  resolution's summary), or `no-action` (considered; nothing to
  change);
- for `edit`: `object` (the object's stable ID) and `edit` (the edit
  event ID / resulting revision the discussion produced);
- for `decision`: a `summary` — the decision text, carried as a blob
  link under the chat v1 linking rules (content by hash and author);
- the resolver's causal parents (`after` / backlink) exactly as chat
  messages carry them, so resolutions order inside the same causal
  projection as the discussion they close.

**Projection, not state.** Thread resolution state is derived by
folding resolution events, following the `note_links` rule that the
projection is derived, never stored: there is no mutable "resolved"
flag on the thread root that replicas could disagree about. A later
resolution event for the same thread **re-opens or re-resolves** it by
superseding the earlier one in the fold (the fold keeps the causally
latest resolution per thread; concurrent resolutions from two heads
both survive in the projection and are presented side by side, again
as `note_links` presents concurrent heads — no head is silently
chosen over another).

**Both directions from one edge set.** The resolved-into edge set is
the fold of `chat.resolution.v1` events with kind `edit`. From a
thread, its outcome edit; from an object revision, the resolution —
and through it the thread — that produced it. There is no second,
separately maintained backlink structure that could disagree with the
edges, for the same reason `note_links` derives backlinks from the
same edge set.

## 3. Change proposals: `chat.proposal.v1` and its lifecycle

A **change proposal** is how a discussion participant who may not have
write authority on an object nevertheless produces a concrete,
reviewable edit. A `chat.proposal.v1` event, signed by the proposer,
names:

- `object`: the object's stable ID;
- `base`: the exact revision of the object the proposal is against
  (pinned, never "latest" — the `note_links` pin rule);
- `content`: the proposed content, by blob link (the full proposed
  revision content, not a diff: blobs are content-addressed, a diff
  against a moving base is not verifiable);
- `thread`: the discussion thread the proposal came out of (which may
  be the object's talk page or any anchored thread).

**Lifecycle is a fold, not a status field.** A proposal's state is
derived from the events that cite it:

- `proposed` — the proposal event exists and nothing below has
  happened;
- `accepted` — an edit event on the object cites the proposal as its
  provenance (§4). Acceptance is the edit itself; there is no separate
  approval write to forge or lose;
- `rejected` — a `chat.proposal-rejection.v1` event by an object
  writer cites the proposal and a reason (blob-linked summary);
- `superseded` — a second proposal by the same proposer against the
  same (object, base) cites the first;
- `withdrawn` — a withdrawal event by the proposer cites it.

Re-proposing against a newer base is a new proposal, not a mutation
of the old one. A proposal whose base is no longer a head is **stale**;
staleness is a projection-time observation, not a state — a writer may
still accept it by making the edit, and the edit's own base is what
the object projection folds.

## 4. Authorization and audit

- **Resolve authority:** a thread participant or a writer on any
  object the thread is anchored to may resolve the thread. Resolution
  is a statement about the discussion, not about the object.
- **Accept authority:** only an author with write authority on the
  object can accept a proposal — because acceptance is an ordinary
  signed edit event under that author's key, and the object's own
  admission rules are the only gate. This design adds no second
  permission system.
- **Provenance chain (the audit trail):** an edit that accepts a
  proposal cites the proposal event ID; the proposal cites its thread
  and its pinned base; the thread is anchored to the object under the
  parent's anchor model. Any reader with access can walk
  edit → proposal → thread and verify each hop is signed by the key it
  claims. An edit that does not cite a proposal simply has no proposal
  provenance; provenance cannot be asserted after the fact, because
  the citation lives inside the signed edit event.
- **Visibility:** the parent's rule — a conversation about an object
  must never leak the object, or its existence, to non-readers —
  applies to resolution and proposal edges unchanged, implemented with
  the `note_links` mechanism: access filtering is a query-time view,
  and an edge that would expose an inaccessible object is dropped
  whole (its existence is object metadata).

## 5. Invariants implementation must hold

1. **Resolution and proposal state are folds.** No stored mutable
   status on threads, proposals, or objects for anything this record
   defines; every state a client shows is re-derivable from the event
   set alone.
2. **Provenance is citation inside signed events.** An edit's claim to
   implement a proposal exists only as the proposal's event ID inside
   the signed edit; ditto a proposal's claim to come from a thread.
3. **IDs and pins, never names or "latest".** Objects, threads,
   proposals, and bases are referenced by stable ID and pinned
   revision, per the `note_links` rule — renaming or editing never
   moves a reference.
4. **Deterministic projection.** Folds return the same result for the
   same event set in any arrival order (sorted outputs, causal order
   from `after` + backlink), matching the chat projection's contract.
5. **Terminal parity.** Proposing, resolving, rejecting, and accepting
   are each a single signed event a terminal client can author and
   read back as text, per the parent's §3 requirement; no step in the
   loop requires a GUI.
6. **No leak by edge.** A projection served to a viewer never reveals
   an object, thread, or proposal the viewer cannot read — including
   by the presence or absence of an edge (§4).

## 6. Alternatives rejected

- **A mutable `resolved` flag / status field on the thread root.**
  The thread root's author would be the only writer, resolutions by
  anyone else need a second write path anyway, and concurrent heads
  make the flag fork. The fleet's own link machinery (`note_links`)
  already settled this shape: signed events plus a derived projection.
- **Recording resolution only inside the edit event** (the edit says
  "this came from thread T"). Loses the thread → outcome direction,
  cannot represent `decision` or `no-action` resolutions (which
  produce no edit), and lets any edit claim any thread with no
  resolver accountable for the claim.
- **A new `ChatLinkKind` for resolutions inside chat messages.**
  Freezes workflow state into the v1 message shape, whose links are
  deliberately content references (`blob` / `event`) and nothing else;
  every message client would have to understand resolution semantics
  to render a conversation. Separate event kinds keep v1 frozen (§7).
- **An off-protocol tracker** (issue-tracker-style status kept outside
  the event log). Breaks the parent's rule that a talk page is a chat
  conversation on the one protocol, and makes the audit chain
  unverifiable from the log — the exact failure mode (queue state
  claiming work that has no landed evidence) that #5874's own queue
  history illustrates.
- **Diffs instead of full content in proposals.** A diff is only
  meaningful against its base; with pinned full content the proposal
  is verifiable forever, and storage dedupes by content hash anyway.

## 7. Relation to existing machinery

- `chat.rs`: unchanged. `chat.resolution.v1` and `chat.proposal.v1`
  are event kinds beside `chat.message.v1`, consumed by their own
  projections; the chat projection (`project`) is untouched, and
  resolutions participate in causal ordering through the same
  `after` / backlink parents.
- `note_links.rs`: the pattern, reused — signed link events, derived
  projection, ID + pin references, query-time access filtering. Where
  an object's own model already has link events (notes do), the
  resolution fold should share the projection plumbing rather than
  growing a parallel one.
- Anchors: consumed from #5870's model when it lands; until then,
  `object` + `thread` references in these events are stable IDs, which
  is all this design requires of anchors.

## 8. Implementation split (successors, when prioritised)

Filed as one implementation issue linked from #5874, to be split by
whoever picks it up:

1. Event kinds and folds in `pkgs/id`: `chat.resolution.v1`,
   `chat.proposal.v1`, `chat.proposal-rejection.v1`, withdrawal;
   resolution and proposal projections with property tests on the §5
   invariants (fold determinism, no-leak filtering, concurrent-head
   behaviour).
2. Acceptance plumbing in object editors: citing a proposal from an
   edit, and the stale-base presentation.
3. Client surfaces: talk-page resolution markers and proposal review
   in the GUI, and the terminal flows (propose / resolve / accept)
   with textual rendering per the parent's §3.

Timing is a separate decision (the issue: "implementation timing is a
separate decision, possibly months out"); this record closes the
design question only.

## 9. What this record does not decide

- The anchor data model and anchor survival under edit/delete —
  #5870.
- Talk-page lifecycle (lazy vs eager creation), pinning, slow modes,
  notification levels — #5871 and the parent.
- Moderation of threads and proposals — the parent's pointer to #835
  stands; nothing here grants or limits moderation power.
- Whether `decision`-kind resolutions need a heavier ratification
  shape for organisation objects (escrow/step-up territory, #5881's
  design) — left to that design; the `decision` kind here is the
  minimal signed record.
