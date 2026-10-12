# Phase 3: sync, blobs, and relays

- Plan: [phased plan](../../2026-10-11T11-08-42Z_design_remaining_work/plan-phases.md), section 8.
- Start commit: `02569c9e` (origin/main, after PR #156, Phase 1). Phase 2 is still in review and is not a blocker for these steps; P3.4 waits for its membership interface.
- Base branch: `phase-3`. Track branches: `p3-range` (P3.1, P3.1a to c), `p3-logsync` (P3.2, P3.3), `p3-relays` (P3.5, P3.6, P3.8).
- Owner decisions used: K and T are local (N6); range above the 3.5 percent crossover, LogSync below (N5); n0 relay now (N19); relay ops per the S11 runbook (N25); QAD-capable relays first (N43); startup waits only for the home relay (N32).

## Steps

| Step                                       | Status        | Commits                                | Tests added                                                   | Notes                                                |
| ------------------------------------------ | ------------- | -------------------------------------- | ------------------------------------------------------------- | ---------------------------------------------------- |
| P3.1 Range sync on the redb store          | done          | a4bc0094                               | unit per mode and bound; grid property over K and T; loopback | K=16, T=16 local; /id-sync/1                         |
| P3.1a Fork in a batch isolated             | done          | ff5401b5                               | red first: 20 unrelated events survive a fork                 |                                                      |
| P3.1b T=0 refused                          | done          | 4509f3d8                               | regression                                                    |                                                      |
| P3.1c Chunked flights under the frame cap  | done          | f398ec91                               | bootstrap above the limit, both directions                    | 512 KiB budget under the 1 MiB stream cap            |
| P3.2 LogSync through p2panda-sync          | done          | fedb1644, f377b552, 0d03a3de, ba45deaa | log_sync tests; batch equivalence property                    | vendored p2panda-sync, sqlx absent (lock check test) |
| P3.3 Method selection                      | done          | 072f1c18, 80c3ce37                     | threshold at 0.034, 0.035, 0.036 of chains; both sides agree  | estimate from the store                              |
| P3.4 Blob gate                             | partly done   | e8f01a86, 19acd260, 0dfa6de5           | 8 end-to-end, 9 unit, property                                | gate not wired into serve: open issue                |
| P3.5 Startup waits only for the home relay | done          | ed9fd4da                               | dead home relay ends at deadline; live home gates alone       |                                                      |
| P3.6 Relay map QAD order                   | done          | f828d2e8                               | 5 validator tests                                             | validator refuses; iroh orders by URL                |
| P3.7 Real-network holepunch and QAD        | owner-blocked |                                        |                                                               | needs two networks                                   |
| P3.8 Relay operations                      | done          | a0b3ce37                               | tests parse the unit file and kit configs                     |                                                      |

Phase 1 and 2 fixes carried into this branch: 937551e9 and bac287b4 (restart property, a backlink that is present but unapplied), 2d6b33e4 (a lower fork does not supersede a proof header an event names; format v6).

Performance: the bulk LogSync test went from a 30 s timeout (230 s) to 22.7 s in debug after batch ingest (0d03a3de, ba45deaa). The large bootstrap test is on demand (`#[ignore]`, release command in its reason).

## Decisions

- K and T are local settings and never on the wire. Changing them needs no version bump. A change to the wire format needs /id-sync/2.
- LogSync dependency: p2panda-sync pulls p2panda-store's sqlite default. Resolve in-repo (vendored copy with the one dependency line changed) rather than an external fork, so no account action is needed.

## Open questions

- Blob gate is not wired into serve: product still serves blobs to any peer until a referencing-world index exists (issue).
- Stores at format v5 or v6 are refused and must be rebuilt (owner: data may be discarded).
- Relay validator is not applied to custom maps yet (Phase 7); await_home_relay has no caller until serve wiring.
- Ingest per session still decodes and verifies each event on the responder (decode-bound).
- Real-network numbers are not measured (P3.7).

## Changes to the plan

- P3.2 vendors p2panda-sync in-repo rather than an external fork (no account action).
- P3.1 K and T are local; no compatibility change needed.
- The QAD rule is a validator on URL order, since iroh orders relay maps by URL.

## Review notes and regression tests

- Restart-property bug found by a deterministic seed: an event waiting on a present, unapplied backlink corrupted the store on reopen (fixed, 937551e9).
- Lower-fork proof supersede: fixed with a proof-header table and format v6 (2d6b33e4).
- Bulk LogSync timeout: fixed by batch resolution and blocking-worker writes (0d03a3de, ba45deaa).
- End-of-phase review: see the review branch commits (P3-review) when merged.

## End

- Full test result on the head before the review: `cargo test --all-features --no-fail-fast` exit 0 (lib 1317, cli 116, doc tests).
- Not verified: Nix build with the vendored p2panda-sync; real-network sync and relay numbers.
