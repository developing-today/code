# Atlas → ID-graph publication format: corpus objects + provenance/lineage relations as ID-graph objects

Design doc — 2026-11-09. Deliverable for
[#6062](https://github.com/developing-today/code/issues/6062) (child of
[#6022](https://github.com/developing-today/code/issues/6022); Atlas ingestion
program [#6018](https://github.com/developing-today/code/issues/6018)).
Design only; no implementation is authorized by this note. Implementation is
split into follow-ons in §9.

State verified at `origin/main`
`c2283a5721` (2026-11-09) for this design pass (queue task 13828):

- Issue #6062 is OPEN, `kind/design`, no hard-stop labels, no linked
  implementation PR. Prior triage (task 12470, 2026-11-08) confirms the gap:
  Atlas file/version/lineage designs and #3125's provenance vocabulary exist,
  but no contract maps Atlas records into ID-graph objects and relations.
- Inputs consumed, not redefined: the Atlas versioning/retention design
  (`doc/2026-11-08T10-26-00Z_design_atlas_content_versioning_retention/`,
  deliverable for #6026, now CLOSED); the writer-set view-projection design
  (`doc/2026-10-30T13-15-00Z_design_id_writer_set_view_projection/`,
  #5545, now CLOSED) with its landed library slice (merged PR #6672:
  `pkgs/id/src/idset/view.rs`, frontier-pinned one-level expansion,
  exclusions win, `Public` a deliberate signed choice); #3125's occurrence
  registry semantics and PROV-lite record (still OPEN — consumed as the
  vocabulary source); the ingest-attestation design
  (`doc/2026-11-09T05-12-32Z_design_atlas_ingest_attestation/`, #6045).
- The `pkgs/id` event vocabulary below was read at this `origin/main`
  (`pkgs/id/src/documents.rs`, `notes.rs`, `note_links.rs`, `tag_claims.rs`,
  `idset.rs`, `attestation.rs`, `chain.rs`) and is cited `path:line`.
  Nothing about that vocabulary is changed by this note.

## 0. Requirement in one paragraph

Atlas stores corpus records (logical files, immutable versions, lineage
edges, chunk occurrences, ingest runs with attestations); the ID graph
stores signed, author-attributed objects with relations. This note fixes
the **publication contract** between them: which Atlas record becomes
which ID-graph object (identity derivation, writer-set, which provenance
fields ride along vs stay by reference), how each PROV-lite relation maps
onto an existing ID event/relation form with directionality fixed once,
and who is canonical when the two disagree. #6022 decides _which_
note/object classes publish; this note designs the _format_ for whatever
Atlas rows exist to be published.

## 1. Authority boundary (Decision D1)

**Canonical in Atlas tables; the ID graph is a projection.** Atlas owns
bytes, versions, lineage edges, retention, cold rollout, and deletion.
The ID graph carries published _records_ — identity plus provenance
pointers — and is never the only copy of anything Atlas owns.

Consequences, all load-bearing:

- **Rebuild is re-projection.** Publication is a deterministic function
  of (Atlas rows, publisher key, projection version §7). Losing the ID
  side loses nothing: re-run the projector over Atlas and re-emit.
  Re-projection emits _new_ events; it never edits or deletes ID
  history (the ID store's no-silent-rewriting rule holds).
- **Disagreement resolves toward Atlas.** If a published claim and an
  Atlas row disagree, Atlas wins; the fix is a correction publication
  (§6), never an Atlas edit driven by the projection.
- **ID-side removal never deletes Atlas rows.** `document.remove`,
  `tag.clear`, and `idset.rescind` affect the projection only. Atlas
  retention runs on its own triggers (§6.3) and never consults ID state.
- **The reverse direction is out of scope but named:** an ID-native
  object (whatever #6022 qualifies) that Atlas later ingests is recorded
  as an _occurrence with a source mark pointing at the ID event_ — a
  locator, not a re-publication. No object is published twice.

## 2. Atlas records → ID-graph objects

### 2.1 Immutable version → `document.put` (Decision D2)

A version's content is a blob; `document.put` names a blob by hash and
size (`pkgs/id/src/documents.rs:32`). The version's content hash **is**
the document identity — no second hash, no alias table. This is the same
split the versioning design already makes: content identity names the
bytes, `(file_id, version_seq)` names the version.

What rides along vs stays by reference (header limits force the split:
tag operations live in the signed header extension
`ChainExt.tag` (`pkgs/id/src/chain.rs:97-118`), and header text is capped
at 512 bytes — a version's full occurrence list cannot fit in a claim):

- **In the publication event / its claims:** `(file_id, version_seq)`,
  content hash + size, source locator in the #6044 mark grammar
  (`repo@commit:path`, `system@host@date`, `chat@session@timestamp`,
  content hash riding in the mark), ingestion timestamp, attestation
  reference (batch id, §2.4), and the edge-kind label where applicable.
- **By reference in Atlas:** the full occurrence rows, chunk bytes and
  embeddings, lineage evidence blobs, classification scan detail. Every
  published claim carries enough keying (`file_id`, `version_seq`,
  batch id) that a verifier can recompute it from Atlas rows alone.

### 2.2 Logical file → `note.create` + `note.revise` chain (Decision D3)

A logical file is a stable identity over changing content — exactly what
a note is: `note.create` establishes a note whose ID is the creating
event's ID (`pkgs/id/src/notes.rs:41-46`), and each `note.revise` adds
one immutable revision with parents (`notes.rs:48+`, `MAX_PARENTS = 32`).

- The file's **note ID is its published identity.** The
  Atlas-`file_id` ↔ note-ID binding is recorded in a claim (§3.2), so
  either side resolves to the other.
- Each new **head version** publishes as a `note.revise` whose content
  is a `DocumentRef` (`documents.rs:43-50`: media type + content hash +
  size) naming the §2.1 document, with parent = the prior head revision.
  The note's revision chain is therefore the file's head history, and
  non-head versions exist in the graph only as documents + claims, never
  as note revisions — heads are the query default (§5.1 of the
  versioning design), and the projection mirrors that.
- **Update behavior:** a new version that becomes head → one
  `note.revise`. A new version that does _not_ become head (out-of-order
  ingest, backfill) → document + claims only, no revise. A rename is not
  a new file: path is an attribute of the version claim, never of the
  note — same rule as the versioning design (§2.1 there).
- **Tombstone behavior:** source deletion publishes a final
  `note.revise` whose content names a tombstone marker document
  (fixed well-known content: `text/atlas-tombstone` + the
  `(file_id, version_seq)` of the last live version), plus a
  tombstone claim (§3.2). The pre-tombstone revisions stay addressable;
  default resolution excludes the file — mirroring "tombstones do not
  resurface" without deleting history.

### 2.3 Chunks and occurrences → claims by reference, heads in full (Decision D4)

Chunks are too fine-grained for one-object-per-chunk: first ingest
dropped ~127K occurrences by dedup (#3125), and header-resident claims
cannot carry occurrence lists. So:

- **Occurrences publish as `tag.set` claims** on the version's document
  event: subject = version document ID, `atlas.*` names (§3.2), span
  offsets + locator + establishment method (observed / segment-matched /
  human-asserted, per #3125's S31 requirement) in the value. Full
  occurrence rows stay canonical in Atlas; the claim is the pointer.
- **Publication depth default (stated default):** head versions publish
  their occurrence claims in full; non-head versions publish a summary
  claim (chunk count + content hash of the occurrence set) unless
  explicitly addressed (pin/`as_of`/trace replay), in which case the
  projector backfills that version's full claims on demand. Rationale:
  bounded publication cost per ingest event; history stays recomputable.
- **Shared chunks across versions** need no fan-out: one chunk, twelve
  versions = twelve occurrence rows in Atlas, twelve claims on twelve
  version documents. No chunk object exists in the graph at all.
- **Unassociated + duplicate states** (#3125 S32) publish as claims
  too: an unassociated file gets an `atlas.unassociated` claim on its
  version document; a dedup-collapsed write gets an
  `atlas.dedupTo` claim naming the surviving content hash. Recorded,
  never silently dropped — on both sides.

### 2.4 Ingest run / batch attestation → `attestation.claim`, one per batch (Decision D5)

The #6045 design emits one in-toto-shaped statement per batch (subject =
chunk hashes + spans, materials = source hashes + verbatim #6044 marks,
builder = ingester build, batch/classification/`prior_attestation`
chain). That statement publishes as **one `attestation.claim` object
per batch digest — not per chunk.** The claim's subject is the statement
hash; materials and marks ride in the claim body (attestation bodies are
body-resident, not header-capped). OpenLineage run fields (job/run,
`nominalTime`, parent) map onto the claim's run descriptor; the
`prior_attestation` chain gives the ID side the same ordering the Atlas
side has.

Refused batches publish too: a rejection is a provenance fact (the batch
existed, was examined, did not land, and why) — same rule as #6045, now
visible in the graph with the refusing policy + reason in the claim.

## 3. PROV-lite relations → existing ID vocabulary (Decision D6)

Source vocabulary (#3125 PROV-lite fold-in): entity id, derived-from ids,
generated-by run id, attributed-to agent, timestamps, source mark,
content hash, span offsets; relations `wasRevisionOf` (version ↔ prior
version), `specializationOf` (version ↔ logical file), `used`,
`wasDerivedFrom`, `wasGeneratedBy`, `wasAttributedTo`.

Carrier decision: **relations publish as `tag.set` claims in the
`atlas.*` namespace, with directionality fixed once — subject is always
the derived/newer entity; the value names the source.** Claims are the
existing author-attributed relation form (`Claim { id, author, subject,
name, value }`, `tag_claims.rs:133+`); the claim's author is the
publisher key (§4). `note.link` is _not_ the carrier: links assert
authored references between notes (`note_links.rs:38+`, `Assert
{ source, revision, target, surface, pin }`) and resolve through note
heads — lineage edges are pipeline-asserted facts about versions, not
authored note references. Existing link folds and their consumers are
untouched.

| PROV relation       | `tag.set` claim (subject → value)                                                     | Notes                                                                                                                                                 |
| ------------------- | ------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| `specializationOf`  | version document → file note ID, name `atlas.specializationOf`                        | Every version gets one, including non-heads and tombstoned. Head revisions additionally express this via note content ref (§2.2).                     |
| `wasRevisionOf`     | new version document → prior version document ID, name `atlas.wasRevisionOf`          | Value carries the Atlas edge kind (`edit`/`rename`/`split`/`merge`) + confidence. Split = one claim per successor; merge = one claim per predecessor. |
| `wasDerivedFrom`    | derived version document → source version document ID(s), name `atlas.wasDerivedFrom` | Segment-matched associations (§3.1(b) of the versioning design) carry method + confidence in the value.                                               |
| `used`              | version document → source material id, name `atlas.used`                              | Materials named the same way the attestation names them.                                                                                              |
| `wasGeneratedBy`    | version document → batch attestation claim ID, name `atlas.wasGeneratedBy`            | One hop from any version to its batch statement (§2.4).                                                                                               |
| `wasAttributedTo`   | version document → agent id string, name `atlas.wasAttributedTo`                      | The event signer is the pipeline key; this claim names the logical agent (ingester build, human asserter) behind it.                                  |
| file ↔ note binding | file note → `file_id`, name `atlas.fileId`                                            | The bidirectional key; value also carries the source-system anchor (native object id / conversation id) where one exists.                             |
| tombstone           | version document → tombstone record, name `atlas.tombstone`                           | Trigger + rule + run id, mirroring the retirement tombstone (§4.4 of the versioning design).                                                          |
| occurrence          | version document → span + locator, name `atlas.occurrence`                            | §2.3; head versions in full, others by summary + on-demand backfill.                                                                                  |

All names live under `atlas.*` so pipeline claims never collide with
user tags. Values are JSON strings; anything that does not fit the
header text cap stays in Atlas by reference with its key in the value.

## 4. Writer-set and views (consumed from #5545 / PR #6672, not redefined)

- The **publisher is the ingest pipeline key**: publication events are
  signed by it, and the pipeline owns one `idset.writer` set admitting
  exactly that key (entries/excludes semantics, one-level pinned group
  expansion, exclusions win, `Public` only by deliberate signed choice —
  all per the landed `idset/view.rs` behavior).
- **Stored ≠ counted applies unchanged:** publication events verify and
  store under the ID validity gate; whether they count for anyone is a
  per-view decision by the view owner, reversible, with every computed
  view naming (view config, set version, claim frontier, resolution
  snapshot). This note adds no set algebra, no view descriptors, no
  discovery changes — those are #5545's (and its v2 follow-ons') to make.
- Consumers filter further with their own views (e.g. a view that counts
  only attested batches, or only heads). The projector MUST NOT
  pre-filter for consumers: it publishes everything #6022 qualifies,
  uniformly.

## 5. Boundary with #6022 (input, not duplication)

#6022's placement decision table (note class → home → lifetime →
Atlas-ingested at which tier) is an **input** to this contract: whatever
rows Atlas holds as a result of that placement publish in the format
above. This note does not select note classes, does not define homes or
lifetimes, and does not duplicate the placement table — it references
it. Concretely:

- If #6022 qualifies a class for **native ID publication** (object born
  in the graph, never a file), §1's reverse rule applies: Atlas may
  ingest it later as an occurrence whose source mark points at the ID
  event. The projector never re-publishes it.
- If #6022 routes a class **through Atlas** (file first, graph by
  projection), this note's §§2–4 govern its graph form.
- If #6022 changes which classes publish, no format change is needed
  here — only projector configuration (which tables/tiers feed it).

## 6. Replay, correction, retention, consistency

### 6.1 Replay

Re-projection is always available (§1) and is the only recovery path the
contract guarantees. Replay inputs are named in every projector run log:
Atlas snapshot marker (max ingested `version_seq` per file + batch
chain head), publisher key id, projection version (§7). A consumer that
pins those three inputs gets bit-identical output; a consumer that does
not pin them gets at-least-once delivery and MUST deduplicate on
(version document ID, claim name, value hash). Gaps are detectable:
`version_seq` in claims makes a missing sequence number visible without
Atlas access.

### 6.2 Correction

Atlas lineage edges are versioned and supersedeable (a stronger signal
may replace an edge). Supersession publishes as: new claim(s) with the
corrected relation **plus `tag.clear` naming the superseded claim IDs**
(`Clear { claims }`, `tag_claims.rs:103+` — clears name claims, which is
exactly the correction primitive). Corrections are themselves attributed
and timestamped; the superseded claim stays in history (clear removes
liveness, not the event). The projector keeps a correction log keyed by
(edge id, superseding edge id) so replay reproduces the same clears.

### 6.3 Retention interplay

- Atlas retirement (trigger AND class rule, per the versioning design
  §4) publishes `document.remove` naming the version document plus an
  `atlas.tombstone` claim (trigger, rule, run id). ID history is
  preserved: removal names the doc; folded content stays ("content is
  kept on removal" — the store's existing rule), so existing references
  resolve to "retired under policy," never dangle.
- Cold rollout publishes **nothing**: cold is a latency class, not a
  visibility class. The graph cannot tell hot from cold and must not try.
- Head-tombstone (source deletion) follows §2.2: final revise to the
  tombstone marker, file excluded from default resolution on both sides.
- The projector never deletes, never compacts, never garbage-collects
  the graph. Any ID-side pruning is a store/retention concern outside
  this contract.

### 6.4 Projection consistency

- **Ordering:** the projector publishes in Atlas ingestion order
  (`version_seq` per file, batch chain order for attestations). Late or
  backfilled rows publish when observed; `version_seq` + batch chain
  make ordering reconstructible regardless of arrival order.
- **Atomicity per row:** a version's document + its relation claims are
  one publication unit; a consumer MUST NOT treat a version as published
  until the unit is complete (document event + `specializationOf` +
  `wasGeneratedBy` at minimum). The unit's manifest is the projector
  run log entry for that row.
- **Verification:** any published unit recomputes from Atlas rows +
  publisher key + projection version. A verifier needs no projector
  state — only Atlas read access and the run log's input pins (§6.1).

## 7. Projection versioning (Decision D7)

The format above is **projection v1**, and every published event carries
its projection version. Format evolution appends new claim names / new
optional fields; it never redefines a v1 name's meaning and never moves
a field from by-reference to inline silently. A v2 that changes any D2–
D6 mapping publishes under new claim names (or a new `atlas2.*`
namespace) with a migration note; v1 claims stay readable. The projector
advertises the versions it emits; consumers pin the versions they
understand (§6.1).

## 8. What this design does not decide (open items with defaults)

1. **Exact body bytes of `attestation.claim` for a batch statement**
   (how the in-toto statement maps field-by-field). _Default:_ embed the
   #6045 statement bytes verbatim as the claim body with the statement
   hash as subject — no transcoding, so Atlas and ID attestations are
   byte-comparable. _Owner:_ whoever implements the §9.3 slice.
2. **Occurrence-claim value encoding at scale** (one claim per
   occurrence vs packed spans per chunk). _Default:_ one claim per
   occurrence for head versions; packed summaries otherwise (§2.3). If
   head-version claim storms prove costly, the follow-on may pack spans
   per `(chunk, version)` with the same names. _Owner:_ §9.3 slice, with
   numbers from the existing-embed audit (#6029).
3. **Whether non-head backfill is push or pull** (§2.3 on-demand rule).
   _Default:_ pull — the projector exposes a backfill request keyed by
   `(file_id, version_seq)`; consumers request, projector publishes.
   _Owner:_ §9.3 slice.
4. **Publisher key custody and rotation** (which key signs, how rotation
   publishes). _Default:_ a dedicated pipeline key per Atlas deployment,
   rotation via the existing `issuer.rotation` event; the `idset.writer`
   set admits old + new during overlap. _Owner:_ ingestion operations
   (#6019), not this contract.
5. **Classification/sensitivity labels on published claims.** This note
   assumes the classification manifest vocabulary travels as ordinary
   string fields in claims; enforcement (who may read what) is a
   view/access concern, not a publication-format concern. If the
   manifest design (#5410) later requires sealed labels, that is a v2
   field, not a v1 rework.

## 9. Implementation split (follow-ons, all under program #6018)

1. **Projector: versions + files + relations** — §§2.1–2.3, §3:
   document/note/claim emission, head-tracking, tombstone revises,
   correction clears. Consumes #6022's placement table as configuration.
2. **Projector: attestations + runs** — §2.4: batch statement → claim
   mapping (§8.1), refusal publications, batch-chain tracking.
3. **Projector operations** — §6: run logging with input pins, replay
   tooling, on-demand backfill (§8.3), ordering/atomicity (§6.4) tests.
4. **Consumer view presets** — §4: one `idset.writer`/view preset per
   audience (heads-only, attested-only, full-history) using landed
   #5545 semantics; no new view machinery.
5. **Conformance audit** — property checks over a fixture corpus:
   every Atlas row projects to a complete unit (§6.4), every claim
   recomputes from Atlas (§6.1), corrections clear exactly the
   superseded claims (§6.2), retirement publishes remove + tombstone
   (§6.3). The oracle is recomputation from Atlas rows.

## Appendix A. Acceptance audit (against the task packet)

- _Authority boundary + projection/rebuild:_ §1 (canonical-in-Atlas,
  re-projection rebuild, disagreement and removal rules).
- _Identity + provenance mapping for files, versions,
  chunks/occurrences, ingest runs, incl. source locators and
  lineage/update/tombstone behavior:_ §§2.1–2.4 (with the #6044 mark
  grammar and #6026 edge kinds carried in claim values).
- _PROV-lite → existing ID event/relation vocabulary; consume #3125
  occurrence semantics and the landed #5545 writer-set projection:_
  §3 (claims carrier, `note.link` explicitly not used, directionality
  fixed) and §4 (consumed unchanged; PR #6672 behavior cited).
- _Replay, correction, retention, projection-consistency; unresolved
  labeled with defaults:_ §6 (+ §7 versioning) and §8.
- _Boundary with #6022 (input; no class selection, no placement-table
  duplication):_ §5 (plus §1 reverse rule and §9.1 configuration hook).

## Appendix B. Cross-reference audit

- #6022 (OPEN): placement/class-selection owner; this note takes its
  table as input (§5). No class chosen here, no table duplicated.
- #3125 (OPEN): occurrence semantics (§2.3: stored-once + occurrence
  rows, deletion only when unreferenced — preserved) and PROV-lite
  vocabulary (§3) consumed as specified; nothing redefined.
- #6026 (CLOSED): versioning/retention design consumed (§§2.1, 2.2
  identity; §6.3 retirement/tombstone/cold rules). No threshold or
  lifecycle changed.
- Merged PR #6672: landed `idset/view.rs` semantics (frontier-pinned,
  one-level, exclusions win, signed `Public`, unresolved = error)
  consumed as the view layer (§4); no view machinery proposed.
- No claim is made about #5686 or #5955 beyond what their own issues
  state: relation display/viewer work is explicitly out of scope (§3
  names existing claim/link folds and their current consumers only).
