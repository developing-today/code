# Authoring Roc worlds with `id world dev`

Guide for the local authoring loop (F12.15, issue #5541; design
`doc/2026-11-01_design_id558.md`). `id world dev DIR` compiles a
Roc world app into a disposable in-memory scratch world on your own
machine, hot-swaps each saved edit through the F12.9 upgrade path,
and gates every promotion on your regression fixtures. It makes no
network connection and can never install into a hosted world
(design invariant 1); when the session exits, the scratch world is
discarded.

## Setup

- **Roc toolchain.** The compiler version is pinned in
  `pkgs/id/nix/roc-version` (currently `nightly-2026-10-04-130536d`);
  the example build scripts refuse a mismatched `roc` on PATH
  (`pkgs/id/examples/roc-toolchain.sh`). `id` locates `roc` itself
  for `world dev`; a wrong-version compiler surfaces as a compile
  diagnostic, not a silent fallback.
- **Platform host (once per checkout).** The `roc-world` platform's
  wasm host is a build artifact, not checked in:
  `pkgs/id/examples/roc-world/build-host.sh` produces
  `targets/wasm32/host.wasm` (gitignored) using the system `zig`.
  The host `build.zig` resolves the Zig `Optimize` enum names at
  comptime, so it builds under both Zig 0.16 (`ReleaseSmall` /
  `Debug`) and Zig 0.17 (`small` / `debug`) spellings — if you see
  `no field named 'Debug' in enum 'lang.Optimize'`, your checkout
  predates that fix; rebuild rather than editing the enum names
  back (they have ping-ponged once each, #8152).
- **App layout.** An app directory holds `main.roc` plus any
  sibling `.roc` modules; `main.roc`'s header declares the platform
  (`platform "../roc-world/platform.roc"` for the checked-in
  examples). `pkgs/id/examples/roc-counter` is the small reference
  app used by the benchmark below.

## The loop

- **One-shot:** `id world dev DIR` compiles once, launches the
  scratch world, applies any `--input` values in order, and
  reports each step's view. Useful for a first compile and for
  scripted checks.
- **Watch:** `id world dev DIR --watch` keeps the session alive.
  Filesystem changes are debounced (`--watch-debounce-ms`, default
  150 ms; bursts coalesce into one candidate) and rescanned within
  the compiler's bounds. Each candidate is compiled once, from one
  source snapshot: if the sources change again while a candidate
  is being built or validated, that candidate is discarded as
  stale and the newest snapshot is built next — a stale candidate
  is never promoted (invariant 2). The session is bounded
  (`--watch-max-cycles`) and always terminates on its own.
- **Promotion is transactional.** A candidate becomes active only
  after (a) every `--fixture` gate passes and (b) the F12.9
  probe/migrate/commit upgrade succeeds and the candidate replays
  the session's input history to the migrated records. Any failure
  leaves the last-good program and its state running, with the
  diagnostic recorded in the session's last-good record
  (invariant 3). Later edits migrate the live state; only the
  initial compile installs fresh.

## Fixture gates

`--fixture FILE` (repeatable) replays a recorded fixture against
each candidate before promotion. Dev uses the design's
cross-module regression mode: a fixture recorded under an earlier
module hash still replays — its recorded inputs and expected
records are the contract, and the gate reports the cross-module
fact. Ordinary `id world replay` / `verify` stay strict and still
reject a module-hash mismatch (invariant 5). A fixture with
incomplete history, no expected records, an unsupported event, or
an input before the last install cannot gate a candidate: the
gate fails with that specific diagnostic instead of guessing.
Fixtures cover only the recorded event stream — they say nothing
about unrecorded host events, and the gate output does not claim
otherwise.

## Deterministic fuzz (separate from the loop)

`id world dev DIR --fuzz DECL` compiles the candidate and runs a
deterministic campaign instead of the edit loop. The declaration
file is bounded, versioned JSON naming the generated input
alphabet and the invariants to check (guest calls do not trap,
records remain valid JSON, declared record keys remain present,
declared numeric bounds/order relations hold). `--fuzz-seed` and
`--fuzz-cases` fix the run: the same candidate, declaration, seed,
and budget produce the same input sequence and the same first
failure (invariant 6). A failure prints the seed and the first
failing input sequence, so it reproduces exactly. Fuzzing never
runs on the save-to-running path; at most, request a small bounded
smoke run explicitly.

## Latency benchmark (Phase 5)

Design invariant 8 defines the number: from the debounced
filesystem change to the CLI's "candidate active" result,
including compile, fixture replay, and migration; cold start is
reported separately. The checked-in harness is the ignored test
`bench_phase5_save_to_active_p95_on_roc_counter` in
`crates/id-worlds/src/world_dev.rs`: it stages `roc-counter` as a
user app dir, launches a session (two `inc` inputs of history),
then performs 22 title-only edits — each changes the module but
not the records, so the fixture gate genuinely replays and passes
— discards the first two as warm-up, and reports the p95 of the
remaining 20 edit→rescan→compile→gate→migrate→active cycles.

Run it with:

```sh
pkgs/id/examples/roc-world/build-host.sh   # once per checkout
cargo test -p id-worlds --features sandbox --lib bench_phase5 -- --ignored --nocapture
```

Recorded run (this document's landing):

- Machine/toolchain: fleet laptop (amd-6), Roc
  `nightly-2026-10-04-130536d`, Zig 0.17.0, debug test profile.
- Cold start (discover + first compile + launch): 3262 ms.
- Warm samples (ms): 4181, 4210, 4229, 4232, 4243, 4265, 4268, 4280, 4316, 4322, 4358, 4366, 4372, 4430, 4498, 4672, 4950, 5119, 5366, 5646
- **p95 save-to-active: 5366 ms** (design target: under
  3000 ms).

## Troubleshooting

- **"not a world platform directory (need platform.roc and
  targets/wasm32/host.wasm)"** — the platform host has not been
  built in this checkout; run `build-host.sh` (above).
- **A save does not promote.** Read the cycle outcome: a compile
  diagnostic means the candidate never launched; `fixture_blocked`
  names the mismatching fixture; `migration_failed` means the
  F12.9 probe rejected the state transition and rolled back;
  `stale_discarded` means you saved again mid-build and the newer
  snapshot is already queued. In every case the last-good version
  is still running — fix forward with the next save.
- **The same failing snapshot is not retried forever.** A snapshot
  that fails any stage is marked known at its attempt; the loop
  waits for the *next* change (invariant 7). Save again (even a
  trivial edit) to retry after fixing the cause.
- **Fuzz found a failure.** Re-run with the printed seed and case
  budget to reproduce; the first failing input sequence in the
  report is the minimal repro the campaign found.

**Status: the 3-second target currently MISSES.** A second run on
a busier machine (p95 9214 ms) split the stages: Roc compile
median 2301 ms and fixture-gate + migration + history-replay
median 2346 ms per edit. Neither stage fits the target alone, so
the follow-up is stage optimization, not gate removal (the design
is explicit: optimize the measured stage and retain the same
correctness gates). The known directions, in expected-value order:
serve repeated compiles from the world compile cache where the
input set allows (#6235 landed the cache for identical inputs;
title edits defeat it, so cache granularity is the question),
avoid re-instantiating the candidate sandbox separately for the
fixture gate and the migration probe, and revisit the Roc build
flags the bounded compiler engine invokes. The benchmark test's
assertion stays at 3000 ms: it is the oracle that proves the miss
today and will prove the fix when the optimization lands.
