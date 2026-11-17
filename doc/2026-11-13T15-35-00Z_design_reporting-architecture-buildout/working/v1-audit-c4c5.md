# V1 baseline audit for C4 (quota forecasting) + C5 (efficiency + provider/model policy)

Scope anchors (read fresh 2026-11-13): code#9572 (C4), code#9573 (C5), under starter epic #9568.
Program intake: `doc/2026-11-13T15-12-00Z_design_reporting-architecture-event-driven/` (note.md + sources/).
Method: grep + read only. No gatherer or live-provider command was run. All paths relative to repo root
(`developing-today/code`, branch `design/reporting-architecture-buildout`) unless marked LAPTOP or MUSE-HOST.

---

## 1. Quota data substrate — what exists today

### 1a. Laptop quota gatherer (the `quota-current.json` producer)

| Asset | Path | Cadence | Schema / content | Owner job |
|---|---|---|---|---|
| Gatherer script | `pm-stack/aggregator/scripts/gather-quota.ts` (header comment lines 1–120 is the contract) | 15-min tick | Single source: `opencode quota --json` (listing) + explicit ask `opencode quota --json meta command-code`. Fail-soft: unparseable stdout fails the tick WITHOUT touching `quota-current.json`; single-provider failure degrades only its rows; errored provider keeps last-known-good rows from prior tick (#1268, `retainedFromFailedTick`) | systemd user timer on laptop |
| Timer + service | `pm-stack/aggregator/systemd/quota-gatherer.timer` (`OnCalendar=*:0/15`, `RandomizedDelaySec=20`, `Persistent=true`) + `quota-gatherer.service` (oneshot, `TimeoutStartSec=300`, runs `bun run …/scripts/gather-quota.ts` from laptop checkout `~/git/code/pm-stack/aggregator`) | `*:0/15` | Env-pinned outputs: `QUOTA_CURRENT_PATH`, `QUOTA_HEARTBEAT_PATH`, `QUOTA_GATHERER_STATE_PATH`, `QUOTA_SNAPSHOTS_DIR` → all under LAPTOP `~/.local/state/pm-aggregator/` | Installed by `pm-stack/aggregator/install.sh:236-249` |
| `quota-current.json` | LAPTOP `~/.local/state/pm-aggregator/quota-current.json` (NOT in repo; no sample in repo) | Rewritten each tick | `QuotaCurrentFile` (`pm-stack/aggregator/src/model.ts:1565-1574`): `version: 1`, `generatedAt`, `source` (=`"opencode-quota"`, `GATHERER_SOURCE`), `heartbeat: QuotaGatherHeartbeat` (tickAt, ok, durationMs, perProvider ms/rows/error — `model.ts:1556-1563`), `providers: QuotaCurrentEntry[]`, optional `providerAuth`, optional `sanitized`. Entry (`model.ts:1519-1555`): providerId/Name, serviceId (pool id), credentialId/Label/Watch/ExpiresAt/Source, planLabel, failureClass, lastSuccessAt, state ok/error/lapsed/unconfigured, measuredAt, retainedFromFailedTick, source, error, `windows: QuotaCurrentWindow[]` (name, value, pctOfLimit, valueLabel, windowSeconds, resetsAt, measuredAt, source, state), raw `usage` | gatherer |
| Snapshot series | LAPTOP `~/.local/state/pm-aggregator/snapshots/usage-YYYY-MM.jsonl` (burn plan cites `snapshots/usage-2026-10.jsonl`, 599 rows, 2026-10-25→10-29) + `SnapshotRecord` (`model.ts:~1576+`): ts, serviceId, metricId, value, unit (`pct|usd|cny|count|gb|tokens|usd-est|minutes|hours|gb-hours`; cny first-class, never FX-converted), limit?, resetsAt?, source, observedBy | Appended per tick for windows whose value moved (gather-quota.ts §4) | gatherer |
| Pool mapping | `pm-stack/aggregator/src/quota-gather-map.ts` (quota-module result → hub contract) + `POOL_SPECS` (`pm-stack/aggregator/src/quota-extract.ts:541-656`): **12 pools** — `meta` (Meta Model API), `anthropic:claude-pro`, `_openai.main`, `_openai.alt` (never combined), `opencode-go.drewrypope`, `opencode-go.popesdrew`, `opencode-go.developing-today-0`, `cline-pass`, `_commandcode`, `_antigravity.gemini`, `_antigravity.claude-gpt`, `opendesign`, `deepseek` (PAYG, parked ¥44.63, never a fallback). Pool id = OpenCode config provider id (2026-11-03 alignment) | — | Per-pool extractors over dated reads (`quota-extract.ts:1-19`); disabled-door filter removed — pool assembly no longer consults `disabled_providers` (`quota-extract.ts:658-681`) | aggregator |
| Auth keepers (credential, not readings) | `src/opendesign-renewal.ts` + `src/meta-renewal.ts` in aggregator; state files `opendesign-renewal.json`, `meta-credential.json`, `meta-renewal.json` beside `quota-current.json`; Meta keeper dispatches Web Push on renewal failure from first bad tick | per-tick + on-auth-failure | Embedded into `quota-current.json.providerAuth` (#6671) | gatherer |
| Meta exporter | Meta file-export probe (`#1301`, same 15-min cadence family); read only for last-good continuity | 15 min | — | exporter unit |

Grep anchors: `quota-current` ×30 (gatherer, SHARED-SCHEMA-MAP.md:35, design-register-2026-11-11.md:695-700, burn plan); `GATHERER_SOURCE` in `gather-quota.ts:157+`.

### 1b. Burn plan ("plan-current.md, v46+")

- **The living doc exists:** `doc/living/quota-full-utilization-burn-plan.md` (committed 2026-11-11 from
  `~/workspace/research_notes/quota-burn-plan-20261029/plan.md`; status: shipped; tracking home for completed
  design F4.5 / epic code#8805). First edition built 2026-10-29 ~07:35Z from gatherer series + `quota-current.json`
  + Supabase `agent.quota_records` + `agent.mirror_token_usage_daily` + provider-quota-registry.md + dated console reads.
- **Shape of burn entries (§§1–3):** per pool (largest window): now %, reset instant, 24h + 72h + post-reset-segment
  burn rates (Δpts/day), required rate to land at 100%, projected % at reset / projected exhaustion date, scenario
  branches where unstable; then the **cascade pass** (pools ordered by projected exhaustion, fall-through per route
  ladder, absorber recomputation, two resets deep); dated audibles A1–A5; target-trajectory table (§3).
  This is the closest thing to C4's "recorded bases" — but it is a **hand-written, single-edition analysis**,
  not a logged/refreshing record stream.
- **The standing-plan mechanism (§5):** vehicle = evolve `go-plan-expiry-watch` (6h) into the full-quota check;
  inputs = `quota-current.json` + snapshot tail (≤60 min gate) + `agent.quota_records` + **"the previous standing
  plan (this document's successor, kept as a versioned file the job rewrites)"**; outputs = regenerated standing
  plan + per-pool one-liners; only 3 things escalate (over-pace flag, unmeasurable pool, route-config change).
- **CRITICAL — `plan-current.md` (any version) does NOT exist in this checkout.** `grep -rn "plan-current"` = zero
  hits repo-wide; `grep "plan_current|burn-plan|burn_plan"` = only the burn-plan doc itself + register refs.
  No v46+ artifact, no versioned standing-file series, no writer/reader code references it. If a `plan-current.md`
  with 46+ versions exists, it lives on the MUSE-HOST (job state) or in the Fleet Reports Space — unverifiable
  from here. **C4 designs must not cite `plan-current.md` as a readable V1 schema; cite the burn-plan doc's §1–§3
  shapes instead, and mark the versioned-file claim UNVERIFIED.**
- Addenda (2026-10-29): Command Code auth restore (401 root cause + CLI re-login fix; weekly 13.996%, resets
  2026-10-31T15:16:57Z — note: NO monthly-percent window on that endpoint); Antigravity feed repair
  (openchamber#764 store-auth fix; Gemini 5% / Claude+GPT 57%).

### 1c. Turso quota records

- Table `agent_quota_records` on Turso `fleet-platform`
  (`https://fleet-platform-dezren39.aws-us-east-1.turso.io`), written via libsql HTTP pipeline.
  Code: `pm-stack/aggregator/scripts/gather-quota.ts:691-763` (`writeTursoRecords`).
- Schema (from the INSERT, line 736): `(id, source, ts, plan, weekly_percent, resets, payload, inserted_at)`;
  PK `(source, id, ts)`, `INSERT OR IGNORE`; `id = ${serviceId}:${tickAt}`; `payload = {id, ts, plan, state,
  source, windows: [{name, percent, resetsAt}], provider}`; `weekly_percent` + `resets` are the weekly-window
  conveniences, full fidelity in `payload`.
- Cadence: one row per pool entry with ≥1 window, per gatherer tick (~15 min), `source="opencode-quota"`.
  Fail-soft after `quota-current.json` is written; `TURSO_QUOTA_SINK=0` disables.
  Token resolved by name: `turso.fleet-platform-rw-token` (Bitwarden Secrets Manager).
- History: wrote to Supabase `agent.quota_records` (muse-dev2) via Management API until the 2026-11-06 Wave 3
  cutover (code#5690); retired "pm-pool" writer died 2026-10-29. Atlas scaler doc corroborates the old shape
  (append-only, jsonb payload, insert-only).
- Readers in-repo: `decision-events-sync.ts:15` (target DB note), burn plan §5 (subscription line), aggregator
  README:797.

### 1d. Fleet Reports Space streams (17 streams)

- What the repo says (exhaustive — only 3 mentions, no implementation):
  `note.md:138` ("17 streams, recordReport + fallback JSONL"), `audit-side-chats.md:20-21,27-28,37-38`
  (2026-11-07 rebuild: 17 rebuilt streams write full reports to the Space, exceptions to Reports chat;
  per-job exception conditions; Space remains the record).
- The 12 rebuilt jobs named (`audit-side-chats.md:167-176`): usage-quota-report, pool-quota-steward,
  pool-intelligence, pool-composition-review, pool-meta-reviewer, fleet-watchdog, git-hygiene-sweep,
  stewardship-cycle, go-plan-expiry-watch, coding-plan-watch-daily, hatch-untracked-review, daily-token-usage-checkin.
- **Stream NAMES are not in this repo.** `grep recordReport|record_report` = only the two audit mentions;
  `grep fleet-reports|FleetReports` = zero. No stream catalog, no schema, no fallback-JSONL path in-repo.
  Space-owner chats (correct pattern per audit:178-190): Fleet Reports `fcf52a31`, Projects Overview `3d1d2f4a`,
  AI Benchmarks `d30275f6`, OpenCode & Muse Activity `785419b6`, AI Plans & Usage `f2178de6`, ID & Atlas `f707171a`.
- → UNKNOWN (Muse-side artifact; enumerate from Space/chats, not from code). Quota-related streams are
  *asserted* (usage-quota full detail, daily-token-usage deviation alerts per audit:129) but not enumerable here.

### 1e. `usage-quota-report` job

- Hourly (audit-side-chats.md:122: "usage-quota-report (hourly)"; detail notes → category #5 if adopted:236).
  Delivery today: full detail → Fleet Reports Space; exceptions → Reports `2657aa66`; originator chat retires.
- **NOT in `doc/fleet-schedule-registry.md`** (grep `usage-quota|page-health` = zero rows; registry verified
  2026-11-03 from `cron.list` + laptop timers). Definition + body live on MUSE-HOST Muse scheduler;
  no in-repo copy. Schedule/cadence beyond "hourly" and exact exception conditions → UNKNOWN from here.

---

## 2. Existing forecasting — capability vs gap

### 2a. Design docs (read in full)

| Doc | Status | Relevance |
|---|---|---|
| `doc/2026-11-01T21-07-00Z_design_pm-stack_forecasting/2026-11-01T21-07-00Z_design_pm-stack_forecasting.md` | Proposed; impl issues #3981–#3984 + #3986 (backtest) as children of #1256 under epic #1184 | The honest-forecasting doctrine: 4-complete-week minimum, count-based throughput, "no measurable pace" states, P50/P80 ranges, rolling-origin backtests, ≥30 resolved forecasts / 8 cutoffs before "calibrated", P80 coverage 70–90%. Quota slice = time-to-exhaustion interval on #3933 live rates, never a second threshold system |
| `doc/2026-11-13T09-26-00Z_design_pool_effort_system/` (`brief.md`, `brief-decisions-supplement.md`, `brief-decisions-amendment-2.md`, `brief-capabilities-supplement.md`) | Drew briefs routed through design mill (code#9399) | C5 substrate + decision models (Clef/Jev System-1, decision pools free→plan→PAYG, decision record schema with prompt version/tokens/percentages/TTFT/outcome gauges, per-facet effort stances). V1 inputs §9 is the best "state of routing 2026-11-13" snapshot |
| `doc/2026-11-13T10-15-00Z_design_scaling_technique_comparisons/…md` | Decided | **Not applicable to C4/C5**: server scaling arms (workers vs sharded DBs). No quota content; cite only to exclude |

### 2b. Live forecasting derivations (all in `pm-stack/aggregator/src/`, collector `scripts/collect-fleet-feed.ts`)

| Capability | Where | What it computes |
|---|---|---|
| Linear pace projection | `derive.ts:1293+`; attention flags `derive.ts:980-1001` (`BUBBLE_PCT=80`, `BUBBLE_USD=5`) | Per-metric linear pace from snapshot history; over-limit bubbles worst-first |
| Measured burn history | `burn-history.ts:1-80` (+ `deriveMeasuredRatesFromPoolTokens`): `PoolWindowObservation{pool,provider,window,unit,used,resetsAt,at}`; `BurnSegment[]` (completed reset periods only); `burnTokensPerMin` measured from two most-recent same-period observations ≤ `RATE_WINDOW_MS` (2h); retention 35d, ≤500 obs/window. Honest-absence: pct/usd windows yield no token rate; the one real token stream is OpenCode session attribution | Per-pool exhaustion intervals (#3983) |
| Exhaustion forecast | `quota-exhaustion.ts:35-160`: states `forecast` (P50/P80 nearest-rank over same-period pair rates, `MIN_RATE_SAMPLES=2`, `p50VsReset`/`p80VsReset`) / `exhausted` / `unknown` (reasons: `missing_limit`, `stale_source`, `reset_passed`, `no_measured_burn`, `insufficient_observations`, `zero_rate`, `missing_reset`) | Served on the pool object; `/api/forecasts.json` (`server.ts:2607`) via `forecast-contract.ts` (#3984, 392 lines) |
| Quota analytics + projections | `quota-analytics.ts:1-60` (Drew 2026-11-13 order): `PctPoint`, `ProjectionState` (`walled|projected|no-wall-before-reset|insufficient-history`), `WindowProjection{ratePtsPerHour` (least-squares slope), samples, projectedWallAt, durationUntilWallMs, wallsBeforeReset, reason}; `PROJECTION_MIN_SPAN_MS=10min`; page `/quota#analytics` + `/api/quota-analytics.json`; store `analytics-store.ts`, view `quota-analytics-view.ts` | Digest outlook |
| Digest wall prediction | `digest.ts:254-380` (`quotaPoolNote`): walled/near-wall/degraded states + "at N tok/min the window fills in ~Hh" for tokens-unit windows only | Collector wiring |
| Feed collector | `scripts/collect-fleet-feed.ts:1230-1260`: derives burn + exhaustion per collection run, rides on pool like `burnTokensPerMin`; burn-history state beside charges state | Item history |
| Item/closure history (backtest input) | `item-history.ts` (332 lines; line 39: windows/backtests need inputs); `flow-analytics.ts:8,23,180` (forecast subject model = grouping) | — |

### 2c. What does NOT exist (explicit negative results)

Searched `pm-stack/aggregator/{src,scripts}` for `issuedAt|methodology|prediction record|prediction log`
→ **zero hits**. Searched `day-type|day_type|spike|weekend|Friday|trailing-30|trailing_30|downtime|complete-downtime`
→ **zero quota hits** (only: a DeepSeek peak-*pricing* window comment and a render.ts chart caption).
Therefore, **none of the following exist in code today**:
1. prediction record (basis, inputs, range, issued-at) — no log, no schema;
2. scoring loop (predictions scored against outcomes) — no code; closest is the *designed but unbuilt* #3986 backtest;
3. methodology-change log;
4. day-type adjustments (same time-of-day, other Fridays, weekends) — time-of-day routing is only *ticketed* (code#9378; DeepSeek peak windows exist as cost gates, not forecast features);
5. spike-day modeling; trailing-30-day recorded bases (retention is 35d of raw obs, but no 30d-rate basis is computed);
6. complete-downtime / all-pools-walled-at-once prediction; new-model/plan arrival prediction;
7. reset-duration ranges in Drew's formats ("X–Y hours", "2.5–3.7 days") — outputs are instants (`projectedWallAt`, `p50ExhaustsAt`) or booleans, never ranges;
8. wall-until predictions (reopening instants are recorded per-window as `resetsAt`/`wallLiftsAt`, not predicted).
Partial credit only: `quota-exhaustion` unknown-reasons + P50/P80 intervals + digest degraded-state notes are the
honest-absence half of C4; the burn plan doc is the recorded-basis half, hand-authored once.

---

## 3. Efficiency / routing substrate

### 3a. Pool stewardship jobs (all MUSE-HOST Muse crons; bodies NOT in repo — paths from registry, verified 2026-11-03)

| Job | Cadence | Registry row (what it measures) | Writes to | Exception condition → Reports |
|---|---|---|---|---|
| `pool-quota-steward` | `interval@1h` | Hourly pool balance, change ledger, impact follow-up. Body: `/home/hatch/workspace/goals/keep-the-four-repos-healthy/crons/hourly/pool-quota-steward__interval@1h.md` | Space (full) + Reports (exceptions); origin chat `38ee620e` retires | stewardship deltas (per 2026-11-07 ruling; exact predicate in job body — UNKNOWN here) |
| `pool-composition-review` | `interval@12h` | Pool membership + right-sizing audit. Body: `…/crons/hourly/pool-composition-review__interval@12h.md` | Space + Reports | **recommendations** (audit:124) |
| `pool-intelligence` | `daily@08:12:00` | Daily AA monitoring, measured session speeds, add/retire flags. Body: `…/crons/daily/pool-intelligence__daily@08:12:00.md` | Space + Reports; proposed category-#5 home chat `d92d987b` | add/retire flags (implied) |
| `pool-meta-reviewer` | `daily@07:12:00` | Daily grading of reviewers, steward audit, guideline updates. Body: `…/crons/daily/pool-meta-reviewer__daily@07:12:00.md` | Space + Reports | **high+ findings** (audit:126) |
| `go-plan-expiry-watch` | `interval@6h` | "Full-quota check — burn plan, audibles, Go lapse watch". Body: `/home/hatch/workspace/goals/openchamber-session-review-oct-17-24-resume-unresolved/crons/hourly/go-plan-expiry-watch__interval@6h.md` | Space + Reports | inside-14-days alerts (audit:127) |
| `coding-plan-watch-daily` | `daily@08:00:00` | Coding AI plan watch | Space + Reports | changes (audit:128) |
| `daily-token-usage-checkin` | `daily@09:12:00` | Daily token usage | Space stream + Reports | deviation alerts (audit:129) |
| `weekly-token-usage-review` | `weekly@Sat-09:12:00` | Weekly token usage review | Space + Reports | — |

Registry: `doc/fleet-schedule-registry.md:94,99,134,168-171,211`. "Budget expectation / kill criteria" columns are
`not recorded` for all of these — C5 inherits that gap. Design-register corroboration:
`pm-stack/design-register-2026-11-11.md:482-484` (quota steward hourly, meta-reviewer + pool-intelligence daily
shipped under the four-repos-healthy goal).

### 3b. Routing-mode policy knobs (config paths + current values)

- **Single source:** `.opencode/opencode.jsonc` `experimental.provider_routes` + `experimental.model_routes`
  + class routes; semantics `.opencode/route-fallbacks.md` (Drew 2026-11-13; guard
  `bin/check-opencode-route-rules.mjs` `CLASS_FALLBACKS`).
- **Available modes in-config:** `ordered`, `round-robin`, `weighted` (`selection` key). Under fork round-robin
  (#1122) a member's `weight` is a **priority tier**, not a traffic share (deepseek-flash-plan comment block).
- **Current values (per-plan pools):** openai-plan round-robin (`_openai.alt`, `_openai.main`;
  ordered alt-first is Drew's *planned later step* — jsonc:2084-2116); mistral-plan ordered (`_vibe`, `mistral`;
  jsonc:2118-2129); opencode-go-plans weighted 1/1/1 (three Go workspaces; jsonc:2131-2154);
  deepseek-flash-plan round-robin with tier weights (Go 2 / ClinePass 1 / OpenDesign 1 / mistral-plan 1;
  CommandCode REMOVED 2026-11-13 per code#9397 — credits exhausted $9.95/$10, last success 2026-11-10);
  deepseek-pro-plan round-robin (Go weight 3); haiku-plan ordered (`_claude` only since 2026-11-11);
  fable-plan ordered single-door; sonnet-plan round-robin (`_claude` → Antigravity); opus-plan ordered;
  spark-plan ordered (Go → CommandCode → ClinePass → `_muse`); workhorse-weighted = 4×w10 round-robin trial seats.
- **Class routes:** small→(limited)→speed; speed ordered (Spark xhigh → flash-plan#high → hydrogen#high →
  haiku-plan#high) →workhorse→summit→specialist; workhorse stages ordered/weighted/last-resort →scout→specialist
  (scout edge = PROPOSAL, Drew unruled); scout ordered (beam-plan → flash#low → haiku#low) →workhorse (PROPOSAL);
  background (MiMo/DeepSeek peak-aware) →workhorse (PROPOSAL); specialist = SINK (no fallback; errors when all down).
- **No measured-rate-limit analysis behind any mode choice in-repo.** Mode changes are dated Drew direct orders
  with burn/pace justifications in comments (e.g. luna-plan burn-by-expiry weights; 2:1→1:1 Go re-weights
  2026-11-03/11-07/11-08), plus standing trials with review hooks (deepseek-effort-rr #9381: `#high` w3/#low w1 in
  workhorse-ordered; luna-effort-rr #9018 round #medium/#high). Rate-limit-measured evidence per provider → gap.
- **Never-serve rules: no table.** Nearest V1 facts: `disabled_providers` switchboard (jsonc:210-224: google,
  google-vertex, cloudflare-ai-gateway, groq, nvidia, vercel, huggingface, opencode, github-copilot, openrouter,
  cloudflare-workers-ai, meta, deepseek — some tombstoned); NEVER FUNDS rule (Drew 2026-10-25: supervised sessions
  on plan quotas only; PAYG tail removed from deepseek-flash, jsonc:~2328); "haiku-plan leg last in light tiers"
  fleet rule; dated seat removals (CommandCode #9397; deepseek API leg #6414; opendesign Flash leg cleanup).
  The "never use provider X for Sonnet" policy is directive-only (source-brief.md:100-102).

### 3c. Free-provider inventory + wiring (C5 §"free")

- **Wired free/cheap seats today:** `beam-plan` (CommandCode `beam:free` → OpenCode Go `beam-free`, round-robin;
  scout's head; route-fallbacks.md); `mimo-flash-plan` (Go → ClinePass → OpenDesign → CommandCode; background +
  limited members, peak-window aware); openrouter Free Tier (models `minimax-m3.1-flash:free` 204.8k ctx,
  `poolside/laguna-s-2.1:free` 131k ctx — jsonc:911-930) is **DISABLED** (`:free` models retired 2026-10-22);
  CommandCode proxy free routes (scout comment: ~21k-token injected agent prompt/call; rate limits unmeasured).
- **Free decision models in front of workhorse: designed, not wired.** Decision pools (free → plan → PAYG),
  Cloudflare Workers $10-included tracking, "cheap LLM (DeepSeek Flash?) vs native decision model" cost question,
  Jev multi-source correction — all in `brief-decisions-supplement.md` + `amendment-2` (code#9399 mill, design stage).
  Agent pins exist (classifier→scout precedent, pool-effort brief §9). No config `decision_pool` default verified
  in `opencode.jsonc` (grep `decision` = only comments/records, no pool wiring).
- **Tested-vs-official context: NO structured inventory.** V1 facts are scattered: per-target `limit.context` caps in
  `limited` (Luna 100k plan cliff vs 1.05M native; Terra 388k policy ceiling vs 1.05M; Haiku 388k vs 1M;
  jsonc:3597-3671); beam scout comment (free-tier limits "unmeasured"); AA v5.0.1 timed measurements
  (`~/workspace/model-review-2026-11-12/aa-results.md/.json` — off-host, cited in pool-effort brief §9).
  The C5 field itself ("tested vs official, usually smaller, sometimes larger") is directive text
  (source-brief.md:106-108). Reliability / rate-ceiling tracking per free provider → gap.
- **Fall prevention (mechanism, cf. #9196 + steward flip machinery):** route-fallbacks.md semantics (100%-walled
  leg never attempted; per-call visited set; specialist sink) + fork route expander + `check-opencode-route-rules`
  guard + quota-demotion of walled members inside *-plan groups. Demonstrated-evidence of a fall (not a stop)
  on quota/rate-limit → not verified from repo; C5 must produce the demo (#9573 acceptance).

### 3d. Plan & model recommendations (in-repo state of knowledge)

- CommandCode: $1 Go tier / ~$10 credits/mo; credits exhausted 2026-11-13 ($9.95/$10, no reset; seat removed #9397).
  No "CommandCode full plan" record — directive-only (S2.11, source-brief.md:49-50).
- Claude: free tier; `claude-code` provider disabled 2026-10-27 → re-enabled 2026-11-06 (Drew); haiku-plan is now
  `_claude`-only. No "Claude again" capacity position quantified beyond burn-plan §4 ("future cascade position").
- OpenRouter: free tier, disabled (above). No "another OpenCode plan" record — directive-only.
- GLM: models wired (`zai-glm-5-3`, `glm-5.3-flash`, `cline-pass/glm-5.3`, `glm-5-3-plan` via `_vibe`), AA v5
  scores cited (GLM max II 32). No "GLM plan" recommendation record — directive-only.
- Efficiency accounting (quota→output join): **does not exist.** `grep efficiency in aggregator/src` = only
  `pages.ts:158` (Cost tab: "plan/API/per-task efficiency" display label) + AA Explorer per-task costs.
  Data halves exist separately: quota/token inputs (gatherer, snapshots, `mirror_token_usage_daily`,
  session attribution `tokensBySession`) and output halves (merged PRs via projector/GitHub, claim-stamped task
  outcomes per code#8176 fix, Roadmap & Stats Space daily pipeline `~/workspace/goals/…/roadmap-stats/`).
  No methodology joining them; no "worse efficiency" detection (the S3.16 genre example — Muse→OpenCode token
  shift with PR decline — is quoted verbatim in source-brief.md:96-99 as a *desired* diagnostic, not a live one).

---

## 4. Push notifications today

| Mechanism | Path / locus | Push targets | Mobile reachability |
|---|---|---|---|
| Aggregator Web Push | `pm-stack/aggregator/src/push.ts:1-60` (RFC 8030 + VAPID via `web-push`; public key hardcoded; private key `pm-aggregator.vapid-private-key` from BWS, memory-only; outbound-only POSTs) | Push subscriptions in Supabase `muse-dev2` ledger (`push_subscriptions`) | PWA design `doc/2026-11-01T03-54-00Z_design_pwa_push_notifications.md`; subscription count / device reach NOT in repo → UNKNOWN |
| Alert throat | `pm-stack/aggregator/src/alerts.ts:1-60` (design #3450, amended 2026-11-01; ONE intake; dedupe/delivery/escalation/ack; ledger `alerts`: incidents/incident_events/push_subscriptions/notification_sends; **incident class breaks quiet hours**; warnings page+ledger only) + `notifications.ts:47-60` classes (`incident, needs-drew, quota-wall, quota-near-wall, section-risk, stack-health, blocked-age`) + `/api/alerts` bearer intake (`server.ts:605+`) + ack-attributed sent-vs-acted roll-up (`notifications.ts:1-34`) | subscribed devices per class (`/alerts/preferences`) | Same UNKNOWN as above |
| `page-health-push` (Muse cron, 5-min) | MUSE-HOST; in-repo refs only: audit-side-chats.md:38-40 ("already pushes PM-page down/recovery + new waiting-on-Drew items to the **main chat** — working precedent for high-urgency"), :60, :77 (keeps main-chat push, adds log → category #4), :231-232; `note.md:33,139` | **Main chat** (chat, not device push) | N/A (chat surface); body/schedule source = Muse scheduler, NOT in `fleet-schedule-registry.md` (zero rows) |
| questions module | `pending-questions-hourly`, `decision-set-activity-processor` (category #4 cluster) | category chat + reports surfaces | — |

C4's "forecast alerts go out as push notifications" has TWO V1 precedents to build on (aggregator `quota-wall` /
`quota-near-wall` classes + page-health-push main-chat urgency path) but no forecasting sender exists.

---

## 5. Explicit gap list (C4/C5 delta from V1)

C4 (→ #9572): no prediction record / scoring / methodology log; no day-type or spike adjustments; no trailing-30d
bases (35d raw retention exists, unused as a basis); no downtime-window or new-model predictions; no Drew-format
ranges; `plan-current.md` unlocatable (do not spec against it — spec the record schema fresh, backfill from the
burn-plan doc shapes + `quota-exhaustion`/`quota-analytics` types). Live rates to compose from: `burnTokensPerMin`,
`ExhaustionForecast`, `WindowProjection`, snapshot JSONL, Turso `agent_quota_records`.
C5 (→ #9573): no efficiency join (halves exist, methodology doesn't); no routing-mode evidence table (modes +
dated orders exist, measured-rate-limit analysis doesn't); no never-serve table (switchboard + NEVER FUNDS +
dated removals exist); no free-provider inventory with tested context windows (wired seats + scattered caps +
AA data exist); no wired free-decision-model-in-front-of-workhorse with demonstrated fall prevention (mechanism
+ guard exist); no costed plan/model recommendations (account states exist).

## 6. UNKNOWNs with where I looked

1. `plan-current.md` content/versioning — `grep -rn plan-current` repo-wide = ∅; §5 of burn plan asserts a
   versioned standing file the watch rewrites (MUSE-HOST or Space). Check: go-plan-expiry-watch state on host.
2. Fleet Reports Space 17 stream names/schemas + fallback-JSONL path — `grep recordReport|fleet-reports` = audit
   mentions only. Check: Space `fcf52a31` / owner chats.
3. `usage-quota-report` + `page-health-push` job bodies, exact cadences, exception predicates — absent from
   `doc/fleet-schedule-registry.md` (verified 2026-11-03) and repo. Check: MUSE-HOST `cron.list` / job files.
4. Pool job internals (change-ledger schema, composition right-sizing rules, meta-reviewer guidelines) — registry
   has one-line descriptions + host paths only. Check: the `…/crons/…/*.md` bodies on host.
5. Snapshot/Turso history depth actually retained on laptop/host (35d/500-obs caps are code; disk truth unknown);
   `quota-current.json` live sample (laptop-only). Check: `~/.local/state/pm-aggregator/` on laptop.
6. AA raw data (`~/workspace/model-review-2026-11-12/`) + Roadmap & Stats Space pipeline + `mirror_token_usage_daily`
   contents — cited, off-host. Needed for C5's join + V1-input claims.
7. Mobile push reach (subscriptions, delivery receipts) — no in-repo source. Check: Supabase `muse-dev2`
   `push_subscriptions` / `notification_sends`.
8. `provider-quota-registry.md` (cited by burn plan §sources as pool map + reset calendar) — `find` repo-wide = ∅;
   likely MUSE-HOST `~/workspace/...`. The in-repo successor is `quota-extract.ts` POOL_SPECS + config comments.
