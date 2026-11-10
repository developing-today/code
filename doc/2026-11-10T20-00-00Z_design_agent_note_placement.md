# Design: where agent notes live — placement rule for zettel vs repo vs ID graph

- Status: design record for issue #6022 (Atlas ingestion programme #6018, brief statements S7–S10)
- Date: 2026-11-10
- Consumes, does not reinvent: writer-set authority (#5545), the capability/relation family (#5686), the worlds programme (#5955), the ID-graph publication format (child issue #6062), the docs-migration classification standard (#5981 / #5983), and the zettel brain-dump pipeline (one `note_submission` envelope; Turso submission DB as system of record; staging → human promotion; nothing auto-commits).
- Cross-links: Atlas programme #6018 · ID epic #2056 · publication format #6062

## 1. The one-step distinction (zettel vs repo)

An agent mid-task applies exactly one test:

> **Would this note still be true, and still be looked for, if the repo it is about were deleted tomorrow?**
>
> - **Yes → zettel.** The note is knowledge: a finding, a synthesis, a person/project fact, a cross-repo observation. It lives in the Zettelkasten (dezren39/zettel), linked into the slip-box graph, written for a future reader who does not know which repo prompted it.
> - **No → the repo.** The note is part of the work product: it describes, justifies, or operates code/artifacts in a specific repo (design doc, ADR, runbook, generated report about that repo's state). It is committed to that repo, versioned with the thing it describes.
> - **Neither yet → workspace/memory.** The note is not yet knowledge and not yet work product: it is working state (scratch, partial findings, session logs). It stays in the agent workspace or memory until it is promoted (§4) or expires (§3).

The crisp form: **zettel = knowledge about the world; repo = knowledge about the artifact; workspace/memory = knowledge about the current attempt.** A note's home is decided by what would orphan it, not by who wrote it or which tool produced it.

## 2. Placement decision table

| Note class | Home | Lifetime | Atlas ingestion |
|---|---|---|---|
| Ephemeral working note (scratch, intermediate output, hypothesis) | Agent workspace scratch (per-task worktree / lane scratch) | Task lifetime; deleted or summarized at task end | Not ingested |
| Session / daily log (what a shift did) | Memory daily log (`~/memory/YYYY-MM-DD.md`) + sweep/backlog records in the owning goal workspace | Permanent as history; curated into MEMORY.md by the memory system | Tier 3 — ingested as provenance only (who did what, when) |
| Durable finding (verified fact about a system, person, or project; survives its repo) | Zettel, via the `note_submission` envelope | Permanent; revised in place under zettel versioning | Tier 1 — first-class Atlas corpus |
| Research synthesis (multi-source, cross-repo) | Zettel, via `note_submission` | Permanent; superseded by newer syntheses, never silently edited away | Tier 1 |
| Design / ADR / architecture record for a specific repo | That repo's `doc/` (dated `*_design_*` naming), PR-reviewed | Life of the artifact; superseded designs kept and marked | Tier 2 — ingested as repo corpus with repo provenance |
| Runbook / operations procedure for a specific system | The repo that owns the system (or hatch-personal if it is sandbox-environment material, per #5981/#5983) | Life of the system; updated in place via PR | Tier 2 |
| User-facing document (deliverable Drew opens) | The requesting surface: goal `files/` / artifact system; canonical copy to repo or zettel when it is also knowledge | Until superseded by the next deliverable | Tier 2 if canonicalized; otherwise not ingested |
| Memory topic / standing fact about the user | Agent memory (`~/memory/topics/`, people/group pages) | Permanent; corrected via the memory system, never by repo edit | Tier 3 — provenance-marked, access-scoped; never published to the ID graph (§5) |
| Machine-generated state snapshot (queue health, quota, backlog state) | Owning system's store (queue DB, goal `hidden_files/` state) | Rolling; snapshots overwrite, history in logs | Not ingested as notes; Atlas reads the system of record |
| Note destined to be shared, cited, and related across systems (published finding, programme record, cross-system object) | ID graph (ID-native note object), per §5 | Permanent; versioned as graph object revisions | Tier 0 — the graph object *is* the Atlas record (projection contract in #6062) |

Atlas tiers: **0** = ID-native (Atlas record is the graph object); **1** = first-class corpus (full text + provenance spans); **2** = repo corpus (full text, provenance = repo @ commit); **3** = provenance only (metadata + pointer, content stays in its home system).

## 3. Lifetimes and expiry

- Workspace scratch expires with the task unless a promotion (§4) happens. Unpromoted scratch is not a loss: if it mattered, the task's outcome record links it.
- Memory is permanent but *curated*: daily logs are raw history; MEMORY.md and topic pages are the curated layer. Curation is the memory system's job, not a repo process.
- Zettel and repo docs are permanent and versioned; deletion is a deliberate, reviewed act in both.
- ID-native notes are permanent; retraction is a relation (superseded-by / retracted-by), never a silent delete — the graph is a provenance record.

## 4. Promotion paths (how a note changes home)

A note moves exactly when its answer to the §1 test changes:

1. **Scratch → finding.** A verified scratch result is rewritten as a self-contained zettel submission (the scratch wording, which assumes task context, does not move verbatim). Route: the single `note_submission` envelope → Turso submission DB (system of record) → staging → human promotion into zettel. Nothing auto-commits.
2. **Scratch → design/doc.** Work product is committed to its repo through the normal PR route, dated-design naming for designs.
3. **Finding/design → ID-native.** Only the classes in §5 qualify; publication is a new graph object that *cites* the zettel/repo original (wasDerivedFrom, per the #6062 mapping), it does not move or delete the original.
4. **Any home → memory.** Facts about the user, their people, and standing decisions are recorded through the memory system; a repo or zettel note may link to a memory topic but never copies its content out.

Every surface emits the same `note_submission` envelope for zettel-bound material, with the mandatory metadata the brain-dump pipeline already fixes: content hash + provenance spans (source system @ machine @ date). Placement does not create new envelopes or new pipelines.

## 5. ID-graph publishing (with #6062)

**Qualifying classes** (all three conditions): the note (a) is meant to be cited and related across systems, not just read; (b) has a stable identity worth addressing (programme records, published findings, cross-system objects, the Atlas corpus objects themselves); (c) has a writer-set that can be named under #5545. Everything else stays a file in zettel/repo/memory and is *projected* into Atlas per its tier.

**Identity and provenance.** An ID-native note is an ID-graph object: identity = its graph object id; provenance = the PROV-lite mapping #6062 defines (wasRevisionOf, specializationOf, wasDerivedFrom, wasGeneratedBy, wasAttributedTo) expressed in the #5686 relation vocabulary; authority = writer-set per #5545 (the creating agent's principal plus named promoters; human promotion is a writer-set act, mirroring staging → promotion in the zettel pipeline).

**Canonical vs projection.** The graph object is canonical for its relations and revision lineage; the human-readable text the author wrote lives as the object's first revision, derived from the zettel/repo original when one exists (wasDerivedFrom). Repo/zettel copies are thereafter *projections* for reading convenience and must link back to the graph object id; edits land on the graph object (new revision), not on the projection. This is the reverse of the #6062 default for Atlas *corpus* objects (canonical in Atlas tables, graph as projection) — deliberately: corpus objects are ingested records, ID-native notes are published objects. #6062's format design owns the object/relation mapping for both directions.

**Never ID-native:** memory content about the user (Tier 3, access-scoped), secrets or credential-shaped material (banned from every note home; the gh-post scan is the standing guard), ephemeral working state, and anything whose writer-set cannot be named.

## 6. Migration expectations for the existing sprawl

| Existing sprawl | Treatment |
|---|---|
| `research_notes/` and goal-workspace findings | **Grandfathered in place, swept opportunistically.** No bulk move. When a note is next cited or revised, it is promoted per §4 (usually to zettel) and the old copy becomes a pointer stub naming the new home. |
| Goal workspaces (`files/` deliverables, `hidden_files/` state) | **Left as-is.** Deliverables stay user-facing records; state stays machine state (Tier 3 / not ingested). Knowledge produced *inside* goal work follows §4 promotion like any other note. |
| Memory topics, people/group pages | **Not migrated.** Memory is a home in the table (§2), not sprawl. Atlas sees provenance only (Tier 3). |
| Sandbox-environment material (tool quirks, host notes) | **Stays in hatch-personal**, per the #5981/#5983 classification standard; everything else canonicalizes to code / zettel / ID as this table directs. |
| Repo `doc/` designs predating this rule | **Grandfathered.** The dated-design corpus is already the repo home §2 names; no renames or moves. |

The rule for all migration: **move on next touch, never big-bang.** A pointer stub is left at every promotion site so old links keep resolving.

## 7. Follow-on implementation (split out of this design)

Design only; implementation is filed as follow-on issues from this record:

1. Extend the `note_submission` envelope handling so a submission can declare its §2 class and intended tier (defaults: finding → zettel/Tier 1).
2. Add the §1 one-step test + §2 table to the fleet paperwork/lifecycle docs where agents file notes (pointer to this doc as canonical).
3. #6062 (already open) owns the ID-native object/relation format this design's §5 consumes.
4. A pointer-stub sweep tool for §6 promotions (records old → new home in the submission DB).

## 8. Decision summary

- Placement is decided by the orphan test (§1): world → zettel, artifact → repo, attempt → workspace/memory.
- Atlas ingests every home at a declared tier; Tier 0 (ID-native) is reserved for published, cross-system objects with a nameable writer-set.
- One envelope (`note_submission`), one system of record (Turso submission DB), staging → human promotion, nothing auto-commits — for every zettel-bound note regardless of surface.
- Migration is move-on-next-touch with pointer stubs; memory and sandbox-environment material do not move.
