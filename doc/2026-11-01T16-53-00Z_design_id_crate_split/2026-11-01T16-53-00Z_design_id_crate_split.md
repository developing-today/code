# pkgs/id crate split: a workspace of importable crates

Design record, 2026-11-01. The crate-split design for #3195, against the
tree on origin/main `d00c6d10` (i.e. after #3160 merged). Facts are from a
full import scan of `pkgs/id/src` (150 `.rs` files, 129,292 lines),
`pkgs/id/Cargo.toml`, `pkgs/id/flake.nix`, and `pkgs/id/default.nix`;
measurements quoted from #3195 are the 2026-10-31 laptop numbers. Drew's
priority order heads everything below and is not traded away anywhere in
this design: **(1) optimal runtime performance of the shipped binary,
(2) build time.**

Method note: import edges were counted from `crate::<module>` references,
aggregated per file (a file either references a top-level module or not),
test modules included. That is exact enough for layering — it shows who
may name whom — and the counts below are file counts, not call counts.

## The current shape

One package, one lib crate, one bin. Every one of the ~70 top-level
modules in `src/lib.rs` is `pub`; `src/main.rs` consumes a flat
re-export surface (`Cli`, `Command`, the `cmd_*` functions, `run_repl`,
`open_store`, …). The features (`world`, `sandbox`, `web`, `ssh`, `dht`)
gate modules inside the single crate. Two vendored path dependencies
(`vendor/p2panda-sync`, `vendor/p2panda-encryption`) already prove path
deps work under the Nix builds.

Where the lines are (top-level module, files, lines):

| module | files | lines | | module | files | lines |
|---|---|---|---|---|---|---|
| web | 13 | 19,363 | | world_store | 1 | 1,824 |
| commands | 16 | 10,716 | | log_sync | 1 | 1,733 |
| world | 8 | 9,096 | | protocol | 1 | 1,731 |
| event_store | 8 | 8,685 | | server_config | 3 | 1,648 |
| groups | 10 | 5,953 | | world_compile | 1 | 1,559 |
| world_session | 4 | 5,271 | | world_fixture | 1 | 1,463 |
| encryption | 8 | 4,901 | | world_hub | 1 | 1,257 |
| cli | 1 | 4,549 | | tuple | 1 | 1,245 |
| repl | 3 | 3,339 | | seitan | 7 | 1,220 |
| world_ssh | 1 | 3,225 | | store_traits | 1 | 1,168 |
| directory | 1 | 2,954 | | discovery | 1 | 1,122 |
| directory_view | 1 | 2,762 | | shell_contract | 2 | 1,108 |
| tags | 1 | 2,698 | | chain | 1 | 1,014 |
| sandbox | 1 | 2,290 | | world_native | 1 | 1,001 |
| membership | 5 | 2,059 | | world_caps | 1 | 974 |
| artifact | 1 | 1,987 | | world_presentation | 1 | 912 |
| world_net | 1 | 1,981 | | world_manifest | 1 | 876 |

Families: the world family (`world`, `world_session`, `world_ssh`,
`sandbox`, `world_net`, `world_store`, `world_compile`, `world_fixture`,
`world_hub`, `world_native`, `world_caps`, `world_presentation`,
`world_manifest`, `world_doctor`, `world_records`, `world_limits`,
`world_line`) is ≈33,600 lines — 26% of the crate. The event/store
family (`event_store`, `chain`, `store_traits`, `store`, `log_sync`,
`event_sync`, `local_events`, `waiting`, `tag_claims`, `tag_index`,
`documents`, `search`, `attestation`, `rejection`) is ≈16,600 lines.
Groups/membership/encryption ≈13,900. Directory/identity ≈10,300.

The hubs (inbound file references): `world` 31, `groups` 20,
`world_session` 19, `directory` 18, `connectivity` 15, `world_caps` 14,
`world_limits` 14, `waiting` 14, `world_hub` 14, `directory_auth` 13,
`tags` 12, `chain` 12. The layering the codebase *wants* is already
visible — a store/event core, a groups layer, a sync layer, the worlds
runtime, then directory/web/CLI on top. The split follows that layering;
it does not invent a new one.

## The unclean seams

Ten concrete defects in the current graph block a clean split. Each has
a named resolution; all of them are small, mechanical inversions or
relocations, done as part of the extraction step that crosses the seam
(never as a separate big-bang cleanup).

1. **`world_caps` ↔ `world`.** Five `world` files use the capability
   catalog; one caps file names `crate::world` back. The catalog must be
   a leaf. Resolution: the shared types caps needs move down into the
   caps crate (or the kernel); caps ends with zero upward references.
2. **`world_limits` reaches up.** Limits (406 lines of config data)
   reference `world_session`, `world_native`, `sandbox`, and `cli`.
   Resolution: the referenced defaults/types move down into the caps
   crate; consumers import limits from there, never the reverse.
3. **`store_traits` ↔ `log_sync` ↔ `event_store` triangle.**
   `store_traits` names both implementations; `log_sync` names
   `store_traits` and `event_store`; `event_store` names `log_sync`.
   Resolution: the traits crate holds traits only. `log_sync` (an
   implementation) depends on the traits, and `event_store`'s one
   `log_sync` reference is re-plumbed through the trait surface.
4. **`rejection` sits too high.** The 51-line veto vocabulary is named
   by `groups`, `event_store`, and `encryption`, and names all three
   back. Resolution: it drops to the kernel crate as pure vocabulary.
5. **`encryption` → `world_session`.** The DataScheme adapter (which
   belongs below sessions) names session types in two files.
   Resolution: the session-side types it needs move down, or the call
   inverts behind a trait owned by the encryption crate.
6. **`groups/worlds.rs` bridges upward.** The groups crate should not
   know worlds exist. Resolution: the bridge file moves to the worlds
   crate — it is the consumer side of the relationship.
7. **`helpers` and `meta_client` name the CLI layer.** `helpers` names
   `commands`/`protocol`; `meta_client` names `commands`. Resolution:
   helpers is split — pure helpers to the kernel, command-shaped
   helpers to the CLI crate; `meta_client`'s command reference is
   re-plumbed through the protocol types it already uses.
8. **`frames` names its consumers.** Wire framing (226 lines)
   references `log_sync`, `envelope_net`, `world_net`, `world_ssh`.
   Resolution: framing drops to the kernel as a leaf; the frame types
   consumers currently define for it move down with it.
9. **`artifact` is an integration module.** It names the directory,
   world, session, envelope, and event-sync modules — it is blob
   transfer welded to its callers. Resolution: it lands in the sync
   crate, and its world/session references invert behind a small
   availability trait owned by the sync crate (sessions publish;
   transfer consumes).
10. **Directory reaches into worlds.** `directory` and
    `directory_auth`/`directory_log` each name `world` once, and
    `directory_view` renders world state. Resolution: the naming/
    identity core must not depend on the worlds crate; the
    world-presenting parts move up to the web/app layer at extraction.

Two more observations the migration must respect: `event_store`'s
ingest names `membership`/`groups` (membership checks at ingest), and
`server_config` names `groups`, `seitan`, `directory`, `world` —
server config is composition, so it belongs to the top (CLI/app) crate,
not mid-graph. The ingest dependency is handled by a membership trait
defined in the events crate and implemented by the groups crate.

## The crate map

`pkgs/id` becomes a Cargo workspace. The root package stays the shipped
`id` binary plus a facade lib; member crates live under
`pkgs/id/crates/`. Workspace-shared dependency versions are pinned once
in `[workspace.dependencies]` from step 1 on, so member crates cannot
drift onto different iroh/wasmtime versions.

| crate | contents (today's modules) | ≈lines | depends on | importable by |
|---|---|---|---|---|
| `id-kernel` | `time`, `rejection`, `frames`, `tuple`, pure parts of `helpers` | 2,100 | — | anyone; the stable vocabulary |
| `id-caps` | `world_caps`, `world_limits`, `world_manifest` | 2,300 | kernel | anyone; capability/limit data model |
| `id-store` | `store_traits`, `store`, `chain`, `waiting`, `documents`, `tag_claims`, `tag_index`, `tags`, `search` | 8,300 | kernel | tools that read/write the store (the bridge/hosted work already builds against these traits) |
| `id-events` | `event_store`, `local_events`, `event_sync`, `attestation` | 10,100 | kernel, store, caps | services hosting the event graph |
| `id-groups` | `groups`, `membership`, `access`, `blob_gate`, `pull_policy`, `encryption` | 13,900 | kernel, store, events | services enforcing group policy |
| `id-sync` | `log_sync`, `sync_method`, `protocol`, `meta_client`, `discovery`, `connectivity`, `dht`, `envelope_net`, `envelope_outbox`, `artifact`, `fileops`, `local` handlers' wire half | 9,600 | kernel, store, events, groups | embedders running the sync protocols |
| `id-worlds` | the world family incl. `sandbox`, `world_session`, `world_ssh`, `world_native`, `shell_contract` | 34,700 | kernel, caps, store, events, groups, sync | embedders running worlds; the runtime |
| `id-directory` | `directory`, `directory_view`, `directory_auth`, `directory_log`, `directory_mail`, `petname`, `invite`, `invites`, `seitan` | 10,300 | kernel, store, events, groups | apps presenting identity/naming |
| `id-web` | `web` | 19,400 | all below | application crate — surface is the server, not the API |
| `id-cli` | `cli`, `commands`, `repl`, `server_config` | 20,300 | all below | application crate — surface is the CLI |
| `id` (facade) | `lib.rs` re-exports, `main.rs`, feature definitions | 1,200 | all | the shipped product; the compatibility surface |

Dependency rules (enforced by review and, where cheap, by a
`cargo metadata` DAG check in the step PRs):

- Dependencies point one way, down the table. No crate names a crate
  above it; shared needs move down or invert behind a trait owned by
  the lower crate (the pattern resolutions 3, 5, and 9 already use).
- The facade re-exports the pre-split public paths (`id::world_caps`,
  `id::sandbox`, …) for the whole migration, so `main.rs`, the 40
  integration tests in `tests/`, and `benches/` do not churn per step.
  A step is only complete when the moved implementation exists *only*
  in its new crate and the old location holds nothing but the
  re-export — re-exports must not be allowed to mask a failed move.
- Feature mapping is preserved at the facade: `world` pulls the
  world-bearing crates, `sandbox` is the wasmtime dependency inside
  `id-worlds`, `web` pulls `id-web`, `ssh` is `russh` inside
  `id-worlds`. The Nix `buildFeatures` lists therefore do not change
  shape at any step.
- Stability expectations: kernel/caps/store are the "build on these"
  libraries (semver care, documented surface). events/groups/sync/
  worlds are importable but their surfaces may still move during the
  migration. web/cli are applications, not libraries.

## Runtime performance: why the split does not cost

Priority 1 is protected by construction, then verified per step.

- **The shipped artifact is still one binary** produced by the facade
  crate; nothing becomes a dylib or a separate process.
- **LTO makes crate boundaries cheap.** Cross-crate inlining under LTO
  operates on LLVM bitcode for the whole dependency graph — a function
  in `id-store` inlines into `id-worlds` exactly as an intra-crate
  function does, provided it is inline-eligible (small, or marked
  `#[inline]`, or generic). The split changes *dev/test* inlining
  (where LTO is off); it does not change release inlining. The one
  real risk is a hot function that is large, non-generic, and
  unmarked becoming a cross-crate call — the mitigation is a rule, not
  hope: hot-path helpers on crate boundaries get `#[inline]` or stay
  co-located (next point), and the bench canary below would catch a
  regression.
- **Hot paths stay inside one crate.** The guest-call path —
  `sandbox` ↔ world core ↔ `world_presentation`, including all fuel
  accounting — lives entirely inside `id-worlds`. The journal/store
  path (`event_store`, `chain`, `store`) keeps its current shape
  across `id-events`/`id-store`, which already communicate through
  async trait calls today; the split adds no new dynamic dispatch on
  any hot path.
- **Release profile audit** (current: `opt-level = 3`, `lto = "thin"`,
  `codegen-units = 1`, `strip = true`, panic unwind):
  - `codegen-units = 1` and LTO: keep. They are the correct settings
    for the shipped binary and most of why check builds are slow —
    which is a reason to separate the profiles, not to weaken them.
  - Thin vs fat LTO: genuinely open. With `codegen-units = 1` the gap
    narrows, but fat can still win on cross-module inlining. Step 0
    A/Bs them on the bench harness and the spot benchmark; adopt fat
    only on evidence, recorded on #3195.
  - Panic strategy: keep unwind. Error paths in the sandbox and the
    native worker rely on unwinding semantics, and switching is a
    behaviour change, not a build optimisation — out of scope here.
  - `target-cpu`: keep portable. The binary is built by Nix and run
    on more than one machine; `target-cpu = native` would make the
    artifact builder-dependent and non-reproducible.
  - PGO: deferred. It needs a representative workload profile and a
    Nix-friendly data flow; the bench harness built for step 0 is the
    evidence gate if it is ever proposed.
- **Determinism canary.** Fuel is a deterministic count, not a timing
  — the `FUEL_BUDGETS` table in `src/sandbox.rs` (measured baseline
  +25%, rounded up to the next 100,000; regenerate with the ignored
  `print_fuel_baseline` test) must come out byte-identical after every
  step, as must the bytes-per-frame baseline. Any drift means
  behaviour changed, and the step stops.

## Build time: what is bought, and in what order

From #3195: cold nix check build ≈16 min wall while 1,963 tests
executed in 12.4 s; a single-crate `rustc` invocation ran 26+ min. Two
independent purchases:

1. **Profile separation (step 0, before any code moves).** Both Nix
   derivations run the test suite through `buildRustPackage`, whose
   check phase compiles tests with the release codegen configuration
   (thin LTO, `codegen-units = 1`) — the slowest possible settings for
   code that is never shipped. A dedicated check/test profile
   (LTO off, codegen units back to the profile default, modest
   opt-level — fuel counts are opt-level-independent, so this cannot
   move the canary) is expected to be the largest single win, and it
   touches the shipped artifact not at all. The standalone
   `pkgs/id/default.nix` check derivation gets the same treatment.
2. **Crate granularity (the split itself).** After the split, an edit
   to `id-web` recompiles `id-web` and the facade — not the worlds
   runtime, the store, and all 40 integration-test harnesses. Cargo
   also compiles independent crates in parallel; today there is
   nothing to parallelise. No single crate remains the 26-minute long
   pole: the largest, `id-worlds` at ≈34,700 lines, is a quarter of
   today's unit, and the design notes it as the candidate for a later
   sub-split (core/session vs SSH/native adapters) if it becomes one.

Supporting hygiene, folded into step 0/1 rather than headlined: a
faster linker (mold/lld) for dev/test links only, evaluated on
measurements; the clippy `--all-targets --all-features` postCheck pass
keeps its gates but stops re-deriving the world per edit once the
crate cache is granular.

## Migration sequence

Every step: one PR, main green throughout, the shipped binary built by
the same Nix derivations at every point. A step that fails the
verification protocol does not merge.

- **Step 0 — profiles + baselines (no code motion).** Dedicated
  check/test profile in the workspace-to-be root manifest (today:
  `pkgs/id/Cargo.toml`) and in the Nix check invocations; thin-vs-fat
  LTO A/B; record on #3195: cold check-build before/after, the
  test-count baseline, `world_bench` medians and their noise band,
  the shipped-binary spot benchmark, and the fuel tables. This step
  also makes every later step's verify loop cheaper — it goes first
  for that reason as much as for its own win.
- **Step 1 — workspace skeleton, `id-kernel`, `id-caps`.** Convert
  `pkgs/id` to a workspace (facade package + members); pin shared
  versions in `[workspace.dependencies]`; extract the leaves and the
  caps family, cutting seams 1, 2, 4, and 8 on the way. Smallest and
  most-referenced first: everything later imports these.
- **Step 2 — `id-store`, then `id-events`.** Traits down (seam 3's
  trait half), then the event store with the membership-trait
  inversion for ingest.
- **Step 3 — `id-sync`.** `log_sync` and the wire layer move;
  `artifact` lands here with its availability trait (seam 9).
- **Step 4 — `id-groups`.** Groups/membership/encryption extract;
  seam 5 (encryption's session leak) is cut, and seam 6's bridge
  file leaves for its future home in `id-worlds`.
- **Step 5 — `id-worlds`.** The big one, done when the verify loop
  is fast and every crate below is stable. `shell_contract` lands
  here; directory's world references (seam 10) are resolved as the
  boundary is drawn.
- **Step 6 — `id-directory`, then `id-web`.** Web is a pure consumer
  and the safest large extraction; it goes late for that reason.
- **Step 7 — `id-cli`, facade slimming.** CLI/commands/REPL/server
  config extract; the facade becomes re-exports + `main.rs`. Decide
  which re-exports are the permanent public API and which were
  migration scaffolding; narrow integration tests to the crates they
  exercise where that is cheap.

Nix wiring per step: the workspace root keeps the single
`Cargo.lock`; `cargoLock.lockFile` in all three Nix entry points
(the sub-flake's `id-web` and `id-lib`, and `default.nix`) therefore
does not move, and the p2panda `outputHashes` are keyed by package
name/version and are unaffected by workspace membership (the two
vendored p2panda crates stay path dependencies, *not* workspace
members). Each step's PR verifies all three entry points build the
same `id` binary, and the root flake's NixOS module
(`pkgs/id/nix/id-module.nix`) is untouched throughout. One standing
check changes shape at step 1: the no-SQLite guard (#3160) reads the
resolved shipped graph — after the split it must cover every
workspace member's non-dev graph, not one package's.

## Verification protocol (every step, no exceptions)

1. **Fuel identity.** `print_fuel_baseline` output and the
   checked-in-examples fuel tests produce numbers identical to the
   step-0 record; bytes-per-frame baseline identical. Diff is
   committed in the step PR.
2. **Full suite green, count held.** The Nix `id-web` build (the
   gating derivation, check phase included) passes; the test count
   matches or exceeds the step-0 baseline, with any delta named.
   No test deleted, no check weakened, no budget re-baselined to
   buy speed — a re-baseline is a behaviour change and belongs to
   its own reviewed change, as #3160's was.
3. **Bench canary.** `benches/world_bench.rs` — `guest_replay`,
   `journal_append_fsync`, `checkpoint_compaction`,
   `session_fanout` — medians inside the step-0 noise band.
4. **Shipped-binary spot benchmark.** Scripted put/get/list against
   a scratch store plus a world-session smoke, timed, recorded on
   #3195 for the step.
5. **Build timings.** Cold `cargo test --no-run` in a clean target
   dir and the cold Nix check build, laptop, recorded on #3195 for
   the step. The trend across steps is the priority-2 evidence.

## Risks and non-goals

- **Re-export masking** is the main process risk; the per-step
  completeness grep (implementation exists only in the new crate)
  is the countermeasure, and it is in the definition of done above.
- **Feature-flag drift:** features gate modules inside one crate
  today; mapping them onto crate dependencies can change what the
  minimal `id-lib` build compiles. Every step verifies the `id-lib`
  (no-default-features) derivation, not just `id-web`.
- **Actively developed tree:** the crate is changing daily (150
  files and growing). The staged order — leaves first, pure
  consumers last — is chosen so in-flight feature work collides
  with extraction as little as possible; a step that collides is
  rebased, never rushed.
- Non-goals: no behaviour changes, no API redesign beyond the
  inversions named in the seams, no PGO, no panic-strategy change,
  no refactoring inside this design action itself. Follow-up
  implementation steps are filed as their own issues when the
  first step is ready to start, per the program pattern.
