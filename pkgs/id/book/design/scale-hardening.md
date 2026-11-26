# Design: scale, performance and adversarial hardening

This chapter is design, plus the measured findings the scale programme recorded. It covers 24 records homed here in the archive survey: the per-world resource and budget records, the #5685 denial-of-service and benchmark programme (#5891–#5897), the spill and sharding line, the waiting-pool bound, and the #6304 scale programme's stress design with its first measured limits. Measured values are cited as recorded findings with their dates and sources; they are findings from the programme's records, not live measurements taken for this book. The final section separates what shipped from what is still design.

Related chapters: [Design: worlds and the guest runtime](worlds-runtime.md), [Design: hosted and serverless editions](hosted-editions.md).

## Budgets first: resource profiles and per-call metrics

The scale design starts by making every cost bounded and named.

- `2026-10-16T10-00-00Z_feature_world_resource_profile` and `2026-10-16T10-30-00Z_feature_durable_world_resource_profiles` (2026-10-16): per-world resource profiles, and the durable form of those profiles, so a world's limits survive restart.
- `2026-10-17T15-30-00Z_feature_world_perf_budgets` (2026-10-17): per-call resource metrics and performance budgets.
- `2026-10-17T16-30-00Z_fix_territory_view_fuel` (2026-10-17): the fix record for territory's view running out of fuel with six or more seats — the incident that made frame cost a measured quantity instead of an estimate.
- `2026-10-27T00-00-00Z_baseline_bytes_per_frame.md` (2026-10-27, F11.9): the bytes-per-frame baseline table.
- `2026-11-01_design_id557.md` (2026-11-01, F12.12, issue #557): a per-world memory budget and many-world scale. The design recommends one finite total per-world memory budget as a deterministic logical-charge ceiling for allocations owned by one world — explicitly not an RSS guarantee, since allocator metadata, fragmentation, thread stacks and process-shared caches sit outside per-world accounting. Its measurement target is stated in the record: 1,000 idle durable worlds plus 100 active worlds at 20 events per second each, with p99 command latency below 50 ms and no state loss through eviction, checkpoint and reopen.
- `2026-11-09T05-15-00Z_design_id_server_invariants`: server-side invariants — budgets, observability, exploit resistance, failure containment.
- `2026-11-10_design_id6002_cooperative_timeout_contract.md`: the cooperative timeout contract — invocation budget, checkpoint convention, hard-kill outcomes.
- `2026-11-08_design_id5779.md`: deterministic testing across time, replay, corpus and soak — the testing design the scale runs depend on, with the testing architecture reference `2026-03-29T00-00-00Z_reference_testing_architecture` (2026-03-29) as its base.

The shipped runtime defaults that these budgets operate on, from the CLI survey for this book: guest fuel budget 10,000,000 per call; world memory 16 MiB; total world memory 64 MiB; native memory 256 MiB; native CPU 10 seconds; native deadline 10,000 ms.

## The #5685 programme: one generator, honest and adversarial

Programme #5685 is the DoS suite and performance benchmark programme under epic #2056. Its structural decision, repeated in each child record, is that honest benchmarks and adversarial profiles run through one shared load and workload generator, one baseline, and one measurement contract — so an adversarial result and a benchmark result are comparable by construction. The children:

- #5891 — the shared workload generator: `2026-11-08T11-27-00Z_design_shared_workload_generator` (2026-11-08). One generator drives both honest workloads and adversarial profiles against the interface surfaces. The record's verified baseline: the existing bench harness has five workloads (`guest_replay`, `journal_append_fsync`, `checkpoint_compaction`, `session_fanout`, `many_worlds`) and no unified generator exists yet; nothing drives one workload definition against more than one interface surface, and nothing generates adversarial traffic at all.
- #5892 — the DoS scenario catalogue per interface: `2026-11-08_decision_id5892_dos_scenario_catalogue.md` (2026-11-08). For each transport and interface surface, the catalogue defines scenario name, interfaces, load profile, pass/fail criteria and recovery assertions. Its bounded fixture profile, as consumed by the CI tiering record: fixed seed, a 30-second attack phase, a 30-second recovery window, 128 offered operations per second open-loop, and a 1 valid operation per second canary.
- #5893 — CI tiering: `2026-11-16T09-52-00Z_design_ci_tiering_adversarial_benchmark_suites` (2026-11-16). The tier structure for both suites, each tier's time budget, what gates and what merely reports, and environment guards that keep every tier off shared or production targets. Its verified input state: the bench workflow runs on push to main and manual dispatch only, with a 60-minute timeout, and is explicitly non-blocking; adversarial coverage exists as an in-process refusal suite (`pkgs/id/tests/world_dos.rs`), not as a tiered load workflow. The generator's three target surfaces are `InProcess`, `LocalSocket` and `Iroh`.
- #5894 — the standard server baseline: `2026-11-08T09-55-28Z_design_standard_server_baseline` (2026-11-08). The reference environment, baseline workload suite, scales and baseline versioning that every #5685 measurement cites. Its adopted rules from the earlier F11.8 baseline: numbers are comparable only on the same machine class; a unit change makes a benchmark incomparable rather than silently comparable; a flag on a different machine class is a prompt to investigate on the reference machine, not proof of a regression.
- #5895 — scaling comparisons, and #5896 — the per-interface matrix: named as siblings in the #5891, #5893 and #5894 records; their own design records are not among the 24 records homed in this chapter, and this chapter makes no claim about their contents.
- #5897 — regression gates: `2026-11-08T23-45-30Z_design_id5897_regression_gates` (2026-11-08). Benchmark regression gates, waivers and result retention: pass/fail expressed as server behaviour under defined load profiles, recovery within a stated number of seconds after load stops.

Comms abuse is the application-level half of the same programme: `2026-10-29T10-55-00Z_investigation_comms_abuse_controls` (blocks, rate limits and per-counterparty budgets) and `2026-11-01_design_id2033.md` (abuse-control primitives v1 for ID comms). `2026-11-08T23-05-00Z_design_id5797_latency_budget_measurement.md` designs the per-fallback-tier latency budget and measurement plan, so each transport fallback tier carries its own latency budget instead of inheriting the fastest tier's numbers.

## One shipped bound: the waiting pool total cap

`2026-10-31T04-00-00Z_design_waiting_pool_total_cap` (2026-10-31, issue #1567) is the chapter's clearest shipped hardening record; its status line reads "implemented". The problem: an event waits in the waiting pool when a dependency has not arrived, held in memory and mirrored to disk. The existing limits were per-author (default 256 waiting events) and maximum age (default 7 days). Author keys are self-certifying 32-byte Ed25519 public keys, generated at zero cost, so a per-author cap bounds nothing against a Sybil flood. The fix adds a pool-wide total cap: `DEFAULT_TOTAL` is `32 * DEFAULT_PER_AUTHOR`, which is 8,192 events. The record is the pattern the rest of the adversarial design follows: find the resource an attacker can multiply for free, and put the bound on the total, not on the identity.

## Hot worlds: sharding, spill and leases

Three designs handle a world that outgrows one host, and they disagree in method on purpose — each is a different answer to a different limit:

- `2026-10-30T19-35-00Z_investigation_hot_world_sharding` (2026-10-30): the sharding investigation — when one world outgrows one Durable Object.
- `2026-11-01_design_id2819.md` (2026-11-01, under parent #2059): the native-spill write lease protocol v1. One authoritative journal and one append gate per world; the Durable Object is sole lease grantor and append authority; epoch fencing at the append linearization point; no local commits by a native node; fail-closed recovery.
- `2026-11-08_design_id5787.md` (2026-11-08): Spill v2 — native spill pools, lease failover and multi-holder fencing. Its recommendation is stated in one line: honest v2 is failover plus read replicas plus a pool of eligible holders; it is not write scaling. At every instant a world still has exactly one holder and one epoch chain. Concurrent multi-writer spill is rejected in the record, with the reason: the append gate is the write bottleneck by design — every accepted mutation is serialized, epoch-checked and synced by the single append gate — so adding holders adds contenders, not throughput. If write scaling is ever proven necessary, the record states it arrives as world splitting under the region and shard designs, not as parallel holders of one journal.
- `2026-11-12T19-35-00Z_feature_world_lease_handoff_actor_boundary` (2026-11-12): world lease handoff at the actor boundary, the feature record that puts the lease transfer at the world's actor edge.

As of 2026-11-25, `id-worlds` contains the spill pool policy and actor failover
journaling. The spill-admission gate requires a verified successor replay cursor
at or beyond the current grant/handoff position; renewals do not move that
ownership boundary. These are library mechanisms, not a deployed host controller:
`evaluate_spill` has no production callers yet, and real detection/grant/replay
timings remain an acceptance gap on #7572. See the
[replay-admission slice record](../../../../doc/2026-11-25T01-38-00Z_design_spill_replay_gate/2026-11-25T01-38-00Z_design_spill_replay_gate.md).

## The #6304 scale programme and its first measured limits

The scale programme is #6304, under Phase 14 core topic #6281, with plan of record epic #6312 and v2 (growth policy and the degradation ladder) filed as #6305 behind it. Its v1 design record is `2026-11-23T07-53-00Z_design_scale_perf_stress_v1` (2026-11-23): how large worlds are generated, what is measured, and what the first measured limits are. The generator extends the existing bench harness instead of building a second one: deterministic (fixed seed; same seed and same tier produce the same worlds, byte for byte in event content), manifest-verified (expected event counts and final state hash per measured world, so a fast-but-wrong run cannot pass as a result), and run as profiles of the existing bench binary. Implementation issues: #13083 (generator), #13084 (join time), #13085 (frame cost), #13086 (memory), #13087 (the 10x run), #13088 (the 100x run).

The tier table, from the record:

| Axis                          | 1x     | 10x     | 100x      |
| ----------------------------- | ------ | ------- | --------- |
| Idle worlds                   | 1,000  | 10,000  | 100,000   |
| Active worlds                 | 100    | 1,000   | 10,000    |
| Single-world history (events) | 10,000 | 100,000 | 1,000,000 |
| Members on the hot world      | 32     | 320     | 3,200     |

A tier is a measurement target, not a promise; the record states that a run hitting a wall — out-of-memory, eviction storm, p99 blowup, fuel exhaustion — is a result, and the wall is recorded as that tier's measured limit.

The first measured limits, as recorded in the 2026-11-23 record from the committed baselines and the #13085/#13086 profiles on the reference laptop (Ryzen 7 7840U, 16 threads, 123 GB RAM, NixOS):

- Frame cost is flat. A Territory-class guest frame at 2 seats costs view 958,616 fuel and layout 958,683 fuel at 10,000 applied events, at 100,000 applied events, and at 1,000,000 applied events — the same fuel at all three sizes, with wall time between about 126 µs and 148 µs. No size comes near the 10,000,000 fuel default budget. Recorded by the #13085 `frame_cost` profile, 2026-11-23.
- Resident memory per world is in the 18–22 KB band, blended. The record's `stress_mem` rows (#13086, median of 5 runs): 21,834 bytes per world blended at the 1x tier (1,101 worlds held, 28.56 MiB total RSS; idle 11,350 bytes per world; active at 100 events 93,593 bytes per world) and 18,320 bytes per world blended at the 10x tier (11,001 worlds held, 197.90 MiB total RSS; idle 9,334 bytes per world; active 87,945 bytes per world). The investigation programme's summary of the same findings states the band as about 18–22 KB per world blended at 1x and 10x.
- Compaction memory spike: 5.59 MiB over pre-operation RSS at a 2,000-entry journal; 89.34 MiB at a 100,000-entry journal (#13086).
- The durable-append ceiling is the constraint behind the rest. Journal append plus fsync measures 5,180 µs median per append in the committed baseline, which derives an unbatched per-world ceiling of about 193 appends per second. Demand at 1x fleet scale is 100 active worlds at 20 events per second, which is 2,000 appends per second host-wide. Group-commit batching is scoped as #8239; the growth policy around the ceiling is #6305 work.
- Two measured target misses, recorded as misses: many-worlds command p99 of 4,549 ms against the 50 ms target (#557; a later run recorded 5,650 ms), and a `fuel_spin_isolation` ratio of about 3.8–3.9 against the 1.10 target (#1067 line, 2026-11-22). Both trace, in the record's analysis, to durable-append and scheduling contention under concurrent load, not to raw compute.
- Codified limits the generator found: journal size cap 64 MiB per world journal; session checkpoint interval 2,000 ms by default; documents 64 KiB each and 512 list items; compile inputs 8 files and 256 KiB total; SSH surface 64 connections and 16 tunnel streams; per-world caps on entities and programs: none codified — recorded as a gap.

### The open tension: 256 participants against 320 and 3,200

The tier table asks for a hot world with 320 members at 10x and 3,200 at 100x. The codified participant cap is 256: `WorldLimits::default().participants` in `crates/id-worlds/src/world/mod.rs`. The #13083 generator found the wall during restore verification — the 10x tier's 320-member hot world is refused — and the generator caps the hot world at 256 and records the wall in its manifest. The tension is open: either the cap rises, or the tier targets fall, and the 10x run (#13087) and the 100x run (#13088) inherit the decision. This chapter records the tension as the programme recorded it on 2026-11-23; it does not resolve it.

## What exists today

Grounding:

- Shipped: the waiting-pool total cap (record status "implemented", 2026-10-31; total default 8,192 events). Shipped: the bench harness and its committed baselines (`benches/world_bench.rs`, `benches/BASELINES.md`, `benches/baseline.txt`), the in-process adversarial refusal suite (`pkgs/id/tests/world_dos.rs`), and the codified limits listed above, each cited to its source location in the 2026-11-23 record.
- Measured and closed as measurement issues: #13083 (the seeded stress generator; tiers quick, 1x, 10x, 100x, manifest with expected state hashes, restore-verified) and #13085 (frame cost against state size, flat at 958,616 fuel), per the investigation programme's state record of 2026-11-23. #13086 (memory) was in progress in that record; #13084 (join-time benches) was flag-held; #13087 (first 10x run) was open, blocked on #13084 and #13086; #13088 (100x feasibility) follows #13087. Those statuses are the investigation record's, dated 2026-11-23, and are not re-verified live for this chapter.
- Still design: the #5685 programme as a tiered CI system (the tiering record's own verified input is that the bench workflow is non-blocking and the adversarial suite is a unit-tier test file), Spill v2 (a decision-ready proposal; the record states no protocol code is implemented in it), hot-world sharding as a mechanism, the #6305 growth policy and degradation ladder, and any resolution of the 256-participant tension.
