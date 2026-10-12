# Phase 4: groups, encryption, and history

- Plan: [phased plan](../../2026-10-11T11-08-42Z_design_remaining_work/plan-phases.md), section 9.
- Start commit: `c12448c9` (origin/main after Phase 3).
- Base branch `phase-4`. Tracks: `p4-adapter` (P4.1, P4.2, P4.3; first, it unblocks the rest), then `p4-scheme` (P4.4), `p4-history` (P4.5, P4.6, P4.12), `p4-heads` (P4.7), `p4-warn` (P4.8 to P4.11).
- Decisions in force: full history by default; recent:N per world (32, 128 presets); scheme v1 (serde_bytes) as the first released scheme; from_join fork kept in the fork; no periodic rotation; PCS recovery by remove and re-add; warnings at 96, 128, 1000; BeeKEM not adopted.

## Steps

| Step                                  | Track   | Status | Commits  | Tests                                 | Notes                                    |
| ------------------------------------- | ------- | ------ | -------- | ------------------------------------- | ---------------------------------------- |
| P4.1 DataScheme adapter               | adapter | todo   |          |                                       | blocks the rest                          |
| P4.2 Our DGM and the from_join fork   | adapter | todo   |          |                                       |                                          |
| P4.3 from_join withholding rule       | adapter | todo   |          |                                       |                                          |
| P4.4 Scheme v1                        | scheme  | todo   |          |                                       |                                          |
| P4.5 recent:N and full                | history | todo   |          |                                       |                                          |
| P4.6 Lazy full history                | history | todo   |          |                                       |                                          |
| P4.7 Heads rule                       | heads   | todo   |          |                                       |                                          |
| P4.8 Rotation policy and PCS recovery | warn    | done   | 205a5483 | 4 rotation and recovery tests         | `Session::recover`; no CLI command yet   |
| P4.9 Warnings                         | warn    | done   | 4b82eff2 | 11 warning tests (9 unit, 2 property) | pure evaluator; not yet wired into serve |
| P4.10 scheme none notice              | warn    | done   | a95c52ae | 4 notice tests                        | function only; no UI yet                 |
| P4.11 DataScheme cost measurement     | warn    | done   | 2a3de9a9 | 1 smoke test; benchmark ignored       | numbers below                            |
| P4.12 Welcome chunking                | history | todo   |          |                                       |                                          |

## Decisions

- See the decisions above and the index.
- Rotation (P4.8): the epoch rotates on create, update, removal, and the update that follows a join. Nothing else moves it, and the module takes no clock. An idle group keeps its epoch over a simulated year (`test_an_idle_group_keeps_its_epoch_over_a_simulated_year`).
- PCS recovery (P4.8, N8): `Session::recover` removes the device, then re-adds it by `add_from_join`, so the device joins the new epoch with no earlier secret. It returns the envelopes and the epoch before and after. `RECOVERY_STEPS` holds the two steps the CLI prints. The re-added device keeps its long-term identity and its registered bundle; rotating its pre-key is not part of the procedure yet.
- Warning thresholds (P4.9, N11): 96, 128 and 1,000 members. A group that jumps past several warns for each, in order.
- Churn (P4.9, N34): a warning when a client-made group has 4 device re-adds in the last 24 hours. Churn re-arms only when the window falls to 2, so a count that flaps around 4 warns once. Size warnings keep a high-water mark, so a group that shrinks and regrows does not warn again.
- Warnings never block an operation, and they apply only to client-made groups. Provisioned groups are not warned about.
- The "scheme: none" notice (P4.10, decision 8): the metadata field `scheme` is `none` for a group with no scheme, and the status string is `scheme: none`. A group with a scheme shows its name and version.

## Open questions

- The warnings are not yet wired into `id serve` or the CLI status line. The evaluator is pure and takes `now` and the member count from the caller. The serve path does not yet have a client-made group event feed to evaluate.
- The notice is not yet shown in the web UI. The status string is the source for it.
- The recovery procedure is a function. The CLI command that runs it (P6.7) is not written.
- The churn window and count (24 hours, 4) and the size thresholds are first choices. The P4.11 numbers are the input for revisiting them.
- The re-added device's pre-key rotation (see Decisions) needs a decision before the CLI runs the procedure.

## Changes to the plan

- None to the plan itself. The P4.8 test ("a compromised member is replaced and cannot read new epochs") runs over three devices in the adapter's test cast, not over the CLI, since the CLI command is not written yet.

## P4.11: DataScheme update cost (measured)

Run: `cargo test --release --lib encryption::tests::bench_update_cost -- --ignored --nocapture`, on an AMD Ryzen 7 7840U (16 threads), Rust 1.99.0, release profile, one thread. The benchmark builds one committer's device for a group of N members, creates the group, runs one warm-up update, then times 5 updates and reports the mean. Bytes are the postcard size of the envelope. The benchmark measures the committer only: receivers, removals, and the transport framing are not in the numbers.

| Members | Create bytes | Create ms | Update bytes | Update ms (mean of 5) | Update bytes per member | Update CPU per member |
| ------: | -----------: | --------: | -----------: | --------------------: | ----------------------: | --------------------: |
|     100 |       40,496 |      26.9 |       30,654 |                  9.71 |                     307 |                 97 µs |
|   1,000 |      408,697 |     267.0 |      309,993 |                 101.2 |                     310 |                101 µs |
|   5,000 |    2,049,696 |     1,333 |    1,543,795 |                 596.5 |                     309 |                119 µs |

Against the BeeKEM spike (the table in the BeeKEM assessment, section 2):

| Members |    BeeKEM update (one copy) | DataScheme update | Ratio (bytes) |    BeeKEM CPU | DataScheme CPU |    Ratio (CPU) |
| ------: | --------------------------: | ----------------: | ------------: | ------------: | -------------: | -------------: |
|     100 |                      2.8 KB |           30.7 KB |           11x | 0.6 to 0.8 ms |        9.71 ms |     12x to 16x |
|   1,000 |                      3.9 KB |            310 KB |           80x | 0.6 to 0.8 ms |       101.2 ms |   126x to 169x |
|   5,000 | about 5.0 KB (extrapolated) |           1.54 MB |    about 309x |     not given |       596.5 ms | not computable |

The BeeKEM CPU in the spike is given without a size, so the CPU ratios assume it is flat. The first commit after a bulk create is 408.7 KB at 1,000 members for DataScheme, against 122 KB for BeeKEM (the BeeKEM table).

Fan-out break-even: a BeeKEM operation goes out once, and DataScheme sends about 309 bytes to each member. So BeeKEM wins on bytes from about 9 members (at 2.8 KB) to about 16 members (at 5.0 KB). The BeeKEM assessment states 11 to 17 from a per-message estimate, which it marks as not measured. The measured figure puts the lower end at 9. The assessment is not edited here; the owner should decide whether to update it.

Reading: DataScheme's update cost is linear in the members, at about 309 bytes and 100 µs of committer CPU per member, so it grows as the table predicts. At 1,000 members a single update is 310 KB, which is the "update exceeds an agreed budget" trigger in BeeKEM's conditions (section 4, item 4). No budget has been agreed, so this is a number, not a finding that a condition holds.

## Review notes and regression tests

- The rotation tests are `test_the_epoch_rotates_on_create_update_and_remove_and_not_on_send` and `test_an_idle_group_keeps_its_epoch_over_a_simulated_year`.
- The recovery tests are `test_recovery_puts_the_device_back_and_its_compromised_copy_cannot_read` and `test_recovery_rejects_a_removal_that_leaves_the_device_in_the_model`. The compromised copy is the removed device's session, which holds the state from before the removal. It is withheld from every message under the new epoch.
- The warning tests cover each threshold (95, 96, 127, 128, 999, 1000), flapping on size and churn, the non-client-made case, and two properties: the warning count follows the largest size, and it is monotone in size.

## End

- P4.8 to P4.11 done on `p4-warn`. Commits: 205a5483, 4b82eff2, a95c52ae, 2a3de9a9. Not pushed.
