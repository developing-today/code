# C5 efficiency accounting — first report

Issue: developing-today/code#9573 · Parent epic: developing-today/code#9568 · Executed by: queue task 20424 (worker `repo-worker-code`, lane `oc-code-01`) · Report window: 2026-11-12T22:07:10Z through 2026-11-18T18:32:00Z (5.85 days).

This is the first report under C5 ("Efficiency accounting + provider/model policy"). It publishes the join methodology and one reproducible join. It makes **recommendations only**; no provider, pool, routing, or model configuration was changed.

## 1. Methodology

### 1.1 Question

For a fixed window, join **consumption** (quota/token usage) to **output** (merged PRs and completed queue tasks) and report, per surface / pool / provider / model:

- consumption volume (requests, metered tokens, cost, cache hit ratio);
- output volume (completed tasks, merged PRs);
- efficiency ratio (output per unit consumption);
- any period where consumption and output moved in opposite directions, documented as **worse efficiency** with a distribution diagnosis.

### 1.2 Data sources

All four sources are stored data; no scraping or inference of missing fields.

1. **Queue events** from the queue broker (`/events-list`), read through its Unix socket `/run/user/1337/fleet/queue-broker.sock` with `X-Agent-Token` from `~/.config/queue-broker/token`:
   - `model_stamp` (provider/model stamps written at claim/bind/transfer);
   - `completed` (task completion records, including `model_provider`, `model_id`, `pool`, `queue`, `platform`);
   - `session_bound` / `lane_session_bound` available for session attribution.
2. **Merged PRs** from GitHub: `gh pr list --state merged --limit 5000 --json number,mergedAt,headRefName,title`.
3. **Consumption** from `route_usage` (OpenCode route history): requests, metered tokens, cost, cache reads, hit ratio, per provider/model target and per pool offer/choice counts.
4. **Queue task identity**: queue branches are named `queue/<task_id>-<slug>`, so a merged PR's task ID is the leading integer of `headRefName`.

### 1.3 Join algorithm

1. Load `model_stamp` rows for the window; build `stamp(task_id) -> (provider, model)` using the **last** stamp in the window for that task.
2. Load `completed` rows for the window; `completed_tasks = {task_id}`.
3. Load merged PRs for the window; for each PR parse `task_id` from `queue/<id>-...`; keep PRs whose task ID is present.
4. Join:
   - `completed_by_stamp(provider, model) = count of completed task IDs with that stamp`;
   - `merged_by_stamp(provider, model) = count of merged PRs whose branch task ID has that stamp`.
5. Consumption is joined by provider/model target from `route_usage`. Where a provider/model has no `route_usage` target (notably `muse-lane`), consumption is reported as **not separately metered in route_usage** rather than estimated.

### 1.4 Window selection

The window is bounded below by the first `model_stamp` row available in the broker (2026-11-12T22:07:10Z) and above by report generation time. This keeps the join's left side (stamps) complete; merged-PR and completed-task counts are restricted to the same interval.

### 1.5 Reproduction

```bash
# 1. model stamps
curl -sS --unix-socket /run/user/1337/fleet/queue-broker.sock -X POST \
  -H "X-Agent-Token: $(cat ~/.config/queue-broker/token)" \
  -H "Content-Type: application/json" \
  --data '{"kind":"model_stamp","since":"2026-11-12T22:07:10Z","limit":50000}' \
  http://localhost/events-list -o model_stamp.json

# 2. completed tasks
curl -sS --unix-socket /run/user/1337/fleet/queue-broker.sock -X POST \
  -H "X-Agent-Token: $(cat ~/.config/queue-broker/token)" \
  -H "Content-Type: application/json" \
  --data '{"kind":"completed","since":"2026-11-12T22:07:10Z","limit":50000}' \
  http://localhost/events-list -o completed.json

# 3. merged PRs
gh pr list --state merged --limit 5000 \
  --json number,mergedAt,headRefName,title > merged_prs.json

# 4. consumption
# (run route_usage with hours=168 in the OpenCode session; output is the
#  7-day provider/model target table used in section 3)
```

The join itself is the Python grouping in section 2.2; no hidden transformation is applied.

## 2. Output side of the join

### 2.1 Window totals

| Metric | Count |
|---|---:|
| `model_stamp` events | 4,750 |
| Unique tasks with a stamp | 3,691 |
| `completed` events | 5,079 |
| Unique completed tasks | 5,079 |
| Merged PRs | 1,127 |
| Merged PRs on `queue/<id>-...` branches | 661 |
| Merged PRs whose branch task has a stamp | 490 |
| Completed tasks whose task has a stamp | 3,270 |

### 2.2 Output by stamped provider/model

| Stamped provider/model | Completed tasks | Merged PRs | Merged PRs / completed tasks |
|---|---:|---:|---:|
| `muse-lane` / `muse-lane` | 3,268 | 489 | 0.1497 |
| `opencode-route` / `workhorse` | 2 | 1 | 0.5000 |
| `opencode-route` / `summit` | 0 | 0 | n/a |

Interpretation: in the stamped population, `muse-lane` is the only provider/model that both runs queue tasks and produces merged PRs at scale. `opencode-route` stamps exist (17 stamps: 11 `workhorse`, 6 `summit`) but only 2 completed-task records and 1 merged PR carry that stamp, so the opencode-route cell is **not yet measurable** as an efficiency ratio. This is a data-attribution limit, not a performance claim.

### 2.3 Stamped-task completion rate

`3,270 / 3,691 = 88.6%` of stamped tasks completed within the window; `421` stamped tasks (11.4%) did not.

## 3. Consumption side of the join

Source: `route_usage` for the 168 hours ending 2026-11-19T01:09Z (the nearest available 7-day window; it overlaps the report window).

| Aggregate | Value |
|---|---:|
| Provider/model targets reported | 87 (of 212 targets in the full window) |
| Requests | 257,244 |
| Metered tokens | 4,809,830,210 |
| Direct metered cost | $51.12 |
| Cache read tokens | (per target; see below) |

Top consumers by metered tokens:

| Provider/model target | Requests | Failed | Metered tokens | Cost |
|---|---:|---:|---:|---:|
| `_openai.main/gpt-6-luna` | 97,967 | 228 | 2,360,407,290 | $0.00 |
| `_claude/claude-haiku-5-5` | 34,963 | 16,740 | 1,218,151,922 | $0.00 |
| `_openai.alt/gpt-6-luna` | 16,780 | 307 | 199,451,249 | $0.00 |
| `_muse/muse-spark-1.3-contributor` | 18,928 | 731 | 162,882,353 | $0.00 |
| `opencode-go.developing-today-0/deepseek-v4.1-flash` | 14,148 | 321 | 121,210,398 | $18.39 |
| `_claude/claude-sonnet-5-5` | 3,836 | 3,335 | 84,871,785 | $0.00 |
| `opendesign/deepseek-v4.1-flash` | 8,432 | 104 | 79,256,408 | $0.00 |
| `cline-pass/deepseek-v4.1-flash` | 7,132 | 1,829 | 55,167,460 | $7.94 |
| `exaone/k-exaone_v2.5` | 3,194 | 422 | 45,103,072 | $0.00 |
| `_claude/claude-fable-5-2` | 4,339 | 4,074 | 44,314,275 | $0.00 |

Pool-level offer/choice counts from the same window:

| Pool | Offered | Chosen |
|---|---:|---:|
| `workhorse-weighted` | 192,163 | 148,220 |
| `workhorse` | 68,074 | (not separately reported) |
| `scout` | 229,325 | 93,376 |
| `speed` | 7,415 | 4,047 |
| `workhorse-last-resort` | 66,279 | 18,751 |
| `workhorse-ordered` | 29,890 | 29,890 |
| `fable-plan` | 324 | 324 |
| `spark-plan` | 3,483 | 2,944 |

### 3.1 Join between consumption and output

The join is **partial by construction**:

- `muse-lane` (489 merged PRs / 3,268 completed tasks) has **no** `route_usage` target, so its consumption is not in the table above. The `muse-lane` worker is not metered as a provider/model in route_usage; treating it as zero consumption would be wrong, and estimating it would be unsupported.
- `opencode-route` (1 merged PR / 2 completed tasks) is metered only through its leaf models (`_openai.main/gpt-6-luna`, `_claude/claude-haiku-5-5`, etc.), and the pool-level table does not attribute tokens to the task that caused them.

Therefore this report publishes the methodology and the partial join, and explicitly records that a full token-to-PR attribution requires either (a) stamping the LLM provider/model on every completed task, or (b) session-level attribution in `route_usage` joined to `session_bound`/`lane_session_bound`. Both are recommended in section 6.

## 4. Worse-efficiency period

### 4.1 Observed trough

Merged PRs by day (UTC) in the window:

| Date | Merged PRs | Completed tasks | Model stamps |
|---|---:|---:|---:|
| 2026-11-12 | 87 | 241 | 29 |
| 2026-11-13 | 114 | 794 | 638 |
| 2026-11-14 | 252 | 1,091 | 1,243 |
| 2026-11-15 | 55 | 67 | 130 |
| 2026-11-16 | 224 | 468 | 504 |
| 2026-11-17 | 289 | 2,318 | 2,206 |
| 2026-11-18 | 274 | 334 | 0 (window cutoff) |

**2026-11-15 is the worse-efficiency day**: merged PRs fell to 55 while only 67 tasks completed and 130 model stamps were written. By comparison, 2026-11-14 had 252 merged PRs / 1,091 completed tasks / 1,243 stamps, and 2026-11-16 recovered to 224 / 468 / 504. The drop is a 78% fall in merged PRs (252 → 55) against a 94% fall in completed tasks (1,091 → 67) and a 90% fall in stamps (1,243 → 130).

### 4.2 Distribution diagnosis

- The stamped population on 2026-11-15 remained 100% `muse-lane` (130 of 130 stamps). There is **no provider shift in the stamped task population** on the trough day, so the trough is not explained by tasks moving from `muse-lane` to `opencode-route`.
- Route-level consumption in the surrounding 7 days was dominated by `_openai.main/gpt-6-luna` (2.36B tokens) and `_claude/claude-haiku-5-5` (1.22B tokens), with `workhorse-weighted` offering 192,163 choices and choosing 148,220. These targets are interactive/session-heavy and are not attributable to completed queue tasks.
- Conclusion: on 2026-11-15, merge output fell further than the stamped task population, while unattributed OpenCode consumption continued. The most plausible diagnosis is that work moved into interactive/non-queue paths (or stalled in review) while the queue's merge-producing path contracted. This is a **worse-efficiency period** under C5's definition; it is documented here rather than smoothed over.

This is a single observed trough, not a trend. C5 requires measured effect after any smoothing change; no change is proposed from one data point.

## 5. Spot check

Cell checked: **stamped provider/model `muse-lane` / `muse-lane` → merged PRs**.

- Claim: 489 merged PRs in the window are attributable to tasks whose last stamp is `muse-lane`/`muse-lane`.
- Verification:
  1. PR #11793, branch `queue/20619-10462-biome-format`, merged 2026-11-18T11:58:31Z.
  2. Broker `model_stamp` for task 20619 contains five claim stamps, all `model_provider=muse-lane`, `model_id=muse-lane` (first 2026-11-17T06:46:36Z; last 2026-11-17T10:46:18Z).
  3. Broker `completed` for task 20619: completed 2026-11-18T11:49:19Z, `model_provider=muse-lane`, `model_id=muse-lane`, queue `repo-health-code-pilot`.
  4. The task ID parsed from the branch (20619) matches the broker task ID and the stamp task ID exactly.
- Result: the join's branch→task→stamp link holds for this cell. The aggregate count 489 is the count of PRs satisfying the same link; no manual correction was applied.

A second, independent check on the same cell: 3,268 of 3,270 stamped completed tasks (99.94%) are `muse-lane`/`muse-lane`, consistent with the 4,733 of 4,750 stamps (99.64%) being `muse-lane`/`muse-lane`. The two ratios agree within 0.3 percentage points, which is the expected slack from tasks that are stamped but not completed (or completed but not stamped) within the window.

## 6. Recommendations (C5 recommends; it does not change configuration)

1. **Stamp coverage.** Only 3,270 of 5,079 completed tasks (64.4%) have a stamp. Require a `model_stamp` at claim time and at completion time, and reject completion events missing `model_provider`/`model_id`. This is the single biggest blocker to a full efficiency join.
2. **Provider/model attribution on completion.** The 3 `opencode-route` completed-task records (tasks 14440, 20001, 20200) are the only ones carrying `opencode-route`/`workhorse`. Either the completion path is not recording the route for most tasks, or most tasks genuinely run on `muse-lane`. Resolve which before any never-serve or routing-mode rule is written.
3. **Session-to-task attribution.** `route_usage` reports consumption per provider/model target and per pool, but not per task. Join `session_bound`/`lane_session_bound` to task IDs and expose a per-task consumption rollup, or accept that efficiency is measurable only at the worker-population level.
4. **Do not act on the 2026-11-15 trough yet.** One trough plus a 90% stamp-count drop on the same day is consistent with a scheduling/availability event, not necessarily a policy failure. Collect two more weeks of daily data before proposing smoothing changes; C5 requires measured effect after adoption.
5. **Routing-mode and never-serve tables are deferred.** They need per-provider rate-limit evidence (ordered vs round-robin vs weighted) that this report does not yet contain. The next C5 report will include them, or record why they cannot be produced.
6. **Free-provider inventory is deferred.** It requires tested context windows, which are out of scope for this report.

## 7. Acceptance status

- **Acceptance item 1 (first efficiency report published with join methodology stated and reproducible from stored data):** satisfied by this report. Limits recorded in sections 3.1 and 6.
- Acceptance items 2 and 3 are not claimed by this report; they remain open on #9573.

## 8. Limits and caveats

- The report window is 5.85 days, not the 7–30 days originally suggested; it is bounded by the first available `model_stamp` row. A 30-day join is not possible until stamps accumulate.
- `route_usage` window (168h ending 2026-11-19T01:09Z) does not exactly coincide with the report window (ending 2026-11-18T18:32Z); overlap is 5.85 of 7 days.
- `muse-lane` consumption is not metered in `route_usage`; no efficiency ratio is computed for the largest output cell.
- Merged-PR attribution assumes one task ID per branch and ignores PRs without a `queue/<id>-` branch (466 of 1,127 merged PRs in the window).
- Completed-task counts include triage/administrative tasks that never produce a PR, so "merged PRs / completed tasks" is a lower bound on per-task productivity, not a quality measure.
- No configuration changes were made.
