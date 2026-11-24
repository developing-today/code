# Atlas

Atlas is the project's unified search system: one search engine over the
corpora Drew and his agents produce — AI-client conversations first, and,
through the same staging discipline, notes, repository documents, GitHub
comments, query traces and reports. Every stored unit keeps its provenance
(where it came from: the session, client, source reference, and commit or
observation date), every unit carries a sensitivity classification on a
four-step ladder enforced before ranking, and retrieval is hybrid: a
full-text (BM25) branch and a vector branch run over the same filtered
candidate set and their rankings are fused with reciprocal rank fusion. The
engine is a Rust library, `atlas-core`; the CLI, the service, the Cloudflare
Worker and the MCP server are surfaces over it. All chunks are embedded with
one model, Snowflake `snowflake-arctic-embed-m-v2.0` at 768 dimensions,
because vectors from different embedding models are not comparable.

## Current state (2026-11-23)

- Atlas is in production on Turso: the canonical merge and the Turso
  production publish both completed 2026-10-26, and Atlas migrated to the
  laptop, verified 2026-10-28.
- The corpus census is a frozen snapshot, dated wherever it is quoted: on
  2026-10-31 the corpus held 18,325 sessions, 122,124 messages and 107,639
  chunks. Counts have not been re-verified since, because ingestion is still
  manual — that freeze is the stated reason merged-list item 8 exists.
- Ingestion is the system's biggest gap: all corpus ingestion is manual and
  script-driven; the only Atlas schedule on the laptop is the daily
  `atlas-backup` timer. Continuous ingestion (#9097) is designed, with its
  acceptance unmet.
- Provenance is mid-rebuild: phase 1 and phase 2 slices of the #3125
  programme landed (12,092 files interned into the content-addressed store
  with verify proof on 2026-11-23); phases 3–4 are item 10 of the merged list.
- There is no learned reranker in the serving path today: production fusion
  is reciprocal-rank fusion over full-text and vector branches only, feedback
  is captured with no ranking consumer, and recommendation generation does
  not exist. Item 13 is designed against the golden-set gate (311 queries,
  version 3, plus 54 held out).
- ID has no live query path into Atlas today. Item 12, the Atlas↔ID
  integration, is designed and not started.

## Where its detail lives

- Book: the [Atlas book](https://developing-today.github.io/code/atlas/) —
  57 chapters, sources at `pkgs/atlas-search/book/`.
- This programme's slice: the Atlas book's [high-level
  roadmap](https://developing-today.github.io/code/atlas/design/high-level-roadmap.html)
  carries Atlas's share of the merged list (items 7–14 in full).
- Programme tracker: [#13241](https://github.com/developing-today/code/issues/13241).
- Merged-list items: 7–14 are Atlas-primary; Atlas also carries cross-links
  to items 6, 15, 17, 25, 26, 29 and 30.
