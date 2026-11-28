# FDR-RA-001 — Chat operating model: category side chats, insert-only job writes

- **Date:** 2026-11-13 (approval and execution); recorded durably 2026-11-24
  (second-pass verification on #9569 found the approval recorded only in an
  issue comment and a daily log — this record closes that gap per #9180).
- **Status:** accepted; implementation partially complete (open items below).
- **Owner:** reporting-architecture program (#9568); maintenance through #9180.
- **Decision maker:** Drew (Drewry Pope), Reports chat, 2026-11-13 09:28 CST.
- **Scope:** delivery routing for all live reporting jobs; the side-chat set
  those jobs write to.
- **Supersedes:** none
- **Superseded by:** none
- **Related records:** indexed in this package's
  [`decisions/README.md`](README.md) and from the canonical index's
  package-scoped table in
  [`doc/decisions/README.md`](../../decisions/README.md); format follows the
  FDR-PMO series in
  `doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/decisions/`.

## Context and problem

Reporting jobs each delivered into their own origin side chat, producing
~180 chats (intake audit in `../sources/audit-side-chats.md`): 28 per-lane
shift chats, ~15 GitHub-activity per-job chats, and per-job origin chats for
the pool, queue, and token families. Nobody could read that surface, and the
chats had drifted into being treated as state stores. Drew's directive
(Reports chat, 2026-11-13 09:11 CST; statements S0.1–S0.9 in
`../sources/statement-inventory.md`) set the target model; #9569 owns the
implementation. The issue's boundary rule: no rerouting ships before Drew
approves the map.

Sources: [#9569](https://github.com/developing-today/code/issues/9569)
(approval + execution record, comment 6857681326);
[#9568](https://github.com/developing-today/code/issues/9568) (program epic);
[#9180](https://github.com/developing-today/code/issues/9180) (decision-record
convention this file follows).

## Options considered

1. **Five category sinks; jobs write insert-only into exactly one sink
   (approved).** One side chat per job family; a job executes, updates the
   backend/repo itself, inserts its update into its category chat, and ends.
2. **Four sinks** — fold Pools & Quota into a Space plus Reports (the Q2
   alternative). Rejected by the Q2 default: quota/pool traffic is its own
   family with its own readers.
3. **Keep per-job origin chats.** Zero migration cost; retains an unreadable
   ~180-chat surface and the chats-as-state failure mode.
4. **Category home for queues/PM health/questions elsewhere than the renamed
   Orchestration pass chat** (Q1 alternatives). The Q1 default — rename the
   existing Orchestration pass chat — was approved instead of minting a new
   chat, preserving its history and membership.

## Decision

One category side chat per job family. A job's agent executes, updates the
backend/repo itself, inserts its update into its category chat
**insert-only (it never reads the chat)**, and ends. Chats are write-sinks,
not state stores. The approved map:

| Sink | Chat | Covers |
| --- | --- | --- |
| Reports | `f5915c63` | exceptions, errors, decisions (2026-11-07 ruling; unchanged) |
| Lanes | `8117667e` | all lane effort (Muse lanes + OpenChamber lane notes) |
| Designs, Issues & PRs | `fc1aea1a` | all GitHub activity |
| Queues, PM Health & Questions | `964a47dc` | queue state, PM-stack health, pending questions (renamed from "Orchestration pass") |
| Pools & Quota | `d92d987b` | pool composition, quota, token usage (renamed from "Pool intelligence") |

Open questions, settled with the approval (defaults in the design doc):

- **Q1:** category-4 home = the Orchestration pass chat, renamed.
- **Q2:** five sinks — Pools & Quota is its own sink.
- **Q3:** archive retired chats only after one verified delivery cycle per
  rerouted job.
- **Q4:** the six space-owner chats stay unpinned owners.

Every rerouted job body carries the per-task notes line naming its sink:
"Send relevant notes to \<sink\>. Exceptions and decisions still go to
Reports, unchanged." Escalation: exceptions → Reports; high-urgency → the
main chat as well (page-health-push is the precedent); jobs may message
other tasks / the orchestrator / enqueue queue items.

## Execution record (2026-11-13, verified by read-back at the time)

Repo copy edited + committed in hatch-personal, identical text applied via
cron update, read back in the runtime-written definition files:

- → **Pools & Quota (8):** pool-quota-steward, pool-composition-review,
  pool-meta-reviewer, go-plan-expiry-watch, coding-plan-watch-daily,
  daily-token-usage-checkin, weekly-token-usage-review, usage-quota-report.
  pool-intelligence already delivered there.
- → **Queues, PM Health & Questions (10):** pending-questions-hourly,
  decision-set-activity-processor, queue-hygiene-sweep,
  dropped-thread-catcher, blocked-until-checker, queue-capture-6450,
  fleet-live-status-refresh, boot-init-todo-check, stewardship-cycle
  (working notes), hatch-untracked-review (routine notes). Dual delivery by
  design: morning-day-start-readout keeps its day-start chat and adds this
  sink; page-health-push keeps its main-chat push and adds its log here.
- → **Designs, Issues & PRs (4 + 3):** stale-pr-warden,
  branch-cleanup-sweep, roadmap-sync-grist-md, action-outbox-processor
  (exceptions); plus pr-diff-review, pr-diff-review-b, pr-comment-review —
  their saved definitions still pointed at their origin chats despite the
  2026-11-05 ruling, so they were rerouted delivery-only; **their bodies do
  not yet carry the notes line** (open item 2 below).
- → **Lanes (6 rerouted, 8 total):** the audit's premise was wrong for 6 of
  the 8 enabled Muse lane jobs — any-a/b/c/d and code-b/code-d still
  delivered to legacy per-lane shift chats; all 6 rerouted. code-a/code-c
  already delivered to Lanes. Disabled variants left untouched.
- Audit-named families with **no live cron** (checked incl. disabled):
  improvement-issues (7 chats), issue-sweep (2), coverage-audit (2),
  design-architect (2), fleet-watchdog. Nothing to reroute; their chats join
  the pending-archive list.

## Consequences and open items

- Reports routing is unchanged everywhere: exceptions and decisions still
  go to Reports; full reports still land in the Fleet Reports Space.
- **Open item 1 — pins.** The chat tooling exposes no pin operation. Reports
  was already pinned; Lanes, Designs, Issues & PRs, Queues, PM Health &
  Questions, and Pools & Quota await Drew's tap-to-pin in the app.
- **Open item 2 — PR-review notes lines.** pr-diff-review, pr-diff-review-b,
  and pr-comment-review deliver to Designs, Issues & PRs but their bodies
  lack the notes line every other rerouted job carries. Prepared text (one
  line, added to each body): "Send relevant notes to Designs, Issues & PRs.
  Exceptions and decisions still go to Reports, unchanged." Applying it
  edits live job bodies and needs Drew's yes.
- **Open item 3 — Q3 archive pass.** No chats were archived in the approval
  pass. Pending archive (~70 chats, full list on #9569 comment 6857681326):
  the two token chats, all 28 per-lane shift chats, and the pool / queues /
  designs cluster origin chats — gated on one verified delivery cycle per
  rerouted job. Caveat recorded at approval time: morning-day-start-readout's
  chat `8c089bc3` is a live co-delivery target by design; archiving it would
  redirect that delivery to the main chat, so it needs Drew's explicit call
  when the archive pass comes.
- **Open item 4 — full-cycle delivery verification.** The 2026-11-13 pass
  verified definitions by read-back; a recorded one-full-cycle live delivery
  verification per rerouted job (acceptance criterion 2 of #9569, and the
  Q3 gate) has not been posted. It must run on the seat that owns those jobs
  and chats.

## Revisit triggers

Reopen if a new job family appears that fits no sink, if a sink's volume
makes it unreadable (split it, recording the split as a superseding FDR), if
chat tooling gains a pin operation (retire open item 1 mechanically), or if
the insert-only rule is found to have hidden a failure a job would have
caught by reading its sink.

## Implementation and source links

- [#9569](https://github.com/developing-today/code/issues/9569) — C1
  implementation issue; approval + execution record in comment 6857681326.
- [#9568](https://github.com/developing-today/code/issues/9568) — program
  epic; design in `../note.md`, intake audit in
  `../sources/audit-side-chats.md`, statements in
  `../sources/statement-inventory.md`.
- [#9180](https://github.com/developing-today/code/issues/9180) — fleet
  decision-record convention.
