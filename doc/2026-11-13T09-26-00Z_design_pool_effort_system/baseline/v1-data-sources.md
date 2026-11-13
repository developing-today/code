# V1 data sources for the effort mapping and the granular stats store

- Program: design mill for code#9399 (pool & effort system). Queue item 15864.
- Branch: `design/pool-effort-system` (worktree `/home/user/git/wt-mill-pools`).
- Measured: 2026-11-13, about 06:05–06:30 CST, by read-only inspection. No
  writes to any store. The OpenCode DB was opened `mode=ro`, and all counts
  and time ranges come from that read-only access.
- Scope: brief §5 (data for effort mapping), §6 (granular stats store),
  §9 (V1 inputs), and supplement §5 (outcome capture), §9 (decision record).
  Inventory only. Design choices are out of scope.

## 0. Summary

1. **The primary event ledger already exists.** The OpenCode SQLite DB
   `~/.local/share/opencode/opencode.db` (21.5 GB live, WAL 66 MB) holds
   `route_attempt` (one row per model call: provider, model, effort/variant,
   outcome, first-token ms, response ms, tokens in/out/reasoning/cache, cost,
   quota-reset JSON, failover target, credential). It has 649,081 rows, covering
   2026-10-24 15:25 to now. It also holds `route_decision` (2.43 M routing
   decisions with candidate lists and weight layers), `route_health` (63 k
   cooldown and quota events), and the session tables `session_v2` (19,617
   sessions, 3,417 of them sub-sessions) and `session_message` (619,611 rows,
   tool calls, reasoning, and turn timing, from 2026-01-18).
2. **Legacy tables are frozen.** `message` (55,004 rows, 8.0 GB on disk) and `part`
   (246 k rows, 962 MB) stop at **2026-10-06 01:23**, as the exporter comment
   says. Any tool-usage or per-part metric must read `session_message` (which
   has its own `type`/`content` shape). The pm-stack analytics scan reads
   `session_message`; `analytics-store.ts` already does (checked, see S8).
3. **The AA source of record moved.** The local `aa-results.json/.md` named
   in brief §9 does **not exist** anywhere searched. Per Drew's 2026-11-12
   ruling, the Supabase `ainventory` schema is the source of record. The
   pm-stack copy is `~/.local/state/pm-aggregator/aa-last-good.json` (653 KB,
   AA v5 pull, 298 rankings, 45 configs, 208 test runs, 13 prices, 1,144
   threshold rows). The hatch-personal CSV export of the workbook sits under
   `~/git/hatch-personal/workspace/supabase/ainventory-import/sheets/`.
4. **Two named V1 sources are missing.** The Roadmap & Stats pipeline at
   `~/workspace/goals/keep-the-four-repos-healthy/hidden_files/roadmap-stats/`
   does not exist. Neither do `~/workspace/model-review-2026-11-12/` nor a
   local AI Benchmarks Space mirror. The `pool-intelligence` daily cron
   points at `hidden_files/pool-intelligence-instruction.md`, which is also
   absent. Nothing measured here stands in for those; they are flagged as gaps.
5. **Cost is not a usable axis yet.** Only 50,367 of 649,081 `route_attempt`
   rows carry `cost_usd > 0`. `route_usage` reports `$0.00` for every target in
   its 24 h window. Subscription traffic is priced at zero, so an AA-cost
   axis must come from AA prices × tokens, not from the ledger.
6. **Per-subcommand fleet and mcpx counts are inferred.** Both are counted by
   scanning shell/tool inputs. `fleet <sub>` appears in 37,171 tool-call
   messages since 2026-11-06, and `mcpx_exec`/`mcpx_discover` names are real
   tool names. No invocation ledger exists: code#9373 (fleet) and code#9374
   (mcpx daemon) are both **OPEN**.
7. Queue (`repo-queue`) task rows carry the claim-model stamp (code#8176, closed
   2026-11-12 by PR #8178). Their schema has no notes table and no typed blocker
   links. Queue rows are on a remote backend and were not measured.

## 1. Source catalog

Columns: ID · location · owner/producer · granularity · time range (measured) ·
retention · cadence/freshness · brief §5/§6 dimensions covered.

| ID | Source (location) | Owner / producer | Granularity | Range measured | Retention | Freshness / cadence | Covers |
|---|---|---|---|---|---|---|---|
| S1 | `opencode.db` table `route_attempt` (`~/.local/share/opencode/opencode.db`) | OpenCode route proxy (`bin/opencode-route-proxy.mjs` `recordAttempt`), written per model call | **event** (one row per call, incl. failover legs) | 2026-10-24 15:25 → 2026-11-13 12:05 (epoch ms 1792855506071 → 1794571541842) | none in-repo (live DB; backups exist as `opencode.db.bak-*`) | write per call, current | provider/model/effort; pool (route_id); attempts; success/failure/timeout; TTFT; response ms; tok/s; tokens; cost; quota reset; credential |
| S1b | `route_decision` (same DB) | route proxy | event (one per selection) | 2026-10-24 → now; 2,428,192 rows | none | per selection | pool choice, candidate list, chosen, weight layers, exclusions (`quota-skip`, `cooldown-until`) |
| S1c | `route_health` (same DB) | route proxy | event | 2026-10-24 → now; 63,014 rows | none | per event | cooldown start/end, quota-gated/readmitted, rate-limit-window, infra-hold (+ credential_id) |
| S2 | `session_v2` (same DB) | OpenCode core (v2 schema) | rollup per session (cumulative cost/tokens) + row per session | 2026-01-18 → now; 19,617 sessions; 3,417 with `parent_id` | none | live | session tree (parent_id), agent, model JSON (pool id as providerID `opencode-route`), tokens totals, cost (zero on subscription), diff summary (468 rows), time created/updated/compacting/archived/idle, subagent_policy |
| S2b | `session_message` (same DB) | OpenCode core | event (types: assistant 534,861; synthetic 31,423; user 26,587; idle 22,162; system 3,438; model-switched 341; location-switched 335; compaction 302; agent-switched 170) | 2026-01-18 → now; 619,611 rows | none | per turn | turn timing (`time.created`, streamed), tool calls (`content[type=tool]`, name, state.input), reasoning, model/agent switches |
| S2c | `message` / `part` (same DB, **legacy, frozen**) | OpenCode core (v1) | event | to **2026-10-06 01:23** only; message 55,004 rows; part 246,216 rows | none | frozen | tool/text/reasoning/patch parts before Oct 6 only |
| S2d | `event` table (same DB) | OpenCode event sourcing | event | 3,757 rows, 14.7 MB | none | per event | not analysed in this pass |
| S3 | `~/atlas/opencode-usage-export/history.jsonl` (871 MB, 1,002 ticks) and `latest.json` (1.4 MB) | OpenCode usage exporter (`opencode-usage-export.mjs`, pull from S2) | rollup per tick: sessions with cost/tokens/`messages.byModel` | ticks 2026-11-03 00:25 → 2026-11-13 12:00 (last tick 12:00:17Z); `latest` = 1,410 sessions, 24 h window | none (append-only history) | ~4–5 ticks/h (inferred from 1,002 lines over ~9.5 days) | session cost/tokens per model (24 h windows); child sessions; moved-store detection |
| S4 | `~/.local/state/pm-aggregator/snapshots/usage-YYYY-MM.jsonl` (`usage-2026-10` 5,241 rows / 1.1 MB; `usage-2026-11` 10,736 rows / 2.2 MB) | pm-aggregator usage snapshot store (`src/snapshots.ts`), written by `scripts/append-usage-snapshots.ts`, quota-gatherer | event (dated % readings, pool windows) | from 2026-10 (file names) to now | 13 months raw, then monthly rollups `usage-YYYY-MM.rollup.jsonl` (min/max/last) | per gatherer run | quota % per pool window (`quota-current.json` for the live read) |
| S5 | AA data: Supabase `ainventory` schema (source of record since 2026-11-12); pm-stack copy `~/.local/state/pm-aggregator/aa-last-good.json` (653 KB); workbook CSVs in `~/git/hatch-personal/workspace/supabase/ainventory-import/sheets/` | Pool-intelligence / AA pull (`src/aa.ts`, `src/aa-batch-sql.ts`); workbook deprecated 2026-11-12 | event rows for runs (`Test_Runs`, `Run_Metrics`, `measurement_events` append-only) + rollup (`Model_TestVersion_Stats`) | AA|v5 (5.0.1) from 2026-10-31; v4.3.2 from 2026-09-28; batch load 2026-11-12 | append-only events; workbook deprecated | per AA pull (manual/cron, last 2026-11-12) | AA score, cost/task, TTFT, tok/s, price/M, context window, version key, test date, effort variant; CAI v1.5 (27 rows) |
| S5b | AA local pull `~/workspace/model-review-2026-11-12/aa-results.{md,json}` | — | — | **not found** | — | — | (named in brief §9; absent) |
| S6 | `repo-queue` task store, Postgres default / Turso option (`pkgs/fleet/crates/fleet-queue/src/turso_queue.rs` DDL) | fleet-queue broker | task row + `events` (append, `kind`, `detail` JSON) + `repo_task_lane_ledger` | remote; not measured | not in repo | live | task status/priority/attempts; claim_provider/claim_model/claim_variant/claim_session_id (code#8176); lane; park/wait; blocked_deps (text); last_checked_at; completed_at; events |
| S7 | `~/.local/state/pm-aggregator/fleet-feed.json` (2.6 MB), `events.json` (3.9 MB), `decisions.json` (2.4 MB), `decision-events.jsonl` (95 lines), `decision-events-outbox.jsonl` (36 KB), `issue_pr_links.json` (50 KB), `notifications.jsonl` (2,104 lines), `value-sessions-7d.json` (1.9 MB), `value-scan-attr.json` (7 MB) | pm-aggregator collectors (`collect-fleet-feed.ts`, `decision-events-sync.ts`, `charges-match.ts`) | mixed: snapshot JSON + JSONL events | live | local state dir (no rotation seen) | per collector run | issue/PR linkage, decision events, value attribution (sessions 7d) |
| S8 | pm-stack analytics read path: `/api/quota-analytics.json` (server.ts:2620), `/api/aa.json` (server.ts:2768), `/quota#analytics` view (`quota-analytics-view.ts`); data layer `analytics-store.ts` | pm-stack aggregator | **derived** (read-only over S1/S2/S4; no own store) | live | — | per request | burn by pool/model/effort (S1), activity (S2 `session_v2`/`session_message`/`part`), token ratios, projection, WoW, fleet inferred |
| S9 | Fleet tool ledgers: fleet CLI (code#9373, OPEN), mcpx daemon (code#9374, OPEN) | — | **none** | — | — | — | per-subcommand usage is inferred from S2b shell inputs only |
| S10 | Route history API `route_usage` (MCP/OpenCode tool) | route proxy | derived over S1/S1b | 24 h / any window; 2026-11-12 → 2026-11-13 seen | — | per call | per target (model/pool/provider) requests, failed, metered tokens, cache read, hit ratio, saved $, sessions, pool used/offered/chosen; per day; per session |
| S11 | Stats pipeline `roadmap-stats` (Roadmap & Stats Space daily pipeline) | — | — | **not found** | — | — | (named in brief §9; absent at stated path; `hidden_files/` has only roadmap-sync logs and an unrelated `stats-5998` GitHub/npm download script in hatch-personal) |
| S12 | AI Benchmarks Space | external | — | **no local mirror found** | — | — | (named in brief §9; only the workbook/CSV and aa-last-good copies exist) |
| S13 | Route config / change markers: `~/.local/share/opencode/opencode.jsonc` (+ `.bak-*` snapshots: `bak-effort-*`, `bak-luna-*`, `bak-20261102`) | operator | versioned file backups | 2026-11-02 → 2026-11-13 | local backups | on change | change markers (config backups) — unstructured |
| S14 | GitHub issues/PRs (`developing-today/code`) | GitHub | live API | — | GitHub | live | issue↔PR↔task linkage (via `issue_pr_links.json`, S7) |

## 2. Per-source detail

### S1 — `route_attempt` (opencode.db)

Schema (live, `sqlite3 .schema`): `id`, `time_started`/`time_ended` (epoch
ms), `session_id`, `assistant_message_id`, `route_id` (pool), `provider_id`,
`model_id`, `variant` (effort), `outcome` (success 532,064; failure
104,784; timeout 12,047; interrupted 166), `error_tag`, `error_code`,
`error_status`, `error_message`, `error_body`, `retryable`, `output_started`,
`failed_over_to`, `first_token_ms`, `response_ms`, `tokens_per_second`,
`tokens_input`, `tokens_output`, `tokens_reasoning`, `tokens_cache_read`,
`tokens_cache_write`, `tokens_estimated` (161 rows), `quota` (JSON:
`{"at":ms,"reset":{"unified-5h":"<epoch>","unified-7d":"<epoch>"}}`, the
reset instants only, **no percent**), `hedged` (always 0), `cost_usd`
(nonzero on 50,367 rows), `continuation_of` / `continuation_index` (13 rows),
`cache_savings_usd`, `credential_id` (456,794 non-null).

Measured: 649,081 rows; 161 distinct provider+model pairs; 29 routes;
variants `xhigh` 308,239, `high` 255,355, `medium` 46,401, `low` 27,934,
null 6,212, `default` 4,387, `max` 496, `none` 34, `minimal` 3. Success
rows: mean `first_token_ms` 2,157; mean `response_ms` 9,061. Rows with
`session_id` NULL exist (≈14.6 k, 649,081 − 634,443; the proxy test fixtures
write NULL for the zero-session proof, so treat as "unattributed" not as a
bug; not checked further).

Coverage against brief §5: provider/model/effort ✔; pool ✔ (route_id);
attempts ✔ (one row per leg; `failed_over_to`); success rate ✔ (outcome);
TTFT ✔; tokens ✔; time (call-level only, not per task/PR); quota ◐ (reset
instants only; % must be joined from S4); diff/triage/time-to-complete ✘.

Retention: none in repo; the live DB file is 21.5 GB and the `.bak-*`
snapshots are 21.4 GB each (three of them). Growth: the table is ~150 MB
(`dbstat`), so ~230 bytes per row; the DB is dominated by `session_message`
and `route_decision`.

### S1b — `route_decision`

Columns: `time`, `session_id`, `route_id`, `selection` (e.g. `ordered`),
`variant`, `candidates` (JSON list), `chosen`, `reason` (`sticky` …), `detail`
(JSON: `weights`, `layers` per candidate with `base/scheduled/prefer/
adjustment/demotion/gate`, `excluded` list with reason `quota-skip` /
`cooldown-until:<ms>`). 2,428,192 rows, about 3.1 GB. Gives pool-selection
dimension and the reason for every skip; this is the best source for "which
pool was offered vs chosen" (already used by `route_usage`).

### S1c — `route_health`

Kinds: `cooldown-start` 58,590; `cooldown-end` 2,298; `quota-gated` 730;
`quota-readmitted` 571; `rate-limit-window` 806; `infra-hold` 21. Per
provider+model, with `credential_id`. Useful as change/incident markers and
as the "walled" signal. About 6.6 MB.

### S2 — `session_v2`

19,617 rows (about 9 MB). Columns include `parent_id` (3,417 children),
`fork_session_id`, `fork_boundary`, `agent`, `model` (JSON:
`{"id":"workhorse","providerID":"opencode-route"}` — the **pool** is recorded
as the model when routed), `cost` (0 for subscription traffic), tokens
input/output/reasoning/cache_read/cache_write (session totals),
`summary_additions`/`summary_deletions`/`summary_files` (**468 of 19,617**
rows populated; sums 22,247,223 added and 2,333,205 deleted, so this is the
only real diff source and it is sparse), `time_created`/`time_updated`,
`time_compacting`, `time_archived` (28), `time_suspended`, `time_idle`,
`idle_outcome`, `resume_attempts`, `subagent_policy`, `detached`,
`workspace_id`, `path`. Sub-session depth needs recursion over `parent_id`
(not measured in this pass).

### S2b — `session_message` (live turn stream; the main source for V1 turns and tools)

619,611 rows, about 7.7 GB. Types: assistant 534,861; synthetic 31,423; user
26,587; idle 22,162; system 3,438; model-switched 341; location-switched 335;
compaction 302; agent-switched 170. `data` JSON carries `time.created`,
`agent`, `model {id, providerID, variant}`, `content[]` (types include
`reasoning`, `text`, `tool` with `name`, `state.status`, `state.input`,
`snapshot`).

Tool calls (`content.type = tool`) since 2026-11-06 (filter
`time_created > 1794000000000`): 181,982 messages. Top names: `shell` 146,524;
`execute` 34,141 (code-mode); `read` 19,886; `grep` 9,135; `patch` 4,239;
`glob` 2,875; `subagent` 1,860; `edit` 1,441; `mcpx_discover` 1,065;
`write` 895; `webfetch` 888; `mcpx_exec` 599; `skill` 445; `websearch` 270;
`pty` 227; `route_usage` 41; `plan_info` 10; `question` 9. mcpx tool names
total 1,681 in that window.

Fleet subcommand counts (from `state.input.command` of `shell`, regex
`\bfleet\s+<sub>`, same window): `queue` 3,061; `reconciliation` 1,136;
`repos` 109; `gh` 95; `help` 91; `repo` 76; `secrets` 72; `deploy` 70;
`serve` 63; `proxy` 55; `worktree` 29; `doc` 25; `pm-stack` 24; `version` 20;
`binary` 19; `work` 19; `pkgs` 17; `runner` 16; `dispatch` 16. Inferred from
text, not from a ledger (code#9373).

The tool-argument shapes for mcpx are not per-action: `mcpx_exec` carries
the TypeScript script as input, so `tools.<ns>.<tool>` calls inside it are
**not** broken out by the store. That is gap G4.

### S2c — `message` / `part` (legacy)

Frozen at 2026-10-06 01:23 (both `message` and `part` max `time_created`).
Holds the reasoning/patch/text/step parts for the pre-October history:
step-start 49,141; step-finish 48,764; tool 46,773; text 27,938; reasoning
15,039; patch 3,263; compaction 327. `pm-stack/aggregator/src/analytics-store.ts`
comments say the activity scan uses `session_v2` + `session_message`, and the
code (lines ~283–335) does read `session_message`. Nothing in the pm-stack reads
`part` for post-Oct-6 metrics. Legacy `part` is only relevant to pre-Oct-6 history.

### S3 — `opencode-usage-export` (~/atlas)

Pull exporter (`opencode-usage-export.mjs`, 2026-11-02 slice of
developing-today/code#2072). Document: `service`, `source`, `status`,
`measuredAt`, `storePath`, `storeNewestAt`, `windowHours` (24), `moved`
(null means the DB path has not changed), `sessions[]` with `id`, `parentId`,
`agent`, `directory`, `model {id, providerID}`, `cost`, `tokens {input,
output, reasoning, cacheRead, cacheWrite}`, `messages {total, assistant,
byModel[]}`, `timeCreated`, `timeUpdated`; `totals`. `state.json` records the
last tick (2026-11-13T12:00:17Z). The history is append-only JSONL of whole
documents, so each tick is a snapshot of 24 h. Not a fact table; a rollup.
Failure documents (store-missing / store-stale / schema-drift / read-failed)
are explicit, never zero-filled.

### S4 — usage snapshot store (pm-aggregator)

JSONL, one line per dated reading: service, metric, pool window, value
(percent or USD), observation time, source class (`api`, `console`, `manual`,
`estimate`, `invoice`), unit (`pct`, `usd`, `usd-est`, `tokens`, …). Rollup
kept after 13 months (min/max/last per service+metric). This is the only
store of **quota percent over time**; it is what `analytics-store.ts` joins
into projections (`UsageView.history`). No attempt/session link.

### S5 — AA data

Source-of-record tables in `ainventory` (per `aa-load-2026-11-12/README.md`):
`sources` (kind `aa-web`, `drive-workbook`), `sheet_rows` (sheet_name,
row_index, cells), `measurement_events` (append-only; 234 events under the
2026-11-12 batch), `price_events` (9 appended), `models` (slug rows), `ingest_batches`.
Views: `v_rankings`, `v_threshold_metrics`, `v_latest_import_batch`.

Local copies and their fields:

- `aa-last-good.json` (653 KB): `batch`, `configFingerprint`, `fingerprint`,
  `sqlRevision`, and `section` {`sources` 5; `rankings` 298; `thresholds` 1,144;
  `configs` 45; `testRuns` 208; `cai` 27; `prices` 13; `batches` 8; `catalog`
  233; `coverage`}. Ranking row fields: `rowIndex`, `testRunKey`,
  `modelVariantKey`, `family`, `creator`, `effort`, `benchmarkVersionKey`, raw
  score, levelled score, cost task, time task, level factor, cumulative
  factor. `prices` fields: `modelName`, `inputPer1M`, `outputPer1M`,
  `cacheReadPer1M`, `cacheWritePer1M`, `contextMinTokens`, `contextMaxTokens`,
  `windowKind`. `catalog` fields: `id`, `provider`, `providerName`, `model`,
  `name`, `contextLimit`, `outputLimit`, `variants[]`.
- Workbook CSV (hatch-personal `sheets/`, deprecated as source but a full copy):
  `Test_Runs` (183 rows) with `test_run_key`, `model_variant_key`,
  `benchmark_version_key`, `test_run_date`, `snapshot_date`, `raw_score`,
  `cost_per_task_usd`, `total_index_cost_usd`, `output_tokens_per_task`,
  `reasoning_tokens_per_task`, `output_speed_tps`, `ttft_seconds`,
  `first_answer_seconds`, `e2e_500_seconds`, `time_per_task_seconds`,
  input/output/cache-hit price per M, `context_window_tokens`, `source_url`,
  `data_quality`. `Run_Metrics` (143 rows: levelled score, `points_per_$`,
  `$/point`, normalised intel/speed/cost). `Threshold_Metrics` (1,144 rows).
  `Model_TestVersion_Stats` (111 rows): per model×version `run_count`,
  `score_sum`, `score_sq_sum`, `avg_score`, `stdev_s`, `min`, `q1`, `median`,
  `q3`, `max`, cost count/sum/avg, time count/sum/avg, speed count/sum/avg,
  `level_factor`, `levelled_avg`, `$/levelled_point`. This is already a
  quartile-and-stdev rollup, the closest V1 artifact to brief §5 statistics.
  `Model_Variants` (145), `Model_Catalog` (61; `context_window_tokens`,
  `release_date`, `currently_available`, no lineage), `Benchmark_Versions`
  (101; cumulative factor, `dual_test_count`, confidence), `Version_Crosswalk`
  (3 rows, only self-pair), `Coding_Agent_Results` (27; CAI v1.5 with
  `deep_swe_v1_1_pct`, `terminal_bench_4_0_pct`, cost, time, token usage,
  turns), `Subscription_Value` (124), `Plan_Events` (40), `SaaS_Plan_Catalog`
  (60), `Chart_Data_Reviewed` (59), `Maintenance_Log` (38).

The `pool-intelligence` cron says to run AA pulls per pool and to record
"our own measured speeds per provider/model/effort". Those measured figures
are S1 (`output` tok/s is `tokens_per_second`), not an AA field.

### S6 — repo-queue (task store)

From the `tasks` DDL in `turso_queue.rs`: `id`, `queue`, `title`, `body`,
`payload`, `dedupe_key`, `priority`, `status` (default `queued`), `not_before`,
`worker_id`, `claimed_at`, `lease_until`, `lane_id`, `platform`, `attempts`,
`max_attempts` (4), `claim_provider`, `claim_model`, `claim_variant`,
`claim_session_id` (the code#8176 stamp), `progress`, `result`, `last_error`,
`park_kind`, `wait_kind`, `ready_condition`, `last_checked_at`,
`check_outcome`, `created_at`, `updated_at`, `completed_at`, `blocked_deps`
(text), `dep_next_recheck_at`, `dep_blocked_until`, `dep_recheck_seconds`.
Also `events` (`worker_id`, `task_id`, `kind`, `detail`, `created_at`),
`repo_task_lane_ledger` (assigned/executed lane, exclusion reason,
opencode/platform limit), `leases`, `lanes`, `workers`, `checkpoints`,
`creation_outbox`. `doc/2026-11-10_design_queue_task_data_model_3424.md`
says the live rows carry `ready_condition`, `last_checked_at`, `check_outcome`,
`park_kind`/`wait_kind`, and `result.park_reason`, and that there is **no
notes table and no typed blocker link** (`blocked_deps` is free text).
Queue data is remote and was not measured here.

### S7 — pm-aggregator state (collector outputs)

- `issue_pr_links.json` (50 KB): the only local issue↔PR link store found.
  Shape not opened in this pass; flagged for the linkage design.
- `decisions.json` (2.4 MB) and `decision-events.jsonl` (95 lines, 27 KB) and
  `decision-events-outbox.jsonl` (36 KB): the decision-event stream written
  by `decision-events-sync.ts` (last write 2026-11-13 05:35). This is the
  nearest existing store for supplement §9 (decision record), but its fields
  were not enumerated; it has no prompt version, no points gauges (supplement §3).
- `events.json` (3.9 MB), `notifications.jsonl` (2,104 lines), `fleet-feed.json`
  (2.6 MB): fleet feed and notification history.
- `value-sessions-7d.json` (1.9 MB), `value-scan-attr.json` (7.0 MB), and
  `value-*.json` (≈ 6 files): session value attribution scratch outputs, one
  scan, not a store.

### S8 — pm-stack analytics (derived)

`analytics-store.ts` opens S1 and S2 read-only with `bun:sqlite`, degrades to
an explicit unavailable state on failure, and reports burn by pool/model/effort
from `route_attempt`, activity from `session_v2`/`session_message`/`part`,
fleet inferred, and pool history from S4. `quota-analytics.ts` holds pure
projection math (least-squares rate, walled / projected / no-wall /
insufficient-history). Its tests are `src/quota-analytics.test.ts`. It
doubles as a working prototype of the §5 aggregations (sum/median not yet;
mean and rate yes; the scan reads the `session_message` `skill`/`shell` parts).

### S10 — `route_usage` (tool, measured 24 h, 2026-11-12 12:05 → 2026-11-13 12:05)

Reports: per target (`provider/model`, and pool) requests, failed, metered
tokens, cache-read tokens, hit ratio, saved $, sessions, and pool
used/offered-not-chosen/configured-not-reached; per day; per session (last
time, requests, tokens, pools, models); continuation chunks (0 here). Totals
over the window: 2026-11-12 24,262 requests / 1.20 B metered tokens; 2026-11-13
(partial) 32,605 / 454 M. All costs are `$0.00`. Useful as a live check;
not a store.

## 3. Coverage matrix

Cells: **F** full (directly stored, correct grain), **P** partial (stored but
wrong grain, sparse, or needs a join or inference), **N** none (no
store found), **—** not applicable.

Source columns: S1 `route_attempt` · S1b/c `route_decision`+`route_health` ·
S2 `session_v2`+`session_message` · S3 usage-export history · S4 quota snapshot
store · S5 AA (Supabase / aa-last-good / CSV) · S6 repo-queue task rows ·
S7 pm-aggregator state (decisions, issue_pr_links, events) · S9 fleet/mcpx ledgers.

| Dimension (brief §/supplement §) | S1 | S1b/c | S2 | S3 | S4 | S5 | S6 | S7 | S9 |
|---|---|---|---|---|---|---|---|---|---|
| §5 provider / model / effort | F | P | P | P | N | F | P (claim_model/variant) | P | — |
| §5 pool | F (route_id) | F (route_id, chosen) | P (pool is `model` JSON) | P | N | N | N | N | — |
| §5 ratio per session | P | N | F (session tokens) | F | N | — | N | N | — |
| §5 sub-sessions: count | P (session_id only) | — | F (`parent_id`) | P | N | — | N | N | — |
| §5 sub-sessions: depth | N | — | P (recursion needed) | N | N | — | N | N | — |
| §5 models / pools / sub-sessions used | F | F | F | F | — | — | P | P | — |
| §5 tool usage (names) | N | — | F (since 2026-11-06 in `session_message`; pre-Oct-6 in `part`) | N | N | — | N | P | — |
| §5 mcpx per subcommand/action | N | — | P (tool name `mcpx_exec`/`mcpx_discover` only) | N | N | — | N | N | N (code#9374 OPEN) |
| §5 fleet per subcommand | N | — | P (inferred from shell text) | N | N | — | N | N | N (code#9373 OPEN) |
| §5 real diff +/- | N | — | P (468 sessions only) | N | N | — | N | N | — |
| §5 attempts | F (one row per leg; continuation 13) | F | P | — | — | — | P (`attempts`) | — | — |
| §5 triages | N | N | N | N | N | N | P (`park_kind`/`wait_kind`/`check_outcome`, not a triage record) | N | N |
| §5 days to complete: created → started → completed | N | — | P (session created/updated) | — | — | — | P (`created_at`, `claimed_at`, `completed_at`) | N | — |
| §5 hours/minutes of actual work | P (call timings only) | — | P (turn timings) | — | — | — | N | N | N |
| §5 success rate | F (`outcome`) | F | P | — | — | — | P (`status`) | — | — |
| §5 time per session / task / PR / attempt | P (attempt only) | — | P (session only) | P | — | — | N | N | N |
| §5 quota % per session / task / PR / attempt | P (reset instants only, no %) | P | N | F (pool windows, dated) | F (pool windows) | — | N | N | — |
| §5 stats: min/max/stddev/count/median/mean/sum | N (no rollups) | N | N | P (sums only) | P (min/max/last rollup) | F for AA metrics (`Model_TestVersion_Stats`: min, q1, median, q3, max, stdev, sums) | N | N | — |
| §5 stats: p90 / p99 / p99.9 | N | N | N | N | N | N | N | N | N |
| §5 stats: IQR / quartiles | N | N | N | N | N | P (q1/q3 in CSV, AA only) | N | N | N |
| §6 event grain with timestamp | F (S1) | F | F | P (rollup) | F | P (test runs; not fact-tagged) | F (`events`) | F | — |
| §6 AA version + test date | — | — | — | — | — | F (`benchmark_version_key`, `test_run_date`, `snapshot_date`) | — | — | — |
| §6 model successors / predecessors | N | N | N | N | N | P (`replaced_by` in qualitative sheet only) | N | N | N |
| §6 price-matched model-effort list | N | N | N | N | N | P (`Subscription_Value`, `Model_Variants`) | N | N | N |
| §6 capabilities / tags | N | N | N | N | N | P (qualitative 1–5 ratings; `role_*` columns) | P (labels absent) | N | N |
| §6 variants split as separate (e.g. DeepSeek 1107) | P (`model_id`/`variant` distinct) | — | P | — | — | P (`model_variant_key`) | — | — | — |
| §6 change markers (dates + measured change) | P (`route_health`) | F (cooldown/quota events) | N | N | N | P (`Maintenance_Log`, `Config_History`) | P (`events`) | P (decision-events) | N |
| §6 storage growth measurement | (this doc §4) | — | — | F (file sizes) | F | — | — | — | — |
| §6 rollups by time since inject / capture / AA version | N | N | N | N (monthly only) | N | N | N | N | N |
| §6 group-by any tag | N | P (fields only) | P | N | N | P | P | P | — |
| §6 model reviewable per sample | F | F | F | P | F | F | F | F | — |
| §3 context limits (in/out/cache/total) | P (tokens per call, no limit column) | — | P (session totals) | — | — | F (`context_window_tokens`, `contextMaxTokens`) | — | — | — |
| §3 "strictest limits" per model | N | N | N | — | — | P (catalog `contextLimit`, `outputLimit`) | N | N | N |
| §7 off-peak / peak windows | P (`route_health`, `route_decision`) | P | N | N | N | N | N | N | — |
| §7 DeepSeek off-peak / Luna 100k / 388k caps | P (`route_decision` candidates) | P | N | N | N | P (`Subscription_Value`) | N | N | — |
| §7 real vs AA speed | F (`tokens_per_second`, `first_token_ms`) | F | P | — | — | F (AA speed) | — | — | — |
| §7 real cost per %-dollar | P (`cost_usd` 50,367 rows; subscription 0) | P | P (cost 0) | F (pct readings) | F | P | — | — | — |
| §8 testing plan: trials, round-robin | P (`route_decision` `selection`) | P | — | — | — | N | N | P (`Plan_Events`, notes) | — |
| supplement §5 outcome: PRs merged / issues closed / reverted / rework / undone | N | N | N | — | — | — | P (`status`, `completed_at`) | P (`issue_pr_links.json`, `events`) | N |
| supplement §5 linkage issue ↔ PR ↔ task ↔ session | N | N | N | — | — | — | P (`claim_session_id`, `dedupe_key`) | P (`issue_pr_links.json`) | — |
| supplement §5 TTFT / first TTFT / step time / step count | F (`first_token_ms`, `response_ms`) | F | P (turn `time.created`; no TTFT) | — | — | — | — | — | — |
| supplement §5 tokens in / cached / context / thinking | F (`tokens_input`, `cache_read`, `reasoning`) | F | F (session totals; reasoning parts) | F | — | — | — | — | — |
| supplement §5 question prompts / questions submitted | N | N | P (`question` tool count 9 since 11-06) | — | — | — | N | N | — |
| supplement §5 errors auto-rerouted | F (`failed_over_to`, `error_tag`) | F | N | — | — | — | N | — | — |
| supplement §5 decision model & decision | N | P (`route_decision`, the routing decision) | — | — | — | — | N | P (`decision-events.jsonl`, `decisions.json`) | — |
| supplement §9 decision prompt with version and date | N | N | N | — | — | — | N | N | — |
| supplement §9 tokens / % / time to decision | P (`route_decision` time; tokens on route_attempt) | P | — | P | — | — | — | P | — |
| supplement §9 successful session goal (sub/parent/anyone perspective) | N | N | N | — | — | — | N | N | N |
| supplement §3 points gauges 0–5 (caller/callee) | N | N | N | — | — | — | N | N | — |
| supplement §4 invalid-JSON path and final answerer | N | N | N | — | — | — | N | N | — |
| supplement §6 effort on decisions (tie-break vs fallback) | N | N | N | — | — | — | N | N | — |
| supplement §7 conditional clause evaluation trace | N | N | N | — | — | — | N | N | — |
| supplement §8 PM-stack review (labels, scores) | N | N | N | — | — | — | P (`queue-view.ts`, no label store found) | P (`comments.json`, `actions.json`) | — |
| supplement §10 cycle-safe traversal evidence (visited-set) | P (`route_decision` `excluded` list) | P | — | — | — | — | — | — | — |

## 4. Measured sizes (cheap measurements; dbstat MB and file sizes)

| Object | Size | Rows | Note |
|---|---|---|---|
| `opencode.db` (live) | 21.55 GB | — | + WAL 66.5 MB; `.bak-2026-11-02-preswitch` 21.37 GB; `.bak-2026-11-11-openai-rename` 21.37 GB |
| `session_message` | 7,664 MB | 619,611 | dominant; the turn stream |
| `message` (legacy, frozen 2026-10-06) | 8,031 MB (dbstat) | 55,004 | do not use for new data; the MB figure is page-based and may include freed pages |
| `route_decision` | 3,142 MB | 2,428,192 | selection log |
| `part` (legacy, frozen 2026-10-06) | 962 MB | 246,216 | do not use for new data |
| `route_attempt` | 150 MB | 649,081 | the event ledger; ~230 B/row |
| `route_health` | 6.6 MB | 63,014 | |
| `session_v2` | 9.0 MB | 19,617 | |
| `event` | 14.7 MB | 10,611 | sequencing events (not analysed) |
| `~/atlas/opencode-usage-export/history.jsonl` | 871 MB | 1,002 documents | ~0.87 MB per tick; grows ~85–105 MB/day at 4–5 ticks/h (inferred cadence) |
| `~/atlas/opencode-usage-export/latest.json` | 1.4 MB | 1,410 sessions | |
| `~/.local/state/pm-aggregator/snapshots/` | 3.3 MB | 15,977 readings (5,241 Oct + 10,736 Nov) | |
| `~/.local/state/pm-aggregator/` (whole) | 53 MB | — | includes `aa-batches/` (9 AA batch JSON files) |
| AA pm-stack copy `aa-last-good.json` | 0.65 MB | 298 + 1,144 + 208 + 233 + 45 + 27 + 13 | |
| `decisions.json` | 2.4 MB | — | |
| `decision-events.jsonl` | 27 KB | 95 | |
| `notifications.jsonl` | 0.8 MB | 2,104 | |
| hatch-personal `ainventory-import/sheets/` | ≈1.2 MB total | ≈ 2,400 CSV lines | workbook CSV copy |
| `repo-queue` local state (`~/.local/state/repo-queue`) | 28 KB | — | queue DB is remote; not measured |

Growth estimate for the granular store (brief §6 "measure storage growth"):
`route_attempt` alone is about 230 B/row at about 40–60 k rows/day in
Nov 2026 (2026-11-09 66,996; 11-10 47,400; 11-11 45,735; 11-12 44,392;
11-13 partial 32,651), so about 10–15 MB/day; the full-fidelity event store
per session would also carry `session_message` (about 12 KB/turn measured
as 7.7 GB / 619 k), which dominates. This is order-of-magnitude only.

## 5. Gaps that need new capture

| # | Gap | Impact on §5/§6 | Owner (existing ticket) | Suggested minimal capture |
|---|---|---|---|---|
| G1 | No invocation ledger for `fleet` subcommands | per-subcommand usage is inference only | code#9373 (OPEN) | append-only invocation table: subcommand, action, session, time, exit |
| G2 | Legacy `message`/`part` frozen at 2026-10-06; any new reader must use `session_message` | pre-Oct-6 history only in legacy tables | none (analytics-store already reads `session_message`) | the stats store reads `session_message`; mark pre-Oct-6 as legacy-shape |
| G3 | No mcpx call ledger | per tool/action mcpx counts missing (only names via `mcpx_exec`/`mcpx_discover`; the TypeScript inside is opaque) | code#9374 (OPEN) | daemon-side `(namespace, tool, ok, ms, session)` rows |
| G4 | `cost_usd` zero for subscription traffic; `route_usage` shows $0.00 | no real cost-per-point or cost-per-% axis from the ledger | none | keep AA prices × tokens as the cost basis; record quota % per attempt (below) |
| G5 | Quota % not on attempts (only reset instants in `quota` JSON) | cannot attribute quota % to session / task / PR / attempt | none | join S4 pool readings by time window onto `route_attempt`, or record the pool % read at call start/end |
| G6 | No triage record or triage outcome | triage counts missing (§5) | none (queue has `park_kind`/`check_outcome` only) | triage event type in the queue `events` table (3424 piece 5) |
| G7 | No real per-PR/per-task diff +/- except 468 session summaries | diff +/- is sparse | none | record diff stats at PR merge and link to task (supplement §5) |
| G8 | Queue: no typed blocker links, no notes table, no provenance on every write | design §5 linkage and outcome feedback cannot cite a dependency | code#3424 (design doc 2026-11-10) | blocker link table, notes table, provenance columns |
| G9 | No outcome capture (PR merged / reverted / undone / rework / incident) linked to a task and a decision | supplement §5 learning loop has no data | none | outcome event table keyed by PR/issue/task/decision id |
| G10 | No decision record with prompt version and date, no points gauges (supplement §3, §9) | cannot tell which decision prompt produced which decision; no feedback gauges | none | decision record table: prompt id+version+date, tokens, %, time, answering model, invalid-JSON path |
| G11 | No percentiles (p90/p99/p99.9) anywhere; mean/median/stdev only in AA CSV | brief §5 stats incomplete | none | compute in the stats store; keep raw per-sample rows |
| G12 | No model lineage (successor/predecessor) or AA-to-own-measurement mapping table | brief §6 cannot be filled | none | lineage table keyed by model_variant_key with ordered successors |
| G13 | Roadmap-stats pipeline, `aa-results.{md,json}`, `model-review-2026-11-12/`, AI Benchmarks Space mirror, `pool-intelligence-instruction.md` not found at stated paths | brief §9 V1 inputs named but absent; the Roadmap & Stats source cannot be extended from what is here | none | locate the owners and paths, or accept the AA ainventory copy as the source of record and record the Roadmap & Stats output as a gap |
| G14 | TTFT and turn timing live only in `session_message`; `session_v2` has no start/finish of work | "minutes of actual work" and days-to-complete per task are not derivable cleanly | none | task-level start/end events from the queue (3424 piece 5) |
| G15 | No change-marker table (dated model/config change with expected vs actual) | §6 change markers (e.g. DeepSeek 1107) only in config backups | none | change table keyed by date, scope (model/pool/effort), expected and measured delta |
| G16 | No local copy of the Stats/AI Benchmark tables with time-since-AA-version fields | rollup-by-time-since-AA-version needs a version date per row | none | carry `benchmark_version_key` + `test_run_date` into every fact row |

## 6. Open questions with defaults (detached; not ruled)

- **Q-src-1** Where is `aa-results.md/.json`? Not found. *Default:* treat the
  Supabase `ainventory` schema and `pm-aggregator/aa-last-good.json` as the AA
  source of record (per the 2026-11-12 ruling). Record the missing file as G13.
- **Q-src-2** Is the Roadmap & Stats pipeline still running and at what path?
  Not found. *Default:* not a V1 input for the effort mapping; the stats store
  replaces the need. Ask Drew before depending on it.
- **Q-src-3** Is `route_attempt` the full call ledger, or are direct calls
  (outside the proxy) missing? *Default:* assume the proxy is the single path
  (the route-proxy comments say every attempt is written to `route_attempt`).
  Unverified against direct provider calls.
- **Q-src-4** Should the stats store read the live `opencode.db` directly, or
  the exporter's `history.jsonl`? *Default:* read the live DB read-only for
  events (S1, S2b), keep S3 as the rollup cross-check. S3 history is 871 MB and
  growing, so do not copy it wholesale.
- **Q-src-5** The `session_v2`→`session_message` migration boundary is
  2026-10-06 for legacy and 2026-01-18 for v2. *Default:* treat any metric
  before 2026-10-06 that came from `part` as legacy-shape, and do not mix it
  with v2 counts without a marker.
- **Q-src-6** Are the "quota %" readings in S4 per pool window a one-to-one match
  to the reset instants in `route_attempt.quota`? *Default:* join by time window
  (nearest reading before the call), flagged as inferred.
- **Q-src-7** The `decision-events` stream (S7) may already hold the supplement §9
  decision record. Its schema was not enumerated. *Default:* list it as partial
  and read its fields in the next pass before designing the decision table.

## 7. Method notes

- Read-only throughout. `opencode.db` was opened with `file:...?mode=ro` (Python
  `sqlite3`) and `sqlite3 -readonly`. No writes to any store.
- `route_usage` was called with `hours: 24`. Output reported above.
- Scans of `session_message` and `part` were filtered by `time_created` to bound
  run time. The 2026-11-06 cutoff (`1794000000000` ms) gives about 7 days.
- Git checks (`gh`): code#9373 OPEN, #9374 OPEN, #9378 OPEN (time-of-day routing),
  #9399 OPEN (this program; the Q1 reopen note in `questions.md` is consistent
  with this), #8176 CLOSED 2026-11-12 with PR #8178 MERGED.
- Greps for `route_attempt|quota-analytics|roadmap-stats|aa-results|artificial
  analysis` covered this repo, `~/git/opencode` (origin/HEAD), the openchamber
  fork, `pm-stack/`, `~/workspace`, and `~/git/hatch-personal/workspace`.
  The opencode fork holds `route_attempt` in `packages/core/schema.json` and in
  session-header cost code, not in a store writer.
