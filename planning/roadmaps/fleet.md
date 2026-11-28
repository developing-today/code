# Roadmap: Fleet

The fleet is the set of agents that work this repository and the ones around it, and the tool they do it through: the `fleet` binary (`pkgs/fleet`) with its queue, dispatch, search and proxy, the lanes that claim tasks and work them, and the pools of models and machines they run on. Where its work is planned and watched is the PM stack ([`pm-stack.md`](pm-stack.md)); this file is where the fleet itself is going.

Written on 2026-11-23 from the programme's epics. A row's status is read from its issue's own labels on that day: an issue in review or in progress is `in-progress`, one filed with its steps and not started is `planned`, a design is `shaped`.

## Now

| Title                                                            | Status      | Intent                                                                                                                                | Links        |
| ---------------------------------------------------------------- | ----------- | ------------------------------------------------------------------------------------------------------------------------------------- | ------------ |
| The fleet tool: one door for coordination, mirroring and backups | in-progress | One tool owns every coordination route and every fallback, so no agent reaches a database, a mirror or a backup by a road of its own. | #8990, #8575 |
| Unification slice 3: the verb contract v2                        | in-progress | Every verb says its routes and what it does in a degraded mode, in the tool and not in each caller.                                   | #8581        |
| Unification slice 5: a local coordination replica                | in-progress | Work goes on when the database cannot be reached.                                                                                     | #8583        |
| Queue fleet reliability                                          | in-progress | A lane worker for every queue and certified shift reports, so the fleet runs for a week at a time without an operator's repairs.      | #1233        |
| Every lane on the intake ladder                                  | in-progress | Every lane takes its work by the same intake ladder.                                                                                  | #12236       |
| Ready pull requests are merged                                   | in-progress | A pull request that is ready is merged by the sweep, not left standing.                                                               | #12280       |
| Closure records                                                  | in-progress | Every close carries its reason, so the audit trail is there by construction.                                                          | #9221        |

## Next

| Title                                                                | Status      | Intent                                                                                                                                                      | Links                      |
| -------------------------------------------------------------------- | ----------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------- |
| Unification slice 7: label de-mirroring and the final reconciliation | planned     | The last of the old mirrors goes, and what is left is checked against the one home per fact.                                                                | #8585                      |
| Queue: resume by session, claim any task, manual-only items          | planned     | A task can be picked up where a session left it, and some are only ever started by hand.                                                                    | #9178                      |
| Pool and routing intelligence                                        | in-progress | Lanes on every queue, live dispatch and babysitting, a steward for quota, a reviewer of the reviewers, and model choices measured against our own sessions. | #1334, #1340, #1342, #2732 |
| Fleet expansion and placement                                        | in-progress | The fleet takes on the repositories it touches and does not manage, and work is placed by a written table.                                                  | #2055, #2230, #4986        |
| Orchestration and dispatch                                           | shaped      | The design family for sweeps, review jobs and dispatch, under one tracking home.                                                                            | #12950                     |
| More machines as job nodes                                           | in-progress | Machines join a pool and placement is designed around it. The PM stack's side of this is a second machine under its own key (#13492).                       | #2179                      |
| Connection performance                                               | in-progress | A claim in one round trip, and streams that share one tunnel.                                                                                               | #5757, #5756               |
| Fleet CA, credentials and secrets                                    | shaped      | The design for the fleet's certificate authority, its credentials and its secrets.                                                                          | #9155                      |

## Later

| Title                     | Status  | Intent                                                                              | Links |
| ------------------------- | ------- | ----------------------------------------------------------------------------------- | ----- |
| Content-creation throttle | planned | A shared ledger and pacing budgets, with what was filed checked by reading it back. | #2227 |
