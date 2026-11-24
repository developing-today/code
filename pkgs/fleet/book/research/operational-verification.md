# Operational verification

## November 23, 2026 readback

An AMD operator can reconcile three separate facts: the executable accepting requests, the deploying registry's record and the state returned by a normal client. During this readback the local Fleet CLI reported `22ec90b9c29d`. The queue service still served the earlier release. This was a read-only observation.

| Service | Active profile generation | Registry activation (UTC) | Running evidence |
|---|---:|---|---|
| Queue broker | 6 | 14:05:46 | The systemd user service was active. Its process executable matched the selected profile closure. Invoking that executable's version command yielded `bcc8043b59ab97c20b639db473f2055c9961d43e`. The deployment journal's source field was `working-tree`. |
| Laptop agent | 3 | 09:11:22 | The service was active. Its journal recorded `b8bbcc0c49564ab31db8d58b026b67532db9491e`. The process executable was a Python interpreter, so its version alone cannot identify the application source. |
| Secrets broker | 2 | 00:35:37 | The service was active. Its process executable matched the selected profile closure and reported source `dirty`. The journal recorded `working-tree`. The exact Git source remains unknown. |

Sources were the AMD user service manager's `MainPID`, the process executable link, the selected deploy profiles, the last activation record in each service's generation journal and the CLI version output. These readings were taken around 18:00 UTC. The release receipt remains an independent record. [Rollout receipt](https://github.com/developing-today/code/issues/13146#issuecomment-7103726386)

## Reconciliation of the 37 repairs

The 15:25 UTC operator receipt names 37 distinct tasks. Between 18:05:13 and 18:05:26 UTC, read-only requests to the active queue broker recovered **exactly one persisted `repaired` event for each task**, with 37 distinct event IDs. Every task's `payload.dedupe.checked_at` matched the timestamp in the published ledger. For 29 tasks the event had the same second as the payload stamp. For eight the event had the following second. This verifies persistence for this set. Proving atomicity under a failed write requires a fault-injection fixture. [Operator ledger](https://github.com/developing-today/code/issues/13191#issuecomment-7105556112) · [Repair writer](https://github.com/developing-today/code/blob/bcc8043b59ab97c20b639db473f2055c9961d43e/pkgs/fleet/crates/fleet-queue/src/turso_queue/repair.rs)

The observed queue states were **18 done, 15 blocked, three claimed and one parked**. Fifteen records carried nonempty stored dependency data. Each read observed a single task and may race with subsequent transitions. The event and task detail are separate reads. The public operator ledger records the immediate `queued` state after repair.

The broker's authenticated local read route accepts a per-task `/events-list` request filtered by `kind=repaired`, followed by a rich `/list` read restricted to the same task ID. Use the configured client credential without printing it. The CLI's `show` and `timeline` implementations at the inspected source enumerate particular event kinds and omit `repaired`. A missing line in either display is therefore insufficient evidence of a missing stored event. Compare release capabilities and error responses before extrapolating to other deployments. [Query implementation](https://github.com/developing-today/code/blob/1f9bc15391ca1c2efa639463a6369392456c27e5/pkgs/fleet/crates/fleet-queue/src/client/commands_query.rs) · [Broker event query](https://github.com/developing-today/code/blob/1f9bc15391ca1c2efa639463a6369392456c27e5/pkgs/fleet/crates/fleet-queue/src/turso_queue/core.rs)

| Task | Linked issue | Repair event | Task state | Stored dependencies present | GitHub issue state | Open native prerequisites |
|---:|---|---:|---|---|---|---:|
| 18141 | [#1826](https://github.com/developing-today/code/issues/1826) | 369716 | done | no | open | 0 |
| 18211 | [#2232](https://github.com/developing-today/code/issues/2232) | 369734 | claimed | no | open | 0 |
| 18212 | [#2249](https://github.com/developing-today/code/issues/2249) | 369739 | done | no | closed | 0 |
| 18228 | [#2381](https://github.com/developing-today/code/issues/2381) | 369744 | done | no | closed | 0 |
| 18257 | [#3399](https://github.com/developing-today/code/issues/3399) | 369753 | done | no | open | 0 |
| 18258 | [#3400](https://github.com/developing-today/code/issues/3400) | 369763 | blocked | yes | open | 0 |
| 18259 | [#3401](https://github.com/developing-today/code/issues/3401) | 369769 | done | no | open | 0 |
| 18275 | [#3898](https://github.com/developing-today/code/issues/3898) | 369774 | blocked | yes | open | 0 |
| 18276 | [#3899](https://github.com/developing-today/code/issues/3899) | 369778 | blocked | yes | open | 0 |
| 18296 | [#5007](https://github.com/developing-today/code/issues/5007) | 369780 | blocked | yes | open | 0 |
| 18424 | [#6025](https://github.com/developing-today/code/issues/6025) | 369784 | done | no | open | 0 |
| 18445 | [#6077](https://github.com/developing-today/code/issues/6077) | 369786 | blocked | yes | open | 0 |
| 18460 | [#6285](https://github.com/developing-today/code/issues/6285) | 369793 | blocked | yes | open | 1 |
| 18461 | [#6286](https://github.com/developing-today/code/issues/6286) | 369799 | claimed | no | open | 0 |
| 18497 | [#6583](https://github.com/developing-today/code/issues/6583) | 369800 | done | no | closed | 0 |
| 18523 | [#7572](https://github.com/developing-today/code/issues/7572) | 369802 | done | no | open | 0 |
| 18524 | [#7573](https://github.com/developing-today/code/issues/7573) | 369809 | done | no | open | 0 |
| 18525 | [#7574](https://github.com/developing-today/code/issues/7574) | 369817 | parked | no | open | 0 |
| 18526 | [#7585](https://github.com/developing-today/code/issues/7585) | 369820 | done | no | open | 0 |
| 18555 | [#7938](https://github.com/developing-today/code/issues/7938) | 369827 | done | no | open | 0 |
| 18629 | [#8300](https://github.com/developing-today/code/issues/8300) | 369832 | blocked | yes | open | 1 |
| 18630 | [#8301](https://github.com/developing-today/code/issues/8301) | 369837 | blocked | yes | open | 1 |
| 18631 | [#8302](https://github.com/developing-today/code/issues/8302) | 369843 | blocked | yes | open | 2 |
| 18632 | [#8303](https://github.com/developing-today/code/issues/8303) | 369847 | blocked | yes | open | 3 |
| 18814 | [#9060](https://github.com/developing-today/code/issues/9060) | 369848 | claimed | no | open | 0 |
| 18827 | [#9077](https://github.com/developing-today/code/issues/9077) | 369857 | done | no | open | 0 |
| 19016 | [#9506](https://github.com/developing-today/code/issues/9506) | 369865 | done | no | open | 3 |
| 19021 | [#9511](https://github.com/developing-today/code/issues/9511) | 369868 | done | no | open | 2 |
| 19055 | [#9608](https://github.com/developing-today/code/issues/9608) | 369874 | done | no | open | 0 |
| 19090 | [#10040](https://github.com/developing-today/code/issues/10040) | 369885 | done | no | closed | 0 |
| 19095 | [#10050](https://github.com/developing-today/code/issues/10050) | 369905 | blocked | yes | open | 1 |
| 19096 | [#10051](https://github.com/developing-today/code/issues/10051) | 369921 | blocked | yes | open | 2 |
| 19097 | [#10052](https://github.com/developing-today/code/issues/10052) | 369932 | done | no | open | 2 |
| 19100 | [#10070](https://github.com/developing-today/code/issues/10070) | 369942 | done | no | open | 0 |
| 19339 | [#10663](https://github.com/developing-today/code/issues/10663) | 369951 | blocked | yes | open | 0 |
| 19340 | [#10664](https://github.com/developing-today/code/issues/10664) | 369965 | blocked | yes | open | 0 |
| 19341 | [#10665](https://github.com/developing-today/code/issues/10665) | 369975 | blocked | yes | open | 0 |

Stored dependencies and GitHub native prerequisites are independent sources. Native counts came from the individual linked issue's REST `issue_dependencies_summary.blocked_by` via `fleet gh api`. Written acceptance gates and other holds may also govern work. A later state read cannot reconstruct the identity of a hold that may have been cleared earlier. Where evidence of that sequence is needed, retain the pre-repair payload, disposition events and dependency-recheck records.

## Quota shadow and weekly review

AMD's active PM aggregator state contained 64 shadow records with distinct timestamps and populated live and shadow orderings. The first retained record was at **02:30:06.420 UTC** and the last was at **18:00:03.915 UTC**. The largest interval between consecutive records was 1,068.112 seconds. Forty-one of the 64 records had different orderings. These are shadow comparisons. They do not establish which pool actually received a job or how the unselected pool would have performed. The collection spans less than one day of the S3 week. Record deployment provenance, continuity and S2 scoring before starting the gated S4 trial. At readback, the user timer for the gatherer was active and its 18:00 invocation had succeeded. [S3 acceptance](https://github.com/developing-today/code/issues/12697) · [Gatherer](https://github.com/developing-today/code/blob/1f9bc15391ca1c2efa639463a6369392456c27e5/pm-stack/aggregator/scripts/gather-quota.ts)

The source defines the S7 weekly review as a persistent user timer on Monday at 07:20 local with up to 60 seconds of randomized delay. Both user and system managers on AMD reported `LoadState=not-found` for `quota-weekly-review.timer` during this observation. The published first report demonstrates report generation. Timer activation still needs its own deployment and firing receipt. [Timer declaration](https://github.com/developing-today/code/blob/1f9bc15391ca1c2efa639463a6369392456c27e5/nixos/services/pm-stack/default.nix#L684) · [First report](https://github.com/developing-today/code/issues/12694#issuecomment-7101452020)
