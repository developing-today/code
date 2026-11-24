# Fleet orchestration & dispatch (F4) — family design

- Status: draft — family boundary and composition are proposed; child decisions remain authoritative
- Date: 2026-11-24T01:37:37Z
- Roadmap item: `pm-stack/design-register-2026-11-11.md`, §F4
- Issues: [#12950](https://github.com/developing-today/code/issues/12950) (tracking/rollup); [#8986](https://github.com/developing-today/code/issues/8986) (design-register hub)

## 1. Problem and context

F4 collects the fleet's orchestration and dispatch work: work selection, queue claims, placement, quota admission, routes, execution surfaces, and agent supervision. Its children already have running-system contracts, decisions, issue bodies, or focused designs, but no family record explains how those pieces compose or where their authority ends. The F4 inventory in the [design register](../../pm-stack/design-register-2026-11-11.md) is the current map, not a cross-cutting design.

This record addresses that missing family-level seam. It does not replace the child designs or claim that every item is complete. The register is a point-in-time inventory and must be rechecked before implementation planning.

## 2. Goals and non-goals

### Goals

- Give F4 a single navigable map of child work, authoritative artifacts, and recorded state.
- State a composition model for the already-defined queue, placement, admission, execution, and supervision boundaries.
- Make gaps explicit and turn each into a separately tracked child design or verification task before implementation is authorized.
- Keep the family record high-level; preserve child ownership of detailed contracts and implementation slices.

### Non-goals

- Re-designing or authorizing implementation of any child item.
- Replacing the one-queue, placement, fleet-front-door, quota, route-pool, or agent-supervision decisions already recorded elsewhere.
- Treating the PM-stack project named `pm-stack-fleet-orchestration` as this F4 family. That package concerns PM-stack priority/dispatch and is adjacent, not equivalent.
- Reopening decisions or inferring owner approval from a design-register status.

## 3. Alternatives

1. **Do nothing; keep the register as the only family map.** Lowest effort, but leaves no statement of composition or boundaries and repeats the gap identified for #12950. Rejected.
2. **Consolidate all F4 child designs into one monolithic specification.** Would duplicate child decisions, drift as implementation evolves, and make ownership ambiguous. Rejected.
3. **Family index plus focused child records and explicit gap work.** This document sets the family contract and links evidence; child issues and designs remain authoritative for specifics. Chosen because it adds the missing composition layer without copying or silently changing decisions.

## 4. Design and contracts

F4 is a composition of existing seams, not a new central scheduler. A normal work item passes through these conceptual stages; implementations may combine them, but must preserve the contracts at each boundary:

1. **Eligibility and claim:** the unified queue determines availability and the worker's claim-time requirements compose with task requirements. Lane names/platform identity do not become hidden eligibility rules. See [one-queue model](../../doc/living/one-queue-model.md), #8097/#8438 and the broader queue-state design #8575.
2. **Placement and execution surface:** the placement system selects an eligible execution surface according to its declared capabilities and fallback behavior. New locations are registry entries; new placement kinds use a registered adapter. Laptop, Actions, and Codespaces retain their distinct workload roles. See [task-placement framework](../../doc/2026-11-03T15-10-56Z_design_task-placement-framework-v1/) and the [fleet transport & pooling design](../../doc/2026-11-23T21-10-36Z_design_fleet-transport-pooling/2026-11-23T21-10-36Z_design_fleet-transport-pooling.md).
3. **Admission before launch/resume:** quota and policy gates are evaluated before a job consumes a route or execution seat. A quota failure is a wall except for an explicitly documented reserve; it is not permission to fall through to metered spend. See [quota admission](../../doc/2026-11-14T18-05-00Z_design_fleet_quota_admission/2026-11-14T18-05-00Z_design_fleet_quota_admission.md) and [quota full-utilization burn plan](../../doc/living/quota-full-utilization-burn-plan.md).
4. **Route selection:** route pools own candidate selection and failover; callers request a managed route rather than choosing a leaf model, except for a documented capability reason. Time-based priority and stickiness are layered according to [routing policy](../../.opencode/instructions/routing-policy.md) and the [time-based route priority design](../../doc/2026-11-17_design_time_based_route_priority.md).
5. **Lifecycle and supervision:** the fleet front door owns user/agent-facing command routing and reports durable outcomes; supervisors detect stalled, dropped, or invalid work and act within their declared authority. See [front-door design](../../doc/2026-11-21T11-30-00Z_design_fleet-agent-front-door/) and the F4.1/F4.7 entries in the register.

### Family map (register snapshot 2026-11-11; follow links for live state)

| F4 item | Focus / current record of authority                                                            | Family disposition                                                                                                           |
| ------- | ---------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| F4.1    | Sweep/review job family; living record `doc/living/fleet-sweep-review-job-family.md`           | Existing scheduled-job behavior; verify current job state in its issue/runtime records.                                      |
| F4.2    | Routine-task architecture; `doc/living/routine-task-architecture.md`                           | Shipped retrospective; not a future dispatch specification.                                                                  |
| F4.3    | Pool/routing intelligence; #1334 and its children                                              | Separate program epic; keep its quota, dispatcher, and supervision decisions linked, not copied.                             |
| F4.4    | Executor expansion; #2169, #2170, #2171, #2174, #2177, #2176, #2072                            | Several distinct workstreams; no umbrella contract is inferred here.                                                         |
| F4.5    | Quota full-utilization; `doc/living/quota-full-utilization-burn-plan.md`, #2173                | Existing operational plan; quota admission remains a separate gate.                                                          |
| F4.6    | Machine pool; #2179; related #13752                                                            | Child design owns machine/transport specifics.                                                                               |
| F4.7    | Minder/catcher/advocate; `doc/living/fleet-roles-minder-catcher-advocate.md`                   | Existing role model; current surface parity and authority need verification.                                                 |
| F4.8    | One queue; `doc/living/one-queue-model.md`, #8097/#8438, #8575                                 | Queue eligibility and durable-state ownership.                                                                               |
| F4.9    | Lane naming; `doc/living/lane-naming-law.md`                                                   | Naming is presentation/registry structure, not an eligibility substitute.                                                    |
| F4.10   | Laptop agent; #5343 and #5769                                                                  | Execution/session-proxy lifecycle is a child concern.                                                                        |
| F4.11   | Queue/secrets brokers; #5362 and broker-family follow-ons                                      | Broker architecture is adjacent infrastructure, not a second queue authority.                                                |
| F4.12   | Secrets broker read-through; #6275                                                             | Credential acquisition is a prerequisite concern, not placement policy.                                                      |
| F4.13   | `fleet` multicall; #5412 and its design record                                                 | Command surface; compose with, do not supersede, queue and placement contracts.                                              |
| F4.14   | Execution surfaces and placement; #5413, placement docs                                        | Surface capabilities and fallback are the placement authority.                                                               |
| F4.15   | Queue-state unification / `fleet` single door; #8575, slices #8579–#8585                       | Durable truth and failure behavior; implementation status must be checked live.                                              |
| F4.16   | Provider browser-auth renewal; #6120                                                           | Credential availability supports dispatch but is not orchestration itself.                                                   |
| F4.17   | Agent SDKs and always-on swarms; investigation goal                                            | Investigation, not an adopted dispatch contract; unresolved names stay unresolved.                                           |
| F4.18   | Time-based route-pool priority; #8838 and `doc/2026-11-17_design_time_based_route_priority.md` | Design exists; the issue's Nov 17 ground-truth comment says runtime support was absent, and implementation remains separate. |

## 5. Dependency-ordered slices and acceptance

This family design itself is documentation-only. It does not authorize runtime changes. Follow-on work should be split and ordered as follows:

1. **Evidence reconciliation (prerequisite):** verify each row against current issue state, source docs, and runtime where claims concern live behavior. Acceptance: every child is marked `implemented`, `designed`, `investigating`, or `unverified` with dated evidence; no register snapshot is presented as live truth.
2. **Missing child records (after reconciliation):** create focused dated designs only for gaps that remain in scope (notably F4.15's whole seven-slice design and any undocumented F4.4/F4.6/F4.17/F4.18 decisions). Acceptance: each record names its source issue, contract, alternatives, slices, verification, and unresolved decisions; no family table duplicates the detailed design.
3. **Cross-boundary conformance (after authoritative child contracts exist):** specify testable queue→placement→admission→route→execution handoffs and failure behavior. Acceptance: each transition has an oracle for deny, retry, failover, and terminal reporting, including that no work is silently dropped and no quota wall becomes paid fallback.
4. **Implementation planning (only after the above):** create issue-linked, dependency-ordered implementation slices under the owning child issues. Acceptance: every slice identifies its changed interface, failing-first test, done check, rollout/rollback, and operator evidence; shared interfaces precede parallel consumers.

The record does not create these follow-on issues or claim the acceptance checks have passed.

## 6. Rollout and rollback

No runtime rollout is part of this record. The documentation change is reversible by reverting its PR. If a future child design changes a family contract, that child must explicitly supersede or amend the relevant statement here through a new dated record or an appended active-rollout update; do not silently rewrite this snapshot.

## 7. Verification plan

- Check every in-repository relative link in this document resolves from the design file; check issue links with GitHub at implementation-planning time.
- Compare the family map against the current F4 register and live child issue states before treating it as current.
- For the cross-boundary conformance slice, test the contract at each handoff with named oracles: eligibility decision, placement choice/fallback, admission result, route leaf/failover, and terminal lifecycle report.
- Confirm explicitly that documentation-only changes do not alter queue, placement, quota, route, or supervisor behavior.

## 8. Open questions

- **F4.15 full design lineage and current slice state:** Drew settles any product-level boundary questions on #8575; the maintainers of #8575/#8580–#8585 provide current slice evidence. Resolve the authoritative links before implementation planning.
- **F4.18 implementation reality:** Drew settles product scope; the implementation owner for #8838/#8805 verifies whether the designed behavior has shipped. The issue's Nov 17 comment establishes absence at that date, not current state.
- **F4.17 names “Rockbot” and “Dots”:** Drew is the source of the unresolved names in the register; this family design does not guess their identity.
- **Multicall D1–D8:** decisions belong to #5412 and its decision comments; Drew settles any missing product picks. Verify the issue history before reusing them as settled family-wide constraints.
- **Surface parity and residual child gaps:** Drew settles product-level scope; the owning issue maintainers provide current OpenChamber/Muse/fleet evidence. The register alone is not sufficient evidence of present runtime state.

## 9. Record and tracking

This dated design is tracked by #12950, which is a rollup home rather than an implementation issue. #8986 remains the design-register hub. Child-specific work remains on the listed child issues and their descendants; no implementation is authorized by this family record.
