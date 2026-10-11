# Phase 1: events, storage, crash safety, and the identity store

- Plan: [phased plan](../../2026-10-11T11-08-42Z_design_remaining_work/plan-phases.md), section 6.
- Start commit: `baba0ea7` (origin/main, after PR #153, Phase 0).
- Base branch: `phase-1`. Track branches: `p1-chainext`, `p1-crash`, `p1-identity`, `p1-caps`, then `p1-store` and `p1-traits` after chainext merges.
- Storage decision (P1.1, decided): redb everywhere. Turso was measured and rejected; libsql is replaced. Record: [round 2 turso results](../2026-10-11T11-08-42Z_design_remaining_work/research-round2/turso/RESULTS.md).

## Steps

| Step | Track | Status | Commits | Tests added | Notes |
| --- | --- | --- | --- | --- | --- |
| P1.1 Storage decision record | lead | done | this file | none | redb everywhere; libsql replaced |
| P1.2 ChainExt | chainext | todo | | | |
| P1.3 Event store on redb | store | todo | | | after P1.2 |
| P1.4 Crash-safety gate | crash | todo | | | in-process fault model; corpus |
| P1.5 Waiting caps | caps | todo | | | |
| P1.6 Store traits over redb | traits | todo | | | after P1.3 |
| P1.7 Canonical CBOR vectors | chainext | todo | | | |
| P1.8 Ingest: verify with AnyHeader::decode, freeze forks | store | todo | | | after P1.3 |
| P1.9 Identity store on redb | identity | todo | | | libsql removed, AES-GCM values |
| P1.10 Crash gate for the identity store | crash | todo | | | after P1.4 and P1.9 |

## Decisions

- ChainExt on every event; no LogExt exists in p2panda-core 0.7.1.
- Waiting caps: 256 per author, 7 days, configurable per world.
- Identity values are encrypted with AES-256-GCM; data may be discarded at this phase.
- Store creation: a temporary file, a first commit, then a rename with a directory fsync.

## Open questions

- None yet.

## Changes to the plan

- None yet.

## Review notes and regression tests

- To be filled at each merge and at the end-of-phase review.

## End

- End commit and full test result: pending.
