# Board view builder — topic/comment tree projection (issue #13096)

- Status: draft
- Date: 2026-11-24T06-00-00Z
- Roadmap item: none (Program #9106, epic #9107)
- Issues: #13096 (this design), parent #9128 (social app reference model, §5 + §9), #9127 (append-list protocol), #9130 (moderation/tombstones), siblings #13094 (token logic), #13095 (envelope)

## 1. Intent and scope

Issue #13096 asks for the board view builder in `pkgs/id`: given a board
object, project its `topics` append-list and each topic's `comments`
append-list into the thread tree view. Readers are board clients rendering
thread trees (topic list, thread view). Today no tree projection exists:
`compose_view` returns a flat object list, and `carried_comments` is a
version-based carry-forward, not a thread tree (see explore notes on
`src/boards.rs`, `crates/id-core/src/graph/`).

Goals: (a) tree shape from `parent_ref`, order from append sequence
(#9127 server order); pending entries never appear; (b) #9130 tombstones
filtered per the view's policy state without renumbering; (c) every
rendered subtree carries its freshness token so folded subtrees re-check
via the nothing-new path (§6 of the #9128 record); (d) pages bounded per
§7 defaults, continued by cursor.

Non-goals: the append-list protocol itself (#9127), the tombstone state
machine (#9130 — consumed, not rebuilt), the token cryptography and
verdict logic (#13094 — consumed as pure logic), the social object
envelope and profile schemas (#13095 — consumed as types), ranking or
"top" projections (#9128 §2.3 — out of scope), client-bound spaces and
petnames (#9131), write paths (submit/moderate flows are siblings).

## 2. Design

### 2.1 Placement

New module `crates/id-core/src/graph/board_view.rs`: `BoardViewBuilder`
plus view types. It is a pure projection over reads: it calls the
append-list read API (accepted entries only — pending is unrepresentable
in the read result by #9127 §4, so "pending never appears" is structural,
not a filter), the tombstone-set revision input from #9130, and the token
issue/check logic from #13094. No new storage, no new ordering, no second
moderation path (record §2.2).

### 2.2 Types

```
BoardThreadView = {
  board,                // board object id
  topics,               // one TopicNode per in-page topic, seq order
  token,                // FreshnessToken for the whole rendered view
  cursor?,              // opaque continuation, FeedCursor shape
}
TopicNode = {
  seq,                  // topics-list accept sequence (stable identity)
  topic,                // SocialObject, profile = topic (#13095)
  children,             // CommentNode forest roots
  token,                // FreshnessToken for this topic's subtree
}
CommentNode = {
  seq,                  // comments-list accept sequence (stable identity)
  comment,              // SocialObject, profile = comment
  children,             // nested replies, append-seq order
  orphaned?,            // true iff parent_ref unresolvable (see §2.4)
  token?,               // present only when the subtree is folded
}
```

`view_id` = hash(board id, projection name + version, policy revision),
per record §6.1. Frontier inputs per view: the topics list
`{head_seq, tombstone_revision}` plus, for each rendered topic, its
comments list `{head_seq, tombstone_revision}`. A folded subtree's node is
replaced by its token alone (record §7); expanding it is the §6.3 check,
not a refetch.

### 2.3 Build algorithm

1. Read the board's `topics` list (accepted only, ascending seq).
2. Page topics first: take the cursor window; only in-page topics incur
   comment reads (fan-out is bounded by page size).
3. For each in-page topic, read its `comments` list (accepted only).
4. Drop tombstoned entries per the view's policy state. Sequences are
   stable server identities, so filtering never renumbers: siblings keep
   their seqs and their relative order. Tombstone seqs stay inside the
   frontier (their lists' head_seq already covers them), so a later
   tombstone still advances the revision and flips `nothing-new` to
   `delta`.
5. Forest assembly per topic: index comments by object id; attach each
   comment under its `parent_ref` (topic id = root, or another comment);
   sort siblings by accept seq. Then §2.4 edge rules.
6. Budget the walk: max display depth 32, max 2,000 visited nodes per
   page, max 8 MiB response (record §7). Traversal stops at the budget
   and continues by bounded opaque cursor over (topic seq, comment seq)
   positions — the `FeedCursor` shape (scope + generation + position),
   reused, not reinvented.
7. Issue tokens: one per rendered topic subtree plus one for the view,
   via #13094 logic. Folded subtrees render as their token only.

### 2.4 Edge rules

- Orphan (parent_ref names a pending, tombstoned, or absent object):
  attach at topic root with `orphaned: true`. Never silently dropped,
  never grafted under a wrong parent.
- Cycle (A parents B parents A — only reachable across adversarial
  submissions): visited set per assembly; the second visit renders a
  back-reference to the already-placed node id instead of recursing
  (record §7).
- Cross-topic parent (comment in topic T names a parent in topic U):
  treated as orphan in T (parents must be same-topic; the Topics list is
  the scope boundary). Flagged, not moved.
- Depth beyond 32: the node at the cap renders folded (token only).

### 2.5 Invariants

Server order is authoritative (#9127 §4); the builder sorts by accept seq
and by nothing else. No per-client server state: poll state is the
client-held token (record §3, feeds). Forged/foreign tokens fail
verification and are treated as no token (full view fetch), per
record §6.5.

## 3. Decisions

1. Page topics before reading comments (§2.3 step 2). Reason: comment
   reads fan out per topic; paging first bounds the fan-out by page
   size. Decided in this record.
2. Orphans attach flagged at topic root, never dropped (§2.4). Reason:
   silent drops hide data loss and break seq continuity for clients
   paging by cursor. Decided in this record.
3. Tombstones filtered with no placeholder in the rendered tree; their
   seqs remain covered by the frontier (§2.3 step 4). Reason: the issue
   requires filtering without renumbering; stable seqs plus revision
   coverage give both. Decided in this record.
4. Cursor reuses the `FeedCursor` opaque shape. Reason: no new cursor
   format for the same (scope, generation, position) triple. Decided in
   this record.
5. Where this record and #9128 §5 disagree, §5 governs and this record
   is the defect (same supremacy rule as the #9128 record §1). Drew
   directive 2026-11-12 governs all product conflicts.

## 4. Alternatives considered

- Client-side tree assembly from flat server lists. Rejected: policy
  enforcement (tombstone authority), token issuance, and paging budgets
  must be server-side; a client cannot mint trustworthy tokens.
- Extending `boards.rs compose_view` in place. Rejected for this slice:
  different hash domains (SHA-256 `ArtifactHash` vs BLAKE3 `ContentHash`)
  and different ordering models (author chains vs central seq); bridging
  them is a separate migration decision, recorded as open question 4.
- Single view-level token only. Rejected: record §7 requires folded
  subtrees to carry their own tokens so expansion is a check, not a
  refetch.
- Do-nothing (no tree projection; clients assemble trees). Rejected:
  #13096 is a filed follow-on of the #9128 record §9; without it every
  board client reimplements parent_ref assembly, ordering, tombstone
  filtering, and token discipline inconsistently.

## 5. Build slices (dependency-ordered, with acceptance)

Entry criteria (built elsewhere, consumed here): #13094 token
issue/check pure logic; #13095 `SocialObject` topic/comment types. Slice
1 starts when both are available.

| Slice                     | Work                                                                                                                                      | Acceptance                                                                                                      |
| ------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| 1. Forest projection core | `board_view.rs`: topics read, per-topic comments read, parent_ref assembly, sibling seq order, pending structurally absent; fixture tests | Fixture append-lists: nested comments render in append order at each level (issue test 1)                       |
| 2. Tombstone filtering    | policy-state filter over tombstone revision; no-renumber property test (seqs stable across filter)                                        | Tombstoned comment hides without disturbing sibling order or seqs (issue test 2)                                |
| 3. Subtree tokens         | per-topic + view token issue; folded-subtree token-only render; §6.3 check wiring via #13094                                              | Folded subtree token checks nothing-new against unchanged lists, delta after a new comment lands (issue test 3) |
| 4. Bounded paging         | depth/node/byte budgets, opaque cursor continuation, back-reference cycles, orphan/cross-topic rules; budget tests                        | 2,001-node fixture pages with cursor; 33-deep fixture folds at 32; cyclic fixture renders back-references       |

Sequencing: 2←1, 3←2 (tokens cover filtered seqs), 4←1 (paging composes
with the forest; token-per-page joins in 4←3 for token-bearing pages).
Slices filed as sub-issues of #13096 by the implementer.

## 6. Rollout and rollback

Additive only: one new module, no stored-format change, no migration.
Land slices in order behind the existing test suite; each slice is
independently revertable (pure projection, no writers depend on it until
board readers adopt it). Rollback is revert of the slice commit. No
flag needed: unadopted code paths are unreachable. Adoption by board
surfaces is a separate slice, not this design.

## 7. Verification plan

Oracle for every test: §5 board semantics in the #9128 record (tree from
parent_ref, order from append sequence, pending absent, tombstone
filtering, token check semantics). Concretely: (a) the three issue
acceptance tests over fixture append-lists (slices 1–3); (b) property
tests per the repo testing doctrine — filtering never renumbers
(seq-stability), sibling order equals accept order, token frontier
comparison is component-wise; (c) budget tests for §7 defaults (slice
4); (d) model-based check of the poll state machine
(nothing-new/delta/resync) against #13094's verdict logic.

## 8. Consequences

Easier: every board reader shares one projection; policy changes move
through the policy revision in `view_id` instead of per-client logic;
folded subtrees make deep threads cheap to poll. Harder: comment reads
fan out per topic (bounded by paging first, §3.1 — a shared read cache
across topics in one view is a likely slice-4 follow-on, not this
design); the frontier grows with rendered topics (one input pair per
topic — digest-sized each, per record §6.2). Compatible: additive
module; the `boards.rs` world is untouched until the bridge question (§9.4)
is decided.

## 9. Open questions, each with a named decider

1. Per-view comment-read fan-out caching (shared cache across topics in
   one view). Default: sequential reads, paged first; add caching only
   on measured need. Decider: implementer at slice 4, with numbers.
2. Tombstone UX: filtered invisibly vs flagged placeholder (the
   boards.rs transparency rule reports hidden items). Default: filtered,
   per the issue. Decider: Drew (product call).
3. Policy revision source and format for `view_id`. Default: content
   hash of the serialized view policy. Decider: implementer, ratified by
   the #9130 owner.
4. Bridge to the `boards.rs` world (hash domains, author-chain order vs
   central seq). Default: no bridge in this slice; board readers adopt
   the builder per surface. Decider: Drew, on a migration proposal.
5. Cursor generation rotation and compaction horizons (when cursors go
   stale → resync). Default: mirror the append-list retention horizon as
   deployment policy (record §8.5). Decider: server operator; documented
   at implementation.

## 10. Sufficient-design standard conformance

This section maps the record onto the nine-point sufficient-design
standard; it adds no new technical decisions.

**Goals.** §1 (a)–(d): parent_ref tree, append-seq order, pending absent,
tombstone filtering without renumbering, per-subtree tokens, bounded
pages with cursor.

**Non-goals.** §1: #9127 protocol, #9130 state machine, #13094 crypto,
#13095 envelope, rankings, #9131 petnames, write paths.

**Design.** §2: `board_view.rs` placement, view/node types, 7-step build
algorithm, orphan/cycle/cross-topic/depth rules, five invariants.

**Decisions.** §3, five decisions with reasons; Drew directive supremacy
for product conflicts.

**Alternatives considered (including do-nothing).** §4: client-side
assembly, compose_view extension, view-level-only token, do-nothing —
each with the reason it lost.

**Build slices (dependency-ordered, with acceptance).** §5: four slices,
2←1, 3←2, 4←1+3, each with acceptance criteria; sub-issues of #13096.

**Rollout + rollback.** §6: additive, slice-ordered landing, revert per
slice, no migration.

**Verification plan.** §7: §5 oracle, three issue tests, property tests
(seq-stability, order, frontier comparison), budget tests, poll-machine
model check.

**Open questions, each with a named decider.** §9: five questions —
implementer (1, 3), Drew (2, 4), operator (5).
