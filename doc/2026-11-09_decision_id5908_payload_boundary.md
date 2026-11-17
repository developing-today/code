# Decision: the payload boundary — decompose-vs-attach decision procedure

- **Date:** 2026-11-09
- **Status:** decision record — no implementation is included in this change.
- **Issue:** [#5908](https://github.com/developing-today/code/issues/5908),
  "id: design: payload boundary: decompose-vs-attach decision procedure
  (from #5688)".
- **Parent design:** [#5688](https://github.com/developing-today/code/issues/5688)
  — its §2.4 "The payload boundary" is the section this record settles.
  The parent's decisions (KDL as the human presentation format, atomic
  triples as the canonical data model, ANSI SQL/PGQ as the query model,
  postcard as the wire format) are **not re-decided here**; §7 records
  that explicitly.
- **Inputs cited:**
  - Parent #5688 §2.2 (pipeline stages; stage 2 canonical JSON is the
    provenance anchor), §2.3 (namespaces and qualifiers are more
    triples, never special cases), §2.4 (the boundary statement this
    record turns into a procedure), §2.5 (tagging is triples).
  - [#5906](https://github.com/developing-today/code/issues/5906) /
    `doc/2026-11-08_decision_id5906_kdl_json_contract.md` — the sibling
    conversion record; its §11 explicitly leaves "Payload boundary
    (decompose vs attach)" to #5908, and its §5 compound-statement
    anchor is the statement node this record's payloads attach to.
  - [#5912](https://github.com/developing-today/code/issues/5912) /
    `doc/2026-11-06_decision_id_tagging_triples.md` — tag claims as
    triple projections; §5's referencing shape follows the same
    "claim node + provenance triples" pattern.
- **Evidence baseline:** `origin/main` at `129df4af83`, worktree
  `~/git/fleet/14233-5908`, 2026-11-09. Parent #5688 and issue #5908
  read fresh at drafting; #5908 is OPEN with no prior decision record
  and no PR owning this design (per triage task 13866, 2026-11-09).

## Decision summary

| Question | Decision |
| --- | --- |
| Where is the boundary? | Decided **per field/part of a statement**, never per document: apply the ordered procedure in §2 to each part; a single statement routinely lands parts on both sides (§4 grey cases) |
| What decides? | First matching test in §2 wins: **addressed-by-others → queried/joined → reasoned/tagged → decomposed**; **carried/displayed/verbatim → attached**; parts matching neither default to attach (§2, step 6) |
| How is a payload referenced? | A payload is a **first-class ID'd object**. Triples reference its ID (`statement —has-payload→ payload-id`, plus media-type / hash / size as ordinary triples); the payload bytes are **never** a triple object and never embedded in the triple table (§5) |
| How is a payload resolved? | At read/query time, through its ID, exactly like every other ID in the parent model (§5.2); the triple store stays byte-free |
| Can a payload later cross the line? | Yes, one way at a time, by an explicit **promotion** (attach → decompose) that mints triples *about* the payload while the payload itself stays attached; demotion never deletes triples, it only deprecates them (§6) |

## 1. What the boundary is (and is not)

Parent §2.4 fixes the intuition: decompose what is *queried, joined,
tagged, or reasoned over*; attach what is *carried, displayed, or
stored verbatim*. That sentence is a direction, not a procedure — two
implementers can apply it to the same field and disagree. This record
supplies the missing procedure (§2), the referencing/resolution shape
(§5), and the invariants (§3) an implementation is tested against.

The boundary is **not**:

- a size threshold. Size is evidence (very large parts are almost
  never atomised), never the rule: a 40-byte string can be a payload
  (a verbatim quote) and a 40 KB structure can decompose (a table
  people filter by). §8 rejects the threshold design explicitly.
- a per-document choice. A document is a bundle of parts; deciding
  "this document is a payload" wholesale is how searchable titles,
  authors, and timestamps end up trapped inside blobs (§4).
- a storage-engine choice. Both sides live under the parent model:
  triples in the triple table, payloads in a payload store keyed by
  ID. Neither side invents a special column or table per content
  type (parent §2.3's rule applies at this boundary too).

## 2. The decision procedure

Apply these tests **in order** to each part of an authored statement
(a field, a child node, a value). The first test that matches decides.
Record the deciding test number in the ingest provenance for any part
where the answer was not obvious — that record is what lets a later
design amend the procedure from evidence instead of taste.

1. **Addressed test.** Will anything else ever need to point *at this
   part* — link it, cite it, tag it, attach provenance to it, or make
   it the subject/object of another statement? If yes → **decompose**
   (it needs an ID and triples; a payload can be pointed at only as a
   whole, §5).
2. **Query test.** Will a query need to select, filter, sort, or join
   on this part's *content* (not merely retrieve it with its parent)?
   If yes → **decompose**.
3. **Reasoning test.** Will rules, inference, aggregation, or the
   SQL/PGQ graph views (parent §3) traverse *through* this part to
   other facts? If yes → **decompose**.
4. **Verbatim test.** Is this part's value precisely its exact bytes —
   would re-encoding, re-flowing, normalising, or summarising it
   change what it *is* (a text body, an image, a geometry, a signed
   document, a recording)? If yes → **attach**.
5. **Display test.** Is this part only ever rendered whole, as-is,
   alongside its parent statement? If yes → **attach**.
6. **Default.** If no test matched, **attach**. Decomposition is
   earned by a demonstrated need under tests 1–3, not assumed:
   triples are the expensive, schema-committed representation, and
   §6's promotion path means an attached part can gain triples later
   without re-ingest, while a wrongly decomposed blob can never be
   faithfully reassembled.

Two corollaries:

- **Split, don't compromise.** When tests point both ways for
  different sub-parts, split the part: decompose the addressed/queried
  sub-parts, attach the verbatim remainder, and link them with triples
  (§4 works the examples). "Decompose a summary, attach the whole" is
  a normal outcome, not an inconsistency.
- **Qualifiers are never payload.** Time, place, certainty, source,
  and role are triples under parent §2.3 even when they describe a
  payload (`payload —recorded-at→ time-id`). Attaching a payload never
  licenses burying its qualifiers inside it.

## 3. Invariants implementation must hold

- **I1 — No bytes in triples.** A triple's subject, predicate, and
  object are IDs. Payload bytes, and payload content of any kind,
  never appear as a triple element or in the triple table.
- **I2 — Every payload has an ID and provenance.** An attached payload
  is stored under its own ID with, at minimum, triples for media
  type, content hash, byte size, and the statement(s) it attaches to.
  An unreferenced or unidentifiable payload is an ingest bug.
- **I3 — Round-trip honesty.** Export (parent §1.4) can reassemble a
  statement's decomposed triples and its attached payloads into the
  authored shape; the payload bytes come back bit-identical (checked
  against the I2 hash), the triples semantically identical.
- **I4 — One-way, lossless promotion.** Promoting a payload (§6) adds
  triples *about* it and never mutates or discards the payload;
  nothing about a payload's ID or hash changes when it is promoted.
- **I5 — No special cases.** A new payload kind needs no schema
  change: media types are data (triples), not columns, tables, or
  parser branches (parent §2.3).
- **I6 — The anchor carries references, not content.** The stage-2
  canonical JSON (parent §2.2, #5906 §5) records payload *references*
  (payload ID + hash) in place of payload bytes above the inline
  floor set by the conversion contract; the authored shape stays
  auditable without duplicating payload storage.

## 4. Grey cases, worked

Each case names the deciding test(s). These are the calibration set:
an implementation (or a future amendment) that disagrees with a case
here must say which test it reads differently, and why.

| Part | Verdict | Why |
| --- | --- | --- |
| Long text body of a note | **Attach** the body; **decompose** title, author, timestamps, tags | Body: verbatim (4) + display (5). Title/author/time: queried and addressed (1, 2). Tags are triples outright (parent §2.5) |
| Photograph | **Attach** the image bytes; **decompose** caption, photographer, subjects, location, time | Bytes are verbatim (4). "Photos of Ada in Lisbon" is a query (2) over the decomposed facts, which reference the payload ID |
| Geometry / route track | **Attach** the geometry; **decompose** named endpoints, region, distance class | Geometry is carried verbatim (4); endpoints are addressed and joined (1, 2). If a future feature queries *inside* geometries, that sub-part is promoted per §6 — the geometry still stays attached |
| A KDL document someone uploaded | **Attach** the document as authored; **decompose** the statements it parses into | The document is the provenance artifact (verbatim, 4) *and* the ingest source: parent §2.2 decomposes its statements. Both, linked — the split corollary, not a contradiction |
| Source-code snippet in a discussion | **Attach** the code text; **decompose** language, the symbols it defines/mentions *if* tooling queries them | Code is verbatim (4) — whitespace is semantics. Symbol extraction is a §6 promotion when a consumer actually arrives (test 2), not speculative ingest work |
| Sensor reading series | **Decompose** each reading *if* readings are filtered/aggregated (the usual case); **attach** a raw capture file (e.g. a waveform) the readings were derived from | Readings fail verbatim (they are queried numbers, 2/3); the raw capture is carried verbatim (4) and is the readings' provenance |
| A person's name as a string | **Decompose** — the name belongs to a person entity referenced by ID (parent §2.2 stage 4) | Names are addressed and joined (1, 2). The string is a property of the entity via triples, never a payload; attaching it would fork identity |
| A timestamped comment thread | **Decompose** each comment's author/time/reply-to; **attach** each comment's body text | Structure is addressed and traversed (1, 3); bodies are verbatim display text (4, 5) — the note-body case, per comment |
| External JSON blob (API response kept for audit) | **Attach** the blob verbatim; **decompose** the fields the audit queries (source, fetched-at, subject) | The blob's value is its exact bytes as received (4). Promoting arbitrary fields "just in case" violates the §2 default; promotion (§6) covers the field that later becomes queried |
| Audio recording with a transcript | **Attach** both the recording and the transcript text; **decompose** speaker, participants, time, and any cited/quoted segments as addressed sub-parts | Recording is verbatim (4). The transcript is also verbatim (4) — but a *quoted segment* another statement cites passes the addressed test (1) and is decomposed as a citation, pointing back at the transcript payload ID |

## 5. Referencing and resolution

### 5.1 How triples reference a payload

A payload is an ordinary ID'd object in the parent model. The
attachment itself is triples, e.g. for statement `S` with payload `P`:

- `(S, has-payload, P)`
- `(P, media-type, <media-type-id>)`
- `(P, content-hash, <hash-id>)`, `(P, byte-size, <size-id>)` — I2's
  minimum provenance set, as ordinary triples (values resolve through
  IDs per parent §2.2 stage 4, as with every other value)
- `(P, derived-from, S)`-style provenance where §6 promotion or §4
  derivation applies

There is no payload column, no payload flag, and no second reference
mechanism. A graph query (parent §3) reaches a payload exactly the
way it reaches anything else: by traversing triples to its ID.

### 5.2 How a payload is resolved

Resolution is the same ID resolution the parent already specifies
(§2.2 stage 4): a reader holding payload ID `P` asks the payload
store for `P` and gets the bytes plus the I2 provenance triples.
Resolution is by ID only — never by content search inside payloads,
never by position inside a parent document. If bytes are needed in
bulk (export, re-ingest, migration), they move under their IDs with
their hashes, and I3's check applies on arrival.

### 5.3 What a payload is not

A payload is not a statement and carries no truth value; claims
*about* it are triples (§5.1), and claims *derived from* it after
promotion are triples with provenance pointing at its ID (§6). A
payload is also not a hiding place: anything a consumer needs to
query, cite, or reason over must not exist *only* inside a payload
(§2 tests 1–3 are the enforcement at ingest).

## 6. Promotion: when an attached part crosses the line

Needs change: a field nobody queried becomes the thing every query
filters on. Promotion is the deliberate, recorded act that handles
this — never a silent re-ingest:

1. Mint the triples the new need requires, about the payload or its
   parts, with provenance triples naming the payload ID and the
   promotion's reason/ticket.
2. The payload stays attached, byte-identical, under the same ID
   (I4). Parts that are themselves addressed by the new triples get
   their own IDs and link back to the payload ID.
3. Demotion is not the reverse operation: triples, once minted, may
   be deprecated (marked superseded by later triples) but are never
   deleted to "re-attach" content — history is data in this model.

## 7. Parent decisions: explicitly not re-decided

This record changes no parent #5688 decision. For the avoidance of
doubt: KDL presentation, atomic triples, SQL/PGQ query, and postcard
on the wire all stand as the parent states them. The #5906 record's
deferral of this question to #5908 is discharged by this record, and
nothing here amends #5906's conversion contract — I6 is written to
compose with it (references in the anchor), and if an implementer
finds I6 and the conversion contract in conflict, that conflict is a
proposed parent-level change and must be recorded explicitly on
#5688, per issue #5908's own instruction, not forked silently.

## 8. Alternatives rejected

| Alternative | Why rejected |
| --- | --- |
| **Decompose everything** (no payloads; all content atomised) | Destroys verbatim fidelity (I3 becomes unmeetable for media, signed documents, code), explodes the triple table with facts nobody queries, and contradicts parent §2.4, which names legitimate attached payloads outright |
| **Attach everything compound** (statements as document payloads, triples only between documents) | Recreates the blob store parent §2.1 rejects: unqueryable content, identity forks (the name-string case), and graph views with nothing to traverse |
| **A byte-size threshold** (under N bytes decompose, over N attach) | Size does not track the §2 tests: verbatim quotes are small, queried tables are large. A threshold would misclassify both §4 directions and would silently re-decide statements when a part grows past N |
| **Per-type hard-coded rules** (a table mapping each content type to a side) | A special case per type — exactly what parent §2.3 forbids; the table would fork from the procedure on the first new type. Types may carry *defaults* recorded as data, but the §2 procedure remains the authority when a default is challenged |
| **Inline small payloads in the triple table as literal objects** | Violates I1 and parent §2.2 stage 4 (triple elements are IDs); literals in the object slot are how triple stores grow a second, unqueryable data model inside the first |

## 9. What this record leaves open (and to whom)

| Question | Owner | Why not here |
| --- | --- | --- |
| The payload store itself (engine, replication, retention) | Implementation follow-on, filed from #5908 | This record fixes the referencing contract (§5) an implementation must satisfy, not the store's internals |
| The inline floor in I6 (below which the anchor may carry bytes directly) | #5906 conversion-contract implementation | It is a conversion-layer constant; this record requires the floor to exist and references to replace bytes above it |
| Media-type vocabulary (which media-type IDs exist, who mints them) | Tagging/vocabulary alignment, #2048 line | Vocabularies are data (parent §2.5); this record only requires media type to be one (I2, I5) |
| Whether promotion is manual, tooling-assisted, or automatic on observed queries | Implementation follow-on | §6 fixes promotion's semantics and invariants; its trigger is an operational choice |
| SQL/PGQ shapes for reaching payloads through graph views | Parent §3 / capability ticket | §5.1 guarantees payloads are reachable by ordinary traversal; query-surface design is the parent's ticket |

## 10. Change log

- 2026-11-09 — first recorded decision (task 14233, lane code-d,
  queue `repo-health-code-pilot`). Drafted from parent #5688 §2.4
  read fresh, the #5906 sibling record's §11 deferral, and issue
  #5908's acceptance criteria. No implementation included.
