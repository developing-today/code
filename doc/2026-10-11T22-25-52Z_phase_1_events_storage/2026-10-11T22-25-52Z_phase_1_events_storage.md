# Phase 1: events, storage, crash safety, and the identity store

- Plan: [phased plan](../../2026-10-11T11-08-42Z_design_remaining_work/plan-phases.md), section 6.
- Start commit: `baba0ea7` (origin/main, after PR #153, Phase 0).
- Base branch: `phase-1`. Track branches: `p1-chainext`, `p1-crash`, `p1-identity`, `p1-caps`, then `p1-store` and `p1-traits` after chainext merges.
- Storage decision (P1.1, decided): redb everywhere. Turso was measured and rejected; libsql is replaced. Record: [round 2 turso results](../2026-10-11T11-08-42Z_design_remaining_work/research-round2/turso/RESULTS.md).

## Steps

| Step                                    | Track    | Status                                  | Commits                                          | Tests added                                          | Notes                                                 |
| --------------------------------------- | -------- | --------------------------------------- | ------------------------------------------------ | ---------------------------------------------------- | ----------------------------------------------------- |
| P1.1 Storage decision record            | lead     | done                                    | this file                                        | none                                                 | redb everywhere; libsql replaced                      |
| P1.2 ChainExt                           | chainext | done                                    | f96b8af6                                         | 22 unit, 11 property, seeds                          | p2panda-core at rev 3e84cac, no features              |
| P1.3 Event store on redb                | store    | done                                    | 949b2222                                         | 26 unit, crash creation sweep                        | first-creation: temp file, commit, sync, rename, sync |
| P1.4 Crash-safety gate                  | crash    | done                                    | d36e53d5, 06d84ed2                               | sampled gate, full sweep, lying-sync control, corpus | `just crash-gate`                                     |
| P1.5 Waiting caps                       | caps     | done                                    | caps track commits                               | 21 unit, property                                    | 256 per author, 7 days                                |
| P1.6 Store traits over redb             | traits   | done (LogSync session deferred to P3.2) | d2b1a6ad, d19d2255                               | 10 trait contract tests                              | p2panda-store without default features; sqlx absent   |
| P1.7 Canonical CBOR vectors             | chainext | done                                    | 983649c4                                         | golden and dcbor vectors                             |                                                       |
| P1.8 Ingest: verify, freeze forks, wait | store    | done                                    | b4e7c1eb, 8fc776be, 1a90c102, 414479ec, 46bb1f2a | property over arrival orders, regressions            | fork voids only the forked chain (46bb1f2a)           |
| P1.9 Identity store on redb             | identity | done                                    | 3f04bdbf, a2f842fc                               | 38 identity tests                                    | libsql removed; 75 packages gone                      |
| P1.10 Crash gate for the identity store | crash    | done                                    | 2eb950b9, fd311808, ad4c6e05                     | identity sweep, first-creation fix                   | creation by rename fixed 20 of 147 stranded runs      |

## Decisions

- ChainExt on every event; no LogExt exists in p2panda-core 0.7.1.
- Waiting caps: 256 per author, 7 days, configurable per world.
- Identity values are encrypted with AES-256-GCM; data may be discarded at this phase.
- Store creation: a temporary file, a first commit, then a rename with a directory fsync.

## Open questions

- A released waiter's arrival time: counted from first arrival (decided in P1.11 review).
- Identity store: the first-creation temp-file pattern is used; the `.creating` file is not locked against another process (reviewer, not reproduced).
- The LogSync session needs p2panda-sync, which pulls p2panda-store's sqlite default. Fork or feature change in P3.2.
- Caps and restarts: a conflict whose rival was dropped by a cap is seen only when the rival is sent again (documented and pinned by a test).

## Changes to the plan

- P1.11 added: waiting events persisted in redb (task 02 said so; the store track had kept them in memory). Format v3.
- Dependency voiding narrowed in Phase 1: a fork voids only its own chain. Authority-based voiding is Phase 2 (P2.3).
- Known issue fixed late: released waiters and fork proof headers left in neither state (P1.11 crash fix, 7a2e503b).
- conform scope `id` added (66983e25).

## Review notes and regression tests

- End-of-phase review (review-phase-1): pinned the sequence and backlink rule at decode and at ingest; pinned the LogSync limit; found the dependency-voiding stall (fixed in 46bb1f2a).
- Regression tests: a fork voids only the forked chain; honest dependents apply; a released waiter keeps its first arrival time; a held proof header that a fork makes present stays held; a rival refused by the cap is seen only when sent again.

## End

- Last commit before the PR: see the PR. Full test result on it: `cargo test --all-features --no-fail-fast` exit 0 (lib 1112, cli 116, crash and property suites, doc 19).
- Not verified: the Nix build (p2panda git dependency needs outputHashes in default.nix and flake.nix, unverified).
