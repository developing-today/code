<!-- Copyright developing.today LLC & contributors. SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Request coverage for the documentation framework

This table restates the [raw request](https://github.com/dezren39/cairn/blob/28c91a61309a5c6fd143fbc6da13ad889aaa2821/other/2026-10-19_research-design-workflow_request.txt) as checkable responsibilities. It does not replace the transcript. See the [main design](2026-10-19T06-09-02Z_design_research_workflow.md) for decisions and tradeoffs.

| ID  | Requested behavior                                                                                              | Owner and evidence                                                                |
| --- | --------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------- |
| R01 | Pull and inspect code; review all Cairn files and skills                                                        | Recorded baselines, read-only review inventories, and raw transcript commit       |
| R02 | Keep the exact original input, including rambling, typos and TTS uncertainty                                    | request-intake; transcript SHA and nonblank-line coverage checks                  |
| R03 | Cleanly restate concepts while keeping excerpts and inferred additions distinct                                 | request-intake and research-prepare; per-concept framing and source mapping       |
| R04 | Use one intake agent; dated main reference only when useful; list every created/edited topic file               | request-intake; multi/single scaffold fixtures                                    |
| R05 | Precreate small substantive stubs with links and relevant excerpts, avoid speculative meta files                | init command and no-empty-note-tree checks                                        |
| R06 | Include an introduction, initial thoughts, stage outcome placeholders, related changes and initial review order | main/design/changes/concept scaffold contract                                     |
| R07 | Show not-started/running/completed status and parallel groups; no false dependencies                            | embedded state, ready view, typed scheduling checks                               |
| R08 | Do only preliminary exploration during preparation; hand off full research later                                | research-prepare stop condition and next-stage contract                           |
| R09 | Research each concept deeply with citations, prior art, anecdotes, feasibility and alternatives                 | concept-research contract and evidence templates                                  |
| R10 | Own one file or use directional to/from notes; allow all directed pairs without precreating them                | collaboration contract and lazy note tool                                         |
| R11 | Reconcile every cross-agent comment before the next handoff                                                     | notes-reconcile and require-resolved gate                                         |
| R12 | A zero-context issue can trigger granular investigation/design issue fanout                                     | issue-handoff modes, self-contained templates and duplicate lookup                |
| R13 | List related and new issues/PRs inline, in a bottom index and in the design/order record                        | main record and issue-handoff/index responsibilities                              |
| R14 | Orchestrate investigation/design issues in parallel and record findings without erasing earlier stages          | design-orchestrate, stage-tagged status/history and worker receipts               |
| R15 | Mark superseded, clarified, revoked or declined claims with time, origin and precise replacement link           | record schema and appended transition history                                     |
| R16 | After preliminary design, create the feature planning handoff and granular plan                                 | feature-plan; links, requirements, steps, tests and rollout                       |
| R17 | Distinguish hard prerequisites from soft rework, crucial from recommended, optional enablers and future paths   | dependency vocabulary, independent priority/criticality/horizon fields            |
| R18 | Each writer uses an appropriate worktree, immediately published branch and PR; only merge commits               | coordination and implementation-orchestrate; actual worker branches and PRs       |
| R19 | Sharing is allowed for read-only or clearly disjoint work with explicit ownership                               | collaboration exception with shared resource checks                               |
| R20 | Orchestrators avoid implementation, verify actual completion/publication/merge, restart incomplete workers      | orchestrator stage gates and resume contract                                      |
| R21 | Disputed designs get reviewer, critic and investigator prompts; look beyond a narrow option set                 | design-orchestrate review prompts and provisional decision log                    |
| R22 | Continue on reversible defaults and independent work; collect a minimal human decision list                     | router and orchestrator escalation policies                                       |
| R23 | Create a final implementation orchestration handoff after the plan and issue graph                              | feature-plan to implementation-orchestrate transition                             |
| R24 | Improve agents, README and existing conventions; support standard and unusual reference material                | code documentation architecture, local profile and Cairn skill/provenance changes |
| R25 | Bring useful Cairn skills here, customized without brittle home-directory dependencies                          | generic library mirror, repository profile and protected sync command             |
| R26 | Be ready for the next complex multipage input without doing all future research now                             | end-to-end fixture intake and explicit requested-stage stop                       |
| R27 | Retain deferred/rejected features with reasons and end goals rather than lose ideas                             | status/history vocabulary, portfolio dependency view and reconsideration triggers |
| R28 | Keep changes through PRs and record substantial evolution after initial creation                                | merge-only worker and integration PRs, per-package change history                 |

Live product issue orchestration is not claimed by static fixtures. Future runs must verify the issue, branch, PR and check state they actually observe.
