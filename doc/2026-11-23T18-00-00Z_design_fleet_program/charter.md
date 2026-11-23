# Fleet charter and delivery contract

Fleet gives each enrolled seat a common way to find work, claim it, run it on an eligible machine, exchange messages and leave a result another seat can verify. The multicall binary composes clients. Brokers own the shared facts that cannot be decided on one seat. An issue, a queue task, a lease, a workflow run and a merged change each have their own identity.

## Governing rules

1. Any agent may present itself for work. A lane name auto-registers on first claim. Enforce task requirements and active ownership in the broker. A display name, a GitHub assignee or a cached shortlist entry cannot grant a lease.
2. Use included plan capacity only. Decide affordability at the billing owner and binding window. A failed quota read leaves a previously verified plan-only route eligible. A provider's confirmed plan-limit response has a different evidentiary basis. Keep OpenAI as the final reserve.
3. Software work is complete when its verified change is merged. When acceptance requires production behavior, record the deployed generation, the consuming seat and the observed result separately.
4. A seat's kind selects capabilities and transport. The protocol remains shared. Enforce grants at the boundary that serves the data or accepts the mutation.
5. Preserve held work, explicit operator decisions and existing claims. A successful repair makes a packet valid. Dependency satisfaction and permission to claim it require their own evidence.

## Authority and evidence

| Question | Authority | Evidence to retain |
|---|---|---|
| Who can mutate this task? | Queue writer and lease record | Task, holder, lease deadline, disposition and timeline. The proposed fencing epoch requires compatibility work. |
| Where did the command actually run? | Placement record and provider job | Decision, target repo and SHA, runner repo, run, attempt, job and terminal receipt |
| Which seat received the handoff? | Authenticated enrolment, bridge receipt and queue transfer | Seat, installation, scope, checkpoint digest, redemption and transfer time |
| Which service is serving? | Deployment registry and authenticated runtime probe | Source and executable digest, generation, activation, observation time and recovery target |
| Which credentials can this seat use? | Scoped service and provider authorization | Principal class, audience, expiry and successful scoped probe. Publish metadata only. |
| Which pool can accept work? | pm-stack admission projection and plan policy | Billing owner, binding window, genuine measurement time, read quality and independent wall evidence |
| Which data did search return? | Atlas index and the consumer's authorized scope | Index version, freshness, query contract and scope |
| What shipped to readers? | Release, bootstrap, Pages deployment and delivered artifact | Immutable identities for each stage and an observation of the rendered chapter |

## Decision boundaries

The [founding multicall design](https://github.com/developing-today/code/tree/main/doc/2026-11-04T11-45-48Z_design_fleet_multicall_binary) owns binary composition. The [Fleet and PM orchestration charter](https://github.com/developing-today/code/blob/main/doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/sources/02_charter_v1_1.md) owns the shared work-item and outbox design. The [quota admission design](https://github.com/developing-today/code/tree/main/doc/2026-11-14T18-05-00Z_design_fleet_quota_admission) and [Stage 3 program](https://github.com/developing-today/code/issues/12694) supply the quota contract. The upstream [member roster](https://github.com/dezren39/fleet/blob/main/FLEET.md) records management tiers. The book distinguishes a proposal, a source implementation, a provider receipt and a deployed observation.

## Living record

Update the matching book chapter with a change to a command, flag, default, capability, surface or procedure. Record semantic decisions in the repository's decision archive. Cite a source revision for code, a provider attempt for execution and a timestamp for a live observation. Review the issue map against native blockers, written prerequisites, individual holds and the newest ownership evidence before assigning work. [Program #13272](https://github.com/developing-today/code/issues/13272)
