# The shared load & workload generator — design decision record

Issue #5891, Decomposition Item 1 of #5685 (DoS suite & performance
benchmark programme). Programme: epic #2056. This record settles only
what #5891 owns: the architecture of the one workload generator that
both the honest benchmark workloads and the adversarial DoS profiles
run through, the interface surfaces it drives, the profile catalogue
shape, and the measurement/assertion contract its consumers read. It
does not re-decide the sibling tickets: the standard server baseline
(environment, scales, versioning) is #5894, the per-interface DoS
scenario catalogue is #5892, CI tiering is #5893, scaling comparisons
#5895, the per-interface matrix #5896, and gate budgets/waivers
#5897. Where a choice here constrains those tickets, the constraint
is stated as an invariant for them to consume, not as their design.

Scope and prior art, verified against `main` (triage task 10293,
2026-11-08, at `9b183d7bb`; re-verified by task 13155 at
`52d72cc5`): `pkgs/id/benches/world_bench.rs` is a hand-rolled
in-process benchmark harness with five workloads (`guest_replay`,
`journal_append_fsync`, `checkpoint_compaction`, `session_fanout`,
`many_worlds`) and a machine-readable result line (median/min/p50/
p95/p99, target checks) that `pkgs/id/benches/BASELINES.md` builds
the F11.8 baseline discipline on. `pkgs/id/tests/common/`
(`mod.rs`, `chain.rs`, `ingest.rs`) is the established shared
test-support module, included by integration tests via `mod common;`.
No unified generator exists: nothing today drives one workload
definition against more than one interface surface, and nothing
generates adversarial traffic at all.

## Decision summary

1. **One generator core, one source file home.** The generator lives
   in `pkgs/id/tests/common/loadgen.rs`, beside the existing shared
   test support. Integration/DoS tests reach it via the existing
   `mod common;` include; benches reach the same source via a
   `#[path]` include. No new crate and no published API in v1.
2. **Workloads are data, profiles are parameters.** A workload is a
   named operation mix over the five honest operation families the
   parent names (identity operations, capability checks, journal
   append/read, world sync, blob transfer). A profile is a
   parameterization — honest or adversarial — of rate, concurrency,
   payload shape, and misbehaviour, applied to a workload. The same
   workload definition runs as a benchmark and as the honest phase
   of a DoS scenario; only the profile changes.
3. **Three target surfaces behind one trait.** `InProcess` (direct
   actor/core calls, as `world_bench` does today), `LocalSocket`
   (the server's local socket/IPC surface), and `Iroh` (network
   endpoints, loopback or remote). A workload names the operations it
   needs, never the transport; the surface adapter owns framing,
   connection handling, and teardown.
4. **Determinism before realism.** Every run is driven by a recorded
   seed; operation mixes are fixed-count, not time-sampled; payload
   corpora are generated from the seed, never fetched. A run that
   cannot be replayed from its header (seed + profile + workload +
   scale + surface) is a debugging session, not a measurement.
5. **One output contract for both kinds of run.** Every run emits
   the measurement record of §4: benchmark runs are judged by
   baseline diff (#5894/#5897), DoS runs by the health assertions of
   §5. The generator asserts nothing itself beyond well-formedness
   of its own traffic; verdicts belong to the consumers.

## 1. Generator core

The core is a single-threaded scheduler driving a fixed population
of virtual actors:

- **Actor model.** A run instantiates `actors` virtual identities
  (signing keys derived from the seed), each with a scripted op
  stream produced by the workload's mix table (e.g. 60% journal
  append, 25% capability check, 15% sync round). Mix tables are
  counts per 1,000 ops, so a run's op stream is a pure function of
  (seed, workload, scale).
- **Pacing.** Two modes, chosen by the profile: *closed-loop*
  (actors issue the next op when the previous completes — the
  benchmark mode, measuring service capacity) and *open-loop*
  (ops issued at a target rate regardless of completions — the DoS
  mode, measuring overload behaviour). The mode is part of the
  profile and recorded in the run header; results from the two modes
  are never compared.
- **Lifecycle phases.** Every run has `warm_up` (discarded, per the
  #5894 baseline discipline), `measure` (honest traffic), and — for
  adversarial profiles only — `attack` and `recover` phases (§5).
  Phase boundaries are op counts, not wall-clock, except the
  recovery window, which is a wall-clock assertion by design.
- **Where it runs.** The generator is test/bench infrastructure. It
  links the same client/core code the real surfaces expose, and it
  never ships in the server binary: a load tool compiled into the
  product would be an attack surface of exactly the kind this
  programme exists to defend against.

## 2. Target interface abstraction

```text
trait Target {
    fn connect(seed, actor) -> Session;
    fn identity_op(session, op) / capability_check(...)
    fn journal_append(session, bytes) / journal_read(...)
    fn sync_round(session) / blob_put(session, bytes) / blob_get(...)
}
```

- `InProcess` implements the operations as direct calls on the core
  types (the `world_bench` pattern). It is the reference surface:
  cheapest to run, no transport noise, used by the F11.8-style
  regression benches.
- `LocalSocket` implements them against the server's local
  socket/IPC protocol. This is the surface the standard server
  baseline (#5894) measures.
- `Iroh` implements them against network endpoints. Loopback Iroh is
  the default in tests; a remote endpoint is an explicit,
  manually-invoked configuration, never a CI default.

A workload that needs an operation a surface cannot express (e.g.
blob transfer in-process before the blob API exists there) is
*unsupported on that surface*, recorded as such in the run header —
never silently downgraded to a different operation. The
per-interface coverage that results is #5896's input, not a gap
this design papers over.

## 3. Profiles

**Honest profiles** are the three scales of #5894 (S0 single user,
S1 small community, S2 large world) applied to each workload: actor
count, dataset shape, and mix come from the scale; the profile adds
only pacing (closed-loop) and duration (op count per actor).

**Adversarial profiles** are named misbehaviour parameterizations,
each a modifier on an honest profile rather than a workload of its
own:

- `churn` — connection open/close floods, short-lived sessions;
- `slowloris` — half-open connections, dribbling partial frames;
- `amplify` — small requests chosen for large responses or fan-out;
- `bomb` — decompression bombs and oversized payloads at the cap
  boundary (cap-sized and cap+1 are both first-class cases);
- `garbage` — malformed frames, wrong-version handshakes, truncated
  messages;
- `exhaust` — state-growth pressure: many identities, many worlds,
  deep journals, within protocol rules but at hostile volume.

The catalogue of *scenarios* — which profile, on which surface,
against which limits, with which expected defence — is #5892's
deliverable. This design fixes only the profile mechanism: a new
adversarial behaviour is a new named modifier with its parameters
in the run header, implemented once in the generator, reusable on
every surface.

## 4. Measurement contract

Every run writes one header line and one summary line, in the
key=value style `world_bench` already emits and `BASELINES.md`
already parses:

- **Header:** seed, workload, profile (incl. pacing mode), scale,
  surface, server commit, machine class, generator version.
- **Summary, per operation family:** count, errors by class
  (refused / timeout / transport / protocol), latency p50/p95/p99
  in microseconds, plus run-level ops/sec and the resource counters
  the surface can report (peak connections, peak memory where the
  harness owns the server process).
- **Units are part of the contract.** A change to a unit, a metric
  definition, or the mix table of a workload is a generator version
  event under #5894's baseline versioning: numbers across the event
  are incomparable (#1619), never silently comparable.

## 5. Assertion contract (DoS runs)

The generator reports; the scenario (in #5892's catalogue, executed
as integration tests) asserts. The assertions a scenario may use are
fixed here so scenarios stay declarative:

- `healthy_within(<duration>)` — after the attack phase ends, the
  honest probe profile (S0, closed-loop) succeeds end-to-end within
  the window;
- `no_wedge` — server queues drain to empty and stay empty for a
  probe interval;
- `honest_throughput_at_least(<fraction>)` — during the attack
  phase, the honest co-tenant profile sustains at least the named
  fraction of its solo rate;
- `state_bounded(<bytes>)` — server state growth attributable to
  attack traffic stays under the named bound once connections close.

Timeouts and windows are scenario parameters with the defaults
#5897's budgets will tune; the assertion *vocabulary* is what this
record fixes.

## Rejected alternatives

- **External load tools (k6, wrk, vegeta).** They speak HTTP/gRPC
  to a network port only: no in-process surface, no signed identity
  operations, no capability/journal semantics, and a second
  toolchain (JS/lua scripts) for the fleet to maintain. The
  programme's surfaces are mostly not HTTP; the tool would test
  the least representative surface best.
- **One harness per suite (benches bespoke, DoS bespoke).** The
  status quo, and the duplication the parent filed #5891 to end:
  two mix tables for the same workload would drift, and honest-vs-
  adversarial comparisons — the programme's core question — would
  compare two generators, not two profiles.
- **A new workspace crate (`id-loadgen`).** Correct long-term shape
  if a third consumer appears (e.g. a shipped soak-test binary),
  but v1 has exactly two consumers, both inside `pkgs/id`'s own
  test/bench targets. The `tests/common` home follows the existing
  precedent; extraction to a crate is a mechanical follow-up if
  the consumer count changes, and the module boundary in §1 is
  drawn so that extraction changes no call sites' semantics.
- **Criterion as the bench engine.** Criterion's statistical model
  serves micro-benchmarks; the baseline discipline here is the
  recorded-median convention of `BASELINES.md` over whole workloads
  with phases, which `world_bench`'s hand-rolled harness already
  implements and the stored baselines already cite.

## Invariants for consumers

1. A baseline number is meaningless without its run header (seed,
   workload, profile, scale, surface, generator version) — #5894
   artefacts cite the header, not just the summary.
2. Honest and adversarial results for one workload are comparable
   only because they share the workload definition and the honest
   phases; a change to a workload definition is a version event
   for both.
3. No adversarial profile runs against a non-loopback, non-owned
   target by default; the generator refuses a remote `Iroh` target
   without an explicit operator flag in the run configuration.
4. The generator never weakens a server limit to make a run pass:
   limits under test are the shipped defaults unless the scenario
   names a configuration as part of its own setup (#5892).

## Implementation order (for the follow-on packets)

1. `loadgen.rs` core: actor scheduler, mix tables, seeded corpus,
   `InProcess` surface; `world_bench` re-expressed on it without
   changing its output lines (the F11.8 baselines must stay
   comparable — same numbers within noise on the reference class).
2. Measurement header/summary writer + the #5894 header fields.
3. `LocalSocket` surface; honest profiles at S0/S1/S2.
4. Adversarial modifiers (churn, garbage first — they exercise the
   framing paths every other modifier needs), then the §5
   assertion helpers for #5892's first scenarios.
5. `Iroh` loopback surface; remote-target guard (invariant 3).

Filed from triage task 10293's scoping (2026-11-08) and queue task
13155. Parent: #5685. Sibling records: #5894 (PR #7620).
