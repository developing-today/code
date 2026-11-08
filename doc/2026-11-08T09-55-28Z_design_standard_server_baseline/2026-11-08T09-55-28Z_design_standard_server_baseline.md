# The standard server baseline — design decision record

Issue #5894, a design child of #5685 (DoS suite & performance
benchmark programme). Programme: epic #2056. This record settles only
what #5894 owns: the reference environment, the baseline workload
suite, the scales, and baseline versioning that every measurement in
the #5685 programme cites. It does not re-decide the parent's other
parts: the shared workload generator is #5891, the scenario catalogue
#5892, CI tiering #5893, scaling comparisons #5895, the per-interface
matrix #5896, and regression-gate budgets/waivers #5897. Where a
choice here constrains those tickets, the constraint is stated as an
invariant for them to consume, not as their design.

Precedent this builds on, verified against `main` at triage
(2026-11-05, task 10296, re-verified by task 12378 on 2026-11-08):
`pkgs/id/benches/world_bench.rs`, `pkgs/id/benches/BASELINES.md`, and
`pkgs/id/benches/baseline.txt` already define an in-process F11.8
world-benchmark baseline on a fleet-laptop machine class, with
medians, a >20% non-blocking CI regression report, and a worked
re-baselining procedure. That baseline is narrower than this one —
it has no network server, no per-interface surface, and no
identity/capability/journal/world-sync/blob workload set — so it does
not satisfy #5894. Its lessons are adopted wholesale: numbers are
only comparable on the same machine class; a unit change makes a
benchmark incomparable rather than silently comparable (#1619); a
flag on a different machine class is a prompt to investigate on the
reference machine, not proof of a regression.

## Decision summary

1. **Reference environment:** one pinned physical machine class —
   the fleet reference laptop class already used by `BASELINES.md` —
   is the canonical environment for stored baselines. A named CI
   runner class is a secondary, relative-only environment: its
   results may be compared against its own stored series, never
   mixed into the canonical series.
2. **Server under test:** one server process, release build, one
   database with default settings, default configuration except the
   fixture and limit settings the workload definition names. Any
   deviation is a different configuration, measured and labelled as
   one (that is what #5895's scaling comparisons are).
3. **Workload suite:** the five honest workloads the parent names —
   identity operations, capability checks, journal append/read,
   world sync, blob transfer — each with a fixed operation mix, a
   fixed dataset shape per scale, and a fixed metric set.
4. **Scales:** three named scales — S0 single user, S1 small
   community, S2 large world — with the concrete parameters in the
   table below. A result that does not name its scale is not a
   baseline result.
5. **Versioning:** the baseline is a versioned artefact
   (`baseline vMAJOR.MINOR`, below). Changing the machine class, a
   workload definition, a scale parameter, the server default
   configuration, or a measurement unit is a version event: announced,
   re-measured, and recorded here and in the stored baseline header —
   never a silent drift, exactly as the parent requires.

## 1. Reference environment

### Canonical: the fleet reference machine class

The class is the one `pkgs/id/benches/BASELINES.md` already records,
restated so this record stands alone:

- AMD Ryzen 7 7840U (16 threads), 123 GB RAM;
- Linux / NixOS, kernel and toolchain recorded per baseline run in
  the artefact header (the F11.8 baselines used rustc 1.98.1 /
  cargo 1.98.0, release/`bench` profile);
- local storage of the machine itself — fsync-bound numbers measure
  this class's storage as much as the code, which is precisely why
  the class is pinned instead of abstracted away.

Rules for the canonical environment:

- **Pinned build inputs.** Every canonical run records: commit SHA,
  flake lock state, toolchain versions, server build profile, and
  the workload/scale/configuration identifiers from §3–§4. A run
  missing any of these is an anecdote, not an artefact.
- **Warm-up.** Each workload runs its warm-up phase (the same
  operation mix at the same scale, results discarded) before any
  measured rep; the F11.8 harness's 5-rep median convention carries
  over: a stored number is a median of at least 5 measured reps,
  with the minimum also recorded.
- **Co-tenancy honesty.** The reference machine is a working fleet
  machine, not a sealed lab. Runs record load average at start; the
  F11.8 precedent showed medians of fsync- and in-process-dominated
  work agreeing within ~5% across two full runs under shared load.
  A canonical run whose reps disagree with each other by more than
  the noise band #5897 sets is discarded and rerun, not averaged
  into compliance.
- **One server, one database, defaults.** The canonical server is a
  single process serving all interfaces it exposes, backed by one
  database instance with the product's default settings. Fixtures
  (identities, worlds, journals, blobs) are generated by the shared
  generator (#5891) from a recorded seed, so any worker can rebuild
  the identical starting state.

### Secondary: the CI runner class

CI runners are a different machine class and their absolute numbers
are never compared with canonical numbers (the F11.8 precedent's
core warning). The runner class exists for per-change relative
checks owned by #5893/#5897. Its stored series is labelled with its
own machine-class tag, and a cross-class comparison in any report is
a defect in that report.

### Alternatives rejected

- **Normalised scoring model** (divide by a calibration workload and
  compare scores across machines): rejected. The precedent shows the
  decisive quantities — fsync latency, storage behaviour — are
  properties of a machine class, not a scalar a calibration can
  divide out; normalisation would manufacture comparability the
  hardware does not support.
- **Fixed cloud instance class as canonical:** rejected for v1. No
  contributor can reproduce it locally, its co-tenancy variance is
  outside our control, and it adds a standing cost and credential
  surface the programme does not otherwise need. The parent leaves
  dedicated hardware as an open question for scenarios that cannot
  be deterministic on shared runners; if #5892 identifies such
  scenarios, that is a version event for this record, not a reason
  to move the whole baseline now.
- **Container with enforced CPU/memory limits as canonical:**
  rejected as the *sole* definition. Limits bound CPU and memory but
  not storage latency or memory bandwidth, the two resources several
  workloads actually measure. Containers remain welcome for fixture
  isolation inside a run; they do not define the environment.

## 2. The server build and configuration (the "standard server")

- Build: release profile from a pinned commit, built by the repo's
  normal flake path; the exact build command is recorded in the
  artefact.
- Configuration: product defaults, with only these named exceptions,
  each recorded per run: fixture data directory, listen interfaces
  enabled for the per-interface runs (#5896), and any safety limits
  the workload definition itself sets (for example the chat rate
  burst the F11.8 harness already had to respect).
- One database: the default storage configuration, freshly created
  from the recorded seed for each canonical run; no carried-over
  state between runs.

## 3. The baseline workload suite

Each workload is a configuration of the shared generator (#5891) —
this record fixes the operation mix and dataset shape; the
generator's mechanics are #5891's to design. Every workload reports
the same metric set: operations/second, latency p50/p95/p99,
CPU-seconds per 1,000 operations, and peak resident memory, each as
median-of-≥5 with minimum, per scale, per interface where the
workload touches the network (#5896 runs the matrix; this record
defines what a cell measures).

| Workload | Operation mix (per measured rep) | What it represents |
|---|---|---|
| W1 identity operations | Create identity; resolve/lookup by petname and by key; sign and verify a message; rotate a key | Joining and being found |
| W2 capability checks | Grant a capability; check an allowed action; check a denied action; check through one delegation hop | The authorisation hot path every request pays |
| W3 journal append/read | Append events (the F11.8 sizes: 2,000-entry journals); read a full journal; read a tail slice; checkpoint/compaction | The event-store core, continuous with the F11.8 journal baselines |
| W4 world sync | A peer joins a world; catch-up replay of 10,000 committed events (the F11.8 replay size); live fan-out to the scale's client count | Multi-client world behaviour, continuous with F11.8 replay/fan-out |
| W5 blob transfer | Store and fetch blobs at 4 KiB, 1 MiB, and 64 MiB; fetch a blob the peer must retrieve from another peer | Attachments and content movement |

Dataset shapes are fixed per scale (§4) and generated from the
recorded seed. Changing a mix, a size, or a shape is a workload
version change (§5).

## 4. Scales

| Scale | Identities | Worlds | Membership | Journal history per world | Connected clients during W4 | Blobs pre-seeded |
|---|---|---|---|---|---|---|
| S0 single user | 1 | 1 | 1 member | 2,000 events | 1 | 10 |
| S1 small community | 50 | 5 | up to 50 per world | 10,000 events | 32 | 500 |
| S2 large world | 5,000 | 20 | up to 1,000 in the largest world | 100,000 events in the largest world | 256 | 10,000 |

The S0/S1 sizes deliberately reuse the F11.8 full-size workloads
(2,000-entry journals, 10,000-event replay, 32 fan-out clients) so
the in-process and server numbers stay cross-readable. S2 is the
stress end of the honest range — still a workload a community server
could plausibly see, not an adversarial profile; adversarial load is
Part 1 of the parent and #5892's catalogue, run against this same
baseline so "attack worked" and "server was already slow" stay
distinguishable, which is the parent's stated reason for one
programme.

## 5. Baseline versioning

- **Identifier.** `id-server-baseline vMAJOR.MINOR`, cited in full —
  with machine class, commit, and date — wherever a number is
  quoted: e.g. "W3 p95, S1, id-server-baseline v1.0, fleet-laptop
  class, <commit>, 2026-11-08".
- **MAJOR increments** (old and new series are incomparable; reports
  must not plot them on one axis): machine-class change; workload
  mix or dataset-shape change; scale-parameter change; measurement
  unit change (the #1619 rule: incomparable, no percentage, no
  verdict, until re-baselined).
- **MINOR increments** (same series, re-measured): a deliberate,
  reviewed change to server defaults or build profile; a harness
  change that does not alter what is measured.
- **The re-baselining procedure** is the F11.8 one, generalised:
  rerun the full suite at all scales on the canonical machine,
  replace the stored baseline, update the header (version, machine
  class, commit, toolchain, date), and state the reason — hardware
  change, deliberate performance change, or harness change — in the
  commit and on #5894's successor record. `--quick`-style reduced
  runs are never committed as baselines (the F11.8 rule, kept).
- **Storage.** Stored baselines live with the harness that produces
  them, following the `pkgs/id/benches/baseline.txt` precedent: a
  small committed text artefact with a header, diffed by tooling,
  with full machine-readable run artefacts (environment, build,
  config, workload, scale, raw reps) retained per the retention
  decision #5897 owns. Git history is the archive of superseded
  baseline versions; nothing is deleted when a version is superseded.

## 6. Invariants implementation must hold

1. No number enters a report, gate, or comparison without its full
   citation: baseline version, machine class, workload, scale,
   configuration, commit.
2. Cross-machine-class comparisons are never presented as
   regressions or improvements.
3. A unit change renders a metric incomparable until re-baselined —
   tooling reports "incomparable", never a percentage across units.
4. Fixtures for a canonical run are reproducible from the recorded
   seed alone.
5. The baseline suite runs the product's default configuration; a
   tuned configuration is a labelled comparison (#5895), never a
   quiet baseline change.
6. Superseded baselines remain readable in git history with their
   headers intact.

## 7. What this record does not decide

- Gate budgets, noise bands, and the waiver/re-baselining *approval*
  procedure: #5897 (this record defines what a version event *is*;
  who may approve one is #5897's ruling, and the parent's open
  question about repeated waivers stays with it).
- CI tier placement and time budgets for these workloads: #5893.
- Whether any Part 1 scenario needs dedicated hardware: #5892
  identifies candidates; adopting one is a MAJOR version event here.
- Artefact retention period and the programme's CI-minute budget:
  the parent's open questions, owned by #5897's results-store design.
- Which single workload is most representative of real usage, and
  drift detection against real usage: left visible as the parent's
  open question; the suite above covers all five named workloads
  rather than betting the baseline on one.

## Relations

- Parent: #5685 (Parts and open questions cited above). Programme:
  epic #2056. Siblings consumed in order: #5891 (generator) builds
  these workloads; #5892 and #5893 consume this baseline; #5895,
  #5896, #5897 compare and gate against it.
- Precedent: `pkgs/id/benches/BASELINES.md` and
  `pkgs/id/benches/baseline.txt` (F11.8, issues #539/#1512/#1513,
  unit-incomparability rule #1619, guest-replay correction #1403).
