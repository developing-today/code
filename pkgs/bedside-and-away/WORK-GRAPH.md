# Bedside & Away implementation work graph

Updated 2026-11-13. Parent: [#9359](https://github.com/developing-today/code/issues/9359). Design intake [#9361](https://github.com/developing-today/code/pull/9361) **merged** at `fd1d1e617c42c0eb98c3f324e9a30e39b50efad7`.

The owner authorized full implementation. This graph preserves all 102 source requirements; issue creation is not implementation completion. Component contracts and plans must be reviewed before their implementation. Requirements retain their original requested/optional/research classifications.

| Key    | Published work item                                                                                                         | BA scope                                       | Dependencies                                    | State                                 |
| ------ | --------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------- | ----------------------------------------------- | ------------------------------------- |
| BA-W01 | [#9365 identity, store membership and granular authorization](https://github.com/developing-today/code/issues/9365)         | BA-003, BA-043–046, BA-078–089                 | Foundation                                      | Open; design/build acceptance pending |
| BA-W02 | [#9366 canonical item graph, variants, ordering and annotations](https://github.com/developing-today/code/issues/9366)      | BA-014–042                                     | #9365, #9367                                    | Open; design/build acceptance pending |
| BA-W03 | [#9367 versioned commands, durable history and safe replay](https://github.com/developing-today/code/issues/9367)           | BA-017–018, BA-035–036, BA-047–051, BA-100     | Foundation                                      | Open; design/build acceptance pending |
| BA-W04 | [#9368 defaults, research jobs, evidence and scoped refresh](https://github.com/developing-today/code/issues/9368)          | BA-059–075                                     | #9365, #9366, #9367                             | Open; design/build acceptance pending |
| BA-W05 | [#9369 decision UX, inventory, receipt staging and reports](https://github.com/developing-today/code/issues/9369)           | BA-048, BA-052–058, BA-092                     | #9365, #9366, #9367, #9368                      | Open; design/build acceptance pending |
| BA-W06 | [#9370 general and contextual steering with verified orchestration](https://github.com/developing-today/code/issues/9370)   | BA-097–100                                     | #9365, #9366, #9367, #9368                      | Open; design/build acceptance pending |
| BA-W07 | [#9371 backend integration, private release and operational evidence](https://github.com/developing-today/code/issues/9371) | BA-076–077, BA-090–091, BA-093–096, BA-101–102 | #9365, #9366, #9367, #9368, #9369, #9370, #9372 | Open; design/build acceptance pending |
| BA-W08 | [#9372 source-list recovery and product-research acceptance gates](https://github.com/developing-today/code/issues/9372)    | BA-001–013                                     | #9366, #9368                                    | Open; design/build acceptance pending |

## Execution order and gates

1. W01/W03 define identity, policy and command contracts together. W07 may inventory infrastructure in parallel; its final acceptance depends on all workstreams.
2. W02 implements the canonical graph against those contracts.
3. W04 scoped research/defaults and W05 decision/reporting follow the graph and authority model; prototype UI modules may be reviewed in parallel without declaring integration complete.
4. W06 steering uses the same commands and visibility checks. Manual mode remains explicitly manual until delivery/callback receipts prove connected mode.
5. W08 private evidence/research progresses independently where access permits. Missing source facts block only their research acceptance, not the generic application.
6. W07 reconciles every requirement with tests, PRs, merge and deployment evidence. Optional/deferred/externally blocked rows remain explicit and cannot count as fulfilled.

## Actual versus pending

**Actual:** intake PR merged; duplicate search found no existing Bedside child work items; eight linked work items published and parent updated; all BA-001–102 mapped with cross-cutting overlap.

**Pending:** reviewed component designs/plans, implementation evidence, integrated acceptance, application PR/main merge, rollout and restore receipts. Local prototype or draft modules do not establish those states. Check live issue/PR status rather than treating this snapshot as a perpetual completion record.

## External gates retained

- Private source-list access and item-specific facts are not public issue payloads and cannot be fabricated.
- Provider authentication and secret lifecycle; proof of both accounts for linking; new audience/security grants.
- Verified Archer endpoint, scoped outbound payload and signed callback/receipt contract before connected steering.
- Optional ChatGPT surface through supported authentication, never cookie/session passthrough.
- Atlas/Snowflake is explicitly deferred; D1/Turso placement is not a dual-write or synchronization promise.

## Queue ownership

Deliberately not externally enqueued while Aster's internal implementation lanes coordinate this request. Every issue contains that disposition to prevent duplicate writers. If work is handed off, explicitly transfer queue ownership; creation alone does not dispatch Archer/Muse.
