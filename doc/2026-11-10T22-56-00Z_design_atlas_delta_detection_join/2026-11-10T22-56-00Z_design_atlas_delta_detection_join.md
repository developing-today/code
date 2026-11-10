# Design: per-source delta detection and non-doubling join semantics (atlas)

- **Issue:** #6028 (parent #6018, Atlas ingestion program) — the remainder
  after #6036 settled the offset matcher.
- **Settled elsewhere, taken as parameters here (not re-decided):**
  - #6036 (offset matching, landed): Gear rolling hash, window `W = 64 B`,
    `MIN_MATCH = 128 B` interior anchor run, BLAKE3 strong verify with byte
    comparison before any copy token is emitted, copy/literal token stream
    covering the new version contiguously.
  - #6035 (span-table schema, open): a span is
    `(file_version_id, start, end)` — byte offsets into one immutable file
    version. §2 and §4 are written against that shape; if #6035 changes the
    shape, this record's field names change, not its semantics.
  - #6037 (chunk-boundary policy v2, open): how *new* literal regions are
    cut into spans. Detection and join below are boundary-agnostic: they
    operate on whatever span list the cutter emitted.
  - #6040 (CDC feasibility, open): whether content-defined chunking joins
    the pipeline later. Nothing here depends on the answer; the matcher of
    #6036 already handles unaligned reuse.
  - Content versioning + retention record
    (`2026-11-08T10-26-00Z_design_atlas_content_versioning_retention`):
    versions are immutable, retirement is a state, not a deletion. §4.3
    relies on that.
- **Status:** design record. No implementation in this document's PR.

## 1. Problem

#6028 states two guarantees the ingest path must make testable:

1. **Delta-only landing.** Re-ingesting a largely unchanged corpus lands
   only the deltas: new files whole, edits as span-anchored embeds against
   the prior version. Untouched content is not re-embedded, not re-tagged,
   not re-dated. Drew's sizing case: a 21 GB corpus of which 20.5 GB is
   already in Atlas — the re-ingest may land ~0.5 GB of new/changed
   material, and the *detection* pass that decides this must cost far
   less than the embedding it saves.
2. **Non-doubling join.** At search time, a hit renders the full
   paragraph, and text assembled from overlapping span-anchored embeds
   plus version text contains every source byte exactly once.

#6036 settled how one file's new version is matched against its previous
version. This record settles the two ends around it: how the system
decides *which* objects need the matcher at all (§2, per source type,
with the cost ceiling), and how matched span sets are turned back into
paragraphs at search time (§4). §3 states the anchoring and overlap
rules both ends share.

## 2. Change detection per source type

Detection answers one question per object: *unchanged, changed, new, or
gone — since the last landed ingest of this source?* It never embeds,
never re-tags, and writes nothing except the ingest ledger entry
recording its verdicts. The per-source mechanisms differ because the
sources' own change signals differ; the ledger verdict shape does not.

### 2.1 Git sources — tree diff, never working-tree reads

- The unit of detection is the commit. For a re-ingest at commit `C`
  with last-landed commit `L`, detection is the tree diff `L..C`
  (equivalently `git diff-tree -r --no-renames L C`, with rename
  detection run as a second pass only over the delete/add pairs).
- The diff names the changed blobs directly. Unchanged subtrees are
  pruned by tree-object identity without descending: detection reads
  tree objects only, never blob content, for everything the diff calls
  unchanged.
- New/changed blobs go to the #6036 matcher (changed) or the plain
  first-ingest path (new). Deleted paths produce a version-retirement
  event for the file, not a re-embed.
- **Cost:** O(size of the tree-diff output + tree objects on changed
  paths). For the 21 GB sizing case with 0.5 GB changed, detection
  reads the commit graph and the changed-path trees — megabytes, not
  gigabytes — plus the 0.5 GB of blobs the matcher must read anyway.
  Detection cost is ~0% of the corpus, by construction.

### 2.2 Plain trees — hash walk with a metadata pre-filter

A directory tree has no commit graph, so detection walks it — but the
walk is staged so the expensive stage touches only suspects:

1. **Stat pass (always).** Walk paths, sizes, and mtimes. Any file whose
   `(path, size, mtime)` equals the landed ledger entry is *presumed
   unchanged* and not opened. This is a directory-listing pass: kernel
   readdir + stat, no content reads.
2. **Hash pass (suspects only).** Files whose stat triple differs, and
   files with no ledger entry, are content-hashed (BLAKE3, streaming).
   Hash equal to the landed version's hash ⇒ unchanged (the stat
   difference was a touch, not an edit); the ledger entry is refreshed
   for the new mtime *in the ledger only* — the landed object is not
   re-tagged or re-dated, per the #6028 guarantee. Hash differs ⇒
   changed; no entry ⇒ new.
3. The mtime pre-filter is an optimisation, never the verdict: any file
   the operator or policy marks untrusted (mtime unreliable — e.g. a
   tree restored from backup, where every mtime is new but content is
   old) skips stage 1 and is decided by stage 2 alone. Backup-restore
   is the common case this protects, and it degrades to "hash
   everything", not to "re-embed everything".

- **Cost ceiling:** worst case (no stat hits) is one streaming read of
  the corpus: 21 GB at commodity sequential-read speed (~1–3 GB/s NVMe,
  ~0.5 GB/s spinning/SMB) is ~10–45 s of pure I/O, single pass, no
  allocation per byte. Compare the cost being avoided: embedding 20.5 GB
  at the embedding pipeline's observed throughput class (tens of MB/s
  per worker through the model door) is hours. Detection at ~1–3% of
  the re-embed cost in the *worst* case, and ~0% in the stat-hit common
  case, satisfies "reads much cheaper than embeds" with two orders of
  magnitude to spare. A detection implementation that reads the corpus
  more than once, or that embeds speculatively, violates this ceiling
  by definition.

### 2.3 Databases and session stores — watermark + row hash

Row stores (SQLite/Postgres exports, session stores) change by rows,
not files; file-level hashing would call the whole store changed on
every append.

- Each ingested table declares a **watermark column** — a monotonically
  increasing key the store itself maintains (rowid, `updated_at` with a
  tiebreaker key, or an append-only sequence). Detection reads
  `max(watermark)` and the row count cheaply (an index probe, not a
  scan); rows at or below the landed watermark are presumed unchanged.
- Presumption is verified per row for the boundary region only: rows
  carry a landed row hash (BLAKE3 over the row's canonical encoding).
  Rows after the watermark are hashed and compared — new watermark
  rows with an existing key are *edits* (the row's new version lands;
  the old row version is retired, not mutated), rows with new keys are
  new, and a **tombstone sweep** (keys present in the ledger, absent in
  a full key listing) runs on a slow cadence (daily, or on operator
  request) to catch hard deletes the watermark cannot see. The key
  listing is keys only — narrow, index-served, cheap.
- Stores with no usable watermark column fall back to §2.2 at row
  granularity: hash every row, compare against the ledger. Still no
  re-embed of unchanged rows.
- **Cost:** O(changed rows + one key listing). For an append-mostly
  session store this is ~the delta itself. The full-key tombstone
  sweep is the only O(table) step and it moves keys, not content.

### 2.4 The detection ledger is the contract

Every detection pass appends one ledger entry per source: counts by
verdict (unchanged / changed / new / gone), the mechanism used, the
stat-presumption rate for §2.2, and the wall-clock + bytes-read of the
pass. The #6028 guarantee is auditable from the ledger alone: a
re-ingest whose `changed + new` byte total is 0.5 GB must show landed
embeds of ~0.5 GB, and any mass re-tag or re-date of unchanged objects
appears as ledger writes against objects whose verdict was *unchanged*
— which the ingest path is forbidden to make. That prohibition, stated
once: **an object whose detection verdict is unchanged receives no
writes of any kind — no embed, no tag write, no metadata edit, no
date bump. The only record of the pass touching it is the aggregate
count in the detection ledger entry.**

## 3. Anchoring and overlap acceptance (shared rules)

- **Anchor unit and target.** Span offsets are **bytes**, into the
  immutable version named by `file_version_id` (#6035 shape). Byte
  offsets are the only unit every source type shares (git blobs and DB
  row encodings have no line structure); renderers convert to
  lines/chars at display time, never in stored anchors.
- **No drift within a version.** Because a version is immutable, an
  anchor into it cannot drift. An edit creates a *new* version; the
  #6036 matcher re-anchors surviving content into the new version's
  span list, and embeds that match nothing are simply not referenced
  by the new version. "Detachment" is therefore not an event that
  happens to a live anchor — it is the absence of a reference from the
  new version, and it is recorded in the ingest ledger as part of the
  changed file's span disposition (referenced / superseded / orphaned
  counts). An embed whose content is referenced by no live version is
  an **orphan**; orphans are retained or retired by the retention
  record's rules, never silently dropped, and a search hit may still
  resolve through an orphan if the querying scope includes its
  version (see §4.3).
- **Minimum anchor run.** Settled by #6036: `MIN_MATCH = 128 B`
  interior, any length at a boundary (offset 0, EOF, or the end of a
  previously emitted token). Drew's 17-byte head match (indices 0–16)
  is exactly the boundary case: it anchors because it touches offset
  0, not because 17 ≥ 128.
- **Maximum overlap / duplicate ceiling.** A new span whose content is
  ≥ 0.9 covered by spans already landed for the same logical file is
  not embedded as a near-duplicate: it is recorded as an *occurrence*
  of the covering spans (the #5733 ≥ 0.9 dedup edge, #3125 occurrence
  semantics) and the version's span list references those spans with
  this version's offsets. Below 0.9, the non-covered residue is a
  literal region for the matcher/cutter and the covered part is
  copy tokens — partial overlap is normal and expected (it is the
  accepted repetition in Drew's requirement), total duplication is
  what the ceiling removes.
- **Occurrence recording.** Every copy token and every ≥ 0.9 coverage
  verdict records `(span_id, version_id, start, end)` as an occurrence
  of the underlying content in the span table (#6035). Occurrences are
  how §4 knows two spans carry the same bytes; they are also the audit
  trail for the dedup guarantee.

## 4. Join / reconstruction semantics

### 4.1 Assembly

A search hit names a span `(version_id, start, end)` and a paragraph
extent — the paragraph the span's text belongs to, computed from the
version's own structure (paragraph boundaries are a property of the
version text, recorded at ingest by the #6037 cutter's structural
pass; a span that crosses a paragraph boundary contributes to each
paragraph it touches). Assembly of one paragraph `P` of version `V`:

1. Collect every span of `V` whose range intersects `P`, plus — for a
   hit that arrived via an older version's span — the occurrence-mapped
   counterpart ranges in `V` when the hit is being rendered against
   `V` (cross-version rendering is by occurrence, never by offset
   arithmetic on the other version).
2. Sort by `start`. Walk left to right maintaining a coverage cursor
   `c`, initially `P.start`. For each span `S` in order:
   - the part of `S` before `c` is already emitted — skip it;
   - a gap `c .. S.start` is filled from `V`'s own stored version text
     (the version text is the ground truth; spans are indexes *into*
     it, not replacements for it);
   - emit `S`'s bytes from `max(S.start, c)` to `S.end`; set
     `c = S.end`.
3. Any tail gap `c .. P.end` is filled from version text as in step 2.

The version text is always available for a live version (it is the
landed object itself), so assembly never *requires* span contents
beyond what the hit carried; spans decide what is highlighted and
what is a hit, the version decides what the paragraph says.

### 4.2 The non-doubling invariant (testable)

> **Invariant J1.** For every paragraph `P` of every live version `V`,
> assembly(§4.1) emits exactly the byte sequence `V[P.start .. P.end)`,
> byte for byte — no byte twice, no byte missing, no byte invented —
> regardless of how many spans intersect `P`, how heavily they
> overlap, or which version the hit arrived through.

This is a property test, not an aspiration: the test harness generates
random texts, random edit sequences, and random span cuts (including
adversarial overlaps: nested spans, spans identical in content at
different offsets, spans crossing paragraph boundaries), runs ingest +
assembly, and compares against the version text directly. J1 must
hold for every generated case, and the 17–241 worked example (§5) is
one fixed regression case in the same suite. A second, search-level
statement follows from J1: **a paragraph returned for a hit contains
the hit's matched text exactly once per occurrence of that text in
the paragraph itself** — if the source paragraph genuinely repeats a
line, the rendering repeats it (that is the source, not doubling);
assembly never adds a repetition the source does not have.

### 4.3 Cold and retired versions

- **Cold version** (retained, not resident): assembly is unchanged.
  The version text is fetched from cold storage once per paragraph
  render; span data (the index) stays hot. Coldness is a latency
  property, never a correctness property — a renderer must not
  substitute a neighbouring version's text because the hit version is
  cold.
- **Retired version** (past retention, per the versioning/retention
  record): its spans are tombstoned in the span table. A hit that
  resolves only through a retired version renders as a **tombstone
  result**: the hit's stored snippet (the span text captured at
  ingest, kept with the span row precisely for this case), the
  version id, and the retirement marker — never a silent substitution
  of the current version's text, and never a fabricated paragraph.
  If a successor version carries an occurrence of the same span, the
  result may *additionally* offer the successor rendering, labelled
  as the successor version. Substituting it silently would violate J1
  for the version the user asked about.
- **Orphaned spans** (§3) behave as retired-version spans when their
  only versions are retired, and as normal spans while any live
  version references them.

## 5. Worked example — Drew's 17–241 case, end to end

File `F`, version `V1` landed with spans covering all of `V1`. An edit
produces `V2`:

1. **Detection.** Git source: tree diff names `F` changed (§2.1). (A
   plain-tree source reaches the same verdict via stat + hash, §2.2.)
2. **Matching (#6036).** The matcher indexes `V1` and slides over
   `V2`: verified copy run at `V2[0..17)` matching `V1` (boundary
   rule — touches offset 0, kept at 17 B); literal region
   `V2[17..242)` — the edit; verified copy run from `V2[242..]`
   matching `V1`'s tail span. The literal region is cut into one new
   span `S_new` anchored `(V2, 17, 242)`; the copy runs become
   occurrence records against the prior spans. This is Drew's "only
   this one diff is new", as data.
3. **Landing.** Only `S_new` is embedded. `V1`'s spans receive no
   writes (§2.4). The ledger entry for the pass shows 1 changed file,
   225 new bytes embedded, 2 occurrence records.
4. **Join.** A search hit in `S_new` asks for its paragraph, which
   spans the boundary: assembly (§4.1) takes the head span's part up
   to the cursor, `S_new`'s contribution from 17, and the tail span's
   contribution after 242 — J1: the rendered paragraph equals `V2`'s
   paragraph bytes exactly, the matched head and tail appearing once
   each even though three spans supplied them.

## 6. Non-goals and follow-ons

- No matcher changes: #6036 is settled; this record consumes it.
- No span-table DDL: that is #6035's record; §3/§4 state the semantics
  the schema must be able to express (occurrence rows, tombstone
  state, per-span stored snippet for §4.3).
- Implementation splits (to be filed as follow-ons when this record
  lands): (a) detection ledger + git tree-diff detector; (b) plain-tree
  hash walk with the stat pre-filter; (c) row-watermark detector for
  the session store; (d) the §4.1 assembler + the J1 property suite.
  Each is independently landable; (a) and (d) carry the two #6028
  guarantees and should go first.
