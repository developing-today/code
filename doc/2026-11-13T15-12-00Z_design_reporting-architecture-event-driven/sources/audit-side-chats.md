# Side-chat audit + proposed target map — 2026-11-13

Deliverable A of the reporting-architecture intake (Drew's directive,
2026-11-13 09:11 CST, Reports chat). **Report only — nothing was rerouted,
renamed, archived, or re-pointed in this pass.** Drew reviews this map before
anything moves.

## Method / evidence

- `chat.list` (incl. archived): ~180 chats. Every chat whose primary content
  is job/lane/report deliveries was classified.
- `cron.list` (incl. disabled): ~110 live Muse jobs; job → originator chat
  is 1:1 by title for the per-job chats.
- Sampled deliveries read directly: Lanes chat (lane shifts code-a/code-c
  deliver there today), a legacy per-lane shift chat (still receiving a
  code-d delivery under a stale "shift F" title), Outstanding Questions
  (conversational relay, not a job sink).
- Prior rulings applied: 2026-11-05 PR-review family → Designs & Issues;
  2026-11-06 pre-rebuild delivery rules; 2026-11-07 reporting rebuild
  (exceptions to chat, full reports to the Fleet Reports Space via
  `recordReport`, per-job exception conditions).

## What exists today — three overlapping layers

1. **Per-job originator chats (~90).** Almost every recurring job was born in
   its own side chat and historically delivered there. Since the 2026-11-07
   rebuild, the 17 rebuilt streams write full reports to the Fleet Reports
   Space and only exception notices to the Reports chat — but their old
   per-job chats remain as dormant duplicates, and non-rebuilt jobs still
   deliver into their own chats.
2. **Category chats (4, only one pinned).** Reports (pinned), Lanes,
   Designs, Issues & PRs, plus the questions pair. Lane shifts already
   deliver into **Lanes** today (verified: code-a and code-c runs,
   2026-11-13 morning) — the category model is already half in effect for
   lanes, while the 26 legacy per-lane shift chats still exist and at least
   one is still receiving deliveries under a stale title.
3. **Records that aren't chats.** Fleet Reports Space (17 streams), PM-stack
   pages/dashboards, the fleet queue DB, and `page-health-push` (5-min job
   that already pushes PM-page down/recovery + new waiting-on-Drew items to
   the **main chat** — an existing precedent for the high-urgency path).

Duplication hotspots: quota facts live in the Space + Reports notices +
usage-quota originator chat + AI Plans & Usage space-owner chat +
Token-usage chats (4). Lane facts live in Lanes + 26 per-lane chats + the
queue DB + OpenCode & Muse Activity space. GitHub-activity findings live in
Designs & Issues + ~15 per-job chats (PR review ×3, improvement-issues ×7,
issue-sweep ×2, coverage-audit ×2, design-architect ×2, stale-PR warden,
Repo Sweeps).

## Proposed target model (from the directive)

- **Five category sinks, pinned.** Every recurring job has exactly ONE home
  category chat. The job's agent executes, updates the backend/repo itself,
  inserts its update into its category chat **insert-only (never reads the
  chat)**, and ends. Chats are write-sinks, not state stores; state lives in
  the Space, PM-stack DB, queue DB, and GitHub.
- **Per-task notes lines** in job bodies literally name the sink ("send
  relevant notes to Lanes", etc.).
- **Escalation paths, unchanged in spirit:** exceptions/decisions →
  Reports; high-urgency → also main chat (page-health-push is the working
  precedent); a job may message another task/the orchestrator or enqueue
  queue items instead of (or besides) chatting.
- **Pinned revival:** pin the four category chats that already exist
  (Reports stays pinned; pin Lanes; pin Designs, Issues & PRs; pin the
  queues/health/questions chat once chosen). The pre-proliferation pinned
  channels were exactly these category channels; the per-job chats that
  replaced them retire as delivery targets (histories stay searchable;
  archiving is a later, separate decision for Drew).

### Proposed category map

| # | Category chat (proposed pin) | Owns | Insert-only writers |
|---|---|---|---|
| 1 | **Reports** `f5915c63` (already pinned) | Exceptions, errors, decisions needed — per the 2026-11-07 ruling. Nothing routine. | All rebuilt jobs' exception notices (already), stewardship material outcomes, git-hygiene one-liner |
| 2 | **Lanes** `8117667e` | All lane effort: Muse queue lanes, OpenChamber lanes' notes, lane censuses/diagnostics | repo-queue-worker-shift-* (already), lane portion of fleet-minder/fleet-watchdog notes |
| 3 | **Designs, Issues & PRs** `fc1aea1a` | ALL GitHub activity: reviews, findings, new issues, sweeps, audits of issue quality, design passes | pr-diff-review, pr-diff-review-b, pr-comment-review (already), improvement-issues family, issue-sweep family, coverage audits, design architects, stale-pr-warden, branch-cleanup, action-outbox exceptions |
| 4 | **Queues, PM Health & Questions** — propose **Orchestration pass** chat `964a47dc`, renamed | Queue state/health, PM-stack health, pending questions for Drew, ops checks | orchestration-pass (already home), queue-hygiene-sweep, pending-questions-hourly, decision-set-activity-processor, dropped-thread-catcher, blocked-until-checker, queue-capture-6450, page-health-push (keeps its main-chat push for urgency; full log here), fleet-live-status-refresh, boot-init, morning readout (summary line) |
| 5 | **Pools & Quota** — propose the **Pool intelligence** chat `d92d987b`, renamed (or fold into Reports+Space; see open question Q2) | Pool/quota detail that isn't an exception: composition, intelligence, meta-review, plan watch, token usage | pool-quota-steward, pool-composition-review, pool-intelligence, pool-meta-reviewer, go-plan-expiry-watch, coding-plan-watch-daily, daily-token-usage-checkin, weekly-token-usage-review, usage-quota-report full detail (Space remains the record; this is the chat sink) |

Space-owner chats (6) are **not** chat sinks for jobs; they own Spaces. See
their section below.

## Chat-by-chat inventory and proposed fate

### Already category chats

| Chat | What lands there now | Duplicates | Proposed fate |
|---|---|---|---|
| Reports `f5915c63` (pinned) | Exception notices from rebuilt jobs; Drew's directives; incident coordination | Space (full reports), per-job chats | **Keep as category #1.** No change; already the model citizen. |
| Lanes `8117667e` | All current Muse lane shift reports (verified today) | 26 per-lane shift chats; queue DB | **Keep as category #2, pin it.** Absorbs the per-lane chats (below). |
| Designs, Issues & PRs `fc1aea1a` | pr-diff-review / -b, pr-comment-review deliveries (2026-11-05 ruling) | ~15 GitHub-activity per-job chats | **Keep as category #3, pin it.** Absorbs the GitHub-activity cluster (below). |
| Orchestration pass `964a47dc` | Hourly queue triage/assignment/goal measurement | Fleet monitor chat; queue-hygiene has no chat (goal-owned) | **Keep, rename to category #4** ("Orchestration — queues, PM health, questions"), pin it. Absorbs questions + queue/ops cluster (below). |

### Per-lane shift chats (26) — all merge into Lanes, retire as delivery targets

Chats: "Repo queue worker shift {A,B,C,D,E,F,G,H,I,J,K,L,M,N,O,P,Q,R,S,T,U,V,W,X,Y,Z} — {code,opencode,openchamber,nix} pilot" (chat ids incl.
`ee264a35` F, `90b9feae` K, `eb8aaa8c` J, `c0469541` M, `657bcb1c` D,
`b338eabf` L, `3715cbda` N, `b1c38497` W, `808bdc8b` X, `241c7040` T,
`5aca232e` Y, `62652fcc` V, `e759286c` O, `5abd63c7` U, `9f957726` G,
`ba745577` P, `7ec74689` A-opencode, `5471105b` B-openchamber, `184eb6e5` S,
`dde003bd` R, `4922ff7c` Z, `59f80b7b` C, `1767405b` Q, `e65d5f71` A-code,
`45556023` I, `ff68f67c` H, `bcb41c24` E, `9cb1731a` B-code, `8a81682a`
B-opencode, `bdf4da8a` A-openchamber, `22b7e470` A-nix, `bb01de80` B-nix),
plus "Agent queue worker shift (pilot)" `ef49e868` and "Repo queue worker —
code" `db0d1dd5`.

- **What lands there:** legacy per-lane shift reports. Live Muse lane jobs
  (any-a…d, code-a…d) already deliver to Lanes; at least one legacy chat
  still received a delivery today under a mismatched title ("shift F" chat
  received a code-d run) — titles and deliveries have drifted apart, which
  is itself the anecdote-chat failure mode Drew named.
- **Jobs whose delivery must change:** none for the current 8 Muse lane
  jobs (already → Lanes). Any OpenChamber-side lane reporting still pointed
  at these chats repoints to Lanes when the orchestration program
  (#9196) touches it. **Fate: retire all 28 as delivery targets; keep
  histories.**

### Pool / quota / token cluster — merge into category #5 (or Reports+Space)

| Chat | Job(s) | Proposed fate |
|---|---|---|
| Usage & quota report → Reports `2657aa66` | usage-quota-report (hourly) | Retire as target — full detail already lands in the Space; exceptions already land in Reports. If category #5 is adopted, detail notes go there. |
| Pool quota & config steward `38ee620e` | pool-quota-steward | Merge → #5; exceptions stay → Reports (already). |
| Pool composition review `7f466563` | pool-composition-review | Merge → #5; recommendations (its exception condition) stay → Reports. |
| Pool intelligence `d92d987b` | pool-intelligence | **Proposed category #5 home** (rename "Pools & Quota"), or merge → Reports if Drew prefers four chats (Q2). |
| Pool meta-reviewer `fd8a5ed2` | pool-meta-reviewer | Merge → #5; high+ findings stay → Reports. |
| Full-quota check / Go lapse watch `a420cdd3` | go-plan-expiry-watch | Merge → #5; inside-14-days alerts stay → Reports. |
| Coding AI plan watch `bc668953` | coding-plan-watch-daily | Merge → #5; changes stay → Reports. |
| Daily token usage check-in `5f8e2f6c` | daily-token-usage-checkin | Merge → #5; deviation alerts stay → Reports + Space stream (already). |
| Weekly token usage review `bb8d0c04` | weekly-token-usage-review | Merge → #5. |
| Token usage tracking `f3937a75` | (working chat, no live job) | Retire — superseded by the check-in/review jobs + Space stream. |
| Review token usage and sidechats `a15e5512` | (working chat) | Retire — its side-chat-review topic is literally this audit; fold into #5/Reports history. |

### GitHub-activity cluster — merge into Designs, Issues & PRs (#3)

| Chat | Job(s) | Notes |
|---|---|---|
| PR commit/diff review (hourly) `0b2c48ff` | pr-diff-review | Already delivers → #3 (2026-11-05). Retire origin chat as a reading surface; no delivery change. |
| PR commit/diff review — Lane B `bea6cca0` | pr-diff-review-b | Same. |
| PR comment review `b964bcd4` | pr-comment-review | Same. |
| PR findings `4faa3b2d` | (findings working chat) | Merge → #3; retire. |
| New improvement issues — code Lane A `98bfbebc`, Lane B `d75305eb` + `58b981f6`, openchamber `b49542c9`, opencode `7251e596`, nix `af345dac`, hourly rotating `6a310374` | improvement-issues family (not all still live as Muse crons) | Merge → #3. Jobs to reroute: whichever variants remain live; the rest are dead-job chats (below). |
| Issue sweep: code `052d49e6`, rotating `a0197933` | issue-sweep family | Merge → #3. |
| Coverage audit — weekly/daily `88c63702`, deep slice `030f85cb` | coverage-audit family | Merge → #3. |
| Design architect — code `052ebdf1`, non-code `ebc06192` | design-architect family | Merge → #3 (designs are GitHub activity); register summaries may also note → Reports when a design needs Drew. |
| Stale PR warden `1885123b` | stale-pr-warden | Merge → #3. |
| Repo Sweeps `b7d12dcb` | (sweep working chat) | Merge → #3; retire. |
| Daily branch cleanup `64546f18` | branch-cleanup-sweep | Merge → #3 (branch/PR hygiene is GitHub activity). |
| Action outbox processor `bc333880` | action-outbox-processor | Keep silent-by-design (it executes filings); **exceptions → #3**. Retire chat as a reading surface. |
| Roadmap sync `a5f3c80a` | roadmap-sync-grist-md | Merge → #3 (roadmap artifacts are repo files); retire chat. |

### Queues / PM-health / questions cluster — merge into category #4

| Chat | Job(s) | Notes |
|---|---|---|
| Pending questions for Drew — hourly `dcb4f09f` | pending-questions-hourly | Questions move → #4 (its job's 1–2 items/run post there). This chat retires as a separate surface. |
| Outstanding Questions `ea543680` | (conversational relay — Drew answers here; verified content is rulings relayed to issues) | Merge → #4: the relay habit moves to the category chat so questions, queue state, and rulings share one home. Retire as separate chat. |
| Fleet monitor `557030f2` | fleet-monitor (no longer a live Muse cron) | Retire — function lives in fleet-minder + orchestration-pass; notes → #4 via those jobs. |
| Fleet watchdog `2740210f` | fleet-watchdog | Already exception-only → Reports for new alerts/recoveries. Routine notes → #4. Retire origin chat as reading surface. |
| OpenCode session minder `83d743a2` | opencode-session-minder (**disabled** 2026-11-08) | Stay retired. Its monitoring gap is a known input to the event-driven thread (hook-based checks) — do not revive the cron chat. |
| Boot init + todo check `2b3eb162` | boot-init-todo-check | Merge notes → #4; retire chat. |
| Morning day-start readout `8c089bc3` | morning-day-start-readout | Keep Drew-facing delivery (main/day-start); a one-line summary inserts → #4. Retire chat as reading surface. |
| Stewardship cycle `8381cc4c` | stewardship-cycle | Material outcomes already → Reports. Working notes → #4 (dispatch is orchestration). Retire origin chat as reading surface. |

### Fleet Reports / rebuilt-job origin chats — retire as reading surfaces (no delivery change)

The rebuilt jobs (usage-quota-report, pool-quota-steward, pool-intelligence,
pool-composition-review, pool-meta-reviewer, fleet-watchdog,
git-hygiene-sweep `55b8ff67`, stewardship-cycle, go-plan-expiry-watch,
coding-plan-watch-daily, hatch-untracked-review / "Sandbox files"
`4c56a953`, daily-token-usage-checkin) already write full reports to the
Fleet Reports Space and exceptions to Reports. Their origin chats duplicate
the Space. **Fate: retire as delivery/reading surfaces; the Space is the
record, Reports is the exception sink, category #5 takes pool detail if
adopted.** Sandbox-files (hatch-untracked-review) exceptions (>7d untracked,
secret-shaped files) keep going → Reports; routine routing notes → #4.

### Space-owner chats (6) — keep, but reclassify

"Space owner: Fleet Reports" `fcf52a31`, "Projects Overview" `3d1d2f4a`,
"AI Benchmarks" `d30275f6`, "OpenCode & Muse Activity" `785419b6`,
"AI Plans & Usage" `f2178de6`, "ID & Atlas" `f707171a` (plus refresh jobs
like id-atlas-refresh, fleet-live-status-refresh, fleet-roadmap-stats-refresh).
These are owner/originator chats for Spaces, not job report sinks — under
the target model they are the *correct* pattern (one owner per artifact).
**Fate: keep all six, unpinned, and make them the freshness owners in the
report-pages thread: each space's refresh job inserts its update here and
into the Space; staleness exceptions → Reports.** Overlap to note: AI Plans
& Usage duplicates category #5's topic at the Space layer (fine — Space is
the data surface, #5 is the chat sink).

### Dead-job chats — stay retired, do not revive

OpenChamber conductor `105bbc0b`, babysitter `18980cca`, dispatcher
`97606e57`; Daily top issue → OpenCode (code) `7a0e3370` and (rotating)
`6257289f`; Main-CI failure trigger `61c96685`; Bolt + Palette `ac896870`;
Monitor the 40 ID design sessions `7e9fca1a`; Standing capture #6450
`59b2ebe` (its live job queue-capture-6450 notes → #4). All correspond to
Muse session-launchers Drew disabled 2026-11-08. **Fate: retire; the
event-driven program replaces their *detection* role with hooks, not with
revived cron chats.**

### Out of scope (not reporting sinks — no fate proposed)

Personal/reminder/nudge chats (Plaid nudges, Porkbun, ChatGPT-cancel set,
DNS/token renewals, Amazon Business, pet care, frontline, Airthings,
unusual-charge, Gemini release watch, Archer Slack review, Globalping,
daily Gmail summary, home-projects brief, hardware roundup, engineering
review, cold-store scan, Turso+Cloudflare idea engine, late-night shift,
memory curation, heartbeat, nixpkgs update, weekly binary/wire check,
platform restic, DNS backup) and Drew's topic/working chats (Cloudflare,
Turso, ID platform status, PM-stack review, dashboard status, SaaS status,
home cluster, Free Inferx/OpenCode models, etc.). They are conversations or
single-purpose reminders, not fleet reporting surfaces. The idea-engine and
readout-style jobs among them can adopt category sinks later without being
part of this map.

## Jobs whose delivery must change (consolidated, for the follow-up pass)

- → **Lanes**: OpenChamber lane notes (when #9196 touches them); nothing for
  the 8 live Muse lane jobs.
- → **Designs, Issues & PRs**: improvement-issues family, issue-sweep
  family, coverage-audit family, design-architect family, stale-pr-warden,
  branch-cleanup-sweep, roadmap-sync-grist-md, action-outbox-processor
  (exceptions only).
- → **Queues/PM Health/Questions (#4)**: pending-questions-hourly,
  decision-set-activity-processor, queue-hygiene-sweep, dropped-thread-
  catcher, blocked-until-checker, queue-capture-6450, fleet-live-status-
  refresh, boot-init-todo-check, stewardship-cycle (working notes),
  fleet-watchdog (routine notes), hatch-untracked-review (routine notes),
  morning-day-start-readout (summary line). page-health-push keeps its
  main-chat urgency push and adds its log here.
- → **Pools & Quota (#5)** (if adopted): pool-quota-steward,
  pool-composition-review, pool-intelligence, pool-meta-reviewer,
  go-plan-expiry-watch, coding-plan-watch-daily, daily-token-usage-checkin,
  weekly-token-usage-review, usage-quota-report (detail notes).
- **No change**: Reports exception routing for all rebuilt jobs; PR-review
  family's #3 routing; lane jobs' Lanes routing; Space recordReport writes.

## Open questions for Drew (defaults recorded in the program intake)

- Q1: Category #4 home — Orchestration pass chat renamed (recommended) vs
  Pending Questions chat vs a fresh chat. Default: rename `964a47dc`.
- Q2: Four category chats or five — does pool/quota detail get its own sink
  (#5), or does it live only in the Space + Reports exceptions? Default:
  five, because the detail volume would drown the exceptions channel.
- Q3: Retired per-job chats — archive them (history kept, list decluttered)
  or leave them dormant? Default: archive after rerouting is verified live
  for one full cycle of each job.
- Q4: Space-owner chats — keep as unpinned owner chats (recommended) or
  fold their notes into categories? Default: keep.
