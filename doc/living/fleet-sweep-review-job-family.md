# Fleet sweep / review job family

Living document · Status: shipped (2026-10-17) · Register entry F4.1 · Tracking epic: developing-today/code#8805

## Purpose

- Keep the four fleet repos healthy by continuously sweeping issues and
  reviewing PR diffs on a schedule, instead of waiting for manual triage.
- Repos in scope (as named in the original ask): developing-today/code,
  developing-today/opencode, dt openchamber, dezren39/nix.
- Owning goal: keep-the-four-repos-healthy (goal_a243772844c3).

Original ask (2026-10-17):
- "make a scheduled task to check and fix one issue in developing today
  code or developing today opencode or dt openchamber or dezren39/nix
  each execution... make 4 tasks that handle this and each runs every
  2 hours, divided evenly 30 minutes apart... Think thoroughly and act
  autonomously... always do something."

## What landed (features, and how each works)

- Issue-sweep rotation (2026-10-17):
  - Four scheduled tasks, one per repo, each every 2 hours, staggered
    30 minutes apart.
  - Each execution checks and fixes one issue in its repo.
- Standing merge authority (2026-10-17, same day):
  - Rule: "go ahead and merge if a pr seems good you can always fix it
    later if it's not."
  - This replaced "Drew reviews / never merge own PRs" for the sweeps.
  - Applied via cron.update + GOAL.md + MEMORY.md.
  - Exercised the same morning: opencode PR #22, openchamber PR #37,
    and dezren39/nix PR #379 merged under it.
- PR diff review job (2026-10-18):
  - Drew: "also regularly review pr commits and diffs open or closed and
    make issues from useful findings there."
- Restructure (2026-10-19, recorded directive ~21:38 CDT):
  - Three per-repo sweeps removed.
  - ONE `issue-sweep-rotating` job every 40m, picking a repo at random.
  - Cadence changes: new-issue-creator 15m → 20m, pr-diff-review
    30m → 20m, boot-init-todo-check 20m → 30m.
- Reporting split + escalation (2026-10-19):
  - Side-chat reporting split approved.
  - Escalation rule approved: "escalate secrets, broken main, and
    blocked-on-me items to the main chat".
  - An ESCALATION paragraph was appended to all six job bodies.
- Audits:
  - 2026-10-19 (same evening): job cadence / duration + billing audit.
  - 2026-10-20 06:03 CDT: fleet usefulness audit — issue ledger
    Oct 17–19: opened 129, closed 161, net −32.

## Rough architecture

- Implementation is cron jobs under goal keep-the-four-repos-healthy.
- Design artifact in the register: the goal's GOAL.md, plus per-repo
  backlogs in the goal's `hidden_files/`, plus the ESCALATION paragraph
  in the six job bodies under the goal's `crons/`.
- No single GitHub issue tracks the family as a whole.

## Status

- Design status in register: built-into-running-system.
- Implementation status in register: shipped (and repeatedly restructured
  in place).
- Tracking: NEVER-ISSUED as a family — searched "issue sweep rotation";
  only Nov-era conversion tasks code#2156 / #2138 exist, not the original asks.
- Triage: (b) DONE-WITHOUT-ISSUE — implemented directly as cron jobs
  under goal keep-the-four-repos-healthy.

## Next steps — immediate / near-term / long-term

- Immediate: fix the stale wording noted below.
- Near-term: not recorded.
- Long-term: not recorded.

## Open follow-ups

- GOAL.md did not yet reflect the 2026-10-19 restructure at record time.
- Stale wording pending rewrite in the code-sweep, new-issue-creator, and
  pr-diff-review bodies: "mcpx lives in BOTH code and nix".
  - That wording was overtaken by F2.2 (mcpx dedup) and F2.4 (mcpx
    consolidation).

## Future ideas not yet specced

- Not recorded in the register entry.

## Links

- Register entry: F4.1 in `~/workspace/design-audit-60d/REGISTER.md`
- Related register entries: F2.2, F2.4 (mcpx canonical source),
  F4 (fleet orchestration & dispatch, parent)
- Later conversion tasks named in the entry: developing-today/code#2156,
  developing-today/code#2138
- Tracking epic: developing-today/code#8805
- Owning goal: keep-the-four-repos-healthy (goal_a243772844c3)
