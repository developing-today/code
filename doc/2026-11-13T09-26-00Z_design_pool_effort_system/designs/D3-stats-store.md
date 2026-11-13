# D3 — Granular stats store and data basis

- Program: pool & effort system (code#9399, queue item 15864). Parent: `../main.md`.
- Status: draft for the design mill. Detached drafting: no question was asked of anyone;
  every open point is in §7 with the default chosen.
- Branch / directory: `design/pool-effort-system`, `/home/user/git/wt-mill-pools`.
- Themes: T5 (data basis for the effort mapping), T6 (granular stats store).
- Binding inputs: main.md **AD7** (every decision and attempt is an addressable event; append-only
  feedback) and **AD8** (stats store extends the ledger, full granularity, measures growth, mergeable
  rollups, every quantile carries `n`, Type-2 `dim_model`, `change_marker`). Versions: V2 = store and
  export; V3 = measured use in D1/D4 and the Roadmap & Stats consumer; V4 = out of scope here.
- Evidence: `baseline/v1-data-sources.md` (S1–S14, gaps G1–G16), `research/R3-stats-store-sota.md`,
  `research/prototypes/stats_rollup_oracle.py` (re-run 2026-11-13: PASS, 20 seeds, max quantile
  relative error 0.009997 against α = 0.01). Live ledger queries were run read-only (`sqlite3 -readonly`)
  for this design; counts are as of 2026-11-13 ~06:30 CT and drift while the DB is live.

---

## 1. Scope

| Brief / supplement section | What D3 owns |
| --- | --- |
| brief §5 (data for the effort mapping, all of it) | the fact tables and the query set (§4.9) that make each §5 dimension answerable; the statistics set (§4.4) |
| brief §6 (granular stats store) | the star schema, tag model, `dim_model` with lineage and AA version, `change_marker`, aggregates-as-events, storage growth measurement, rollup policy |
| brief §7 (cost efficiency, real vs AA) | cost basis and provenance (§4.8) — the cost axis D1/D4 read |
| brief §8 (trials) | the trial/change-marker data shape D4 uses (`change_marker` with expected vs actual) |
| brief §9 (V1 inputs) | how Roadmap & Stats and the quota analytics page consume the store (§4.7) |
| supplement §5 (outcome capture), §9 (decision record) | the **storage shape** only: `fact_outcome` (append-only), `fact_link`, and the `fact_decision` projection of D5/D6's `decision_record`. The field semantics belong to D6/D5 |

Statement ids: `inventory/` is empty in this package, so statements are mapped by brief/supplement
section (§/line). The coverage table is in §8.

Out of scope: the decision record's content (D5/D6), the outcome-capture UX (D6), the effort tier
table (D1), the trial design itself (D4). D3 provides the data each needs.

---

## 2. V1 — what exists today (documented, not redesigned)

Citations are from the baseline audit (S-ids) and from source/live schema.

**Ledger tables** (live `opencode.db`, `sqlite3 -readonly .schema`; creation migration
`packages/core/src/database/migration/20261006083034_model_route_log.ts` in `~/git/opencode`,
the fork):

| Table | Rows (2026-11-13 ~06:30) | Grain | Written by |
| --- | ---: | --- | --- |
| `route_attempt` | 649,414 | one model call, incl. failover legs | `bin/opencode-route-proxy.mjs:506` `recordAttempt` → `INSERT` at `:511`; called at `:1254, :1281, :1307, :1333, :1357, :1386` |
| `route_decision` | 2,428,940 | one pool selection (candidates, chosen, `detail.weights`, `detail.excluded[]`) | fork route selector (migration `20261006083034`); no writer in this repo |
| `route_health` | 63,014 (baseline) | cooldown / quota gate events, with `credential_id` | route proxy |
| `route_note` | exists in fork schema | operator notes to the router | fork migration (not in baseline S-list; flagged, §7 Q-D3-9) |
| `session_v2` | 19,629 (3,423 children) | session; token/cost totals; `parent_id`; `time_*` | OpenCode core |
| `session_message` | 619,611+ | turn (`assistant`), `user`, `synthetic`, `idle`, `compaction`, … ; `data.content[]` holds `text`, `reasoning`, `tool` parts with `state.status` (`completed` 221,827 / `error` 3,140 / `running` 120 / `streaming` 6 since 2026-11-06); `data.time` = `{created, streamed, completed}` | OpenCode core v2 |
| `message`, `part` | legacy, frozen at 2026-10-06 01:23 | — | do not read for new data (baseline G2) |

`route_attempt` columns (live): `id, time_started, time_ended, session_id, assistant_message_id,
route_id, provider_id, model_id, variant, outcome, error_tag, error_code, error_status,
error_message, retryable, output_started, failed_over_to, first_token_ms, response_ms,
tokens_per_second, tokens_input, tokens_output, tokens_reasoning, tokens_cache_read,
tokens_cache_write, tokens_estimated, error_body, hedged, cost_usd, continuation_of,
continuation_index, cache_savings_usd, credential_id, quota`.

**Facts that differ from the baseline and that this design must use** (measured 2026-11-13):

1. `quota` JSON is **not one shape**. Four shapes occur: `{"at":ms,"reset":{"unified-5h":…,"unified-7d":…}}`
   (≈29.5 k rows carry `reset`), `{"retryAfterMs":…,"headers":{…}}` (≈23.6 k), `{"headers":{…}}`
   (≈5.7 k), and `{"at":ms,"limit":{"requests":"3000","tokens":"4000000"},"remaining":{…}}`
   (≈2.4 k). **No row carries a percent.** `quota` is therefore parsed into typed columns, and the
   percent comes only from the S4 snapshot join (§4.8).
2. `cost_usd > 0` on 50,367 of 649,414 rows (7.8 %); 205,231 rows have `cost_usd` NULL, the rest are 0.
   Every row with `cost_usd > 0` has token counts, so it is a calibration set for the derived cost (§4.8).
3. `provider_id` is **not normalised**: `_openai.main` (79,876 rows since 2026-11-06), `openai.main`
   (75,151), `_openai` (17,949), `openai.alternate` (6,385); `_claude`, `_vibe`, `opendesign`,
   `_commandcode`, `_muse`, `_antigravity`, `opencode-go.developing-today-0`, `opencode-go.popesdrew`,
   `opencode-go.drewrypope`, `vibe-direct`, `cline-pass`. Credentials are in `credential_id`. A
   `dim_provider` / `dim_door` mapping is required before any cross-provider grouping.
4. DeepSeek ids in the ledger: `deepseek-v4-1-flash-1017` (`_vibe` 22,615; `mistral` 203),
   `deepseek-v4.1-flash` (classic naming; `opendesign` 19,135; `opencode-go.*` 17,000+; `cline-pass` 4,218),
   `deepseek/deepseek-v4.1-flash` (`_commandcode` 13,226, slash-prefixed). **The string `1107` never
   appears as a `model_id` or `variant`** (0 rows). The brief's "DeepSeek 1107 vs classic" therefore has
   no ledger key yet (§7 Q-D3-1).
5. `variant` values: `xhigh` 308,239 · `high` 255,692 · `medium` 46,417 · `low` 27,934 · NULL 6,212 ·
   `default` 4,387 · `max` 496 · `none` 34. Remaps are not recorded in this column (baseline Fact 2).
6. Sub-session depth is computable: recursive walk over `parent_id` gives depth 0: 16,206 · 1: 2,944 ·
   2: 343 · 3: 93 · 4: 26 · 5: 15 (max depth 5; the 16,206 roots are all sessions with `parent_id IS NULL`).
7. Turn timing is in `session_message.data.time`: `streamed − created` is a TTFT proxy (mean 10,860 ms over
   190,353 assistant turns since 2026-11-06); `completed − created` is step time (mean 15,991 ms).
8. Route decisions are the storage driver, not attempts. `route_decision` averages ≈1,350 bytes/row
   (`candidates` JSON + `detail` JSON). Daily rows since 2026-11-06 (live): 8,845 · 73,311 · 210,624 ·
   223,768 · 154,841 · 139,749 · 131,225 · 110,984 (partial). That is about **0.1–0.3 GB/day of route_decision
   text**, against about 0.01 GB/day for `route_attempt` (45 k rows × ≈230 B). The baseline's growth
   estimate (§4 of the audit) understates route_decision by roughly 10×.
9. Attempt grain carries `session_id` for about 635 k rows (live count 635,064 at 06:30 CT); about 14.6 k are
   NULL (test fixtures per the audit). They are kept as `unattributed`, not dropped.
10. S4 snapshot readings are keyed by `serviceId` + `metricId` + `unit`, not by pool. Examples:
    `opencode-go` (`5h`, `weekly`, `monthly`, `pct`), `openai:codex` (`weekly`, `pct`), `antigravity`
    (per-model `<model>:5h`, `pct`), `mistral` (`vibe`, `pct`), `opendesign` (`5h`, `pct`), `turso`
    (infra; not a route door). 265 distinct service/metric/unit/source combinations in Oct–Nov.
11. AA prices (`~/.local/state/pm-aggregator/aa-last-good.json`, `section.prices`, 13 rows) use
    OpenRouter-style names (`anthropic/claude-haiku-4.5`, `anthropic/claude-opus-5.5`). Route `model_id`s
    (`gpt-6-luna`, `deepseek-v4.1-flash`) do not match them. A **crosswalk** table is required (§4.2).
12. Code#9373 (fleet invocation ledger) and code#9374 (mcpx daemon call ledger) are still the only gaps
    for per-subcommand and per-action usage (baseline G1, G3). Until they land, tool usage is inferred
    from `session_message` tool parts (§4.2 `fact_tool_call.source`).

**Existing consumers.** `pm-stack/aggregator/src/analytics-store.ts` opens `route_attempt` and
`session_v2` / `session_message` read-only with `bun:sqlite` (`:228`, `:246`; `:298–324` uses
`json_each(sm.data,'$.content')` for tool and skill counts). `quota-analytics.ts` holds the pure
projection math. The `/quota#analytics` view is `pm-stack/aggregator/src/quota-analytics-view.ts`. Nothing
computes p90/p99/p99.9, medians or mode; `Model_TestVersion_Stats` in the AA CSV is the only quartile/stdev
rollup and it covers AA metrics only.

**Export precedent.** `pm-stack/cloudflare-export/` (`cloudflare-quota-export.mjs`, `.sh`, `.service`,
`.timer`, `install-laptop.sh`, `README.md`) is a laptop systemd timer that writes derived quota data. D3's
exporter copies that shape.

---

## 3. Options considered

Each choice lists at least four options. Pros and cons are on the six axes the spec names
(architecture, scalability, performance, reliability, extensibility) plus cost. Prior art cites R3 sections.

### 3.1 Storage engine for the granular store (V2)

| Option | Architecture | Scale (10⁵–10⁷ rows/day) | Performance | Reliability | Extensibility | Cost | Prior art |
| --- | --- | --- | --- | --- | --- | --- | --- |
| **A. Parquet per day on laptop (+R2 mirror), DuckDB views** | files are the source; immutable; re-runnable | fine (10⁷/day with day partitions) | batch and rollup fast; interactive per-user fine | files durable; pipeline re-runnable | new columns = new Parquet schema version | ~0 (disk, one R2 bucket) | R3 §1.1, §7 option 1 |
| B. DuckLake catalog + Parquet | snapshots, time travel | good | good | catalog to host | schema evolution built in | low; catalog ecosystem young | R3 §1.1, option 2 |
| C. ClickHouse MergeTree + AggregatingMergeTree | server | excellent | best interactive | ops load; TTL-rollup bugs (#70023) | strong | medium–high | R3 §1.2, option 3 |
| D. Cloudflare D1 sharded by month | edge SQLite | 10 GB per DB cap; sharding is app work | fine for operational tables | good | medium | low | R3 §1.3, option 5 |
| E. Supabase Postgres partitioned + hand-written rollups | relational | medium | good for outcome joins | good | good for outcomes; rollups are our code | low–medium | R3 §1.7, option 7 |
| F. Stay on live SQLite (read-only views over `opencode.db`) | in-place | 21.5 GB file, WAL 66 MB, bak snapshots 21 GB each | slow scans (`session_message` is 7.7 GB) | reads contend with the live writer | no history beyond the live file | none | the status quo |

**Chosen: A** for the event and rollup store, **E** for outcome and review rows (they join to issues and
PRs and are small and append-only). D (D1) only for hot decision records if D5 needs them. C is the upgrade
path (same Parquet copied into a MergeTree) if interactive multi-user slicing is ever needed; not V2.
F is rejected as the store: it is the source, not the store. It is read live by the proxy and the pm-stack, and a
full scan of `session_message` (7.7 GB) competes with that writer.

### 3.2 Event grain and fact set

| Option | Shape | Pros | Cons |
| --- | --- | --- | --- |
| 1. One attempt row only (`route_attempt` as-is) | 1 row / call | already exists | no tool, turn, session or decision grain; the brief §5 list cannot be answered |
| 2. One wide canonical line per **session** (Stripe style) | 1 row / session | simple | cannot group by attempt or tool; sums lose order |
| 3. One row per **turn / step** | turn grain | TTFT and step counts | tools and attempts still separate |
| 4. Multi-grain star: attempt, turn, tool call, session, route decision, decision, outcome, link, change marker | several facts, shared keys | answers every §5 row; each grain stays narrow; joins via ids | more export code; joins at query time |
| 5. One mega-table of everything (EAV) | 1 row per fact per tag | generic | unreadable; tags and values blur; no types |

**Chosen: 4.** Grain is fixed per table (Kimball, R3 §3.1). The brief's "group-by any tag" is served by a
long **tag assignment** table joined on `(event_kind, event_id)`, not by a mega-table (option 5 rejected).

### 3.3 Quantile method

| Option | Mergeable | Error | Tails (p99.9) | Size | Verdict |
| --- | --- | --- | --- | --- | --- |
| Exact from raw on demand | n/a (recompute) | 0 | exact | O(n) | **Chosen as the oracle and on-demand path** |
| **DDSketch log buckets, α = 1 %** | yes (sum counters) | relative ≤ α (oracle: max 0.009997) | yes | ~2 kB per group (Datadog, R3 §2.2) | **Chosen for stored states** |
| t-digest | yes, approx | weaker, not formal (R3 §2.2, unverified) | good | comparable | rejected: no formal bound for the report |
| KLL | yes | additive rank error (1 % rank = large p99.9 error) | poor at tails | small | rejected for tails |
| HDR Histogram | yes | relative | yes | larger than DDSketch (Datadog) | rejected: larger |
| Prometheus fixed buckets | yes | bounded by bucket width | poor | small | rejected: bucket-bounded, not for decisions (R3 §1.6) |
| GK | yes | rank error; too large at p99 (Datadog) | poor | small | rejected |

Quantile definition: **nearest rank** (`ceil(q·n)`-th smallest). Stored in every state as
`quantile_method = nearest_rank_v1`. The oracle, the SQL (§4.9 Q15) and the sketch use this rule, so they can
be compared exactly.

### 3.4 Dimension history (model, door, pool, provider)

| Option | Keeps history of meaning? | Verdict |
| --- | --- | --- |
| Type 1 (overwrite) | no | rejected for anything that changes meaning (variant, AA version, price) — brief §6 "fully separate" |
| **Type 2 (new surrogate row on change)** with valid_from/valid_to and lineage | yes | **Chosen** for `dim_model`, `dim_door`, `dim_pool` (R3 §3.2) |
| Type 3 (previous-value columns) | one step only | rejected: lineage is longer than one step |
| Bitemporal everywhere (valid time + transaction time) | yes | **partly chosen**: event time, ingest time and AA-version time are kept as three clocks on every fact (§4.3.5); full bitemporal dims are V3 |

### 3.5 Tag model (group by any tag or label)

| Option | Group by any tag | Cost | Verdict |
| --- | --- | --- | --- |
| Wide typed columns only | only pre-decided tags | cheap | rejected as the only way: the brief wants any tag |
| JSON map column (`tags MAP(VARCHAR,VARCHAR)`) | yes, via `UNNEST` | fine in DuckDB | considered; slower for facet joins |
| Long `tag_assignment` table joined on event id | yes, standard join | medium rows | **Chosen**, plus typed hot keys on facts |
| Tags only inside a label string | no | cheap | rejected |

Hot keys (typed columns on facts): day, provider_norm, model_sk, variant_key, effort_tier, effort_raw, route_id,
door_sk, outcome, error_tag, session_id, root_session_id, depth, agent, task_id, pr, issue, aa_version_key.
Everything else is a tag row (key namespace in §4.2.3).

### 3.6 Cost basis

| Option | Covers subscription traffic | Honest about provenance | Verdict |
| --- | --- | --- | --- |
| Recorded `cost_usd` only | no (50,367 rows, 7.8 %; subscription = 0) | yes | rejected as the axis (baseline G4) |
| **Derived: tokens × AA price table, with `cost_basis` and `price_version`; two columns: list-equivalent and paid** | yes | yes | **Chosen** |
| OpenRouter activity export as cost | only OpenRouter doors | yes | reconciliation check only (R3 §4.3) |
| Scrape provider price pages | yes | weak | rejected: not reproducible |
| Ask each provider for usage cost at call time | yes | yes | V3 (needs provider support) |

### 3.7 Quota percent per attempt

| Option | Attribution | Verdict |
| --- | --- | --- |
| Status quo: reset instants only | none | rejected: brief §5 "quota % per attempt" is not answerable |
| **Nearest prior S4 reading for the door's window + nearest following reading (time-window join)** | inferred | **Chosen for V2**, flagged `inferred` (baseline Q-src-6) |
| Interval interpolation between readings | inferred | rejected for V2: readings are sparse; interpolation invents values |
| Record the percent at call start and end in the proxy | exact | **V3 slice** (needs a proxy change in the fork; out of V2) |
| Infer from token counts × published limits | modelled | rejected: limits are not public per door |

### 3.8 Rollup and retention policy

| Option | Keeps detail | Rollup clocks | Verdict |
| --- | --- | --- | --- |
| Keep everything forever | yes | none | rejected: 0.1–0.3 GB/day of route_decision text grows without bound |
| TTL with GROUP BY (ClickHouse-style) | rolls up then drops | one clock (event time) | rejected for V2 (no ClickHouse); the idea is kept |
| Timescale-style policy (raw 30 d, daily 600 d) | yes | one clock | rejected: retention-gotcha risk (R3 §1.7) and no Postgres store for raw |
| **Keep raw Parquet; mergeable states by three clocks (event, capture, AA version) and by inject marker; retention decided after 30 days of measured bytes/day** | yes until measured | three clocks | **Chosen** (§4.6) |

### 3.9 Where the export runs

| Option | Pros | Cons | Verdict |
| --- | --- | --- | --- |
| **Laptop systemd timer, mirroring `pm-stack/cloudflare-export/`** | the DB and the state dir are on this machine; no secret needed | one machine; laptop must be on | **Chosen** |
| Cloudflare Worker cron reading from the DB | runs off-laptop | the DB is not on Cloudflare; would need a copy anyway | rejected |
| GitHub Actions nightly | familiar | CI on main only (standing rule); the DB is not in CI | rejected by standing rule |
| In-process write from OpenCode (fork change) | exact timing | fork change for every table; couples the core to analytics | rejected for V2; the decision-record write (D5/D6) is already in-process and stays so |

---

## 4. V2 design

### 4.1 The ledger that D3 reads (unchanged; additions named)

D3 adds **no** write to `route_attempt` or `route_decision`. Two additions are needed from other components and
are named here so the dependency is explicit:

- `decision_record` (AD7, D5/D6): a new table beside `route_attempt`. D3 reads it; its columns are D5/D6's.
- `route_attempt.effort_tier`, `route_attempt.native_variant` and `remap_declared` (D1). Until D1 lands,
  D3 derives `effort_tier` from `variant` through the rung table as dimension data (§4.2.4), not from the row.

Code#9373 (fleet ledger) and code#9374 (mcpx ledger) are **not** D3's to build; D3 defines the contract D3
will read (§4.2.5) and marks its fact rows `source = inferred` until they land.

### 4.2 Schema

Notation: `sk` = surrogate key (integer), `ms` = epoch milliseconds, all times UTC unless named.
Every fact row carries `schema_version` (integer, starts at 1), `ingested_at_ms`, and the three clocks
(§4.3.5).

#### 4.2.1 Facts

**`fact_attempt`** — grain: one row per `route_attempt.id`. Source: `route_attempt` joined to dims.

| Column | Type | Source / rule |
| --- | --- | --- |
| `attempt_id` | BIGINT PK | `route_attempt.id` |
| `day` | DATE | `time_started` in UTC |
| `time_started_ms`, `time_ended_ms` | BIGINT | as recorded |
| `session_id`, `assistant_message_id` | VARCHAR | as recorded (NULL → `unattributed`) |
| `root_session_id`, `session_depth` | VARCHAR, SMALLINT | from `fact_session` |
| `route_id` | VARCHAR | pool id |
| `door_sk` | INTEGER | `dim_door` by `(provider_id, credential_id, time_started)` |
| `provider_raw`, `provider_norm` | VARCHAR | raw from `provider_id`; normalised via `dim_provider` |
| `model_sk` | INTEGER | `dim_model` (Type 2) by `(provider_norm, model_id, time_started)` |
| `model_id_raw` | VARCHAR | as recorded (keeps `deepseek/…` prefix forms) |
| `variant_key` | VARCHAR | `dim_model.variant_key` (classic, snapshot-1017, …) |
| `effort_raw` | VARCHAR | `variant` as recorded (NULL allowed) |
| `effort_tier` | ENUM(low, medium, high, unmapped) | `dim_effort` (AD1); `unmapped` = not in rung table |
| `native_variant` | VARCHAR | what the door actually received (rung table; NULL if unknown) |
| `remap_declared` | BOOLEAN | rung-table row exists with `declared = true` |
| `outcome` | ENUM(success, failure, timeout, interrupted) | as recorded |
| `error_tag`, `error_code`, `error_status`, `retryable` | VARCHAR, VARCHAR, SMALLINT, BOOLEAN | as recorded |
| `failed_over_to` | VARCHAR | as recorded |
| `output_started` | BOOLEAN | as recorded |
| `first_token_ms`, `response_ms`, `tokens_per_second` | DOUBLE | as recorded |
| `tokens_input`, `tokens_output`, `tokens_reasoning`, `tokens_cache_read`, `tokens_cache_write` | BIGINT | as recorded; `tokens_estimated` BOOLEAN |
| `cost_usd_recorded` | DOUBLE NULL | `route_attempt.cost_usd` (NULL where NULL) |
| `cost_list_usd` | DOUBLE NULL | derived at list price (§4.8) |
| `cost_paid_usd` | DOUBLE NULL | recorded if present; 0 for plan doors; else NULL |
| `cost_basis` | ENUM(recorded, derived-aa, subscription-zero, unknown) | §4.8 |
| `price_version` | VARCHAR | AA batch id used for the derivation |
| `quota_reset_5h_ms`, `quota_reset_7d_ms` | BIGINT NULL | parsed from `quota` (`reset.unified-5h/7d` when present) |
| `quota_limit_requests`, `quota_limit_tokens`, `quota_remaining_requests`, `quota_remaining_tokens` | BIGINT NULL | parsed from `quota.limit` / `quota.remaining` |
| `quota_shape` | ENUM(reset, retry-after, headers-only, limit-remaining, none) | which of the four observed shapes |
| `quota_pct_before`, `quota_pct_after`, `quota_pct_delta` | DOUBLE NULL | S4 join (§4.8), always with `quota_pct_source = 'snapshot-window-join'` and `inferred = true` |
| `credential_id` | VARCHAR NULL | as recorded |
| `continuation_of`, `continuation_index` | BIGINT, INTEGER | as recorded |
| `aa_version_key` | VARCHAR | from `dim_model` in force at `time_started` |
| `change_marker_ids` | LIST(VARCHAR) | markers whose window contains `time_started` (§4.3.4) |
| `schema_version`, `ingested_at_ms` | | |

**`fact_route_decision`** — grain: one row per `route_decision.id`.

| Column | Type | Rule |
| --- | --- | --- |
| `decision_id` (= `route_decision.id`), `time_ms`, `day` | | as recorded |
| `session_id`, `root_session_id` | VARCHAR | as recorded |
| `route_id` | VARCHAR | pool |
| `selection` | ENUM(ordered, weighted, round-robin, drawn, …) | as recorded; the value set is open, so unknown values are kept as-is |
| `variant_requested` | VARCHAR | `route_decision.variant` |
| `effort_tier_requested` | ENUM(low, medium, high, unmapped) | from `variant_requested` via `dim_effort` |
| `pools_offered` | LIST(VARCHAR) | `candidates` JSON array (observed: pool ids such as `opencode-route/workhorse-ordered`) |
| `pool_chosen` | VARCHAR | `chosen` |
| `reason` | VARCHAR | as recorded (`adjusted`, `drawn`, `sticky`, …) |
| `excluded` | LIST(STRUCT(candidate VARCHAR, reason VARCHAR)) | `detail.excluded[]`; reason prefixes observed: `pool-quota-exhausted:<pool>`, `cooldown-until:<ms>`, `quota-skip` |
| `weights` | MAP(VARCHAR, DOUBLE) | `detail.weights` |
| `layers` | JSON | `detail.layers` kept as JSON (empty in the sample) |
| `schema_version`, `ingested_at_ms` | | |

**`fact_session`** — grain: one row per `session_v2.id`. Built by walking `parent_id`.

| Column | Type | Rule |
| --- | --- | --- |
| `session_id`, `parent_id`, `root_session_id` | VARCHAR | `root` by recursive walk |
| `depth` | SMALLINT | 0 = root; observed max 5 |
| `direct_children`, `descendants` | INTEGER | counts |
| `agent`, `project_id`, `directory` | VARCHAR | as recorded (`directory` can be hashed for export if sensitive; default: keep) |
| `title` | VARCHAR | as recorded (default: keep; see §7 Q-D3-7) |
| `time_created_ms`, `time_updated_ms`, `time_compacting_ms`, `time_archived_ms`, `time_idle_ms` | BIGINT | as recorded |
| `active_minutes_5m` | DOUBLE | sum of gaps ≤ 5 min between consecutive `assistant` turns (§4.9 Q12) — default threshold 5 min (assumption, §7) |
| `turns`, `tool_calls`, `user_messages`, `compactions` | INTEGER | from `session_message` counts |
| `tokens_input`, `tokens_output`, `tokens_reasoning`, `tokens_cache_read`, `tokens_cache_write` | BIGINT | `session_v2` totals |
| `cost_recorded` | DOUBLE | `session_v2.cost` |
| `cost_list_usd`, `cost_paid_usd` | DOUBLE | sums of `fact_attempt` (§4.8) |
| `diff_additions`, `diff_deletions`, `diff_files` | INTEGER NULL | `summary_*` (sparse: 468 rows in baseline) |
| `model_json`, `pools_used` | JSON, LIST | `session_v2.model` JSON; pools from `fact_attempt.route_id` |
| `resume_attempts`, `subagent_policy`, `detached`, `idle_outcome` | | as recorded |

**`fact_turn`** — grain: one row per `session_message` with `type = 'assistant'`.
`turn_id`, `session_id`, `seq`, `time_created_ms`, `time_streamed_ms`, `time_completed_ms`,
`ttft_proxy_ms = streamed − created`, `step_ms = completed − created`, `agent`, `model_id`, `provider`,
`variant`, `tool_part_count`, `reasoning_chars`, `text_chars`. (TTFT here is the **session-level** proxy;
`fact_attempt.first_token_ms` is the proxy's own number.)

**`fact_tool_call`** — grain: one row per tool part (`content[].type = 'tool'`), and one row per
`tools.<ns>.<tool>(` occurrence inside an `mcpx_exec` script (`source = 'script-parse'`).

| Column | Type | Rule |
| --- | --- | --- |
| `tool_call_id` | VARCHAR | `session_message.id` + index in `content[]`, or + occurrence index for script-parse |
| `session_id`, `root_session_id`, `turn_id` | VARCHAR | |
| `time_ms` | BIGINT | `session_message.time_created` |
| `tool_name` | VARCHAR | `content[].name` (e.g. `shell`, `execute`, `read`, `mcpx_exec`, `subagent`) |
| `tool_family` | ENUM(native, mcpx, fleet, other) | `mcpx_*` → mcpx; `shell` with a `fleet` command → fleet |
| `mcpx_ns`, `mcpx_tool` | VARCHAR NULL | script-parse: `tools.<ns>.<tool>`; bracket form `tools["ns"]["tool"]` also parsed |
| `fleet_sub`, `fleet_action` | VARCHAR NULL | regex `\bfleet\s+([a-z][a-z-]*)(?:\s+([a-z][a-z-]*))?` over `state.input.command` (inferred until code#9373) |
| `status` | ENUM(completed, error, running, streaming) | `state.status` |
| `input_bytes`, `output_bytes` | BIGINT | lengths; no content stored |
| `source` | ENUM(session-message, script-parse, fleet-ledger, mcpx-ledger) | `fleet-ledger` / `mcpx-ledger` appear when code#9373/#9374 land |

**`fact_decision`** — projection of D5/D6's `decision_record` (AD7). D3 defines the **columns it needs**;
D5/D6 own the values. Required: `decision_id`, `time_ms`, `decision_kind`, `prompt_id`, `prompt_version`,
`prompt_date`, `decision_model_sk`, `answering_leg`, `leg_index`, `invalid_json_retries` (0–2),
`default_answer_used` (bool), `tokens_input`, `tokens_output`, `percent_result` (JSON),
`time_to_decision_ms`, `effort_tier` (lean, supplement §6), `session_id`, `parent_session_id`,
`task_id`, `parent_decision_id`, `schema_version`. Points gauges (0–5) and outcomes are **not** in this table;
they are `fact_feedback` rows (D6) and `fact_outcome`.

**`fact_outcome`** — append-only (AD7; Langfuse scores pattern, R3 §4.2). Grain: one row per observation.

| Column | Type | Rule |
| --- | --- | --- |
| `outcome_id` | VARCHAR | ULID-like; generated on append |
| `subject_kind` | ENUM(pr, issue, task, decision, session, attempt, commit) | |
| `subject_id` | VARCHAR | `owner/repo#n`, task id, decision id, session id, attempt id |
| `outcome_code` | ENUM (closed; §4.2.2) | |
| `observed_by` | ENUM(self, parent, reviewer, system, user) | supplement §9 perspectives |
| `observed_at_ms` | BIGINT | |
| `window_days` | SMALLINT | the window the code refers to (e.g. revert within 14 d) |
| `evidence_url` | VARCHAR | GitHub URL, commit, or log pointer |
| `confidence` | ENUM(mechanical, inferred, asserted) | revert = mechanical; rework = inferred (file overlap) |
| `supersedes` | VARCHAR NULL | a later observation may name an earlier one; "current" is a view, never an edit |
| `schema_version`, `ingested_at_ms` | | |

**`fact_feedback`** (D6, shape only): `feedback_id`, `decision_id`, `observer` (caller|callee|reviewer),
`points_correct` (0–5), `points_wrong` (0–5), `note`, `observed_at_ms`. Append-only.

**`fact_link`** — append-only join: `link_id`, `from_kind`, `from_id`, `to_kind`, `to_id`,
`link_kind` (`commit-trailer`, `pr-body`, `issue-ref`, `task-session`, `attempt-task`, `revert-of`, `supersedes`),
`observed_at_ms`, `evidence_url`. Commit trailers follow R3 §5.2: proposed `Assisted-by:`, `Fleet-Task:`,
`Fleet-Session:`, `Fleet-Decision:` (design proposal, not a standard).

#### 4.2.2 Outcome codes (closed enum; each has a definition)

Lifecycle: `merged`, `closed-incomplete`, `canceled`, `not-pushed`, `issue-resolved`, `issue-made`.
Quality: `reverted`, `rework`, `undone`, `duplicate-work`, `caused-incident`, `data-loss`, `invalid`,
`unclear`, `misunderstood`, `too-big`, `overcomplicated`, `oversimplified`, `perfect`, `simple`, `safe`,
`fixed`, `partial`, `cheaper-than-expected`, `unneeded-worry`, `overtested`, `undertested`,
`significant-expense`, `slow`, `heavy`, `network`, `not-scalable`.

Adding a code is a schema change (new `schema_version` note in `dim_outcome_code`); renaming is forbidden.
Each code's definition lives in `dim_outcome_code` (one row each, with `confidence` default).

#### 4.2.3 Tags

**`tag_assignment`** — long table: `(event_kind, event_id, tag_key, tag_value, source, valid_from_ms, valid_to_ms)`.
`source` ∈ {`derived`, `config`, `label`, `dim`}. Dimension tags are on the dimension and reach facts by join,
so they are not copied per event (a change in a tag is a new `dim_model` row, Type 2).

Key namespaces (the brief §6 list, made concrete):

| Namespace | Examples | Where it lives |
| --- | --- | --- |
| `cap.*` | `cap.tools=yes`, `cap.reasoning=yes`, `cap.context=1m`, `cap.vision=no` | `dim_model` tags (`source = dim`) |
| `match.price` / `match.ability` / `match.speed` | `match.price=workhorse/luna@100k#medium` | `dim_model_match` (§4.2.4) |
| `door.plan` / `door.payg` / `door.free` / `door.promo` | | `dim_door` |
| `peak` | `peak=offpeak`, `peak=peak` | derived from the window rules of the door (`model_route_windows` config) |
| `lineage.step` | `lineage.step=2` | `dim_model_lineage` |
| `aa.version`, `aa.test_date` | `aa.version=5.0.1`, `aa.test_date=2026-10-31` | `dim_model` |
| `change.marker` | `change.marker=mk-…` | `change_marker` link |
| `label.*` | `label.misunderstood`, `label.perfect` | `fact_outcome` / labels (D6) — tag_assignment source = `label` |
| `session.kind` | `session.kind=subsession`, `session.kind=root` | derived |

#### 4.2.4 Dimensions

**`dim_model`** (Type 2; the grain is one row per *meaning*: model × variant × AA version in force).

| Column | Type | Note |
| --- | --- | --- |
| `model_sk` | INTEGER PK | surrogate |
| `provider_norm` | VARCHAR | e.g. `deepseek` (not the door) |
| `family` | VARCHAR | e.g. `deepseek-v4.1-flash`, `gpt-6-luna`, `gpt-6.1-terra` |
| `model_id_raw` | VARCHAR | the exact string(s) the ledger uses for it (list in `dim_model_alias`) |
| `variant_key` | VARCHAR | e.g. `classic`, `snapshot-1017`, `snapshot-1107` (**provisional**, §7 Q-D3-1) |
| `lineage_seq` | INTEGER | position in the family's ordered lineage (1 = oldest) |
| `predecessor_sk`, `successor_sk` | INTEGER NULL | the adjacent lineage rows (ordered); the full chain is `dim_model_lineage` |
| `aa_version_key`, `aa_test_date`, `aa_snapshot_date` | VARCHAR, DATE | AA version and test date in force for this row (brief §6) |
| `aa_row_ref` | VARCHAR | AA `testRunKey` / `modelVariantKey` (aa-last-good) |
| `price_in_per_1m`, `price_out_per_1m`, `price_cache_read_per_1m`, `price_cache_write_per_1m` | DOUBLE | from `prices` (AA); NULL when AA has no row |
| `price_version` | VARCHAR | AA batch id |
| `context_window`, `max_output` | BIGINT | native limits (AD4 input) |
| `valid_from_ms`, `valid_to_ms`, `is_current` | | Type 2 |
| `source` | ENUM(aa, own-measure, config, ruled) | provenance |

**`dim_model_lineage`**: `family, seq, model_sk, successor_of_sk, reason ∈ {replaced-by, renamed,
variant-of, dominated, unknown}, decided_at, source`. Ordered by `seq`. Predecessors and successors are
the same table read in both directions, so a fact can always reach "what came before/after it".

**Variant rules (brief §6, all fixed by this design):**

- **DeepSeek 1107 vs classic:** variant rows under one `family`, distinguished by `variant_key`. Split by the
  variant **each provider actually serves** (the ledger's `model_id_raw` tells us which). A door that serves
  both is two `dim_door`-level rows in `dim_model_alias`. The brief's pool order (`ds1107-offpeak,
  ds-classic-offpeak, ds1107-peak, ds-classic-peak, ds-payg-offpeak, ds-payg-peak`) is D1/D4's to write; D3
  makes each name a `(door_sk, model_sk)` pair.
- **Luna 6 vs 6.1:** **fully separate families** (`gpt-6-luna` and `gpt-6.1-terra` are separate families in
  this ledger already; Luna 6 is not a variant of Luna 6.1). Connected only by a `lineage` row with
  `reason = replaced-by` if the successor is declared. Their AA rows never merge.

**`dim_model_match`** — the model-efforts a model aims to match (brief §6): `model_sk`, `effort_tier`,
`target_model_sk`, `target_effort_tier`, `axis` ∈ {price, ability, speed}, `basis` ∈ {measured, aa-extrapolated,
aliased, ruled} (AD3 provenance), `quality_band`, `valid_from_ms`, `valid_to_ms`.

**`dim_door`** (Type 2; a door = provider + credential/plan + pricing class): `door_sk`, `provider_norm`,
`credential_id`, `plan_kind` ∈ {plan, payg, free, promo, direct}, `pricing_class`, `quota_service`
(the S4 `serviceId` this door's windows come from, e.g. `opencode-go`), `quota_metric_map` (JSON
`{ "5h": "5h", "weekly": "weekly" }`), `peak_rule` (window name), `valid_from_ms`, `valid_to_ms`, `is_current`.

**`dim_provider`**: `provider_norm`, `provider_raw_list` (e.g. `_openai.main`, `openai.main`, `_openai` → `openai`),
`display_name`. Normalisation is a reviewed mapping, never a regex, so a new raw id is an error until mapped.

**`dim_pool`** (Type 2): `route_id`, `tier` ∈ {medium, low, high} (AD2 — the pool as written is `medium`),
`members_hash` (over the ordered member list), `overlay_id` (D1), `valid_from_ms`, `valid_to_ms`.

**`dim_effort`** — the rung table as data (AD1): `effort_raw` → `effort_tier`, `native_variant` per door,
`declared` (bool), `reason`. Observed `effort_raw` values: `xhigh`, `high`, `medium`, `low`, `default`,
`max`, `none`, `minimal`, NULL. Default rows (until D1 lands, mirroring the brief §4 alias rule):
`xhigh|max|ultra|fast → high (alias)`, `default → medium (alias, declared)`, `none|minimal → low (alias,
declared)`, NULL → `unmapped`. The DeepSeek rows (`medium → high`, Go and ClinePass; `low → high` on Pro)
are declared remap rows with `declared = true` and the PR number as `reason`.

**`dim_time`**: `day`, `hour_utc`, `hour_ct` (America/Chicago; DST-aware), `weekday`, `peak_flag` by door rule.

#### 4.2.5 Contract for the code#9373 and code#9374 ledgers (named now, built elsewhere)

- **fleet invocation** (code#9373): `invocation_id, subcommand, action, session_id, started_at_ms, ended_at_ms,
  exit_code, cwd, source`. Joins to `fact_tool_call` on `(session_id, time_ms ±1 s, subcommand)`.
- **mcpx call** (code#9374): `call_id, namespace, tool, ok, duration_ms, session_id, started_at_ms, error_tag`.
  Joins to `fact_tool_call` on `(session_id, tool name)` for `mcpx_exec` scripts.

Until these exist: `fact_tool_call.source ∈ {session-message, script-parse}` and every fleet/mcpx count is
labelled `inferred`.

### 4.3 Change markers and the three clocks

#### 4.3.1 `change_marker`

| Column | Type | Note |
| --- | --- | --- |
| `marker_id` | VARCHAR PK | `mk-<yyyymmdd>-<slug>` |
| `kind` | ENUM(config, model-add, model-retire, price, aa-version, remap, pool, effort, window, trial-start, trial-end, outage) | |
| `at_ms` | BIGINT | when the change took effect (from the backup file's mtime or the merge commit date — §4.3.2) |
| `scope_model_sk`, `scope_door_sk`, `scope_route_id` | INTEGER/VARCHAR NULL | what it touches |
| `description` | VARCHAR | one line |
| `source_ref` | VARCHAR | PR, commit, or backup path (e.g. `opencode.jsonc.bak-luna-*`) |
| `hypothesis` | VARCHAR | the pre-registered reason (AD9) |
| `expected_delta` | JSON | `{metric, value, unit, basis, recorded_by, recorded_at_ms}` — written **before** the measurement |
| `review_at_ms`, `revert_condition` | BIGINT, VARCHAR | AD9 |
| `pre_window_days`, `post_window_days` | SMALLINT | default 7 and 7 |

**`change_marker_measure`** (append-only): `marker_id, measured_at_ms, metric, pre_n, post_n, pre_value,
post_value, delta, delta_interval_lo, delta_interval_hi, method, n_min_met`. The expected value is never
overwritten; the measured rows accumulate. `n_min_met` is false when either arm is below the per-metric
minimum (default 100 attempts per arm, §4.4).

#### 4.3.2 Seed markers (V2 backfill slice)

Backfilled from the V1 change record: `S13` backups (`bak-effort-*`, `bak-luna-*`, `bak-20261102`) and the
merged PRs named in brief §9: #9381 (DeepSeek effort trial), #9382 (explore→scout), #9384 (`limited` pool),
#9385 (route classes), #9388 (Haiku #high last resort), #9018 (Luna effort round-robin pattern). Dates come
from the merge commit (`git log -1 --format=%cI <sha>`); backups from file mtime. Where neither exists,
`at_ms` is NULL and the marker is flagged `undated`.

#### 4.3.3 Expected vs actual

Expected is recorded at marker time from AA or from config (`basis`). Actual is a `change_marker_measure` row
computed by the same SQL in §4.9 (Q15 with a time filter pre/post). A marker without an expected delta is
allowed but flagged `no-expectation`, so the review can see the difference.

#### 4.3.4 Attempt → marker link

`fact_attempt.change_marker_ids` = markers with `scope` matching the attempt's model/door/pool and
`at_ms <= time_started < at_ms + post_window`. Recomputed on rebuild, never written by hand.

#### 4.3.5 The three clocks

Every fact carries three times, so a rollup "since X" is a query, not a rewrite:

- **event time** (`time_started_ms`, `time_ms`): when the thing happened.
- **capture time** (`ingested_at_ms`): when the store saw it. Its age is the "time since capture" clock.
- **AA time** (`aa_version_key`, `aa_test_date` from `dim_model` in force): the "time since AA version" clock.
- **inject time**: the `change_marker.at_ms` the row is linked to (§4.3.4). Its age is the "time since inject" clock.

### 4.4 The statistics set

Every statistic is computed per **group** (any tag set, §4.2.3) and **window** (event-time interval, or
relative to a clock in §4.3.5). Every group row carries `n` for each quantile.

| Statistic | Merge class | State stored | Rule / note |
| --- | --- | --- | --- |
| `n` (count) | **exact** | integer | |
| `sum` | **exact** | double (and int when integer-valued) | |
| `min`, `max` | **exact** | double | |
| `mean` | **exact** | `sum / n` | |
| `stddev` (sample, n−1) | **exact** up to float rounding | Welford `mean`, `M2`; merged with Chan's cross term (R3 §2.1; the oracle's mutation check fails without it) | |
| `mode` | **exact** for discrete values | value→count map (`vals`) | integer-valued fields exact (tokens, counts, whole ms); continuous fields use 1 ms bins and are labelled `mode_binned` |
| `mode±x` | **exact** for discrete | count and share of samples with `|v − mode| ≤ x` from `vals` | `x` is a parameter; default `x = 10 %` of the mode for continuous fields |
| `median` (P50), `Q1` (P25), `Q3` (P75), `IQR = Q3 − Q1` | **sketched** | DDSketch histogram, α = 1 % | IQR error can exceed α when the IQR is narrow relative to Q3 (both quantiles carry α·value error); report with the bound |
| `p90`, `p99`, `p99.9` | **sketched** | same | p99.9 needs n ≥ 1000 to be resolvable (oracle: all 1,440 day-cells at 20 seeds are below 1,000, so the report must flag them) |
| exact quantile on demand | recomputed from raw | none stored | `ROW_NUMBER` nearest-rank SQL (§4.9 Q15); used for review and as the oracle |

Reporting rules (binding):

1. **Every quantile is printed with `n`.** A quantile with `n < 100` for p90, `n < 1000` for p99.9 (defaults,
   §7 Q-D3-4) is shown as `unresolved` next to its value.
2. Always print `quantile_method = nearest_rank_v1` and `alpha` on the same row.
3. Mergeable statistics (rows 1–8 of the table) are exact across any re-grouping; sketched statistics are exact
   only within the α bound and are never averaged across groups (average of medians is not a median).
4. `mode` is never reported without its `n`.

**Oracle check.** `scripts/stats/stats_rollup_oracle.py` (slice D3-S5) is the prototype copied into the repo.
Its invariants are the acceptance test for the production rollup: conservation (day counts sum to raw), exact
merge for count/sum/min/max/mode, float merge within 1e-9 for mean/stddev, sketch quantiles within α. Re-run on
2026-11-13 with 20 seeds: PASS, max relative quantile error 0.009997.

### 4.5 Aggregates as events

Each generated rollup is itself a row in an append-only **`stat_rollup_event`** stream. This is the brief §6
"constantly review stored aggregates … as events" requirement.

| Column | Type | Note |
| --- | --- | --- |
| `rollup_id` | VARCHAR PK | hash of `(group, window, generator_version, input_digest)` |
| `generated_at_ms` | BIGINT | generation time |
| `generator` | VARCHAR | repo, script path, git sha of the generator |
| `generator_version` | VARCHAR | semver of the rollup code |
| `aa_version_key` | VARCHAR | AA version in force for the group (or `mixed` with the list) |
| `aa_test_date_min`, `aa_test_date_max` | DATE | |
| `time_frame_basis` | ENUM(event, capture, aa-version, inject) | which clock the window uses |
| `window_start_ms`, `window_end_ms` | BIGINT | |
| `group_tags` | JSON | the tag set of the group |
| `n_sessions`, `n_turns`, `n_attempts`, `tokens_total` | BIGINT | the counts the brief asks for |
| `models`, `efforts`, `providers`, `pools`, `doors` | LIST(VARCHAR) | coverage |
| `quantile_method`, `alpha` | VARCHAR, DOUBLE | |
| `state_uri`, `state_digest` | VARCHAR | Parquet path and SHA-256 of the state file |
| `input_event_id_min`, `input_event_id_max`, `input_count` | BIGINT | membership check: a re-run over the same predicate must reproduce `input_count` |
| `supersedes_rollup_id` | VARCHAR NULL | a re-run is a new event that names the one it replaces |
| `reviewed_by`, `review_note` | VARCHAR NULL | D6 review surface writes here, append-only |

Sample review: the predicate plus `[input_event_id_min, input_event_id_max]` recreates the sample set exactly;
the review UI reads `fact_attempt` rows by the predicate, so "review per sample" needs no per-sample membership table.

### 4.6 Storage growth and rollup policy

#### 4.6.1 Measured growth (2026-11-13, live, read-only)

| Table | Rows since 2026-11-06 | Bytes per row (measured or baseline) | Growth per day |
| --- | ---: | ---: | --- |
| `route_attempt` | 330,344 (since 2026-11-06); 45–67 k/day | ≈230 B (baseline §4) | ≈10–15 MB/day |
| `route_decision` | 1,053,744 | ≈1,350 B (measured on a 27.8 h window, 88,471 rows) | **≈120–300 MB/day** (131 k–224 k rows/day, 2026-11-08 to 11-12) |
| `session_message` | 619,611 total since 2026-01-18 | ≈12 KB per turn (baseline; mostly `content` text) | not sized per day here (largest table, 7.7 GB) |
| `session_v2` | 19,629 | ≈460 B | negligible |
| S4 snapshots | 15,977 readings (Oct+Nov) | ≈200 B/reading | ≈0.1 MB/day |

Route decisions dominate the new-event growth in bytes; attempts dominate in row count for the attempt facts.

#### 4.6.2 The `storage_growth` slice (the measurement)

`storage_growth(measured_at_ms, day, table_name, rows, sqlite_dbstat_bytes, parquet_bytes, bytes_per_row_raw,
bytes_per_row_parquet, export_version)` — one row per table per day, written by the nightly export (§4.6.4).
The first 30 days of rows decide the retention policy (below). The slice D3-S2 creates the table and the
backfill for the days already in the DB.

#### 4.6.3 Rollup and retention policy (V2 defaults)

| Data | Keep | Rollup | Trigger |
| --- | --- | --- | --- |
| Raw Parquet (all facts) | full for 30 days of measurement, then decide (default: keep) | none | — |
| Mergeable states, hour | 13 months | by event hour | nightly |
| Mergeable states, day | forever (small) | by event day | nightly |
| Rollups by **inject** (change marker) | forever | windows `[at, at+1d]`, `+7d`, `+14d`, `+30d` relative to each marker | when a window closes |
| Rollups by **capture** | forever | window by `ingested_at_ms` (lag measure) | when a window closes |
| Rollups by **AA version** | forever | one state per `aa_version_key` | when a new AA batch loads |
| Fact rows with an open outcome window | never deleted | — | until `window_days` closes |
| `fact_outcome`, `fact_link`, `change_marker*`, `stat_rollup_event` | forever (append-only) | — | — |

Rule: a rollup state is only ever **recomputed** from raw, never patched; raw is the source of truth.

#### 4.6.4 The export pipeline

Location: **`pm-stack/route-ledger-export/`** (new; mirrors `pm-stack/cloudflare-export/`):

- `route-ledger-export.mjs` — the exporter (Bun, as `analytics-store.ts` already uses).
- `route-ledger-export.test.mjs` — oracle tests (§4.10 slice D3-S6).
- `route-ledger-export.sh` — wrapper for the timer; `route-ledger-export.service` / `.timer` (nightly, 03:20 CT);
  `install-laptop.sh`; `views.sql` (DuckDB views); `README.md`.

Steps (nightly; each step is idempotent):

```text
1. Open ~/.local/share/opencode/opencode.db with bun:sqlite { readonly: true }.
   Never write to it. Never copy the whole file.
2. Read high-water marks from ~/.local/state/route-ledger/export-state.json
   (per table: last id / last time). First run: from 2026-01-18 for session tables,
   from 2026-10-24 15:25 for route_* (the baseline's first rows).
3. Stream new rows per table in id order, in batches of 50,000:
   route_attempt (id), route_decision (id), route_health (id), session_v2 (time_updated),
   session_message (time_created; assistant + user + idle + compaction types only; tool
   parts are exploded into fact_tool_call inside the same pass).
4. Parse, do not guess:
   - quota JSON → typed columns (4 shapes, §2 fact 1); unknown shape → quota_shape='none'
     and a row in export_errors, never a dropped row.
   - provider_id → provider_norm via the reviewed mapping file (unmapped → 'UNMAPPED', counted).
   - variant → effort_tier via dim_effort (unmapped → 'unmapped', counted).
   - mcpx_exec scripts → fact_tool_call rows with regex
       tools\.([A-Za-z0-9_-]+)\.([A-Za-z0-9_]+)\s*\(
       and tools\[\s*"([^"]+)"\s*\]\[\s*"([^"]+)"\s*\]\(
   - shell commands → fleet_sub/fleet_action with
       \bfleet\s+([a-z][a-z-]*)(?:\s+([a-z][a-z-]*))?
5. Write NDJSON staging under ~/.local/state/route-ledger/staging/<table>/day=YYYY-MM-DD/.
6. Convert with the DuckDB CLI (COPY … TO … (FORMAT PARQUET)) into
   ~/.local/state/route-ledger/parquet/<table>/day=YYYY-MM-DD/part-<run-id>.parquet.
   Partition: by UTC day (event time), then nothing else (provider is a column, not a folder).
   Schema version in the file metadata.
7. Dimension tables (dim_model, dim_door, dim_pool, dim_effort, change_marker, …) come from
   reviewed files in pm-stack/route-ledger-export/dims/*.json (git-tracked; changes are PRs).
8. Write storage_growth rows for the run (§4.6.2).
9. Advance the high-water marks only after step 6 succeeds (atomic rename of export-state.json).
```

Parquet partitioning: `table/day=YYYY-MM-DD/part-*.parquet`. Compaction: one file per table per day
(re-running a day replaces its file atomically). Snapshots are not needed for V2; DuckLake (option B) is the
upgrade if time travel becomes necessary.

**DuckDB views** (`views.sql`, read with `read_parquet('…/*/*.parquet', hive_partitioning = true)`):
`v_attempt`, `v_decision`, `v_session`, `v_turn`, `v_tool_call`, `v_decision_record`, `v_outcome_current`
(latest `outcome_code` per subject), `v_dim_model_current` (`is_current`), `v_rollup_latest`.

**Where it feeds:**

- **Quota analytics page** (`/quota#analytics`, `/api/quota-analytics.json`): today it reads live
  `route_attempt` through `analytics-store.ts`. V2 keeps the live reader for the last 24 h and adds a
  historical reader over `v_attempt` for windows over 24 h (pm-stack slice in D3-S8). The page's per-pool
  burn, ratio and projection math (`quota-analytics.ts`) does not change.
- **Roadmap & Stats Space** (brief §9): consumes `stat_rollup_event` and `v_rollup_latest`. The pipeline
  it names is **absent** (baseline G13), so V2 writes the contract (the view and the event schema) and the
  consumer is wired when the owner's path is known (§7 Q-D3-6).
- **D1 / D4 / D5 / D6**: the §4.9 query set (run against the views).
- **HF Spaces / PM review surface** (R3 §7.1 step 3): read the same Parquet with DuckDB; no second store.

Runner: laptop systemd user timer, nightly. DuckDB CLI must be on the laptop profile (open question §7
Q-D3-8: default is to add it to the Nix profile in the same slice as the exporter; if it is not available,
the exporter keeps NDJSON staging and the views wait).

### 4.7 Dependence on code#9373 and code#9374 (summary)

| Need | Without the ledger (V2 now) | With it (V2.1 / V3) |
| --- | --- | --- |
| fleet per subcommand / action | `fact_tool_call` from `shell` text regex; labelled `inferred`; counts shown with the regex version | `fleet-ledger` rows; the inference is kept only for cross-checking the ledger |
| mcpx per namespace / tool / action | `mcpx_exec` script-parse (`source = script-parse`); `mcpx_discover` names only | `mcpx-ledger` rows with `ok`, `duration_ms` |
| task-level attribution | by session only | by call, once the ledger carries `session_id` |

D3 does not block on either ledger. The slice that needs them (D3-S12) is blocked on the ledger issue, not
on D3 slices.

### 4.8 Cost and quota attribution

#### 4.8.1 Cost basis (per attempt)

Let `p = price row` for the attempt's `dim_model` (AA `prices`, via the crosswalk in `dim_model_alias`).

```text
cost_list_usd =
    (tokens_input − tokens_cache_read − tokens_cache_write) × in_per_1M / 1e6
  + tokens_cache_read  × cache_read_per_1M  / 1e6
  + tokens_cache_write × cache_write_per_1M / 1e6
  + tokens_output × out_per_1M / 1e6            -- reasoning assumed INCLUDED in output (§7 Q-D3-3)

cost_paid_usd =
    cost_usd (recorded)      if cost_usd IS NOT NULL
    0                        if door.plan_kind = 'plan'   (subscription: no marginal cost)
    cost_list_usd            if door.plan_kind = 'payg'
    NULL                     otherwise

cost_basis =
    'recorded'         if cost_usd IS NOT NULL
    'subscription-zero' if plan_kind = 'plan' and cost_usd IS NULL
    'derived-aa'       if p exists and plan_kind = 'payg'
    'unknown'          otherwise
```

Calibration (oracle): on the 50,367 rows with `cost_usd > 0`, `cost_list_usd` must match `cost_usd` within
±1 % for the rows whose door is payg. A calibration slice (D3-S7) reports the match rate and the median
absolute error per door. This is how the reasoning-inclusion question (§7 Q-D3-3) is answered.

`price_version` is the AA batch id that supplied `p`; a price change is a `change_marker` (`kind = price`).

#### 4.8.2 Quota percent per attempt (inferred, V2)

For each attempt `a` on door `d` with `quota_service = s` and window metric `m` (from `dim_door.quota_metric_map`):

```text
before = latest S4 reading r with service=s, metric=m, ts <= a.time_started
         and ts >= a.time_started - 30 min                      (else NULL)
after  = earliest S4 reading r with service=s, metric=m, ts >= a.time_ended
         and ts <= a.time_ended + 30 min                        (else NULL)
quota_pct_before = before.value ; quota_pct_after = after.value
quota_pct_delta  = after − before  (NULL if either is NULL; can be negative after a window reset)
quota_pct_source = 'snapshot-window-join' ; inferred = true
```

Reading cadence is not regular (`usage-2026-11.jsonl`: 10,736 rows over 13 days; `opencode-go` 5h rows
520, weekly 520), so the 30 min window is a parameter, and each attempt records the lag to the reading it used
(`quota_pct_before_lag_s`). A reset between the readings is detected when `after < before` and reported as
`reset-suspected`, not as negative use.

#### 4.8.3 Time per session, task, PR, attempt

- **Per attempt**: `response_ms`, `first_token_ms` (recorded).
- **Per session**: `active_minutes_5m` (§4.9 Q12): sum of gaps ≤ 5 min between assistant turns.
  The 5-minute threshold is an assumption (§7 Q-D3-5); a session row carries the threshold it used.
- **Per task / PR**: from `fact_link` (`task-session`, `attempt-task`) and `fact_session` — once the queue
  task links exist (code#3424 piece 5, baseline G14). Until then, "days to complete" is session created → last
  update, labelled `session-proxy`.

### 4.9 The data-basis query set (for D1 / D4)

One query file per brief §5 dimension, under `scripts/stats/queries/`. SQLite dialect against the live DB
(read-only) for now, and the same logic against the `v_*` views in V2. Parameters: `:since` (epoch ms).
Each file starts with a comment naming the dimension, the source table, and the oracle that tests it.

Status column: **T** = run and checked against the live DB on 2026-11-13 (read-only); **D** = designed,
needs the V2 view or the exporter to run.

| # | Brief §5 dimension | Query | Status |
| --- | --- | --- | --- |
| Q01 | provider / model / effort: attempts, success rate | `SELECT route_id, provider_id, model_id, COALESCE(variant,'-'), COUNT(*), SUM(outcome='success') FROM route_attempt WHERE time_started >= :since GROUP BY 1,2,3,4 ORDER BY 5 DESC` | T |
| Q02 | pool: offered vs chosen | `route_decision` with `json_each(candidates)` for offered, `chosen` for chosen; counts by `(route_id, candidate)` | T (see below) |
| Q03 | ratio per session | `SELECT id, tokens_output*1.0/NULLIF(tokens_input,0) FROM session_v2 WHERE time_created >= :since` | T |
| Q04 | sub-sessions: count and depth | recursive CTE over `parent_id` (below) | T |
| Q05 | models / pools / sub-sessions used per session | `route_attempt` grouped by `session_id`: `COUNT(DISTINCT route_id)`, `COUNT(DISTINCT model_id)` | D (trivial; V2 view) |
| Q06 | tool usage by name | `json_each(sm.data,'$.content')` on assistant rows with `type='tool'` | T |
| Q07 | mcpx per namespace / tool / action | `fact_tool_call` where `tool_family='mcpx'` (script-parse) | D |
| Q08 | fleet per subcommand / action | `fact_tool_call` where `tool_family='fleet'` | D |
| Q09 | real diff +/− | `session_v2.summary_additions/deletions/files` (sparse) | T |
| Q10 | attempts per session / task | `COUNT(*)` of `route_attempt` per `session_id`; per task later | T (per session) |
| Q11 | days to complete (created → updated) per root session | `session_v2`: `(time_updated − time_created)/86400000` for roots with children | T |
| Q12 | hours of actual work | `active_minutes_5m` over `session_message` assistant turns (below) | T |
| Q13 | success rate per target and effort | Q01 with `outcome` | T |
| Q14 | quota % per attempt | S4 window join (§4.8.2) | D |
| Q15 | the statistics block (min, max, mean, stddev, n, median, p90, p99, p99.9, Q1, Q3, IQR) per group | nearest-rank with `ROW_NUMBER` (below) | T |
| Q16 | TTFT and step time | `session_message.data.time`: `streamed − created`, `completed − created` | T |

**Q04 — sub-session count and depth (T).**

```sql
WITH RECURSIVE t(root, id, depth) AS (
  SELECT id, id, 0 FROM session_v2 WHERE parent_id IS NULL
  UNION ALL
  SELECT t.root, s.id, t.depth + 1 FROM session_v2 s JOIN t ON s.parent_id = t.id
)
SELECT root, MAX(depth) AS max_depth, COUNT(*) - 1 AS descendants
FROM t GROUP BY root
HAVING MAX(depth) > 0
ORDER BY descendants DESC;
```

Live result, 2026-11-13: 19,629 sessions, max depth 5, 16,206 roots.

**Q12 — active minutes, 5-minute gap clip (T).**

```sql
WITH a AS (
  SELECT session_id, time_created AS t,
         LAG(time_created) OVER (PARTITION BY session_id ORDER BY time_created) AS prev
  FROM session_message
  WHERE type = 'assistant' AND time_created >= :since
),
g AS (SELECT session_id, MAX(0, t - prev) AS gap FROM a WHERE prev IS NOT NULL)
SELECT session_id,
       ROUND(SUM(CASE WHEN gap <= 300000 THEN gap ELSE 0 END) / 60000.0, 2) AS active_minutes_5m
FROM g GROUP BY session_id;
```

Live result (since 2026-11-06): 7,441 sessions, 50,082 active minutes clipped at 5 min. Each session's turn
gaps are measured from its own assistant rows only.

**Q15 — statistics block with nearest rank (T).** Integer nearest rank: `rank(q) = max(1, ceil(q·n))`, written
as integer division so it is exact. The file is `scripts/stats/queries/q15_stats_nearest_rank.sql`.

```sql
WITH s AS (
  SELECT route_id, provider_id, model_id, COALESCE(variant,'-') AS variant, response_ms AS v
  FROM route_attempt
  WHERE outcome = 'success' AND response_ms IS NOT NULL AND time_started >= :since
),
r AS (
  SELECT *,
         ROW_NUMBER() OVER (PARTITION BY route_id, provider_id, model_id, variant ORDER BY v) AS rn,
         COUNT(*)     OVER (PARTITION BY route_id, provider_id, model_id, variant)            AS n
  FROM s
)
SELECT route_id, provider_id, model_id, variant, n,
       MIN(v) AS min_v, MAX(v) AS max_v, ROUND(AVG(v), 1) AS mean_v,
       MIN(CASE WHEN rn = MAX(1, (  n + 1) / 2) THEN v END) AS p50,
       MIN(CASE WHEN rn = MAX(1, (3*n + 3) / 4) THEN v END) AS q3,
       MIN(CASE WHEN rn = MAX(1, (9*n + 9) / 10) THEN v END) AS p90,
       MIN(CASE WHEN rn = MAX(1, (99*n+99)/100) THEN v END) AS p99
FROM r
GROUP BY route_id, provider_id, model_id, variant, n
HAVING n >= 30
ORDER BY n DESC;
```

Live result (since 2026-11-06, top rows): `workhorse · _openai.main · gpt-6-luna · high` n = 71,371,
median 9,997 ms (exact nearest-rank), p90 19,508 ms, p99 60,249 ms; `workhorse · _openai · gpt-6.1-terra · low`
n = 17,221, median 9,092 ms. The exact sqlite value at p99 is shown as an oracle; the sketch in V2 must agree
within α. **Note:** this is `response_ms` only (the call's own time, not queue time); it is a success-only
distribution, and the failure rate is in Q01.

**Q02 — pool offered vs chosen (T).**

```sql
SELECT route_id, j.value AS candidate, COUNT(*) AS offered,
       SUM(chosen = j.value) AS chosen
FROM route_decision, json_each(candidates) j
WHERE time >= :since
GROUP BY route_id, candidate
ORDER BY offered DESC;
```

**Q16 — TTFT and step time (T).**

```sql
SELECT COUNT(*) AS turns,
       AVG(json_extract(data,'$.time.streamed') - json_extract(data,'$.time.created'))  AS ttft_proxy_ms_mean,
       AVG(json_extract(data,'$.time.completed') - json_extract(data,'$.time.created')) AS step_ms_mean
FROM session_message
WHERE type = 'assistant' AND time_created >= :since
  AND json_extract(data,'$.time.streamed') IS NOT NULL;
```

Live result since 2026-11-06: 190,353 turns, TTFT proxy mean 10,860 ms, step mean 15,991 ms.

**Q06 — tool usage by name (T).**

```sql
SELECT json_extract(j.value,'$.name') AS tool, COUNT(*) AS calls
FROM session_message sm, json_each(sm.data,'$.content') j
WHERE sm.type = 'assistant' AND sm.time_created >= :since
  AND json_extract(j.value,'$.type') = 'tool'
GROUP BY tool ORDER BY calls DESC;
```

Live result since 2026-11-06 (top): `shell` 146,683 · `execute` 34,143 · `read` 19,911 · `grep` 9,135 ·
`patch` 4,239 · `glob` 2,875 · `subagent` 1,866 · `edit` 1,459.

**Q01 — attempts and success by target and effort (T).**

```sql
SELECT route_id, provider_id, model_id, COALESCE(variant,'-') AS variant,
       COUNT(*) AS attempts, SUM(outcome = 'success') AS ok,
       ROUND(1.0 * SUM(outcome = 'success') / COUNT(*), 4) AS success_rate
FROM route_attempt
WHERE time_started >= :since
GROUP BY route_id, provider_id, model_id, variant
ORDER BY attempts DESC;
```

**Q07 / Q08 (D)** — DuckDB over `v_tool_call`:

```sql
-- Q07 mcpx per namespace, tool (script-parse; inferred until code#9374)
SELECT mcpx_ns, mcpx_tool, COUNT(*) AS calls, SUM(status = 'error') AS errors
FROM v_tool_call WHERE tool_family = 'mcpx' AND mcpx_ns IS NOT NULL AND time_ms >= :since
GROUP BY 1, 2 ORDER BY calls DESC;
-- Q08 fleet per subcommand and action (inferred until code#9373)
SELECT fleet_sub, fleet_action, COUNT(*) AS calls, SUM(status = 'error') AS errors
FROM v_tool_call WHERE tool_family = 'fleet' AND time_ms >= :since
GROUP BY 1, 2 ORDER BY calls DESC;
```

**Q14 (D)** — the quota join, in DuckDB over `v_attempt` and the S4 snapshot view `v_quota_reading`
(`service_id, metric_id, ts_ms, value`); the window is the parameter in §4.8.2.

### 4.10 Prototype: the rollup oracle as the test oracle

The prototype `research/prototypes/stats_rollup_oracle.py` is the oracle for the production rollup. The
slice D3-S5 moves it to `scripts/stats/stats_rollup_oracle.py` (stdlib only; run as `python3 -I`), adds a
pytest-free runner `scripts/stats/test_stats_rollup_oracle.py` that calls it over ≥ 200 seeds, and makes the
production rollup (`pm-stack/route-ledger-export/rollup.mjs`, slice D3-S9) pass the same invariants on the same
generated events. The oracle is the **decision procedure** for "does the production rollup equal the exact
answer": every invariant above is a decidable check. Recorded result 2026-11-13: 20 seeds PASS, max relative
quantile error 0.009997 (bound 0.01); 1,440 of 1,440 day-cells below n = 1,000 (p99.9 unresolved in this
synthetic volume, as the oracle says).

Testing doctrine (AGENTS.md): PBT is the right tool here (the invariants hold for any event set); MBT is the
right tool for the rollup-by-clock policy (a model of the three windows); the oracle is the reference
implementation above, plus the exact nearest-rank SQL (Q15) on live data.

---

## 5. V3 and V4 layers

**V3 (measured and automatic; gate: ≥ 14 days of V2 facts per tuned decision, `main.md` §0):**

- In-place quota percent at call start and end, recorded by the proxy (replaces the S4 inference where it
  exists; keeps the inferred column for the old rows). Gate: the S4 join's agreement with the recorded
  readings measured for 14 days.
- Stats-driven values for D1/D4: the per-door per-effort medians and tails in Q15, and the change-marker
  measurements of §4.3.3 feed D1's overlay values with provenance `measured`.
- Guards: a CI guard that fails when a route's measured median or p90 drifts beyond its recorded band
  (named drift check; data from `v_attempt`).
- Roadmap & Stats Space consumes `stat_rollup_event` as its daily input.
- Upgrade path to ClickHouse if interactive slicing across many users is required (R3 §7 option 3).

**V4 (learned and adaptive; gate: V3 shows the hand-tuned baseline is the limit):** a bandit's reward is an
`fact_outcome`-weighted metric (merged, reverted, rework); the store must serve per-arm `n`, sketched tails and
the change-marker expectations with no extra joins. D3 provides those, V4 owns the policy.

---

## 6. Worked examples

**Example 1 — "What did DeepSeek Flash cost per correct outcome on the deepseek plan door, peak vs off-peak?"**

1. Dimension: `dim_model` rows for `deepseek-v4.1-flash` (variant `classic`) and `deepseek-v4-1-flash-1017`
   (variant `snapshot-1017`, provisional name), each a separate `model_sk` (§7 Q-D3-1).
2. Door: `dim_door` for `opencode-go.developing-today-0` (plan) and `_vibe` (plan/free, to be confirmed by D1).
   `peak` tag from the door's `peak_rule`.
3. `fact_attempt` gives tokens and `cost_list_usd` (derived, `cost_basis = derived-aa`, price version pinned),
   `cost_paid_usd = 0` for plan doors.
4. Outcomes: `fact_outcome` with `subject_kind = attempt`→ via `fact_link` (`attempt-task`) → task → PR
   `merged` / `reverted` (mechanical).
5. Output per (variant, door, peak): `n`, `cost_list_usd` sum, `merged` count, `merged/cost` ratio, and the
   p90 of `response_ms` with `n` shown. Reporting rule 1 applies.
6. The answer is a **data basis**, not a decision: D4 decides whether the difference is a trial to run.

**Example 2 — "Luna 6 vs Luna 6.1 (terra) at 388k context, effort low vs high, last 14 days, around the Luna round-robin change."**

1. `gpt-6-luna` and `gpt-6.1-terra` are separate families (brief §6). No merge, no shared `variant_key`.
2. The change marker `mk-…-luna-rr` (PR #9018 pattern; date from its merge commit) has an expected delta
   recorded before the window (`expected_delta`). `change_marker_measure` gives pre/post `n`, medians and
   p90 with intervals (Q15 with the two windows).
3. `fact_attempt.change_marker_ids` shows which attempts sit in the post window; `n_min_met = false` is shown
   if either arm is under 100.
4. The result is recorded as a `change_marker_measure` row; the expected delta is not rewritten.

**Example 3 — "Which pools were offered but never chosen for workhorse last week?"** Q02 over `route_decision`
with `candidates` and `chosen`; the `excluded` reasons (`pool-quota-exhausted:<pool>`, `cooldown-until:<ms>`)
from `fact_route_decision.excluded` separate "walled" from "not preferred", the distinction the brief's §10
cycle safety needs.

---

## 7. Risks and open questions (with the default chosen)

Detached: no question was asked. Each item below records the default applied. Items marked **RULING** are
Drew's to settle; they are batched with the mill's questions log.

| ID | Question | Default chosen | Why / where applied |
| --- | --- | --- | --- |
| Q-D3-1 | **RULING.** The brief's "DeepSeek 1107" does not appear in the ledger (0 rows). Ledger ids are `deepseek-v4-1-flash-1017` and `deepseek-v4.1-flash`. Is `1107` a later snapshot not yet served, or the same as one of these? | Name variants from the ledger (`snapshot-1017`, `classic`). Add `snapshot-1107` as a row when first observed; never rename | §4.2.4, §6 Ex. 1 |
| Q-D3-2 | Is `route_attempt` the full call ledger, or are direct calls missing? (baseline Q-src-3) | The proxy is the single path (route-proxy comments); unverified | §2 |
| Q-D3-3 | Is `tokens_output` inclusive of reasoning tokens? | Yes (OTel convention, R3 §4.1); the calibration slice D3-S7 against `cost_usd > 0` rows decides it | §4.8.1 |
| Q-D3-4 | Quantile n thresholds (p90 ≥ 100, p99.9 ≥ 1000) | These are heuristics (R3 §2.3, unverified); keep and revisit after measurement | §4.4 |
| Q-D3-5 | Active-time gap threshold | 5 minutes | §4.8.3; the value is stored on every row that uses it |
| Q-D3-6 | Roadmap & Stats pipeline path (G13) | Consumer contract only until the owner's path is known; ask Drew before depending on it | §4.6.4 |
| Q-D3-7 | Session `title` and `directory` contain free text | Keep them for review; do not export message content, ever (only lengths) | §4.2.1 |
| Q-D3-8 | Is the DuckDB CLI on the laptop profile? | Add to the Nix profile in slice D3-S6; NDJSON staging until then | §4.6.4 |
| Q-D3-9 | `route_note` (fork schema) is not in the baseline source list | Include as a source for notes in V2 review; no fact table until D6 needs it | §2 |
| Q-D3-10 | Route-decision bytes per day (≈100–300 MB) — is the route-decision log retained by a separate rule? | Keep full; measure 30 days (§4.6.2) before any retention decision | §4.6 |
| Q-D3-11 | Attempts with `session_id` NULL (14,646) | `unattributed`; excluded from per-session stats, included in per-attempt | §2 |
| Q-D3-12 | `provider_id` spellings (`_openai.main` vs `openai.main`) — same account or two? | Treated as one `provider_norm` (`openai`) with two `door_sk` rows keyed by `credential_id`; if they are two accounts, the door split stands | §2 fact 3 |
| Q-D3-13 | The AA pull is `aa-last-good.json` with OpenRouter-style names; route ids differ | Crosswalk table `dim_model_alias` (reviewed mapping); unmapped models get `cost_basis = unknown` | §2 fact 11, §4.8.1 |
| Q-D3-14 | Quota % reading cadence is irregular | 30 min window, lag recorded per attempt; `reset-suspected` when after < before | §4.8.2 |
| Q-D3-15 | Chosen store (Parquet + DuckDB) cannot be tested here (no duckdb/pyarrow on this machine) | The SQLite forms (Q01–Q16 "T") are tested now; the Parquet forms are tested in D3-S6 | §4.9 |

**Risks:**

- **Growth of route_decision text** (≈100–300 MB/day). Mitigation: measure first (D3-S2), decide retention
  after 30 days.
- **Silent drift in the live DB** (counts moved between two queries in this session). Mitigation: every
  published number carries its as-of time and the exporter's high-water mark.
- **Inferred fleet/mcpx counts** may be read as measurements. Mitigation: `source` on every row and the word
  `inferred` in every report line.
- **Remaps corrupt measurement** (AD1, baseline Fact 2). Mitigation: `dim_effort` rows with `declared`, and
  `remap_declared` on every attempt.
- **Stale AA data** (the source of record moved on 2026-11-12). Mitigation: `aa_version_key` and
  `price_version` on every derived row.

---

## 8. Statement coverage table

Statements: `inventory/` is empty; the table uses brief/supplement sections. Slice ids are in
`slices/D3-slices.json`.

| Statement (brief / supplement §) | Section of this design | Slice id(s) |
| --- | --- | --- |
| brief §5 provider / model / effort | §4.2.1 `fact_attempt`; §4.9 Q01 | D3-S1, D3-S4 |
| brief §5 pool | §4.2.1 `fact_route_decision`; §4.9 Q02 | D3-S1, D3-S10 |
| brief §5 AA data | §4.2.4 `dim_model`; §4.8.1 | D3-S3 |
| brief §5 real fleet data: ratio per session | §4.2.1 `fact_session`; §4.9 Q03 | D3-S1, D3-S10 |
| brief §5 sub-sessions: number and depth | §4.2.1 `fact_session`; §4.9 Q04 | D3-S1, D3-S10 |
| brief §5 models / pools / sub-sessions used | §4.9 Q05 | D3-S10 |
| brief §5 tool usage | §4.2.1 `fact_tool_call`; §4.9 Q06 | D3-S4 |
| brief §5 mcpx per subcommand / action | §4.2.1 `fact_tool_call`; §4.7; §4.9 Q07 | D3-S4, D3-S12 |
| brief §5 fleet per subcommand / action | §4.2.1; §4.7; §4.9 Q08 | D3-S4, D3-S12 |
| brief §5 real diff +/− | §4.2.1 `fact_session`; §4.9 Q09 | D3-S10 |
| brief §5 attempts; triages | §4.2.1 `fact_attempt`; triage: §7 risk (no ledger) | D3-S1, D3-S12 |
| brief §5 days to complete; hours of work | §4.8.3; §4.9 Q11, Q12 | D3-S10 |
| brief §5 success rate | §4.9 Q01, Q13 | D3-S10 |
| brief §5 time and quota % per session / task / PR / attempt | §4.8.2, §4.8.3 | D3-S8, D3-S11 |
| brief §5 statistics (min, max, stddev, count, median, mean, sum, p90, p99, p99.9, Q1–Q3, IQR) | §4.4; §4.9 Q15 | D3-S5, D3-S9 |
| brief §6 table of all report details, review per sample, rollups, group-by any tag | §4.2, §4.3.5, §4.5, §3.5 | D3-S1, D3-S2, D3-S9 |
| brief §6 aggregates as events (generation time, AA version, time frame, counts, coverage, AA test version and date) | §4.5 | D3-S9 |
| brief §6 successors / predecessors in order | §4.2.4 `dim_model_lineage` | D3-S3 |
| brief §6 model-efforts it aims to match (price / ability / speed) | §4.2.4 `dim_model_match` | D3-S3 |
| brief §6 capabilities and unique details as tags | §4.2.3 | D3-S3 |
| brief §6 keep full granularity; measure storage growth | §4.6.1, §4.6.2 | D3-S2 |
| brief §6 rollup by time since inject / capture / AA version | §4.3.5, §4.6.3 | D3-S9, D3-S13 |
| brief §6 variants split (DeepSeek 1107; Luna 6 vs 6.1 fully separate) | §4.2.4 variant rules; §7 Q-D3-1 | D3-S3 |
| brief §6 known dates with expected and actual difference | §4.3 `change_marker`; §4.3.2 | D3-S3, D3-S13 |
| brief §7 cost efficiency; extrapolation where we differ from AA | §4.8.1; §3.6 | D3-S7 |
| brief §8 trials, testing plan (data) | §4.3.3; §6 Ex. 2 | D3-S13 |
| brief §9 Roadmap & Stats consumer | §4.6.4 | D3-S14 |
| brief §9 quota analytics page | §4.6.4; §2 consumers | D3-S8 |
| brief §9 code#9373 / code#9374 | §4.7 | D3-S12 |
| supplement §5 outcome capture (outcomes, linkage, TTFT, tokens, question prompts) | §4.2.1 `fact_outcome`, `fact_link`; §4.2.2 | D3-S11 |
| supplement §5 "every session can report" (shape only) | §4.2.1 `fact_outcome` `observed_by` | D3-S11 |
| supplement §9 decision record (storage shape) | §4.2.1 `fact_decision` | D3-S11 |
| AD7 every decision and attempt is an addressable event; append-only | §4.2.1, §4.2.1 `fact_outcome` | D3-S1, D3-S11 |
| AD8 export, Parquet by day, DuckDB, Type-2 `dim_model`, `change_marker` | §4.6.4, §4.2.4, §4.3 | D3-S1, D3-S3, D3-S6 |
| AD8 mergeable states; sketches α = 1 %; every quantile with n | §3.3, §4.4 | D3-S5, D3-S9 |
| AD1 remaps as explicit rung rows (store side) | §4.2.4 `dim_effort` | D3-S3 |
| AD9 pre-registered trials (store side: expected/review/revert) | §4.3.1 | D3-S13 |
