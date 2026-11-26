# Muse→OpenCode cron wrapper inventory — dispositions and wave order

Inventory of record for developing-today/code#14222, the inventory slice of
developing-today/code#2174. Compiled 2026-11-26 by a felix lane (Muse
sandbox seat). Migration itself is out of scope here; each **migrate** row
names its follow-on issue, where the porting work lives.

A **wrapper** is a Muse cron job whose runs launch, shepherd, or stand in
for OpenCode work — the set #2174's migration program covers. Dispositions
use #2174's vocabulary: **migrate** (to an OpenChamber scheduled task),
**keep** (stays on Muse cron, with a named reason), **retire**.

## Sources, and what they cannot prove

Named honestly, per the inventory's completeness risk (wrapper ground truth
spans seats):

| Source | What it establishes | Limit |
|---|---|---|
| The 2026-10-29 study, `~/workspace/research_notes/opencode-scheduled-tasks-20261029/findings.md` + `rollout.md` (seat copy in hatch-personal) | The wrapper families, what each does, per-family Claude input-token measurements (trailing 24h, runtime-DB attribution, measured 2026-10-29), native-fit analysis, the wave ranking, the keep list | Measurements are the study's, not re-measured in this pass; the study predates several jobs below |
| `doc/fleet-schedule-registry.md` (this repo, current main) | Job ids, cadences, enabled/disabled states, owners, definition pointers for Surface 1 (Muse cron) and Surface 2a (OpenChamber scheduled tasks) | Last full live census 2026-11-03 (131 jobs); later amendments are partial. States below are "as recorded", not a fresh live read |
| Cron definition projections under `~/workspace/goals/keep-the-four-repos-healthy/crons/` and `~/workspace/cron.d/` (hatch-personal, this seat's copy) | The newest lane definitions visible from this seat (e.g. the 33 shift-lane files) | Projections are read-only copies; some registry-pointed files (dispatcher/babysitter/conductor, creators) are absent from this seat's copy — those rows rest on the registry + study + the goal's `hidden_files/` instruction files, which are present |
| This seat's live `cron.list` (2026-11-25) | Which jobs run on Felix's sandbox seat: `main-ci-failure-fix-trigger` among them | One seat only. **The primary seat's live scheduler and the laptop's OpenChamber stores were not readable from this seat** (laptop amd-6 unreachable, incident #15131) — per-job live enabled state on the primary seat is unverifiable here and is marked as recorded-state throughout |

## The inventory

Token figures are the study's trailing-24h Muse **input** tokens measured
2026-10-29, quoted for sizing only.

| # | Wrapper (Muse cron job id) | Cadence | State at inventory (recorded) | What it does with OpenCode | Study tokens /24h | Disposition | Wave | Follow-on |
|---|---|---|---|---|---|---|---|---|---|
| 1 | `openchamber-conductor` — merged successor of `openchamber-dispatcher` (15m) + `openchamber-babysitter` (20m) | 20m | All three recorded **disabled** (registry, 2026-11-03); conductor run-scratch in the goal workspace runs to 2026-11-08 | The dispatcher launches OpenCode sessions for queued tasks via the OpenChamber API (quota gate, tier/pool rules, engagement verification); the babysitter shepherds live sessions (stall classification, nudges, duplicate stops, terminal queue states) | 197M + 193M | **migrate** — existing-session scheduled pokes into persistent sessions first (study shape (a)); a native OpenChamber module is the destination (shape (b)) | **1 — first** (the pair is the management plane the other migrations need) | #15339 |
| 2 | Issue creators: `new-issue-creator-code-a`, `new-issue-creator-code-b` | 30m | enabled | Each run hunts for genuine improvements in its repo, verifies candidates, dedupes, files issues, enqueues queue packets — a self-contained runbook that ports near-unchanged | (family) 0.96B | **migrate** — one scheduled task per lane, new-session mode | 1 | #15340 |
| 3 | Issue creators: `new-issue-creator-opencode` (1h), `new-issue-creator-openchamber` (1h), `new-issue-creator-nix` (2h) | 1–2h | enabled | Same runbook, pinned to their repos | (family) 0.96B | **migrate** — as row 2 | 1 | #15340 |
| 4 | Creator base lanes: `new-issue-creator`, `new-issue-creator-b` (unpinned rotation) | 30m | **disabled** (registry, 2026-11-03) | Superseded by the per-repo pinned lanes (rows 2–3) | — | **retire** — if still idle when #15340 starts; the confirmation is that issue's first step | — | #15340 |
| 5 | Repo-queue worker shifts — registry generation: `repo-queue-worker-shift` + `-b`..`-p` (code pilot A–P), `-nix`, `-openchamber`, `-opencode` | 40m | enabled | The Muse layer claims queue tasks and does the implementation work itself in laptop worktrees (the identical runbook an OpenCode session executes when dispatcher-launched) | (family) 2.26B | **migrate** — per-queue OpenChamber scheduled tasks, new-session mode; partially landed already (see "Already native") | 1 | #15341 |
| 6 | Repo-queue worker shifts — `-nix-b`, `-opencode-b` | 40m | **disabled** (registry, 2026-11-03) | Overflow lanes of row 5's family | — | **migrate** with the family — re-enable natively only if lane counts call for it; the Muse copies retire with the family | 1 | #15341 |
| 7 | Repo-queue worker shifts — projection generation: `repo-queue-worker-shift-any-a`..`-y` (25 unfiltered), `-code-a`..`-e` (5 code-only), `-not-code-a`..`-c` (3) | 40m | definitions present in the repo projections on this seat; live state unverifiable from here | The current designated-lane generation of row 5's family (filtered lanes per #8097) | (family) 2.26B | **migrate** with the family — the roster reconciliation against the live scheduler is #15341's first step | 1 | #15341 |
| 8 | `repo-fleet-monitor` | 2h | enabled | Fleet safety net: gathers gh/queue state across the four repos, intake enrichment, in-progress tracking; issues verdicts (cancels tasks, routes triage) | 150M | **migrate (partial)** — the gathering half ports to a scheduled session producing the digest; verdicts stay Claude-reviewed initially (~60–70% of its tokens are the gathering half) | 2 | #15342 |
| 9 | `pr-diff-review` + `pr-diff-review-b` | 2h | enabled | Review PR diffs and file findings as issues; since the 2026-11-22 Stage B charter amendment (#2313) they also issue live verdicts, merge on accept, and close on reject for epic #2055 PRs | 97M | **migrate (partial)** — OpenCode sessions draft the reviews, Claude spot-checks; the Stage B verdict duties postdate the study and raise the judgment content — the port design must state explicitly which parts move | 2 | #15343 |
| 10 | `pool-quota-steward` | 1h | enabled | Hourly pool balance, change ledger, impact follow-up; its reads substantially duplicate the laptop's native quota-gatherer (15m systemd timer, no Muse) | 74M | **migrate (partial)** — a thin Claude check over gatherer output replaces most of the job; the change-ledger duty needs an explicit home in the replacement shape | 2 | #15344 |
| 11 | Daily launchers: `opencode-bolt-palette` (daily 08:12), `opencode-daily-top-issue` (daily 07:12, rotating repos), `opencode-daily-top-issue-code` (daily 07:12) | daily | enabled | Exist only to launch OpenCode sessions (Bolt+Palette improvement sessions; the day's top issue) — fixed prompt, no cross-round state | not in study (postdate it) | **migrate** — the cleanest scheduled-task shape in the fleet (new-session mode); proposed by this inventory, not ranked by the study | 2 | #15345 |
| 12 | `main-ci-failure-fix-trigger` | 10m | enabled; runs on the primary seat and on Felix's sandbox seat (live `cron.list`, 2026-11-25) | Detects new main-branch CI failures across the four repos, files/reuses the tracking issue, enqueues the fix task, and dispatches OpenCode fix sessions via the conductor mechanics; also runs the CI run-silence probe | not in study | **migrate — assess first** (proposed by this inventory): the port turns on shared-state locality (dedupe watermarks), the queue-enqueue leg, and the probe's checkout — the follow-on is an assessment, and the port proceeds only if the dedupe guarantee survives | 2 | #15346 |
| 13 | `fleet-watchdog` | 15m | enabled | The fleet's monitor: checks for stuck, failing, or silent fleet jobs | 121M (≈63M/day at the post-fix run-rate) | **keep** — it is the monitor *of* the fleet; moving it into a watched system destroys its value. It must instead **gain** coverage of the OpenChamber scheduled tasks (#2174 gap 3; the watchdog-side fix, PR #11158, is merged per #14222's record) | — | — |
| 14 | `orchestration-pass` | 1h | enabled | Cross-system synthesis for Drew (needsDrew / needsOrchestrator); consumes every other job's output | 74M | **keep** — it is the consumer and escalation collector for the migrated jobs; judgment- and Drew-facing by definition | — | — |
| 15 | `session-action-consumer` | 1m | enabled | Applies dashboard session actions (poke / answer / stop) to live OpenCode sessions inside the ~600s question-tool window | not in study | **keep** (proposed by this inventory) — a latency shim, not a work loop: its contract is a 1-minute cross-system response window, and its native home would be an OpenChamber server module, not a scheduled task | — | — |
| 16 | `issue-sweep-rotating` | (was 1h) | **gone from the surface**: converted to the Make-role triage feeder 2026-10-30 (#2138 CONVERT, impl #2156), last run 2026-11-08, found disabled 2026-11-16, absent from the 2026-11-17 full cron read; registry row removed 2026-11-22 | Was the study's wave-2 triage candidate (88M/24h at study time) | **retire** — already off the surface; its feeder role is covered by the hourly `triage-step` job (#2152). Whether the removal is permanent retirement or a pause is the endgame decision still open on #2138 — this inventory records the surface fact, not that decision | — | — |

## Wave order (of record)

- **Wave 0 — already landed, additive (2026-10-29 → 2026-10-31):** OpenChamber
  scheduled queue lanes were created beside the untouched Muse lanes —
  15 queue lanes across the code / opencode / openchamber / nix projects
  plus the #3026 claimant/executor (registry Surface 2a; rollout record in
  the study folder). Row 5–7's migration is therefore *partially* done;
  #15341 owns parity measurement and the Muse lanes' retirement.
- **Wave 1:** conductor (row 1) first — it is the management plane — then
  one creator lane and one shift lane as measured A/B ports, then the
  remaining creators (rows 2–4) and the shift family's completion
  (rows 5–7). Estimated saving ≈3.4B Claude input tokens/day (study).
- **Wave 2:** rows 8–12, in the order tabled. Estimated additional saving
  ≈0.25B/day for rows 8–10 (study); rows 11–12 were not sized by the study.
- **Keep:** rows 13–15 stay on Muse cron for the named reasons.
- **Retire:** row 16 (done on the surface; ratification open on #2138) and
  row 4 (inside #15340).

**Gate between inventory and any cutover** (#2174's own acceptance): the
four monitoring gaps closed and demonstrated on one pilot — ONE queue lane
running as an OpenChamber scheduled task for 48h with per-run history,
token accounting, and watchdog coverage — before the rest of wave 1 moves.
Status per #14222's record: gap 1 (per-run history, PR #8437) and gap 3
(failure watchdog, PR #11158) are merged; gap 2 (token accounting — the
#2072 OpenCode-side exporter) and gap 4 (server residency — the laptop
OpenChamber deployment confirmed always-on) were still open at inventory
time. Every follow-on above repeats this gate.

## Already native (not wrappers — the migration target state)

Registry Surface 2a, recorded 2026-11-03 and amended 2026-11-23: 15 enabled
OpenChamber queue lanes (`*/40 * * * *`, America/Chicago, workhorse route;
the code project's lanes now take `fleet agent next` as their intake front
door) + the #3026 opencode claimant/executor (`*/10 * * * *`). On the
systemd surface, `scheduled-history.timer` (laptop, code#2174) consumes
scheduled-run history — gap-1 infrastructure. These are listed so the
inventory does not read as if nothing has moved; they are OpenCode
schedules, not Muse cron wrappers, and carry no disposition here.

## Considered and excluded

| Job | Why it is not a wrapper row |
|---|---|
| `triage-step` (1h) | Claude-side triage judgment (the Make-role feeder that replaced `issue-sweep-rotating`'s feeder role, #2152). It launches no OpenCode sessions in the sources read. If a future pass finds it dispatching sessions, it joins wave 2 beside row 8. |
| `stewardship-cycle` (1h) | Review-and-dispatch over queue/goals state; the study's keep list names stewardship explicitly ("judgment- and Drew-facing by definition"). No session-launching found in its definition. |
| `agent-queue-worker` (2h, goal:supabase-muse-dev-platform-backups) | An earlier queue-worker pilot. No evidence in the sources visible from this seat that it launches or shepherds OpenCode sessions; its definition file is absent from this seat's projections. Flagged here so a primary-seat pass can confirm or add it. |
| `felix-lane-check`, `felix-seat-keeper`, `felix-fleet-machine-watchdog`, `heartbeat` (Felix's seat) | Seat-local Muse jobs; they spawn Muse lanes or run shell probes — no OpenCode sessions. (`main-ci-failure-fix-trigger` is the one job that runs on both this seat and the primary seat, and it is row 12.) |
| Jules schedules (registry Surface 3), Mac launchd (Surface 5), PM-stack intervals (Surface 6), Worker cron triggers (Surface 7) | Different surfaces, not Muse cron; #2174's scope is the Muse→OpenCode wrappers. The OpenClaw sibling question is #2177. |

## Verification

This inventory was verified against current main of developing-today/code
(aca75d865) as follows: the registry was read from that checkout; every
job id, cadence, and state in the table was cross-checked between at least
two of the named sources (registry ↔ study ↔ projections ↔ this seat's
live `cron.list`); the follow-on issues #15339–#15346 were filed in this
pass and are linked from the table and from #2174. Live-state limits are
stated in the sources section — no row claims a live read of the primary
seat's scheduler or the laptop's stores, because neither was reachable.
