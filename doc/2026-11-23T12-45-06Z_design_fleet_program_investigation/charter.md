# Fleet charter

Fleet gives an enrolled seat one way to discover work, obtain a lease, run a command on a compatible surface, exchange a checkpoint and demonstrate what finished. The multicall binary composes the clients. Brokers and providers hold the records that other seats must be able to verify.

## Governing rules

- Any agent may seek any job. A lane name auto-registers on first claim. Explicit task requirements, active leases and granted capabilities govern each claim. Seat kinds describe different environments running the same protocol.
- Only included plan capacity may be consumed under this program. A stale meter cannot bench an eligible provider. A decision with potential monetary exposure requires a known plan-only route.
- Software work is finished when verified and merged. A deployment additionally needs its own generation, executable identity and consumer result. Queue completion must preserve the difference.
- Keep one authority for each fact. The provider owns the workflow run. The broker owns the lease. The seat owns its placement decision. The local profile records what the installation observed. Every cross-system projection carries its source and age.
- The existing book documents shipped behavior. The R&D chapters label proposals and their acceptance oracles. DOC-01 couples changed behavior with its operator documentation.

## Boundaries and ownership

| Component                       | Responsibility                                                                      | Evidence                                                                                  |
| ------------------------------- | ----------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------- |
| fleet-queue and its broker      | Claim, renew, park, resume, deduplicate, repair and record transitions              | Task ID, holder, lease, event timeline, source revision                                   |
| fleet-agent and bridge          | Enrolment, capability probes, intake, delivery, acknowledgements                    | Authenticated sender, installation observation, delivery receipt                          |
| Placement and upstream executor | Choose the eligible surface, submit, correlate and retrieve                         | Decision record, immutable checkout, provider run and attempt, digest and terminal result |
| Secrets broker and laptop agent | Scoped credential resolution, authenticated transport, per-service activation       | Class, audience, age, service generation and redacted probe                               |
| pm-stack and Atlas              | Measure quotas and publish decisions; index bounded, authorized operational records | Meter timestamp, policy revision, index watermark and ACL scope                           |
| Maintainers                     | Review, merge, roll out, and document discrepancies                                 | Linked issue, reviewed PR, verification route and deployment readback                     |

The queue fronts Fleet, ID and other program lanes. Atlas owns ingestion freshness. Fleet owns the search client's request/response behavior. The PM orchestration program owns its design DAG and remains an independent program. Changes affecting shared contracts require the corresponding consumer oracle.

## Program design record

The dated [commission](2026-11-23T12-45-06Z_design_fleet_program_investigation.md) and [tracking issue](https://github.com/developing-today/code/issues/13272) govern this series. The [roadmap](../../pkgs/fleet/book/development/program-roadmap.md) links the design components and acceptance order. The [open issue map](../../pkgs/fleet/book/operations/program-issue-map.md) records the census, prerequisite graph and published limits of that evidence. Update those records when evidence changes. Item-specific holds, installation decisions and retention decisions remain with Drew.
