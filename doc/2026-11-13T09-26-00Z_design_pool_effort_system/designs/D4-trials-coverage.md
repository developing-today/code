# D4 — Trials, dominated-model replacement, variant splits, provider coverage

Program: code#9399 (pool & effort system). Component D4 under `main.md` §3.
Drafted 2026-11-13 by the D4 drafter (workhorse route, directory
`/home/user/git/wt-mill-pools`, branch `design/pool-effort-system`). Status: draft for
the mill; nothing here is applied to config.

Companion artefacts (same program folder):
- `research/prototypes/dominance.py` — stdlib dominance prototype, self-test and run.
- `research/prototypes/d4-measured-14d.json` — the 14-day `route_attempt` snapshot the
  prototype reads (produced read-only by `dominance.py extract`).
- `slices/D4-slices.json` — the issues to file later (not filed).

## 0. Reading rules for every number here

- **Source of measured data:** `~/.local/share/opencode/opencode.db`, table
  `route_attempt`, opened `sqlite3 -readonly` / `mode=ro`. Window: last 14 days to
  2026-11-13 12:15 UTC (626,308 attempts, 2026-10-30 .. 2026-11-13). Window: last 72 h
  = since 2026-11-10 12:04 UTC, the same window `baseline/v1-pools-efforts.md` uses.
- **Provider ids are as recorded in the ledger.** The ledger still carries pre-rename ids
  (`openai`, `_openai`, `openai.alternate`, `opencode-go.personal`, `opencode-go.lab`,
  `vibe-direct`, `muse-direct`, `antigravity-direct`, `claude-code`, `commandcode`).
  They are not normalised in the table. A door alias table is slice D4-S4; until it
  exists, every row below says which id it is.
- **Conditional success** = `successes / (attempts − walled)`, where *walled* =
  `error_tag IN ('QuotaExceeded','Authentication','UnknownProvider')`. Those are
  quota walls, dead credentials and retired ids: the door could not serve the call for a
  reason unrelated to the model. Raw success is also shown where it matters.
- **AA** = Artificial Analysis v5 data from the pm-aggregator copy of the ainventory
  source of record, `~/.local/state/pm-aggregator/aa-last-good.json` (batch
  `fe02783b…`, fingerprint `1ce5b354…`, sections `rankings` (298), `catalog` (233)). The
  workbook and the `~/workspace/model-review-2026-11-12/` files are absent (see
  `baseline/v1-data-sources.md`).
- Statement ids: `inventory/statement-map.json` does **not** exist (the inventory folder
  is empty). Statements are therefore cited by brief location: `B§n.m` = brief §n, m-th
  bullet; `S§n` = supplement; `B9.k` = brief §9 state-of-world bullet k. Themes T7 and T8
  are cited as the inventory names them in `main.md` §3.

## 1. Scope

| Theme | Brief / supplement / main location | Statement ids |
|---|---|---|
| T7 Trials | brief §8 (testing plan), §9 (Luna and DeepSeek trials), AD9 | B§8.1, B§8.2, B§8.3, B§8.4, B§8.5, B§8.6, B9.3, B9.4, AD9 |
| T8 Coverage, variants, extrapolation | brief §6 last bullet (variant splits), §7 (low-tier preferences, real-vs-AA), §8 (DS-Flash policy, Beam, provider use) | B§6.5, B§7.1, B§7.2, B§7.3, B§7.4, B§7.5, B§7.6, B§8.3, B§8.4, B§8.5, B§8.7 |
| Dollar efficiency (cross-cut) | brief §8.4 "best dollar efficiency", §7.6 "cost efficiency is king in workhorse" | B§8.4, B§7.6 |

Explicitly out of D4 (owned elsewhere): effort tier resolution and overlays (D1);
categories and Auto (D2); the stats store schema (D3 — D4 consumes its `dim_model`
alias rules and its export); decision pools (D5); decision records and feedback gauges
(D6). Where D4 needs a D3 field it says so.

## 2. V1 — what exists today (cited, not redesigned)

### 2.1 Trial mechanisms in config

| Trial | Where | What it does today | Source |
|---|---|---|---|
| `luna-effort-rr` (round-robin medium/high) | `.opencode/opencode.jsonc` 2481–2505 | Seats `openai-plan/gpt-6-luna#medium` and `#high` for `workhorse-weighted`. Config says "measurement trial; no review date recorded". | `baseline/v1-pools-efforts.md` a.2, f |
| `deepseek-effort-rr` (weighted high 3 : low 1) | 2970–3003 | Review after 5,000 low draws or 7 days; revert if low success < 90 % of high. | `v1-pools-efforts.md` a.2; config 2992–2997 |
| Haiku #high leads workhorse-last-resort | 3400–3436 | Head swap, PR #9388. No trial record. | brief §9 |
| Explore → scout pin | 1754 | Pin, no trial. | `v1-pools-efforts.md` d |

Neither trial has a machine-readable record, a sample-size target, or a review date in
the file. The trial state lives in comments. The registry in §4 fixes that.

### 2.2 Measured facts that bind D4 (14-day and 72-hour ledger)

**Volume and outcome.** 626,308 attempts in 14 days: 511,016 success (81.6 %), 103,393
failure, 11,733 timeout, 166 interrupted. Route mix: `workhorse` 606,977; `speed`
11,991; `summit` 1,597; `specialist` 1,031; `luna-plan` 1,540; `small` 84; `scout` 59.
In the last 72 h `workhorse` carries 141,874 of 147,000-odd attempts; `speed` 5,315;
`summit` 1,609; `specialist` 140; `small` 84; `scout` 49.

**Error tags (14 d, non-success).** RateLimit 34,966 · Authentication 32,448 ·
QuotaExceeded 29,082 · Transport 12,413 · InvalidRequest 4,690 · ProviderInternal 1,028 ·
UnknownProvider 484. The tag split is the evidence the baseline says `route_usage` could
not give (`v1-pools-efforts.md` e.1). Authentication is credentials, not quota. The
largest single Authentication bucket is an **expired OpenAI token** on `openai.main` for
gpt-6-luna at `xhigh` (18,494 attempts). The second is **missing `api.responses.write`
scope** on `_openai.main` and `_openai.alt` for gpt-6.1-terra (2,811 + 2,760). Claude
OAuth (not signed in on this machine, or refresh rejected with 429) accounts for another
~1,000 at `_claude`, and Antigravity's keyring is unset for ~1,200. These are fixable
faults, not walls: they belong in the dead-door list (§6.6) with a repair owner, not in
the trial arms.

**RateLimit storm.** `_claude` has 19,985 RateLimit errors in the last 72 h (of 28,370
attempts; 19 % success, 5,327 successes). The message is "This request would exceed your
account's rate limit". The standing rule says a 100 %-walled leg is never attempted. A
rate-limited leg is walled for minutes, not days, so it is not in the same class; but the
ledger shows the workhorse route retrying it at a rate that burns the rest of the Claude
window. See slice D4-S15 (cooldown guard).

**Renamed ids still in traffic (14 d, attempts):** `openai` 80,414 (gpt-6-luna `xhigh`
71,456) · `_openai` 17,949 · `openai.alternate` 34,407 · `opencode-go.personal` 18,633 ·
`opencode-go.lab` 253 (0 % success) · `vibe-direct` 17,408 · `muse-direct` 32,849 ·
`antigravity-direct` 12,069 · `claude-code` 4,377 · `commandcode` 900. The current
canonical ids (`_openai.main`, `_openai.alt`, `opencode-go.drewrypope`,
`opencode-go.popesdrew`, `_vibe`, `_muse`, `_antigravity`) carry the 72-hour traffic.
The 14-day numbers mix two id regimes, so every trial reads the 72-hour window for
current state and the 14-day window only for history.

**Door health, last 72 h (canonical ids where the ledger has them).**

| Door (ledger id) | Attempts | Success | Conditional success | Dominant error | Reading |
|---|---:|---:|---:|---|---|
| `_openai.main` (gpt-6-luna high/medium) | 79,876 | 96.2 % | ~99.9 % | Transport/Authentication tail | live, main volume |
| `_openai` (gpt-6.1-terra low; gpt-6-luna) | 17,949 | 96.3 % | ~99.9 % | — | live |
| `opendesign` (deepseek-v4.1-flash high) | 13,329 | 76.5 % | ~96 % | InvalidRequest 2,477 (concurrency 11/10) | **the only live DeepSeek Flash door** |
| `_openai.alt` (gpt-6.1-terra low) | 3,761 | 25.1 % | 25 % | Authentication 2,762 (missing scope) | dead for terra; fix credential |
| `_claude` (sonnet, haiku, fable, opus) | 28,370 | 18.8 % | ~21 % | RateLimit 19,985, Authentication 1,322 | throttled and unsigned |
| `_antigravity` | 475 | 40.8 % | 40.8 % | RateLimit 195 | half-live |
| `claude-code` | 161 | 31.1 % | 31.1 % | Authentication 18 | residue of `_claude` |
| `openai.alternate` | 151 | 64.9 % | 65 % | Authentication 52 | residue (retired id) |
| `opencode-go.drewrypope` / `.popesdrew` / `.developing-today-0` | 628 / 702 / 654 | 0 % | 0 % | QuotaExceeded (Go weekly window) | walled until 2026-11-15 (config 2774–2775) |
| `cline-pass` (deepseek-v4.1-flash; mimo) | 654 | 0 % | 0 % | RateLimit 651 | throttled |
| `_vibe` (deepseek-v4-1-flash-1017) | 882 | 0.1 % | 0.1 % | QuotaExceeded 880 "Vibe budget exhausted" | **walled** |
| `mistral` (API reserve) | 137 | 0 % | 0 % | QuotaExceeded 137 | walled |
| `_muse` (muse-spark) | 256 | 0 % | 0 % | RateLimit 255 "Subscription quota exhausted, resets 2026-11-16" | walled until 11-16 |
| `_commandcode` | 1,381 | 0 % | 0 % | QuotaExceeded 778 | **dead** (Command Code, config 2167) |

**DeepSeek Flash, the one live door.** 14-day Flash rows by door (`high` unless noted):
`opendesign` 21,801 attempts / 86.4 % success; `opencode-go.personal` 18,249 / 97.2 %;
`opencode-go.popesdrew` 8,625 / 82.7 %; `opencode-go.developing-today-0` 6,699 / 83.7 %;
`opencode-go.drewrypope` 5,329 / 78.1 %; `cline-pass` 4,606 / 76.1 %; `_vibe` (1017
snapshot) 22,615 / 48.4 % (11,217 QuotaExceeded). **Peak versus off-peak on the four
Go workspaces and cline-pass** (weekday 01–04 and 06–10 UTC, per `model_route_windows`,
config 3690–3717): success falls from 81–97 % off-peak to 58–65 % at peak on the two
drewrypope/popesdrew workspaces (n = 966–1,095 peak rows), and median TTFT rises from
about 3.0 s to 4.0–4.3 s. The peak penalty is real on this door; it is not a pricing
effect (cost is not the variable that moves).

**Effort rungs are served differently from how they are seated.** In 14 days
gpt-6-luna was served at `xhigh` 274,370 times (91.1 % success), `high` 71,982
(99.6 %), `medium` 4,584 (99.7 %). In the last 72 h it was `high` 71,476 and `medium`
4,583, with **no `xhigh`** on the canonical door. The 14-day `xhigh` mass is the
pre-rename `openai`/`openai.alternate` ids, so it is history. The live `luna-effort-rr`
split is 15.6 : 1 `high` : `medium` in the last 72 h, not the 1 : 1 round-robin the
config states. **The Luna trial is not balanced, and it has been drawing about 6 %
`medium`.** That is a trial defect, not a result. Slice D4-S3.

**DeepSeek effort trial status.** `deepseek-v4.1-flash` at `low` has **36 attempts in
14 days, all on 2026-11-05 and all failed**. In the 72 h since the trial's config date
(2026-11-13) there are **zero `low` draws**, and the Go doors that carried the workhorse
Flash traffic are walled. The trial has not drawn. Its revert condition (low < 90 % of
high) cannot be evaluated until it does. Slice D4-S3 again.

**Beam.** `_commandcode` `reflection/beam:free`: 513 attempts 2026-11-13 08:42–09:35 UTC,
all 403 "The free Beam 1.0 tier has ended". **Beam is still reachable through OpenCode Go:**
`opencode-go.drewrypope|beam-free` 8 attempts (8 successes, 2026-11-04 15:50 – 11-05
03:18) and `opencode-go.popesdrew|beam-free` 1 success (2026-11-04 18:03). No `beam-free`
attempt since; config says the Go door is walled until the 2026-11-15 reset (2774–2775).
So the Go door is a working door with a current quota wall, and it has 9 of 9 successes
in its only test window. It is a fast road candidate after 11-15 (§6.4).

**Dead and unseated seats.** `deepseek-pro-plan` (config 2348) is **defined, referenced by
no route**, and has **zero route attempts in 14 days**. The 2026-10-25 route design doc
(`doc/2026-10-25T21-11-31Z_design_route-pools/`) still seats it on `specialist`, so the
design doc is stale. `glm-5-3-plan` and `glm-5-3-flash-plan` are defined only
(`v1-pools-efforts.md` a.3). `_claude` `claude-sonnet-5-5#medium` and
`_antigravity/claude-sonnet-5-5#medium` seat on `workhorse-weighted` with 13 % and 0 %
success in 72 h.

### 2.3 Dollar data

`cost_usd` is populated for the Go doors, cline-pass, mistral and commandcode. It is
**zero or null for `_claude`** (4,032 of 31,393 Claude attempts have a value, all zero),
because Claude is a subscription and its quota is carried in `quota` as `unified-5h` and
`unified-7d` reset times with no utilisation percentage. So the ledger cannot give
Claude's per-%-dollar cost. The Go `cost_usd` is a proxy (deduction per call), and it
does not agree across workspaces for the same model (popesdrew 6.4 $/1k successes;
personal 1.5 $/1k successes). Treat it as an index, not a bill.

## 3. Options considered

Each major choice lists at least four options with the architecture, scalability,
performance, reliability and extensibility trade-offs, and the prior art it borrows
from (`research/R1-routing-effort-sota.md` sections in brackets).

### 3.1 Trial registry format (where trials live, what they look like)

| Option | Pros | Cons | Prior art |
|---|---|---|---|
| **O1** Keep trials as comments above the route (status quo, Luna and DeepSeek today) | Zero code; already where Drew reads them | Not checkable; the Luna trial shows 15 : 1 imbalance and no one saw it; no review date | — |
| **O2** Structured registry file `.opencode/trials.jsonc`, read by the route guard `bin/check-opencode-route-rules.mjs` | Machine-checkable (arms exist, review date set, revert condition present); lives next to the route config; reviewable in PRs | Registry and config can disagree unless the guard checks both ways | Helm values + schema; R1 §4.5 rule 1 |
| **O3** `trial` table in `opencode.db`, append-only status events | Queryable with the ledger; status history is free | Config and DB can drift; not reviewable in a PR; needs D3 export | AD7 append-only records; R3 Langfuse scores |
| **O4** Markdown registry `doc/fleet-schedule-registry.md`-style | Human-readable | Not enforced; the same failure as O1 | AGENTS.md "fleet schedules" precedent |
| **O5** Trial as a route kind only (`kind: "trial"` on a route, no separate record) | No new file | Measurement fields (margin, n, revert) have nowhere to go; the route already carries weights, which are not the trial | NixOS priorities |

**Chosen: O2 now, with O3 as the V3 event log.** O2 is the smallest change that makes a
trial falsifiable in review. The guard becomes the enforcement point. O3 arrives when
D3's export exists, carrying status transitions as append-only rows that reference the
registry key (AD7 pattern).

### 3.2 Dominance method (how a replacement is proposed)

| Option | Pros | Cons | Prior art |
|---|---|---|---|
| **O1** Pairwise on AA scores only, per tier | Trivial; uses the source of record | Ignores the fleet's measured success and speed; proposes swaps onto dead doors | R1 §4.1 |
| **O2** Pareto on AA (quality ↑, AA cost ↓, AA speed ↑) per tier | Principled; one pass | AA speeds are unusable for DeepSeek (1,486 t/s non-reasoning vs 160–250 measured) | RouterBench hull, R1 §4.1 |
| **O3** Hybrid: AA quality; cost per **completed** task = AA cost ÷ conditional success on the best live door; measured median tokens/s on that door when present; dominators must be on a **live** door (conditional success ≥ 0.5, n ≥ 50) | Uses what the fleet measured; avoids dead doors; cost is dollars per useful result | Quality is still AA's; door rows below n = 50 are ignored | R1 §4.1 "cost per task ÷ success delta" |
| **O4** Bradley–Terry strengths from pairwise judged outputs | Principled per-task strengths with CIs | Needs a pairwise judge the fleet does not have | R1 §4.2 (V4) |
| **O5** Convex hull of cost per AA point (RouterBench AIQ band) | Matches "cost per point bracket" in brief §4 | Hull ignores the speed axis the brief keeps for speed tiers | R1 §4.1, RouterBench |

**Chosen: O3, implemented as a stdlib skyline (§5.3).** Speed is an axis only in the
tiers the brief names (small, speed, scout; B§7.6). O4 is V4; O5 is a report
column, not a decision rule.

### 3.3 DeepSeek Flash policy

| Option | Pros | Cons |
|---|---|---|
| **O1** DS Flash seated in every pool that accepts it (status quo, 7 pools) | Maximum door diversity on paper | On 72 h data it has **one** live door (OpenDesign, 76 %); every other seat is a dead or throttled door and adds failure latency |
| **O2** DS Flash only where it is the door (its pool is its only live leg) + last resort only where Luna is the pool's best-quota door | Concentrates Flash where it earns its keep; the last resort is a named, measured exception | Needs a quota signal to know "best Luna quota" (D3/D6) |
| **O3** DS Flash as last resort in every pool | Never fully dead | Last resorts that fail are the most expensive attempts (60 s timeouts on 7 routes); burns the peak window |
| **O4** Dynamic weight from quota % (route-level `budgets`) | Adapts | `budgets` is unused in config (`v1-pools-efforts.md` a.6); needs D3 quota feed |
| **O5** Remove DS Flash | Simple | Gives up the only cheap live door at peak-free hours |

**Chosen: O2 (§6.3).** The brief's "DS Flash only where it is the door; last resort
elsewhere only where Luna quota is best or DS Flash quota is worst" (B§8.3) is O2.

### 3.4 Variant splits (what becomes a pool)

| Option | Pros | Cons |
|---|---|---|
| **O1** One pool per provider, no variant split (status quo) | Simple | Mixes 1107 and classic, and off-peak and peak, in one weighted pool; the ledger cannot tell them apart by pool |
| **O2** Split a pool into one member per **served** variant × time band, only where the provider serves more than one variant (chosen) | Each member is what the door actually serves; no fake split | Needs the served-variant table (§6.5) and time-window gates |
| **O3** Split every variant of every model into its own pool | Complete | Explodes the route count; most splits have one door |
| **O4** Variant as a weight dimension on one pool | Single pool | Weights cannot express the window gate or the ordering the brief asks for |
| **O5** Variant only in `dim_model` (D3), no pool split | Pure data | Routing cannot act on it; ordering stays wrong |

**Chosen: O2, with D3's `dim_model` carrying the variant identity (AD8).**

### 3.5 Extrapolation from AA

| Option | Pros | Cons |
|---|---|---|
| **O1** Trust AA entirely | Simple | Ignores the Claude per-%-dollar gap and the OpenAI high-effort penalty the brief names (B§7.2) |
| **O2** Hand-set rule table with provenance (`measured`, `extrapolated-from-AA`, `aliased`, `ruled`) (chosen for V2) | Auditable; matches AD3's provenance field | Rules are judgement until calibrated |
| **O3** Per (door, model) multiplicative correction fitted from the ledger | Data-driven | Quota% is missing for Claude; needs ≥ 14 days per cell |
| **O4** Bayesian shrinkage of measured toward an AA prior (Beta on success, log-normal on cost) (chosen for V3) | Handles small n honestly | Needs the stats store (D3) and the prior's variance |
| **O5** Measured only, no AA | Pure | Blind to every model with no door yet |

**Chosen: O2 in V2, O4 in V3.** Rules in §6.6.

### 3.6 Testing plan (which untried models get trialled, in what order)

| Option | Pros | Cons |
|---|---|---|
| **O1** Trial every catalog model | Complete | 146 catalog entries have zero attempts; most are dominated or have no door |
| **O2** Trial only models not dominated by a live seat, ordered by AA points per dollar, door required (chosen) | Spends trial volume where a swap can win | Excludes models whose AA row is wrong (flagged) |
| **O3** Trial in shadow only | No user-visible risk | No data on real quality |
| **O4** Trial on the speed road only | Cheap | Ignores the workhorse dollar objective |
| **O5** Trial by vendor family | Simple | Not an objective |

**Chosen: O2 (§6.2).** Dominated candidates are retired from the list before any arm is
spent on them.

## 4. The trial registry (format and worked records)

### 4.1 Format — `.opencode/trials.jsonc`

Modeled on the Luna round-robin (`luna-effort-rr`, config 2481–2505, PR #9018) and the
DeepSeek effort trial (`deepseek-effort-rr`, config 2970–3003, PR #9381): a named
measurement, a review date, and a revert condition. Every field is required unless noted.

```jsonc
{
  "schema": 1,
  "trials": {
    "deepseek-effort-rr": {
      "name": "DeepSeek Flash - workhorse effort split, high/low 3:1",
      "hypothesis": "low is no worse than high on conditional success and cheaper per completed task on the workhorse-ordered stage",
      "route": "opencode-route/workhorse-ordered",   // where the arms are seated
      "arms": [
        { "id": "high", "pool": "opencode-route/deepseek-flash-plan", "model": "deepseek-v4.1-flash", "effort": "high", "weight": 3 },
        { "id": "low",  "pool": "opencode-route/deepseek-flash-plan", "model": "deepseek-v4.1-flash", "effort": "low",  "weight": 1 }
      ],
      "control": "high",
      "margin": { "kind": "non-inferiority", "metric": "conditional_success", "delta": 0.05,
                  "also": "cost_per_completed_task ratio <= 1.00" },
      "metric": ["conditional_success", "cost_per_completed_task", "ttft_p50_ms", "fallthrough_rate"],
      "sample_target": { "per_arm": 446, "basis": "R1 §4.4.2 NI at delta 0.05, p = 0.90, one-sided alpha 0.05, power 0.80" },
      "start": "2026-11-13",
      "review": "2026-11-20",
      "revert": "low conditional success < 0.90 x high conditional success over the review window, OR fall-through to the next stage rises by more than 2 points",
      "status": "running",
      "status_note": "0 low draws in the 72 h since start (see D4 §2.2): imbalance, not a result",
      "config_line": 2970
    }
  }
}
```

Field list and meaning:

| Field | Type | Meaning |
|---|---|---|
| `name` | string | Human title |
| `hypothesis` | string | One sentence, falsifiable |
| `route` | `opencode-route/*` | Route where arms are seated |
| `arms[]` | `{id, pool, model, effort, weight?}` | Each arm names a pool member and its effort. `effort` is the canonical tier from D1 once it exists; until then the native variant string |
| `control` | arm id | The arm the others are compared against |
| `margin` | `{kind, metric, delta, also?}` | Non-inferiority margin on the named metric; `also` is a cost guard |
| `metric[]` | strings | Named measurements; the first is primary |
| `sample_target` | `{per_arm, basis}` | Minimum attempts per arm before a verdict; `basis` cites the formula |
| `start`, `review` | ISO dates | Review date is mandatory |
| `revert` | string | Condition that reverts the arms to control, stated before the data |
| `status` | enum | `proposed` · `running` · `review-due` · `adopted` · `reverted` (`parked` / `cancelled` are Drew's alone, standing rule) |
| `status_note` | string | Latest factual observation, dated |
| `config_line` | int | Where the seat lives in `opencode.jsonc` |

Guard rules (slice D4-S1): every `arms[].pool` resolves to a route in `opencode.jsonc`;
`control` is one of the arm ids; `review` is present and not before `start`; `revert`
is non-empty; `status` is in the enum; a trial in `running` status for longer than
`review` + 3 days is a guard failure (it has been forgotten). A `workhorse-weighted` seat
that is not in the registry is a warning, not a failure, until D4-S2 seeds the file.

### 4.2 Registry seed (what V2 records on day one)

| Trial | Arms | Status | Why |
|---|---|---|---|
| `deepseek-effort-rr` | Flash high (3) / low (1), control high | `running` · imbalanced | 0 low draws in 72 h |
| `luna-effort-rr` | Luna medium / high (round-robin, control high) | `running` · imbalanced | 4,583 medium vs 71,476 high in 72 h (6 %) |
| `deepseek-pro-retire-or-trial` | Pro high / max, control high | `proposed` | Defined, zero attempts, dominated on AA (§5.4) |
| `mimo-v2.6-pro-opendesign` | default / high (variant set unverified) | `proposed` | AA 46, $0.13/task, no door traffic yet (§6.2) |
| `gpt-6-astra-openai` | medium / high, control high | `proposed` | AA 51–55, no traffic, plan coverage unverified |
| `beam-fast-road-go` | beam-free, 1 arm (no effort rung in AA) | `proposed`, starts 2026-11-15 | Go door walled until the reset |

## 5. Dominated-model detection (algorithm, prototype, first results)

### 5.1 Algorithm

Input: for each (model, effort) pair, quality `q` (AA raw score), AA cost per task `c_aa`,
measured door rows `(door, model, variant, n, walled, ok, tps_med)`.

```
for (model, effort) in AA rows with q and c_aa:
    door   = argmax over measured rows of conditional_success
             where n >= 50 and conditional_success >= 0.5          # live door only
    if door is None:   c = c_aa; s = AA speed; source = aa-only
    else:              c = c_aa / conditional_success(door); s = tps_med(door); source = measured
    tier   = cheap if c <= 0.5, standard if c <= 3, frontier otherwise (unpriced if c missing; excluded)

skyline per tier, dominance on (q up, c down) and, for tiers small/speed/scout only, (s up)
    a dominates b  iff  a.q >= b.q and a.c <= b.c and (not speed or a.s >= b.s)
                        and at least one strict

for each seat (pool, model, effort) in the pool's current members:
    if seat measured and dead (every live-eligible row < 0.5 conditional success): verdict dead-seat
    elif seat not on frontier of its tier:
        live dominators = [d in tier if dominates(d, seat)  and d.source == measured]
        verdict replace (live) else trial-first (dominators exist, none live)
    else verdict on-frontier
output: JSON per seat: {pool, seat, verdict, by[]}
```

Properties checked by `dominance.py selftest` (3,000 random point sets with integer
grids so ties occur; PBT with a fixed seed, stdlib `random`):

1. The skyline (sort by quality desc, cost asc, speed desc, then keep points not
   dominated by a kept point) equals the brute-force O(n²) pairwise set. **Oracle:
   brute-force pairwise dominance.**
2. The frontier does not depend on input order.
3. Every dominated point has a frontier dominator (dominance is well-founded).
4. Dominance is antisymmetric (no two points dominate each other).

Ties are not dominance: two identical points both stay on the frontier. That matters on
the speed axis, where measured tokens/s are often equal across two doors.

### 5.2 Where the output goes

Output is a **proposal**, never a config change. A `replace` verdict becomes a trial
record (§4) with the dominator as the treatment arm and the seat as control. A
`dead-seat` verdict is not a trial: a seat whose only live door is dead is removed in a
config PR with the ledger rows cited (AD9 says "never burn walled doors"; removing a dead
seat burns nothing, so it needs no trial; challenge C2). A `trial-first` verdict is a
candidate for §6.2, not a swap.

### 5.3 Prototype

`research/prototypes/dominance.py`:
- `extract` reads `route_attempt` read-only over the 14-day window and writes
  `d4-measured-14d.json` (156 door-model-variant rows; models restricted to the
  `AA_MAP` names).
- `run` builds candidates from the AA rankings (AA|v5 rows, de-duplicated; rows labelled
  "new release / awaiting source-backed assessment" dropped) and the snapshot, prints the
  frontier size per tier and the seat verdicts for the current pool seats (the
  `POOL_SEATS` table mirrors the config as of 2026-11-13).
- `selftest` runs the oracle and PBT (§5.1).

Mapping hypotheses are explicit in `AA_MAP`: `deepseek-v4-flash → deepseek-v4.1-flash`
(not exact: classic vs 1107), `claude-fable-5-2 → claude-fable-5.1` (not exact: version
differs), `mistral-small-latest → mistral-small-4` (not exact), `beam-free → reflection-beam`
(not exact). The non-exact rows are flagged and must not decide a swap on their own.

### 5.4 First results (2026-11-13, live data)

Frontier sizes: cheap 8 of 27 candidates, standard 5 of 26, frontier 5 of 28.

Seat verdicts (14-day snapshot, AA_MAP mapping):

| Pool | Seat | Verdict | Dominated by (live) | Reading |
|---|---|---|---|---|
| workhorse-weighted | gemini-4-hydrogen#low | **replace** | claude-haiku#high, gpt-6.1-terra#low, #medium | hydrogen door is dead (`_antigravity/gemini-4-hydrogen` 125/125 failed, 14 d); haiku/terra are live |
| workhorse-weighted | claude-sonnet-5-5#medium | **replace** | gpt-6.1-sol#xhigh | Sonnet medium door 13–18 % success in 72 h |
| workhorse-weighted | gpt-6-luna#medium | trial-first | claude-haiku#low (not live: conditional 0.49) | Luna medium is live (99.7 %) — the verdict is about a dominator that isn't live; keep, do not swap |
| workhorse-last-resort | claude-haiku-5-5#high | trial-first | gpt-6.1-sol#low (AA only) | Haiku high has 5,000+ throttled Claude attempts; no live dominator |
| workhorse-last-resort | claude-fable-5-2#low | trial-first | gemini-4-hydrogen#xhigh, sonnet#xhigh, kimi-k3.1-pro#max, opus#high (none live) | fable low 9 % conditional |
| speed | gemini-4-hydrogen#high | **replace** | gpt-6.1-sol#xhigh | dead door |
| speed | deepseek-v4.1-flash#high, muse-spark#xhigh | `no-aa-row` / dead-seat | — | AA has no `high` rung for DS Flash (only non-reasoning and max) and no `xhigh` for muse; the seat cannot be scored (§6.6) |
| speed | claude-haiku-5-5#high | on-frontier (speed tier) | — | haiku high is the fastest live Claude seat (165 t/s measured) |
| summit | claude-fable-5-2#medium | trial-first | gemini-4-hydrogen#xhigh, sonnet#xhigh, kimi#max, opus#high | none live |
| summit | claude-opus-5-5#high | trial-first | gemini-4-hydrogen#xhigh | Opus door 16 % conditional |
| summit | gemini-4-argon#high | trial-first | gemini-4-carbon#high/medium, gpt-6-astra#max, opus#high, … | carbon and astra have no live door (§6.2) |
| summit | gemini-4-hydrogen#high | **replace** | gpt-6.1-sol#xhigh | dead door |
| summit | gpt-6.1-sol#xhigh | on-frontier | — | |
| specialist | claude-sonnet-5-5#high | **replace** | gpt-6.1-sol#xhigh | sonnet high 1 % conditional (`_claude`, RateLimit) |
| specialist | gpt-6.1-sol#medium | on-frontier | — | |

Read these as the mill's shortlist, not a decision. Three reasons:

- AA quality and cost are per AA version (5.0.1, 2026-10-31 snapshot); the fleet's
  effort mapping is not the AA effort (`medium` on Go DeepSeek is a remap, `v1-pools-efforts.md` b.4).
- The conditional success uses 14 days; the 72-hour door health (§2.2) is worse for
  Claude and better for OpenAI, so the same verdict can flip in a week.
- The "replace" list is dominated by one model, gpt-6.1-sol#xhigh. Its live door is
  `_openai.main`/`openai.alternate` with conditional success ~99 %. Its plan coverage
  (`sol-plan` lost coverage 2026-11-09, config 2649–2652) is the reason it is not
  automatically the replacement: the mill must confirm the plan allows it before a swap.

### 5.5 Dominated models in the catalog (not seats)

These catalog rows are dominated on AA and should be retired from the testing list
(§6.2), not trialled:

| Model (AA) | Dominated by (AA, same tier) | Note |
|---|---|---|
| minimax-m3.1-flash max (29, $0.49, 119 t/s) | claude-haiku-5-5 high (30, $0.16, 166 t/s) | haiku dominates on every axis |
| mistral-large-4 medium (23, $1.41) | claude-sonnet-5.5 medium (30.8, $0.85) | sonnet dominates |
| kimi-k3 low (17.4, $2.37) | claude-haiku-5-5 low (19, $0.08) | haiku dominates |
| inkling-small (12.9, $0.20) | claude-haiku-5-5 low (19, $0.08) | non-speed tiers only |
| reflection-beam (17.7, $0.28, 299 t/s) | claude-haiku-5-5 low on quality and cost | **not dominated on speed**: beam stays on the speed/scout road (§6.4) |
| gpt-6-sol high (44), gpt-5.6-sol (41) | gpt-6.1-sol high (50, $0.59) | predecessor/successor pair (brief §6 "known successors") |
| gpt-5.6-luna / gpt-5.6-terra | gpt-6-luna / gpt-6.1-terra | same |
| muse-spark-open xhigh (24.7, $2.02) | claude-sonnet-5.5 medium (30.8, $0.85) | non-speed |
| deepseek-v4-pro (AA 36 in catalog, **23** in rankings) | claude-haiku-5-5 high (30, $0.16) on quality and cost | catalog and rankings disagree; resolve first (§6.2 step 0) |

## 6. V2 design — the chosen options, precise

### 6.1 Trial protocol (AD9, operational)

1. **Register** (§4.1). No arm is seated without a registry entry.
2. **Balance.** Arms are seated with round-robin or weighted selection and the guard
   reads the 72-hour draw ratio: a registered arm whose draw share is below
   `0.5 × weight-share` for 72 h is a guard warning and the trial goes `review-due`
   (this is the defect found on Luna, §2.2).
3. **Measure.** Primary metric = conditional success per arm (walled excluded, §0);
   secondary = cost per completed task, TTFT p50, fall-through. Every number is reported
   with its `n` (R3; AD8).
4. **Verdict.** Non-inferiority: arm passes if the lower 95 % bound of
   `success(arm) − success(control)` is above `−delta` **and** cost per completed task
   ratio ≤ 1.00 (if the cost guard is stated). Use two pre-registered looks (the review
   date and `review + 7 d`) and no others (R1 §4.4.2, group-sequential).
5. **Sample size.** For NI margin `delta` at success `p` (both arms p), one-sided α = 0.05
   and power 0.80: `n per arm = (1.645 + 0.842)² · 2p(1−p) / delta²`. At p = 0.90:
   delta = 0.05 → **446**; delta = 0.02 → 2,790. (Two-sided point test at 5 points gives
   431 in R1, same order.) Pools that cannot reach it (summit: ~1,600 attempts / 72 h,
   specialist ~140) judge on the workhorse-measured row for the same (door, model,
   effort) with `provenance: measured-elsewhere` and do not decide a swap alone.
6. **Revert.** The revert condition in the registry is applied automatically by the
   guard when it trips; a human (Drew) confirms the revert.
7. **Bandit allocation (V4 only)** draws inside the arms' safe set (AD9); never outside.

### 6.2 Testing plan — never-tried and zero-attempt models

Population: catalog entries (non-disabled) with zero route attempts in 14 days after
name normalisation, joined to AA rows. From `aa-last-good.json` and the ledger:
**146 catalog rows** have no attempt (most are the ChatGPT-plan list, the Command Code
list, groq/openrouter/meta disabled, and `_vibe` names without a model). Of the 31 AA
rankings rows with a price and no 14-day attempt, the list below is the set that
survives dominance (§5.5) and has or can get a door.

Order is by AA points per dollar at the lowest priced rung, door-live first. Arms are
the rungs AA actually scores; where AA scores one rung the arm set is the variant set
the door declares (step 0 checks it).

| Pri | Model (door) | AA (q / $ per task / t/s) | Arms (≥ 2 effort levels where AA has them) | Door status (72 h) | Target n per arm | Status |
|---|---|---|---|---|---|---|
| P1 | **DeepSeek V4 Pro** (`deepseek-pro-plan`: OpenCode Go ×3, cline-pass, opendesign) | 23 max in rankings vs 36 in catalog — **conflict** / 0.67 / — | max vs high | only opendesign live for DeepSeek | 446 | step 0: resolve the AA conflict; if 23 holds, **retire the seat** (dominated by haiku high) instead of trialling |
| P1 | **MiMo V2.6 Pro** (cline-pass; opendesign) | 46 / **0.13** / — | default vs high (variant set unverified) | opendesign mimo 2 attempts (14 d) — door not yet exercised; cline walled | 446 | `proposed`, step 0: probe door variants |
| P1 | **Reflection Beam** (`opencode-go-plans/beam-free`) | 17.7 / 0.28 / **299** | 1 (AA has no effort rung) | Go walled to 2026-11-15; 9/9 success 11-04/05 | 446 on speed road | `proposed`, starts 11-15 (§6.4) |
| P2 | **GPT-6 Astra** (`openai` ChatGPT plan; gpt-6-astra) | high 55 (no $), max 57 / **$6.24** / 78 | medium vs high (AA: 51, 55), low (43) optional | `_openai` family live (96 %); plan coverage for astra unverified | 446 (summit: pooled) | `proposed`, step 0: plan check (D4-S9) |
| P2 | **Gemini 4 Carbon** (no door in catalog) | medium 58.3 / **6.59** / 73; high 60 / 7.76 / 80 | low, medium, high (AA: 54.2, 58.3, 60) | **no door** | 446 | door-first (D4-S11). If it lands it dominates summit opus#high on quality and cost |
| P2 | **Kimi K3** (cline-pass) | low 17.4 / 2.37 / 58; max not priced | low vs max | cline walled-throttled | 446 | dominated by haiku low on quality/cost (§5.5); retire from list |
| P3 | Inkling-small, Nemotron 3.5 Super VL, Gemini 3.5 Flash-Lite (speed/small candidates) | 12.9 / 0.20 / 204 · 13.6 / 0.54 / 474 · 11.7 / 0.69 / 406 | 1 each | **no door** (commandcode dead) | 446 | door-first; speed-tier only (dominated elsewhere) |
| P3 | Muse Spark Open, Step 5, Grok 4.7 | 24.7 / 2.02 · 28.5 / 2.35 · 30.9 (low) / 2.92 | low/xhigh | no door (Meta API disabled; StepFun, xAI not in config) | — | dominated on workhorse; door-first if ever needed |
| — | Claude Fable 5.1 vs fable-5-2 | AA 5.1 vs ledger 5-2 | low…max | `_claude` throttled 9 % at low | — | **version mismatch**; not a trial, an AA mapping fix |
| — | mistral-large-4 | medium 23 / 1.41; high 26 / 2.06 | medium, high | `_vibe` budget exhausted | — | dominated by sonnet medium; retire |

**Step 0 for every P1 row (cheap):** probe the door's declared variants (catalog and
`/route`), confirm the plan covers the model, and record the AA version it maps to.
Anything that fails step 0 leaves the list with the reason recorded.

### 6.3 DeepSeek Flash policy (B§8.3)

Rules, in order:

1. **Only where it is the door.** A pool may seat DS Flash as a normal member only when
   its other members are dead, walled or absent on the 72-hour window. Today that is
   `deepseek-flash-plan` (`opendesign` the only live leg) and `background` (MiMo first,
   DS Flash after; 2957–2961).
2. **Last resort only where Luna's quota is best.** A pool may seat DS Flash as last
   resort after its primary members only if the pool contains a Luna leg
   (`luna-plan`, `limited`, `workhorse-weighted`) and the quota feed (D3/D6) says Luna's
   headroom exceeds DS Flash's. This is the reading of B§8.3 "only places with the best
   Luna quota or worst DeepSeek-Flash quota": **open question OQ-D4-1** (§7) — the
   sentence can also mean "DS Flash quota is the worst, so use it as the last resort
   where it is scarce"; the default is the Luna reading because it keeps Flash out of
   pools where it would only fail.
3. **Off-peak first.** Within a DS pool, off-peak legs precede peak legs (§6.5).
4. **No Command Code, no Vibe DS 1017 seat while walled.** `_vibe` and `_commandcode`
   are removed from DS pools while their quota record shows walled (72 h: 880 and 778
   QuotaExceeded).

Effect on the current seats: `deepseek-flash-plan` (4 legs) keeps `opendesign` and the
Go legs (on the 11-15 reset) and loses `_vibe` and `cline-pass` as normal members (cline
is 0 % with RateLimit in 72 h, so it goes to last resort).

### 6.4 Beam on fast roads without stealing DS Flash quota (B§8.5)

- Beam (AA 17.7, $0.28, 299 t/s) is **not dominated on speed**: it stays on the speed
  road (`scout` first, 2890; `beam-plan`).
- It is reached only through `opencode-go-plans/beam-free` (the Command Code free tier
  ended 2026-11-13). The Go door is walled until 2026-11-15 and then gets the trial in
  §4.2 with one arm and the speed metric.
- **Quota guard:** Beam's door is the Go workspace that also serves DS Flash. Beam is
  placed ahead of DS Flash only in `scout` and `speed` (speed-tier pools) and only when
  the Go quota headroom (D3/D6) exceeds 20 %. Otherwise scout goes to DS Flash first.
  This keeps DS Flash quota for workhorse.

### 6.5 Variant splits (B§6.5, link code#9378, opencode#821)

**What each provider serves, by variant (14-day and 72-hour ledger).**

| Door (canonical) | DeepSeek variant served | Attempts 14 d | Success | Note |
|---|---|---:|---:|---|
| `opendesign` | `deepseek-v4.1-flash` @high | 21,801 | 86.4 % | 1107 (hypothesis, OQ-D4-2) |
| `opendesign` | `deepseek-v4-flash` @high | 1,733 | 94.9 % | classic (hypothesis) |
| `opencode-go.drewrypope` / `.popesdrew` / `.developing-today-0` | `deepseek-v4.1-flash` @high | 5,329 / 8,625 / 6,699 | 78–84 % | 1107 |
| `opencode-go.personal` (pre-rename) | `deepseek-v4.1-flash` @high | 18,249 | 97.2 % | 1107; history only |
| `cline-pass` | `deepseek-v4.1-flash` @high | 4,606 | 76.1 % | 1107 |
| `_vibe` / `mistral` | `deepseek-v4-1-flash-1017` @high | 22,615 / 141 | 48 % / 0 % | a third snapshot (1017), not the classic |
| `_commandcode` | `deepseek/deepseek-v4.1-flash` @high | 13,226 | 13.6 % | dead |
| `deepseek` (official, disabled) | `deepseek-flash` @low | 72 | 100 % | 2026-11-05 only |

So the served identities are: **1107 on Go, cline-pass, opendesign (most), and
Command Code; classic on opendesign only (small); 1017 on Vibe/Mistral.** No door serves
classic on a metered plan with volume.

**Pools (the ordering the brief names, B§6.5).** The `deepseek-plan` (the Flash pool)
becomes six members, each a door × variant × time band, with window gates from
`model_route_windows` (3690–3717):

```jsonc
"deepseek-flash-plan": {
  "strategy": "ordered",
  "targets": [
    { "ref": "opendesign/deepseek-v4.1-flash",  "variant": "high", "window": "NOT deepseek-peak-early", "id": "ds1107-offpeak" },
    { "ref": "opendesign/deepseek-v4-flash",    "variant": "high", "window": "NOT deepseek-peak-early", "id": "ds-classic-offpeak" },
    { "ref": "opencode-go-plans/deepseek-v4.1-flash", "variant": "high", "window": "NOT deepseek-peak-early", "id": "ds1107-offpeak-go" },
    { "ref": "opendesign/deepseek-v4.1-flash",  "variant": "high", "window": "deepseek-peak-early", "id": "ds1107-peak" },
    { "ref": "opendesign/deepseek-v4-flash",    "variant": "high", "window": "deepseek-peak-early", "id": "ds-classic-peak" },
    { "ref": "_vibe/deepseek-v4-1-flash-1017",  "variant": "high", "id": "ds-payg-offpeak" }
  ]
}
```

Peak versus off-peak is not a single gate: `deepseek-peak-early` and `deepseek-peak-late`
are two windows (3690–3717), and the existing config composes them the same way the
background route does. The `id` fields are the names the brief uses; the schema for `id`
is D1's (overlay keys). The brief's `ds-payg-*` names map to the metered Vibe and
DeepSeek-direct legs: the direct `deepseek` provider is disabled (config 830–903), so
PAYG-direct is **not** available and `ds-payg-*` in the first version is the Vibe 1017
leg only. Order within each band: 1107 before classic on the evidence that 1107 is the
volume door, and classic only where the classic door is measurably better (opendesign
94.9 % vs 86.4 %, n = 1,733 vs 21,801: directionally better, not yet significant; the
registry trial `ds-classic-vs-1107` tests it, §4).

**Peak data that the ordering rests on.** Off-peak success 81–97 %, peak 58–65 % on the
Go workspaces (§2.2). Off-peak is the default band for every DS member; peak legs are
the fallback. Holiday handling (Chinese public holidays Mon–Fri treated as peak by the
current windows) is not expressible; that is opencode#821. code#9378 carries the
time-of-day routing change that consumes these windows.

### 6.6 Extrapolation rules (B§7.2–7.5)

Each row is a rule with its provenance. Measured overrides extrapolation wherever the
ledger has n ≥ 50 on the (door, model, effort).

| # | Where AA and the fleet differ | Rule (V2) | Evidence | Provenance |
|---|---|---|---|---|
| E1 | **OpenAI punishes effort above high** (B§7.2) | Effort rung above `high` on an OpenAI model is priced at AA's marginal: `xhigh → max` costs **+139 %** for **+6 points** on terra (0.40 → 0.96 $; 45 → 51) and **+67 %** for **+3** on sol (0.76 → 1.27; 52 → 55). Never seat `max` on OpenAI unless the route needs it; `xhigh` is the ceiling for workhorse | AA rankings, gpt-6.1-terra and gpt-6.1-sol rows | extrapolated-from-AA |
| E2 | **Luna `xhigh` vs `high`** | AA: xhigh 23 at $0.10, high 21 at $0.07: +2 points for +43 % cost. The ledger's `xhigh` mass is history (pre-rename); the trial must measure before any change | AA; §2.2 | extrapolated-from-AA, awaiting measurement |
| E3 | **All Claude may cost more per %-dollar than AA expects** (B§7.4) | Multiply AA $/task by **3.0** as a provisional Claude factor for ranking only, until the quota feed gives utilisation % per call. Do not publish the factor as a measurement | Ledger has no Claude $ (subscription; `quota` has reset times only). AA high/max Opus $10.04 vs high $3.46 | extrapolated-from-AA, provisional (OQ-D4-3) |
| E4 | **Off-peak is half price on DeepSeek** | AA's `offpeak-derived` row for DS Flash non-reasoning is $0.2134 vs $0.4268 at snapshot: a **mirror pair** giving the off-peak factor **0.50** (matches DeepSeek's published "off-peak rates are half of peak"). Apply 0.5 to DS off-peak members | AA rankings rows (2026-10-31, offpeak-derived and snapshot) | measured-pair, DeepSeek published rate |
| E5 | **Low context is valuable** (B§7.3) | No AA row exists for Luna at 100 k vs full. No extrapolation. The `limited` pool's `cap.context: 100000` is used as the value and is itself the trial: arm = Luna @100 k vs Luna @1 M on the same door | AA has no row | ruled-unknown, trial-first |
| E6 | **Real speed differs from AA** (B§7.5) | AA speed is **unusable for DeepSeek** (1,486 t/s non-reasoning vs 160–250 measured). For Haiku, Muse and OpenAI the AA speed is within ±20 % of measured. Rule: speed comes from the measured door in speed-tier pools (small, speed, scout) and from AA elsewhere; never from AA for DS | rankings vs ledger tps | measured overrides |
| E7 | **Speed does not matter off the speed road** (B§7.5, B§8.4) | `workhorse`, `summit`, `specialist`, `background` ignore speed in dominance (§5.1) | — | ruled (brief) |
| E8 | **Measured success is a door fact, not a model fact** | Dominance uses conditional success of the *door*; quality comes from AA; the two are never averaged | §0 | ruled |
| E9 | **Effort rungs AA does not score** | `deepseek-v4.1-flash#high` (AA has non-reasoning and max only) and `muse-spark#xhigh` (estimated row) take the nearest AA rung with provenance `aliased`, and their verdict is `no-aa-row` until a measured row exists | rankings | aliased |
| E10 | **AA version drift** (catalog 36 vs rankings 23 for DS V4 Pro; fable 5-2 vs 5.1) | Any AA row whose version does not match the fleet's model id is `aliased` and cannot decide a swap alone | §5.3 | ruled |

### 6.7 Dollar-efficiency objective per pool (B§8.4)

For each pool P with quality floor `Q_P` (the AA band the pool's job needs, set by D1's
tier table) and, for speed-tier pools only, a speed floor `S_P`:

```
minimise  E[cost per completed task]  =  c(member) / conditional_success(door)
subject to  q(member) >= Q_P
            s(member) >= S_P        only if P ∈ {small, speed, scout}
            member is on a live door (conditional_success >= 0.5)
```

Ranking within P is by that expected cost; ties go to the higher quality. This is the
objective `dominance.py` implements as the cost axis, with the speed axis added only in
the speed tiers. "Cost is king in workhorse" (B§7.6) means the workhorse floor `Q_P` is
set by the lowest-cost member that is not dominated, not by the best member.

### 6.8 Provider coverage audit (B§8.7 "all providers adequately used")

"Adequately used" is operationalised as: a configured provider with at least one
seat in a route is **either** live (conditional success ≥ 0.5 over 72 h with ≥ 50
attempts) **or** on the dead/walled list with a named owner and a dated repair. Current
state (72 h unless noted):

| Provider (canonical) | 72 h attempts | State | Seats today | Action |
|---|---:|---|---|---|
| `_openai.main` (+ `_openai`) | 97,825 | **live, 96 %** | luna-plan, workhorse-weighted, sol-plan, terra | Over-concentrated: 66 % of 72 h traffic is OpenAI. Spread via trials (Astra, Carbon), not by forcing |
| `_openai.alt` | 3,761 | **dead for terra (25 %)**, missing `api.responses.write` scope | terra-plan | Repair scope (owner: Drew); until then remove terra from `_openai.alt` |
| `opendesign` | 13,329 | **live 77 %** (InvalidRequest concurrency 11/10) | DS Flash, DS Pro, Luna, Sol, MiMo, Haiku | Concurrency tier limit: cap DS concurrency per pool; keep |
| `_claude` | 28,370 | **throttled, 19 %; unsigned for 1,322** | haiku, sonnet, fable, opus | Cooldown on RateLimit (D4-S15); Claude sign-in repair (owner: Drew) |
| `_muse` / `muse-direct` | 256 / 0 | **walled to 2026-11-16** | speed, workhorse-weighted, summit, specialist | No action; trial resumes 11-16 |
| `opencode-go.*` (3 canonical) | 1,984 | **walled to 2026-11-15** | DS Flash, DS Pro, Beam, MiMo, Muse, Argon, Haiku | Beam trial at reset (§6.4) |
| `cline-pass` | 654 | **throttled, 0 %** | DS Flash, DS Pro, MiMo, Muse, Kimi | Rate-limit cooldown; DS Flash member demoted to last resort |
| `_vibe` / `mistral` | 882 + 137 | **walled (budget exhausted)** | DS 1017, GLM | Remove from normal DS members until budget verified |
| `_antigravity` | 475 | **40.8 %** (keyring unset for ~1,200 errors in 14 d) | Hydrogen, Argon, Sonnet, Gemini Flash | Hydrogen seat replaced (§5.4); keyring repair |
| `_commandcode` | 1,381 | **dead** | none (removed) | Keep provider off; re-seat only on verified balance (config 2167) |
| `opencode-go.lab` | 0 (14 d: 253, 0 %) | **retired** | none | Remove the id from the ledger alias map (D4-S4) |
| `deepseek` (official, disabled) | 0 (14 d: 82) | residue | none | Confirm disabled and leave |

Nothing in this list is a silent failure any more: each row has a state and an owner.

### 6.9 Worked examples

**Example A — replacing a dead seat on `workhorse-weighted` (hydrogen#low).** The seat
`_antigravity/gemini-4-hydrogen#low` has weight 10 and 125 attempts, all failed, in 14
days (`keyring credential` and `not signed in` errors), so its conditional success is
0 and it is `dead-seat` under §5.1. The dominance run also finds live dominators
(claude-haiku#high, gpt-6.1-terra#low, #medium). Because the seat is dead, the action is
a config PR removing the seat with the ledger rows cited (no trial, §5.2). The
replacement is a **trial** only if the mill wants to test haiku#high against the Luna
seat: a registry entry `hydrogen-to-haiku-high` with arms haiku#high (treatment) and
`luna-effort-rr` high (control), n = 446, NI delta 0.05.

**Example B — the DeepSeek effort trial, reviewed on 2026-11-20.** Registry entry
`deepseek-effort-rr` (§4.1). On 2026-11-13 it has 0 `low` draws in 72 h. The guard
trips the balance check (§6.1 step 2) and sets `review-due`. Two things must happen
before a verdict: the low arm is seated so that it draws (the cause is the workhorse
stage, not the arm), and 446 low attempts are collected. The verdict is then a 2-look
non-inferiority decision on conditional success (low ≥ high − 0.05 relative, per the
registry; the registry's revert of "< 90 % of high" is a relative test and stays as the
revert). If low draws at 1 : 3 and succeed at 0.9× high, the trial moves to `adopted`
for the workhorse stage with the cost guard checked.

**Example C — a new model with no traffic (MiMo V2.6 Pro).** Step 0 probes cline-pass
and opendesign for the declared variants and checks the plan. It is cheap (AA $0.13 per
task) and quality-competitive (46, above sonnet medium). It is `proposed` as a
registry entry with arms default / high on `workhorse-weighted` with weight 1 and
control = current `workhorse-weighted` member with the lowest cost per point. Its
dominance status: not dominated (no cheaper live member with higher quality). n = 446 at
the workhorse volume (~4k attempts / day on the workhorse route) takes under a day once
the door is live.

**Example D — variant split on opendesign.** The 1107 leg (`ds1107-offpeak`) and the
classic leg (`ds-classic-offpeak`) both sit on opendesign, so the door-level 76 % and
the variant-level 86 % / 95 % are two rows, not one. The registry trial `ds-classic-vs-1107`
(arms classic, 1107, control 1107, NI delta 0.03) decides the order inside the off-peak
band. Until it reports, 1107 stays first (volume door).

## 7. V3 and V4 layers

**V3 — measured and automatic (gate: ≥ 14 days of D3 events per tuned decision).**
- Shrinkage of measured conditional success toward the AA-derived prior (O4 in §3.5),
  with Beta posteriors per door-model-effort and an `n`-aware interval in every report.
- Weekly dominance proposals from the D3 export (not from the 14-day snapshot), written
  as registry proposals only.
- Quota-aware Beam and DS Flash guard (§6.4, §6.3) fed by the D3/D6 quota series.
- Calibrated extrapolation: E1, E3 refit from the quota feed; E5 measured by trial.
- Door alias table (D4-S4) becomes the `dim_model`/door dimension in D3.

**V4 — learned and adaptive (gate: V3 shows the hand-tuned baseline limits the outcome).**
- Thompson sampling over (door, effort) arms **inside** a pool's safe set, with a floor
  guard: never route to an arm whose lower bound on success is below the pool floor (R1
  O11; AD9).
- Bradley–Terry strengths from pairwise judged outputs (R1 §4.2) once a judge exists
  (OQ-D4-4), replacing AA quality for the dominance axis.
- Automatic swap decisions by group-sequential test (R1 O14), with a human confirmation
  for any change to a `dead-seat` or a plan-covered model.

## 8. Risks and open questions

| # | Risk / question | Default chosen | Why / where applied |
|---|---|---|---|
| R-1 | Trials are unbalanced (Luna 6 % medium; DS low 0 draws) and conclusions drift | Guard rule §6.1 step 2 (balance); status `review-due` | §2.2 evidence; D4-S1, D4-S3 |
| R-2 | Dominance proposes swaps on AA rows that are the wrong version | `aliased` provenance cannot decide alone; step 0 for every P1 | §5.3, E10 |
| R-3 | Conditional success hides a true walled door (quota wall classed as "walled", but the wall is the problem) | Walled attempts are excluded from the rate but reported; the 72 h table shows the raw rate | §0 |
| R-4 | RateLimit retries burn the Claude window | Cooldown guard D4-S15 (default: 5 min per leg on RateLimit, 30 min on repeated) | §2.2, 19,985 in 72 h |
| R-5 | OpenAI concentration (66 % of 72 h traffic on one plan) | Trials on Astra and Carbon before any rebalance; no forced spread | §6.8 |
| R-6 | Peak penalty on Go workspaces could be a mis-measured window (window timezone) | Default: windows as configured (UTC, 01–04 and 06–10 Mon–Fri); test on ledger hours in D4-S14 before ordering changes | §2.2 peak table |
| R-7 | Holiday weekdays are misclassified | Accept until opencode#821 | config 3698–3717 |
| OQ-D4-1 | Brief B§8.3 sentence: "best Luna quota or worst DeepSeek-Flash quota" — does "worst" mean scarcest (use DS as last resort where it is scarce) or the opposite? | Luna reading (§6.3 rule 2). **Recorded for the mill; a RULING candidate for Drew** | the sentence reads two ways |
| OQ-D4-2 | Is `deepseek-v4.1-flash` the 1107 snapshot and `deepseek-v4-flash` the classic? The config does not say | Yes, as hypothesis; verify from provider model cards (D4-S14) before ordering pools | §6.5 |
| OQ-D4-3 | Claude per-%-dollar factor 3.0 | Provisional, ranking-only, until the quota feed gives % per call (D4-S17) | E3 |
| OQ-D4-4 | Is there a pairwise judge the fleet can run for BT (V4)? | Not in V2; AA quality is the axis | §7 |
| OQ-D4-5 | `cost_usd` on Go is a proxy; can it be a bill? | Index only; not used for ranking across Go workspaces (they disagree by 4×) | §2.3 |
| OQ-D4-6 | Does DeepSeek Pro belong at all? AA says 23 (rankings) or 36 (catalog) | Step 0 resolves; if 23, retire (dominated by haiku high) | §6.2 |

Challenges to `main.md` (for the mill; AD numbers are binding but these are noted):

- **C1 (AD9 — every swap is a trial).** A swap away from a dead seat (`dead-seat`) burns no
  walled door and needs no trial; requiring one would keep a dead seat in place for the
  trial's duration. Proposal: dead-seat removals go through config PR with ledger
  evidence; trials apply to live-to-live swaps.
- **C2 (AD9 sample sizes).** R1's 431 per arm at p = 0.90 is a point test; the registry
  uses a non-inferiority test (446) — same order. But AD9 does not say which look schedule;
  this design fixes two looks. Also the 90 % revert threshold in `deepseek-effort-rr` is a
  relative test; AD9 should state relative and absolute margins separately.
- **C3 (AD8, `dim_model`).** AD8 treats `route_attempt` as the ledger. Its `provider_id`
  is not normalised (11 renamed or retired ids in 14 days). D3's `dim_model` needs the
  alias table as a precondition, not as a V3 item.
- **C4 (AD1/AD2 — variant splits).** AD2's overlays key on `ref` + door. The DS
  variant split (§6.5) also needs the time band as a key. Ask D1 to allow a window key in
  overlay identity, or express the time band as a separate pool member (as §6.5 does).
- **C5 (brief §8.3, "DS Flash only where it is the door").** On 72 h data the only live
  DS Flash door is opendesign. That is consistent with the rule, but it means DS Flash
  is no longer a "last resort" anywhere the Go doors come back; the rule should say which
  quota wins when Go reopens on 11-15.

## 9. Worked numbers for reviewers

- 14-day ledger: 626,308 attempts; success 81.6 %; RateLimit 34,966; Authentication 32,448;
  QuotaExceeded 29,082; Transport 12,413.
- 72-hour canonical: `_openai.main` 79,876 / 96.2 %; `opendesign` 13,329 / 76.5 %;
  `_claude` 28,370 / 18.8 %; Go canonical 1,984 / 0 %; `_vibe` 882 / 0.1 %.
- Luna 72 h: high 71,476 (99.6 %), medium 4,583 (99.7 %) — ratio 15.6 : 1.
- DS Flash 14 d `low`: 36 attempts, 0 success, all 2026-11-05.
- Peak vs off-peak (Go drewrypope + popesdrew, DS Flash `high`, 14 d): off-peak 81.5 %
  and 85.8 % success (n 4,234 and 7,659); peak 65.2 % and 57.6 % (n 1,095 and 966).
- Sample size (NI, one-sided α = 0.05, power 0.80, p = 0.90): delta 0.05 → 446; delta 0.02 → 2,790.
- Frontier sizes (prototype): cheap 8/27, standard 5/26, frontier 5/28.

## 10. Statement coverage table

| Statement | Section of this design | Slice |
|---|---|---|
| B§6.5 variant splits, 1107 vs classic, peak/off-peak ordering | §2.2, §3.4, §6.5, §6.9 D | D4-S13, D4-S14 |
| B§7.1 low-tier preferences (DS off-peak, Luna 100k, OpenAI 388k, free models) | §6.6 E4, E5; §6.3 | D4-S12, D4-S18 |
| B§7.2 OpenAI punishes effort above high | §6.6 E1, E2 | D4-S17 |
| B§7.3 off-peak / low-context value via mirror pairs | §6.6 E4, E5 | D4-S16, D4-S17 |
| B§7.4 Claude per-%-dollar | §6.6 E3, OQ-D4-3 | D4-S17 |
| B§7.5 real speed sensitivity only in small/speed/scout | §6.6 E6, E7; §6.7 | D4-S5, D4-S18 |
| B§7.6 cost efficiency is king in workhorse | §6.7 | D4-S5 |
| B§8.1 schedule tests for never-tried models | §6.2 | D4-S6, D4-S7, D4-S8, D4-S10 |
| B§8.2 replace successors / dominated models; all providers used | §5, §6.8 | D4-S5, D4-S6, D4-S19 |
| B§8.3 DS Flash only where it is the door; last resort rule | §3.3, §6.3, OQ-D4-1 | D4-S12 |
| B§8.4 best dollar efficiency; speed only in small/speed/scout | §6.7 | D4-S5 |
| B§8.5 Beam on fast roads, not stealing DS Flash quota | §6.4 | D4-S10 |
| B§8.6 trial many models at 2+ effort levels | §6.2 | D4-S6, D4-S7, D4-S8, D4-S11 |
| B§8.7 provider coverage | §6.8 | D4-S19, D4-S15 |
| B9.3 Luna round-robin trial as the standing pattern | §2.1, §2.2, §4.1 | D4-S1, D4-S2, D4-S3 |
| B9.4 DeepSeek effort trial (PR #9381) | §2.1, §2.2, §4.2, Example B | D4-S2, D4-S3 |
| B9.2 `route_usage` vs ledger (quota vs dead doors) | §2.2 tag split | D4-S4 |
| AD9 pre-registered non-inferiority trials | §4, §6.1 | D4-S1, D4-S2 |
| AD8 `dim_model` alias / variant identity | §2.2, §6.5, C3 | D4-S4 |
| code#9378 time-of-day routing | §6.5 | D4-S13 |
| opencode#821 date-aware windows | §6.5, R-7 | D4-S13 (blocked on upstream) |
| Trial registry format (requested item 1) | §3.1, §4 | D4-S1, D4-S2, D4-S16 |
| Testing plan with arms and targets (requested item 2) | §6.2 | D4-S6..S11 |
| Dominance algorithm and prototype (requested item 3) | §3.2, §5 | D4-S5 |
| DS-Flash policy, Beam, coverage (requested item 4) | §6.3, §6.4, §6.8 | D4-S12, D4-S10, D4-S19 |
| Variant pools (requested item 5) | §3.4, §6.5 | D4-S13, D4-S14 |
| Extrapolation rules (requested item 6) | §3.5, §6.6 | D4-S16, D4-S17, D4-S18 |
| Dollar-efficiency objective (requested item 7) | §6.7 | D4-S5 |
