# Content model cluster — decision record for #5839

Status: decided (design only; no implementation in this record).
Parent: #5678 (knowledge layer feature program). Foundation: #2048
(zettelkasten layer). Programme: epic #2056. Evidence base: the
#6435 competitor feature matrix (`doc/knowledge-layer-feature-matrix-6435.md`).

This record settles the content-model cluster #5678 names: block-based
editing (Notion/Logseq) vs page/document models (Evernote/OneNote/
Obsidian), daily notes as an organising primitive, outlines vs
freeform, and databases/tables-as-content (Notion databases, Anytype
sets). Per the parent programme's method, each feature is adopted,
adapted to ID's multiplayer object model, or explicitly rejected with
reasons. Nothing here re-decides #2048 or the parent's ruled
read-authority default; where an answer would change one of those, §7
routes it back instead of forking it.

Decisions this record builds on (not re-decided):

- #2048: a note is a stable logical object — a signed `note.create`
  event whose ID is the note identity, with immutable signed
  revisions, typed references stored as (target note ID, sigil/type,
  authored surface, optional pinned revision), and names as claims,
  never identity.
- #5678 read-authority ruling: default public read; resolution order
  object → user default → server config → public. §6 states this
  cluster's compliance.

## D1. The primary unit is the note; blocks live inside it (ADAPT)

ID adopts the page as the unit of identity and adapts the block as
the unit of _content structure_:

- The **note** (page) is the only content object with identity. It is
  what links target, what citations pin, what permissions attach to,
  and what syncs as a revision lineage (#2048).
- A note's content is an **ordered tree of blocks**. A block is a
  typed content node (paragraph, heading, list item, to-do, quote,
  code, image/attachment reference, table, transclusion — D2) with a
  stable block ID **scoped to its note**.
- Blocks are not independent graph objects. A block has no identity
  outside its note, carries no permissions of its own, and is never
  the target of a note-level link. This is the deliberate middle
  between the two competitor poles the matrix documents: Notion,
  where the block is an implementation detail of a page and database
  rows are pages; and Logseq, where the block is the object and
  pages are outlines of blocks. ID takes Notion's identity shape and
  Logseq's structured content, because #2048 already fixed identity
  at the note and permissions resolution (§6) only works at one
  granularity.

## D2. Block addressing: anchors, not objects

- A reference may address a block as (note ID, pinned revision ID,
  block ID) — an extension of #2048's typed references with an
  intra-note anchor. The pinned revision makes the anchor exact: it
  resolves to the block as it was in that revision, forever.
- An unpinned block reference resolves against the current head and
  degrades honestly: if the block no longer exists at head, the
  reference degrades to the note itself with an unresolved-anchor
  state (the same degrade-to-surface rule #2048 sets for names).
- Block IDs are unique within a note and are never reused within
  that note, including after deletion (tombstoned IDs stay taken).
- Moving a block to another note mints a new block ID in the target
  note; the move is recorded in the revision so provenance survives.
  Anchors to the old (note, block) pair degrade per the rule above
  rather than silently following the content.

## D3. Outlines are a mode, freeform is deferred (ADAPT / REJECT for v1)

- The block tree is a tree, so an **outline view** — folding, zooming
  to a block, hoisting — is a presentation and editing mode over any
  note, not a separate document type. Logseq-style outlining is
  adopted as a mode; there is no outline storage model distinct from
  a note.
- **Freeform/canvas documents** (infinite-canvas spatial content)
  are rejected for v1: they need a spatial content model that does
  not reduce to the block tree, and no cluster evidence in the
  matrix makes canvas load-bearing for a knowledge layer's first
  shape. Revisit only as its own cluster if the matrix programme
  redraws clusters (parent method step 3).

## D4. Daily notes are notes; the journal is a view (ADAPT)

- A **daily note is an ordinary note** created from a date template
  and carrying a date claim. It has no special storage, identity, or
  permission semantics.
- The **journal is a projection**: a date-ordered view over daily
  notes the viewer may read, plus capture actions that append blocks
  to today's note (creating it from the template on first capture).
  Logseq's journal-as-organising-primitive is adopted as a view and
  a capture habit, not as a second content store.
- Capture into a past or future date targets that date's note the
  same way; there is exactly one daily note per (owner, date), found
  by its date claim, and contested claims resolve by #2048's
  name-claim rules rather than silently merging.

## D5. Databases are views over notes, not a content store (ADAPT)

- A **database is a saved query plus a presentation** over notes:
  rows are notes, columns are typed properties carried on notes
  (the Anytype sets/collections pattern and Obsidian's
  database-like views over a vault, per the matrix — not Notion's
  substrate, where the database is a storage object of its own).
- Properties are note-level typed claims with the same provenance
  discipline as #2048's other claims; a database definition (query,
  columns, sort, presentation) is itself a note, so databases are
  shareable and revisable like everything else.
- Editing a cell edits the underlying note's property. There is no
  row content that is not a note, and deleting a database deletes a
  view, never the notes it showed.
- **Tables inside a note** are unrelated: a table is one block type
  (D1) holding grid content. Conflating the two — a table block is
  not a database, a database is not a block — is the naming error
  this decision exists to prevent.

## 6. Read-authority compliance

- Read resolution (#5678 ruling: object → user default → server
  config → public) is evaluated **at the note**, the only granularity
  this model has. Block-level read settings are rejected: they would
  create a second, finer resolution order the ruling does not
  define, and would leak through anchors and transclusion.
- **Transclusion** (a block that renders another note's block)
  renders only when the reader may read the source note under the
  resolution order; otherwise it degrades to the reference's
  authored surface in an unresolved state — never to the content.
- Journal views, database views, backlinks, and search inherit the
  same rule per note, exactly as #2048 already requires for its
  projections: a view never exposes a note — or a note's existence
  metadata — to a reader the resolution order would deny.

## Invariants implementation must hold

1. One granularity of identity: links, citations, permissions, and
   revision lineage attach to notes; blocks are addressed only
   through (note, revision, block).
2. A pinned block anchor is exact and permanent; an unpinned anchor
   degrades to the note, never to a different block.
3. Block IDs are never reused within a note; cross-note moves mint
   new IDs and record provenance in the revision.
4. A daily note is indistinguishable in kind from any other note;
   the journal is derivable from date claims alone.
5. A database is derivable from its definition note plus the notes
   it queries; deleting it deletes no content.
6. No view (journal, database, outline, graph) exposes a note to a
   reader the read-authority resolution order would deny, including
   through transclusion.

## Alternatives rejected

- **Block-as-object (pure Logseq).** Every block independently
  addressable, linkable, and permissioned. Rejected: it contradicts
  #2048's settled note identity, multiplies the permission surface
  past what the read-authority ruling defines, and makes revision
  lineage (which block revision belongs to which note revision) a
  second graph to keep consistent.
- **Opaque document (pure Evernote/OneNote).** A note as an
  unstructured document blob. Rejected: no anchors, no
  transclusion, no structured capture — it throws away the typed
  references #2048 already decided and the block structure every
  matrix product except the oldest converges on.
- **Journal as a separate store.** A dedicated journal database
  beside notes. Rejected: a second content store duplicates sync,
  permissions, and search for content that is notes in every
  respect except its date.
- **Database as storage substrate (pure Notion).** Rows as a
  distinct object kind living in databases rather than in the note
  graph. Rejected: it forks identity (a row is and is not a page),
  and the sets/views pattern reaches the same user capability with
  one object model.

## 7. Open questions routed to the parent

- **Canvas/freeform:** rejected for v1 here (D3); if the programme
  wants it, it belongs to #5678 as a redrawn cluster, not as an
  extension of this record.
- **Block-level comments and reactions:** commenting _on_ a block
  without making the block an object is unspecified here; it is a
  capture/organisation or sharing-cluster concern for #5678 to
  route, and any answer must preserve invariant 1.
- **Property schema governance** (who may define property types a
  database relies on): the extensibility cluster's question; D5
  assumes only that properties are typed claims with provenance.

## Next actions

- The block content schema inside note revisions (block types, the
  anchor resolution rules of D2, and the D4/D5 projections) is filed
  as implementation follow-on issue #8218 alongside this record; the
  note substrate itself remains #2048's implementation packets
  (#3814/#3816/#3819), which this record does not reopen.
- Sibling cluster designs inherit D1's granularity: any cluster
  proposing a new addressable unit must reckon with invariant 1 or
  route the change through #5678.
