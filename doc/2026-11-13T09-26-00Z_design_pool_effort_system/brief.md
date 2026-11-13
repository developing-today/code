# Design brief — pools, effort tiers, OpenChamber categories, and the granular stats store

**Source:** Drew, main chat, 2026-11-13 03:26 CST (dictated, transcribed faithfully;
sectioning and grouping are the orchestrator's, wording and intent are Drew's).
Routed through the design mill at Drew's suggestion (2026-11-13 03:27 CST).

## 1. OpenChamber categories get appropriate pools

- Set appropriate pools for **all OpenChamber categories**.
- "wire up clef jev opencodego free then paid then cloudflare etc." — **[orchestrator
  note: this phrase did not decode; "opencodego free then paid then cloudflare"
  reads as a provider order (free tiers first, then paid, then Cloudflare).
  "clef jev" is carried verbatim for the statement inventory to map against the
  real OpenChamber category list, or record as a question with a default.]**
- **Ensure auto works** (categories on auto model selection must actually run).
- Make appropriate customizations; ensure it's wired well.

## 2. Self-switching, sub-sessions, and guidance

- Allow a model to **suggest/switch its own pool**, but only to **category pools
  or opencode-route pools**.
- Encourage **heavy use of parallel sub-sessions with limited context**, with
  guidance on **when not to use auto** and **when to change its own model**.
- Let them know **sub-sessions can make sub-sessions**.
- **By default use auto.** If auto breaks: fall back to **workhorse / general /
  build**, and set **small, scout, speed as the small-model options** (or so).

## 3. Context limits

- Use the **strictest context limits for in / out / cache / total** for each
  model, going up the pool (each tier applies the strictest limits that fit it).

## 4. Effort tiers: low / medium / high for every pool

- Ensure **low, high (and low/medium/high at least) work for every pool**.
- **Align current seats as medium.** Medium is the default.
- Using **AA scores and other factors**, determine a **relative equivalent
  option for every model and provider** at low and at high.
- Other efforts may have **entirely separate member lists per model**
  (scaling), or **overlay on top of the defaults**.
- **Until implemented, make xhigh / max / ultra / fast all point to high.**
- Overlays must be able to: **set any effort, remove a model-effort, replace a
  specific model or a specific model-effort**, etc.
- If there isn't a good low or high (low not capable enough; high too expensive
  for the improvement, or too slow, or whatever), **set it to the same value as
  medium, or don't overlay it**, depending.
- Cross-pool equivalences are real: sometimes **summit low is workhorse high**,
  or the middle between workhorse high and summit low; sometimes other factors
  dominate — **speed, or cost per point bracket**, etc.
- Present **and** implement low/medium/high for every pool.

## 5. Data the effort mapping must take into account (all of it)

AA data **and** current real fleet data: provider / model / effort; pool;
ratio per session; number of and depth of sub-sessions; which models, pools,
and sub-sessions are used; tool usage and **mcpx and fleet tool usage, each per
subcommand / action**; real diff +/-; number of attempts; triages; days to
complete from created / started, in hours or minutes of actual work; success
rate; existing time and quota % per session, task, PR, or attempt; etc.

Statistics wherever possible: **min, max, stddev, count, median, mean, sum,
p90, p99, p99.9, quartiles (Q1–Q3, IQR)**, etc.

## 6. The granular stats store

- Build **all the report details as a table** — materialized view or aggregated
  facts log over time — allowing **review per sample**, rolled up to intervals,
  aggregated by space and time (min / max / avg / med / mode / mode±x / etc.),
  and **group-by on any tag or label** — all possible groupings and subgroups.
- Ideally **constantly review stored aggregate stats** that are incredibly
  detailed and granular, **as events**, marking: when generated; **AA version**;
  time frame; number of sessions, turns, tokens; models, efforts, providers,
  pools across all; AA test version and test date; the model's **known
  successors / predecessors, in order**; the model-efforts it is aimed to match
  in price, ability, or speed; capabilities and unique details as **tags and
  labels**.
- For now **keep full granularity and measure storage growth**; later support
  rollup based on time since inject, time since capture, time since AA version.
- Variants **merged or split as separate**, e.g. marking known dates where a
  significant change happened, with **expected and actual measurable
  difference** — e.g. the newer **DeepSeek 1107**. **Luna 6 and Luna 6.1 are
  fully separate.** If variants like 1107 matter, split classic and 1107 as
  pools based on **which variant each provider actually serves**, and whether a
  provider offers DeepSeek 1107 and DeepSeek Flash original as separate
  variants. Then e.g. deepseek-plan might be ordered: **ds1107-offpeak,
  ds-classic-offpeak, ds1107-peak, ds-classic-peak, ds-payg-offpeak,
  ds-payg-peak**.

## 7. Low-tier preferences and real-vs-AA extrapolation

- In **low**, use or prefer: **DeepSeek off-peak, Luna 100k, the OpenAI
  equivalent at 388k, free models/providers, lower effort** — where
  appropriate: cost-efficient, and the level of up/down in score, cost, and
  real data where we have it.
- **Extrapolate where our scores differ from AA** in cost, success, or speed:
  e.g. OpenAI is much cheaper on small models and **punishes effort above high
  significantly** (relevant per model); off-peak or low context can be heavily
  valuable (no AA score other than inferring from a mirror pair); **all Claude
  may cost way more per %-dollar than AA scores would expect**; a provider may
  in real life be much slower or faster than benchmarks imply — that doesn't
  matter in background or specialist, but might for speed or scout; **cost
  efficiency is king in workhorse** — if a model is much slower or only a
  little dumber with real cost savings, switch.

## 8. Testing plan and dominated models

- **Schedule tests for models we never tried** (like the Luna test): round-robin
  cheaper-or-better trials depending on appropriateness. Usually aiming for
  **cheaper**, sometimes parallel or sensitive (small summit deviations; too
  much expense in small could have outsized impact).
- Try **replacing clear successors or dominated models** — slower, dumber, and
  more expensive than an alternative — across same or all providers; ensure
  **all providers are adequately used**.
- Maybe almost all places with **DS Flash should only use that**; only places
  with the **best Luna quota or worst DeepSeek-Flash quota** get DS Flash as
  **last resort** in other pools, to increase provider options per pool.
- Aim for **best dollar efficiency where all tasks are possible and as cheap as
  can be**, not caring about speed except in **small, speed, and scout**.
- **Add Beam on fast roads** where it isn't stealing useful DS Flash (etc.)
  quotas.
- Make a testing plan; **trial many/most major models at 2+ effort levels**
  where the efforts' price efficiency, cheapness, and scores align.

## 9. State of the world the design starts from (V1 inputs, 2026-11-13)

- Route classes and fallback chains landed (PR #9385): small → speed →
  workhorse/scout → summit → specialists (sink); semantics = regular chain
  first, fallback as second pass, walked with a per-call visited set; guard
  tests enforce reachability + termination. Full table:
  `.opencode/route-fallbacks.md` on main.
- `limited` pool landed (PR #9384): Luna @100k #medium, Terra @388k #low,
  DeepSeek @1M #low peak-aware, Haiku @388k #medium-last (388k = policy caps;
  natives are 1M / 1.05M). `small-100k` deleted; `small` consumes `limited`.
- DeepSeek effort trial landed (PR #9381): `deepseek-effort-rr` (#high w3 /
  #low w1) in workhorse-ordered. **DeepSeek #medium is not a real rung** — it
  silently remaps to #high on the Go and ClinePass doors.
- Explore sub-sessions pinned to scout (PR #9382). Agent model pins exist in
  the jsonc `agents` object (classifier→scout, reasoner→specialist,
  coder→workhorse precedents).
- Haiku #high leads workhorse-last-resort (PR #9388).
- Luna effort round-robin (PR #9018) is the standing trial pattern: named
  measurement, review date, revert condition.
- Quota analytics page + API live (/quota#analytics, /api/quota-analytics.json)
  — per pool/model/effort burn, ratios, projections, hour/day patterns.
- Roadmap & Stats Space (fleet-roadmap-stats) + its daily pipeline
  (~/workspace/goals/keep-the-four-repos-healthy/hidden_files/roadmap-stats/)
  compute throughput, churn, redo/duplicate, per-pool/per-model stats — the
  granular store in §6 extends this, it does not replace it.
- Time-of-day aware routing is ticketed: code#9378. DeepSeek peak windows
  already exist in config (`model_route_windows`, deepseek-peak-early/late;
  the background route composes them). Windows have no calendar/holiday
  support yet (fork issue opencode#821).
- Fleet tooling gaps already ticketed: code#9373 (fleet CLI invocation
  ledger), code#9374 (mcpx daemon call ledger) — per-subcommand usage data in
  §5 is partly inference until those land.
- AA data: ~/workspace/model-review-2026-11-12/aa-results.md/.json (AA v5.0.1)
  + the AI Benchmarks Space. Claim-model stamping works (code#8176 fix), so
  new trials are attributable per task.
