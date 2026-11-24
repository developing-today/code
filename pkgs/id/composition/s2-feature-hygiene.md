# S2 feature hygiene record

Date: 2026-11-24. Slice of developing-today/code#14056, plan §7.4 / §10.

## Decisions and changes

- Pinned `resolver = "3"` in the workspace manifest.
- Removed `id-web/sandbox`, rather than forwarding it. Inspection of all
  eight `#[cfg(feature = "sandbox")]` sites in `id-web/src/world_ws.rs`
  showed they gate test-only helpers/tests. Production code has no sandbox
  cfg dependency. The web crate's `id-worlds` dev-dependency already enables
  `sandbox` and `test-utils`, so tests retain execution support.
- Kept member defaults as-is and documented their divergence: `id-caps`
  defaults to `sandbox`; `id-cli` to `sandbox,world`; `id-core` to
  `world,sandbox`. The root facade sets `default-features = false` on each
  member dependency and owns the effective build defaults.
- Added `composition/variants.toml` as the source for the shipped `web` and
  `lib` feature sets and root defaults. Nix reads the TOML, while just and
  `scripts/build.sh` use `scripts/variant-features.sh`. Validation check 8
  pins the root defaults, declared feature names, and these consumers.
- Updated `catalogue.toml` and this directory's README to record the resolved
  web gap. `default.nix` remains a legacy, unreferenced consumer and is not a
  shipped variant definition.

## Verification

- Variant helper outputs `web,sandbox,ssh,world`, `sandbox,world`, and
  `web,ssh` (web minus Cargo defaults); unknown variants/options fail.
- Nix evaluation of `id-lib` and the variant TOML expression succeeds.
- `just --fmt --check` and `just --dump` pass. `just just-recipes-check`
  passes after regenerating `just-recipes.json` (which also catches up
  main's stale entries: `test-sketch`, rust-cells/sketch example tests).
- The S2 done check verified 2026-11-24 on current `origin/main` plus this
  slice: the upstream `cargo check -p id-store` blocker (#14315, fix #14325)
  is resolved — Fleet Actions run 45440505886 (`cargo check -p id-store` on
  developing-today/code) succeeded, and `cargo check -p id-web --all-targets`
  passes locally. `cargo tree -e features -p id` for the Nix invocation
  (`--features web,sandbox,ssh,world`, defaults on) and the cargo invocation
  (`--features web,ssh`, defaults on) is byte-identical (6201 lines each);
  `nix eval` of `id-web`/`id-lib` `buildFeatures` returns the variant sets
  from the single source. `validate.py`, `build-sh-test.sh`, and
  `golden/recheck.sh` (canonical bytes and BLAKE3 hash) all pass.
