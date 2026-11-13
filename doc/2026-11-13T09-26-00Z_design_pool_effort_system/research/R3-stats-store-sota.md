# R3 — Granular stats store: state of the art for the pool/effort system

Program: design-mill, starter code#9399. Companion to `brief.md` §5–§8 and `brief-decisions-supplement.md` §5, §8, §9.
Author: research subagent (route opencode-route/workhorse), 2026-11-13. Prototype: `prototypes/stats_rollup_oracle.py`.

Conventions: every external claim carries a URL. Claims with no fetched or search-confirmed source are marked **[unverified]**. Claims from the prototype are marked **[prototype]**. Arithmetic is marked **[estimate]**.

---

## 0. Summary

- The fleet's real need is **event-level facts plus mergeable rollup states**, not a dashboard database. Full granularity must be kept, so the store is an append-only event log. Rollups are derived, mergeable states that can be recomputed.
- Mergeable statistics are count, sum, min, max, and mean/variance (Welford/Chan). Median, p90–p99.9, Q1/Q3, IQR and mode are **not** mergeable from summaries. They need a sketch (DDSketch-style log histogram, t-digest, KLL) or an exact value map. The prototype confirms this. Exact quantiles need the raw values.
- The 10^5–10^7 events/day range is comfortably served by a file-based store (Parquet on R2, queried by DuckDB) at V2. ClickHouse becomes attractive only if interactive, many-user, sub-second slicing across 10^9+ rows is needed. Cloudflare Analytics Engine is **not** a system of record: it samples, keeps data three months, and caps blob size.
- Outcome capture should be **append-only** (scores and outcome facts as new rows, never edits to events). That matches the observability-tool pattern (Langfuse treats traces as immutable and attaches scores later).
- Recommended V2 shape (§7): raw event Parquet on R2, daily hour/day mergeable state Parquet, DuckDB as the query engine (reachable from the HF Spaces dashboards and the PM review surface), outcomes and review labels as separate append-only fact tables, and model lineage as a Type-2 dimension.

---

## 1. Event/facts store engines at 10^5–10^7 events/day

Scale arithmetic. 10^7/day is about 116 events/s on average, with peaks some multiple of that. 10^5/day is trivial for every engine below. The real constraints are cost, operational load, retention, and how many people query it.

### 1.1 DuckDB + Parquet on object storage (DuckLake)

- DuckLake keeps table metadata in a SQL database (PostgreSQL, MySQL, SQLite or DuckDB) and data as Parquet on local disk or object storage. The FAQ lists Cloudflare R2 as a supported store for data files. https://ducklake.select/faq.html — https://ducklake.select/
- DuckLake supports snapshots, time travel, schema evolution and ACID multi-table transactions. https://ducklake.select/ (feature list)
- Data files must be Parquet. https://ducklake.select/faq.html
- Writes must go through DuckLake so the catalog is updated. A DuckDB team member's HN comment says dropping Parquet files into the bucket directly does not work. https://news.ycombinator.com/item?id=44106934 (informal source)
- A DuckDB blog (2026-10-07) describes a view-only catalog that is a few hundred KB over a 380,959-row Parquet file on object storage. https://duckdb.org/2026/10/07/view-only-mode.html
- Quack (client/server DuckDB protocol) is used in examples for a catalog server. https://ducklake.select/ — the stability of this protocol is **[unverified]**.
- Fit for us: strong for batch and rollup jobs, cheap, no server, and HF Spaces can read Parquet directly. Weak for many concurrent writers and sub-second dashboards at high volume (not verified here; general knowledge).

### 1.2 ClickHouse (MergeTree, materialized views, AggregatingMergeTree)

- AggregatingMergeTree stores aggregate states; a materialized view writes `minState/maxState/avgState`-style columns, and queries use the matching `*Merge` functions. https://clickhouse.com/docs/reference/engines/table-engines/mergetree-family/aggregatingmergetree ; https://clickhouse.com/blog/using-materialized-views-in-clickhouse
- Example `byMinute` table uses `AggregateFunction(uniq, String)` and `sumState`, read back with `uniqMerge` and `sumMerge`. https://clickhouse.com/blog/chaining-materialized-views
- Caution: cascading materialized views over MergeTree engines can produce surprising results; one community write-up says so (informal, Medium post by sathishdba, "Why Your ClickHouse Materialized Views May Be Lying", URL not retained **[unverified]**). Incorrect results after TTL rollup are reported in ClickHouse issue #70023. https://github.com/ClickHouse/ClickHouse/issues/70023
- TTL with `GROUP BY` rolls old rows up into aggregates (e.g. `SET sum_hits = sum(sum_hits)`) before deleting detail. https://clickhouse.com/docs/concepts/features/operations/delete/ttl — this is the native "rollup then expire" mechanism that matches brief §6's later time-based rollup.
- Quantile states: `quantilesTDigestState` exists and its error is described as under 1% "your mileage may vary" (Stack Overflow answer, informal). https://stackoverflow.com/questions/77318126/aggregation-percentile-clickhouse . The official quantileTDigest doc page I tried returned 404 at the URL I guessed; the search result says `quantile()` uses t-digest and is typically within 1% **[unverified against docs]**.
- Fit: best engine for interactive group-by on huge data. Highest operational cost; overkill for 10^5–10^6/day until a multi-user query load exists.

### 1.3 SQLite / Turso / libSQL

- Turso's pricing page lists Developer/Scaler/Pro/Enterprise, with per-GB overage on Pro. https://turso.tech/pricing
- Embedded replicas: a local file synced from a remote libSQL primary, with a sync interval. https://docs.turso.tech/features/embedded-replicas/introduction
- A third-party 2026 comparison says Turso's hosted edge replicas are closed to new users **[unverified; single secondary source, https://www.buildmvpfast.com/alternatives/turso is not authoritative — check before relying on it]**.
- Cloudflare D1 is managed SQLite: 10 GB max per database (Paid), 500 MB (Free), 10 Free databases, up to 1,000,000 databases on Paid. https://developers.cloudflare.com/d1/platform/limits/ . Sharding into multiple databases is the documented scale-out pattern. https://developers.cloudflare.com/d1/platform/limits/ (FAQ).
- Fit: good for the hot, small, operational tables (decision records, review labels). For full event history at 10^6/day, 10 GB fills in about 100 days at ~100 B/event or ~20 days at ~500 B/event **[estimate]** — see §6 storage arithmetic. D1 needs sharding by month.

### 1.4 Cloudflare Analytics Engine (WAE)

- Limits: up to 20 blobs, 20 doubles and **one** index per `writeDataPoint`; index ≤ 96 bytes; total blobs ≤ 16 KB per data point; ≤ 250 data points per Worker invocation. https://developers.cloudflare.com/analytics/analytics-engine/limits/
- Retention: data is stored for **three months**. https://developers.cloudflare.com/analytics/analytics-engine/limits/
- Sampling: weighted adaptive sampling at write time (equitable per index) and adaptive bit rate at read time. Every row has `_sample_interval`; `count` is `sum(_sample_interval)`; quantiles need `quantileExactWeighted(p)(x, _sample_interval)`. Accuracy depends on the sampled number of rows, not the interval. Individual records are not guaranteed retrievable, and exact event sequences cannot be reconstructed. https://developers.cloudflare.com/analytics/analytics-engine/sampling/
- Fit: useful for live counters and ops telemetry from Workers. **Not** a system of record: sampling breaks exact outcome counts, retention is three months, and one index means one sampling key per row.

### 1.5 Apache Druid / Apache Pinot

- Not researched in depth in this pass **[unverified]**. Both are real-time OLAP systems with ingestion from streams and heavy operational footprints (general knowledge). For 10^5–10^7/day they are over-specified. Decision: do not adopt at V2; revisit only if ClickHouse is ruled out and real-time segment ingestion becomes a requirement.

### 1.6 InfluxDB / Prometheus (metrics stores)

- Prometheus classic histograms: quantile error is limited by the width of the bucket the quantile falls in; interpolation assumes an even spread inside the bucket. Native histograms improve this but are still bounded by resolution. Summaries have low configurable error but cannot be aggregated across instances. https://prometheus.io/docs/practices/histograms/
- Implication: histograms are an approximation dictated by bucket layout. They are fine for ops alerting; they are **not** the source for "p99.9 of cost-per-successful-task by effort". Influx's Flux docs describe the same bucket model. https://docs.influxdata.com/flux/v0/prometheus/metric-types/histogram/
- Fit: keep for infra health. Do not use as the decision store.

### 1.7 Postgres + TimescaleDB (continuous aggregates, retention)

- Continuous aggregates are refreshed by policy (`add_continuous_aggregate_policy` with start/end offsets and schedule). https://www.tigerdata.com/docs/reference/timescaledb/continuous-aggregates
- Retention can keep raw data 30 days while daily aggregates stay 600 days. https://github.com/timescale/docs/blob/latest/use-timescale/data-retention/data-retention-with-continuous-aggregates.md
- Gotcha: a retention policy on raw chunks can silently delete history from continuous aggregates if the job order is wrong. https://www.tigerdata.com/blog/how-retention-policies-silently-break-continuous-aggregates
- Fleet fit: the fleet already runs Supabase Postgres for the queue. Whether Supabase exposes TimescaleDB is **[unverified]**. Plain Postgres partitioning plus hand-written rollups is the conservative option.

### 1.8 Honorable mention: the existing roadmap-stats daily pipeline

- The brief says the granular store extends, not replaces, the daily pipeline (brief §9). That is the right call. Keep its outputs as one rollup consumer.

---

## 2. Quantile and statistics mechanics

### 2.1 Mergeability by statistic

| Statistic | Mergeable from partial summaries? | How |
|---|---|---|
| count, sum | yes, exact | add |
| min, max | yes, exact | min/max |
| mean | yes, exact | sum/count |
| variance / stddev | yes, exact up to float rounding | Welford per cell, combined with Chan et al. |
| median, p90, p99, p99.9, Q1, Q3, IQR | **no** from summaries | need a sketch or the raw values |
| mode | **no** from summaries | needs a value-count map (exact, grows with distinct values) or raw values |

Welford's online update and Chan's parallel merge are given in https://en.wikipedia.org/wiki/Algorithms_for_calculating_variance. The merged M2 is `M2_a + M2_b + delta² · n_a · n_b / n`, with `delta = mean_b − mean_a`.

Datadog states the principle directly: min, max, sum and count merge, and percentiles of subsets do not give the global percentile. https://www.datadoghq.com/blog/engineering/computing-accurate-percentiles-with-ddsketch/

### 2.2 Sketch families

- **DDSketch**: fully mergeable, relative-error guarantee. Buckets are geometric with ratio gamma = (1+α)/(1−α). Merging sums bucket counters. Datadog reports about 2 kB for 1 ms–1 min at 2% relative error with 64-bit counters. https://arxiv.org/abs/1908.10693 ; https://www.datadoghq.com/blog/engineering/computing-accurate-percentiles-with-ddsketch/. Its memory is several times smaller than HDR Histogram, and it ingests about 10× faster than GK. Same Datadog post.
- **t-digest**: accurate near the tails, merges by centroid re-compression. ClickHouse exposes `quantileTDigest` and `quantilesTDigestState`. Error guarantees are weaker and less formal than DDSketch **[unverified in this pass; general knowledge]**.
- **KLL** (Karnin, Lang, Liberty): optimal rank-error quantile sketch, O((1/ε) log log(1/δ)) space for additive rank error εn. https://arxiv.org/abs/1603.05346. Rank error is additive: a 1% rank error is a large relative error at p99.9 for skewed latencies (Datadog makes the same point). https://www.datadoghq.com/blog/engineering/computing-accurate-percentiles-with-ddsketch/
- **HDR Histogram**: relative-error, bucketed; larger than DDSketch by Datadog's comparison. Same Datadog post.
- **GK (Greenwald–Khanna)**: rank-error; Datadog found its errors too large at p99. Same Datadog post.
- **Exact value map**: the value-count Counter used by the prototype for mode. Exact, mergeable, but size grows with distinct values. Good for integer-ms latencies, bad for continuous costs unless binned.

Recommendation: use **DDSketch-style log buckets** for latency and cost-like positive values (relative error bounds fit "cost per %" and "ms" well), with a fixed α (e.g. 1%). Keep an **exact value map** only where the value domain is discrete (ms rounded, token counts, counts). Keep **raw events** as the only source for exact quantiles on demand.

### 2.3 Prototype results on mergeability

From `prototypes/stats_rollup_oracle.py` **[prototype]**:
- 200 seeds × 8,000 synthetic events × 48 hours × 36 groups, two-level rollup (hour cells merged to day cells in shuffled order).
- 14,400 day-group cells checked.
- Exact equality for count, sum, min, max and mode. Mean and stddev matched the direct oracle within relative 1e-9.
- DDSketch-style log histogram (α = 1%): max observed relative error was 0.009997 for p90, p99 and p99.9, 0.009983 for Q1 and median. All within the α bound.
- Mutation check: removing the Chan cross term from the merge made the test fail on stddev in the first seeds **[prototype]**.
- Sample-size finding: **every** day-group cell had fewer than 1,000 samples, so p99.9 is not resolvable at this volume. The per-cell rule of thumb of about 10/(1−q) samples for a stable tail estimate is common practice but **[unverified; heuristic]**. Implication: p99.9 only makes sense on pooled groups or on months of data. The report must show n next to every p99.9.

### 2.4 Quantile definition must be fixed and stored

The oracle, the sketch and the dashboards must use the same definition, or two correct tools will disagree. The prototype uses nearest rank (the ceil(q·n)-th smallest sample). Interpolated quantiles (e.g. numpy's default) differ by up to one sample gap. Record the definition in the stats schema (`quantile_method = nearest_rank_v1`) so historical numbers stay comparable.

---

## 3. Dimensional modeling and event design

### 3.1 Star schema and grain

- The fact table grain must be fixed first; the fact "stays tall and narrow", dimensions are denormalized. https://dataskew.io/blog/dimensional-modeling/ ; https://datawarehouseinfo.com/foundations/dimensional-modeling/ . Kimball's technique list: https://www.kimballgroup.com/data-warehouse-business-intelligence-resources/kimball-techniques/dimensional-modeling-techniques/
- Proposed grain: **one row per LLM step, attempt, tool call or decision** (event), with foreign keys to dimensions: `dim_model` (SCD2), `dim_pool`, `dim_provider`, `dim_effort`, `dim_session`, `dim_task`, `dim_issue`, `dim_pr`, `dim_aa_version`, `dim_test_run`, `dim_change_marker`. Outcomes are a separate fact at grain "one outcome per item per observation time" (§5).

### 3.2 Slowly changing dimensions for model lineage and AA versions

- Type 2 "add new row": a new surrogate key per version, plus effective date, expiration date and a current-row indicator. https://www.kimballgroup.com/data-warehouse-business-intelligence-resources/kimball-techniques/dimensional-modeling-techniques/type-2 ; https://en.wikipedia.org/wiki/Slowly_changing_dimension
- Historical facts keep the surrogate key that was active when they happened. https://datawarehouseinfo.com/foundations/dimensional-modeling/ . Microsoft's Power BI guidance says the same and adds that the load must expire the current version and insert a new one. https://learn.microsoft.com/en-us/power-bi/guidance/star-schema
- **Application to the brief**:
  - `dim_model` (SCD2): model id, variant (e.g. DeepSeek 1107 vs classic, Luna 6 vs 6.1 kept separate), `successor_of`, `predecessor_of` (ordered lineage), `aa_version`, `aa_test_date`, price/speed/capability snapshot, matched-to models (the brief's "efforts it aims to match").
  - Type 1 is fine for display labels; Type 2 for anything that changes the meaning of past data (variant, AA version, price).
  - Variant split as separate dimension rows, not a flag: this matches "Luna 6 and Luna 6.1 are fully separate" (brief §6).

### 3.3 Wide events (canonical log lines)

- Stripe's canonical log line: one line per request per service, collating the request's key telemetry; flexible to query. https://stripe.com/blog/canonical-log-lines
- Honeycomb's observability 2.0 position: arbitrarily wide structured events as the single source of truth. https://www.honeycomb.io/blog/time-to-version-observability-signs-point-to-yes
- Application: emit **one wide event per LLM step / attempt / tool call / decision**, with every tag from brief §6 on that row. The brief's tag list (provider, model, effort, pool, session, subsession depth, task, PR, issue, AA version and test date, lineage, variant, change markers) fits a row of roughly 30–40 columns. Store wide, query narrow.

### 3.4 Change markers and bitemporal facts

- Change markers: a separate `change_marker` fact (kind, date, hypothesis, expected delta, actual delta measured over a pre/post window) keyed by model and pool. Expected vs actual delta is recorded at marker time and at review time, so the later measured delta never overwrites the prediction. **[design pattern, no external source fetched]**.
- Bitemporal: separate event time (when the step happened), ingest time (when the store saw it) and AA-version time (which benchmark version was in force). Rollups by "time since inject", "time since capture" and "time since AA version" (brief §6) are queries over these three clocks. ClickHouse TTL and Timescale retention both act on one time column, so keep event time as the partition key. **[design pattern]**.
- Kimball's timespan tracking in fact tables (effective and expiration span) is the same idea. https://www.kimballgroup.com/data-warehouse-business-intelligence-resources/kimball-techniques/dimensional-modeling-techniques/ (Advanced Fact Table Techniques, as shown in the search snippet)

---

## 4. LLM-usage observability schemas

### 4.1 OpenTelemetry GenAI semantic conventions

The GenAI attribute registry has moved to https://github.com/open-telemetry/semantic-conventions-genai (the old registry page is marked "moved"). Attribute names confirmed on the registry page (https://opentelemetry.io/docs/specs/semconv/registry/attributes/gen-ai/), with the stability note that they are deprecated **in the old location**:

- Operation and identity: `gen_ai.operation.name` (e.g. `chat`, `generate_content`, `text_completion`), `gen_ai.provider.name` (e.g. `openai`, `gcp.gen_ai`), `gen_ai.request.model`, `gen_ai.response.model`, `gen_ai.response.id`, `gen_ai.response.finish_reasons` (string array), `gen_ai.conversation.id`, `gen_ai.agent.name`, `gen_ai.agent.id`, `gen_ai.agent.version`, `gen_ai.workflow.name`.
- Token usage: `gen_ai.usage.input_tokens`, `gen_ai.usage.output_tokens`, `gen_ai.usage.cache_read.input_tokens`, `gen_ai.usage.cache_creation.input_tokens`, `gen_ai.usage.reasoning.output_tokens`, `gen_ai.token.type` (`input`/`output`). Cache and reasoning counts are documented as included in input/output totals.
- Timing: `gen_ai.response.time_to_first_chunk` (seconds, double), `gen_ai.request.stream` (boolean).
- Request parameters: `gen_ai.request.temperature`, `gen_ai.request.top_p`, `gen_ai.request.top_k`, `gen_ai.request.max_tokens`, `gen_ai.request.seed`, `gen_ai.request.stop_sequences`, `gen_ai.request.choice.count`, `gen_ai.request.frequency_penalty`, `gen_ai.request.presence_penalty`.
- Tools: `gen_ai.tool.name`, `gen_ai.tool.type` (`function`, `extension`, `datastore`), `gen_ai.tool.call.id`, `gen_ai.tool.call.arguments`, `gen_ai.tool.call.result`, `gen_ai.tool.description`, `gen_ai.tool.definitions`.
- Content (opt-in, large): `gen_ai.input.messages`, `gen_ai.output.messages`, `gen_ai.system_instructions`, `gen_ai.retrieval.documents`, `gen_ai.retrieval.query.text`.
- Evaluation: `gen_ai.evaluation.name`, `gen_ai.evaluation.score.value` (double), `gen_ai.evaluation.score.label`, `gen_ai.evaluation.explanation`.
- Source for all of the above: https://opentelemetry.io/docs/specs/semconv/registry/attributes/gen-ai/ . The `gen_ai.*` names (and `gen_ai.client.token.usage`, `gen_ai.client.operation.duration` as metrics) are also summarized at https://openobserve.ai/blog/opentelemetry-genai-semantic-conventions/ (secondary).

Not in the spec, but needed by us: `gen_ai.request.effort` (the effort rung), `gen_ai.request.pool`, `task`, `pr`, `issue`, `aa.version`, `aa.test_date`, `model.variant`, `model.lineage`, `change_marker`. Use a namespace such as `fleet.*` for these and keep `gen_ai.*` for the standard fields, so any OTel-speaking backend can read the standard part. Note: the spec's stability for these attributes is mixed (many are "development"); pin the spec version we emit.

### 4.2 Langfuse data model

- Three levels: observations (steps) grouped into traces by `trace_id`; traces grouped into sessions by `session_id`. https://langfuse.com/docs/observability/data-model ; https://langfuse.com/docs/observability/features/sessions
- **Traces and observations are immutable once ingested; enrichment goes into scores.** Scores attach to a trace, observation, session or dataset run; each score references exactly one of those; scores can be overwritten. https://langfuse.com/faq/all/tracing-data-updates ; https://langfuse.com/docs/evaluation/scores/data-model
- Langfuse suggests user feedback (explicit ratings and implicit retries, abandonment, corrections) lands as scores on traces. https://langfuse.com/blog/2024-07-ai-agent-observability-with-langfuse
- Application: the decision and outcome model in §5 is the same shape. Outcomes and reviews are scores (or outcome facts) attached to tasks, sessions, decisions and issues. Events never change.

### 4.3 OpenRouter activity export and analytics

- Activity export: monthly CSV or PDF by model, key or org member. https://openrouter.ai/docs/cookbook/administration/activity-export ; https://www.truefoundry.com/blog/openrouter-usage (secondary, mentions BYOK caveat)
- Analytics query API: `POST /api/v1/analytics/query` with metrics (`total_usage`, `tokens_total`, `cache_hit_rate`), dimensions (`model`), granularity (`day`) and a time range. https://openrouter.ai/blog/announcements/activity-dashboard/
- Responses carry `usage` with token counts and a `cost` field. https://openrouter.ai/docs/cookbook/administration/usage-accounting
- Application: OpenRouter is a **cross-check** for spend per model, not the event store. Reconcile our per-event cost to its daily totals, and log the discrepancy as a metric.

### 4.4 Other fleet-relevant items

- OpenLLMetry / other OTel instrumentations emit `gen_ai.*` spans; the MLflow doc maps its own span fields to the GenAI names. https://mlflow.org/docs/latest/genai/tracing/opentelemetry/genai-semconv/ (secondary evidence that the mapping is practical).

---

## 5. Outcome attribution in software delivery

### 5.1 Metrics with defined meanings

- DORA's four keys include **change fail rate**: the ratio of deployments requiring immediate intervention (rollback or hotfix). https://dora.dev/guides/dora-metrics/ . In 2024 DORA made a fifth, "rework rate", as the proxy for unplanned rework. https://dora.dev/insights/dora-metrics-history/
- Google's Four Keys: change failure rate = failed deployments / total deployments, linked to incidents. https://cloud.google.com/blog/products/devops-sre/using-the-four-keys-to-measure-your-devops-performance
- Revert and churn: GitClear defines churn as code written and then reverted or significantly altered within two weeks. https://www.gitclear.com/coding_on_copilot_data_shows_ais_downward_pressure_on_code_quality (via the jonas.rs summary and Arc) — this is a vendor-defined metric, not a standard.
- GitClear's 2026 data (vendor, not independently checked): within-commit copy/paste +41%, two-week churn +15%, refactoring down. https://www.gitclear.com/the_ai_code_quality_maintainability_gap and https://www.gitclear.com/ai_assistant_code_quality_2025_research (2025: copy/paste rose from 8.3% to 12.3% of changed lines, 2020–2024). Use as priors, not as truth.

### 5.2 Linking commits, PRs, issues and sessions

- Git trailers are the standard place for structured metadata. `git interpret-trailers` and `git commit --trailer` (Git 2.32+) add them. https://crashoverride.com/resources/knowledge-base/code-ownership/attributing-ai-commits-git (secondary)
- Attribution convention: `Assisted-by: AGENT:MODEL [TOOL]` as in the Linux kernel's coding-assistant policy (the kernel docs page is cited in a blog, not fetched here). https://williamhuster.com/ai-attribution-assisted-by/ ; https://github.com/cue-lang/cue/issues/4452 (a project adopting `Assisted-by` and rejecting `Co-authored-by` for AI).
- Recommendation for our stack: `Assisted-by: Claude Code (<model id>)` on commits, plus a fleet trailer such as `Fleet-Task:`, `Fleet-Session:` and `Fleet-Decision:` with the ids. These are proposed names, not a standard **[design proposal]**. The PR body (pr skill) carries the same ids so GitHub search can link them.
- The commit → PR → issue → task → session chain is then a join over ids, not a guess. Store the join as a fact (`fact_link(item_a, item_b, kind, observed_at)`), append-only.

### 5.3 Outcome vocabulary (from the brief)

Map the brief §5 outcome list to a closed enum with a definition each, and store one row per outcome observation:

- Lifecycle: merged, closed-incomplete, canceled, not-pushed, issue-resolved, issue-made.
- Quality: reverted (commit or PR), rework (follow-up fix within a window), undone (removed later), duplicate-work, caused-incident, data-loss, invalid, unclear, misunderstood, too-big, overcomplicated, oversimplified, perfect, simple, safe, fixed, partial, cheaper-than-expected, unneeded-worry, overtested, undertested.
- Each outcome carries: `observed_by` (self / parent / reviewer / system), `observed_at`, `window` (e.g. "14 days after merge"), `evidence_url`, and `confidence`.
- Revert detection is mechanical: `git log --grep` on revert trailers and `Revert "..."` titles, plus GitHub's revert PRs. Rework within a window is a file-level overlap test, not an LLM judgement. **[design proposal]**.

### 5.4 Eval-driven model selection in practice

- OpenRouter publishes Artificial Analysis benchmark data via its `/api/v1/benchmarks` endpoint (sources include AA), which is an external cross-check for the AA numbers we ingest. https://openrouter.ai/docs/cookbook/administration/data-api
- Langfuse datasets/runs exist for evaluation experiments; the same scores model covers them. https://langfuse.com/docs/evaluation/scores/data-model
- The brief's round-robin trials (brief §8, Luna pattern) are controlled experiments; each trial is a change marker plus a cohort, and the success metric is pre-registered (named measurement, review date, revert condition — brief §9).

---

## 6. Storage growth, retention and rollup policies

### 6.1 Measured (prototype, synthetic)

From `prototypes/stats_rollup_oracle.py` seed 0 **[prototype]**:
- 8,000 events: raw JSON ≈ 99.3 bytes/event (794,377 B total).
- Hour-level mergeable states: 1,714 cells, 280,263 B (≈ 35% of raw). The exact value map (`vals`) dominates; the histogram is small.
- Day-level states: 72 cells, 105,548 B (≈ 13% of raw).
- These are **uncompressed JSON**, not Parquet, and the event has only 7 tags. Real wide events will be larger; Parquet compression will be much smaller. Treat the ratios as an upper-bound sketch, not a forecast.

### 6.2 Growth arithmetic (estimate)

Assume 100 B/event (prototype-like, uncompressed) or 500 B/event (wide, uncompressed). Do not compress.

| events/day | 100 B/event: per day | per year | 500 B/event: per day | per year |
|---|---|---|---|---|
| 10^5 | 10 MB | 3.7 GB | 50 MB | 18 GB |
| 10^6 | 100 MB | 37 GB | 500 MB | 180 GB |
| 10^7 | 1 GB | 365 GB | 5 GB | 1.8 TB |

**[estimate]**. Against the D1 10 GB cap, 10^6 events/day fills one database in about 100 days (100 B) or 20 days (500 B). This is why D1 needs monthly sharding. Compression of Parquet typically reduces such data a lot; the factor is **[unverified]** for our data and must be measured in the first month (brief §6 asks us to measure growth).

### 6.3 Retention and rollup patterns

- ClickHouse: TTL with `GROUP BY` keeps rollups and drops details after a period. https://clickhouse.com/docs/concepts/features/operations/delete/ttl
- TimescaleDB: raw retention 30 days, daily aggregates 600 days, in one configuration. https://github.com/timescale/docs/blob/latest/use-timescale/data-retention/data-retention-with-continuous-aggregates.md . Beware the silent-deletion gotcha. https://www.tigerdata.com/blog/how-retention-policies-silently-break-continuous-aggregates
- Cloudflare Analytics Engine: three months, fixed. https://developers.cloudflare.com/analytics/analytics-engine/limits/
- Brief §6 wants rollups later by time since inject, time since capture, and time since AA version. These are three different clocks; keep all three as columns on the event so later rollups are a query, not a rewrite.
- Parquet compaction of many small daily files into one per month: standard practice **[unverified; no source fetched]**. DuckLake's snapshot model avoids expensive compaction for snapshots (https://ducklake.select/ — "You can have as many snapshots as you want without expensive compaction steps").

### 6.4 Policy I would set

- Raw events: keep in full at V2 (brief §6 says full granularity now). Measure bytes/day for 30 days, then decide retention.
- Hour states: keep for 13 months.
- Day states: keep forever (small).
- Outcome and review facts: keep forever (append-only, small).
- Never delete raw events that carry an outcome until the outcome window closes.

---

## 7. Design options catalog

Cost is order-of-magnitude for 10^5–10^7 events/day. "V2/V3/V4" placement follows the brief's version split; if the brief has no formal V-list, read it as V2 = next slice, V3 = after the first analytics, V4 = optional.

| # | Option | Pros | Cons | Cost (order) | Scale | Reliability | Placement |
|---|---|---|---|---|---|---|---|
| 1 | **Parquet on R2 (raw) + daily mergeable state Parquet + DuckDB queries** (DuckDB via HF Space or Worker-side batch) | cheap, immutable, full granularity, easy to recompute states, fits daily pipeline | DuckDB concurrency limits; DuckLake catalog or manual manifest needed; not interactive for many users | very low | 10^7/day fine with partitioning | good (files are durable; pipeline re-runnable) | **V2** |
| 2 | Same as 1 with **DuckLake** catalog (SQL catalog + Parquet) | snapshots, time travel, schema evolution, ACID | catalog to host; writes must go through DuckLake; ecosystem young | low | good | good | V2 or V3 (if snapshots needed early) |
| 3 | **ClickHouse** (self-hosted or cloud), AggregatingMergeTree + materialized views + TTL rollup | best interactive group-by, native rollup via TTL, quantile states | ops load; cascading-view pitfalls; TTL rollup bugs reported | medium–high | excellent | good if run well | **V3/V4** — when interactive multi-user slicing is needed |
| 4 | **Turso / libSQL** embedded replica of an operational DB | simple, SQLite everywhere, embedded reads | hosted-replica availability in question **[unverified]**; not for 10^7 history | low | low–medium | good | V2 for decision records, not for raw history |
| 5 | **Cloudflare D1** (sharded per month) for operational tables | in-Workers, free-tier friendly, 10 GB per DB | 10 GB cap per DB; sharding is app work | low | medium | good | V2 for decision/review tables |
| 6 | **Cloudflare Analytics Engine** for live counters | unlimited cardinality, Workers-native, cheap | sampling (weighted, adaptive), 3-month retention, one index, 16 KB blobs | low | very high ingest | not authoritative | V2 for live telemetry only, never as system of record |
| 7 | **Postgres (Supabase) partitioned tables + hand-written rollups** | fleet already has Postgres, relational joins for outcomes, familiar | rollups are our code; quantiles need our sketches; no native continuous aggregates | low–medium | medium | good | **V2 for outcome and review tables** |
| 8 | **TimescaleDB continuous aggregates** on Postgres | policy-driven rollups and retention | Supabase availability **[unverified]**; retention gotcha | medium | medium–high | good with care | V3 if Postgres path is chosen |
| 9 | **Druid / Pinot** | real-time OLAP, streaming ingest | heavy ops; over-spec | high | very high | good | V4 only |
| 10 | **Prometheus / Influx histograms** | ops-grade, mature dashboards | bucket-bounded quantile error; not for decision stats | low | high | good | infra health only, not decisions |
| 11 | **OTel GenAI + collector → any backend** (ingest format, not the store) | standard `gen_ai.*` names, portable | attribute set incomplete for fleet tags; backend is still needed | low (format) | depends | good | **V2 for naming**; store per option 1 |
| 12 | **Extend the existing roadmap-stats daily pipeline** (outputs into option 1) | reuses working code, no new service | batch only; not live | very low | fine at this scale | good | **V2 — yes** (brief §9 says extend, not replace) |

### 7.1 Recommended V2 shape

1. **Event log (source of truth):** one wide, append-only row per LLM step / attempt / tool call / decision, written as Parquet per UTC day to R2 (partition by day, then by provider). Columns: the `gen_ai.*` standard fields, `fleet.*` tags (§4.1), event time, ingest time, AA-version time, decision id, prompt version id. Schema version in every file.
2. **Mergeable states:** a daily job (the existing roadmap-stats pipeline) writes hour and day states per (group key) with count, sum, min, max, Welford mean/M2, a DDSketch-style histogram (α = 1%), and a value-count map for discrete fields. Recompute-from-raw is always possible.
3. **Query:** DuckDB reads the Parquet directly. The HF Spaces dashboards and the PM review surface use the same query layer. Quantiles use the state histograms, and exact quantiles fall back to raw data on demand.
4. **Outcomes and reviews:** append-only tables in Supabase Postgres (the queue's DB): `outcome(item, kind, observed_by, observed_at, window, evidence_url, confidence)`, `review_score(item_set, label, score, reviewer, at)`, `link(a, b, kind, observed_at)`. Edits are new rows; "current" is a view.
5. **Model lineage:** `dim_model` as Type 2 (surrogate key, valid_from, valid_to, is_current), with variants (DeepSeek 1107 vs classic, Luna 6 vs 6.1) as separate rows.
6. **Change markers:** `change_marker(kind, model_sk, pool, date, expected_delta, measured_delta_pre_post, window, owner)`; the measured delta is a new row when reviewed.
7. **Storage policy:** measure bytes/day for 30 days in V2, set retention after that (§6.4).
8. **Cloudflare Analytics Engine:** only for live Workers counters, with the sampling rules from its docs applied in every query.

Why this shape: it keeps every event (brief §6), makes all rollups recomputable from raw, needs no new server, reuses the daily pipeline (brief §9), keeps the three-month and sampling limits of Analytics Engine out of the source of truth, and keeps outcomes append-only (Langfuse lesson, §4.2). The upgrade path to ClickHouse (option 3) is a copy of the same Parquet into a MergeTree table when interactivity demands it.

---

## 8. Open questions and unverified items

1. Is DuckLake's Quack protocol stable enough to depend on, or should the catalog be a plain SQLite/Postgres file managed by the pipeline? **[unverified]**
2. Does Supabase expose TimescaleDB on our plan? **[unverified]**
3. Exact name and accuracy of ClickHouse `quantilesTDigestState` and whether the official t-digest error bound is documented; my fetch of the quantileTDigest docs page 404'd. **[unverified]**
4. Are Turso hosted edge replicas actually closed to new users? A single third-party page says so. **[unverified]**
5. Parquet compression ratio on our wide-event schema: must be measured. **[unverified]**
6. The Assisted-by trailer convention: the Linux kernel docs page was cited in a blog, not fetched. **[unverified]**
7. Druid/Pinot (§1.5): not researched in depth. **[unverified]**
8. The per-cell sample-size rule for p99.9 (§2.3) is a heuristic. **[unverified]**
9. GitClear figures are vendor data. Use as priors only.

---

## 9. Sources (fetched or search-confirmed in this pass)

- https://ducklake.select/ ; https://ducklake.select/faq.html
- https://news.ycombinator.com/item?id=44106934
- https://duckdb.org/2026/10/07/view-only-mode.html
- https://clickhouse.com/docs/reference/engines/table-engines/mergetree-family/aggregatingmergetree
- https://clickhouse.com/blog/using-materialized-views-in-clickhouse ; https://clickhouse.com/blog/chaining-materialized-views
- https://clickhouse.com/docs/concepts/features/operations/delete/ttl
- https://github.com/ClickHouse/ClickHouse/issues/70023
- https://stackoverflow.com/questions/77318126/aggregation-percentile-clickhouse
- https://turso.tech/pricing ; https://docs.turso.tech/features/embedded-replicas/introduction
- https://developers.cloudflare.com/d1/platform/limits/
- https://developers.cloudflare.com/analytics/analytics-engine/sampling/ (fetched)
- https://developers.cloudflare.com/analytics/analytics-engine/limits/ (fetched)
- https://prometheus.io/docs/practices/histograms/
- https://docs.influxdata.com/flux/v0/prometheus/metric-types/histogram/
- https://www.tigerdata.com/docs/reference/timescaledb/continuous-aggregates
- https://github.com/timescale/docs/blob/latest/use-timescale/data-retention/data-retention-with-continuous-aggregates.md
- https://www.tigerdata.com/blog/how-retention-policies-silently-break-continuous-aggregates
- https://en.wikipedia.org/wiki/Algorithms_for_calculating_variance
- https://arxiv.org/abs/1908.10693 (DDSketch)
- https://www.datadoghq.com/blog/engineering/computing-accurate-percentiles-with-ddsketch/ (fetched)
- https://arxiv.org/abs/1603.05346 (KLL, abstract fetched)
- https://opentelemetry.io/docs/specs/semconv/registry/attributes/gen-ai/ ; https://github.com/open-telemetry/semantic-conventions-genai
- https://openobserve.ai/blog/opentelemetry-genai-semantic-conventions/ (secondary)
- https://mlflow.org/docs/latest/genai/tracing/opentelemetry/genai-semconv/ (secondary)
- https://langfuse.com/docs/observability/data-model ; https://langfuse.com/docs/observability/features/sessions ; https://langfuse.com/faq/all/tracing-data-updates ; https://langfuse.com/docs/evaluation/scores/data-model ; https://langfuse.com/blog/2024-07-ai-agent-observability-with-langfuse
- https://openrouter.ai/docs/cookbook/administration/activity-export ; https://openrouter.ai/blog/announcements/activity-dashboard/ ; https://openrouter.ai/docs/cookbook/administration/usage-accounting ; https://openrouter.ai/docs/cookbook/administration/data-api
- https://stripe.com/blog/canonical-log-lines ; https://www.honeycomb.io/blog/time-to-version-observability-signs-point-to-yes
- https://www.kimballgroup.com/data-warehouse-business-intelligence-resources/kimball-techniques/dimensional-modeling-techniques/ ; https://en.wikipedia.org/wiki/Slowly_changing_dimension ; https://learn.microsoft.com/en-us/power-bi/guidance/star-schema ; https://datawarehouseinfo.com/foundations/dimensional-modeling/ ; https://dataskew.io/blog/dimensional-modeling/
- https://dora.dev/guides/dora-metrics/ ; https://dora.dev/insights/dora-metrics-history/ ; https://cloud.google.com/blog/products/devops-sre/using-the-four-keys-to-measure-your-devops-performance
- https://www.gitclear.com/ai_assistant_code_quality_2025_research ; https://www.gitclear.com/the_ai_code_quality_maintainability_gap (vendor)
- https://williamhuster.com/ai-attribution-assisted-by/ ; https://github.com/cue-lang/cue/issues/4452 ; https://crashoverride.com/resources/knowledge-base/code-ownership/attributing-ai-commits-git
- https://www.truefoundry.com/blog/openrouter-usage (secondary)
- https://www.buildmvpfast.com/alternatives/turso (secondary; Turso claim unverified)

Prototype: `prototypes/stats_rollup_oracle.py`, run output in the final message of the subagent session.
