# Recovered original: section-to-requirement crosswalk

Source 04 is a flattened transcript on physical line 13. The byte/character spans and `O-*` occurrence IDs in `statements.jsonl` are the precise citations; the section labels below are reconstructed navigation aids, explicitly provisional. Rows point to existing `O-R*` nodes and their children in `requirements.jsonl` rather than treating a long span as one indivisible outcome.

## Part 0 — coordinator handoff

| Reconstructed passage | Source occurrences | Semantic rows |
|---|---|---|
| Combined request, preservation source and current handoff precedence | O-016–O-024 | O-R040, O-R053 |
| Preservation/authority clause in the recovered wrapper | O-006, O-011 | O-R053 |
| Repository review, verified issue/dispatch, no false receipts | O-025–O-051 | O-R041, O-R047 |
| Reuse Fleet/PM Stack boundaries; avoid competing infrastructure | O-053–O-057 | O-R042 |
| Two design scopes and artifacts | O-058–O-080 | O-R043 |
| Architecture review: outcomes govern, mechanisms are proposals; record rationale/tradeoffs | O-081–O-106 | O-R044 |
| Starter issue, decision list and definition of done | O-107–O-134 | O-R045 |
| Task-specific 450/High and separate Do Now, not runtime mapping | O-135–O-141 | O-R046; CF-001 |
| Discover and invoke actual authorized direct scheduling; verify/bind/report | O-142–O-158 | O-R047 |
| Dispatched-agent operating brief and verified continuation | O-159–O-170 | O-R048 |

## Part 1 — priority and PM Stack design

| § | Reconstructed section | Source occurrences | Semantic rows |
|---:|---|---|---|
| 1 | Purpose and shared legibility | O-174–O-197 | O-R001 |
| 2 | Separate urgency, importance, effective pressure, state, directive and ownership | O-199–O-236 | O-R002 |
| 3 | Proposed numeric bands, cold backlog and manual controls | O-237–O-261 | O-R003–O-R004 |
| 4 | Stable total ordering, clusters, collisions and drift | O-262–O-296 | O-R005; children O-R054–O-R057 |
| 5 | Evidence-informed reprioritization | O-297–O-332 | O-R006 |
| 6 | Time-limited opportunities and escalation | O-333–O-356 | O-R007 |
| 7 | Dependency-driven effective urgency | O-357–O-378 | O-R008 |
| 8 | Triage comparison, sweep, provenance and refinement | O-379–O-392 | O-R009; children O-R058–O-R060 |
| 9 | Boundary hysteresis and bypass | O-393–O-405 | O-R009; child O-R061 |
| 10 | GitHub priority labels and triage | O-406–O-449 | O-R010; CF-004 |
| 11 | Do Now and authorized immediate dispatch | O-450–O-486 | O-R011; CF-006 |
| 12 | Board lifecycle projections, views and continuation context | O-487–O-534 | O-R012 |
| 13 | Interactive mobile surface and agent panel | O-535–O-581 | O-R013 |
| 14 | Comment security, routing, adapters and failure behavior | O-582–O-635 | O-R014; children O-R062–O-R065 |
| 15 | Normal selection and bounded exploration | O-636–O-666 | O-R015 |
| 16 | Weighted exploration formula and controls | O-667–O-687 | O-R016; CF-005 |
| 17 | Selective project/epic scheduling pressure | O-688–O-698 | O-R017 |
| 18 | Source-of-truth and synchronization | O-699–O-703 | O-R018; CF-003 |
| 19 | Queue Table Improvements integration/staging | O-704–O-735 | O-R019 |
| 20 | Priority, selection, visibility and direct-interaction acceptance | O-736–O-777 | O-R020 |

## Part 2 — work-item orchestration

| § | Reconstructed section | Source occurrences | Semantic rows |
|---:|---|---|---|
| 1 | Purpose and standard workflow | O-779–O-781 | O-R021 |
| 2 | Logical component architecture | O-782–O-845 | O-R022 |
| 3 | Proposed Fleet command families | O-846–O-850 | O-R023; see `census/commands.jsonl` and `census/command-slice-map.jsonl` |
| 4 | Common flags and inferred context | O-851–O-852 | O-R024; see `census/commands.jsonl` and `census/command-slice-map.jsonl` |
| 5 | Composed next/open/park/complete workflows | O-853–O-856 | O-R025 |
| 6 | Work identity and logical schema | O-857–O-859 | O-R026 |
| 7 | Atomic claims, leases and fencing | O-860–O-866 | O-R027 |
| 8 | Comprehensive queue/GitHub/context hydration | O-867–O-922 | O-R028 |
| 9 | Durable artifacts and summary | O-923–O-945 | O-R029 |
| 10 | Changes since a checkpoint | O-946–O-955 | O-R030 |
| 11 | Notes, comments and managed content | O-956–O-969 | O-R031 |
| 12 | Transactional outbox | O-970–O-988 | O-R032 |
| 13 | Lifecycle and dependencies | O-989–O-994 | O-R033 |
| 14 | Direct verified completion | O-995–O-1036 | O-R034 |
| 15 | Ordinary queue-worker walkthrough | O-1037–O-1049 | O-R049 (example) |
| 16 | Resume-and-finish walkthrough | O-1050–O-1061 | O-R050 (example) |
| 17 | Live-user-instruction walkthrough | O-1062–O-1088 | O-R051 (example) |
| 18 | Near-timeout walkthrough | O-1089–O-1091 | O-R052 (example) |
| 19 | Failure and recovery cases | O-1092–O-1102 | O-R035 |
| 20 | Implementation sequence | O-1103–O-1142 | O-R036 |
| 21 | Acceptance criteria | O-1143–O-1177 | O-R037 |

## Part 3 — cross-document decisions

| Passage | Source occurrences | Semantic rows |
|---|---|---|
| Priority axes, aging, hysteresis, labels, Do Now, comments, exploration, dispatch safeguards and shared PM service decisions | O-1178–O-1241 | O-R038; relevant conflicts CF-001–CF-006 |

## Part 4 — review, simulation, verification and handoff

| Passage | Source occurrences | Semantic rows |
|---|---|---|
| Repository audit, simulation families, end-to-end verification, deliverables and final reporting | O-1242–O-1290 | O-R039 |

## Part 5 — immediate execution checklist

| Passage | Source occurrences | Semantic rows |
|---|---|---|
| Preserve docs, cross-link Queue Table Improvements, assign task-specific urgency/directive, invoke authorized dispatch, verify binding/receipt, and report truthfully | O-1291–O-1310 | O-R039, O-R047; CF-001, CF-006 |
| Closing Markdown fence | O-1311 | Formatting disposition; not a requirement |

### Reading rule

Occurrence-to-row links in `dispositions.jsonl` are the machine-readable join. A range in this table is a navigation aid, not a claim that every occurrence in the range has the same modality. Requirement/candidate/history/example modality remains on each semantic row and disposition. The source’s alternatives and examples remain preserved; this crosswalk does not promote them to runtime policy or implementation status.
