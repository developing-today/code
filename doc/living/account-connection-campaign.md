# Account connection campaign

Living document · Status: shipped (2026-10-20) · Register entry X11 · Tracking epic: developing-today/code#8805

## Purpose

- Get every service signed in / connected on Muse's machine.
- Then repeat the same set on Drew's laptop.
- First asked: 2026-10-20.

## What landed (features, and how each works)

- A standing campaign, tracked as a goal, not as an issue.
- Goal: account-connection-campaign
  (goal_6f48d745f802 in the register).
- Campaign rules, as recorded:
  - Hunt down every account.
  - API keys where possible.
  - OAuth wherever possible.
  - No new MFA unless a service forces it.
  - Every token Muse creates gets filed in Bitwarden under a
    clear name.
- Campaign opened 2026-10-20 at 07:51 CDT.
- A 6-hour nudge cron surfaces one Drew-needed item at a time.
- Progress is tracked in a checklist that lives in the goal's
  hidden_files:
  `~/workspace/goals/account-connection-campaign-every-service-signed-in-on-muse-s-machine/hidden_files/accounts-checklist.md`
- The register records the checklist as covering 17 accounts,
  each with an auth route and a status.
- Checklist contents and any credential information are not
  reproduced in this document.

## Rough architecture

- Vehicle: the account-connection-campaign goal.
- Record of truth for progress: the checklist in the goal's
  hidden_files (path above).
- Cadence mechanism: a 6-hour nudge cron, one Drew-needed item
  at a time.
- Credential storage rule: tokens filed in Bitwarden under a
  clear name (see rules above).
- Tracking in the register: NEVER-ISSUED — no GitHub issue;
  goal-tracked by design.

## Status

- design_status in the register: built-into-running-system
  (standing campaign).
- implementation_status in the register: partial / standing.
- Triage in the register: (b) DONE-WITHOUT-ISSUE as a vehicle;
  per-account completions are the checklist's job.
- One completion named in the register: Plaid — at record time
  Plaid beyond Royal Credit Union stayed Drew-only in his own
  browser; the November record shows Plaid verified fully
  linked on 2026-11-08, 4 institutions / 13 accounts.

## Next steps — immediate / near-term / long-term

- Immediate: continue per-account completions in the checklist.
- Near-term: repeat the connected set on Drew's laptop, per the
  original campaign ask.
- Long-term: not recorded.

## Open follow-ups

- Per-account completions tracked in the checklist (see above).
- No other follow-ups are recorded for this entry.

## Future ideas not yet specced

- None recorded in the register entry.

## Links (register entry, related issues/PRs named in the register entry)

- Register entry: X11, in the design register (REGISTER.md).
- Goal: account-connection-campaign (goal_6f48d745f802).
- Checklist: the goal's hidden_files accounts checklist (path
  above; contents not reproduced here).
- Tracking epic: developing-today/code#8805.
- Tracking in the register: NEVER-ISSUED (no GitHub issue).
