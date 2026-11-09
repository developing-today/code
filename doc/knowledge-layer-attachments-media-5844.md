# Knowledge layer — attachments & media cluster decision record (#5844)

Status: decided (design only; implementation timing is a separate
decision, per #5844). Parent program: #5678. Object model: #2048.
Evidence base: the resolved feature matrix from #6435,
`doc/knowledge-layer-feature-matrix-6435.md` (PR #7887, merged
bfee2fe6b6b1a37b2c6f65871f5dd33465c2b328; #6435 closed completed
2026-11-09). This record settles the attachments & media cluster
only: file handling, OCR/handwriting, and PDF annotation. It does
not re-decide any parent or sibling decision (see "Preserved parent
decisions") and it designs no importer (importers: #6436, #6438,
#6439, #5845–#5848).

## Preserved parent decisions

- The ruled read-authority resolution order from #5678 applies
  unchanged and applies to every object this record creates:
  **object → user default → server config → public**. An attachment
  resolves its own read decision as an object; it never inherits a
  wider decision from a page that references it.
- #2048's object model is the substrate: attachments, derivatives,
  and annotations below are objects in that model, not side files
  with parallel bookkeeping.
- Program-level open questions recorded on #5678 / #6435 §6
  (product editions, evaluation budget, AI model choice, and the
  rest) stay open; nothing here answers them by assumption. Where
  hosted OCR would depend on the AI-model question, this record
  rules only the default (local / user's own infrastructure) and
  leaves a hosted option to that question.

## Decisions

### 1. File handling — ADOPT attachments as first-class resources

Attachments are addressable resources with their own identity
(Joplin-resources / Logseq-assets shape, matrix §3.6/§3.7 and §5
#5844), stored as content-addressed payloads with an object record
in the #2048 model. Pages and blocks *reference* an attachment;
they do not contain a copy of it.

Evidence: attachments are first-class resources in the local-first
products (Joplin resources, OCR-searchable; Logseq assets under the
graph's assets/ directory; Obsidian files in the vault folder) and
embedded content in the cloud products (matrix §5 #5844).

Rejected alternatives:

- **Embed-by-copy on the page (OneNote shape)** — Insert > File
  Attachment places a copy on the page and notebooks grow large
  from embedded documents, scans, PDFs, and media clips (matrix
  §3.4). Copies fork identity: the same file becomes several
  unrelated payloads with no shared read decision, dedupe, or
  update path.
- **Plan caps as a data-model property (Notion shape)** — the 5 MB
  free vs. 5 GB paid per-file cliff (matrix §5 #5844) is a pricing
  fact, not a model fact. Size limits here are sync/deployment
  policy (the Joplin shape: caps come from the sync plan, not the
  app), configured per deployment, never baked into the object
  model.

### 2. OCR and handwriting — ADAPT capture + local OCR derivatives

- Handwriting/drawing captured on mobile is stored as an attachment
  resource (the ink/image payload itself), following Joplin's
  mobile capture shape (handwriting/drawing, camera/photos,
  speech-to-text; matrix §3.7). A native in-app ink-editing surface
  is **not** adopted for v1: no matrix product's ink model survives
  export with fidelity, and ink editing is a separable follow-on if
  demand appears (see "Follow-ons").
- OCR is a **derived-data pipeline**, not a property of the file:
  OCR output is a derivative object linked to its source attachment,
  recording engine, language, and confidence. Joplin demonstrates
  OCR search inside PDFs and images as a resource-level feature
  (matrix §3.7).
- OCR and search indexing run **locally / on the user's own
  infrastructure by default**. The derivative and any index entry
  inherit the source attachment's resolved read decision and never
  widen it.

Rejected alternative:

- **Server-side OCR/indexing by default (Evernote shape)** —
  Evernote's semantic search indexes content, including attachment
  text via OCR, on vendor servers (matrix §3.2). An index is a read
  path: indexing content whose resolved policy is not public, on
  infrastructure the user does not control, violates the #5678
  read-authority order. (Same reasoning as the matrix's #5842
  AI-cluster reject.) A hosted OCR option is not ruled out forever;
  it is gated on the open AI-model question (#6435 §6 Q5) and, if
  adopted, must resolve the same policy per object before any bytes
  leave.

### 3. PDF annotation — ADAPT Logseq's attachment-native shape

PDF annotation is adopted as an attachment-native feature (Logseq
ships built-in PDF annotation over graph assets; matrix §3.6, §5
#5844), with one structural rule: **annotations are separate
objects, never edits to the source PDF**. An annotation object
references (attachment, page, region/anchor) and carries its own
author, timestamps, and read decision. Rendering an annotated copy
is an export-time operation.

Rejected alternatives:

- **Annotatable printouts / mutated copies (OneNote shape)** — a
  printout is a second, diverging payload; annotations on it are
  stranded when the source changes (matrix §3.4).
- **Annotation as a viewer extra** — viewer-local highlights that
  never enter the object model are unsearchable, unshareable, and
  lost on export; the matrix's adopt/adapt framing for #5844 names
  attachment-native annotation precisely to avoid this.

### 4. Export honesty — REJECT silent payload drops

Any export we ship states, in the export UI before it runs, what it
excludes (matrix §5 #5844). Full-fidelity export carries attachment
payloads and a manifest (resource id + content hash per payload);
a format that cannot carry payloads (e.g. a PDF rendering) says so
explicitly and names the fidelity-preserving alternative.

Rejected alternative: the Evernote PDF shape — PDF export silently
excludes the actual attachment files (ENEX/HTML required; matrix
§3.2/§5 #5844). Silent loss at export time is a defect, not a
format limitation. The interchange/full-fidelity export design
itself is #6439's; this record rules only the honesty invariant it
must satisfy for attachments.

## Implementation invariants

1. **Read authority:** the order object → user default → server
   config → public resolves independently for an attachment, its
   OCR derivative, its thumbnails/previews, and each annotation.
   No derivative ever widens access beyond its source's resolved
   decision.
2. **Identity:** a resource's id and content hash are stable across
   sync, import, and export. References are by id; identical
   payloads dedupe by hash.
3. **Reference, not copy:** embedding a file in a page creates a
   reference. Deleting a reference never deletes the payload while
   other references or grants exist; payload GC requires zero
   references and no retention hold.
4. **Source immutability:** OCR, annotation, and preview generation
   never mutate the source payload. Computed derivatives (OCR
   output, thumbnails/previews) are separate objects that can be
   regenerated or discarded. Authored annotations are **not**
   derived state: they are durable user-authored objects (decision
   3) and are never discarded or rebuilt as part of clearing or
   regenerating derived state.
5. **Sync shape:** payloads sync as content-addressed blobs,
   chunked and resumable; object records (metadata, derivatives,
   annotations) may sync ahead of payloads, and a missing payload
   renders as a resolvable placeholder, never as data loss.
   Offline capture queues OCR/indexing until the payload is local.
6. **OCR provenance:** every OCR derivative records engine,
   language, and confidence; OCR text is evidence about the source,
   never a silent replacement for it, and search results cite the
   source attachment.
7. **Export manifest:** every export lists the attachment payloads
   it carried and states its exclusions before running (decision 4).

## Follow-ons

No new split is required to close #5844: importer-specific
attachment handling stays with the importer designs (#6436, #6438,
#6439, #5845–#5848), and sync mechanics stay with the multi-device
sync cluster (#5838) subject to invariant 5. Two items are named
non-goals for v1, to be filed only on demonstrated demand: a native
ink-editing surface (decision 2) and hosted OCR (gated on #6435 §6
Q5, decision 2).

## Verification

Decision-record review against #5844's acceptance: the #6435 gate
was verified resolved before deciding (issue closed completed,
matrix on main via PR #7887); chosen/adapted/rejected outcomes are
recorded for file handling, OCR/handwriting, and PDF annotation,
each with matrix evidence and rejected alternatives; invariants and
boundaries for the #2048 object model are recorded; #5678's ruled
read-authority order is preserved verbatim and applied, not
re-decided.
