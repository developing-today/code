# Design: PM Stack & Fleet orchestration — priority, dispatch, and work-item lifecycle

**Status:** design-mill intake scaffold (2026-11-13). Starter epic: #9196. This directory is the
preservation + design home for the PM Stack & Fleet orchestration program. The bound Summit
session owns the build-out from here: statement inventory to atomic coverage → requirement
crosswalk → state-of-the-art expansion → versioned component designs → granular issues →
coverage audit.

**Program in one paragraph.** Upgrade the existing Fleet multicall binary and PM Stack — not a
replacement system — into the shared control plane for queue-driven agents, along two coordinated
tracks: **(A)** urgency/triage/PM visibility/direct dispatch/mobile control, and **(B)** work-item
lifecycle/context hydration/typed dependencies/journal/outbox/direct verified completion.
Preservation comes first: the recovered requirements are source requirements, not a list to
shrink around today's code. Recent repo work is integration evidence and a collision check,
never a scope limit.

**Intrinsic urgency:** 450 (High) per the charter. **Dispatch directive:** Do Now — a separate
one-shot directive, not a score.

## Source map

All sources are preserved verbatim (plain-text exports) in `sources/`, staged immutably on the
laptop at `~/.local/share/fleet/intake/pm-orchestration-20261112/` with hashes re-verified after
transfer. Precedence: the handoff governs the intake; where the documents disagree, the order is
charter → proposal → recovered original, and disagreements are recorded in the statement
inventory (§3), not silently resolved.

| #   | File                                                 | Source                                                                                                                                     | Role                                                                                                                             | SHA-256 (export)                                                   |
| --- | ---------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------ |
| 01  | `sources/01_handoff_pm_stack_fleet_orchestration.md` | [Google Doc 1zPCTo_a4p5qiVts2HEFL0WXKeh9_KiS7oQ3mTprKYEQ](https://docs.google.com/document/d/1zPCTo_a4p5qiVts2HEFL0WXKeh9_KiS7oQ3mTprKYEQ) | Governing intake document (2026-11-12)                                                                                           | `e293349ec31e997ac3330d8df947f121e8aba537477b1bfd6e6f8290b85675c0` |
| 02  | `sources/02_charter_v1_1.md`                         | [Google Doc 1_DQwNbuRmJnITDKKJJ7A3Q0FJa-q2rkzWlz-4PyA_iw](https://docs.google.com/document/d/1_DQwNbuRmJnITDKKJJ7A3Q0FJa-q2rkzWlz-4PyA_iw) | Execution Charter v1.1 (2026-11-06) — governing requirements text for both tracks                                                | `293925e58f5b19f319ec494665ec7646d36f9bdb1f823501ed8211ad8dfd9136` |
| 03  | `sources/03_full_orchestration_proposal.md`          | [Google Doc 15md8Zff2BTQjZ5ZJE7k3QyAXm_d258IN_CB2ojsV1LA](https://docs.google.com/document/d/15md8Zff2BTQjZ5ZJE7k3QyAXm_d258IN_CB2ojsV1LA) | Full supporting work-item orchestration draft                                                                                    | `7d24690d14409e4dc32876a50d1a7ac19fe3599b0ecaed7ae909009baec695d1` |
| 04  | `sources/04_recovered_original_combined_design.md`   | [Google Doc 1h8zF-uWTFuXQZ2KFeAPg-aNXR_S3BY9ebKtr3pc6Tg8](https://docs.google.com/document/d/1h8zF-uWTFuXQZ2KFeAPg-aNXR_S3BY9ebKtr3pc6Tg8) | Recovered original combined design ("Naming Fleet Agents" conversation; formatting flattened in recovery, per the source itself) | `6607fba35aee81e7e4e521e8ff7cb6ee31280823c667b9c22f3766fd30edff86` |
| 05  | `sources/05_supplemental_raw_note.md`                | [Google Doc 1MSdUt97SnWIzgqyglMJuSzTMq4RKtoThQq1a4JVVZvU](https://docs.google.com/document/d/1MSdUt97SnWIzgqyglMJuSzTMq4RKtoThQq1a4JVVZvU) | Supplemental raw note — design candidates, NOT an overriding directive                                                           | `d5855c278a7f7276adb7bf32ea25da1b41ab978fb3d9e207154d1803d4cf1121` |

Upstream conversation anchors (named in the sources; the recovered text of the first is source 04):
`chatgpt.com/c/6aedd6ba-93dc-83ea-a1cd-78fe1b234f0a` (original combined design + user
prioritization specification) and the recovered historical short prompt at
`chatgpt.com/s/t_6af66737e9348191849d403726fcd631`, which names **Queue Table Improvements** as
an explicit discovery anchor.

## Domain starter index (native sub-issues of #9196)

| Starter | Domain                                                                                                                |
| ------- | --------------------------------------------------------------------------------------------------------------------- |
| #9198   | Requirement crosswalk & current-state census (the mill's first deliverable; locates Queue Table Improvements by name) |
| #9199   | Distributed priority model — urgency, importance, effective pressure (Track A)                                        |
| #9200   | Priority distribution, provenance & reconciliation across producers/consumers (Track A)                               |
| #9201   | PM projection & mobile control surface (Track A)                                                                      |
| #9203   | Direct dispatch, slash commands & event routing (Track A)                                                             |
| #9204   | Queue selection & bounded exploration (Track A/B)                                                                     |
| #9206   | Work-item lifecycle & fleet subcommand surface (Track B)                                                              |
| #9207   | Context hydration & incremental activity (Track B)                                                                    |
| #9208   | Typed dependencies, notes, journal & field authority (Track B)                                                        |
| #9209   | Leases, fencing, transactional outbox & direct verified completion (Track B)                                          |

Collision map (consume / extend / supersede verdicts for #1184, #9060 + children, #9078 +
children, #9178, #9180, #9179, #9016/#8887, and the handoff's discovery anchors #8187, #6533,
#7030): recorded in full on #9196. Headline: #1184 is at GitHub's 100-sub-issue cap, so #9196 is
parented under the fleet-tool epic #8990 and cross-linked to #1184 (comment 6842760443), per the
#9060 design's intermediate-grouping rule.

## Statement inventory — skeleton

The Summit refines this skeleton to atomic statements, numbered, each mapped to a component
design or a named existing issue. Zero unmapped is the pass bar. ID scheme: `H-` handoff,
`C-` charter (by section), `P-` proposal (by section), `O-` recovered original, `S-` supplemental
note. Each statement's crosswalk classification: verified existing / partially implemented / open
implementation / design-only / research / superseded with rationale / blocked / unverified.

### H — Handoff (source 01)

- H-INTAKE: preservation-first rule; two-track preservation; no duplicate live session; verified launch receipt standard (label/queue position/session shell ≠ launch).
- H-RECONCILE: pinned-commit census; discovery anchors #9060, #9078, #9091/#9102, #9106, #8187, #6533, #7030; urgency taxonomy mapping (450/1000 scheme → current ratified taxonomy, ascending order preserved, no silent renumbering).
- H-SURVIVE-A: priority mechanics that must survive refinement (axes separation, dependency pressure, boosts, aging, hysteresis, overrides, label normalization, single-writer projection, mobile control, slash commands, direct dispatch, durable receipts, idempotent webhooks).
- H-SURVIVE-B: lifecycle mechanics that must survive (atomic claim/fencing, hydration bundle, cursors, attribution, notes vs comments, managed sections, checkpoints, outbox, direct verified completion).
- H-EXPLORE: bounded exploration parameters as suggestions (4%, three candidates, r=1.25/d=200/α=0.5, ε=0.05) — preserve outcomes, not arbitrary constants.
- H-SCENARIOS: end-to-end scenario set (fresh work; blocked/parked resume; late external comment; near-timeout checkpoint; crash after remote mutation; duplicate webhook; stale lease; conflict; live launch/bind proof).
- H-DISTRIBUTED: distributed priority as its own design question (producers/consumers, precedence, versioned payloads, provenance/reason codes, replay/reconciliation, cross-target ordering validation).
- H-COMMANDS: the full command/feature inventory (discovery, queue, work, GitHub, metadata, dependencies, activity/sync, agent control, shared flags) preserved explicitly, even where implementation names change.
- H-QTI: "Queue Table Improvements" discovery anchor — locate by name, crosswalk before assuming equivalence.

### C — Charter v1.1 (source 02), by section

- C-1 mandate & two tracks; C-2 coordinator duties (execute, pin commit, two design documents, urgency 450 + Do Now separate, verified dispatch); C-3 four-axes mental model; C-4 bands & stable total order; C-5 dynamic triage (clump refinement, hysteresis, boosts, dependency pressure); C-6 labels/Projects/PM projections (single writer, board states); C-7 mobile-first operations; C-8 commands/comments/event routing; C-9 selection & bounded exploration (formula + defaults); C-10 lifecycle primitives & CLI surface; C-11 hydration & incremental activity (bundle layout, cursors, attribution); C-12 notes/field authority/dependency semantics; C-13 lease & outbox reliability; C-14 direct verified completion; C-15 end-to-end scenarios A–D; C-16 sequencing & acceptance; C-17 evidence & reporting distinctions; C-18 source references & final principle.

### P — Full proposal (source 03), by section

- P-1 executive summary; P-2 goals & principles (control plane, primitives + sugar, persist comprehensive/return concise, canonical identity, interruption safety, state separation); P-3 component architecture (queue broker, hydrator, GitHub adapter, activity reconciler, dependency resolver, work journal, mutation coordinator, sync outbox); P-4 command structure (bootstrap, queue, work, GitHub, metadata, dependency, activity, flags); P-5 composed workflows (next/open/park/complete); P-6 work-item data model; P-7 priority/ranking/selection; P-8 context hydration; P-9 hydration files & agent context (summary contents, example output); P-10 titles/descriptions/comments authority (field ownership, selective sync, managed sections, managed comments); P-11 activity tracking (attribution, cursors, timestamps, completion-time reconciliation); P-12 dependencies & blockers; P-13 lifecycle & ownership (leases, checkpointing, time budgets, and following sections to be enumerated at atomic pass).

### O — Recovered original (source 04)

- O-PART1: original Part 1 §§2–17 (priority system in full: bands/defaults, distribution across lanes, clump shape example 50→25/10/5/singles, aging, inheritance, hysteresis, label normalization, exploration dispositions, weighted-selection formula) — each mechanism preserved alongside any better replacement, with the mapping explained.
- O-PART2: original work-item orchestration specification (schema, lifecycle, hydration, commands, worked examples, error/receipt semantics).
- O-PROMPT: the recovered historical short prompt and its dispatch requirements (historical record; current dispatch follows source 01).

### S — Supplemental note (source 05)

- S-AGING: bidirectional aging examples (week-old medium rises; post-pivot high falls); S-BANDS: rough range definitions for priority levels; S-LABELS: label-change webhook behavior (priority removed → top of triage; do-now label ≠ priority change); S-STATE: picked-up/parked status + comment conventions; S-READY: high-and-above picked-up-by-Drew → todo/ready, everything else backlog; S-COMMANDS: `/`-command handling by bots/webhook (retry/queue/start/merge); S-MOBILE: mobile-friendly interactive board + views; S-SMEAR: clump smearing mechanics (±10 neighbor moves, collision push-back, multiples-of-25 managed updates, oldest-first ties); S-EXPLORATION: catch/kill, catch/release disposition hints; S-RATIO: the weighted-selection rationale in Drew's own words.

## Version plan (sketch — the Summit owns the real one)

- **V1 — what exists today:** fleet queue + broker (Turso), claimant/dispatch paths, orchestration pass, PM-stack aggregator/projector, Project #4 projection, current urgency conventions (450/High in live use), #8187 launch-on-handoff, #9178 claim semantics. Established by the census (#9198).
- **V2 — contracts:** priority model + distribution contracts, lifecycle/CLI contracts, hydration bundle + cursor contracts, outbox/completion contracts — as reviewed component designs with ADRs (FDRs per #9180).
- **V3 — staged implementation:** granular implementation issues executed through the queue in dependency order (shared schema changes reconciled with Queue Table Improvements first).
- **V4 — hosted parity:** contracts carried into the hosted editions (#9079 PM service, #9081 fleet-tool equivalent) without semantic drift; cross-target ordering validation.

## Open questions (with intake defaults — the Summit records its own on #9196)

1. Where do the two charter-named design documents live? Charter suggests `docs/design/fleet-priority-urgency-pm-stack.md` and `docs/design/fleet-work-item-orchestration.md`; this repo's current convention is `doc/<timestamp>_design_*/`. **Default:** follow the repo's current `doc/` convention for new designs (this directory), and record the deviation + rationale in the census.
2. Is importance a stored field or a derived/provisional dimension at V1? **Default:** design both, stage stored-field behind the distribution design (#9200); never fabricate importance from urgency.
3. Does this mill's lifecycle design extend or supersede #9178's claim semantics? **Default:** extends — #9178 remains the baseline; changes route through it and its FDRs.

## Safety gates

Design + granular issues only. No live queue-semantics changes, no production dispatch changes,
no live queue schema migration, no new queue/scheduler/database/binary. Implementation follows
through the queue under the filed issues. See #9196 for the full gate list.
