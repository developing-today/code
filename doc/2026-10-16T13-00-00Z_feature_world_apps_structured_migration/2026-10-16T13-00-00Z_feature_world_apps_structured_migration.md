# Structured World Apps Migration to RocJson and Screen

See the [Phase 9 plan](../../.opencode/plans/phase-9-portable-platform.md), the [RocJson record](../2026-10-15T09-27-28Z_feature_roc_json/2026-10-15T09-27-28Z_feature_roc_json.md), and the [portable surfaces record](../2026-10-14T19-39-30Z_architecture_portable_world_surfaces/2026-10-14T19-39-30Z_architecture_portable_world_surfaces.md).

## Intent

Migrate interactive world example programs to the P9 standard platform libraries (`RocJson` for structured capability events and records; `Screen` for versioned terminal cell presentations):

1. Migrate `territory` to use pure `RocJson` for decoding presence events (`joined`, `left`, `participant.id`, `participant.name`) instead of fragile string slicing, and add player color styling.
2. Fix `lounge` presence participant name decoding and rebuild `lounge.wasm`.
3. Provide build scripts, checked-in WASM binaries, and README for `examples/roc-screen` demonstrating portable `Screen` cell grids and `Key` input.
4. Add a sandbox integration test `checked_in_roc_screen_renders_structured_surface` verifying structured screen parsing and cell grid content.

## Decisions

- **Typed Event Handling:** In `territory/main.roc`, replaced manual delimiter searches (`Str.contains`, `after`, `until`, `parse_pid`) with `RocJson.decode` and typed object/string/number accessors.
- **Checked-in WASM Parity:** Rebuilt all example WASM binaries with the import-free `roc` toolchain and validated them with `wasm-tools`.
- **Screen Integration Test:** Added `checked_in_roc_screen_renders_structured_surface` to `pkgs/id/src/sandbox.rs` to guarantee that checked-in screen programs produce `WorldSurface::Screen` contracts that parse and validate against presentation policy.

## Acceptance

- All 19 sandbox tests pass, including `checked_in_territory_plays_persists_and_rejoins`, `the_lounge_uses_the_capability_protocol`, and `checked_in_roc_screen_renders_structured_surface`.
- All checked-in WASM modules validate import-free.
- Formatting and git diff checks pass cleanly.
