# Design: the knowledge layer

This chapter is design. It describes the knowledge layer as designed in the records of programme #5678 and its neighbours. The archive survey for this book homes 57 records in the combined knowledge-layer and first-apps theme; this chapter carries 31 of them — the records about notes, documents, tags, links, import, export, search and the knowledge programming model, including the worlds-adjacent library and programming-model work filed under the #5955 worlds programme. The other 26 records — the concrete first applications and the program ecosystem ladder — are in [Design: first apps and the ecosystem](first-apps.md). The split is stated in full in [Roadmap, issue map and the design-record index](roadmap-and-issue-map.md), where every one of the 57 records is listed exactly once.

Related chapters: [Design: first apps and the ecosystem](first-apps.md), [The file core: content addressing, names and tags](../concepts/file-core.md).

## The object model: stable notes, signed revisions, typed links

The governing record is `2026-11-01_design_id2048.md` (2026-11-01), the Zettelkasten layer design under epic #2056: multiplayer docs and wiki on ID. The decision it takes: a note is a stable logical object, its content is a sequence of signed immutable revisions, and its links are typed references — not filenames and not generic tags. The record rejects the two simpler options with reasons stated in the record: a `document.put` event cannot be the note, because the event ID changes on every save, so history does not group successive puts into one lineage and links cannot survive edits or renames; and filenames plus generic tags cannot define the note graph, because filenames are mutable locators and tag claims are signed observed-remove claims, not typed directed edges.

The substrate the design builds on is stated in the same record: signed, hash-identified events with a generic `kind` and a causal `after` frontier; document content as blobs; aliases as claims that are never silently selected when contested. Resolution follows the #2047 rule the record adopts: resolve within the viewer's configured stores, show provenance and ties, and persist the stable target ID plus the chosen surface form.

Three boundary statements in the #2048 record constrain everything built on it:

- Live ProseMirror collaboration is transient server state, keyed by filename, held in memory. Save persists a document blob and a signed put; live steps are not durable graph events. The design reuses the live transport and does not treat it as a replicated conflict-resolution model.
- A writer-set view is not a read access-control list. Valid events are stored; a view counts only its configured writer set. Shared use of the knowledge layer waits on a real event and blob access policy, which the record states the current sync wiring does not provide.
- Search at the design baseline covers tag metadata and excludes full-text search. The knowledge layer's search design starts from that baseline, not from a full-text index.

Supporting records: `2026-11-01T16-05-00Z_design_documents_session_frames` (documents as a session frame pair, for web and CLI parity), `2026-11-02T04-52-00Z_feature_note_link_backlink_projection` (note links and the head-based backlink projection — backlinks are a projection of the link graph at a head, not stored data), and `2026-11-09T03-57-04Z_design_id_edit_fork_graph` (the edit/fork graph for notes: forks are first-class graph objects with provenance, not copies).

## The #5678 programme cluster

Programme #5678 is the knowledge-layer programme. Its parentage shows in the records: #5842 (AI over notes) names #5678 as parent, and the cluster records #5843–#5848 and #6435 decompose it.

- `knowledge-layer-extensibility-5843.md` (undated in the archive; issue #5843): the extensibility cluster decision record — how the knowledge layer grows new object types and behaviours without changing the core model.
- `knowledge-layer-attachments-media-5844.md` (issue #5844): the attachments and media cluster — how media attaches to knowledge objects.
- `knowledge-layer-feature-matrix-6435.md` (issue #6435, observation date 2026-11-08): the feature matrix investigation. Its method is part of the design record: every cell carries an evidence type (primary, hands-on, secondary), product facts were observed from vendor documentation on 2026-11-08, unknowns are recorded as unknown with the gap named, and nothing is inferred from a competitor's category. One product in scope, Open Knowledge, is recorded as an explicit identity blocker: identity unconfirmed, most plausible candidate noted as unverified.
- `2026-11-10T20-12-00Z_design_id_content_model` (2026-11-10, cluster record for #5839): the content model decisions the importers and views consume.
- `2026-11-08_design_id5837.md`: the knowledge graph view cluster — local and global views, edges, filters, scale and live behaviour.
- `2026-11-08_design_id5832.md`: multiplayer edit sessions — user experience, lifecycle and participation (issue #5832).
- `2026-11-08T11-06-39Z_design_id_draft_approval_flow`: draft approval queues, outcomes, and rejected-draft forks — a rejected draft becomes a fork, not a deletion.
- `2026-11-08_decision_id5874_thread_resolution.md`: thread resolution and the discussion-to-edit loop — resolved-into links and change proposals connect a discussion thread to the edit it produced.
- `2026-11-08_design_id5830.md`: the note-to-board sharing model — embedded live references, version semantics, reply anchoring, backlinks, and the cross-server case.
- `2026-03-23T12-00-00Z_feature_metadata_tags` (2026-03-23): the metadata tag system, the earliest record in this chapter and the tag substrate the #2048 design builds on.

Sharing and permissions for the layer are designed in `2026-11-08T08-19-40Z_design_id_knowledge_sharing_permissions`, homed with the identity records in the survey and cross-linked here; its read-authority default is the one the AI record below inherits.

## Importers: five products, one object model

Every importer design targets the #2048 object model, so imported content lands as stable notes with revisions and typed links, not as files in a folder:

- `2026-11-08_design_id5845.md` — Obsidian importer (#5845).
- `2026-11-08_design_id5846.md` — Evernote importer (#5846).
- `2026-11-08_design_id5847.md` — Logseq importer (#5847).
- `2026-11-08_design_id5848.md` — Joplin importer (#5848). Continuous sync for Joplin is a separate design, `2026-11-12_design_id9004.md`, homed with the sync records.
- `2026-11-08T08-53-12Z_design_onenote_importer` — OneNote importer.
- `2026-11-10_design_id6436.md` — Notion importer (#6436). A shipped importer crate exists for Notion; its state is in the grounding section below.

The reverse direction is `2026-11-08_design_id6439_knowledge-interchange-and-export.md`: knowledge-layer interchange and full-fidelity export. Export is a design requirement of the layer, not an afterthought: the export must carry the graph — revisions, links, provenance — at full fidelity.

## Query, search and AI over notes

- `2026-11-09_design_id5821.md`: ID queries — result shaping and cursor pagination, a decision record. Query results are shaped server-side and paged by stable cursors.
- `2026-11-08T00-45-00Z_design_id_recommendations` (2026-11-08): recommendations across all object types, v1 design. Recommendations are a knowledge-layer service over the whole graph, not a per-app feature.
- Personalized search ranking for notes is decided in `2026-11-09T01-10-00Z_design_id_search_ranking`, homed with the shell records in the survey and cross-linked here and in [Design: shells and client surfaces](shells-surfaces.md).
- `2026-11-10T21-45-00Z_design_ai_over_notes` (2026-11-10, issue #5842, parent #5678): AI over notes — source-grounded question answering over a user's note corpus, summarisation, citation back to source, and generated audio overviews as a named possibility. The record inherits the parent programme's read-authority ruling verbatim: the default is public read; the owner may grant read to any number of groups and/or users; and the resolution order is object, then user default, then server config, then public — an object's own read setting wins, else the owner's user-level default, else server configuration, else public. The parent's binding corollary, quoted in the record: AI indexing and search must never expose content to a reader the resolution order would deny. The design applies it by treating the model as a reader, not a principal: an AI feature holds no read authority of its own; for each question the system computes the asker's read set, and retrieval, ranking, generation context and citations operate strictly inside that set. Filtering runs before ranking — the record states that rank-then-filter leaks through result counts, phrasing, timing and near-miss snippets — and every factual claim in an answer cites the note it came from.

Two records place agent notes, which is knowledge-layer policy for the programme's own agents: `2026-11-08T13-55-51Z_design_agent_note_placement` (where agent notes live — scratch, memory, repo, Zettelkasten, or ID graph) and `2026-11-10T20-00-00Z_design_agent_note_placement.md` (the placement rule for zettel against repo against ID graph, 2026-11-10).

## The worlds-adjacent programming model (#5955 cluster)

The #5955 worlds programme filed a cluster of records that the survey homes in this theme; they define how libraries and user code behave as knowledge the platform carries:

- `2026-11-05_design_id5965.md` (2026-11-05): compile economics — compiled-world and DLL caching, compile-cost penalties, premium gating.
- `2026-11-05_design_id5997.md` (2026-11-05): effect-language borrowings — which Koka and Unison properties ID tooling emulates, approximates, or skips. The record's three-way disposition (emulate, approximate, skip) is the design output.
- `2026-11-08T17-23-00Z_design_id5963_programming_model`: the programming model — explicit platform contexts for pure libraries.
- `2026-11-08_design_id5969.md`: library typing and upload taxonomy — subtypes, entity attachment, DLL against compile-in, libraries by URL.
- `2026-11-11_roc_library_evidence_5995.md` (2026-11-11): Roc shared-library outputs and host loading, evidence for #5995 — the measured base the library-import design in [Design: shells and client surfaces](shells-surfaces.md) consumes.

## What exists today

Grounding, from the surveys for this book (checkout `1ccb4a19457`, 2026-11-23):

- Shipped: the substrate the design builds on — signed document events, blob-backed content, tag claims, aliases, the web collaborative editor with transient live sessions and signed saves, and tag-metadata search. The #2048 record itself documents this baseline from the code.
- Shipped: one importer crate, `id-import-notion`, with a recorded-inventory integration test suite. It is a default workspace member with no dependents. The other five importer designs (Obsidian, Evernote, Logseq, Joplin, OneNote) have no shipped crate named in the surveys; treating any of them as built is unverified.
- Shipped in the file core: the did-it tracker module in the facade (`didit`) and the boards module (`boards`) — application modules, not the knowledge layer of this design.
- Still design: the knowledge layer as designed — stable logical notes with signed revision lineages as the note model (the #2048 record states implementation is split across #3814, #3816 and #3819 and includes no implementation), typed link and backlink projections as the link system, the #5678 cluster behaviours, full-fidelity interchange and export (#6439), recommendations as a graph service, and AI over notes (#5842, a design record whose status is draft). None of these is presented in this chapter as working today.
