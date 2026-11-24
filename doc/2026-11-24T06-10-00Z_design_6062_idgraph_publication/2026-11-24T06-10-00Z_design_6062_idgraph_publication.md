# ID-graph publication format — Atlas objects + provenance/lineage relations (9-point sufficiency record for #6062)

- Date: 2026-11-24T06:10:00Z
- Source issue: [developing-today/code#6062](https://github.com/developing-today/code/issues/6062) — `design(id/atlas): ID-graph publication format — Atlas objects + provenance/lineage relations as ID-graph objects`
- Parent: [#6022](https://github.com/developing-today/code/issues/6022) (agent-note placement; now CLOSED) — decides _which_ classes publish
- Program: [#6018](https://github.com/developing-today/code/issues/6018) (Atlas ingestion program)
- Tracking: [#13873](https://github.com/developing-today/code/issues/13873) (AUDIT-7 Lane 6 DESIGN-GAPS)
- Queue task: 21677 (`repo-health-code-pilot`, slice-1, owned paths `doc/` only, design-only)
- Technical core (consumed, not rewritten): [`doc/2026-11-09T07-20-00Z_design_atlas_id_publication/2026-11-09T07-20-00Z_design_atlas_id_publication.md`](../2026-11-09T07-20-00Z_design_atlas_id_publication/2026-11-09T07-20-00Z_design_atlas_id_publication.md) — merged via PR #7998 (2026-11-14). This record wraps that core in the 9-point sufficient-design standard; it changes no mapping.

## 1. Problem and context (evidence-bounded from #6062)

#6062's body scopes a follow-up to #6022: #6022 decides _which_ note/object classes publish into the ID graph; #6062 designs the _format_ for Atlas-side publication. Quoted scope from the issue body:

- "Atlas objects (files, versions, chunks/spans, ingest runs) as ID-graph objects: identity, writer-set, and which provenance fields ride along."
- "Provenance/lineage relations mapped from the PROV-lite record (sibling comment on #3125: `wasRevisionOf` for version↔version, `specializationOf` for version↔logical file, `wasDerivedFrom`, `wasGeneratedBy`, `wasAttributedTo`) into the ID graph's relation vocabulary — one mapping, so the graph shows the same lineage the corpus stores."
- "Display: reuse the common relation-display methods already ticketed in the #5686 family; this design names the Atlas-side projection, not a new viewer."
- "A publication-format design: object mapping, relation mapping, and the projection contract (canonical in Atlas tables; ID graph as projection — or the reverse, decided explicitly)."

What breaks today without it (verified 2026-11-24 on this branch at `58bbda7407b`): Atlas file/version/lineage models and #3125's provenance vocabulary exist, but no contract maps Atlas records into ID-graph objects/relations — so any projector work (#11182–#11186) would invent its own mapping. Prior triage (tasks 10427/12470, revalidated 2026-11-08) confirmed the same gap; the 2026-11-09 design (PR #7998, merged) closed the technical gap, and the §9 split is now filed as sub-issues #11182–#11186. This record supplies the missing sufficiency wrapper (goals, alternatives, slices, rollout, verification, deciders, checklist) that the prior core does not state in the 9-point shape.

## 2. Goals and non-goals

Goals:

1. Fix one object mapping: each Atlas record class (logical file, immutable version, chunk occurrence, ingest run/batch) becomes a named ID-graph object form with identity derivation, writer-set, and which provenance fields ride along vs stay by reference.
2. Fix one relation mapping: each PROV-lite relation (`wasRevisionOf`, `specializationOf`, `wasDerivedFrom`, `used`, `wasGeneratedBy`, `wasAttributedTo`, plus file↔note binding, tombstone, occurrence) onto the existing ID event/relation form with directionality fixed once.
3. Decide canonical authority explicitly (Atlas tables vs ID graph) with rebuild, disagreement, and removal rules.
4. Cover versioning of the projection (projection v1) and signing/attestation for ingest batches (one `attestation.claim` per batch digest, pipeline-key signed) — in scope because the issue's Atlas-object list includes ingest runs and provenance.
5. Leave a dependency-ordered build split with per-slice acceptance, so #11182–#11186 (already filed) can be checked against it.

Non-goals (explicit):

- No class selection: #6022 (now CLOSED) owns which note/object classes publish; this design takes its placement table as configuration input.
- No new relation vocabulary, viewer, or display methods: relation/display work stays in the #5686 family (and #5545 writer-set projection, landed via PR #6672); this design names only the Atlas-side projection.
- No redefinition of #3125 occurrence semantics, the #6026 versioning/retention lifecycle, or the #6045 ingest-attestation statement shape — all consumed as inputs.
- No implementation: projector code, view presets, and audits belong to #11182–#11186.

## 3. Alternatives incl. do-nothing

- **A. Do nothing.** Keep the merged 2026-11-09 core as-is with no sufficiency wrapper. Honest tradeoff: the mappings exist and the §9 split is filed, but a reader checking the 9-point standard finds no stated goals/non-goals, alternatives, rollout/rollback, verification oracles, or named deciders in one place — the gap this task (via #13873) exists to close. Rejected as the status quo this record fixes.
- **B. Canonical in the ID graph (reverse of chosen).** Atlas would edit its rows from projection state. Rejected: it makes the signed, append-only graph the system of record for bytes it never owned, breaks rebuild-from-Atlas, and contradicts the versioning design's retention ownership.
- **C. One ID object per chunk.** Rejected: ~127K occurrences were already deduped at first ingest (#3125); header-resident claims (512-byte cap, `chain.rs:97-118`) cannot carry occurrence lists. Chosen instead: occurrences as `tag.set` claims by reference, heads in full, non-heads by summary + on-demand backfill.
- **D. Relations as `note.link` instead of claims.** Rejected: links assert authored references between notes (`note_links.rs:38+`, head-resolved); lineage edges are pipeline-asserted facts about versions. Chosen instead: `tag.set` claims in the `atlas.*` namespace, subject always the derived/newer entity (directionality fixed once), leaving existing link folds and consumers untouched.
- **E. Rewrite the 2026-11-09 core.** Rejected: it is merged, reviewed, and referenced by five filed sub-issues; rewriting risks silent semantic drift. Chosen instead: this wrapper record that cites the core by path and changes nothing.

## 4. Chosen approach (publication format)

Authority (D1): **canonical in Atlas tables; the ID graph is a re-projectable view.** Publication is a deterministic function of (Atlas rows, publisher key, projection version). Disagreement resolves toward Atlas via correction publication, never an Atlas edit. ID-side removal never deletes Atlas rows. Reverse direction named but out of scope: an ID-native object later ingested by Atlas is recorded as an occurrence with a source mark pointing at the ID event — never re-published. Full rules in core §§1, 6.

Object mapping (core §2):

- Immutable version → `document.put`; content hash **is** the document identity. In the event/claims: `(file_id, version_seq)`, content hash + size, #6044 source-mark locator, ingestion timestamp, attestation reference. Full occurrence rows, chunk bytes/embeddings, lineage evidence stay in Atlas by reference (keyed for recomputation).
- Logical file → `note.create` + `note.revise` chain; note ID is the published identity, `file_id`↔note-ID binding in an `atlas.fileId` claim. New head version → one `note.revise` with a `DocumentRef` to the version document; non-head versions → document + claims only. Rename is a version-claim attribute, never a new file. Source deletion → final revise to a well-known tombstone marker document + tombstone claim; history stays addressable, file excluded from default resolution.
- Chunks/occurrences → `tag.set` claims on the version document (`atlas.*` names, span offsets + locator + establishment method). Default: head versions in full, non-heads as summary (chunk count + occurrence-set hash) with on-demand backfill. No chunk object exists in the graph. Unassociated (`atlas.unassociated`) and dedup-collapsed (`atlas.dedupTo`) states publish as claims, never silently dropped.
- Ingest run / batch → one `attestation.claim` per batch digest (in-toto-shaped statement per #6045; refused batches publish too, with policy + reason). Attestation bodies are body-resident, not header-capped.

Relation mapping (core §3, D6): all PROV-lite relations publish as `tag.set` claims under `atlas.*`, subject = derived/newer entity. Table: `specializationOf` (version doc → file note ID), `wasRevisionOf` (new version doc → prior version doc ID + edge kind/confidence; split = one claim per successor, merge = one per predecessor), `wasDerivedFrom` (→ source version IDs + method/confidence), `used` (→ source material id), `wasGeneratedBy` (→ batch attestation claim ID), `wasAttributedTo` (→ agent id string; event signer remains the pipeline key), plus binding/tombstone/occurrence rows. Values are JSON strings; anything over the header cap stays in Atlas by reference.

Writer-set / views (core §4, consumed from #5545 / PR #6672): publisher is the ingest pipeline key; one `idset.writer` set admits exactly that key (stored ≠ counted; per-view counting by view owners with named config/set-version/frontier/snapshot). The projector publishes everything #6022 qualifies, uniformly — no consumer pre-filtering.

Versioning / signing (core §§2.4, 7, 8): format is **projection v1** carried on every event; evolution appends names/optional fields, never redefines a v1 name; breaking changes go under new names (or `atlas2.*`) with a migration note. Batch statements embed verbatim (#6045 bytes, statement hash as subject) so Atlas and ID attestations are byte-comparable (default, §8.1 owner: slice S2 implementer). Publisher key custody/rotation default: one dedicated pipeline key per Atlas deployment, rotation via existing `issuer.rotation`, old + new admitted during overlap (owner: ingestion operations, not this contract).

Replay / correction / retention / consistency (core §6): run logs pin (Atlas snapshot marker, publisher key id, projection version) for bit-identical replay; unpinned consumers get at-least-once delivery and dedupe on (version doc ID, claim name, value hash). Corrections = new claims + `tag.clear` naming superseded claim IDs (history preserved). Retirement publishes `document.remove` + `atlas.tombstone`; cold rollout publishes nothing. Per-row atomicity unit (document + `specializationOf` + `wasGeneratedBy` minimum) published in Atlas ingestion order; any unit recomputes from Atlas rows + key + projection version with no projector state.

## 5. Dependency-ordered implementation slices (one lane/session each)

These mirror the filed sub-issues #11182–#11186 (filed 2026-11-17 under #6062/#6018); each slice states its acceptance so a lane can work it independently. Order is load-bearing: S1 → S2 → S3 → S4 → S5.

- **S1 — Projector: versions, files, relations emission** (tracks #11182; core §§2.1–2.3, §3). Acceptance: on a fixture corpus the projector emits `document.put` / `note.create` / `note.revise` chains and `atlas.*` tag claims per the mapping; head-tracking, tombstone revises, and correction clears behave per §§2.2/6.2; consumes #6022's placement table as configuration only.
- **S2 — Projector: attestations + ingest runs** (tracks #11183; core §2.4). Acceptance: each batch digest yields one `attestation.claim` with verbatim #6045 statement bytes; refusals publish with policy + reason; batch-chain tracking preserved. Depends on S1 (claim IDs it references).
- **S3 — Projector operations: run logging, replay, backfill** (tracks #11184; core §6 + §8.3). Acceptance: every run log pins the three replay inputs; replay reproduces bit-identical output when pinned; on-demand backfill keyed by `(file_id, version_seq)`; ordering/atomicity tests per §6.4 pass. Depends on S1–S2.
- **S4 — Consumer view presets** (tracks #11185; core §4). Acceptance: one `idset.writer`/view preset per audience (heads-only, attested-only, full-history) using landed #5545 semantics; no new view machinery. Depends on S1–S3 (needs emitted shapes to filter).
- **S5 — Conformance audit over a fixture corpus** (tracks #11186; core §§6–7). Acceptance: every Atlas row projects to a complete unit (§6.4); every claim recomputes from Atlas rows (§6.1); corrections clear exactly the superseded claims (§6.2); retirement publishes remove + tombstone (§6.3). Depends on S1–S4.

## 6. Rollout and rollback

Rollout (design-only record, so this is the rollout of the _contract_, consumed by S1–S5): land this record + README bullet via PR to `main` (this task); link the record from #6062; keep #6062 OPEN as the parent until #11182–#11186 land. Projector rollout (for implementers): ship behind a dry-run/project-behind flag — emit to a scratch view, run the S5 conformance audit, then promote the view; projection v1 advertised by the projector, pinned by consumers.

Rollback: no data migration is involved. If the record is wrong, supersede by a new dated record (dated records are never rewritten); if a projector slice mis-emits, fix by correction publication (`tag.clear` + new claims, core §6.2) and re-projection from Atlas — never by editing Atlas from the graph or by deleting ID history. ID-side removal primitives (`document.remove`, `tag.clear`, `idset.rescind`) affect the projection only.

## 7. Verification plan (oracles)

- **Oracle O1 (recomputation):** any published unit recomputes from (Atlas rows + publisher key + projection version). A verifier with Atlas read access and the run-log pins needs no projector state (core §§6.1, 6.4). Failing case: a claim whose value cannot be derived from the named Atlas rows.
- **Oracle O2 (unit completeness):** every published version carries at minimum its document event + `specializationOf` + `wasGeneratedBy`; gaps detectable via `version_seq` without Atlas access (core §§6.1, 6.4).
- **Oracle O3 (correction exactness):** a supersession clears exactly the superseded claim IDs and adds the corrected relations; superseded events remain in history (core §6.2).
- **Oracle O4 (retirement/tombstone):** Atlas retirement yields `document.remove` + `atlas.tombstone`; folded content stays resolvable as "retired under policy" (core §6.3).
- **Oracle O5 (no silent redefinition):** v1 claim names keep v1 meanings; any D2–D6 change appears under new names with a migration note (core §7).
- **Record-level checks for this task:** the doc exists at the dated path below; all 9 standard points present; slices dependency-ordered with per-slice acceptance; every open question names a decider; linked from #6062; owned paths `doc/` only.

## 8. Open questions with named deciders + defaults

1. Exact `attestation.claim` body bytes for a batch statement (field-by-field transcoding vs verbatim). **Decider:** S2 implementer (projector attestations lane). **Default:** embed #6045 statement bytes verbatim, statement hash as subject (core §8.1).
2. Occurrence-claim encoding at scale (one claim per occurrence vs packed spans). **Decider:** S3 implementer, with numbers from the existing-embed audit (#6029). **Default:** one claim per occurrence for heads, packed summaries otherwise (core §8.2).
3. Non-head backfill push vs pull. **Decider:** S3 implementer. **Default:** pull — backfill request keyed by `(file_id, version_seq)` (core §8.3).
4. Publisher key custody and rotation. **Decider:** ingestion operations (program #6018 ops; Drewry Pope confirms). **Default:** dedicated pipeline key per Atlas deployment, rotation via `issuer.rotation`, overlap admission (core §8.4).
5. Classification/sensitivity labels on published claims (sealed labels if #5410 requires them). **Decider:** Drewry Pope (program owner) on the manifest design's requirement. **Default:** ordinary string fields in v1; sealed labels arrive as v2 fields, never a v1 rework (core §8.5).
6. Whether #6062 itself can close once this sufficiency wrapper lands while #11182–#11186 remain open. **Decider:** Drewry Pope. **Default:** stays OPEN as the parent tracker until the children land (matches the 2026-11-17 triage verdict; this record closes nothing).

## 9. Record/tracking/acceptance checklist + evidence references

- [x] 1. Problem + context, evidence-bounded from #6062 body (§1 above; issue body quoted; #6022/#3125/#5686/#5545 boundaries as the issue states them).
- [x] 2. Goals + explicit non-goals (§2).
- [x] 3. Alternatives with honest tradeoffs, incl. do-nothing (§3, options A–E).
- [x] 4. Chosen approach: object mapping, relation mapping, projection contract with canonical authority, versioning, signing/attestation (§4; technical detail in the consumed core).
- [x] 5. Dependency-ordered slices, one lane/session each, with acceptance criteria (§5, S1–S5 → #11182–#11186).
- [x] 6. Rollout + rollback (§6).
- [x] 7. Verification plan with oracles (§7, O1–O5 + record-level checks).
- [x] 8. Open questions, each with a named decider + default (§8, Q1–Q6).
- [x] 9. This dated record + tracking + queue linkage (this section).

Evidence references:

- Source: #6062 body (scope: files/versions/chunks-spans/ingest-runs; PROV-lite relations from the #3125 sibling comment; #5686 display reuse; canonical-or-reverse decision) + 2026-11-08 triage notes on the issue.
- Inputs consumed, not redefined: #3125 (occurrence + PROV-lite vocabulary, OPEN); #6022 (placement input, now CLOSED); #5545 writer-set projection + landed PR #6672 (`idset/view.rs`); #6026 versioning/retention design; #6045 ingest attestation; #6044 mark grammar.
- Prior deliverable: `doc/2026-11-09T07-20-00Z_design_atlas_id_publication/` via PR #7998 (merged 2026-11-14); remainder filed as sub-issues #11182, #11183, #11184, #11185, #11186 (program #6018).
- Tracking: #13873 (AUDIT-7 Lane 6 DESIGN-GAPS, slice source); queue task 21677 (this record's lane).
- State note: verified at branch base `58bbda7407b` (2026-11-24); #6062 OPEN with sub-issues 0/5 complete at time of writing.
