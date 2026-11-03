# `pkgs/id` crate split: Phase 1 architecture

**Status:** Phase 1 design record for [issue #3447](https://github.com/developing-today/code/issues/3447) and [PR #3471](https://github.com/developing-today/code/pull/3471).

This record captures the design implemented by the Phase 1 extraction. It is being added during review remediation after a review found that the in-tree architecture record was missing.

## Goal and constraints

Split the large `id` compilation unit along existing module-dependency seams without changing behavior, the public `id` crate paths, CLI output, or wire formats. Phase 1 establishes a foundation crate for low-dependency leaf modules; later phases remain separate changes. Tests must not be deleted or weakened, and the `pkgs/id` Nix build must continue to work from outside the package directory.

## Workspace and crate boundary

Keep `pkgs/id/Cargo.toml` as both the existing `id` package manifest and the Cargo workspace root. Add `crates/id-core` as a workspace member. `id-core` owns only the Phase 1 leaves: `time`, `waiting`, `connectivity`, `discovery`, `tuple`, `petname`, `qr`, `access`, `sync_method`, `membership/`, `world_caps`, `world_limits`, and `world_presentation` (including each module's local resources and tests).

The root `id` package retains the binary, CLI, and all modules outside that list. Modules are moved, not copied. A leaf-owned shared type moves with its defining module; for types whose construction depends on root-only CLI arguments, keep that construction in `id` and expose the narrow interface needed by `id-core` (for example, `WorldLimitArgs`).

## Compatibility and feature forwarding

The root crate re-exports the extracted modules and their public items at their historical `crate::` paths. Existing internal and external callers therefore do not need path changes. The root package depends on `id-core` with its defaults disabled and explicitly forwards the `world`, `sandbox`, and `web` feature selections that affect extracted code. `id-core` keeps its own dependency and lint declarations; optional dependencies remain optional and are enabled by the corresponding forwarded feature.

The root package's dev-dependency enables `id-core`'s `test-utils` feature so root unit tests can use the extracted connectivity dial helpers. That feature is not enabled by normal dependencies or shipped builds.

Shared types follow their defining leaf modules: `Refusal` is defined at the `id-core` root and re-exported by the existing root modules that used it; `SandboxLimits`, `NativeLimits`, and `MAX_WORLD_MODULE_BYTES` are defined in `id_core::world_limits` and re-exported at their historical paths. `RuntimeLimits::from_args` is defined in `id_core::world_limits` and takes the `WorldLimitArgs` trait, which the root CLI implements for its argument type. This keeps CLI parsing out of the foundation crate while retaining the existing call paths.

## Build and test selection

The workspace's default members include both `id` and `id-core`, so existing Cargo commands launched from `pkgs/id` continue to compile and test the extracted crate. Standard unit-test and Nix check entry points must cover both packages. The `scripts/build.sh` freshness check must consider workspace crate sources and manifests, not only the root `src/` tree, so an edit confined to `crates/id-core` cannot reuse a stale binary.

The SQLite/LogSync guard distinguishes active shipped dependencies from optional dependencies that Cargo records in the lockfile even when they are not enabled. It must continue to reject a shipped SQLite path and retain checks for the root manifest and vendored `p2panda-sync` configuration.

## Validation

For this phase, verify formatting, the all-features workspace test suite with the unchanged test count, the shipped binary and unchanged `id --help` output, and the existing Nix build. Record before/after cold `cargo check` (with an empty `CARGO_TARGET_DIR`) and cold `nix build .#id-lib` wall times on the laptop, following issue #3195's method. If a measurement cannot finish in a worker shift, preserve its result and resume from the recorded artifact rather than treating an incomplete run as a pass.

## Follow-on boundary

Phase 1 does not extract the world, sync, groups, web, or binary-facing remainder. The next planned phase is the separately queued `id-world` extraction; it may depend on this crate's stable leaf boundary but is not implemented here.
