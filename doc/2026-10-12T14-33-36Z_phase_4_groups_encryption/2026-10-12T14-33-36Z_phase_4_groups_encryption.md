# Phase 4: groups, encryption, and history

- Plan: [phased plan](../../2026-10-11T11-08-42Z_design_remaining_work/plan-phases.md), section 9.
- Start commit: `c12448c9` (origin/main after Phase 3).
- Base branch `phase-4`. Tracks: `p4-adapter` (P4.1, P4.2, P4.3; first, it unblocks the rest), then `p4-scheme` (P4.4), `p4-history` (P4.5, P4.6, P4.12), `p4-heads` (P4.7), `p4-warn` (P4.8 to P4.11).
- Decisions in force: full history by default; recent:N per world (32, 128 presets); scheme v1 (serde_bytes) as the first released scheme; from_join fork kept in the fork; no periodic rotation; PCS recovery by remove and re-add; warnings at 96, 128, 1000; BeeKEM not adopted.

## Steps

| Step                                  | Track   | Status | Commits            | Tests                                 | Notes                                              |
| ------------------------------------- | ------- | ------ | ------------------ | ------------------------------------- | -------------------------------------------------- |
| P4.1 DataScheme adapter               | adapter | done   | a630a440           |                                       | blocks the rest                                    |
| P4.2 Our DGM and the from_join fork   | adapter | done   | e57c1223           |                                       |                                                    |
| P4.3 from_join withholding rule       | adapter | done   | daa4f200           |                                       |                                                    |
| P4.4 Scheme v1                        | scheme  | done   | d22f6987, 3b671bb4 |                                       |                                                    |
| P4.5 recent:N and full                | history | done   | 6b2a9005, 67207add |                                       |                                                    |
| P4.6 Lazy full history                | history | done   | 25d7d814           |                                       |                                                    |
| P4.7 Heads rule                       | heads   | done   | 6001cfdd, e1fee2ce | 8 new (4 heads, 4 concurrency)        | cap 32, merge above the cap (2979); open epoch gap |
| P4.8 Rotation policy and PCS recovery | warn    | done   | 205a5483           | 4 rotation and recovery tests         | `Session::recover`; no CLI command yet             |
| P4.9 Warnings                         | warn    | done   | 4b82eff2           | 11 warning tests (9 unit, 2 property) | pure evaluator; not yet wired into serve           |
| P4.10 scheme none notice              | warn    | done   | a95c52ae           | 4 notice tests                        | function only; no UI yet                           |
| P4.11 DataScheme cost measurement     | warn    | done   | 2a3de9a9           | 1 smoke test; benchmark ignored       | numbers below                                      |
| P4.12 Welcome chunking                | history | done   | c38e6a39           |                                       |                                                    |
| P4.13 Roster follows the group fold   | scheme  | done   | a39cc834           | 11 scheme tests (incl. fold property) | mismatch poisons the session                       |

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

## P4.7: heads and concurrency

Commits on `p4-heads` (not pushed):

- `6001cfdd` test(rust): concurrent membership changes converge [P4.7]. The concurrency tests and the welcome-first fix.
- `e1fee2ce` feat(rust): heads rule with a cap of 32 parents [P4.7]. The heads rule, the cap, and the head tests.

### The rule as built

- A control message names the heads: the delivered or published control messages that no delivered or published control message names. It also names its author's own latest control message, even when a later message covers it. Application messages name the same list.
- At most 32 parents, own latest included. A publish that would name more is refused with `SchemeError::TooManyParents { count, cap }`. The check runs before the crate call, for update, send, add, remove and recovery. A refusal leaves the epoch and the roster as they were, and nothing is dropped from the parents.
- The merge in the plan (P4.7: merge when the heads would exceed the cap) is built (queue task 2979). While a change's parents would exceed 32, the session first publishes update merges: each is an update naming up to 32 heads, own latest first, and the heads it leaves stay heads. Each merge turns 32 parents into one, so the count falls by at least 31 and the loop ends. The change that follows names at most the cap, including the last merge and the heads the merge left. Every `Session` change that can merge returns all the messages to broadcast, merges first: a member without the merge cannot deliver the change that names it. `TooManyParents` remains as the orderer's guard for a publish that skipped the merge; it is not reachable through the `Session` API.
- The rule it replaces named the latest control message of each author. That list grew with the number of authors. At 33 authors it went past 32 with no check, and it named messages that a later head already covered.

### Findings

- The red test (`test_two_concurrent_adds_converge_for_a_third_member`) had two admins add members 4 and 5 from the same view, and member 1 ordered the two adds in either order. It failed before the fix. A joiner that received the other admin's add before its own welcome applied that add to an empty roster, and its roster lacked a member. The orderer now delivers the welcome after its own past and before any other pending message. A welcome that is not yet ready holds delivery.
- Resolved: an update made without knowing a concurrent add never reaches the joiner by delivery. The joiner then holds one epoch fewer than the members, and messages under that epoch are withheld from it. `test_open_a_joiner_misses_an_update_made_without_knowing_it` pins this. The heads rule does not fix it, since the update names nothing about the add. The decision (issue #189) is lazy history (P4.6): the joiner fetches the missing epoch through `Session::fetch_history`. That fetch runs only when a caller supplies a transport -- inside `pkgs/id` the only call sites are the history tests -- so a caller that receives `Received::Withheld` must drive it; nothing in this crate fetches on its own.
- In the two-add case, every member agrees on the latest epoch and on the epoch set.

### Properties and tests

- `test_concurrent_updates_and_sends_converge` and `test_a_concurrent_add_and_sends_converge`, 16 cases each. Two or three admins each make one change from the group as it was: an update, a send, or the add of member 4. Each member then hears the changes it did not make, one at a time, in its own shuffled order, with the members taking turns. At the end every member has the same roster and the same epoch set, and reads every send by another member. A local run at 128 cases passed.
- A property that mixed updates, sends and an add found the open gap above. The add and the updates are therefore in separate properties.
- Heads: `test_a_message_names_the_heads_and_not_every_authors_latest` (red before the change), `test_an_author_names_its_own_latest_even_when_a_later_message_covers_it`, `test_a_message_may_name_32_parents`, and `test_a_group_above_32_heads_merges_and_publishes_again`. The last one was the refusal test at 33 parents; it now checks the merge: a publish above the cap returns a merge naming 32 of the 33 parents and the change that follows, the left head and the merge are both named by that change, and every member of the 33 ends on the same roster and epoch set and reads a message sent after the merge.
- A change refused above the cap lost its merges (found in review of this change): `add_with_history`, `add_from_join` and `remove` merged the heads before the change step checked the view policy, so a policy refusal dropped merge envelopes the session had already absorbed, and a later publish named a merge nobody received. The policy is now checked before the merge in all three. `test_a_refused_change_above_the_cap_loses_no_merge` pins it: red without the reorder, green with it.
- Results after the merge (queue task 2979): the encryption module has 72 passed and 1 ignored (the P4.11 benchmark). The count fell by one because the 33-parent refusal test became the merge test, then rose by one for the refusal-above-the-cap test below. The full `cargo test --lib` has 1172 passed, 0 failed and 4 ignored. Clippy reports nothing in `src/encryption`.

### Open

- The open epoch gap for a concurrent update and add (above).
- The S10 scenarios at 40 members are not reproduced. Concurrency is tested with 2 or 3 admins, and 33 members only for the cap. Storms and partitions at scale are open.
- Concurrent removals, and a removal concurrent with an update, are not in the properties.
- Message loss and long delays are not tested. Only reorderings are.

## End

- P4.8 to P4.11 done on `p4-warn`. Commits: 205a5483, 4b82eff2, a95c52ae, 2a3de9a9. Not pushed.
- P4.7 done on `p4-heads`. Commits: 6001cfdd, e1fee2ce. Not pushed.

## 2026-10-12 Suite: phase 4 full run

- Merged tracks: `p4-scheme` (6b9235ab), `p4-history` (8dc0ca61), `p4-warn` (b05d0d4f), `p4-heads` (cd7b8636).
- `cargo test --all-targets --no-fail-fast` on the merged tree: 1418 passed, 9 ignored, 1 failed.
- The failure is `serve_tests::test_world_module_shared_over_iroh_and_web_presentations`: the server stalls after `world: records document ready` and never prints `node:`. It fails the same way on the phase-3 tip (3a51589d), so phase 4 did not cause it. Issue #190.
- One earlier run failed `test_an_interrupted_session_restarts_without_duplicates` under load and passed alone three times. Issue #188.
- Not run in this pass: `just crash-gate` and the nix checks.
- Open from the tracks: the joiner that misses an update made without knowing a concurrent add (issue #189, resolved by lazy history in P4.6), the held-message cap (issue #186), and stalled delivery on missing pre-join controls (issue #187).
