# Edit/fork graph for notes — decision record

Decision record for issue #5833 ("id: design: edit/fork graph (from
#5677)"). Written per `doc/DOCUMENTATION_PROTOCOL.md` (dated record in
`doc/<UTC>_design_<name>/`). Origin issue: #5833, a decomposition child
of #5677 (notes, boards, multiplayer editing v1; decomposition item 4).
Adjacent contracts: #5830 (note→board sharing — the note↔board reference
model), #5831 (native graph model & navigation API), #5832 (multiplayer
edit sessions — the session merge model), #5834 (per-object talk page +
changelog), #5835 (draft → approval flow), #5836 (personalized search
ranking — consumes this record's marks), and the landed revision
substrate of #3814 (`pkgs/id/src/notes.rs`). Programme: epic #2056.
Status: v1 decision, settled except §10. No implementation is
authorized by this record alone; §9 states what implementation must
wait for and how it splits.

Verified at origin/main `aa01b61853` (2026-11-09): the signed revision
DAG this record builds on exists — a signed `note.create` event
establishes a stable note ID, each `note.revise` event is an immutable
signed revision naming its causal parent revision IDs, concurrent
revisions are all heads until an explicit merge revision names them,
and the fold is deterministic and independent of event arrival order
(`pkgs/id/src/notes.rs`). No edit/fork graph, divergence view,
mark-most-correct mechanism, or merge-proposal record exists yet:
`git grep` for "mark-most-correct" over `doc/` and `pkgs/id/src`
returns nothing, and #5677 §4's four requirements are unimplemented
above the revision layer.

---

## 1. Problem

#5677 §4 requires every object to have an edit/fork graph: who edited
what, which versions diverged from which, and how forks relate — with a
divergence view, a mark-most-correct mechanism, and merge proposals, and
with forks as first-class graph citizens. §7 (sovereignty, fixed by the
parent) sets the ground rules: anyone can fork or edit any object they
can see, every edit is attributed, effective authority is social, and
nothing is ever deleted. The revision substrate (#3814) already gives
the raw material — a deterministic DAG of signed revisions with stable
object identity — but nothing above it: revisions have no lineage
structure, no view that shows divergence, no mark semantics (the
ranking record #5836 consumes most-correct marks but explicitly defers
the marking mechanism to this record, its §7), and no recorded way for
a fork to propose merging back. This record settles those four things —
divergence views, mark-most-correct, merge proposals, fork citizenship —
without re-deciding anything the parent or the sibling designs fixed.

## 2. Decisions in one paragraph

The edit/fork graph is a **projection over the existing signed revision
DAG**, not a new store: the object is the stable note ID, revisions are
the nodes, and a **lineage** — a maximal unbranched path identified by
its first revision — is the unit that gets viewed, marked, and merged.
A fork is not a new object and gets no new identity; it is what the DAG
already records when a revision branches, and it carries the same
rights as the original line (referenceable, markable, mergeable,
rankable). **Divergence views** are deterministic projections that
compare lineages by ancestry set-difference against their shared
ancestry, disclose every lineage, and hide none. **Mark-most-correct**
is an attributed, scoped, superseding assertion recorded in the
object's changelog: one current mark per (object, community scope),
set by the object's owner or a scope admin, following its lineage as
the lineage extends, always displayed with its scope — there is no
global most-correct, and marks never alter the graph. **Merge
proposals** are recorded, attributed coordination objects with a
lifecycle (open → merged / rejected / withdrawn / superseded); a merge
itself is still exactly what the substrate says — an explicit signed
revision naming the heads it joins — and rejection never deletes the
rejected line.

## 3. The graph: objects, revisions, lineages

1. **Object.** One edit/fork graph per note, keyed by the stable note
   ID (the `note.create` event ID). Editing, forking, and merging never
   change the object's identity and never create a second object. Forks
   as new objects is rejected (§7): it would split backlinks, marks,
   and the changelog across identities for what is one object's
   history.
2. **Revision.** The substrate unchanged: immutable, signed, naming its
   causal parents in ID order. Every graph fact in this record is
   derived from revisions and from the attributed graph actions (§5,
   §6), which are themselves changelog-recorded events (#5834 owns the
   changelog's data model).
3. **Lineage.** A lineage is a maximal unbranched path in the DAG,
   identified by the ID of its first revision. First revisions are
   exactly: the create revision; any revision whose parent has more
   than one child (each branch child starts a lineage); and any merge
   revision (a revision with two or more parents starts a new lineage,
   and the lineages it joins end at it). Every revision belongs to
   exactly one lineage, so lineages partition the graph, and the
   partition is a pure function of the DAG — deterministic and
   arrival-order independent, like the fold beneath it. A lineage's
   _tip_ is its last revision; a lineage is _live_ while its tip is a
   head.
4. **Fork.** Forking is not an operation on identity; it is the
   observable event that a revision branches — either a second child of
   an existing revision, or simply a new head appearing beside others.
   The fork's author is the branching revision's signer (attribution is
   the substrate's, never anonymous). Under sovereignty (#5677 §7) any
   actor who can see a revision may branch from it; no permission,
   registration, or notification duty gates the act itself.
5. **Edit vs fork is presentation, not state.** A single-child
   extension of a head and a branch are the same kind of event at
   different shapes; the graph records shape, and views (§4) may label
   a branch a "fork" and a head extension an "edit". No flag on a
   revision declares intent, and consumers must not require one.

## 4. Divergence views

A divergence view is a read-only, deterministic projection over one
object's graph, with identical semantics on CLI, API, scripting, and
agent surfaces (the traversal operations are #5831's; this record owns
the shape of this projection, not the query layer).

1. **Contents.** For the object: the lineage tree (fork points, merge
   points, tips, authors, time spans); the set of live lineages; and,
   for any selected pair or set of lineages, the comparison in rule 2.
   The default selection is all live lineages.
2. **Comparison is ancestry set-difference.** For lineages A and B:
   A's unique part is the revisions reachable from A's tip and not
   from B's tip, and symmetrically for B; the shared part is the
   revisions reachable from both. This needs no single merge base and
   stays well-defined under criss-cross merges, where maximal common
   ancestors are a set. Each side's unique part is rendered as its
   revision list (author, timestamp, per-revision content diff against
   its parent — content references are verified by the substrate) plus
   a summary (revision count, authors, marks (§5), proposal state
   (§6)). A tip-to-tip content diff may be offered as a convenience;
   the revision list is the contract.
3. **Nothing hidden.** A view discloses every lineage of the object,
   including lineages a scope has marked graffiti (ranking demotion,
   #5836 §5, operates on search order, never on this view) and
   lineages whose proposals were rejected. A view may collapse
   lineages for display but must show their count and expand on
   request — the same reachability rule the ranking record applies to
   demoted results.
4. **Default lineage for display** (a view default, never a gate):
   the viewing actor's own lineage first; else the marked lineage of
   a scope the viewer belongs to (§5); else the deterministic first
   live lineage in the fold's ID order. Where the approval flow
   (#5835) is enabled for the object, its effective version outranks
   this default for _effective-version_ display — approval decides
   effect, this rule only decides what an ungoverned view opens on.

## 5. Mark-most-correct

The mechanism the ranking record (#5836 §7) defers to this record. A
mark is a signal layered on the graph (#5677 §4): it changes no
revision, no lineage, and no other scope's view.

1. **A mark is an attributed, scoped assertion**, recorded as an event
   in the object's changelog (#5834): (object, lineage, tip revision
   at mark time, scope, marker's key, timestamp, superseded mark if
   any). Anonymous or unattributed marks do not exist — the same rule
   the ranking record sets for graffiti marks, which are a second mark
   type under this same model (their demotion mechanics are #5836 §5's;
   this record defines no demotion).
2. **Scopes.** A scope is either _personal_ (the marker's own key —
   the mark exists only in their view) or a _community scope_ named by
   a board or collection whose membership defines it. There is no
   server-wide, network-wide, or global scope.
3. **Who may mark.** In a personal scope: the marker, over any lineage
   they can see. In a community scope: the object's owner of record
   (in v1, the create revision's author) or an admin of the
   scope-defining board/collection. Community-wide voting is rejected
   for v1 (§7). Attribution and the supersession history are the
   accountability mechanism: a scope that disagrees contests a mark on
   the object's talk page (#5834), not by editing mark state.
4. **One current mark per (object, scope).** Marking a different
   lineage in the same scope supersedes that scope's previous mark;
   the old mark stays in the changelog. Intra-scope conflict is
   therefore impossible by construction, and cross-scope disagreement
   is not conflict but coexistence: two scopes may mark different
   lineages of the same object at the same time, and both are correct
   _for their scope_.
5. **Marks follow the lineage, not the pinned tip.** A mark names a
   lineage; as the lineage extends, the mark's current referent is the
   lineage's current tip. The tip at mark time is kept in the record,
   so a view can show "marked at revision X, lineage now at Y" —
   staleness is displayed, never silently hidden. Pinning marks to one
   revision is rejected (§7).
6. **Display is always scope-qualified.** No surface may render a bare
   "most correct" without its scope ("most correct — board B"), and a
   viewer in several scopes sees each scope's mark labelled. This
   answers the parent's open question: marks _can_ point different
   ways across communities, and the display rule is that they are
   never merged into one verdict.
7. **Consumer mapping (for #5836).** Ranking tier 2 reads: the current
   tip of the lineage a searcher's scope currently marks counts as
   that scope's marked version. Mark semantics impose **no precedence**
   between mark types or scopes; where a consumer combines signals
   (e.g. a lineage both marked in one of the searcher's scopes and
   graffiti-demoted in another), the consumer states its precedence —
   the ranking record's §9 default (demotion wins) therefore stands as
   that consumer's rule and needs no amendment from this record.

## 6. Merge proposals

Merging at the graph level is the substrate's explicit merge revision.
A proposal is the coordination record around producing one: the
parent (§5677 §4) requires the graph to record "proposal, discussion
(on the talk page), and outcome" — this section is that record.

1. **A proposal is an attributed graph object**: (object, source
   lineage and its tip, target lineage and its tip, proposer,
   timestamp, talk-page anchor, state). It is recorded in the object's
   changelog (#5834) and discussed on the object's talk page; the
   proposal carries no discussion content of its own.
2. **States and transitions.** `open` → `merged` (rule 3), `rejected`
   (rule 4), `withdrawn` (by the proposer), or `superseded` (rule 5).
   Terminal states are final; a changed intent is a new proposal, not
   a reopened one.
3. **Merged.** A proposal closes as merged when a merge revision
   exists whose parents include the source tip or a descendant of it
   within the source lineage, and the target tip or a descendant of
   it within the target lineage. The closing records the merge
   revision's ID. Who authors the merge revision is not restricted
   beyond visibility (sovereignty): typically the proposer or a
   maintainer, and the revision's signer is its attribution.
4. **Rejected.** Rejection is an explicit recorded decision by the
   object's owner of record or an admin of the scope the proposal
   asked adoption into, with the talk-page discussion as its record.
   Rejection deletes nothing: the source lineage remains a first-class
   line — the parent's rejected-draft rule (#5677 §5) applied to
   forks. Where the approval flow (#5835) governs the object, a
   proposal's decision goes through that flow instead (boundary, §8).
5. **Superseded.** A proposal names tips; when either lineage's tip
   has advanced past the named tip and the proposal is not merged,
   the proposal is stale and may be closed as superseded by anyone
   (the check is mechanical against the DAG). The proposer re-proposes
   against current tips. Stale proposals never auto-merge.
6. **Proposals are coordination, not a gate.** A merge revision may
   exist without any proposal (a direct merge by an actor who can see
   both lineages); it is fully attributed in the changelog like any
   other. Proposals become the _required_ path only where governance
   requires them — an approval-enabled object/collection per #5835 —
   never by this record's default.
7. **No silent content merging.** A merge revision's content is
   authored content (a human or agent resolves the two lines, possibly
   with tooling help). Session-level conflict-free merging inside a
   live edit session is #5832's merge model and is an input here, not
   restated; nothing in this record authorizes a system-generated
   merge of diverged lineages without an author signing the resulting
   revision — the substrate's explicit-merge rule, restated as a
   graph invariant (see Invariants).

## 7. Alternatives rejected

- **Fork as a new object with its own identity.** Rejected in §3:
  splits identity, backlinks (#5830's reference model names objects),
  marks, and changelog across what is one object's history, and makes
  "merge back" a cross-object operation instead of a revision.
- **One canonical lineage with forks as second-class copies.**
  Rejected by sovereignty (#5677 §7) outright: authority is social and
  per-community, and #5677's non-goals exclude a global canonical
  truth. The graph gives every lineage the same operations.
- **Marks pinned to a single revision.** Rejected in §5: a living
  lineage extends past its pinned tip immediately, so pinned marks are
  permanently stale and ranking's tier 2 would reward frozen versions
  over maintained ones.
- **Community voting for most-correct in v1.** Rejected: a vote needs
  a weighting/identity design (Sybil resistance, quorum, scope
  membership at vote time) that no sibling record owns yet; owner/
  admin marks with full attribution and talk-page contestation deliver
  the signal now, and a vote, if ever wanted, arrives as a new mark
  source by dated amendment, not a reinterpretation of this one.
- **Merge proposals as a mandatory gate on all merges.** Rejected in
  §6: it would contradict sovereignty for ungoverned objects and
  duplicate the approval flow's opt-in governance (#5677 §5) as a
  global gate, which the parent explicitly rejects.
- **Auto-merge of diverged lineages by the system.** Rejected in §6:
  the substrate requires merges to be explicit signed revisions; a
  silent system merge would break attribution — the one thing the
  sovereignty rule never trades away.
- **A separate fork registry / graph store beside the revision DAG.**
  Rejected: the DAG plus changelog events already contain every fact;
  a second store would need its own consistency story against the
  deterministic fold for zero new information.

## 8. Boundaries with the adjacent systems

- **#5677 (parent).** This record re-decides nothing the parent fixed:
  the note↔board reference model, the session merge model, the
  sovereignty rule, and the approval flow's shape are inputs, cited
  not restated. It answers the parent's mark-most-correct open
  question (who may mark: §5 rule 3; cross-community marks: §5 rules
  4 and 6) within item 4's remit.
- **#5830 (note→board sharing, decided).** References name objects and
  optionally revisions; under this record they may also name a lineage
  (by its first revision). A reference to the object resolves against
  the viewer's default lineage (§4 rule 4); #5830's model is otherwise
  unchanged and wins any disagreement about referencing.
- **#5831 (graph model & navigation API, decided).** Traversals,
  pagination, and cross-surface parity are #5831's. This record's
  lineage partition and view are projections expressed over those
  operations (history-for-object, neighbors); if a projection cannot
  be expressed, the gap is filed against #5831, not worked around with
  a private query path.
- **#5832 (multiplayer edit sessions, open).** The CRDT-class session
  merge model is #5832's, including its limits. Sessions operate
  _within_ a lineage's head; a session never silently creates a fork —
  branching is a revision-level event (§3), however the session's
  output is committed.
- **#5834 (talk page + changelog, open).** The changelog's data model
  and the talk page are #5834's. This record specifies only which
  events must be changelog-recorded (marks and their supersessions,
  proposals and their outcomes, forks, merges — most already implied
  by #5677 §4's "every edit, fork, mark, and approval") and that
  proposal discussion lives on the talk page.
- **#5835 (approval flow, open).** Where approval governance is
  enabled, it decides the object's _effective version_, and proposal
  decisions route through it (§6 rule 4). Marks remain signals and
  never override an approval outcome; #5835 must confirm this
  composition when it settles (§10).
- **#5836 (search ranking, decided).** Ranking consumes marks per §5
  rule 7 and consumes lineage/provenance for its tiers 0 and 3 (§8 of
  that record names this design as its hard prerequisite). This record
  consumes nothing from ranking; graffiti demotion never feeds back
  into graph visibility (§4 rule 3).
- **Revision substrate (#3814, landed).** Fork/merge _representation_
  is the substrate's signed events; this record adds no wire types.
  Deletion/erasure stays out of scope exactly as the substrate leaves
  it — the takedown design the parent names must still be reconciled
  with the append-only changelog explicitly, elsewhere.

## 9. Dependencies and implementation slices

**Hard prerequisites (implementation must wait):**

- **#5834** — marks and proposals are changelog events; the changelog
  data model must exist before either is recorded.
- **#5831** — decided; the lineage projection is built from its
  traversal operations. The substrate itself (#3814) has landed.

**Slices (each its own implementation issue, filed when its
prerequisite lands — not before):**

1. Lineage derivation + divergence view (§§3–4), read-only over the
   existing DAG, CLI/API parity per #5831.
2. Marks (§5): personal and community scopes, supersession, changelog
   recording; scope-qualified display.
3. Merge proposals (§6): lifecycle, talk-page anchoring (on #5834),
   mechanical merged/superseded detection against the DAG.
4. Consumer wiring: ranking tier 2 mapping (§5 rule 7) with #5836,
   default-lineage view selection (§4 rule 4).

## 10. Open questions

- **Approval composition.** §8 assigns proposal decisions under
  governance to #5835's flow and keeps marks subordinate to approval
  outcomes; #5835's design must confirm or amend this when it settles.
  Recorded here as the dependency, not decided for it.
- **Ownership transfer.** "Owner of record = create author" is a v1
  simplification; if a later design adds ownership transfer for notes,
  mark and rejection authority follow the role — that design amends
  this record's §5 rule 3 and §6 rule 4 by dated update.
- **Mark delegation.** Whether a scope admin can delegate marking
  (e.g. to maintainers of one collection) is deferred; v1 has exactly
  two mark authorities per community scope (§5 rule 3) so the audit
  story stays trivially readable.
- **Cross-scope view composition.** A viewer in several scopes with
  different marks sees them side by side, labelled (§5 rule 6). A
  combined "boards I trust" aggregate is a _view_ choice a client may
  build from the per-scope facts; standardizing one is deferred until
  a consumer needs it.

## Invariants (summary — implementation must hold all of these)

1. One object, one identity: forks and merges never create, split, or
   rename the object's stable ID.
2. Lineages partition the revision DAG deterministically; the
   partition is a pure function of the DAG, independent of arrival
   order.
3. Every graph action — edit, fork, mark, proposal, merge — is
   attributed to a signing key and recorded; anonymous graph state
   does not exist.
4. No graph operation deletes a revision, a lineage, a mark, or a
   proposal outcome. Supersession and rejection are new records, not
   edits to old ones.
5. Marks are per-scope, at most one current per (object, scope),
   follow their lineage, and are always displayed scope-qualified.
   There is no global most-correct.
6. Divergence views disclose every lineage; ranking demotion and
   proposal rejection never hide a lineage from the graph.
7. A merge is an explicit signed revision naming the heads it joins,
   authored by an identifiable signer; the system never merges
   diverged lineages silently.
8. Where approval governance is enabled it, not marks or view
   defaults, decides the effective version — and nothing in this
   record overrides #5830/#5831/#5835/#5836 inside their boundaries
   (§8).
