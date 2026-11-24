# Research: ID graph query, visualization, and story-to-structure exploration

- **Date:** 2026-11-24.
- **Scope:** #9126, under Part A epic #9107 and program #9106. Evaluate query and visualization options against the encoding recommendation recorded in the merged #9125 bake-off note; also cover the #9126 amendment on story-to-structure exploration.
- **Boundary:** This is a research recommendation, not a language or runtime implementation decision. The encoding bake-off PR #11780 is merged, but #9125 remains open; this note therefore treats its recommendation as the current evidence-backed basis, not as a formally closed dependency or an engine lock-in.
- **Related records:** [`part-a-graph/research.md`](../2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-graph/research.md) (§ #9125–#9126), [`part-a-graph/versioned-design.md`](../2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-graph/versioned-design.md) (§§ 4–5), and [#9128 social app reference model](../2026-11-23_design_id9128_social_app_reference_model.md).

## Result

**Recommendation:** keep the portable contract a typed, bounded conjunctive triple-pattern API/IR with stable pagination; expose an optional typed JSON representation at process boundaries. Treat SPARQL, SQL/PGQ, Cypher-like traversals and application-specific ranking as adapters or later experiments, not the V2 semantic contract. Use a client graph renderer over authorized, provenance-bearing query pages; the renderer is neither a store nor a query authority. Keep story decomposition, language interpretation, embeddings and correction review as separate derived-artifact workflows which write candidate structured assertions and query them through the same API.

This aligns with the encoding evidence in #9125: canonical typed binary is the current signed/interchange candidate, while a rebuildable byte-key index is the current selective-read candidate. These are separate concerns. Query behavior must not depend on a compressed blob layout, SQLite JSONB, or one engine's private query syntax.

## Evidence and comparison

### Existing repository baseline

- The summit research record describes the repo's tuple-encoded ordered keys and subject/value/author access indexes, and explicitly characterizes them as indexing precedents rather than graph-scale measurements (`part-a-graph/research.md`, § #9125 “Repo evidence”).
- The same record describes bounded-neighborhood prior designs and makes the distinction between a typed triple model and RDF semantics explicit (`part-a-graph/research.md`, § #9126 “Repo and standards evidence”).
- The versioned design already gives a candidate portable query shape, conjunctive basic-pattern semantics, deterministic order, finite operator-configured budgets, typed failures, and the rule that visualization consumes query pages and provenance (`part-a-graph/versioned-design.md`, §5.1–5.2). This research endorses that shape as a V2 starting point; the suggested numeric ceilings there remain deployment-profile proposals, not protocol constants.
- The #9128 record makes feeds and social surfaces named projections over immutable objects and signed relations, not an independent graph store. Query/viz should therefore preserve source identity and projection provenance rather than merge them into anonymous edges ([#9128 design](../2026-11-23_design_id9128_social_app_reference_model.md), §§1–3).
- The merged #9125 bake-off reports compressed independently framed binary, JSONB rows and a normalized byte-key map at 1k/10k/100k rows. Its bounded result is: compressed binary was smallest in that prototype; the byte-key map had the fastest exact lookup/full scan; JSONB point lookup was linear without a separate index. It explicitly did not test production redb/Turso adapters, cold cache, multi-prefix query orders, concurrency, writes or semantic story cases ([#9125 research](../2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-graph/research.md), § “Adapter microbenchmark follow-up”).

### Language and query-model options

| Option | Evidence and fit | Disposition |
|---|---|---|
| Typed triple-pattern API / IR | Fits typed opaque terms and the current byte-key index candidate. A finite set of conjunctive patterns can be validated, authorized and costed before execution; adapters can lower scans/joins to ordered indexes or parameterized SQL. The existing summit design already specifies this seam. | **V2 baseline.** Keep canonical semantics independent of a textual DSL. Expose a typed wire form only where needed; preserve stable cursor, provenance and cost fields. |
| SPARQL 1.2 | W3C's 15 Nov 2026 Working Draft describes SPARQL as a query language for RDF and includes required/optional graph patterns and conjunctions; the wider family includes richer graph-query semantics. It is meaningful only if RDF term and dataset semantics are deliberately adopted. Merely storing triples does not establish RDF conformance. | **Do not adopt as V2 contract.** Revisit a declared subset only if RDF interoperability becomes a requirement and the subset has conformance tests. No implicit entailment, remote federation, property paths or optional joins should be smuggled into the basic API. |
| SQL / SQL:2023 PGQ | Can express graph patterns where engines expose compatible support, but the current adapter set and local evidence do not establish one uniform execution/cost model. Backend syntax also does not define the signed term model or graph authorization. | **Backend adapter / experiment only.** Keep query IR as the conformance oracle; prove lowering parity and budget enforcement per engine before exposure. |
| Cypher / openCypher; Gremlin | openCypher describes itself as a property-graph query language and frames its evolution toward ISO GQL. Apache TinkerPop describes Gremlin as a functional data-flow traversal language over application property graphs. Both carry graph models and operational semantics beyond a basic RDF-like triple pattern. | **Application adapter candidates, not the substrate contract.** Evaluate only for a demonstrated consumer and a bounded subset; do not make traversal expressiveness an excuse for unbounded recursion. |
| GraphQL or bespoke traversal DSL | Can be ergonomic at a product boundary but requires project-owned rules for recursion, joins, ambiguity, query cost and authorization. | **Not a core query language.** A product may compile its API into the typed IR with explicit budgets. |

### Visualization options and invariants

The relevant design problem is not “render the whole graph”; it is progressive exploration of a bounded result set. Cytoscape.js is a client-side graph theory visualization/analysis library written for browser use and provides multiple layouts; it is evidence for a renderer option, not a storage/query service. The existing graphical-client design calls for bounded neighborhoods, explicit truncation, and list/matrix fallbacks (`doc/2026-11-08T08-10-00Z_design_id5782_graphical_client_tooling.md`, lines 99–102 and 172–181, as cited in the summit research record).

Any renderer should:

1. Consume only authorized query pages, with explicit page/cost limits and a stable continuation cursor; never fetch an unbounded graph.
2. Show incompleteness, truncation and omitted-page state conspicuously. A sample or budget-limited page must not look like a complete graph.
3. Preserve assertion/object identity, signer/provenance and source references. Distinguish asserted edges from inferred, projected, alias or embedding-derived relationships.
4. Render cycles, missing references and repeated nodes as explicit references; layout is best-effort and not signed/canonical output.
5. Offer a table/list or matrix fallback for dense, disconnected or nonvisual results; permit a user to inspect the source structured assertion and its natural-language rendering.

Recommendation: use Cytoscape.js as a plausible V2 client renderer if the application already has a browser graph view; compare it with a minimal custom canvas/SVG view only against the actual interaction and accessibility requirements. Do not add a separate graph database or query store for visualization.

## Story-to-structure, language explorer, and ranking

The #9126 amendment asks for much more than a graph UI: documents may be decomposed into one or more structured fact documents; order matters only where represented explicitly; phrases/aliases connect vocabulary to entities; originals, structures and regenerated fact-only prose link to one another; corrections can mark a derived structure stale; and review is a four-step old/new input → new structure → generated prose → corrective edit loop. It also calls for embeddings, dictionary growth, disambiguation, permissive cross-language grammar, small-model review and a whole-story demo.

These requirements should not enlarge the query language's truth semantics. Recommended separation:

- **Source and assertion layer:** retain original bytes/content reference and author. A decomposition is a new, versioned derived artifact containing typed assertions, explicit ordering objects when order is meaningful, and references to the exact source spans/objects and producer/reviewer. Do not overwrite or silently “correct” source or prior signed history.
- **Interpretation and correction layer:** represent alias, sense choice, pronoun/referent resolution, generated fact-only prose, human correction and approval as versioned claims with provenance and confidence/status. A correction may supersede a binding and mark dependent structures stale; it must not silently mutate the structure or claim that an embedding is semantic validation.
- **Query layer:** query aliases, assertions, units and derived structures with typed patterns and explicit relation/profile semantics. Multi-subject/multi-predicate claims that have one assertion identity must remain a grouped assertion object or equivalent typed expression; flattening them into unrelated triples can lose the assertion boundary called out by the directive.
- **Embedding and ranking layer:** embeddings are rebuildable derived indexes linking the original, structured artifact and regenerated prose. Permission to incur embedding/model cost belongs to an explicit refresh/review action. Link weights (one person, many, admin, subject, or user-relative context), BM25-style scores and recommendation signals are ranking inputs, not changes to whether an assertion exists or is authorized. Authorization and visibility filtering precede scoring; ranking output must retain evidence and be labeled as ranking, not truth.
- **Visualization/explorer:** clicking a word, phrase, assertion or node should show typed relationships and provenance through the same bounded query API. Agents should use the same API/IR and receive equivalent truncation, authorization and evidence metadata. A UI editor should show stale/unverified state and support the four-step review protocol.

**Feasibility boundary:** the repository's current graph/query and encoding evidence does not demonstrate the full pipeline, cross-language semantic accuracy, disambiguation quality, false-merge safety, or a small model's ability to adjudicate it. The #9125 benchmark explicitly defers these semantic workloads. Do not claim that compression, graph connectivity or embeddings solve “we” resolution or establish identity. The whole-story demo is a later end-to-end validation, not a gate for defining the bounded V2 query API.

## Preliminary recommendation and sequence

1. Freeze a query IR and executable semantic oracle: typed terms, nonempty conjunctive patterns, stable ordering, pagination/snapshot behavior, visibility and explicit budget/error semantics. Keep updates separate.
2. Implement/benchmark adapters against actual selected storage engines only after the #9125 engine follow-up exists; test each supported prefix order, joins, write/index rebuild, cold/warm reads and cursor continuation. Keep signed canonical bytes and database layout independent.
3. Build one bounded neighborhood explorer against these pages, with provenance and incomplete-state UX plus list/matrix fallback.
4. Add structured-document/alias/phrase exploration as versioned derived artifacts; test assertion-group preservation and stale/correction behavior before adding automated translation.
5. Treat embeddings, relative ranking, model review and the children's-story demo as separate experiments with labeled evaluation corpora, cost/permission controls and false-merge/meaning-change oracles.

## Open questions (not silently decided)

- Does the product require RDF interoperability, including RDF term identity and dataset semantics, or only a typed graph model? This determines whether a SPARQL adapter has real value.
- Which concrete storage adapter(s) are in V2, and which of SPO/POS/OSP or other prefix orders are available with bounded costs? The bake-off has not answered this.
- What constitutes a stable cursor snapshot across index rebuild, mutation and policy change? The query oracle and adapter contract must state expiry/mismatch behavior.
- Which assertion schema preserves grouped/multi-subject semantic units, signing/co-signing and later extension without rewriting history?
- What labeled, multilingual, representative corpus and adjudication process can evaluate translation correctness, pronoun resolution, alias selection and false merges?
- Which user actions authorize model/embedding spend, and how are automatic, reviewed and approved derived artifacts distinguished in clients and APIs?
- What accessibility and dense-graph interactions are required before selecting a visualization library?

## Sources and retrieval log

| Source | Retrieved | Used for / limitation |
|---|---|---|
| [`part-a-graph/research.md`](../2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-graph/research.md), §§ #9125–#9126 | 2026-11-24 | Existing repo evidence, bake-off methodology/limits, W3C/Cytoscape summaries and query-language comparison. It is a project research record, not independent implementation proof. |
| [`part-a-graph/versioned-design.md`](../2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-graph/versioned-design.md), §§4–5 | 2026-11-24 | Existing proposed API and budget/error shape; treated as a proposal, not shipped behavior. |
| [`2026-11-08T08-10-00Z_design_id5782_graphical_client_tooling.md`](../2026-11-08T08-10-00Z_design_id5782_graphical_client_tooling.md) | 2026-11-24 | Bounded neighborhood and truncation precedent, cited through the project research record. |
| W3C, [SPARQL 1.2 Query Language, Working Draft, 15 Nov 2026](https://www.w3.org/TR/2026/WD-sparql12-query-20261115/) | 2026-11-24, archived copy | Confirms document status (Working Draft) and its declared RDF query scope. Ordinary fetch returned an anti-bot interstitial; the archived snapshot was readable. This is not a W3C Recommendation. |
| openCypher, [project overview](https://opencypher.org/) | 2026-11-24 | Describes openCypher as a property-graph language and its relationship to ISO GQL; project overview, not the normative GQL text. |
| Apache TinkerPop, [Gremlin](https://tinkerpop.apache.org/gremlin.html) | 2026-11-24 | Describes Gremlin as a functional data-flow traversal language over application property graphs; overview, not a comparative benchmark. |
| Cytoscape.js, [documentation home](https://js.cytoscape.org/) | 2026-11-24 | Browser graph visualization/analysis and layout capabilities; not evidence of this product's performance or accessibility fit. |

## Change log

- 2026-11-24: Initial focused #9126 research note. It reuses the merged #9125 evidence while explicitly retaining #9125's open issue status and limits the recommendation to the current prototype evidence.
