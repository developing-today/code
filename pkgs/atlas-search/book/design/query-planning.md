# Design: query planning

> **Design — not shipped.** This chapter describes a query planner that does not exist yet. Nothing in it should be read as working behaviour. Tracking: #9092 (the search build-out that owns the planner), #5773 (the query front-end), #5774 (the full parameter surface). The API surface the planner would occupy already exists and already refuses to run it — see [What exists today](#what-exists-today). Any behaviour described below is a design target, to be adopted only through the evaluation discipline in the final sections.

A query planner is the stage between the query as typed and the retrieval as executed. Atlas today has none: the query string goes to the full-text branch as a mechanically derived match expression and to the vector branch as an embedding of the raw text, and the two pools are fused. That works for a well-formed, self-contained query aimed at the whole corpus. It fails predictably on the queries agents and humans actually ask: a follow-up that only makes sense inside its conversation ("and what about the second one?"), a question whose words do not match the corpus's words, a request that names a project and a date range in prose while the corresponding filters sit unused, and a compound question that is really three questions wearing one question mark. The planner is the API the current contract already pretends to have — the design below is the planner that would make that pretence true.

## The pipeline shape

The planner is one stage in a longer pipeline, and its position is fixed:

```
query → classifier → conditional transforms → routed retrieval → fuse → rerank
         (<10 ms)      (only those selected)    (per route)        (RRF)   (separate stage)
```

- **Classifier first, and fast.** A lightweight classifier reads the raw query (plus conversation context where the surface supplies it) and assigns a query class, an intent, and the signals the later stages need. Its budget is **under 10 milliseconds**: it is a small, local decision — heuristics and a small model, never a hosted call and never a large language model round-trip on the critical path. A planner whose first stage costs more than the retrieval it optimises has inverted its purpose.
- **Transforms are conditional.** Each transform below runs only when the classifier's output calls for it (see *Adaptive selection*). The default path for a simple, self-contained query is no transform at all.
- **Retrieval is routed.** The (possibly transformed) query is sent to the branches and shards its route selects — not broadcast everywhere by default.
- **Fuse and rerank are not the planner's job.** Fusion is reciprocal rank fusion over the branch pools, as today; reranking is a separate stage with its own design ([Design: the reranking stage](reranking.md)). The planner decides *what to retrieve and how to ask*; it does not reorder the final ranking, and conflating the two makes both unmeasurable.

Every stage emits its decision into the query trace — the class assigned, the transforms applied, the routes taken, the filters extracted — so that any result can be explained after the fact and every strategy can be evaluated per class (below).

## Components

### Conversational rewriting

A follow-up query is rewritten into a self-contained query using the conversation history: pronouns and ellipses are resolved, and the topic established by earlier turns is carried into the rewrite. "What did it cost?" after a discussion of one named system becomes a query naming that system. The rewrite replaces the query for retrieval; the original text is retained in the trace, and the rewrite is a transform of the *query*, never of the corpus. This component is first-wave (see *Minimum viable planner*) because agent surfaces are conversational by default — a planner that cannot handle a follow-up fails the most common agent query shape there is.

### Expansion

The query is widened with related terms — synonyms, abbreviations and their expansions, and corpus vocabulary for the query's concepts — so the full-text branch can match documents that express the same idea in different words. Expansion feeds the lexical side in particular; the vector branch already generalises across wording, which is one reason expansion is conditional rather than universal: on a query whose terms are already corpus terms, expansion adds noise, not recall.

### Multi-query and RAG-Fusion

The query is restated as several distinct queries — different phrasings, different angles on the same information need — each is retrieved separately, and the resulting rankings are fused (reciprocal rank fusion over the per-variant rankings; this fusion-of-variants pattern is RAG-Fusion). It buys recall and robustness against any single phrasing's blind spots, at the price of one retrieval per variant. It is therefore a selected strategy for queries where phrasing risk is high, not a default multiplication of every query's cost.

### HyDE, gated to underspecified queries

HyDE (hypothetical document embeddings) generates a short hypothetical document that *would* answer the query, embeds that document, and retrieves by its vector — document-to-document similarity instead of question-to-document similarity. It is powerful exactly where the query is too short or too vague to embed well, and wasteful or actively misleading where the query is already specific (a generated answer to a precise factual question invents details the corpus never claimed). The design therefore **gates HyDE to underspecified queries**: the classifier must judge the query underspecified — few content terms, no named entity, no filterable structure — before HyDE may run. It never runs by default, and its gate is part of what its evaluation slice must prove.

### Decomposition

A compound query is split into sub-queries, each retrieved on its own, with the results combined for the caller. "Which system replaced X, and what did its first release break?" is two retrievals; forcing it through as one query retrieves documents about neither half well. Decomposition is selected when the classifier detects genuinely independent information needs in one query — multiple questions, or a comparison whose sides live in different parts of the corpus — and each sub-query then flows through the same routed retrieval and fusion as a top-level query.

### Step-back

The query is generalised one level — from the specific incident to the class of problem, from the named version to the system — and the generalised query is retrieved alongside the specific one. Step-back helps when the corpus answers the general question (a design record, a post-mortem pattern) but no document matches the specific phrasing. It is a supplement, never a replacement: the specific query always runs, and the step-back ranking joins it at fusion.

### Routing by source, intent, and tier

The classifier's route decides where retrieval happens:

- **By source and corpus** — a query about code goes to the code shards, a memory-recall query to the memory corpora, a statistics question to the statistics source. Routing is a recall *and* a cost decision: shards outside the route are not scanned.
- **By intent** — an exact-fact lookup, an error investigation, and a conceptual question demand different branch weights, pool depths, and transforms. The route carries those choices; the branches themselves stay generic.
- **By sensitivity tier** — routing respects the caller's ceiling as a hard boundary, exactly as retrieval does today: filters and ceilings are applied inside the branch queries, before ranking. The planner may narrow a route below the caller's ceiling (a public-surface answer routes only to public material — the lowest rung, spelled `shareable` in code); it may never widen one. The planner is not a place where tier discipline can leak — it inherits the ceiling from the request and can only intersect with it.

### Structured filter extraction

The filters the search surfaces already expose are extracted from the query text itself. `fleet search` and `atlas-search` already accept kind, topic, project, and client filters, plus `since` and `until` session dates and the `max-sensitivity` ceiling. Today a caller must supply them as flags; a query that says "the queue broker decision from last month in the fleet project" carries the project and the date range in prose and gets neither applied. The planner extracts them:

- a named project, topic, kind, or client becomes the corresponding filter value;
- a date expression ("last month", "before the November rewrite", a literal date) becomes a `since` or `until` bound;
- an extracted filter is applied with the same semantics as the flag — inside the branch queries, before ranking — and is recorded in the trace as extracted, with its source span, so a wrong extraction is diagnosable as a planner error rather than a corpus mystery;
- extraction never invents a filter the text does not support, and an ambiguous extraction (two projects named, a date that could bound either end) is left unapplied rather than guessed. A missing filter costs recall; a wrong filter costs the correct results entirely, by filtering them out before ranking.

Filter extraction is first-wave: it is cheap, deterministic enough to test exhaustively, and it converts information already in the query into the filter language the engine already speaks.

## Adaptive per-query strategy selection

The failure mode this design exists to avoid is stacking: running every transform on every query because each one helped somewhere. Stacked transforms multiply latency and cost, interact in ways no single component's evaluation predicted, and make a regression unattributable — when a stacked pipeline gets worse, nothing says which layer did it.

Selection is therefore per-query and driven by the classifier:

- The classifier's class, intent, and underspecification judgements select a **strategy**: the set of transforms to apply, the routes to take, and the branch parameters to use. A simple exact-fact query selects no transforms. A conversational follow-up selects rewriting. An underspecified query may select HyDE or expansion. A compound query selects decomposition.
- Strategies are named, versioned configurations, not emergent behaviour — so a strategy can be evaluated, compared, and rolled back as a unit.
- Every transform is individually defeasible: if a selected transform produces an empty or degenerate output (a rewrite that drops the query's only named entity, a decomposition with one part), the pipeline falls back to the raw query for that branch rather than retrieving on a broken transform.
- The trace records the strategy and each transform's input and output identity, so the evaluation below can attribute a win or a loss to the strategy that caused it.

## Evaluation sliced by query class

A planner evaluated only on an overall mean will be adopted on evidence that does not support it: a transform that helps one class and hurts another can net out positive while damaging exactly the queries it was built for. The evaluation discipline is therefore:

- Every measurement is **sliced by query class** — the same classes the evaluation harness already carries on its queries (`code`, `documentation`, `error`, `concept`, and the golden set's own categories) — and a strategy is judged on the classes it targets. HyDE is judged on underspecified queries; rewriting on conversational follow-ups; decomposition on compound queries. A gain on any other class is a side effect to be reported, not the evidence for adoption.
- Retrieval effects and ranking effects are read separately, with the harness's metrics in their causal order: Recall@100 and Recall@1000 gate retrieval (did the planner's query find more of the relevant material?), and nDCG@10 and MRR@10 judge the resulting ranking. See [Tutorial: run the golden set and read a gate](../tutorials/run-an-evaluation.md) for how those numbers are read.
- Each strategy ships on its slice's result, under the programme's standing rule: a change ships on eval result, not on completion. A strategy with no slice that proves it does not ship, however elegant its design.
- The classifier itself is evaluated as a component: its class and underspecification judgements against judged examples, with its latency measured against the 10 ms budget. A misclassification is not a neutral event — it selects the wrong strategy — so classifier accuracy bounds everything downstream of it.

## Minimum viable planner, and the second wave

**First wave — the minimum viable planner.** Three components, in this order:

1. **Routing** by source, intent, and tier — the cheapest component and the one every later component depends on for its scope.
2. **Structured filter extraction** matching the existing filter surface — deterministic, exhaustively testable, immediately useful on flag-free agent queries.
3. **Conversational rewriting** — the component the dominant consumer (agents in conversation) needs first.

The minimum viable planner also includes the classifier that selects among these, the trace emission that makes them attributable, and the per-class evaluation slices that judge them. A planner without its measurement is not a smaller planner; it is an unaccountable one.

**Second wave.** HyDE (with its underspecification gate), decomposition, multi-query / RAG-Fusion, step-back, and term expansion — each adopted individually, on its own slice, in no fixed order beyond its evidence. Second-wave status is not a judgement that these are worth less; it is a judgement that their value is conditional in ways the first wave's is not, and conditional value is exactly what per-class evaluation exists to settle.

## What exists today

Grounding, so the design above is never mistaken for the present:

- **There is no planner.** Retrieval today is hybrid full-text plus vector search with reciprocal rank fusion over the two pools, and no transform stage of any kind. The query front-end and parameter surface are tracked as #5773 and #5774; the planner as part of #9092.
- **The contract already names the planner, and the code refuses it.** The versioned query-parameters contract (`query-parameters-v1.json` in atlas-proto) carries `planner_enabled: false`, `spell_enabled: false`, `query_class_hint: null`, and `variant: "raw"` as first-class parameters, included in every result-cache key. Setting any of them to a non-default value is rejected at validation: planner and spell stages are "not implemented yet", a query-class hint is "not implemented yet", and a rewritten or combined query variant is "not implemented yet". The API pretends to have a planner in its parameter file and says so, honestly, the moment anyone asks it to plan.
- **The filter surface the planner would extract into already exists.** Kind, topic, project, and client filters, `since` and `until` dates, and the sensitivity ceiling are live flags on the search surfaces and are enforced inside the branch queries today — extraction is a new producer for an existing, tested consumer.
- **The measurement substrate already exists.** The evaluation harness scores per query class, the query-trace store records every executed search, and the golden sets are in place — the sliced evaluation this design requires needs no new infrastructure, only planner strategies to measure.
- **The fusion the planner feeds is the shipped one**: RRF at `k = 10`, pool 100 per branch by default, with the optional Jaccard rescore present in code but gated off by its own failed evaluation (`jaccard_depth: 0`). Nothing in this chapter changes any of those defaults; a planner strategy that wants a different pool or fusion setting must prove it on its slice, like everything else.
