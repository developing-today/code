# Phase 6: invites, pull, and the CLI

- Plan: [phased plan](../2026-10-11T11-08-42Z_design_remaining_work/plan-phases.md), section 11.
- Start commit: 479cc76c (origin/main, after Phase 5 tags were started elsewhere; Phase 4 merged as #191 and #192).
- Base branch `phase-6`. Tracks run in parallel:
  - `p6-seitan`: P6.1 Seitan V2, P6.2 freshness window.
  - `p6-invites`: P6.3 tickets (option E), P6.4 invite events and links.
  - `p6-recovery`: P6.6 fork recovery, P6.8 rejection reasons.
  - Later: P6.5 pull policy, P6.7a and P6.7b CLI (groups, delegation).
  - Blocked: P6.7c CLI documents and tags needs Phase 5 merged.
- Decisions in force: freshness window default 5 minutes (N23); ticket option E (N24); address exposure documented (N30); pull open with an authorized option (decision 11); fork recovery by remove and re-enrol under a new device key (decision 17).

## Steps

| Step  | Track    | Status               | Commits  | Tests         | Notes                                                                                                                                                                          |
| ----- | -------- | -------------------- | -------- | ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| P6.1  | seitan   | done, open item #194 | 3eafe3b3 | 25            | Keybase V2 vectors match (stretch, invite ID, Ed25519 key). Accept bytes and admin handler read, not run; the server side, old-style invites and a real accept are open (#194) |
| P6.2  | seitan   | done                 | 065a29b9 | 6 (31 seitan) | N23: created - W <= ctime <= min(now + W, expires). Default 5 minutes; set per inviter, world, or server; the most specific wins                                               |
| P6.3  | invites  | todo                 |          |               |                                                                                                                                                                                |
| P6.4  | invites  | todo                 |          |               |                                                                                                                                                                                |
| P6.5  | pull     | todo                 |          |               |                                                                                                                                                                                |
| P6.6  | recovery | todo                 |          |               |                                                                                                                                                                                |
| P6.7a | cli      | todo                 |          |               |                                                                                                                                                                                |
| P6.7b | cli      | todo                 |          |               |                                                                                                                                                                                |
| P6.7c | cli      | blocked              |          |               | needs Phase 5                                                                                                                                                                  |
| P6.8  | recovery | todo                 |          |               |                                                                                                                                                                                |
