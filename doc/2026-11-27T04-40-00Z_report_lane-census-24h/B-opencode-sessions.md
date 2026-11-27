# Census B — OpenCode sessions on the laptop, 24h

- **Window:** 2026-11-26T04:10Z → 2026-11-27T04:10Z (Thu 2026-11-26 22:10 CST ± 24h)
- **Source:** `~/.local/share/opencode/opencode.db` on laptop `amd` (273 MB, live — modified during the census), read via `~/tools/laptop-ssh.sh` + sqlite3.
- **Store shape:** the legacy `session` table is **stale** (515 rows, newest `time_created` 2026-10-05). All live data is in **`session_v2`** (39,349 rows). Smaller `opencode.db` copies exist in snapshots/config/canary dirs and hold no live sessions. Outcome evidence comes from `session_v2.idle_outcome` plus the terminal `idle` rows in `session_message` (which carry structured `error.type` / `error.message`), and per-attempt routing records in `route_attempt`.
- **Live-store caveat:** sessions were being written while the census ran; counts from different queries drift by 1–2. Figures below are as read 2026-11-27T04:11–04:35Z.

## Headline totals

| Measure | Count |
|---|---|
| Sessions created in window | **1,603** (1,208 top-level + 395 child/subagent) |
| Sessions updated in window | 1,626 |
| Tokens (as stored) | ~956M input / ~9M output |
| Recorded cost | **0 everywhere** — the route provider does not write cost into the store |
| Route attempts (`route_attempt`) | 81,978 attempts across 1,662 distinct sessions: success 64,329 (78.5%), **timeout 15,000 (18.3%)**, failure 2,619, interrupted 30 |

**The hole at the start of the window:** zero sessions were created between 04:10Z and 09:00Z (~4h50m). Creation resumes at 09:00Z and runs at a steady 72–106/hour for the rest of the window. The hole matches the laptop/tailnet outage period; the session engine was simply not running (or not reachable) for the first quarter of the window.

Hourly creations (Z): 09:00 24 · 10:00 75 · 11:00 77 · 12:00 85 · 13:00 90 · 14:00 100 · 15:00 100 · 16:00 106 · 17:00 88 · 18:00 98 · 19:00 95 · 20:00 87 · 21:00 84 · 22:00 88 · 23:00 80 · 00:00 72 · 01:00 74 · 02:00 81 · 03:00 80 · 04:00 19 (partial hour).

## Where sessions ran (directory)

| Directory class | Sessions | Top-level |
|---|---|---|
| `/home/user/git/code` — the shared primary clone | 1,399 (87%) | 1,112 |
| `/home/user` (home dir, interactive/misc) | 76 | 38 |
| Fleet worktree dirs under `~/git/fleet/` (oc-code / code-* / agent-next / task dirs, ~71 distinct) | ~99 | ~42 |
| Repo `.worktrees` (openchamber/nix forks) + other clones + other | ~29 | ~16 |

87% of all sessions share the one primary clone directory — the lane worktrees exist but hold only a small minority of sessions; most "lane" sessions run in the primary clone and differentiate by title/claim, not by directory.

## Agent + model mix

- **Agent:** build 1,177 · explore 231 · general 146 · (none) 44 · review-changes 5.
- **Model (route alias, as stored):** `opencode-route/workhorse` 1,396 (+43 `variant:default`, 1 `variant:low`, 1 workhorse-weighted, 1 workhorse-ordered) · `opencode-route/scout` 140 · speed 2 · openrouter-free-plan 2 · groq-free-plan 2 · kilo-free-plan 2 · (none) 9. Effectively everything rides the one `workhorse` route head; the underlying per-attempt providers are in `route_attempt` (below).

## Named series (top-level sessions, by title)

| Series | n | Succeeded | Failed | No terminal record |
|---|---|---|---|---|
| Queue lane (scheduled) — oc-code-*, code-*, any-* lanes | 585 | 296 | **264 (45%)** | 25 |
| Opencode lane claimant/executor — #3026 | 212 | 170 | 42 (20%) | 0 |
| Main CI failure — fix trigger | 108 | 63 | 44 | 1 |
| Improvement issue creator | 73 | 47 | 23 | 3 |
| Fleet conductor — session sweep + dispatch | 56 | 39 | 17 | 0 |
| Quota gatherer guardian | 55 | 46 | 9 | 0 |
| quota-freshness-summary-* | 20 | 18 | 2 | 0 |
| Builder's advocate | 19 | 12 | 6 | 1 |
| Plans Space freshness keeper | 18 | 13 | 5 | 0 |
| Orchestration pass | 17 | 12 | 5 | 0 |
| Issue sweep | 10 | 6 | 3 | 1 |
| One-offs (OK-reply probes, Bolt/Palette dailies, single task sessions, reviews) | ~35 | most | few | 0 |

Notes:
- **The conductor is alive.** A prior record (2026-11-24) said the conductor series stopped being launched on 2026-11-24T04:25Z. The store contradicts that for this window: 56 "Fleet conductor — session sweep + dispatch" sessions ran all day, the most recent created 2026-11-27T04:00Z (22:00 CST) and succeeded. Whatever stopped on Nov 24, a conductor series is running now.
- The **claimant/executor #3026** series is the healthiest lane shape (80% success, zero left open) — short, frequent, single-purpose.
- The **scheduled queue lanes are the failure mass**: 585 sessions, 45% failed, and they also hold the largest token burns (below).

## Categorization

Rule (store evidence only): category = the session's terminal `idle_outcome` in `session_v2`; failure causes = the structured `error.type` on its terminal `idle` rows in `session_message`. The v2 store does **not** populate `summary_files` / `summary_additions` / `summary_deletions` (all zero for all 1,603 sessions), and merges land via `gh` outside the store — so the store **cannot prove a merged PR per session**. "Landed" is therefore reported as *completed (terminal success)*, with sampled in-depth evidence that succeeded lanes run claim→shepherd→merge flows; a true landed-vs-not split needs the GitHub/queue join (Census A/C territory), not this store.

| Category | All sessions | Top-level only |
|---|---|---|
| Completed — terminal record `succeeded` | **1,036 (64.6%)** | 747 (61.8%) |
| Errored — terminal record `failed` | **530 (33.1%)** | 429 (35.5%) |
| Interrupted — terminal record `interrupted` | 4 (0.2%) | 0 (all 4 are children) |
| No terminal record (`idle_outcome` NULL) | 33 (2.1%) | 32 |
| — of which **suspended by design** (`time_suspended` set) | 31 of 32 top-level | — |
| — still active at window close (updated after 04:10Z) | most of the remainder | — |
| — genuinely evaporated candidates (not suspended, stale update) | **~1** | — |

So the "stalled/evaporated" bucket is nearly empty: the fork's idle-suspend mechanism parks unfinished sessions (`time_suspended` set on 38 window sessions; 18 show `resume_attempts > 0`, i.e. they were woken and resumed). Sessions do not silently vanish — they fail loudly (530) or get suspended (31).

## Why the failed sessions failed

Terminal `idle` events for failed sessions created in the window (576 events over 530 sessions — some sessions idle-failed more than once):

| error.type | Events | Share |
|---|---|---|
| `provider.invalid-request` | **500** | 87% |
| `aborted` | 35 | 6% |
| (no error type recorded) | 24 | 4% |
| `provider.rate-limit` | 4 | |
| `provider.no-route` | 3 | |
| `compaction.failed` | 3 | |
| `unknown` | 2 | |
| `provider.quota` | 2 | |
| `provider.transport` / `provider.internal` / `provider.content-filter` | 1 each | |

**Cause 1 — "model not found" (the overwhelming one).** 552 of the 555 `provider.invalid-request` idle events recorded anywhere in the window carry the identical message:

> "The requested model was not found. It may not exist, or this API key may not have access to it. Check the model name; GET /v1/models lists the models this API k…"

`route_attempt` pins it to one door: **every** attempt whose error message matches "not found" is `_muse` provider, model `muse-wonder-1.0-contributor` — **912 attempts**. The workhorse route kept offering sessions to a door that does not carry the model it asked for; after the route's retries, the session idled as failed. This is a route/catalog mismatch, not provider downtime: the same `_muse` door served `muse-spark-1.3-contributor` attempts (which failed separately, at transport level — below).

**Cause 2 — aborts after repeated interruption.** All sampled `aborted` idle events carry:

> "Execution was interrupted repeatedly and will not be resumed automatically."

i.e. the session was interrupted (steer/stop/transport drop) enough times that the fork gave up on it. The single largest session of the window died exactly this way (below).

**Cause 3 — the attempt-level weather: transport failures and timeouts.** Sessions mostly survive individual attempt failures (the route fails over), but the attempt ledger shows how bad the day was: 15,000 of 81,978 attempts (18.3%) ended `timeout`, and the top failure combos are transport-level:

| Provider / model | error_tag | Attempts |
|---|---|---|
| _openai.main / gpt-6-luna | Transport | 5,681 |
| _muse / muse-spark-1.3-contributor | Transport | 2,679 |
| _openai.main / gpt-6.1-sol | Transport | 2,571 |
| openai-payg-free / gpt-6-luna | Transport | 1,077 |
| openai-payg / gpt-6-luna | Transport | 941 |
| _muse / muse-wonder-1.0-contributor | InvalidRequest | 914 |
| _openai.main / gpt-6.1-terra | Transport | 877 |
| exaone / k-exaone_v2.5 | RateLimit | 154 |
| google (3 doors) / gemini-4-hydrogen | QuotaExceeded | ~373 |
| _claude / claude-haiku-5-5 | Transport | 131 |

This matches the day's known conditions: Claude Pro walled at 100% of its 5h window, OpenAI alt (popesdrew) walled at 100% weekly, and the laptop broker tunnel dropping repeatedly in the evening. The router kept working — 64,329 attempts succeeded — but it burned enormous retry volume to do it.

**Minor causes:** two `provider.quota` terminal failures, four `provider.rate-limit`, three `provider.no-route` ("Variant unavailable for opencode-route/workhorse: low" — the `low` variant had no live door at those moments), three `compaction.failed`. One `invalid-request` outlier was a request-too-large TPM rejection on `openai/gpt-oss-20b`/`gpt-oss-120b`, and one a tool-name pattern rejection (`` `name` must match ^[a-zA-Z0-9_.-]+$ ``).

## In-depth samples

**Largest sessions by tokens** — five of the top eight are *failed queue lanes* that burned 10–44M input tokens before dying:

| Tokens (in+out) | Outcome | Session |
|---|---|---|
| 44,185k | failed | Queue lane (scheduled) - oc-code-07, 2026-11-26 14:58 CST |
| 40,094k | failed | Queue lane (scheduled) - code-i, 16:00 CST |
| 30,918k | failed | Queue lane (scheduled) - code-h, 15:00 CST |
| 15,546k | failed | Queue lane (scheduled) - oc-code-08, 16:28 CST |
| 12,883k | succeeded | Queue lane (scheduled) - oc-code-01, 16:02 CST |
| 12,259k | (open) | Queue lane (scheduled) - code-g, 19:40 CST |
| 11,972k | failed | Queue lane (scheduled) - code-g, 16:40 CST |
| 10,576k | failed | Queue lane (scheduled) - code-f, 16:00 CST |

**The 44M-token lane (oc-code-07, ses_e5de6595bd4ffeKhq7lKO3BBG2):** its idle history shows it completed one full work cycle successfully (idle `succeeded` at 2026-11-26T22:35Z store time) and later terminated `failed` / `aborted` — *"Execution was interrupted repeatedly and will not be resumed automatically."* Long-lived scheduled lanes accumulate context until interruption, not model error, kills them.

**Largest succeeded lane (oc-code-01, 16:02 CST):** sampled in depth — its `session_part` rows show real queue work: 118 parts referencing `pull/`, including `gh pr view 15523 --repo developing-today/code` and protocol §8 shepherd/salvage searches. Succeeded claimant/lane sessions do run the claim→shepherd→merge flow; the merge itself is recorded on GitHub, not in this store.

**Most recent hour:** the pattern at window close is unchanged — fresh queue lanes (oc-code-01…08, code-f…j, any-n/o) spin up on schedule, claimant/executor #3026 sessions complete in 3–7 minutes each, and a residue of just-created lanes sits open (suspended or still running) rather than failed.

## Appendix — 60 most recent top-level sessions (created 03:10–04:07Z, 2026-11-27)

| Created | Last update | Outcome | Title |
|---|---|---|---|
| 04:07Z | 04:14Z | — | Issue sweep — developing-today/code 2026-11-26 22:07 |
| 04:06Z | 04:14Z | — | Queue lane (scheduled) - oc-code-02 2026-11-26 22:06 |
| 04:05Z | 04:14Z | succeeded | Opencode lane claimant/executor - #3026 2026-11-26 22:05 |
| 04:02Z | 04:14Z | — | Queue lane (scheduled) - oc-code-01 2026-11-26 22:02 |
| 04:00Z | 04:03Z | succeeded | quota-freshness-summary-20261127T040059Z |
| 04:00Z | 04:07Z | succeeded | Opencode lane claimant/executor - #3026 2026-11-26 22:00 |
| 04:00Z | 04:15Z | — | Queue lane (scheduled) - code-h 2026-11-26 22:00 |
| 04:00Z | 04:14Z | — | Queue lane (scheduled) - code-g 2026-11-26 22:00 |
| 04:00Z | 04:15Z | — | Queue lane (scheduled) - code-f 2026-11-26 22:00 |
| 04:00Z | 04:14Z | succeeded | Fleet conductor — session sweep + dispatch 2026-11-26 22:00 |
| 04:00Z | 04:15Z | — | Queue lane (scheduled) - code-j 2026-11-26 22:00 |
| 04:00Z | 04:14Z | — | Queue lane (scheduled) - code-i 2026-11-26 22:00 |
| 04:00Z | 04:15Z | — | Builder's advocate — inside the fleet 2026-11-26 22:00 |
| 04:00Z | 04:04Z | succeeded | Quota gatherer guardian — keep the quota record fresh 2026-11-26 22:00 |
| 04:00Z | 04:15Z | — | Improvement issue creator — lane B 2026-11-26 22:00 |
| 04:00Z | 04:11Z | — | Queue lane (scheduled) - not-code-d 2026-11-26 22:00 |
| 04:00Z | 04:15Z | — | Main CI failure — fix trigger 2026-11-26 22:00 |
| 04:00Z | 04:14Z | — | Queue lane (scheduled) - any-o 2026-11-26 22:00 |
| 03:58Z | 04:15Z | — | Queue lane (scheduled) - oc-code-07 2026-11-26 21:58 |
| 03:55Z | 04:02Z | succeeded | Opencode lane claimant/executor - #3026 2026-11-26 21:55 |
| 03:54Z | 04:14Z | — | Queue lane (scheduled) - oc-code-06 2026-11-26 21:54 |
| 03:50Z | 03:56Z | succeeded | Opencode lane claimant/executor - #3026 2026-11-26 21:50 |
| 03:50Z | 04:14Z | — | Queue lane (scheduled) - oc-code-05 2026-11-26 21:50 |
| 03:50Z | 03:56Z | succeeded | Main CI failure — fix trigger 2026-11-26 21:50 |
| 03:46Z | 04:14Z | — | Queue lane (scheduled) - oc-code-04 2026-11-26 21:46 |
| 03:45Z | 03:48Z | succeeded | Opencode lane claimant/executor - #3026 2026-11-26 21:45 |
| 03:45Z | 04:15Z | — | Improvement issue creator 2026-11-26 21:45 |
| 03:40Z | 03:46Z | succeeded | Main CI failure — fix trigger 2026-11-26 21:40 |
| 03:40Z | 03:47Z | succeeded | Queue lane (scheduled) - code-j 2026-11-26 21:40 |
| 03:40Z | 03:48Z | succeeded | Queue lane (scheduled) - any-o 2026-11-26 21:40 |
| 03:40Z | 03:47Z | succeeded | Fleet conductor — session sweep + dispatch 2026-11-26 21:40 |
| 03:40Z | 04:14Z | — | Queue lane (scheduled) - any-n 2026-11-26 21:40 |
| 03:40Z | 04:15Z | succeeded | Queue lane (scheduled) - code-h 2026-11-26 21:40 |
| 03:40Z | 03:42Z | succeeded | Quota gatherer guardian — keep the quota record fresh 2026-11-26 21:40 |
| 03:40Z | 03:49Z | succeeded | Queue lane (scheduled) - code-f 2026-11-26 21:40 |
| 03:38Z | 04:13Z | — | Queue lane (scheduled) - oc-code-08 2026-11-26 21:38 |
| 03:36Z | 04:15Z | — | Queue lane (scheduled) - oc-code-02 2026-11-26 21:36 |
| 03:35Z | 03:39Z | succeeded | Opencode lane claimant/executor - #3026 2026-11-26 21:35 |
| 03:32Z | 03:41Z | succeeded | (untitled) |
| 03:32Z | 03:44Z | succeeded | Queue lane (scheduled) - oc-code-01 2026-11-26 21:32 |
| 03:30Z | 03:35Z | succeeded | Main CI failure — fix trigger 2026-11-26 21:30 |
| 03:30Z | 03:41Z | succeeded | Improvement issue creator — lane B 2026-11-26 21:30 |
| 03:30Z | 03:36Z | succeeded | Opencode lane claimant/executor - #3026 2026-11-26 21:30 |
| 03:28Z | 04:01Z | succeeded | Queue lane (scheduled) - oc-code-08 2026-11-26 21:28 |
| 03:26Z | 03:33Z | succeeded | Queue lane (scheduled) - oc-code-07 2026-11-26 21:26 |
| 03:25Z | 03:33Z | succeeded | Opencode lane claimant/executor - #3026 2026-11-26 21:25 |
| 03:23Z | 03:27Z | succeeded | Plans Space freshness keeper — complete + fresh quota picture 2026-11-26 21:23 |
| 03:22Z | 03:41Z | succeeded | Queue lane (scheduled) - oc-code-06 2026-11-26 21:22 |
| 03:20Z | 03:48Z | succeeded | Queue lane (scheduled) - any-n 2026-11-26 21:20 |
| 03:20Z | 03:33Z | succeeded | Fleet conductor — session sweep + dispatch 2026-11-26 21:20 |
| 03:20Z | 03:33Z | succeeded | Main CI failure — fix trigger 2026-11-26 21:20 |
| 03:20Z | 03:26Z | succeeded | Opencode lane claimant/executor - #3026 2026-11-26 21:20 |
| 03:20Z | 03:21Z | succeeded | Quota gatherer guardian — keep the quota record fresh 2026-11-26 21:20 |
| 03:18Z | 03:45Z | succeeded | Queue lane (scheduled) - oc-code-05 2026-11-26 21:18 |
| 03:15Z | 03:18Z | succeeded | Opencode lane claimant/executor - #3026 2026-11-26 21:15 |
| 03:15Z | 03:33Z | succeeded | Improvement issue creator 2026-11-26 21:15 |
| 03:14Z | 03:51Z | — | Queue lane (scheduled) - oc-code-04 2026-11-26 21:14 |
| 03:12Z | 03:41Z | succeeded | Orchestration pass — queue triage, tiers, Turso publish 2026-11-26 21:12 |
| 03:10Z | 03:47Z | succeeded | Queue lane (scheduled) - oc-code-03 2026-11-26 21:10 |
| 03:10Z | 03:29Z | succeeded | Main CI failure — fix trigger 2026-11-26 21:10 |

("—" = no terminal outcome yet at pull time; those sessions were created in the window's last minutes and were still running or freshly suspended.)
