# Routing tables

Read this in steps 1 and 3 of the router. The stage contract, including what each stage produces, is in [stages.md](../../../../planning/workflow/stages.md). This file only maps state to an action.

## Stopping point from the user's words

Pick the furthest stage the user asked for. The words are examples, not an exhaustive list:

- "capture", "write it down", "intake only", "store it", "set up the package" stop after **S0**.
- "prepare", "preliminary research", "research issues", "get it ready for research" stop after **S1**.
- "research it", "full research", "definitive docs" stop after **S3** (S2 and reconcile).
- "hand off", "make the issues", "fan out" stop after **S5**.
- "design it", "make the decisions" stop after **S6**.
- "plan it", "feature plan", "tickets" stop after **S7**.
- "build", "implement", "ship", "all the way" stop after **S8**.
- Nothing stated: finish the stage the package is at for every ready concept, then stop. Add the question "Continue to S<n+1>? -> default: yes, on the next run".

A "don't do X yet" in the request is a stopping point too. The stop stage you chose and the words it came from go in the router log.

## S0: there is no package yet

Intake is one worker. Dispatch it with the input's path or URL, never the text. If the input exists only in this conversation, no worker can see its bytes. In that case, load `request-intake` and run S0 in this session (it is the one exception to "never write content"), then return to routing.

## Stage to skill

| Stage | Skill                                               | Unit                | Dispatch when                                                                                           |
| ----- | --------------------------------------------------- | ------------------- | ------------------------------------------------------------------------------------------------------- |
| S0    | `request-intake`                                    | the input           | no package holds this input's sha256                                                                    |
| S1    | `research-prepare`                                  | concept             | S0 merged                                                                                               |
| S2    | `concept-research`                                  | concept             | S1 merged and `type` is not `hint`                                                                      |
| S3    | `notes-reconcile`                                   | concept             | every concept's S2 merged (notes can arrive until then) and `check --remote-notes` shows none in flight |
| S4    | `issue-handoff` mode `parent`                       | concept             | S3 merged and `check --require-resolved` passes for it                                                  |
| S5    | `issue-handoff` mode `fanout`, then `orchestration` | parent issue        | S4 issue exists                                                                                         |
| S6    | `design-orchestrate`                                | orchestration issue | S5 orchestration issue exists                                                                           |
| S7    | `feature-plan`                                      | design              | S6 design merged and its human-review list published                                                    |
| S8    | `implementation-orchestrate`                        | feature plan        | S7 plan merged with its issues                                                                          |

## Run status to action

The run status is the one for the concept's current stage, as `rw status --live` reports it:

- `not-started`, concept in the ready set, stage within the stop point: **dispatch** the stage's skill.
- `not-started`, not in the ready set: **wait**. Log the `requires` ids that block it, or the `shares-files-with` partner that is running.
- `claimed` or `running`: **monitor**. Check its log's last line and its lease. If either is stale, it is a restart candidate.
- `pushed` (commits on the branch, no PR): **monitor** while the worker is alive. If it ended without a PR, **restart**.
- `pr-open`: **verify-and-merge** once the worker has reported. Before that, monitor.
- `merged`: if the stage is `validated` or needs no validation, the concept moves to the next stage. Back to the top of this list.
- `blocked`: read the reason in its PR or log. On a human question, **ask** (batched). On another concept, **wait**.
- `abandoned`: if the branch has commits, **restart** on it. If it has none, **dispatch** fresh.

## Barriers and special cases

- **S3 barrier.** Reconcile starts only after every concept's S2 has merged, because a late S2 can still send notes. A concept with no incoming notes can skip ahead once `rw check --package "$PKG" --require-resolved` passes.
- **Hints.** A `type: hint` concept gets S1, which prepares the questions that would resolve it, and then waits. It moves only when a note or a human answer changes its type. Its questions join the batched human list.
- **Soft edges.** A `recommended-after` concept may start early only if a worker would otherwise sit idle. Its brief must then state the `rework_if_early` cost ([ordering.md](../../../../planning/workflow/ordering.md)).
- **Contingent concepts** (`optional-if`) are dispatched only once their `trigger` is met.
- **Nested orchestrators.** S6 and S8 run their own workers. If the harness lets subagents spawn subagents, dispatch one orchestrator subagent per unit. If it doesn't, load the orchestrator skill in this session and run its loop yourself.
- **Decisions.** The router never picks between conflicting results. It routes them to the decision protocol in [decisions.md](../../../../planning/workflow/decisions.md), and work continues on the preliminary choice.

## Single-concept shortcut

One concept still gets the full package layout. Its stages run like this:

- **S1 and S2:** one worker, one branch per stage. If the concept fits in one session, the same session can run S2 right after S1 has merged.
- **S3:** a check only. Run `rw check --package "$PKG" --require-resolved`. There is nobody to send or receive notes.
- **S4 and S5:** as normal. The parent issue is still needed for the zero-context handoff.
- **Waves:** none. The run table has one row.
