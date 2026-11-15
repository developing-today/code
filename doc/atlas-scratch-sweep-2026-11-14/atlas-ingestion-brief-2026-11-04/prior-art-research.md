# Atlas ingestion program — prior-art & technical grounding

**Date:** 2026-11-04 (CST) · **Lane:** research (no issues filed from this doc; a later wave folds the follow-up list in)
**Brief:** Drew's 2026-11-04 Atlas ingestion dictation — semi-automated ingestion, delta-only updates with offset embeds, provenance/versioning, retrieval analytics + feedback, classification revalidation, GitHub-comment extraction with a licensing gate.
**Method:** primary sources where it matters (GitHub ToS read in full text, 2026-04-27 effective version; Turso docs; SLSA spec; git's own diffcore documentation), tool docs and engineering write-ups elsewhere. URLs cited inline. Speculation is labeled. Standing Atlas facts (from the program record, not re-derived here): ~107,639 chunks, 768-dim, internal + secret Turso shards, fan-out search with merge + rescore, cold tiers on rustfs, Jaccard rescore design at developing-today/code#5733, parameter surface at #5774.

---

## Cluster 1 — Delta ingestion + offset embeds (the technical core)

### Findings

**The brief's offset-embed idea is a known, proven shape — it is the rsync/restic span model applied to embeddings.** Drew's design ("this embed, joined to its most recent file, covers index 17–241 because 0–16 matched; everything after 241 matched a different one; search returns the full paragraph but the join doesn't double up the merged lines") is exactly how two mature families of systems describe a new file version:

- **rsync deltas.** Tridgell & Mackerras (TR-CS-96-05, 1996): the new file is expressed as a token stream — literal bytes where nothing matched, copy-references (block index + length) where it did. The matcher slides a window **one byte at a time** across the new file computing a cheap rolling (Adler-32-derived) checksum; only on a weak hit does it pay for the strong checksum. That sliding-window match **at arbitrary offsets** is precisely the mechanism that computes "0–16 matched old span A, 17–241 is new, 242+ matched old span B" without requiring any chunk boundaries to line up. (http://oceanpark.com/webmuseum/rsync.html)
- **Content-addressed span lists.** restic/borg store a file as an _ordered list of chunk references_; a new version reuses every chunk whose hash already exists and stores only the rest. A file version in Atlas can likewise be an ordered list of span references `(embed_id, start, end)`; the "version" costs only its genuinely new spans plus a manifest row.

**Content-defined chunking (CDC) is the alternative way to get boundary stability — and it mostly doesn't apply at embedding scale.** CDC (LBFS, Muthitacharoen/Chen/Mazières 2001: Rabin fingerprint over a 48-byte window, cut when `fp mod D == r`) makes boundaries a function of content, so an insertion only disturbs nearby chunks and boundaries re-synchronize downstream. FastCDC (Xia et al., USENIX ATC '16) — Gear rolling hash + cut-point skipping + normalized chunking — runs ~10× faster than Rabin CDC at equal-or-better dedup; a 2024 nine-algorithm evaluation confirms Gear/FastCDC as the practical pick. restic uses Rabin CDC with a 64-byte window and a **per-repository random polynomial**, min 512 KiB / avg 1 MiB / max 8 MiB; Borg 2 defaults to FastCDC. (https://github.com/wjordan/go-binsync/blob/HEAD/docs/research/cdc-cas.md, https://terbium.io/2019/02/content-defined-chunking/, http://umatechnology.org/content-defined-chunking-can-help-backups-reuse-unchanged-data/)

Why CDC is the wrong tool for the embed layer, despite being right for the storage layer:

1. **Scale mismatch.** Backup CDC targets ~1 MiB chunks; embedding chunks are ~256–512 _tokens_ (~1–2 KB). At that size, min/max constraints dominate, boundary placement is mostly forced, and the dedup CDC buys is small — while every boundary decision change re-embeds text.
2. **Embeddings are not hashes.** A stored chunk in restic is _identical bytes_; an embedding is a function of the chunk's exact text **including its neighbors inside the window**. The same paragraph embedded with different surrounding text is a _different vector_. CDC can make the bytes line up; it cannot make yesterday's vector the right vector for today's window. Drew's span model handles this honestly: the span's stored text _is_ what was embedded, and spans are reused only when the text is byte-identical.
3. **CDC does badly where content shifts structurally.** A controlled experiment on a 25.6 MB compiled binary: a +15-byte insertion invalidated 57% of 64 KiB FastCDC chunks (same-length edits: 2 chunks). Atlas's corpus is text (markdown, source, logs), where CDC resync works well — but session/chat logs with prepended headers and re-wrapped lines behave more like the binary case. (go-binsync doc, above)

**Search-time reconstruction without double-counting** falls out of the span model with one rule: results carry their own text (self-contained, so a hit reads correctly alone), and _presentation/join_ deduplicates by **interval union per file version** — sort the returned spans of one file version by start offset, merge overlapping/adjacent intervals, emit each byte once. This is the same move production RAG makes with parent-child ("small-to-big") retrieval: embed small precise units, return the larger coherent parent — except Atlas can compute the parent exactly from offsets instead of storing a second chunk tier. (https://github.com/prateekkk26/engineering-map/blob/HEAD/docs/ai/rag-and-retrieval/chunking-strategies.md)

**Chunk-boundary evidence from RAG practice** (for the spans Atlas _does_ create fresh):

- Fixed-size 256–512 tokens with 10–20% overlap is the baseline; **structural** splitting (markdown headings, code definitions, log-entry boundaries) beats both fixed and semantic splitting on technical corpora, because authors already segmented the text into coherent units. Recursive (paragraph→sentence→token) is the strong cheap default; semantic chunking costs an extra embedding pass, over-fragments, and benchmarks _below_ recursive in at least one 2026 guide comparison (69% accuracy for recursive-512 vs lower for semantic — secondary source, treat as directional). (https://dev.to/dishant_sethi/rag-pipeline-chunking-strategies-split-documents-for-better-retrieval-aoe)
- Overlap past ~20% is mostly waste: index inflation and near-duplicate hits that must be deduped at retrieval anyway. Structural boundaries need less overlap than blind windows.
- **Metadata must be captured at ingest** — source, section path, date, ACL/tier. "Metadata is what makes filtering, citation, and access control possible, and it is much harder to backfill than to capture during ingestion." This is the empirical backbone under the brief's provenance demands (Cluster 3).

**What breaks when chunk boundaries don't align across versions** (the failure catalog the design must answer):

1. _Vector drift on identical content_ — same paragraph, different window → different vector → yesterday's "known good" hit silently changes rank.
2. _Top-K crowding_ — v1 and v2 chunks of the same paragraph are near-duplicates; both surface, diverse results get pushed out. Needs result-level dedup (Cluster 1 follow-up 8) or version filtering (Cluster 2).
3. _Phantom diffs_ — a pure re-wrap/reflow (markdown hard-wrap change) produces all-new fixed-window chunks while changing no content; span matching on normalized text or CDC at the _storage_ layer absorbs this, fixed windows don't.
4. _Orphan spans_ — a span reused into a new version whose surrounding context changed meaning (the paragraph now sits under a "Deprecated:" heading). Span text is honest; span _context_ (heading path) is metadata and must version with the reference, not the span.

### Pros / cons

| Approach                                             | Pros                                                                                                               | Cons                                                                                                                                                                     |
| ---------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Offset spans + rsync-style matcher (brief's design)  | Exact reuse at any offset; honest vectors; file versions are cheap manifests; reconstruction join is deterministic | Matcher + span bookkeeping is new code; span store ≠ today's chunk store (migration); tiny edits still create a new span for the changed region (correct, but more rows) |
| CDC chunking at embed scale                          | Proven dedup machinery; boundary resync                                                                            | Wrong scale (see above); vector-drift problem untouched; re-chunking changes embed identity wholesale                                                                    |
| Fixed windows + whole-file re-embed on change        | Dead simple; today's behavior                                                                                      | The 21 GB → re-embed-everything cost the brief explicitly rejects; phantom diffs on reflow                                                                               |
| CDC at storage layer + spans at embed layer (hybrid) | Storage dedup for blobs/backups AND honest embed spans                                                             | Two systems to keep consistent; only worth it if blob storage becomes a cost driver                                                                                      |

### Dead ends

- **Using embedding cosine similarity as the dedup/lineage mechanism.** Cosine catches paraphrase, misses exact lineage, and is O(n²)-ish without an index. Dedup is a hashing problem (Cluster 3); embeddings are the third signal, not the mechanism — consistent with Drew's own Jaccard conclusion in the parent conversation.
- **Keyed/per-repo CDC polynomials (restic-style) for Atlas spans.** restic randomizes its polynomial per repository so chunk boundaries don't leak across stores — the opposite of what Atlas wants, since cross-source span matching is the point.
- **Trying to make old vectors valid for new windows** (averaging, stitching vectors). No sound basis; a vector is of a text, not of a position.

---

## Cluster 2 — Content versioning + retention

### Findings

**Git does not record renames; it re-guesses them at diff time, and the guesses have known failure modes.** Git stores snapshots, not lineage. Rename detection is a heuristic run per-diff: exact-content matches first, then a same-basename pre-pass, then pairwise similarity scoring (similarity index = shared content fraction; **default threshold 50%**, `-M50%`) against unmatched delete/add pairs, with a rename limit that silently caps the quadratic step on big diffs; copies need `-C`, and copies from unmodified files need `--find-copies-harder`. Documented failure modes: rename + heavy edit in the same commit (similarity drops under threshold → reads as delete+create); file splits and merges (whole-file similarity can't see one function move into a larger file); threshold configured to 100 is silently treated as 99 in some implementations. Drew's instinct is right — "if commits are done in the right way, renames show up" (rename-only commits, `git mv`, no simultaneous edit) — but that makes git lineage a _convention-dependent hint_, not a substrate. **Atlas must compute its own lineage edges** (via segment search, Cluster 3) and store them as first-class data; git's `-M` output is one input signal among several. (https://www.spinics.net/lists/git/msg400439.html — gitdiffcore doc patch describing the basename pre-pass and its false-rename risk; https://jira.atlassian.com/browse/BSERV-13531)

**Versioned-data systems converge on one retention shape: keep everything until a policy-driven vacuum, and the vacuum is where time travel dies.** Delta Lake: every write is a version; `VACUUM` deletes files unreferenced by the current log once they're older than `delta.deletedFileRetentionDuration` (**default 7 days**; log retention separately defaults to 30 days). Iceberg: `expire_snapshots` with `older_than` + `retain_last_n`, plus `remove_orphan_files`. The documented trap: after vacuum, old versions still appear in the log but fail to read — history becomes a list of tombstones. lakeFS applies git semantics (branch/commit/merge) over object storage with **policy garbage collection that only deletes unreferenced objects** — including the write-audit-publish pattern: ingest onto a branch, validate, merge to main only if checks pass. DVC is the file-level content-addressing version of the same idea (hash-named cache, small pointer files in git). (http://docs.databricks.com/aws/en/sql/language-manual/delta-vacuum, https://www.aicassindra.com/blogs/ai/ai_data_versioning.html, https://lakefs.io/blog/git-like-data-versioning-meets-mlops-lakefs-with-mlflow-datachain-neptune-quilt/)

**The brief's three retention knobs (age / supersede-count / access) each have a production precedent, and the composite is standard practice in backup retention:**

- _Age:_ S3 lifecycle transitions/expirations; Delta/Iceberg time-based expiry.
- _Supersede-count:_ Iceberg `retain_last_n`; restic's grandfather-father-son `forget --keep-daily 7 --keep-weekly 4 --keep-monthly 12 --prune` is the same "keep N per period bucket" thinning applied to snapshots — and sanoid-style ZFS pruning is the filesystem twin. (https://tech.weburz.com/blog/automated-restic-backups/)
- _Access-frequency:_ S3 Intelligent-Tiering moves objects by observed access; Atlas has a _better_ signal available — its own query traces (Cluster 4) record exactly which versions ever surface. "Never in any top-K in 180 days + superseded ≥3 times" is a defensible cold-tier rule that no generic storage system can express.

**"Take the max / newest version" must be a query-time view, not a destructive replace.** Delta/Iceberg time travel (`VERSION AS OF`, `TIMESTAMP AS OF`) works because resolution happens at read. For Atlas: newest-version resolution = a view over the lineage graph (per logical file: the version reachable from the current head edge), with explicit handling for deleted files (head = tombstone, not "previous version resurfaces") and renamed paths (identity follows the lineage edge, not the path string). Destructive replacement would make Cluster 4's traces unresolvable — a served result must forever resolve to the exact embed state that was served.

**Turso's own point-in-time features are a different layer — useful, but not corpus retention.** Turso Cloud PITR: restore/branch a database at any timestamp within a plan window (**24 h free; 10/30/90 days by paid tier**), implemented as shared generations + WAL fragments, branching is metadata-only copy-on-write. (https://docs.turso.tech/features/point-in-time-recovery, https://turso.tech/blog/how-does-the-turso-cloud-keep-your-data-durable-and-safe) That is disaster recovery for the _database_; it does not give per-file version semantics, and its windows (≤90 days) are far shorter than corpus retention horizons. Conversely, Atlas's cold-tier moves to rustfs are corpus-level and permanent-policy-driven. Keep the layers separate: PITR = "the shard DB broke"; Atlas retention = "this file version aged out." One caveat from the Turso docs: restores create a _new_ database and count against the plan's database quota — relevant to the shard-count economics already in the program record.

### Pros / cons

| Pattern                                                                              | Pros                                                                                                    | Cons                                                                                                            |
| ------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| Lineage graph in Atlas's own tables (path identity + content identity + typed edges) | Queryable by the same fan-out; feeds stats/feedback; survives non-git sources (chats, logs have no git) | Atlas must build/maintain it; edge quality depends on Cluster 3 matching                                        |
| Rely on git rename detection                                                         | Zero build; right most of the time with commit discipline                                               | Heuristic, per-diff, threshold-sensitive; nothing for non-git sources; not recorded anywhere queryable          |
| Delta/Iceberg-style table snapshots for the chunk store                              | Battle-tested version+vacuum semantics                                                                  | Atlas is libSQL shards, not a lakehouse; adopting the format is a platform change out of proportion to the need |
| lakeFS-style branch staging for ingest batches                                       | Write-audit-publish maps exactly onto "classify before it lands" (Cluster 5)                            | lakeFS itself is another service; borrow the _pattern_ (staging shard → validate → publish), not the product    |

### Dead ends

- **Time-travel-as-retention via Turso PITR.** 90-day ceiling, whole-database granularity, restore-to-new-DB mechanics. Wrong layer.
- **Path-as-identity.** The brief's own rename discussion kills it: a renamed file is the same logical file; a new file at an old path is not. Identity must be a node; paths are versioned attributes of it.
- **Vacuum without a dry-run reporter.** Every system above deletes silently once policy fires. For a personal corpus, retention runs should report first ("these 412 versions would cold-tier, these 38 would delete") until the policy has earned trust.

---

## Cluster 3 — Provenance standards

### Findings

**W3C PROV is the right vocabulary, and two of its relations are literally Drew's "past and future selves."** PROV (W3C Recommendation, 2013) models **Entity** (a thing in a state — a file version, a chunk), **Activity** (ingest run, transformation, merge), **Agent** (Drew, an agent process, the ingester build). Core relations: `wasGeneratedBy`, `used`, `wasDerivedFrom`, `wasAttributedTo` — plus the expanded terms that matter here: **`wasRevisionOf`** (new file version ↔ prior version) and **`specializationOf`** (a version ↔ the timeless logical file). An Atlas provenance record needs maybe eight fields, not the full ontology: entity id, derived-from ids, generated-by run id, attributed-to agent, timestamps, source mark, content hash, span offsets. PROV's value is that these names and directions are already standardized, so the ID-graph publication (a brief question) inherits a defensible model instead of an invented one. (https://www.w3.org/TR/prov-o/, summarized at https://github.com/elliotvness/master-rack/blob/HEAD/src/research/research-cpq-bom-provenance.md)

**OpenLineage supplies the pipeline-event shape.** Where PROV is per-artifact, OpenLineage models **Job / Run / Dataset** with facets (`sourceCodeLocation`, `version`, `nominalTime`, `parent`) — i.e., exactly the record an ingest batch should emit: this run of this ingester consumed these source snapshots and produced these shard writes. Adopting its field names for the ingest-run record makes the run log interoperable with lineage tooling (Marquez etc.) if that ever matters, at zero present cost. (https://openlineage.io/docs/spec/object-model/)

**in-toto / SLSA show how to bind a claim to content.** An in-toto attestation = envelope + **subject** (artifacts identified by digest) + **predicate** (typed claim). SLSA's provenance predicate: builder identity, build parameters, and **materials** — the input artifacts _with their digests_. Translated to ingestion: each batch gets an attestation-shaped record — subject = the chunk/span hashes written; materials = the source files' hashes + source marks; builder = ingester name+version (+ git hash, per the fleet's own versioning ruling); predicate extras = classification tier and scan results (Cluster 5). Signing is optional for a personal system; the _shape_ is the value — it makes "where did this embed come from" a record lookup, not an archaeology project. (https://slsa.dev/spec/v0.1/provenance, https://sbomify.com/2024/08/14/what-is-in-toto/)

**Source marks: the brief's `repo@hash:path` and "OpenCodeDB@AMD-laptop on this day" are composite marks, and there are proven grammars to steal.**

- Git itself: blob SHA = content identity, commit SHA = tree state; `repo@commit:path` is unambiguous and resolvable forever (as long as the repo survives — hence keeping the content hash _in the mark_ too).
- Go module pseudo-versions (`v0.0.0-20261104120000-abcdef123456`) prove a compact `time+commit` encoding works at ecosystem scale.
- OCI image digests (`name@sha256:…`) prove the `locator@digest` shape.
- For non-git sources the mark needs `(system, host, locator, captured-at, content-hash)` — Drew's "OpenCodeDB@AMD on this day" is that tuple in prose; the design work is making it one parseable grammar with one formatter, so the Cluster-3 audit ("scan all existing embeds and determine if they use the appropriate path or details") is a parse-and-validate job, not a regex safari.

**Duplicate / near-duplicate association is a solved pipeline, in three stages of increasing cost:**

1. **Exact:** SHA-256 of normalized text → identical-content groups. Free.
2. **Candidate generation:** shingling (word n-grams, 5–9 tokens) + **MinHash** signatures (Broder 1997; 128–256 permutations) + **LSH banding** tuned to Jaccard ≈ 0.8 → candidate pairs in near-linear time. This is the de facto standard from web-corpus and LLM-training-data dedup (Lee et al. 2022; Llama 3's pipeline). (https://github.com/mekkcyber/llm-atlas/blob/HEAD/data/deduplication.md)
3. **Verification + edges:** compute true Jaccard on candidates only; record a typed **dedup edge** `(chunk_a, chunk_b, jaccard, method, run_id)`; cluster by connected components with one canonical member per cluster and a discarded→canonical map. (https://www.valtersit.com/python/data-deduplication-pipeline-with-minhash-lsh/)

This is the concrete machinery under the existing Atlas Jaccard work (#5733): #5733 uses Jaccard at _rescore_ time (query-side); this pipeline uses it at _ingest_ time (corpus-side, lineage edges). Same math, different table — the design should say so explicitly so the two don't grow into rival implementations. SimHash is the alternative near-dup hash (better for very short texts where shingle sets are tiny); worth one comparison experiment, not a commitment. Preprocessing dominates outcomes — tokenization, lowercasing, boilerplate stripping move Jaccard more than LSH parameters do — so the normalization function is part of the provenance record (a Jaccard edge is only meaningful with the normalizer version that produced it).

### Pros / cons

| Choice                                         | Pros                                                                 | Cons                                                                                                                  |
| ---------------------------------------------- | -------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------- |
| PROV-lite field set (8 fields, standard names) | Standard vocabulary; maps to ID graph later; tiny                    | Discipline cost: every ingest path must emit it, forever                                                              |
| Full PROV-O / RDF serialization                | Maximal interoperability                                             | Heavy machinery for a personal corpus — **dead end** (below)                                                          |
| in-toto-shaped batch attestations (unsigned)   | Binds chunks→sources→ingester in one record; audit-ready             | One more record type to version                                                                                       |
| MinHash/LSH ingest-time dedup edges            | Near-linear; proven at web scale; directly feeds lineage + retention | Threshold tuning is corpus-specific; false edges possible → edges must carry method+score so they can be re-cut later |

### Dead ends

- **Full W3C PROV-O ontology deployment.** Multiple practitioners converge on "learn the vocabulary, skip the ontology" — provenance is evidence about production, and a 200-class RDF model produces metadata nobody reads. (https://john-hodge.com/blog/data-provenance-agentic-ai/)
- **Embeddings as the dedup mechanism** (see Cluster 1).
- **Recording provenance only in git commit messages / file headers.** Doesn't survive copies, chats, logs, or DB rows; provenance must live in Atlas's tables beside the chunks it describes.

---

## Cluster 4 — Retrieval analytics + feedback

### Findings

**The trace Drew specced (scores at each stage, shard of origin, exact embed state, metadata) is implementable as one append-only query log — and it cannot be retrofitted.** The fields that make a result reconstructible later: `query_id`, timestamp, querier identity/context, raw query + rewritten query (Atlas's dual-submission rewriter makes these differ), corpus snapshot id (per-shard head markers), and per candidate: embed id + **embed version id**, shard, per-stage (score, rank) triples for fan-out → merge → rescore, final rank, and which K cutoffs it survived. Production RAG observability converged on the same shape as _span tracing_: TruLens instruments apps with feedback functions over traced runs; Arize Phoenix / OpenInference define retrieval spans; Langfuse (open-source, self-hostable) is the trace-store pattern. Atlas doesn't need those products — it needs their _schema discipline_: every stage emits (inputs, scores, outputs) against a trace id, at query time, because stage scores are not recomputable after the corpus moves. (https://github.com/handbook-academy/engineering-handbook/blob/HEAD/content/hld/part-9-ai-ml-system-design/05-llm-evaluation-observability.md)

**Top-K survival stats are funnel analytics over that log.** "Came in from a shard → survived merge → survived rescore → landed top 1/2/3/5" is a per-shard, per-tier, per-source-type funnel computed by aggregation, not a separate system. Its uses are exactly the brief's: which shards earn their fan-out cost, which sources produce chunks that never survive (retention input, Cluster 2), and whether the rescore stage (Jaccard #5733, SPLADE eval #5741) is actually changing outcomes or just re-sorting noise. One design requirement falls out: ranks must be logged at _each_ stage for the full candidate list, not just final top-K — survival at K=3 is only computable if stage outputs were recorded deeper than 3.

**Explicit feedback: thumbs are cheap ground truth; the machinery around them is what matters.**

- Practitioner consensus: explicit user feedback (thumbs up/down) is "the cheapest ground truth you will ever get"; pair it with implicit signals (copy-to-clipboard ≈ success, regenerate/requery ≈ failure, dwell) because explicit feedback is sparse and negatively biased. (https://www.aiworkshack.com/tools/rag-evaluation/beyond-ragas-a-practical-guide-to-evaluating-rag-and-agent-pipelines.html)
- Drew's "thank them for the essay" instinct has a concrete pipeline form: feedback (rating + optional comment) lands in an annotation queue (Label Studio / Argilla are the reference tools); **negative-feedback queries get promoted into the evaluation set with corrected ground truth** — "when users flag incorrect responses, add those queries to your evaluation set" is the standard loop. (https://particula.tech/blog/rag-evaluation-retrieval-accuracy-testing)
- **Normalizing free-text comments** (Drew: "we can go over the comments and normalize them ourselves based on context and increased detail"): the current practice is LLM-as-judge with an explicit rubric + chain-of-thought before the score, _calibrated_ by periodically hand-scoring a sample and measuring judge agreement; an uncalibrated judge drifts silently. The judge's labels (wrong-source / stale-version / right-doc-wrong-span / missing-doc / ranking-only …) are the taxonomy that turns essays into statistics. (aiworkshack guide, above)
- Result IDs: results already need stable IDs for feedback binding; the binding must be `(query_id, result_id, embed_version_id)` — feedback against a _version_, or retention/versioning changes make old feedback uninterpretable.

**Offline eval is the other half: a golden set that gates changes.** RAGAS (faithfulness, answer relevancy, context precision, context recall — reference-free) and the TruLens RAG triad (context relevance, groundedness, answer relevance) are the standard offline metrics; the standard operating pattern is a versioned golden set (50–200 real queries) run on every index/config change with merge gates on thresholds. For Atlas the golden set is what converts feedback + traces into decisions: chunking changes (Cluster 1), rescore weights (the #5774 parameter surface), and SPLADE/Jaccard experiments all become "did the golden set move" questions instead of vibes. (https://www.opodab.com/2026/07/rag-python-tutorial-stop-building-demos.html)

**Learning-to-rank is a v3+ topic with a known on-ramp.** Online interleaving (alternate A/B results in one list, infer preference from clicks) and click models (correcting position bias) are how production search learns from implicit feedback without fooling itself. Both require exactly the trace log above plus months of volume — at personal scale, explicit thumbs + golden-set gating will dominate for a long time. Log now, learn later.

### Pros / cons

| Practice                                       | Pros                                                                           | Cons                                                                                                                      |
| ---------------------------------------------- | ------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------- |
| Append-only query trace log (per-stage scores) | Enables survival stats, feedback binding, regression forensics; cheap to write | Storage grows with query volume; needs its own retention (it's also Atlas input — session/stats ingestion, per the brief) |
| Thumbs on every result (+ optional comment)    | Zero-friction; binds to exact version                                          | Sparse, negativity-biased; needs implicit signals + golden set to stay honest                                             |
| LLM-judge normalization of comments            | Turns essays into a label taxonomy                                             | Judge drift; requires periodic human calibration samples                                                                  |
| RAGAS-style golden-set gates                   | Makes index evolution empirical                                                | Golden set must be maintained; metrics are proxies, not truth                                                             |

### Dead ends

- **Recomputing historical traces after corpus changes.** Impossible by construction; any design that doesn't log at query time has no analytics. This is the cluster's one hard constraint.
- **Raw thumbs-count as a ranking signal without bias correction.** Position bias + negativity bias make naive counts misleading; fine for triage, wrong for auto-reranking at personal scale.
- **Buying a RAG-observability SaaS for a personal corpus.** The schema lessons are free; the products solve multi-team problems Atlas doesn't have. Self-hosted trace tables in the same libSQL world fit the fleet's shape.

---

## Cluster 5 — Classification safety

### Findings

**The tool stack for "nothing sensitive in the wrong tier" is mature, and it layers by cost:**

1. **Secret scanners (cheap, deterministic, full-corpus).** _gitleaks_ — regex/rule-based, MIT-licensed, fully local, no published false-positive rate; it ships inline `gitleaks:allow` comments, path allowlists, and stopwords _because per-corpus tuning is expected_. _trufflehog_ — broader sources + **live credential verification**, two catches for Atlas: (a) it is **AGPL-3.0** — which collides with Drew's own 2026-10-24 adoption hard filter ("NOT AGPL, NOT GPL") recorded in the worlds-brief mining, so adopting it needs the adoption-vs-use question answered deliberately; (b) **verification is on by default and calls external vendor APIs with candidate secrets** — inside a secret-tier scan that behavior would exfiltrate the very material being hunted unless run with `--no-verification`. (https://github.com/rivendale/local-ai/blob/HEAD/pii/README.md)
2. **PII detection (expensive, probabilistic, sampled or tier-scoped).** Microsoft _Presidio_ (MIT; now community-governed under data-privacy-stack): Analyzer (regex + checksum validators + spaCy NER + context words) + Anonymizer, 40+ entity types incl. national IDs for 16+ countries, IBANs, cards. Its own documented caveats: person/organization names rely on NER + context and false-positive without it; some country recognizers carry high FP risk absent context words; checksum-backed recognizers (cards, IBAN) are the reliable tier. At ~107k chunks a full NER pass is the expensive step; regex+checksum passes are not.
3. **Personal-inventory matching (highest precision, unique to a personal corpus).** A three-layer pattern from a working personal setup: (1) secrets via gitleaks/trufflehog, (2) generic PII via Presidio, (3) matches against _the owner's own_ PII inventory (home address, family names, personal phone, ID numbers) loaded from a private file — matches in lower tiers flagged HIGH severity. Generic tools can't know that a specific street address is sensitive; the inventory can. (https://github.com/danielrosehill/claude-pii-scanner-plugin/blob/HEAD/README.md)

**Strategy for a ~107k-chunk corpus (and growing):**

- **Full deterministic scan, every revalidation:** gitleaks-class rules over all chunk text in both tiers. This is fast (these tools scan GB-scale git histories) and catches the catastrophic class (live credentials).
- **NER pass scoped:** Presidio over the _internal_ tier in full (that's where a leak does damage — internal content is the broadly-queried tier) and over the secret tier on a sample (there, PII is expected; the question is calibration, not presence).
- **Sampled human audit:** a small random sample of each tier hand-checked per revalidation round, to estimate the scanners' miss rate instead of assuming it. "No detector catches everything" is every tool's own disclaimer; the audit sample is what turns that caveat into a number.
- **False-positive handling:** findings go to a **quarantine state, never silent deletion** — the chunk is flagged, tier-locked upward (treat as secret until cleared), and queued for review with the matching rule + context. Allowlists carry reasons and expiry; a stale allowlist is how leaks get grandfathered.

**Classification must be enforced at fan-out, not just recorded at ingest.** A tier label in a metadata column is advisory; the guarantee Drew wants ("nothing sensitive in the wrong classification level") requires the query path to filter by tier _before_ merge/rescore — a secret-tier chunk must be structurally unreturnable to an internal-tier query context, including via the rescore stage and via span-join reconstruction (Cluster 1: a span reused across versions must not drag secret context into an internal file version — span tier = max tier of its source contexts is the conservative rule; label this a design decision for the follow-ups, it's _derived here, not sourced_). Revalidation is then a recurring scan with _current_ rules: new rules find old leaks, so the scan is a gate that re-runs, not a one-time certification. This mirrors the fleet's existing reality — a pasted third-party API key was already found once in the conversation corpus by the gap pass — i.e., the corpus is known to contain at least one real instance of the failure class.

### Pros / cons

| Approach                                               | Pros                                                                     | Cons                                                                                                                                                |
| ------------------------------------------------------ | ------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| gitleaks full-scan + Presidio scoped + inventory layer | Layered cost; MIT-clean core; catches known-shape secrets with certainty | NER FP/FN rates need the audit sample to be known quantities                                                                                        |
| trufflehog with verification                           | Confirms which secrets are _live_ — severity gold                        | AGPL; verification calls are an exfiltration channel unless disabled; verification of third-party credentials is ethically/legally its own question |
| LLM-based classification pass over everything          | Catches context-shaped sensitivity regex can't                           | Cost/latency at 107k+ chunks; nondeterministic; the classifier itself sees the secrets (fine locally, not via API for the secret tier)              |
| Manual review only                                     | Highest judgment quality                                                 | Doesn't scale past a few hundred chunks — sampling only                                                                                             |

### Dead ends

- **Scan-once-at-ingest as the whole strategy.** Rules improve; corpora get re-tiered; sources turn out dirtier than believed. One-time scanning certifies a moment, not a corpus.
- **Redact-and-forget.** Redacting a found secret in Atlas while the source (and the credential itself) stays live treats the symptom; the workflow must emit a rotation task for the credential owner — for Drew's own keys, a tracked item; for third-party keys found in ingested material, a notify/purge decision.
- **Deleting findings to make the report green.** Quarantine preserves evidence and lets FP tuning be measured; deletion destroys both.

---

## Cluster 6 — GitHub content licensing (the gate question)

_Read in full from the primary source: GitHub Terms of Service, effective 2026-04-27, Section D (User-Generated Content) and Section H (API Terms), fetched 2026-11-04. https://docs.github.com/en/site-policy/github-terms/github-terms-of-service — This is research, not legal advice; confidence labels are explicit._

### What the ToS actually says

**Copyright stays with the commenter.** Section D's short version: "You own the content you post on GitHub." Nothing in the ToS transfers ownership of issue/PR/discussion comments to GitHub or to other users. _(Confidence: high — primary text.)_

**D.4 — the license to GitHub is broad, and it runs to GitHub, not to us.** Users "grant GitHub and our Affiliates the right to store, host, archive, parse, display, and make copies of Your Content as necessary to provide, develop, and improve the Service, **including by training AI Features**… parse it into a search index or otherwise analyze it on our servers; share it with other Users…" Two consequences: (a) GitHub itself may index and even train on public comments — that right is GitHub's; (b) **Atlas acquires no rights from D.4.** Anyone arguing "GitHub can index it, so I can too" is borrowing a license granted to someone else. _(High.)_

**D.5 — the license to other users is real but narrow, and scoped to the Service.** Verbatim: _"Your Content that you post publicly, including issues, comments, and contributions to other Users' repositories, may be viewed by others. By setting your repositories to be viewed publicly, you agree to allow others to view and 'fork' your repositories… By making a repository public, you grant other Users a nonexclusive, worldwide license to use, display, perform and reproduce (by forking) Your Content **through the Service as permitted by GitHub's functionality**."_ The operative limits are "(by forking)" and "through the Service": the license other users get is to view and to fork **within GitHub**. Copying comment text _off_ the Service into an external corpus (Atlas) is not within the four corners of D.5's grant. Note that comments are expressly named as covered content — so the question isn't whether comments are "Content" (they are), it's whether the _use_ fits the grant (off-Service copying doesn't, on the text). _(High on the text; the conclusion is a reading of it, labeled as such.)_

**D.6 — inbound=outbound is the strongest pro-reuse provision, and it's genuinely ambiguous for comments.** Verbatim: _"Whenever you add Content to a repository containing notice of a license, you license that Content under the same terms, and you agree that you have the right to license that Content under those terms."_ GitHub glosses it: "This is widely accepted as the norm in the open-source community; it's commonly referred to by the shorthand 'inbound=outbound'." If an issue comment counts as "add[ing] Content to a repository," then comments in an MIT repo are MIT-licensed by their authors — a colorable, ToS-grounded argument. The counter-reading: D.6 was written for contributions _to the repo's content_ (patches, files), and issue discussion is conversation _about_ the repo hosted alongside it. The ToS does not define the boundary. _(Text: high. Application to comments: contested — this is Lawyer Question 1.)_

**D.8 — the ToS deliberately does not restrict third-party use; "lawful" does the work.** Verbatim: _"By choosing to contribute Content to a public repository, you are choosing to and directing us to make such Content accessible to everyone on the internet. Unless specifically set forth herein, these Terms do not restrict lawful access to or use of the contents of public repositories by third parties, or by GitHub or its Affiliates."_ Read carefully: GitHub is saying _its terms_ aren't the obstacle — which pushes the question back to copyright law and the content's own license. Public accessibility is not a license. "Public ≠ public domain" remains the default rule: no license → all rights reserved → reproduction needs a copyright doctrine (fair use) or permission. _(High.)_

**Section H (API Terms) grants nothing and restricts little that's relevant.** API use is subject to the ToS + Privacy Statement; the named prohibitions are abuse/rate-limit evasion, token sharing to exceed limits, and downloading data "for spamming purposes, including for the purposes of selling GitHub users' personal information." Bulk comment extraction via the API for a personal index is not named — but Section H is a conduct section, not a rights grant; silence there confers no copyright license. _(High.)_

**Creative Commons: GitHub applied it to its own materials, not to user content.** The ToS _document itself_ is CC0 (Section G: "This Agreement is licensed under this Creative Commons Zero license"). GitHub's help/docs content — the github/docs repository — is dual-licensed: **CC BY 4.0 for documentation/content, MIT for code** (repo README). So Drew's "I don't know if it's Creative Commons" has a precise answer: **GitHub's own docs are CC BY; users' comments are not CC anything** unless the repo's own license reaches them via D.6 or the user separately licensed them. The common belief that "stuff on GitHub is Creative Commons" conflates GitHub-the-author with GitHub-the-host. (https://github.com/github/docs/tree/main)

**Deletion duty signal.** D.5's neighboring text notes the D.4–D.8 licenses "end when you remove Your Content, **unless other Users have forked it**." For an external corpus the fork exception wouldn't apply (Atlas isn't a fork within the Service), which suggests a defensible corpus should honor source deletions: if a comment is deleted or a repo goes private, Atlas should purge on next sync. That's both an ethical norm and a risk reducer. _(Reading; medium-high.)_

**Context, not law: the Copilot training disputes.** The Doe v. GitHub/Microsoft/OpenAI litigation over Copilot training on public code saw most claims dismissed (2024), and no court has categorically held that training/indexing public code is fair use or infringement. Treat the area as unsettled norms with active litigation risk for _commercial redistribution_ specifically — it neither blesses nor condemns a private personal index, and it should not be cited either way in the design. _(Background from general knowledge, not re-fetched for this doc — flagged for the follow-up verification item.)_

### Bottom line, by zone

| Zone                                                                              | Posture                                                                                                                                          | Basis                                                                                                                                                                                                                                                               |
| --------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Our own repos** (Drew's + org repos; his comments anywhere)                     | **Clear — ingest freely** (subject to Cluster 5 tiering)                                                                                         | He holds the rights or is the author; org content is his to license                                                                                                                                                                                                 |
| **External repos where Drew participated** (his threads; others' replies in them) | **Mostly defensible for private personal search; do not redistribute**                                                                           | His own comments are his. Others' replies: D.6 gives a colorable license where the repo is OSS-licensed; otherwise fair-use-style arguments (personal, non-commercial, no market substitute, search/index purpose) are plausible but untested for this fact pattern |
| **External public repos generally** (bulk comment corpus)                         | **Store-privately-for-personal-search is the defensible line; serving others or redistributing the corpus is not defensible on current sources** | D.5's grant is through-the-Service only; D.8 defers to copyright law; no-license repos = all rights reserved. Factual metadata (titles, URLs, timestamps, counts, hashes) is not copyrightable expression and is safe to keep even where full text is not           |
| **GitHub's own docs/help content**                                                | **Reusable with attribution**                                                                                                                    | CC BY 4.0 (content) / MIT (code) in github/docs                                                                                                                                                                                                                     |

Confidence summary: the ToS readings are high-confidence (primary text, current version). The fair-use posture for Zone 2/3 private indexing is a _risk assessment_, not a settled right — US fair use is fact-specific (purpose/character, nature, amount, market effect), and a whole-comments corpus takes entire works, which cuts against one factor even for private use. Other jurisdictions differ (EU text-and-data-mining exceptions have opt-outs; no general fair use). **This cluster gates the _external_ portion of the first-pass build only** — per the brief's own ordering ("figure that out before all of this is built"), Zones 1 and the metadata layer of Zones 2–3 can proceed while the text question is open.

### Questions a lawyer answers (the ToS does not settle these)

1. Does D.6's inbound=outbound reach issue/PR/discussion _comment text_, or only contributed files/patches?
2. Is a private, single-user, non-commercial search index (embeddings + stored text + snippet display to the owner only) over others' public comments fair use in the US? Does storing _embeddings_ (derivative? non-expressive?) change the analysis vs storing text?
3. What changes if the corpus is ever queried by anyone else (agents acting for Drew? family? a future ID-graph publication)? Where exactly is the redistribution line — snippets in answers? full text in an export?
4. EU/UK exposure: do TDM exceptions apply to a US-based personal user, and do GitHub's terms or repo-level signals constitute an opt-out?
5. What deletion/takedown process should the corpus implement to stay on the right side of D.5's license-termination logic and ordinary decency (author deletes comment → corpus purges)?

### Dead ends

- **"It's public, so it's free to use."** Refuted by D.5/D.8's own structure — GitHub drafted a narrow license precisely because public visibility isn't one.
- **"GitHub's ToS licenses it to everyone."** The broad license (D.4, including training) runs to GitHub and Affiliates only.
- **"Comments are too small to be copyrighted."** Some are (one-line reactions); substantive technical comments plainly can be original expression. A corpus-wide rule can't rest on de minimis.

---

## Cluster 7 — Ingestion ops + cost

### Findings

**Trigger shapes, by source class** (precedents in parentheses):

- _Git repos:_ push webhooks or post-commit hooks → ingest the commit range (the range _is_ the delta manifest: `git log old..new --name-status -M` gives adds/edits/renames in one pass — with Cluster 2's caveat that `-M` is a hint). Cheapest correct trigger that exists.
- _Append-mostly logs (sessions, chats, boot logs, stats):_ cron delta scan keyed on a **watermark file** per source (last path + byte offset / last timestamp). Append-only sources make offsets stable — the same span discipline as Cluster 1, one level up.
- _Databases (OpenCode session store, Supabase exports):_ scheduled export deltas keyed on `updated_at`/id high-water marks.
- _Validation before publication (all classes):_ lakeFS's write-audit-publish — ingest into a **staging shard/branch**, run classification scan (Cluster 5) + dedup edges (Cluster 3) + provenance check, then publish to the live shard set. A failed batch never touches the searchable corpus. (https://lakefs.io/blog/git-like-data-versioning-meets-mlops-lakefs-with-mlflow-datachain-neptune-quilt/)

**The delta protocol must be metadata-first.** The brief's 21 GB / 20.5 GB-already-present example defines the requirement: discovering the delta must not read the 21 GB. The pattern (restic's file-level shortcut is the reference implementation): per source, keep a manifest of `(path, size, mtime, head-hash)`; unchanged triple → skip without opening the file; changed → span-match (Cluster 1) so even then, only changed spans are read/embedded. restic reuses all chunk references when mtime matches without re-reading content — acceptable there because content-addressing makes a wrong skip detectable later; Atlas should add periodic full-verification passes (sampled re-hash) so mtime lies (clock skew, touch-without-change, restore-from-backup) get caught. (https://deepwiki.com/bizflycloud/bizfly-backup/4.2-file-scanning-and-chunking)

**I/O politeness is a solved ops problem; use the boring tools.** Sequential reads (no parallel random I/O over spinning/network storage), `ionice -c3` + `nice` for scanners, systemd `CPUQuota`/`IOWeight` on the laptop-side units, checkpointed progress (the watermark file doubles as the resume point), and off-peak windows for full-verification passes. The failure mode to design against is the scanner competing with the laptop's actual work — the fleet already learned this class of lesson with laptop connection storms; ingestion should be a _polite batch citizen_, throttled by policy, not by hope. _(Derived from fleet operating principles + standard tooling; labeled as engineering judgment, not a cited result.)_

**Embedding cost accounting — the numbers say delta discipline matters more than model choice.** Current published prices (verified Sept–Oct 2026 by the comparison sources): OpenAI text-embedding-3-small **$0.02/1M tokens** ($0.01 batch), 3-large **$0.13/1M** ($0.065 batch); self-hosted BGE-M3 ≈ **$0.001/1M** at high GPU utilization, break-even vs API at ~15M tokens/month sustained. (https://embeddingcost.com/compare, https://superlinked.com/blog/sie-vs-openai?ref=hackernoon.com)

Worked scale for Atlas (estimates — label as such): ~107,639 chunks at, say, 250–500 tokens average ≈ 27–54M tokens of corpus. A _full_ re-embed on 3-small batch ≈ **$0.27–$0.54**; on 3-large standard ≈ $3.50–$7.00. So today's corpus is cheap in absolute terms — and the brief's fear scenario is _churn_: re-embedding the unchanged 95% on every version of every log, forever, plus corpus growth from comments/docs expansion (plausibly 5–20× current size). The cost ledger design follows: log per batch `(chunks_new, chunks_reused, tokens_embedded, model, $)` — reuse ratio is the headline metric; if reuse <90% on a log-source batch, the delta protocol is broken, not the budget. Against the standing plan cap (Turso scaler $24.92/mo in the program record), embedding spend is a rounding error _if_ deltas work — storage and DB-count quotas bind first.

**One cost argument overrides price: the secret tier must be embedded locally.** Sending secret-tier text to a hosted embedding API is a classification violation regardless of the bill (Cluster 5). A local embedder (BGE-M3 class, or the fleet's existing local models) on fleet hardware at bursty utilization costs more per token than the API's $0.001–0.01 and is worth it; the internal tier can use whichever route the ledger favors. This also caps the "avoiding spending more than the plan" risk structurally: local marginal cost is electricity, not metered API.

**Storage growth model (for the retention defaults in Cluster 2).** Versioned-corpus growth ≈ Σ over sources of (churn bytes/day × avg versions retained). With span reuse, a version's _marginal_ storage is only its new spans + manifest; the model's inputs are all measurable from the ingest ledger after a few weeks of operation — so v1 retention defaults should be conservative (keep more), with the growth model recalibrated from real churn before any deletion policy fires. Cold-tier (rustfs) moves should precede deletions by at least one full retention cycle.

### Pros / cons

| Choice                                        | Pros                                                       | Cons                                                                                                                                                                                                                                                                                                                                                 |
| --------------------------------------------- | ---------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Metadata-first delta + span matching          | Delta cost ∝ change, not corpus; reuse ratio is measurable | Manifest is another stateful artifact to keep consistent (watermark corruption = missed or doubled ingest → needs the sampled verification pass)                                                                                                                                                                                                     |
| Staging-shard publish gate                    | Bad/sensitive batches never searchable; retry is safe      | Extra shard writes; publish step is a consistency point to get right                                                                                                                                                                                                                                                                                 |
| API batch embedding (internal tier)           | Cheapest ops burden; batch discount; zero GPU babysitting  | Metered; data leaves the fleet (internal tier only, by policy)                                                                                                                                                                                                                                                                                       |
| Local embedding (secret tier; optionally all) | No egress; unmetered marginal cost                         | GPU/CPU time on fleet hardware; model-quality parity must be eval'd (Cluster 4 golden set), and _mixing embedding models across tiers makes cross-tier score comparison meaningless_ — flag: fan-out merge assumes comparable vectors; per-tier models break score comparability unless rescore normalizes (open design question for the follow-ups) |

### Dead ends

- **Full-corpus re-embed on any chunking/model change, as the default.** Sometimes necessary (a model change invalidates every vector) — but as a _routine_ it converts a $0.50 corpus into an unbounded recurring bill and multi-hour jobs. Model/chunking migrations should be rare, planned, and shadow-built (new index beside old, cut over on golden-set parity — the pattern the fleet already used for the Turso shard restore).
- **Always-on GPU for bursty embedding.** ~$1,080/mo for a spot A100 at zero usage vs pennies of API batch — utilization is the whole game; batch the deltas instead.
- **Polling sources at high frequency "to be fresh."** Freshness the user can't perceive, paid in laptop I/O. Watermarks + event triggers where they exist; slow cron where they don't.

---

## Proposed follow-up list (for the filing wave — granular, title + one-line scope)

### Cluster 1 — Delta ingestion + offset embeds

1. **Span-table schema design** — embeds as `(file_version_id, start, end)` spans with their own embedded text snapshot; how spans and today's chunks coexist during migration.
2. **Offset-matching algorithm** — rsync-style sliding-window (rolling weak hash + strong verify) matcher that aligns a new file version against prior spans at arbitrary offsets.
3. **Reconstruction join semantics** — interval-union merge of overlapping result spans per file version; ordering, adjacency gaps, and context-heading attribution.
4. **Chunk-boundary policy v2** — structural-first splitting for markdown/code, entry-boundary splitting for logs, fixed-window fallback; per-source-type configuration.
5. **Boundary-drift measurement** — quantify how much a paragraph's vector/rank moves when embedded with different neighbors; decides how much span reuse is safe.
6. **Overlap budget experiment** — 0% / 10% / 20% overlap: recall on the golden set vs index size, per source type.
7. **CDC feasibility at embedding scale** — FastCDC with ~512-token targets on the real corpus: dedup ratio and boundary stability vs the span approach.
8. **Result-level dedup rules** — near-identical chunks from different versions crowding top-K: merge/suppress rules at merge and rescore stages.

### Cluster 2 — Versioning + retention

9. **File-lineage graph schema** — logical-file nodes vs path attributes; typed rename/split/merge/delete edges and who writes them.
10. **Newest-version resolution view** — max-version semantics at query time, incl. tombstoned deletes and renamed paths; default-on vs explicit all-versions search.
11. **Retention rule engine** — composite policy (age, supersede-count, last-surfaced) with a dry-run reporter that lists what _would_ cold-tier/delete before anything fires.
12. **Cold-tier shard format** — self-describing rustfs shard files (manifest + spans + vectors) and the restore/re-import path back to hot tiers.
13. **Commit-discipline convention + fallback** — rename-only commits guidance for our repos, and what Atlas does when git rename detection fails anyway (segment-search repair pass).
14. **Layer-separation note: Turso PITR vs corpus retention** — one-page decision record: PITR is DB disaster recovery (≤90-day windows); corpus retention lives in Atlas tables + rustfs; never substitute one for the other.

### Cluster 3 — Provenance

15. **Atlas provenance record v1** — PROV-lite field set per chunk/span (entity, derived-from, generated-by run, attributed-to agent, timestamps, source mark, content hash, offsets).
16. **Source-mark grammar** — one parser + one formatter for `repo@commit:path`, `system@host@date`, `chat@session@timestamp` marks; validation rules.
17. **Existing-embed provenance audit** — scan all ~107k current chunks for missing/malformed/wrong marks (incl. the laptop-DB path errors Drew flagged); repair queue.
18. **Dedup-edge pipeline** — SHA-256 exact groups → MinHash/LSH candidates → verified Jaccard edges with method + normalizer version recorded; ingest-time companion to rescore-time #5733.
19. **Ingest attestation record** — in-toto-shaped per-batch statement: subjects = written chunk hashes, materials = source hashes + marks, builder = ingester name/version/git hash, extras = tier + scan results.

### Cluster 4 — Retrieval analytics + feedback

20. **Query trace schema** — append-only log: query/rewritten query, corpus snapshot ids, per-candidate per-stage (score, rank), K-survival, querier context; retention policy for the log itself.
21. **Top-K survival report** — funnel stats at K=1/2/3/5 by shard, tier, and source type; which shards/sources earn their fan-out.
22. **Feedback capture v1** — thumbs on every result + optional comment, bound to `(query_id, result_id, embed_version_id)`; works from every search surface Drew uses.
23. **Feedback normalization pass** — rubric-based LLM judge over the comment corpus producing a label taxonomy; periodic human calibration sample with agreement measurement.
24. **Golden-set builder + eval gate** — thumbed-down and sampled queries become a versioned eval set; RAGAS-style metrics gate chunking/rescore/model changes.
25. **Implicit-signal logging** — clicks/copies/dwell/requery as weak labels, scoped to Drew's own usage, with the bias caveats recorded.

### Cluster 5 — Classification safety

26. **Tier-classification scanner v1** — gitleaks-class rules over all existing chunks in both tiers; quarantine-not-delete workflow with tier-lock-upward default.
27. **Presidio NER pass design** — full internal-tier pass + sampled secret-tier pass: runtime/cost at corpus scale, FP review queue shape.
28. **Personal-inventory pattern pack** — Drew-specific identifiers (addresses, phones, family names, account fragments) as highest-severity detectors, stored in the secret tier only.
29. **Trufflehog decision record** — AGPL adoption-filter collision + verification-calls-exfiltrate finding; gitleaks-first ruling, trufflehog only `--no-verification` if ever.
30. **Revalidation cadence design** — rule-update-triggered re-scans, per-round sampled human audit with miss-rate estimate, allowlist expiry policy.

### Cluster 6 — Licensing gate

31. **External-content ingestion policy (decision doc)** — the three zones (ours / participated / general public) with what's stored per zone: full text vs metadata-only vs excluded.
32. **License capture at ingest** — record each external repo's detected license + D.6 applicability flag in provenance, so policy can be re-evaluated per repo without re-crawling.
33. **Deletion-propagation design** — source comment deleted / repo privated → next-sync purge path + tombstone in the trace log (feedback referencing purged results stays interpretable).
34. **Lawyer-question packet** — the five open questions from Cluster 6, packaged for one review session; external full-text build waits on it, metadata layer doesn't.
35. **Copilot-litigation verification note** — confirm current docket state of Doe v. GitHub et al. before the policy doc cites it as context.

### Cluster 7 — Ingestion ops + cost

36. **Source-connector inventory** — every source class (repos, docs, hatch tree, session stores, chat logs, stats, reports) with trigger type, watermark location, and owner.
37. **Manifest-first delta protocol** — `(path, size, mtime, head-hash)` change detection; bytes read only for changed files; sampled full-verification pass schedule.
38. **Staging-shard ingest gate** — write → classify → dedup → provenance-check → publish, per batch; failed batches retry without touching the live corpus.
39. **I/O politeness budget** — ionice/nice/systemd quotas + checkpointing for laptop-side scanners; off-peak windows for verification passes.
40. **Embedding cost ledger** — per-batch `(chunks_new, chunks_reused, tokens, model, $)`; reuse-ratio alert (<90% on append-mostly sources = broken delta, not a budget problem).
41. **Secret-tier local embedder** — model selection + throughput/quality test on fleet hardware; secret tier never leaves the fleet.
42. **Cross-tier vector comparability** — if tiers use different embedding models, how fan-out merge/rescore normalizes scores (or: mandate one model everywhere and eat the local/API split differently).
43. **Storage growth model** — churn × retention simulation from the ingest ledger; calibrates v1 retention defaults before any deletion policy fires.

### Cross-cutting (from the brief itself)

44. **Agent-notes placement decision** — where agent notes live (repo vs the dictation's "tile cast" — term unresolved, likely a product name garble to confirm with Drew) vs publishing directly into the ID graph; decision record tying into the existing ID-relations tickets.
45. **ID-graph publication format** — how Atlas objects + provenance/lineage relations publish into the ID graph (PROV-lite mapping from item 15), reusing the common relation-display methods already ticketed elsewhere.
46. **First-pass corpus build plan** — sequencing after the license gate: our repos → all findable docs → the hatch tree ("can repo" in the dictation read as the hatch/hatch-personal corpus) → comments per the Zone policy; each stage gated on classification scan + provenance audit passing.

_End of research deliverable. Companion lanes hold the statement inventory and any conversation tie-ins; filing happens in the wave after this._
