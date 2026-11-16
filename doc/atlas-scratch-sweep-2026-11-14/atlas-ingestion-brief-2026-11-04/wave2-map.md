# Atlas Ingestion Brief — Wave-2 Disposition Map (2026-11-04)

Wave 2 folded the research lane's 46 proposed follow-ups (`prior-art-research.md`, Clusters 1–7 + cross-cutting) into the filed issue set (Lane C: tracker [#6018](https://github.com/developing-today/code/issues/6018) + children [#6019](https://github.com/developing-today/code/issues/6019)–[#6029](https://github.com/developing-today/code/issues/6029), provenance home [#3125](https://github.com/developing-today/code/issues/3125)).

**Counts: 46 items → 28 newly filed · 16 covered by a Lane C issue · 2 covered by a pre-existing issue. Unhomed remainder: ZERO.**

## Newly filed (28) — [#6035](https://github.com/developing-today/code/issues/6035)–[#6062](https://github.com/developing-today/code/issues/6062)

### Cluster 1 — Delta / offset embeds (parent [#6028](https://github.com/developing-today/code/issues/6028))

| Item                                 | Disposition                                                       |
| ------------------------------------ | ----------------------------------------------------------------- |
| 1 Span-table schema                  | NEW [#6035](https://github.com/developing-today/code/issues/6035) |
| 2 Offset-matching algorithm          | NEW [#6036](https://github.com/developing-today/code/issues/6036) |
| 4 Chunk-boundary policy v2           | NEW [#6037](https://github.com/developing-today/code/issues/6037) |
| 5 Boundary-drift measurement         | NEW [#6038](https://github.com/developing-today/code/issues/6038) |
| 6 Overlap budget experiment          | NEW [#6039](https://github.com/developing-today/code/issues/6039) |
| 7 CDC feasibility at embedding scale | NEW [#6040](https://github.com/developing-today/code/issues/6040) |

### Cluster 2 — Versioning / retention (parent [#6026](https://github.com/developing-today/code/issues/6026))

| Item                                         | Disposition                                                       |
| -------------------------------------------- | ----------------------------------------------------------------- |
| 11 Retention rule engine (dry-run reporter)  | NEW [#6041](https://github.com/developing-today/code/issues/6041) |
| 12 Cold-tier shard format                    | NEW [#6042](https://github.com/developing-today/code/issues/6042) |
| 14 PITR vs corpus-retention layer separation | NEW [#6043](https://github.com/developing-today/code/issues/6043) |

### Cluster 3 — Provenance

| Item                         | Disposition                                                                                                                                                                                                  |
| ---------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 16 Source-mark grammar       | NEW [#6044](https://github.com/developing-today/code/issues/6044) (parent [#6018](https://github.com/developing-today/code/issues/6018); home [#3125](https://github.com/developing-today/code/issues/3125)) |
| 19 Ingest attestation record | NEW [#6045](https://github.com/developing-today/code/issues/6045) (parent [#6019](https://github.com/developing-today/code/issues/6019))                                                                     |

### Cluster 4 — Stats / feedback (parent [#6027](https://github.com/developing-today/code/issues/6027))

| Item                              | Disposition                                                       |
| --------------------------------- | ----------------------------------------------------------------- |
| 24 Golden-set builder + eval gate | NEW [#6046](https://github.com/developing-today/code/issues/6046) |
| 25 Implicit-signal logging        | NEW [#6047](https://github.com/developing-today/code/issues/6047) |

### Cluster 5 — Classification (parent [#6021](https://github.com/developing-today/code/issues/6021))

| Item                               | Disposition                                                       |
| ---------------------------------- | ----------------------------------------------------------------- |
| 27 Presidio NER pass design        | NEW [#6048](https://github.com/developing-today/code/issues/6048) |
| 28 Personal-inventory pattern pack | NEW [#6049](https://github.com/developing-today/code/issues/6049) |
| 29 Trufflehog decision record      | NEW [#6050](https://github.com/developing-today/code/issues/6050) |
| 30 Revalidation cadence design     | NEW [#6051](https://github.com/developing-today/code/issues/6051) |

### Cluster 6 — Licensing gate (parent [#6024](https://github.com/developing-today/code/issues/6024))

| Item                                               | Disposition                                                       |
| -------------------------------------------------- | ----------------------------------------------------------------- |
| 31 External-content ingestion policy (three zones) | NEW [#6052](https://github.com/developing-today/code/issues/6052) |
| 32 License capture at ingest                       | NEW [#6053](https://github.com/developing-today/code/issues/6053) |
| 33 Deletion-propagation design                     | NEW [#6054](https://github.com/developing-today/code/issues/6054) |
| 34 Lawyer-question packet                          | NEW [#6055](https://github.com/developing-today/code/issues/6055) |
| 35 Copilot-litigation verification note            | NEW [#6056](https://github.com/developing-today/code/issues/6056) |

### Cluster 7 — Ingestion ops / cost (parent [#6019](https://github.com/developing-today/code/issues/6019))

| Item                               | Disposition                                                       |
| ---------------------------------- | ----------------------------------------------------------------- |
| 38 Staging-shard ingest gate       | NEW [#6057](https://github.com/developing-today/code/issues/6057) |
| 40 Embedding cost ledger           | NEW [#6058](https://github.com/developing-today/code/issues/6058) |
| 41 Secret-tier local embedder      | NEW [#6059](https://github.com/developing-today/code/issues/6059) |
| 42 Cross-tier vector comparability | NEW [#6060](https://github.com/developing-today/code/issues/6060) |
| 43 Storage growth model            | NEW [#6061](https://github.com/developing-today/code/issues/6061) |

### Cross-cutting

| Item                           | Disposition                                                                                                                                                          |
| ------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 45 ID-graph publication format | NEW [#6062](https://github.com/developing-today/code/issues/6062) (parent [#6022](https://github.com/developing-today/code/issues/6022); labels app/id, kind/design) |

## Covered by a Lane C issue (16) — no new issue

| Item                                       | Covered by                                                                                                                      | Note                                                                                                                                                                                                                                                                                                    |
| ------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 3 Reconstruction join semantics            | [#6028](https://github.com/developing-today/code/issues/6028)                                                                   | Core scope of the delta/offset design (interval-union form added by the findings comment)                                                                                                                                                                                                               |
| 8 Result-level dedup rules                 | [#6026](https://github.com/developing-today/code/issues/6026) (+ [#5733](https://github.com/developing-today/code/issues/5733)) | #6026 names merge/rescore treatment of multi-version hits; #5733 is the rescore-time Jaccard mechanism                                                                                                                                                                                                  |
| 9 File-lineage graph schema                | [#6026](https://github.com/developing-today/code/issues/6026)                                                                   | Core of the versioning design ("version model, lineage rules")                                                                                                                                                                                                                                          |
| 10 Newest-version resolution view          | [#6026](https://github.com/developing-today/code/issues/6026)                                                                   | Requirement 3 of #6026 ("the max"); query-time-view form added by the findings comment                                                                                                                                                                                                                  |
| 13 Commit-discipline convention + fallback | [#6026](https://github.com/developing-today/code/issues/6026)                                                                   | #6026 sets commit-hygiene expectations + segment-association fallback                                                                                                                                                                                                                                   |
| 17 Existing-embed provenance audit         | [#6029](https://github.com/developing-today/code/issues/6029)                                                                   | Exactly the Lane C audit task                                                                                                                                                                                                                                                                           |
| 20 Query trace schema                      | [#6027](https://github.com/developing-today/code/issues/6027)                                                                   | Core of #6027; log-at-query-time constraint added by the findings comment                                                                                                                                                                                                                               |
| 21 Top-K survival report                   | [#6027](https://github.com/developing-today/code/issues/6027)                                                                   | #6027's stats aggregation model (shard survival + top 1/2/3/5)                                                                                                                                                                                                                                          |
| 22 Feedback capture v1                     | [#6027](https://github.com/developing-today/code/issues/6027)                                                                   | #6027 requirements 3–5 (thumbs + optional comment on all searches)                                                                                                                                                                                                                                      |
| 23 Feedback normalization pass             | [#6027](https://github.com/developing-today/code/issues/6027)                                                                   | #6027 design question (essay normalization is our job); judge+rubric+calibration detail in the findings comment                                                                                                                                                                                         |
| 26 Tier-classification scanner v1          | [#6021](https://github.com/developing-today/code/issues/6021)                                                                   | #6021 IS the scan execution task; the reusable layers (NER pass, inventory pack, cadence) split out as #6048/#6049/#6051                                                                                                                                                                                |
| 36 Source-connector inventory              | [#6019](https://github.com/developing-today/code/issues/6019)                                                                   | #6019's deliverable includes the per-source trigger table                                                                                                                                                                                                                                               |
| 37 Manifest-first delta protocol           | [#6028](https://github.com/developing-today/code/issues/6028)                                                                   | #6028 design question 1 (change detection per source + its cost ceiling); also cited in the #6019 findings comment                                                                                                                                                                                      |
| 39 I/O politeness budget                   | [#6019](https://github.com/developing-today/code/issues/6019)                                                                   | #6019 requirement 3 (disk/IO monitoring + throttle/pause/report)                                                                                                                                                                                                                                        |
| 44 Agent-notes placement decision          | [#6022](https://github.com/developing-today/code/issues/6022)                                                                   | Exactly the Lane C design. Research doc flagged "tile cast" unresolved; Lane A's inventory resolved it as the Zettelkasten (dezren39/zettel) — that reading governs                                                                                                                                     |
| 46 First-pass corpus build plan            | [#6025](https://github.com/developing-today/code/issues/6025) (+ [#6023](https://github.com/developing-today/code/issues/6023)) | #6025 is the first-pass task; comments stage rides #6023 under the zone policy (#6052). Interpretation note: the research doc read "can repo" as the hatch tree; Lane A read it as dezren39/cairn and #6025 enumerates cairn — the cairn reading governs; the doc's stage sequencing applies either way |

## Covered by a pre-existing issue (2) — substantive research comments posted

| Item                                      | Covered by                                                    | Comment                                                                                                                                                                                               |
| ----------------------------------------- | ------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 15 Atlas provenance record v1 (PROV-lite) | [#3125](https://github.com/developing-today/code/issues/3125) | [comment 6650537113](https://github.com/developing-today/code/issues/3125#issuecomment-6650537113) — PROV-lite 8-field set, `wasRevisionOf`/`specializationOf`, OpenLineage run record, in-toto shape |
| 18 Dedup-edge pipeline (MinHash/LSH)      | [#5733](https://github.com/developing-today/code/issues/5733) | [comment 6650537719](https://github.com/developing-today/code/issues/5733#issuecomment-6650537719) — three-stage pipeline, normalizer-version discipline, cosine-as-mechanism dead end                |

## Findings comments (wave-2 step B) — all six posted

| Issue                                                                        | Substance                                                                                                                                                                                                                                                                                                                                       | Comment                                                                                    |
| ---------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------ |
| [#6024](https://github.com/developing-today/code/issues/6024) licensing GATE | **Answered-provisional**: D.4 → GitHub only; D.5 → through-the-Service/by-forking only; D.6 contested for comment text; comments NOT Creative Commons; three zones; external design proceeds on zone rules, Zones 2–3 full-text build stays personal-search-only; lawyer packet = [#6055](https://github.com/developing-today/code/issues/6055) | [6650532889](https://github.com/developing-today/code/issues/6024#issuecomment-6650532889) |
| [#6028](https://github.com/developing-today/code/issues/6028) delta/offset   | CDC refuted at embed scale (embeddings aren't hashes); span model = rsync token matching + restic-style ordered span refs + interval-union reconstruction                                                                                                                                                                                       | [6650533446](https://github.com/developing-today/code/issues/6028#issuecomment-6650533446) |
| [#6026](https://github.com/developing-today/code/issues/6026) versioning     | Git records NO renames (per-diff re-guess, 50% default, documented failure modes) — Atlas stores its own lineage edges; newest-resolution = query-time view, never destructive replace                                                                                                                                                          | [6650534035](https://github.com/developing-today/code/issues/6026#issuecomment-6650534035) |
| [#6027](https://github.com/developing-today/code/issues/6027) stats/feedback | Traces logged AT QUERY TIME (query_id, raw+rewritten, snapshot ids, per-candidate per-stage score/rank, embed version id); thumbs sparse/negativity-biased → pair with implicit signals; negative-feedback queries → versioned golden set                                                                                                       | [6650534885](https://github.com/developing-today/code/issues/6027#issuecomment-6650534885) |
| [#6021](https://github.com/developing-today/code/issues/6021) classification | Three-layer scan (gitleaks full → Presidio scoped → inventory pack); trufflehog AGPL + verification-exfiltration warning; findings quarantine upward, never silently delete; tier enforced at fan-out pre-merge                                                                                                                                 | [6650535509](https://github.com/developing-today/code/issues/6021#issuecomment-6650535509) |
| [#6019](https://github.com/developing-today/code/issues/6019) ingestion ops  | Cost model (full re-embed $0.27–$7; churn is the driver); ledger metric = delta reuse ratio (<90% on append-mostly = broken delta protocol); secret tier embedded LOCALLY regardless of price; staging-shard ingest gate                                                                                                                        | [6650536353](https://github.com/developing-today/code/issues/6019#issuecomment-6650536353) |

## Cross-link / tracker comments

- Parent cross-links are merged into the findings comments above for #6024/#6028/#6026/#6027/#6021/#6019 (one comment per parent).
- [#6022](https://github.com/developing-today/code/issues/6022) cross-link for its new child #6062: [comment 6650538465](https://github.com/developing-today/code/issues/6022#issuecomment-6650538465).
- Tracker [#6018](https://github.com/developing-today/code/issues/6018) child list extended (Lane C kept the child list in a comment; followed): [comment 6650539438](https://github.com/developing-today/code/issues/6018#issuecomment-6650539438) — all 28 new children grouped by cluster + the covered dispositions.
- Item 16's parent is the tracker itself (its implementation home is #3125, named in the issue body and in the #3125 comment).

**Unhomed remainder: ZERO.** All 46 research follow-ups have a disposition above; all six step-B findings comments are posted with URLs.
