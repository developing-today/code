# The OpenCode and OpenChamber forks

The project maintains forks of OpenCode and OpenChamber — the agent harness
and its chamber/session surface that the fleet's OpenCode sessions run in.
The forks carry the project's routing, quota and plugin adaptations: the
provider pools and their ordering doctrine, the mcpx plugin, and the fixes
that upstream releases do not carry for this fleet's way of working. They
are high-volume working repositories in their own right: in the
retrospective's merged-PR spine, the OpenCode fork merged 181 PRs in W43 and
366 in W44, and the OpenChamber fork 181 and 365 in the same weeks.

## The problem the forks programme answers

The two forks duplicate machinery. AUDIT-7's Lane 7 census (18,594 items,
report PR #13904 merged) named the forks its top convergence candidate:
quota and routing fixes recur in both forks — 249 titles in one, 201 in the
other — and across the census window the forks' work was 66 percent
fix-titled. That reactive stream is the finding merged-list item 27 answers:
converge the duplicated machinery into one shared routing implementation,
and finish the OpenCode 2 configuration and plugin migration (#730, #3286,
the mcpx plugin v2 remainder #2841).

## Current state (2026-11-23)

- Both forks are in flight, worked daily. OpenCode 2 configuration
  validation (#730) was active on 2026-11-23; the fleet pool and routing
  intelligence epic (#1334) had slices landing the same day.
- Fork sync is fleet infrastructure, not fork-local: since 2026-11-22,
  pushes to the fleet repository's main branch sync to all 102 forks, with
  failures collected into a todo issue in this repo (first filed as #12484;
  after the token fix, #12490 with 2 forks failing).

## Where its detail lives

This chapter is the high-level treatment, by design. The detailed treatment
is the liftable forks section of the Project Management book, carried under
the PM-BOOK programme tracker
[#13900](https://github.com/developing-today/code/issues/13900) — the forks
programme is PM-primary in the merged list's assignment table (item 27).
The convergence evidence is AUDIT-7 Lane 7,
[#13874](https://github.com/developing-today/code/issues/13874).
