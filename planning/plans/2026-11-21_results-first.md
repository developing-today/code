# Results first: priorities and next work for `id` and the fleet

- Status: in-progress (written 2026-11-21 at Drew's request; he said to continue and to merge it the same day)
- Roadmap item: [ROADMAP.md, "Now, across the repo"](../../ROADMAP.md#now-across-the-repo) and the [`id` roadmap](../roadmaps/id.md)
- Design: none yet; the four designs this plan asks for are listed under [Designs to write](#designs-to-write)
- Issues: #557, #1067, #805, #1062, #796 (Phase 12 close-out); #9106, #9112 (substrate); #1233, #1187 (fleet); the step issues are filed when this plan is merged (step 0)

This is the document to come back to. It says what the priorities are, what is finished, what is being worked now, and what waits. The [Status board](#status-board) and [Progress](#progress) are updated as work lands; [How to update this document](#how-to-update-this-document) says when and by whom.

## Goal

In four weeks, `id` has a tagged release whose claims were each checked by a command, one end-to-end path through the new signed-record substrate runs between two nodes, and a change to `pkgs/id` is tested before it reaches main. The fleet spends more of its merges on `id` features and fewer on re-reading issues it cannot close.

## The short version

The fleet is not short of output. In the 30 days to 2026-11-21 it merged 4,532 commits to main. The problem is where the output goes and what stops:

- **27% of merged commits touch `id`.** 36% build the fleet's own tooling.
- **Small things close and large things stall.** 98% of `effort/xs` issues are closed; 48% of `effort/l` are, and 276 are open.
- **The queue has no way to build part of a feature.** Its packet kinds are triage, small fix, PR shepherd, CI investigation and second pass. A large `id` feature is handed out as triage, judged "not a small fix", noted, and handed out again.
- **Nothing tests an `id` change before it merges.** CI runs nightly on main. `id` accounts for 93 of the 160 main-broken issues.
- **108 open issues are parked on the owner.** He did not park them. Agents did, and then repeated each other.

So the priorities, in order:

1. **Test `id` before it merges** (fleet). Every other verdict reads this signal.
2. **Give the fleet a way to build a slice of a feature, and stop it re-triaging** (fleet).
3. **Finish Phase 12 and tag the first `id` release** (`id`). Fourteen of eighteen items are closed; four are open and one misses its target by a factor of 113.
4. **Run one end-to-end slice of the #9106 substrate** (`id`). A week of library code is merged and not yet reachable from the CLI.
5. **Stop parking work on the owner** (fleet). Agents decide, record the decision and continue.
6. **Everything else waits in Later with a named trigger.** Nothing is dropped.

Two `id` tracks, three fleet changes, one rule for decisions.

## Status board

### Done (checked against GitHub on 2026-11-21)

| What                                                | Evidence                                                                                                                                                                                                                                                                                              |
| --------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Path to Phase 12: all 14 ordered steps              | #1235 closed 2026-11-08                                                                                                                                                                                                                                                                               |
| Validation audits, Phases 0–11                      | #1075, #1076, #1077, #1078 closed 2026-10-27                                                                                                                                                                                                                                                          |
| Phase 11 foundations                                | Every F11 issue in the [`id` roadmap's tracker table](../roadmaps/id.md) is closed                                                                                                                                                                                                                    |
| Phase 12: 14 of 18 items                            | Closed: F12.1 #807, F12.2 #556, F12.3 #835, F12.4 #1061, F12.6 #1063, F12.7 #1064, F12.8 #1065, F12.9 #1066, F12.13 #806, F12.14 #1068, F12.15 #558, F12.16 #559, F12.17 #1069, F12.18 #560                                                                                                           |
| Phase 13: 5 of 8 items                              | Closed: F13.1 #783, F13.2 #784, F13.3 #785, F13.7 #789, F13.8 #790                                                                                                                                                                                                                                    |
| `pkgs/id` split into 14 crates                      | #3447 closed 2026-11-13; `pkgs/id/crates/` holds 14 members                                                                                                                                                                                                                                           |
| Phase 14 decided, first designs written             | Plan of record #6312; v1 designs closed for hosted worlds #6283, discovery #6287, ecosystem #6291, shells #6296, creator tooling #6308                                                                                                                                                                |
| #9106 summit design agreed                          | PR #9238 merged 2026-11-13; 384 statements mapped to 126 issue rows                                                                                                                                                                                                                                   |
| #9106 first library code                            | `id-core`, `id-sync` and `id-store` PRs merged 2026-11-16 to 2026-11-19 (for example #11939, #11947, #11988)                                                                                                                                                                                          |
| One fleet queue, unfiltered claims                  | PR #8438 merged 2026-11-11; FDR-0001 to FDR-0012 in `doc/decisions/`                                                                                                                                                                                                                                  |
| Self-mirroring of this repo's issues stopped        | #11748 merged 2026-11-18, after 299 copies were filed in two days                                                                                                                                                                                                                                     |
| Roadmap drift found in this review, fixed           | This plan's PR: `ROADMAP.md`, `planning/roadmaps/id.md`, `planning/plans/README.md`                                                                                                                                                                                                                   |
| Scoreboard tool, three skills                       | This plan's PR: `scripts/fleet/flow-report/`, `.agents/skills/next-work`, `feature-slice`, `decide-and-proceed`                                                                                                                                                                                       |
| Agent front door: `fleet agent`, `fleet shell`      | #12227: enrol any kind of agent; `fleet agent next` walks push, queue, shortlist, priority; `fleet shell` runs builds on the laptop or a codespace. [Design](../../doc/2026-11-21T11-30-00Z_design_fleet-agent-front-door/2026-11-21T11-30-00Z_design_fleet-agent-front-door.md)                      |
| Queue refuses waits on a person                     | #12227: `fleet queue park` and `block` exit 2 unless the wait is a hands item; 13 tests, 5 of them against the real binary                                                                                                                                                                            |
| Three more build surfaces in rotation               | #12227: `fleet-org`, `fleet-org-blacksmith` and `fleet-forks` dispatch as the user; proven by runs 44900088840, 44900255411 and 44900259404 on 2026-11-21                                                                                                                                             |
| GitHub agrees with this plan (step 0)               | Tracker #12238; ten issues filed (#12228 to #12237); `now` and `next` re-set to the order below; five duplicate door reports closed into #12228; #9112 closed with its decision                                                                                                                       |
| Dashboard separates hands items from agent parks    | #12227: the attention page and the digest count only hands items and review requests as needing the owner; agent parks are listed as fleet work                                                                                                                                                       |
| Rulebook carries the two rules agents kept breaking | #12265: root `AGENTS.md` says nothing waits on the owner, and that the worker who merges runs the issue's done check and closes it. Testing doctrine moved to `doc/agents/testing-doctrine.md`                                                                                                        |
| Merged-but-open issues swept (2026-11-22)           | 110 open issues carried `status:in-review` with every PR merged. 21 closed after their declared check passed on main (20 #9106 delivery rows and #10219); the label cleared with a next-action note on 98. 12 remain, each with an open PR                                                            |
| Main queue moving again (2026-11-22)                | Was queued 0, blocked 595. 147 tasks returned to the pool (17 whose blockers had closed, 130 parked on the owner). Two hours later: queued 124, claimed 10, blocked 457. Counts are `tasks` rows by status for `repo-health-code-pilot` in the queue store                                            |
| New fleet binary on the laptop                      | Built from `d26fa16` (#12227 and #12243): `fleet agent kinds` lists 16, a person-wait without a hands kind exits 2, `fleet shell --build` takes the build lock. Previous binary kept as `~/.local/bin/fleet.prev-2aee4cc`                                                                             |
| #9106 substrate is part of its crates               | #12268: 44 merged source files (19,477 lines) in `id-core`, `id-sync` and `id-store` were in no crate and could be called only from tests that include them by path. All declared; `check-rust-module-reach.sh` fails on any file nothing declares (44 on main before, 0 after; 817 files, 91 crates) |
| Walking skeleton, steps 1 and 2, in process         | #12268: `substrate_path` (a signed record read back from its `id://` address) and `substrate_sync_path` (written on node A, synced to node B, a conflicting write reported on both, closed by a record that follows both). 12 faults planted in the library are each caught                           |

"Closed" in this table means the issue is closed as completed. It does not mean the phase's exit test passes; see [Track A](#track-a-close-phase-12-and-tag-idv0120).

### Now

| Track or step                                                 | Exit test                                                                                                               | State on 2026-11-21                                                                                                                                                                              |
| ------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **F1** `id` gate before merge (#12229, #12230)                | `id` main-broken issues at 3 a week or fewer for two weeks                                                              | No gate. 43 `id` main-broken issues in the last 7 days. Design not written.                                                                                                                      |
| **F2** Feature slices in the queue; no repeat triage (#12233) | A large issue triaged once is never triaged again without a change; a `feature_slice` packet is claimed and lands a PR  | Large issues now reach agents as slice tasks through the shortlist and priority rungs of `fleet agent next`. The queue kind and the repeat-triage fix are not started.                           |
| **A** Close Phase 12, tag `id/v0.12.0` (#12231)               | All nine rows of the Phase 12 exit table recorded as met, each with a command and its output; release tagged            | 4 items open: #1062, #1067, #805, #557. No exit audit. No `id` release has been tagged.                                                                                                          |
| **B** One end-to-end slice of #9106 (#12232)                  | A two-node test: signed record written by `id://` address on A, synced to B, a conflicting write reported; CLI shows it | Modules are in their crates and the path runs between two in-process nodes (#12268). Left, in order: check the join at admission, a store on disk, the real transport, the CLI.                  |
| **F4** No work waits on the owner (#12234)                    | Open `user/escalated` issues that are not a hands item: 0                                                               | 87 open on 2026-11-22 (was 108). The rule is in the root rulebook, the queue guard is deployed, and the 130 queue tasks parked on the owner are back in the pool with the instruction to decide. |

### Next (starts when a Now track passes its exit test)

| Item                                                 | Why it is next                                                                        | Issues                                |
| ---------------------------------------------------- | ------------------------------------------------------------------------------------- | ------------------------------------- |
| Hosted worlds v1 and discovery                       | Phase 14's first implementation items; they need F12.10 and F12.12 to hold under load | #6276, #10695, #10706, #10708, #10709 |
| Agents-in-worlds spike                               | The differentiating spike named in the Phase 14 decision                              | #6300, #7913, #7914, #7916, #7917     |
| Phase 13 shells: tiers, kitty keyboard, native shell | Wait behind Phase 12 foundations per the `id` roadmap                                 | #786, #787, #788                      |
| Remaining #9106 rows, in the recorded sequence       | After the Track B slice proves the path                                               | #9106, #9112                          |

### Later (kept, with the trigger that starts each)

Nothing here is dropped. In-flight PRs in these lines are shepherded to merge or closed with a salvage note; no new slice starts until the trigger fires or the row is moved in this plan.

| Line                                      | Trigger                                                     | Entry issues                        |
| ----------------------------------------- | ----------------------------------------------------------- | ----------------------------------- |
| Currency, conversion, grants, TigerBeetle | `id/v0.12.0` tagged and Track B exit test passing           | #5522, #10070, #2723, #2803, #2292  |
| Jurisdiction-partitioned hosting          | Hosted worlds v1 running                                    | #3399 to #3404, #5523, #5528, #9935 |
| Boards, notes, knowledge layer, chat      | Track B exit test passing (they are apps on that substrate) | #5525, #7587, #5677, #5678, #5682   |
| Messaging and relay protocol v1           | Track B exit test passing                                   | #5534, #2036, #3796, #3797          |
| Serverless edition, store abstraction     | Hosted worlds v1 running                                    | #5524, #9080, #9082, #5681          |
| Importers (Notion, Joplin)                | Notes model implemented                                     | #6513, #8419, #7675                 |
| Spill v2                                  | F12.12 target met                                           | #7572, #7573, #7574                 |
| Android, ESP32, Bevy, native GUI and TUI  | Phase 13 native shell (#788) landed                         | #7938, #5783, #5781, #1071, #1072   |
| Phase 14 v2 to v4 outlines                | The matching v1 is implemented                              | The open outlines under #6312       |
| Crate splits outside `pkgs/id`            | A Now or Next item needs the split                          | #8977, #8954 to #8969               |
| New fleet and PM-stack features           | Names the scoreboard row it moves (rule F6)                 | #1184, #1340, #1352                 |

## Evidence

Every number below came from [`scripts/fleet/flow-report`](../../scripts/fleet/flow-report/README.md) run against `developing-today/code` at main `48c34e12` with issue data fetched 2026-11-21 08:00 UTC, unless the row names another source. Re-run it to check.

### Where merged work goes

4,532 first-parent commits on main, 2026-10-22 to 2026-11-21, by the area holding most of each commit's changed files:

| Area                                                                     | Commits | Share |
| ------------------------------------------------------------------------ | ------: | ----: |
| Fleet tooling (`pm-stack`, `pkgs/fleet`, agent config, bridges, proxies) |   1,643 | 36.3% |
| `id`                                                                     |   1,224 | 27.0% |
| `mcpx`                                                                   |     554 | 12.2% |
| Docs and planning                                                        |     414 |  9.1% |
| NixOS and infra                                                          |     260 |  5.7% |
| CI                                                                       |     212 |  4.7% |
| Other                                                                    |     143 |  3.2% |
| Atlas                                                                    |      82 |  1.8% |

Of the 1,244 commits that touch `pkgs/id` at all, 46.1% have a PR title starting `fix` and 14.9% `feat`. The fixes are small and mostly self-assigned: 79% of the 573 change one or two files, and 505 of the 639 `id` bug issues carry `source/creator`, the fleet's own bug-finding job. (These two counts are from the same inputs by hand; the tool does not print them yet.)

### What closes and what stalls

| Effort label | Issues | Open | Closed |
| ------------ | -----: | ---: | -----: |
| `effort/xs`  |    441 |    7 |  98.4% |
| `effort/s`   |  1,088 |   63 |  94.2% |
| `effort/m`   |    862 |  225 |  73.9% |
| `effort/l`   |    533 |  276 |  48.2% |

- 6,442 issues exist; 1,117 are open. 456 of the open ones (40.8%) are blocked, parked or waiting on a dependency.
- 255 open issues carry an `id` label; 91 of them (35.7%) are stalled.
- 240 open issues are `kind/design`. `doc/` holds 460 dated records from October and November; 289 are designs (`ls doc/` at `48c34e12`).
- 108 open issues carry `user/escalated`.
- 7.4% of all issues (475) carry `source/human`. 3,169 carry another `source/*` label, which means the fleet filed them.

### Motion

- 93,717 comments on 6,442 issues. 341 open issues have 20 or more.
- Every one of the 1,117 open issues was last updated on 2026-11-20 or 2026-11-21 (`updated_at` in the fetched data).
- #1062 (F12.5) was triaged by queue tasks 11885, 16920, 18051 and 19405. Each found it open, large and unimplemented, said so, and completed. No implementation PR exists.
- The fleet's own draft efficiency report (PR #11979, not merged) counts 5,079 completed queue tasks and 1,127 merged PRs over 5.85 days, and 4.81 billion metered tokens over 7 days.
- Seventeen times, four identical "Scheduled …" or "Daily …" issues were opened in the same hour (for example #12034 to #12037).

### Trust

- CI runs nightly on main. "No automated PR check is a merge requirement" ([`.github/ci-policy.md`](../../.github/ci-policy.md), rulings of 2026-11-13 and 2026-11-16, made when about 1,000 runs were queued).
- 46.6% of merged PRs merged within 10 minutes of being opened; the median is 20 minutes.
- 160 main-broken issues since 2026-10-29: `id` 93, `id-worlds` 1, `mcpx` 18, `nix` 8, other title shapes 40. In the 7 days to 2026-11-21, 43 were for `id`.
- `bash .github/scripts/check-suite-registry.sh` reports 36 test suites on main that no registry row covers. All 36 are in `pkgs/id` (18 in `id-core`, 13 in `id-sync`, 3 in `id-store`, one each in `id-import-notion` and `hosted-worker`). They arrived with the #9106 library code.
- No `id` release exists. The repository's releases are `fleet-static-*` builds, `mcpx/v0.1.0` (2026-10-30) and tags from 2023.

### Phase 12 against its own exit table

The exit table is in the [Phase 12 plan](../../.opencode/plans/phase-12-platform-flexibility.md). This is a first reading from issue state and the tree, not the audit step A1 asks for.

| Criterion                                                | Reading on 2026-11-21                                                                                                                        |
| -------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| 100 active worlds at 20 events a second, p99 under 50 ms | **Not met.** Measured 5,650.792 ms (agent note on #557, 2026-11-17).                                                                         |
| Typical redraw under 10% of a full frame                 | **Not met.** F12.11 #805 is open and waits on four protocol decisions.                                                                       |
| At least 3 guest languages passing the conformance suite | **Not met.** Roc and Rust guests are in `pkgs/id/examples/`. The third language is an open owner decision (agent note on #1065, 2026-11-09). |
| New capability with no edit to `world.rs`                | Claimed met: `provider_registry_exit_proof` test, PR #4965. Not re-run here.                                                                 |
| 1,000 idle worlds with bounded memory                    | Memory budget and eviction criteria are ticked on #557. Not re-run here.                                                                     |
| Empty directory to a joinable world in under 5 minutes   | Unmeasured. No scripted run of the documented steps was found.                                                                               |
| Upgrade keeps state and a failed upgrade rolls back      | Claimed met by #1066. Not re-run here.                                                                                                       |
| `world.toml` plus a module hash reproduces a world       | Claimed met by #807 and #556. Not re-run here.                                                                                               |
| No new reach except through a named, ceilinged provider  | F12.5 #1062 is open, so the new providers this row is about do not exist yet.                                                                |

## Why large `id` work stalls

Five causes, each visible in the repository.

1. **No packet says "build part of this."** `KINDS` in [`validate.rs`](../../pkgs/fleet/crates/fleet-queue/src/validate.rs) is `pr_shepherd`, `triage`, `small_fix`, `ci_investigation`, `second_pass`. On #557 a lane wrote that the packet "is typed small_fix, but this is a performance investigation at size L" and blocked it as a packet defect for the owner to re-scope.
2. **Coverage re-queues what triage cannot close.** The reconciliation in [`scripts/fleet/queue-reconcile`](../../scripts/fleet/queue-reconcile/README.md) holds that every open issue needs a live task and that "a done task is not coverage"; an uncovered issue gets a new triage packet. A large issue triaged to "done" is uncovered again. This is consistent with the four triage passes on #1062, though the reconciler's live mode on the laptop is not visible from the repository.
3. **Merges are unverified, so fixes chase merges.** With no check before merge, a broken `id` change is found by the nightly run, filed as main-broken, and fixed by another task. The 36 unregistered suites mean some new tests are not on record as running anywhere.
4. **Breadth outruns depth.** `id` has open work in at least twelve product lines at once, and Phase 14 alone has seven topics with four version outlines each. Each line has a design; few have a slice that runs.
5. **Agents park work on the owner.** #805 waited on four choices, #9112 on a confirmation requested on 2026-11-19, #3195 on "owner decides". Drew on 2026-11-21: these are "a wrong agents decisions getting regurgitated as my proclamations"; the problem is "agents not actually working on real issues". The choices were the agents' to make. [FDR-0013](../../doc/decisions/FDR-0013-no-owner-decision-gates.md) removes the gate.

Cause 4 is a choice, and it is Drew's: the directives of 2026-10-28, 2026-11-05 and 2026-11-12 each asked for breadth with nothing dropped. This plan keeps that. It orders the breadth and holds most of it behind a trigger.

## Scope

In: which `id` work is active and what finishes it; how the fleet picks, builds and verifies `id` work; how owner decisions are asked; the scoreboard.

Out, and where each is tracked:

- The content of the #9106 designs: the [summit package](../../doc/2026-11-13T03-40-00Z_design_id-atlas-summit/main.md).
- Model routing, pools and quota: #1340 and the routing policy.
- `mcpx`, NixOS, the basement cluster and Atlas as products: their [roadmaps](../roadmaps/).
- Queue bands and re-banding: the queue lane. This plan changes no band; step 0 asks for it.

## Steps

Each step is one PR unless it says otherwise.

### Step 0: ratify and wire

0. **Done 2026-11-21 (tracker #12238).** **Make GitHub and the queue agree with this plan.** File one epic with the steps below as children; set `now`, `next` and `priority/*` labels in the order of the [Status board](#status-board); remove `user/escalated` and `status/blocked` from the Now rows' issues with the decision recorded on each. Queue bands follow from the labels at the next reconcile. Done when: every Now row links an open issue that carries `now` and no owner-wait label.

### Fleet

1. **F1a. Register the 36 unregistered `id` suites.** One registry row per crate, each naming the workflow step that runs it; add the step where none does. Done when: `bash .github/scripts/check-suite-registry.sh` exits 0.
2. **F1b. Affected-crate selector.** A script that takes a diff and prints the `pkgs/id` workspace members to check and test, from the workspace dependency graph. Done when: a test feeds it a diff touching `id-core` and it names every dependent crate, and a diff touching only `id-render` names only `id-render` and its dependents.
3. **F1c. `id` gate design.** Write the design named under [Designs to write](#designs-to-write). Done when: the record is merged.
4. **F1d. Gate, first slice.** One landing lane takes ready `pkgs/id` PRs in batches, applies each batch to a candidate branch, runs `cargo check --workspace` and the affected crates' tests through `fleet-build-lock`, merges a green batch, and bisects a red one. Done when: ten consecutive batches land through it and the gate's log names the offender for a seeded bad PR.
5. **F2a. No repeated verdicts.** Extend `pm-stack/lane-guard/lane_guard.py note` so a triage verdict is skipped when the newest agent note already gives the same verdict and no commit, label, body or human comment has changed since, across task ids. Done when: a test with two tasks and one unchanged issue shows the second note skipped, and a third with a new commit shows it posted.
6. **F2b. Coverage accepts a verdict.** An open `effort/l` or `effort/m` issue with a recorded verdict holds a parked task with a resume condition (a slice landed, a dependency closed) and is not re-enqueued for triage. Needs a decision record, because it changes park and claim semantics. Done when: the reconciler's fixture test shows such an issue covered, and an issue with no verdict still uncovered.
7. **F2c. `feature_slice` packet kind.** Add the kind with required fields: parent issue, the criterion it moves, the done-check command, owned paths. Needs a decision record. Done when: `fleet queue enqueue` accepts a slice packet, rejects one with no done check, and a lane claims and completes one.
8. **F3a. One issue per scheduled run.** Find why a scheduled run opens four identical issues and make it open one. Done when: a day of scheduled runs shows one issue per run.
9. **F3b. Creator scope.** Until `id` main-broken is at 3 a week or fewer, creator-class runs file hardening bugs only in crates an active track touches. Done when: the creator's instructions carry the rule and a week of `source/creator` issues is inside it.
10. **F4. Clear the owner parks.** Work the 108 open `user/escalated` issues with [`decide-and-proceed`](../../.agents/skills/decide-and-proceed/SKILL.md): each is decided and returned to eligible, or re-blocked as a hands item with one concrete action. The queue refuses a person-wait block that is not a hands item. Done when: no open issue carries `user/escalated` without a hands action, and `fleet queue block --reason-class escalated-to-user` without `--hands` exits 2.
11. **F5. Weekly scoreboard.** Run the flow report each Monday and update the [Scoreboard](#scoreboard) by PR. If it becomes a scheduled job, register it in `doc/fleet-schedule-registry.md` in the same PR. Done when: four consecutive Mondays have a dated row.
12. **F6. Fleet-tooling budget.** A new fleet or PM-stack feature states in its issue which scoreboard row it moves and by how much; without that it goes to Later. Done when: the epic template asks for it and the fleet-tooling share is measured weekly.

### Track A: close Phase 12 and tag `id/v0.12.0`

13. **A1. Exit audit.** One dated record with a row per exit criterion: the command, its output, and met, not met or unmeasured. No fixes in this step. Done when: the record is merged and every "not met" or "unmeasured" row links an open issue.
14. **A2. F12.12 profile (#557).** Before any fix: where the 5.65 seconds go at 100 worlds and 20 events a second, as a committed profile and the command that produced it. Done when: the profile names the top three costs with their share of p99.
15. **A3. F12.12 fixes (#557).** One PR per cost from A2, each with a before and after number. Done when: the benchmark reports p99 under 50 ms, or the profile shows the target is wrong and the worker records a new one with that profile as its reason.
16. **A4. F12.10 fair scheduling (#1067).** The scheduler and per-account quotas, in slices; the `fuel_spin_isolation` workload already exists (PR #8322). Done when: a spinning world raises another world's p99 by no more than 10% in that benchmark.
17. **A5. Delta frames, phase 3 on (#805).** Both open design questions were decided on 2026-11-21 as the design record recommends (D9). Next is the negotiated MessagePack representation with its parity test, then compression where the compressed frame is smaller. Done when: a typical redraw is under 10% of the full frame and a patched client equals a full-frame client.
18. **A6. F12.5 providers (#1062), inward-facing first.** `kv`, then `blob`; `http.fetch` and `notify` after, each off by default with a refusal test. Done when: each provider has bound, refusal and replay tests, and an unlisted host is refused.
19. **A7. Third guest language: Zig.** A Zig toolchain is already in the tree for the Roc host. Done when: the Zig SDK passes the conformance suite and ships a playable example.
20. **A8. Verification debt (#796).** Each inventory item gets a proving test or its own issue. Done when: #796's checklist is complete.
21. **A9. Five-minute path.** A script that runs the documented steps from an empty directory to a world a second node joins, and prints the elapsed time. Done when: it runs in the nightly and reports under 5 minutes.
22. **A10. Tag `id/v0.12.0`.** Release notes list each exit criterion with its command and result from a re-run of A1. Done when: the tag exists and the A1 record shows nine rows met.

### Track B: one end-to-end slice of #9106

23. **B0. Sequence accepted.** The delivered 39-component sequence and its #9078 and #9091 dependencies stand as recorded. Done when: #9112 carries the decision record and no owner-wait label.
24. **B1. Walking-skeleton design.** Done in code rather than as a document (#12268): the path's modules are `id_core::{graph::record, signed_envelope, hash_profile, id_uri_parse, id_uri_format}` and `id_sync::federation::slices::{b32_ingest_gate, b07_conflict_report, b08_sync_scope, b09_anti_entropy}`, and the interface they share is decision D14. Every other row stays in its recorded slot.
25. **B2. The path, in slices.** Write a signed record at an `id://` address on node A; store it; sync it to node B over the existing transport; write a conflicting record on B; get a conflict report on both. Each slice lands with its test registered. Done when: one integration test does all five and fails if any module on the path is stubbed. State: write, sync, conflict and report run between two in-process nodes (#12268). Next slices, in order: (a) refuse a broken join at admission, not only on read; (b) keep versions and reports in a store on disk through `id_store::content_store`; (c) carry the exchange over the existing transport between two processes.
26. **B3. Reach it from the CLI.** The smallest command surface that drives B2 by hand. Done when: a README section shows the five steps as commands and the nightly runs them.

## Designs to write

Four, each needed by a Now step. No other design is asked for.

| Design                      | Decides                                                                                                                                                                | Needed by | Kind     |
| --------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------- | -------- |
| `id` gate before merge      | Batch size and cadence; which host builds; how a red batch is bisected and the offender returned; what happens when the gate is down; how it coexists with the nightly | F1c, F1d  | `design` |
| Feature slices in the queue | The `feature_slice` packet's fields; how coverage treats an issue with a verdict; which lanes claim slices; the decision records F2b and F2c need                      | F2b, F2c  | `design` |
| Phase 12 exit audit         | Not a design: the A1 record. Its format is one row per criterion with command, output and state                                                                        | A1, A10   | `audit`  |
| #9106 walking skeleton      | The one path through the 126 rows; the shared interfaces and their single owners; the integration test's oracle                                                        | B1, B2    | `design` |

## Decisions made

Made on 2026-11-21 by the session that wrote this plan. None waits on anyone. Each is revisable by a later PR with evidence; none needs a confirmation.

| #   | Decided                                                                                                                                                                                                                                                                   | Why                                                                                                                                                                                                                                           | Undo                                                                     |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------ |
| D1  | Tracks A and B are the active `id` lines; the rest wait in Later with the triggers above                                                                                                                                                                                  | 14 of 18 Phase 12 items are closed, and the #9106 library code has nothing reaching it                                                                                                                                                        | Move a row                                                               |
| D2  | A `pkgs/id` change is tested before it merges, through a batched gate on the fleet's build host                                                                                                                                                                           | `id` broke main 43 times in 7 days; batching answers the cost that made hosted PR checks unaffordable                                                                                                                                         | Turn the gate off; nightly CI is unchanged                               |
| D3  | The third guest language for F12.8 is Zig                                                                                                                                                                                                                                 | A Zig toolchain is already in the tree for the Roc host (`pkgs/id/examples/roc-world/host/build.zig`)                                                                                                                                         | Add another SDK                                                          |
| D4  | F12.12 keeps its p99 target of 50 ms until the A2 profile shows where the time goes                                                                                                                                                                                       | A target changed before the measurement is a guess                                                                                                                                                                                            | Record a new target with the profile                                     |
| D5  | #9112's delivered 39-component sequence and its dependencies stand as recorded                                                                                                                                                                                            | The sequence was delivered on 2026-11-19 and nothing in it was disputed                                                                                                                                                                       | Re-order rows on #9112                                                   |
| D6  | While `id` main-broken is above 3 a week, creator runs file hardening bugs only in active tracks' crates                                                                                                                                                                  | 505 of 639 `id` bug issues came from the creator; they compete with the tracks for the same lanes                                                                                                                                             | Drop the rule from the creator's instructions                            |
| D7  | An open large issue with a recorded verdict is covered by a parked task with a resume condition                                                                                                                                                                           | Re-triaging it hourly produced four verdicts and no code on #1062                                                                                                                                                                             | Revert the reconciler change                                             |
| D8  | The first release is `id/v0.12.0`                                                                                                                                                                                                                                         | Matches `mcpx/v0.1.0`                                                                                                                                                                                                                         | Tag again                                                                |
| D9  | F12.11: the F11.12 gate covers journaled frame kinds only, so binary screen frames are a negotiated v2 of the ephemeral `View`; the under-10% target is measured on the logical `View` payload                                                                            | Both are the recommendations in `doc/2026-11-01_design_id805.md` §7, unanswered since 2026-11-01                                                                                                                                              | Leave the binary path off; record another denominator with a measurement |
| D10 | Surfaces may dispatch with the fleet user's own token; `fleet-org`, `fleet-org-blacksmith` and `fleet-forks` are active                                                                                                                                                   | The user can already start those workflows; the App grant they waited on bought nothing in 17 days                                                                                                                                            | Remove `dispatch_as` and set the status back                             |
| D11 | The five dead-door reports are one defect, tracked in #12228; `_antigravity` leaves the rotation until its account answers                                                                                                                                                | A quota wall resets by itself; what the fleet controls is failover, which is code with a test                                                                                                                                                 | Reopen a report                                                          |
| D12 | On the dashboard and in the digest, only hands items and review requests count as needing the owner                                                                                                                                                                       | An agent's parked choice is fleet work under FDR-0013                                                                                                                                                                                         | Revert the census change                                                 |
| D13 | A slice declares its own module in its parent in the change that adds the file. This replaces "consumers do not edit roots" for #9106                                                                                                                                     | That rule left 44 merged files outside every crate: the root-owning slice closed before the leaves existed and the wiring slice (#10267) waits on six blockers. One added line per slice conflicts less than a week of unreachable code costs | Revert #12268; restore the rule in the module roots                      |
| D14 | A #9106 record joins the two halves this way: a version's body is the signed record's canonical bytes, its event id is the record's envelope digest (the digest in its `id://` address), its object is the record's logical graph, its parents are the records it follows | Parts A and B each have their own envelope and nothing connected them. This is the smallest join that lets an address name a version and makes two successors of one record a reported conflict                                               | Change `publish` in `substrate_sync_path.rs` and this row                |
| D15 | An issue whose PRs are all merged is closed by running its declared check, or loses `status:in-review` with a note saying what is left. No such issue waits for a reviewer                                                                                                | 110 open issues were in that state and each blocked the tasks that named it                                                                                                                                                                   | Reopen the issue                                                         |

## Scoreboard

Baseline from the flow report on 2026-11-21. Targets are this plan's proposal for 2026-12-19. A row is updated only from a fresh run.

| Measure                                        |     2026-11-21 | Target 2026-12-19 |
| ---------------------------------------------- | -------------: | ----------------: |
| `id` share of mainline commits (30 days)       |          27.0% |       45% or more |
| `feat` share of commits touching `pkgs/id`     |          14.9% |       30% or more |
| Fleet-tooling share of mainline commits        |          36.3% |       25% or less |
| `id` main-broken issues (previous 7 days)      |             43 |        3 or fewer |
| Unregistered test suites                       |             36 |                 0 |
| Open `id` issues blocked, parked or waiting    |             91 |       40 or fewer |
| Open `user/escalated` that are not hands items |            108 |                 0 |
| Open `effort/l` issues                         |            276 | falling each week |
| Phase 12 exit criteria recorded as met         | 0 of 9 audited |            9 of 9 |
| `id` releases tagged                           |              0 |                 1 |
| Track B integration test                       | does not exist |           passing |

## Tests and evidence

- The flow report has 19 contract tests with hand-counted fixtures (`scripts/fleet/flow-report/test_flow_report.py`); removing `--first-parent` from its git call makes the mainline test fail.
- Each step above names its own done check. Steps F1b, F2a, F2b and F2c start from a failing test.
- A1 and A10 are the evidence for Track A: the same record, run twice.
- The Track B integration test is its own evidence and must fail when any module on the path is stubbed.

## Rollout and rollback

Merging this file changes no queue state or schedule. Step 0 sets labels, and each later step is its own PR.

To undo: revert the plan's PR. The skills are additive and are loaded by description; reverting removes them. The `ROADMAP.md` and roadmap edits in the same PR restore with it. Re-banding done in step 0 is undone by the queue lane from the before-image it records.

## Risks

- **This becomes one more tracker.** The repository's own notes say registers and matrices consumed budget and fixed nothing. Limit: this file is updated on events only, its numbers come from a tool, and it replaces the stale "Now" lists instead of sitting beside them.
- **The gate becomes the bottleneck.** A slow or flaky gate would stall `id` the way the queued CI runs did. Limit: batches, one host with a warm cache, affected crates only, and a stated bypass in the F1c design for when the gate is down.
- **Later reads as cancelled.** It is not. Each row has a trigger; Drew can move any row with one line.
- **Targets are guesses.** The four-week numbers are proposals, set before F1 and F2 exist. Revise them in the first Monday update if the first week says they are wrong.
- **Closed is taken for verified.** A1 exists because Phases 9 and 10 were, in the roadmap's words, delivered faster than they were verified.

## Open questions this review did not answer

- Mainline commits fell from 242 to 317 a day on 2026-11-16 to 2026-11-18 to 33 and 47 on the next two days. The Turso migration and a queue-transport alert landed in that window, but the cause was not established from the repository.
- Token cost per merged `id` feature is unknown: the C5 report (#9573) says most consumption is not yet attributable to tasks.
- How many of the 108 owner parks are real hands items is not counted. F4 will show it; the expectation is few.

## How to update this document

- **Who:** the worker that lands a step, in that step's PR; the Monday scoreboard run; Drew at any time.
- **When:** a step's done check passes, a track's exit test changes state, an item moves between Now, Next and Later, or a decision is answered. Not for shift reports or triage verdicts.
- **What:** tick or move the row, then add one dated line to Progress with the PR and, for a number, the command that produced it.
- **How to pick work from it:** the [`next-work`](../../.agents/skills/next-work/SKILL.md) skill.

## Progress

- 2026-11-21: Plan written from a review of the roadmaps, plans, designs, 6,442 issues, 5,759 PRs and 30 days of mainline history at `48c34e12`. Same PR: flow report tool and tests; skills `next-work`, `feature-slice`, `decide-and-proceed`; drift fixed in `ROADMAP.md`, `planning/roadmaps/id.md` and `planning/plans/README.md`. Found on the way and fixed separately: `AGENTS.md` was 3 bytes over its rulebook budget (PR #12209).
- 2026-11-21: #12227 landed the agent front door (`fleet agent`, `fleet shell`), the queue guard for FDR-0013, user-token dispatch with three more build surfaces active, and an optional `op` secrets source. Checked by `cargo test --manifest-path pkgs/fleet/Cargo.toml --workspace --all-targets --locked` (942 passed) and three real dispatches. The forks copy of the fleet repo was synced to upstream the same day; the org copy still fails its sync on a merge conflict in `fleet-run.yml`.
- 2026-11-21: Step 0 done. Tracker #12238 and issues #12228 to #12237 filed; `now` holds the two `id` tracks, the gate and the fleet steps; `next` holds what follows. `fleet agent shortlist fill` against GitHub returned #4000, #12228, #7030, then the `now` set, in that order. #805 and #9112 decided; five door reports closed into #12228. One hands item exists: #12237.
- 2026-11-22: The fleet had stopped: main queue queued 0, blocked 595, about one merge an hour. Cause one: 110 issues open with all their PRs merged, blocking every task that named them. 21 closed on their own done checks, 98 relabelled (D15). Cause two: 130 tasks parked on the owner; returned to the pool. #12265 put both rules in the root rulebook and fixed the benchmark validator (#10219 closed). Queue two hours later: queued 124, claimed 10. New fleet binary deployed on the laptop.
- 2026-11-22: Track B. Found that the #9106 library was in no crate (44 files, 19,477 lines) and why (D13). #12268 declares the modules, adds the reach check, and runs the path between two in-process nodes with the join fixed (D14). Checked by `bash .github/scripts/check-rust-module-reach.sh`, `cargo test -p id-core --test substrate_path` (6 passed) and `cargo test -p id-sync --test substrate_sync_path` (4 passed).
- 2026-11-22: Not done, and why. 19 PRs opened by other agents are ready and unmerged, among them #12240 (suite registry), #12241 and #12246 (F12.11 binary frames); most carry unanswered review findings. The fleet's own merge sweep (`stale-pr-warden`) has filed no task since 2026-11-17. Lanes still claim from the queue only; `fleet agent next` is not wired into the claimant (#12236).
