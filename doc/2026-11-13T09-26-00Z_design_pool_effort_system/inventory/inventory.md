# Statement inventory — pool & effort system (code#9399)

Mill step: statement inventory. Every requirement, preference, example, constraint and fact clause in
`brief.md` (§1–§9), `brief-decisions-supplement.md` (§0–§10) and the launch packet's standing rules is one
numbered statement. Machine-readable form: [`statement-map.json`](./statement-map.json); the checker is
[`check_inventory.py`](./check_inventory.py) (`python3 -I inventory/check_inventory.py`).

IDs: `B<section>.<n>` brief, `S<section>.<n>` supplement, `R<n>` launch-packet standing rules (kind `rule`, and
`fact-v1` where the packet states a V1 fact). Lines are 1-based in the source file. `Disposition` is a hint only:
`v1-exists` (already landed), `new-design` (needs a design decision), `new-slice` (buildable on its own),
`research` (needs measured data), `ruling-needed` (Drew's call).

## Counts

**Total statements: 366.**

| Theme | Count |
|---|---:|
| T0 v1-state | 45 |
| T1 categories-auto | 9 |
| T2 self-switch-subsessions | 6 |
| T3 context-limits | 2 |
| T4 effort-tiers-overlays | 21 |
| T5 data-basis-stats | 19 |
| T6 stats-store | 33 |
| T7 low-tier-extrapolation | 18 |
| T8 trials-dominated | 21 |
| T9 decision-endpoint | 24 |
| T10 decision-pools | 20 |
| T11 decision-executor-record | 13 |
| T12 llm-decider-protocol | 17 |
| T13 outcome-feedback-loop | 53 |
| T14 decision-effort | 19 |
| T15 conditional-clauses | 19 |
| T16 pm-review-surface | 9 |
| T17 cycle-safe-traversal | 8 |
| T18 standing-rules | 10 |

| Kind | Count |
|---|---:|
| constraint | 42 |
| example | 32 |
| fact-v1 | 45 |
| preference | 16 |
| question | 6 |
| requirement | 213 |
| rule | 12 |

| Disposition hint | Count |
|---|---:|
| new-design | 290 |
| new-slice | 19 |
| research | 10 |
| ruling-needed | 2 |
| v1-exists | 45 |

## Statements by theme

Columns: id, source lines, kind, verbatim text (shortened where over 110 characters; the JSON has the full text), disposition, existing issues/PRs with coverage.

### T0 v1-state (45)

| id | lines | kind | text | disposition | existing issues |
|---|---|---|---|---|---|
| B9.1 | 126 | fact-v1 | Route classes and fallback chains landed (PR #9385) | v1-exists | code PR9385 closed (full) |
| B9.2 | 126-127 | fact-v1 | small → speed → workhorse/scout → summit → specialists (sink) | v1-exists | code PR9385 closed (full) |
| B9.3 | 127-128 | fact-v1 | semantics = regular chain first, fallback as second pass, walked with a per-call visited set | v1-exists | code PR9385 closed (full) |
| B9.4 | 128-129 | fact-v1 | guard tests enforce reachability + termination | v1-exists | code PR9385 closed (full) |
| B9.5 | 130 | fact-v1 | `.opencode/route-fallbacks.md` on main | v1-exists | — |
| B9.6 | 131 | fact-v1 | `limited` pool landed (PR #9384) | v1-exists | code PR9384 closed (full) |
| B9.7 | 131 | fact-v1 | Luna @100k #medium | v1-exists | — |
| B9.8 | 131 | fact-v1 | Terra @388k #low | v1-exists | — |
| B9.9 | 132 | fact-v1 | DeepSeek @1M #low peak-aware | v1-exists | — |
| B9.10 | 132 | fact-v1 | Haiku @388k #medium-last | v1-exists | — |
| B9.11 | 132 | fact-v1 | 388k = policy caps | v1-exists | — |
| B9.12 | 133 | fact-v1 | natives are 1M / 1.05M | v1-exists | — |
| B9.13 | 133 | fact-v1 | `small-100k` deleted | v1-exists | — |
| B9.14 | 133 | fact-v1 | `small` consumes `limited` | v1-exists | — |
| B9.15 | 134 | fact-v1 | DeepSeek effort trial landed (PR #9381) | v1-exists | code PR9381 closed (full) |
| B9.16 | 134-135 | fact-v1 | `deepseek-effort-rr` (#high w3 / #low w1) in workhorse-ordered | v1-exists | code PR9381 closed (full) |
| B9.17 | 135 | fact-v1 | DeepSeek #medium is not a real rung | v1-exists | — |
| B9.18 | 135-136 | fact-v1 | it silently remaps to #high on the Go and ClinePass doors | v1-exists | — |
| B9.19 | 137 | fact-v1 | Explore sub-sessions pinned to scout (PR #9382) | v1-exists | code PR9382 closed (full) |
| B9.20 | 137-138 | fact-v1 | Agent model pins exist in the jsonc `agents` object | v1-exists | — |
| B9.21 | 138-139 | fact-v1 | classifier→scout, reasoner→specialist, coder→workhorse precedents | v1-exists | — |
| B9.22 | 140 | fact-v1 | Haiku #high leads workhorse-last-resort (PR #9388) | v1-exists | code PR9388 closed (full) |
| B9.23 | 141 | fact-v1 | Luna effort round-robin (PR #9018) is the standing trial pattern | v1-exists | code PR9018 closed (full) |
| B9.24 | 141-142 | fact-v1 | named measurement, review date, revert condition | v1-exists | — |
| B9.25 | 143 | fact-v1 | Quota analytics page + API live (/quota#analytics, /api/quota-analytics.json) | v1-exists | code PR9363 closed (full) |
| B9.26 | 144 | fact-v1 | per pool/model/effort burn, ratios, projections, hour/day patterns | v1-exists | — |
| B9.27 | 145 | fact-v1 | Roadmap & Stats Space (fleet-roadmap-stats) + its daily pipeline | v1-exists | — |
| B9.28 | 146 | fact-v1 | ~/workspace/goals/keep-the-four-repos-healthy/hidden_files/roadmap-stats/ | v1-exists | — |
| B9.29 | 147 | fact-v1 | compute throughput, churn, redo/duplicate, per-pool/per-model stats | v1-exists | — |
| B9.31 | 149 | fact-v1 | Time-of-day aware routing is ticketed: code#9378 | v1-exists | code #9378 open (full) |
| B9.32 | 149-150 | fact-v1 | DeepSeek peak windows already exist in config | v1-exists | — |
| B9.33 | 150-151 | fact-v1 | `model_route_windows`, deepseek-peak-early/late; the background route composes them | v1-exists | — |
| B9.34 | 151-152 | fact-v1 | Windows have no calendar/holiday support yet (fork issue opencode#821) | v1-exists | opencode #821 closed (related) |
| B9.35 | 153 | fact-v1 | Fleet tooling gaps already ticketed | v1-exists | — |
| B9.36 | 153-154 | fact-v1 | code#9373 (fleet CLI invocation ledger) | v1-exists | code #9373 open (partial) |
| B9.37 | 154 | fact-v1 | code#9374 (mcpx daemon call ledger) | v1-exists | code #9374 open (partial) |
| B9.38 | 154-155 | fact-v1 | per-subcommand usage data in §5 is partly inference until those land | v1-exists | code #9373 open (partial); code #9374 open (partial) |
| B9.39 | 156 | fact-v1 | AA data: ~/workspace/model-review-2026-11-12/aa-results.md/.json (AA v5.0.1) | v1-exists | — |
| B9.40 | 157 | fact-v1 | the AI Benchmarks Space | v1-exists | — |
| B9.41 | 157 | fact-v1 | Claim-model stamping works (code#8176 fix) | v1-exists | code #8176 closed (full) |
| B9.42 | 158 | fact-v1 | new trials are attributable per task | v1-exists | — |
| R13 | packet | fact-v1 | Command Code lost Sol plan coverage | v1-exists | code PR9401 closed (related) |
| R14 | packet | fact-v1 | free Beam tier ended permanently | v1-exists | — |
| R15 | packet | fact-v1 | monthly credits exhausted | v1-exists | — |
| R16 | packet | fact-v1 | PR #9401 removed it from six pools | v1-exists | code PR9401 closed (full) |

### T1 categories-auto (9)

| id | lines | kind | text | disposition | existing issues |
|---|---|---|---|---|---|
| B1.1 | 9 | requirement | Set appropriate pools for all OpenChamber categories. | new-slice | openchamber #14 open (related) |
| B1.4 | 13-14 | question | "clef jev" is carried verbatim for the statement inventory to map against the real OpenChamber category list | research | — |
| B1.5 | 15 | requirement | Ensure auto works | new-slice | — |
| B1.6 | 15 | requirement | categories on auto model selection must actually run | new-slice | — |
| B1.7 | 16 | requirement | Make appropriate customizations | new-slice | — |
| B1.8 | 16 | requirement | ensure it's wired well | new-slice | — |
| B2.7 | 25 | requirement | By default use auto. | new-slice | — |
| B2.8 | 25-26 | requirement | If auto breaks: fall back to workhorse / general / build | new-slice | — |
| B2.9 | 26 | requirement | set small, scout, speed as the small-model options | new-slice | — |

### T2 self-switch-subsessions (6)

| id | lines | kind | text | disposition | existing issues |
|---|---|---|---|---|---|
| B2.1 | 20 | requirement | Allow a model to suggest/switch its own pool | new-design | opencode #2384 open (related) |
| B2.2 | 20-21 | constraint | only to category pools or opencode-route pools | new-design | — |
| B2.3 | 22 | requirement | Encourage heavy use of parallel sub-sessions with limited context | new-design | opencode #1881 closed (related) |
| B2.4 | 23 | requirement | when not to use auto | new-design | — |
| B2.5 | 23 | requirement | when to change its own model | new-design | — |
| B2.6 | 24 | requirement | Let them know sub-sessions can make sub-sessions | new-design | opencode #1887 closed (related); opencode #1889 closed (related) |

### T3 context-limits (2)

| id | lines | kind | text | disposition | existing issues |
|---|---|---|---|---|---|
| B3.1 | 30 | requirement | Use the strictest context limits for in / out / cache / total for each | new-design | — |
| B3.2 | 31 | requirement | going up the pool (each tier applies the strictest limits that fit it) | new-design | — |

### T4 effort-tiers-overlays (21)

| id | lines | kind | text | disposition | existing issues |
|---|---|---|---|---|---|
| B4.1 | 35 | requirement | Ensure low, high (and low/medium/high at least) work for every pool | new-design | code PR9018 closed (related) |
| B4.2 | 35 | requirement | low/medium/high at least | new-design | — |
| B4.3 | 36 | requirement | Align current seats as medium. | new-slice | code PR9381 closed (related) |
| B4.4 | 36 | requirement | Medium is the default. | new-design | — |
| B4.5 | 37 | requirement | Using AA scores and other factors | new-design | code #8211 closed (related) |
| B4.6 | 37-38 | requirement | determine a relative equivalent option for every model and provider at low and at high | new-design | — |
| B4.7 | 39-40 | requirement | entirely separate member lists per model (scaling) | new-design | — |
| B4.8 | 40 | requirement | overlay on top of the defaults | new-design | — |
| B4.9 | 41 | requirement | Until implemented, make xhigh / max / ultra / fast all point to high. | new-slice | opencode #2203 closed (related) |
| B4.10 | 42 | requirement | Overlays must be able to: | new-design | — |
| B4.11 | 42 | requirement | set any effort | new-design | — |
| B4.12 | 42 | requirement | remove a model-effort | new-design | — |
| B4.13 | 42-43 | requirement | replace a specific model or a specific model-effort | new-design | — |
| B4.14 | 44-45 | constraint | If there isn't a good low or high (low not capable enough; high too expensive for the improvement, or too slo… | new-design | — |
| B4.15 | 45-46 | requirement | set it to the same value as medium, or don't overlay it, depending | new-design | — |
| B4.16 | 47 | constraint | Cross-pool equivalences are real | new-design | — |
| B4.17 | 47 | example | sometimes summit low is workhorse high | new-design | — |
| B4.18 | 48 | example | or the middle between workhorse high and summit low | new-design | — |
| B4.19 | 48-49 | constraint | sometimes other factors dominate | new-design | — |
| B4.20 | 49 | example | speed, or cost per point bracket | new-design | — |
| B4.21 | 50 | requirement | Present and implement low/medium/high for every pool | new-design | — |

### T5 data-basis-stats (19)

| id | lines | kind | text | disposition | existing issues |
|---|---|---|---|---|---|
| B5.1 | 52 | requirement | Data the effort mapping must take into account (all of it) | new-design | — |
| B5.2 | 54 | requirement | AA data and current real fleet data | new-design | code #8211 closed (related) |
| B5.3 | 54 | requirement | provider / model / effort | new-design | — |
| B5.4 | 54 | requirement | pool; | new-design | — |
| B5.5 | 55 | requirement | ratio per session | new-design | — |
| B5.6 | 55 | requirement | number of and depth of sub-sessions | new-design | — |
| B5.7 | 55-56 | requirement | which models, pools, and sub-sessions are used | new-design | — |
| B5.8 | 56 | requirement | tool usage | new-design | — |
| B5.9 | 56-57 | requirement | mcpx and fleet tool usage, each per subcommand / action | new-design | code #9373 open (partial); code #9374 open (partial) |
| B5.10 | 57 | requirement | real diff +/- | new-design | — |
| B5.11 | 57 | requirement | number of attempts | new-design | — |
| B5.12 | 57 | requirement | triages | new-design | — |
| B5.13 | 57-58 | requirement | days to complete from created / started, in hours or minutes of actual work | new-design | — |
| B5.14 | 58-59 | requirement | success rate | new-design | — |
| B5.15 | 59 | requirement | existing time and quota % per session, task, PR, or attempt | new-design | code PR9363 closed (related) |
| B5.16 | 61 | requirement | Statistics wherever possible | new-design | — |
| B5.17 | 61 | requirement | min, max, stddev, count, median, mean, sum | new-design | — |
| B5.18 | 62 | requirement | p90, p99, p99.9 | new-design | — |
| B5.19 | 62 | requirement | quartiles (Q1–Q3, IQR) | new-design | — |

### T6 stats-store (33)

| id | lines | kind | text | disposition | existing issues |
|---|---|---|---|---|---|
| B6.1 | 66 | requirement | Build all the report details as a table | new-design | code PR8297 closed (related); code PR8373 closed (related) |
| B6.2 | 66-67 | requirement | materialized view or aggregated facts log over time | new-design | code PR8297 closed (related) |
| B6.3 | 67 | requirement | allowing review per sample | new-design | — |
| B6.4 | 67 | requirement | rolled up to intervals | new-design | — |
| B6.5 | 68 | requirement | aggregated by space and time (min / max / avg / med / mode / mode±x / etc.) | new-design | — |
| B6.6 | 69 | requirement | group-by on any tag or label | new-design | — |
| B6.7 | 69 | requirement | all possible groupings and subgroups | new-design | — |
| B6.8 | 70 | preference | Ideally constantly review stored aggregate stats | new-design | code PR9363 closed (related) |
| B6.9 | 70-71 | requirement | that are incredibly detailed and granular | new-design | — |
| B6.10 | 71 | requirement | as events | new-design | — |
| B6.11 | 71 | requirement | marking: when generated | new-design | — |
| B6.12 | 71 | requirement | AA version | new-design | — |
| B6.13 | 72 | requirement | time frame | new-design | — |
| B6.14 | 72 | requirement | number of sessions, turns, tokens | new-design | — |
| B6.15 | 72-73 | requirement | models, efforts, providers, pools across all | new-design | — |
| B6.16 | 73 | requirement | AA test version and test date | new-design | — |
| B6.17 | 73-74 | requirement | the model's known successors / predecessors, in order | new-design | — |
| B6.18 | 74-75 | requirement | the model-efforts it is aimed to match in price, ability, or speed | new-design | — |
| B6.19 | 75-76 | requirement | capabilities and unique details as tags and labels | new-design | — |
| B6.20 | 77 | requirement | keep full granularity and measure storage growth | new-slice | — |
| B6.21 | 77-78 | requirement | later support rollup based on time since inject, time since capture, time since AA version | new-design | — |
| B6.22 | 79 | requirement | Variants merged or split as separate | new-design | — |
| B6.23 | 79-80 | example | e.g. marking known dates where a significant change happened | new-design | — |
| B6.24 | 80-81 | requirement | with expected and actual measurable difference | new-design | — |
| B6.25 | 81 | example | e.g. the newer DeepSeek 1107 | new-design | — |
| B6.26 | 81-82 | requirement | Luna 6 and Luna 6.1 are fully separate | new-design | — |
| B6.27 | 82 | requirement | If variants like 1107 matter | new-design | — |
| B6.28 | 82-83 | requirement | split classic and 1107 as pools based on | new-design | — |
| B6.29 | 83 | requirement | which variant each provider actually serves | new-design | — |
| B6.30 | 83-85 | question | whether a provider offers DeepSeek 1107 and DeepSeek Flash original as separate variants | research | — |
| B6.31 | 85 | example | Then e.g. deepseek-plan might be ordered | new-design | — |
| B6.32 | 85-87 | example | ds1107-offpeak, ds-classic-offpeak, ds1107-peak, ds-classic-peak, ds-payg-offpeak, ds-payg-peak | new-design | — |
| B9.30 | 147-148 | constraint | the granular store in §6 extends this, it does not replace it | new-design | — |

### T7 low-tier-extrapolation (18)

| id | lines | kind | text | disposition | existing issues |
|---|---|---|---|---|---|
| B7.1 | 91 | requirement | In low, use or prefer: | new-design | — |
| B7.2 | 91 | example | DeepSeek off-peak | new-design | — |
| B7.3 | 91 | example | Luna 100k | new-design | — |
| B7.4 | 91-92 | example | the OpenAI equivalent at 388k | new-design | — |
| B7.5 | 92 | example | free models/providers | new-design | — |
| B7.6 | 92 | example | lower effort | new-design | — |
| B7.7 | 92-94 | requirement | where appropriate: cost-efficient, and the level of up/down in score, cost, and real data where we have it | new-design | — |
| B7.8 | 95 | requirement | Extrapolate where our scores differ from AA in cost, success, or speed | new-design | code #6972 open (related) |
| B7.9 | 96 | example | OpenAI is much cheaper on small models | new-design | — |
| B7.10 | 96-97 | example | punishes effort above high significantly (relevant per model) | new-design | — |
| B7.11 | 97-98 | example | off-peak or low context can be heavily valuable | new-design | — |
| B7.12 | 98 | example | no AA score other than inferring from a mirror pair | new-design | — |
| B7.13 | 98-99 | example | all Claude may cost way more per %-dollar than AA scores would expect | new-design | — |
| B7.14 | 99-100 | example | a provider may in real life be much slower or faster than benchmarks imply | new-design | — |
| B7.15 | 100-101 | constraint | that doesn't matter in background or specialist | new-design | — |
| B7.16 | 101 | constraint | but might for speed or scout | new-design | — |
| B7.17 | 101-102 | preference | cost efficiency is king in workhorse | new-design | — |
| B7.18 | 102-103 | requirement | if a model is much slower or only a little dumber with real cost savings, switch | new-design | — |

### T8 trials-dominated (21)

| id | lines | kind | text | disposition | existing issues |
|---|---|---|---|---|---|
| B8.1 | 107 | requirement | Schedule tests for models we never tried | new-slice | code PR9018 closed (related); code PR9381 closed (related) |
| B8.2 | 107 | example | like the Luna test | new-design | — |
| B8.3 | 107-108 | requirement | round-robin cheaper-or-better trials depending on appropriateness | new-design | — |
| B8.4 | 108-109 | preference | Usually aiming for cheaper | new-design | — |
| B8.5 | 109 | constraint | sometimes parallel or sensitive | new-design | — |
| B8.6 | 109 | example | small summit deviations | new-design | — |
| B8.7 | 109-110 | constraint | too much expense in small could have outsized impact | new-design | — |
| B8.8 | 111 | requirement | Try replacing clear successors or dominated models | new-design | — |
| B8.9 | 111-112 | constraint | slower, dumber, and more expensive than an alternative | new-design | — |
| B8.10 | 112 | requirement | across same or all providers | new-design | — |
| B8.11 | 112-113 | requirement | ensure all providers are adequately used | new-design | — |
| B8.12 | 114 | preference | Maybe almost all places with DS Flash should only use that | new-design | — |
| B8.13 | 114-116 | requirement | only places with the best Luna quota or worst DeepSeek-Flash quota get DS Flash as last resort in other pools | new-design | — |
| B8.14 | 116 | preference | to increase provider options per pool | new-design | — |
| B8.15 | 117-118 | requirement | Aim for best dollar efficiency where all tasks are possible and as cheap as can be | new-design | — |
| B8.16 | 118 | constraint | not caring about speed except in small, speed, and scout | new-design | — |
| B8.17 | 119 | requirement | Add Beam on fast roads | new-slice | code #6972 open (related) |
| B8.18 | 119-120 | constraint | where it isn't stealing useful DS Flash (etc.) quotas | new-design | — |
| B8.19 | 121 | requirement | Make a testing plan | new-slice | code PR9018 closed (related) |
| B8.20 | 121 | requirement | trial many/most major models at 2+ effort levels | new-design | — |
| B8.21 | 122 | constraint | where the efforts' price efficiency, cheapness, and scores align | new-design | — |

### T9 decision-endpoint (24)

| id | lines | kind | text | disposition | existing issues |
|---|---|---|---|---|---|
| S0.1 | 7 | constraint | Clef and Jev are not regular LLM models | new-design | — |
| S0.2 | 7 | constraint | They are decision models | new-design | — |
| S0.3 | 7 | constraint | the schema is known as System 1 decisions | new-design | — |
| S0.4 | 9 | constraint | Jev is Command Code's decision model, served on the `v1/systemone` contract | ruling-needed | — |
| S0.5 | 9 | constraint | JSON state + typed questions in | new-design | — |
| S0.6 | 9 | constraint | probabilities out, never text | new-design | — |
| S0.7 | 10 | constraint | Clef fronts that same shape for OpenChamber's classifier client | new-design | opencode #1651 closed (related) |
| S0.8 | 11 | constraint | Existing anchors in the fork | new-design | — |
| S0.9 | 11 | constraint | the façade program opencode#1146 | new-design | opencode #1146 open (partial) |
| S0.10 | 11 | constraint | one endpoint per real standard | new-design | opencode #1146 open (partial) |
| S0.11 | 11 | constraint | chat-shaped traffic gets `/v1/chat/completions` + `/v1/responses` | new-design | opencode #1146 open (partial) |
| S0.12 | 11 | constraint | the decision family keeps a systemone-standard endpoint | new-design | opencode #1146 open (partial) |
| S0.13 | 11 | constraint | the clef fold-in opencode#1410 | new-design | opencode #1410 closed (related); code PR5583 closed (related) |
| S1.1 | 15 | requirement | OpenCode should support the System 1 standard if it doesn't already | new-design | opencode #1146 open (partial) |
| S1.2 | 15 | requirement | all conversation/etc. endpoints are accepted coming in | new-design | — |
| S1.3 | 15 | requirement | either pass through or adapt to the appropriate end state | new-design | — |
| S1.4 | 16 | preference | If one protocol is always better when supported | new-design | — |
| S1.5 | 16 | preference | a smarter protocol is equivalent for interaction but more wire-efficient | new-design | — |
| S1.6 | 16 | preference | or lets us add metadata for better performance or quality | new-design | — |
| S1.7 | 16 | preference | prefer it | new-design | — |
| S1.8 | 16 | requirement | Support long-term caching of all sessions | new-design | — |
| S1.9 | 16 | requirement | conversations continuable by id, search id, tags, create/modify date | new-design | — |
| S1.10 | 17 | requirement | Accept all model endpoints for raw usage of pools and providers | new-design | — |
| S1.11 | 17 | requirement | pass through or transform into a better protocol when there is no downside or a worthwhile upside | new-design | — |

### T10 decision-pools (20)

| id | lines | kind | text | disposition | existing issues |
|---|---|---|---|---|---|
| B1.2 | 10 | requirement | wire up clef jev opencodego free then paid then cloudflare etc. | research | code #9399 open (related) |
| B1.3 | 12 | question | reads as a provider order (free tiers first, then paid, then Cloudflare) | research | — |
| S2.1 | 21 | constraint | Drew gave two taxonomies | new-design | — |
| S2.2 | 21 | requirement | the mill settles the final shape | new-design | — |
| S2.3 | 23 | example | decision plan · decision LLM plan · decision free · decision LLM free · decision pay-as-you-go · decision LLM… | new-design | — |
| S2.4 | 24 | example | decision bland · decision free · decision pay-as-you-go | new-design | — |
| S2.5 | 24 | example | composed of decision native free, decision LLM free | new-design | — |
| S2.6 | 26 | requirement | The parts build up together into the decision pool | new-design | code #9399 open (related) |
| S2.7 | 26 | requirement | the one used by default because config sets it as the default decision model | new-design | — |
| S2.8 | 28 | requirement | Add free decision models from every place we can find them first | new-slice | — |
| S2.9 | 28 | requirement | then all the appropriate plan decision models | new-design | — |
| S2.10 | 28 | requirement | then PAYG | new-design | — |
| S2.11 | 29 | requirement | Where more than one candidate exists, compare cost and capability | new-design | — |
| S2.12 | 30 | requirement | Track the $10 (or whatever) of Cloudflare usage included with Workers | new-slice | code #8807 open (related); code #8820 open (related) |
| S2.13 | 30 | requirement | use up to that included amount for decision calls | new-design | — |
| S2.14 | 30 | requirement | disengaging once the included amount is hit | new-design | — |
| S2.15 | 30 | requirement | pivoting after that | new-design | — |
| S2.16 | 31 | requirement | Add the OpenAI decision option | new-design | code #8163 open (related) |
| S2.17 | 31 | constraint | if it is cheap enough when converted into dollars of quota percentages | new-design | — |
| S2.21 | 33 | preference | Possibly integrate local decision models if they exist and are used | new-design | — |

### T11 decision-executor-record (13)

| id | lines | kind | text | disposition | existing issues |
|---|---|---|---|---|---|
| S3.1 | 37 | requirement | Make a skill or runbook or mcpx tool for executing a decision | new-slice | — |
| S3.2 | 37 | requirement | It takes in a JSON decision supporting either of the two or three main formats | new-design | — |
| S3.3 | 37 | requirement | returns valid JSON matching the schema | new-design | — |
| S3.4 | 37 | requirement | with valid percentages / the response to the decision | new-design | — |
| S3.5 | 38 | requirement | Config should set the decision-model default to our decision pool | new-slice | — |
| S3.6 | 39 | requirement | Every decision gets an ID. | new-slice | — |
| S9.1 | 88 | requirement | Track the decision LLM's prompt with a version and a date | new-design | — |
| S9.2 | 88 | requirement | make clear which prompt was used for every decision | new-design | — |
| S9.3 | 90 | requirement | tokens used | new-design | — |
| S9.4 | 90 | requirement | percentages | new-design | — |
| S9.5 | 90 | requirement | quota (maybe) | new-design | — |
| S9.6 | 90 | requirement | time to decision | new-design | — |
| S9.12 | 92 | requirement | the invalid-JSON path (§4) and which pool/model finally answered | new-design | — |

### T12 llm-decider-protocol (17)

| id | lines | kind | text | disposition | existing issues |
|---|---|---|---|---|---|
| S2.18 | 32 | requirement | Per provider, the mill must answer with data | research | — |
| S2.19 | 32 | question | is a native decision model worth using for decisions | research | — |
| S2.20 | 32 | question | or are the alternatives cheap enough that asking a cheap LLM (DeepSeek Flash or something) is the better buy | research | — |
| S4.1 | 44 | constraint | Don't use `small`. | new-design | — |
| S4.2 | 44 | requirement | Only Luna small / Luna 100k as the last-resort decision model | new-design | code PR9384 closed (related) |
| S4.3 | 44 | preference | Maybe there is a super-fast free one that doesn't need to be smart at all | new-design | — |
| S4.4 | 44 | constraint | Beam might be too slow *and* too smart | new-design | code #6972 open (related) |
| S4.5 | 44 | preference | consider Beam or one of the really, really fast ones that are just not decision-API fast | new-design | — |
| S4.6 | 45 | requirement | An LLM decider uses up to its context limit without compaction, or less when possible | new-design | — |
| S4.7 | 45 | requirement | to review the input and possibly do a few quick scans of files | new-design | — |
| S4.8 | 45 | requirement | then produces valid JSON matching the schema | new-design | — |
| S4.9 | 46 | requirement | feed back the invalid JSON, the decision schema, the prompt, and suggestions | new-design | — |
| S4.10 | 46 | requirement | the result must be valid JSON after two tries | new-design | — |
| S4.11 | 46 | requirement | Then move on to the next LLM or pool | new-design | — |
| S4.12 | 46 | requirement | if all pool routes are exhausted, set a default answer | new-design | — |
| R10 | packet | rule | is a native decision model worth it vs a cheap LLM (DeepSeek Flash class) | research | — |
| R11 | packet | rule | must be answered with measured data | research | — |

### T13 outcome-feedback-loop (53)

| id | lines | kind | text | disposition | existing issues |
|---|---|---|---|---|---|
| S3.7 | 39 | requirement | Caller and called can each provide points correct / points wrong, 0 to 5, on separate gauges | new-design | — |
| S5.1 | 50 | requirement | Later we compare model outcomes to cost, speed, etc. | new-design | — |
| S5.2 | 50 | requirement | Capture, per decision and per linked work item | new-design | — |
| S5.3 | 52 | requirement | PRs merged | new-design | — |
| S5.4 | 52 | requirement | issues closed/resolved | new-design | — |
| S5.5 | 52 | requirement | issues made | new-design | — |
| S5.6 | 52 | requirement | canceled | new-design | — |
| S5.7 | 52 | requirement | PRs closed incomplete | new-design | — |
| S5.8 | 52 | requirement | duplicate work | new-design | — |
| S5.9 | 52 | requirement | work not pushed | new-design | — |
| S5.10 | 52 | requirement | PRs and commits reverted | new-design | — |
| S5.11 | 52 | requirement | rework | new-design | — |
| S5.12 | 52 | requirement | work undone | new-design | — |
| S5.13 | 52 | requirement | caused incident | new-design | — |
| S5.14 | 52 | requirement | data loss | new-design | — |
| S5.15 | 52 | requirement | invalid | new-design | — |
| S5.16 | 52 | requirement | unclear | new-design | — |
| S5.17 | 52 | requirement | misunderstood | new-design | — |
| S5.18 | 52 | requirement | too-big | new-design | — |
| S5.19 | 52 | requirement | overcomplicated | new-design | — |
| S5.20 | 52 | requirement | oversimplified | new-design | — |
| S5.21 | 52 | requirement | perfect | new-design | — |
| S5.22 | 52 | requirement | simple | new-design | — |
| S5.23 | 52 | requirement | safe | new-design | — |
| S5.24 | 52 | requirement | fixed | new-design | — |
| S5.25 | 52 | requirement | partial | new-design | — |
| S5.26 | 52 | requirement | cheaper than expectation | new-design | — |
| S5.27 | 52 | requirement | unneeded-worry | new-design | — |
| S5.28 | 52 | requirement | overtested | new-design | — |
| S5.29 | 52 | requirement | undertested | new-design | — |
| S5.30 | 52 | requirement | significant expense | new-design | — |
| S5.31 | 52 | requirement | slow | new-design | — |
| S5.32 | 52 | requirement | heavy | new-design | — |
| S5.33 | 52 | requirement | network | new-design | — |
| S5.34 | 52 | requirement | not-scalable | new-design | — |
| S5.35 | 53 | requirement | Every session everywhere must know it can report | new-design | code #9373 open (related); code #9374 open (related) |
| S5.36 | 53 | example | this issue or that PR got undone | new-design | — |
| S5.37 | 53 | example | this issue was misunderstood and caused this PR, which blew a quota and caused perf issues, then an outage | new-design | — |
| S5.38 | 53 | requirement | for every issue and PR | new-design | — |
| S5.39 | 54 | requirement | each issue/PR is linked to a task | new-design | code #8176 closed (related) |
| S5.40 | 54 | requirement | the task is linked to one or more sessions and attempts | new-design | — |
| S5.41 | 54 | requirement | various subsessions, tool calls, token uses, pools and models hit/failed/used | new-design | — |
| S5.42 | 54 | requirement | tokens in, tokens cached, tokens context, user input, thinking, tool calls and reads | new-design | — |
| S5.43 | 54 | requirement | TTFT, total step time, first TTFT, total step count and time for every session and input | new-design | — |
| S5.44 | 54 | requirement | number of question prompts, number of questions, time until submitted | new-design | — |
| S5.45 | 54 | requirement | errors auto-rerouted | new-design | opencode #2223 closed (related) |
| S5.46 | 54 | requirement | the decision model and the decision | new-design | — |
| S5.47 | 54 | requirement | the feedback from PRs and issues, parent sessions and subsessions | new-design | — |
| S9.7 | 91 | requirement | whether the decision resulted in a successful session goal | new-design | — |
| S9.8 | 91 | requirement | from the sub-session's own perspective | new-design | — |
| S9.9 | 91 | requirement | the parent's perspective | new-design | — |
| S9.10 | 91 | requirement | anyone's perspective | new-design | — |
| S9.11 | 91 | requirement | through association with an issue, PR, or epic that was provided a rating | new-design | — |

### T14 decision-effort (19)

| id | lines | kind | text | disposition | existing issues |
|---|---|---|---|---|---|
| S3.8 | 40 | requirement | Decision effort: allow any effort to be set on decisions | new-design | — |
| S3.9 | 40 | requirement | by default it has no impact and is not used | new-design | — |
| S6.1 | 58 | constraint | Effort overlaps two related but technically distinct concepts | new-design | — |
| S6.2 | 60 | constraint | Tie-break lean | new-design | — |
| S6.3 | 60 | constraint | when using a decision API to determine a routing direction | new-design | — |
| S6.4 | 60 | requirement | low/medium/high add a *very slight* boost toward the lowest / closest to the middle (workhorse/scout) / close… | new-design | — |
| S6.5 | 60 | constraint | Not an absolute amount, not a plain multiplication | new-design | — |
| S6.6 | 60 | requirement | An appropriate mechanism slightly prefers one direction | new-design | — |
| S6.7 | 60 | example | e.g. when the two highest results are within some distance of each other | new-design | — |
| S6.8 | 60 | requirement | Low/medium/high might also change which default is used when auto is used | new-design | — |
| S6.9 | 61 | constraint | Fallback pool choice | new-design | — |
| S6.10 | 61 | constraint | the pool to use if all decision models fail | new-design | — |
| S6.11 | 63 | preference | For now, especially for the UI, leave it this way to avoid new widgets | new-design | — |
| S6.12 | 63 | requirement | In the API and on tasks we can distinguish the two | new-design | — |
| S6.13 | 63 | requirement | low/medium/high for quality, speed, and cost as separate axes | new-design | — |
| S6.14 | 63 | question | if they all match, their ratio is identical, so what does changing low→high mean? | research | — |
| S6.15 | 63 | requirement | Working answer to test | new-design | — |
| S6.16 | 63 | requirement | choose the best all-around at a given overall capability band and pass to the appropriate pool without an eff… | new-design | — |
| S6.17 | 63 | preference | Consider extensions supporting such steering for other decision types, dynamically at runtime, or both | new-design | — |

### T15 conditional-clauses (19)

| id | lines | kind | text | disposition | existing issues |
|---|---|---|---|---|---|
| S6.18 | 65 | constraint | a short setup context may hold a decision between 3+ probabilities and another with 2 | new-design | — |
| S6.19 | 65 | constraint | probabilities may change depending on the first question | new-design | — |
| S6.20 | 65 | constraint | the combination of Q1+Q3 determines Q4's value | new-design | — |
| S6.21 | 65 | constraint | Q2+Q4 determine or highly influence Q5 | new-design | — |
| S7.1 | 69 | requirement | On top of the System 1 API, support automatic if-this-then-that | new-design | — |
| S7.2 | 69 | requirement | Every question submitted to the decision model can carry functions and baked-in clause types | new-design | — |
| S7.3 | 71 | example | If 1, 2, 3 are yes/no/yes → switch Q4 to no | new-design | — |
| S7.4 | 71 | example | If yes/no/no → Q4 is yes but Q5 is no | new-design | — |
| S7.5 | 71 | example | If yes/yes/yes → Q4 is "1 and 2", Q5 is yes, Q6 is both | new-design | — |
| S7.6 | 72 | requirement | `if blank then this is X` | new-design | — |
| S7.7 | 72 | requirement | `if this and this and this then slight preference / strong preference for or against each of them` | new-design | — |
| S7.8 | 72 | requirement | optional rankings and other subclasses | new-design | — |
| S7.9 | 73 | example | If this is for 1 and that is for 2, then for Q4, if 3 is yes, strongly boost checking options B and C | new-design | — |
| S7.10 | 74 | example | If deferred or not answered → prefer checking just option 1 | new-design | — |
| S7.11 | 74 | example | If no, or a number greater than 100 → check all the boxes and insert into the make-your-own | new-design | — |
| S7.12 | 76 | requirement | Implementation shape: a template string with access to variables | new-design | — |
| S7.13 | 76 | requirement | all dependent questions, answers, and so on | new-design | — |
| S7.14 | 76 | constraint | This should not live in the systemone endpoint directly | new-design | — |
| S7.15 | 76 | constraint | unless it can be done as a superset that does not affect the real standard in any way | new-design | — |

### T16 pm-review-surface (9)

| id | lines | kind | text | disposition | existing issues |
|---|---|---|---|---|---|
| S8.1 | 80 | requirement | Build review into the PM stack | new-design | code PR9343 closed (related); code #9335 closed (related); code #8766 closed (related) |
| S8.2 | 82 | requirement | Review worked items | new-design | code #9354 closed (related) |
| S8.3 | 82 | requirement | provide feedback, labels, and scores | new-design | code PR9343 closed (related) |
| S8.4 | 82 | requirement | for one item, a model-selected set of items, a set marked as a set, custom selections within a set, multiple … | new-design | — |
| S8.5 | 83 | requirement | Search and review terminal-status items by epic, roadmap, and timeline | new-design | — |
| S8.6 | 83 | requirement | add labels | new-design | — |
| S8.7 | 84 | requirement | Review queued work | new-design | — |
| S8.8 | 84 | requirement | reprioritize it | new-design | — |
| S8.9 | 84 | requirement | add labels | new-design | — |

### T17 cycle-safe-traversal (8)

| id | lines | kind | text | disposition | existing issues |
|---|---|---|---|---|---|
| S10.1 | 96 | requirement | Support any number of cycles in the priority of pools and plans accessed | new-design | code PR9385 closed (partial) |
| S10.2 | 96 | requirement | If a pool has already been seen | new-design | code PR9385 closed (partial) |
| S10.3 | 96 | requirement | it has already been processed through all available options | new-design | code PR9385 closed (partial) |
| S10.4 | 96 | requirement | weighted, round-robin, and ordered, for the entirety of the pool | new-design | — |
| S10.5 | 96 | requirement | so it can be safely skipped | new-design | — |
| S10.6 | 96 | requirement | This prevents cycles and unnecessary error responses from providers we already know are walled or would rejec… | new-design | — |
| S10.7 | 96 | preference | Later, mutual references might be supported | new-design | — |
| S10.8 | 96 | preference | using something like the NixOS module fixpoint concept, or another method | new-design | — |

### T18 standing-rules (10)

| id | lines | kind | text | disposition | existing issues |
|---|---|---|---|---|---|
| R1 | packet | rule | GitHub via gh CLI only | new-design | — |
| R2 | packet | rule | merge commits only | new-design | — |
| R3 | packet | rule | never force-push main | new-design | — |
| R4 | packet | rule | CI runs on main only | new-design | — |
| R5 | packet | rule | secrets by name only | new-design | — |
| R6 | packet | rule | a 100%-walled pool leg is never attempted | new-design | — |
| R7 | packet | rule | parked/abandoned design states are Drew's alone to set | ruling-needed | — |
| R8 | packet | rule | starter issue is an epic under EPIC GUARD | new-design | code #9399 open (partial) |
| R9 | packet | rule | never closed merely because subtasks complete | new-design | code #9399 open (partial) |
| R12 | packet | rule | _commandcode seats are suspect | new-design | code #9397 closed (related) |

## Compound statements split

Bullets joined by `;`, `,` clauses that carry separate requirements, or lists were split. The rule: one statement per
requirement, preference, example, constraint or fact. Lists of named items (outcome labels, pool shapes, quoted
examples) are kept as one statement per item only where each item has its own meaning. The main splits:

- brief §1 bullet 2: "wire up clef jev …" plus the bracketed orchestrator note (note kept as a `question`, not as Drew's words).
- brief §1 bullet 1: "Set appropriate pools for all OpenChamber categories" kept separate from the auto clauses.
- brief §1 bullet 3: "Ensure auto works" and "categories on auto model selection must actually run" are two statements.
- brief §1 bullet 4: "Make appropriate customizations; ensure it's wired well" is two statements.
- brief §2 bullet 1: "Allow a model to suggest/switch its own pool, but only to …": the permission and the restriction are separate.
- brief §2 bullet 2: the "guidance on" lead-in is shared by "when not to use auto" and "when to change its own model".
- brief §2 bullet 3: "By default use auto" and the fallback chain are separate; the small-model set is its own statement (A2).
- brief §4 bullet 1: "low/medium/high at least" parenthetical is its own statement.
- brief §4 bullet 4: the three overlay operations (set, remove, replace) are three statements.
- brief §4 bullet 7: the three cross-pool examples and the "other factors dominate" clause are separate.
- brief §5: the data list is one statement per item (provider/model/effort, pool, ratio, sub-sessions, tool usage, mcpx/fleet per-subcommand, diff, attempts, triages, days, success rate, quota %). The statistics list is one statement per statistic group (min…sum, p90–p99.9, quartiles).
- brief §6: the report table, the event fields, the variant-split rule, and each example of the 1107 / Luna / ds-plan ordering are separate.
- brief §7: the low-tier list is one statement per named preference (DeepSeek off-peak, Luna 100k, OpenAI 388k, free, lower effort).
- brief §8: the testing plan, the successor replacement, the DS Flash rule, the efficiency rule and the Beam rule are separate.
- brief §9: each V1 fact is one `fact-v1` statement (route classes, `limited` pool, DeepSeek trial, subsession pins, Haiku, Luna trial pattern, analytics, roadmap pipeline, time-of-day, fleet ledgers, AA data, claim stamping).
- supplement §2: the two pool taxonomies are separate statements from the build-up rule; the Cloudflare clause is split into tracking, use, disengage, and pivot.
- supplement §5: each outcome label is one `requirement` statement (32 labels, S5.3–S5.34, from "PRs merged" to "not-scalable"); "etc." is not a statement.
- supplement §6: the tie-break lean, the fallback pool choice, the separate-axes question, the working answer and the dependent-probability examples are separate.
- supplement §7: each worked example (yes/no/yes, yes/no/no, yes/yes/yes, blank, deferred, >100) is its own statement.
- supplement §8: the review scope (single, set, custom, multiple, standalone) is one statement; each review surface (items, terminal-status by epic/roadmap/timeline, queued work) is its own.
- supplement §9: each record field (tokens, percentages, quota, time) and each perspective (sub-session, parent, anyone) is its own statement.
- supplement §10: the cycle-skip rule, the "already processed" premise, the skip-safety clause, and the future mutual-references idea are separate.
- launch packet: the standing rules are split one per rule; the `_commandcode` paragraph is split into the rule and the three facts plus the PR #9401 fact.

## Ambiguous readings (chosen reading)

- **A1 — "clef jev opencodego free then paid then cloudflare" (B1.2).** Read as: wire the decision models (clef, jev, and the OpenCode Go free ones) free first, then paid, then Cloudflare, following the supplement §2 order (free, then plan, then PAYG, Cloudflare's included usage burned first among metered options). It maps to the decision pool (T10), not to an OpenChamber category name. The supplement §0 decodes clef and jev as decision models on the v1/systemone contract, which settles the phrase. The "opencodego free" reading is OpenCode Zen's free Jev door (questions.md Q5), not a Go plan model.
- **A2 — "small, scout, speed as the small-model options (or so)" (B2.9).** The "(or so)" softens the set. Kept as one requirement with the soft wording in the note; disposition `new-slice`, not a hard list.
- **A3 — orchestrator notes in brief §1 (bracketed).** Two statements (B1.3, B1.4) are the orchestrator's words, not Drew's. Kept as `question` so that the design does not treat them as Drew's requirements.
- **A4 — who serves Jev (S0.4).** The supplement says Jev is Command Code's decision model. The V1 audit (questions.md Q5) and the code show Jev on Zen, TypeSafe, OpenRouter and Vercel, not `_commandcode`; the standing rules say `_commandcode` seats are suspect. Kept verbatim with disposition `ruling-needed`.
- **A5 — the 1107 variant and the Flash original (B6.30).** Read as an open question about provider variants, not a pool list; the ds-plan ordering (B6.31) is an example, not a mandate.
- **A6 — the Cloudflare "$10 (or whatever)" (S2.12).** The number is Drew's hedge. Read as a tracked included allowance with the cap configurable. Questions.md Q3 defaults it to 10,000 Workers AI neurons per day. Disposition `new-slice`.
- **A7 — effort is two concepts (S6.1–S6.3).** Tie-break lean and fallback pool choice are separate statements (T14), per supplement §6. The UI keeps one control; the API and tasks keep both.
- **A8 — "xhigh / max / ultra / fast all point to high" (B4.9).** Kept as an interim requirement (`new-slice`); the brief says "until implemented", so it is temporary.
- **A9 — brief §9 states opencode#821 is unsupported.** The brief says "Windows have no calendar/holiday support yet (fork issue opencode#821)", but `gh` shows opencode#821 CLOSED ("date-aware route windows … holiday exclusions"). Recorded as-is on B9.34 with the conflict in its note. Not resolved: whether #821 shipped is for the design to verify, not for this inventory.
- **A10 — launch-packet rules have no repo file.** The standing rules (R1–R12) come from the launch packet, which is not committed. The checker verifies them against an embedded copy of the packet paragraph (`LAUNCH_PACKET` in `check_inventory.py`). The text is verbatim from the launch packet; a repo file would be a better source.
- **A11 — "rule" vs "fact-v1" in the packet.** Standing rules are `rule`; packet statements that describe the V1 world (Command Code coverage lost, Beam tier ended, credits exhausted, PR #9401) are `fact-v1` (T0). Both sit under the R-prefix as the packet's own sections.
- **A12 — the outcome labels (S5.x).** Each label is a `requirement` (capture every label). Labels with a plain meaning (for example "simple", "safe") are not further defined by the brief; the taxonomy is open.

## Existing-issue coverage

Search method: `gh search` is rate-limited in this session (HTTP 403, 12:07 CT), so coverage was taken from `gh issue list` and `gh pr list` in three repos, with `--state all --limit 1200` (opencode fork 1,186 issues and PRs, openchamber 800, code repo 1,200 issues and 1,200 PRs at the cap, so older code-repo items may be missing), filtered by keyword in title. Each anchor was then checked with `gh api repos/<repo>/issues/<n>` for the current title and state. Anchors listed here were seen in gh output; nothing was taken from memory.

- Statements with at least one existing issue or PR: 62 of 366.
- Coverage marks: full 14, partial 16, related 42 (counted per statement-to-issue link).
- Distinct issues/PRs referenced: 37.
- **Not covered by any existing issue or PR (new design or new slice):** the decision-endpoint and System 1 conditional-clause layer (opencode#1146 is the only façade anchor), the decision executor and its ID/gauge record, the LLM-decider protocol, the outcome/feedback loop (only fleet ledgers code#9373/#9374 are adjacent), the granular event stats store (the roadmap-stats pipeline is the base, no issue found), effort overlays and the low/medium/high mapping, the context-limit rule, the cycle-safe traversal (code#9385 is partial: it has the visited set but not the decision-pool reuse), and the PM-stack review surface (code#9343 / #9335 / #8766 are the decision-set base, not the review of worked items).

| Issue / PR | State | Coverage marks | Statements |
|---|---|---|---|
| openchamber #14 — feat(routing): interactive model tier chooser and quota fallback card via render.page awaitAnswer | OPEN | related 1 | B1.1 |
| opencode #821 — feat(routes): date-aware route windows — specific dates / holiday exclusions (DeepSeek peak suspends on Chinese public holidays) | CLOSED | related 1 | B9.34 |
| opencode #1146 — Design: generic OpenAI façades on the server — /v1/chat/completions + /v1/responses, one translation layer, proxy doors retire | OPEN | partial 5 | S0.9, S0.10, S0.11, S0.12, S1.1 |
| opencode #1410 — Door front migration: clef-proxy (18742) callers repoint to the façade | CLOSED | related 1 | S0.13 |
| opencode #1651 — fix(providers): cloudflare-ai-gateway credential fails upstream auth (code 10000), blocking clef classifications | CLOSED | related 1 | S0.7 |
| opencode #1881 — Workshop test: subsession machinery from a workhorse session — results and system-prompt suggestions | CLOSED | related 1 | B2.3 |
| opencode #1887 — docs/prompts: base system prompt is silent on the entire subsession machinery | CLOSED | related 1 | B2.6 |
| opencode #1889 — docs: `subagent` tool description omits inheritance, background semantics, and the question-stall warning | CLOSED | related 1 | B2.6 |
| opencode #2203 — fix(ai): v1-dialect reasoning_effort silently dropped by native providers other than muse-direct — fork-wide follow-up flagged on #1825, never split | CLOSED | related 1 | B4.9 |
| opencode #2223 — fix(core): route_usage lastServing reports a hedged race won by the hedge leg as a failover | CLOSED | related 1 | S5.45 |
| opencode #2384 — feat(core): agent-side tier chooser + quota fallback card wiring (split from openchamber#14) | OPEN | related 1 | B2.1 |
| code PR5583 — Retire the clef door (opencode#1410) | CLOSED | related 1 | S0.13 |
| code #6972 — Flag: right-size the Beam seat in beam-plan / scout — AA II sub-20 on two consecutive pulls (17.7 → 17.73) | OPEN | related 3 | B7.8, B8.17, S4.4 |
| code #8163 — _openai provider: self-contained main OpenAI door that does it all without Codex (Drew top priority) | OPEN | related 1 | S2.16 |
| code #8176 — queue: stamp the claiming session's own model on the task at claim time | CLOSED | full 1, related 1 | B9.41, S5.39 |
| code #8211 — aggregator(aa-explorer): cached historical AA batches freeze the v_current_prices overlay — version switching silently switches token rates | CLOSED | related 2 | B4.5, B5.2 |
| code PR8297 — feat(aggregator): versioned project/epic spend rollup store (#4823) | CLOSED | related 2 | B6.1, B6.2 |
| code PR8373 — feat(pm-stack): flow analytics computation + JSON twin (#4829) | CLOSED | related 1 | B6.1 |
| code #8766 — Decisions page: batched decision sets — one push per set, per-item/partial submit, defer-to-orchestrator with leans (extends #8568) | CLOSED | related 1 | S8.1 |
| code #8807 — Grounded search plane: AI Gateway billing decision — load ~$10 credits vs BYOK vs hold | OPEN | related 1 | S2.12 |
| code #8820 — Cloudflare credential architecture — scoped tokens + Bitwarden backup (shipped — tracking home) | OPEN | related 1 | S2.12 |
| code PR9018 — opencode config: workhorse Luna seat round-robins medium/high via luna-effort-rr (Drew 2026-11-12) | CLOSED | full 1, related 3 | B4.1, B8.1, B8.19, B9.23 |
| code #9335 — pm-stack decision sets v2: rich questions, event log, submit API + activity webhook/hook | CLOSED | related 1 | S8.1 |
| code PR9343 — feat(pm-aggregator): decision sets v2 — rich schema, event log + outbox, submit API (#9335) | CLOSED | related 2 | S8.1, S8.3 |
| code #9354 — pm-stack: surface the decision-set question stack inside Today/Needs-you + nav — currently only a standalone /decisions page | CLOSED | related 1 | S8.2 |
| code PR9363 — feat(pm-aggregator): quota analytics & projection section | CLOSED | full 1, related 2 | B5.15, B6.8, B9.25 |
| code #9373 — fleet CLI: append-only invocation ledger (subcommand use is inference-only today) | OPEN | partial 3, related 1 | B5.9, B9.36, B9.38, S5.35 |
| code #9374 — mcpx daemon: durable call ledger (direct daemon calls are invisible to analytics) | OPEN | partial 3, related 1 | B5.9, B9.37, B9.38, S5.35 |
| code #9378 — Time-of-day aware routing: peak/off-peak pool, model, and effort selection | OPEN | full 1 | B9.31 |
| code PR9381 — Workhorse DeepSeek effort trial: deepseek-effort-rr (high/low 3:1) in workhorse-ordered | CLOSED | full 2, related 2 | B4.3, B8.1, B9.15, B9.16 |
| code PR9382 — Subsession routing: explore agent pinned to scout route | CLOSED | full 1 | B9.19 |
| code PR9384 — Limited pool: rebuild small on per-member plan limits, delete small-100k | CLOSED | full 1, related 1 | B9.6, S4.2 |
| code PR9385 — Class fallback chains (second-pass escalation, visited-set semantics) + guard extension | CLOSED | full 4, partial 3 | B9.1, B9.2, B9.3, B9.4, S10.1, S10.2, S10.3 |
| code PR9388 — config(routes): workhorse-last-resort head Sonnet #medium → Haiku #high — land Drew's 2026-11-12 edit (primary reconcile) | CLOSED | full 1 | B9.22 |
| code #9397 — Fleet-stalling: lane route wall 2026-11-13 — sessions start, models never answer (MODEL_NOT_IN_PLAN gpt-6.1-sol 403; Claude failover rate-limited 429) | CLOSED | related 1 | R12 |
| code #9399 — Program: pool & effort system — effort tiers per pool, OpenChamber category pools, granular stats store (design mill) | OPEN | partial 2, related 2 | B1.2, S2.6, R8, R9 |
| code PR9401 — Remove dead _commandcode doors from all plan pools (code#9397) | CLOSED | full 1, related 1 | R13, R16 |

Notes on specific anchors:

- **code#9399** (the epic) is open again. questions.md Q1 records a bot closure and a reopen. The EPIC GUARD applies.
- **opencode#821** is CLOSED ("date-aware route windows"); the brief says the gap is open (A9).
- **opencode#1651** (clef door) is CLOSED, consistent with questions.md Q5 ("won't-do"). Coverage of the clef leg is `related`, not `full`.
- **code#9401** (PR, MERGED) removed `_commandcode` doors from all plan pools; it is the anchor for the R-facts.
- **code#9397** is CLOSED: the 2026-11-13 lane route wall. It is `related` evidence for the walled-leg rule.
- **code#8176** is CLOSED and is the claim-model stamping fix used as a V1 fact (B9.41).
- **code#9018** and **code#9381 / #9382 / #9384 / #9385 / #9388** are MERGED V1 facts (T0), `full`.

## Open questions (defaults chosen)

- Launch-packet standing rules have no repo file; the checker verifies them against an embedded copy (A10). Default: keep the embedded copy and recommend that the packet be committed.
- A4: Jev's home (Command Code vs Zen/TypeSafe). Default: follow the code (questions.md Q5); mark the Command Code reading `ruling-needed` for Drew.
- A9: opencode#821 status. Default: record the conflict; the design verifies whether holiday windows shipped.
- The `"or so"` in B2.9 and the outcome-label list in S5.x are soft. Default: keep them as statements with the softness in the note.

