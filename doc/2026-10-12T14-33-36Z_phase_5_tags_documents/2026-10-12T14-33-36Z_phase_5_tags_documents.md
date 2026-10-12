# Phase 5: tags, documents, and search

- Plan: [phased plan](../../2026-10-11T11-08-42Z_design_remaining_work/plan-phases.md), section 10.
- Start commit: `c12448c9` (origin/main after Phase 3).
- Base branch `phase-5`. Tracks: `p5-tags` (P5.1, P5.2), then `p5-compact` (P5.3), `p5-docs` (P5.4, P5.6), `p5-search` (P5.5), `p5-alias` (P5.7).
- Decisions in force: tags are claims on any event or document ID, anyone may tag anything, attributed to the signing key; observed-remove clears; user subject is the first identity event (owner); aliases are tags shown contested, with server primary, prefix disambiguation; compaction by snapshot at a causal cut; content on removal kept.

## Steps

| Step                         | Track   | Status | Commits | Tests | Notes                  |
| ---------------------------- | ------- | ------ | ------- | ----- | ---------------------- |
| P5.1 Observed-remove tags    | tags    | todo   |         |       | S8 model               |
| P5.2 Tag index on tuple keys | tags    | todo   |         |       |                        |
| P5.3 Compaction              | compact | todo   |         |       |                        |
| P5.4 Documents               | docs    | todo   |         |       |                        |
| P5.5 Search                  | search  | todo   |         |       |                        |
| P5.6 Content on removal      | docs    | todo   |         |       | decision 4             |
| P5.7 Aliases as tags         | alias   | todo   |         |       | owner answers recorded |

## Decisions

- See above and the index.

## Open questions

- User tag subject needs an identity event (issue).

## Changes to the plan

- None yet.

## Review notes and regression tests

- To be filled.

## End

- Pending.
