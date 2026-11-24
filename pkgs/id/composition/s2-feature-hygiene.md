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
- `just --fmt --check` and `just --dump` pass.
- The S2 done check (Nix `id-web` and Cargo web produce identical feature
  graphs) is not yet verified: `cargo check -p id-store` fails on `origin/main`
  due to iroh-blobs 0.105 / iroh-docs 0.101 Hash type drift, tracked by #14315
  and its fix task #23352. This is an upstream workspace compile blocker, not
  a mismatch in the variant source.
