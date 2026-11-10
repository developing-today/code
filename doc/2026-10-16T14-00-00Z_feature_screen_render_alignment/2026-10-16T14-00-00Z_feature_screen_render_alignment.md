# Portable Screen Rendering for Territory and a Host Allocator Alignment Fix

See the [Phase 9 plan](../../.opencode/plans/phase-9-portable-platform.md), the [portable surfaces record](../2026-10-14T19-39-30Z_architecture_portable_world_surfaces/2026-10-14T19-39-30Z_architecture_portable_world_surfaces.md), and the [structured apps migration record](../2026-10-16T13-00-00Z_feature_world_apps_structured_migration/2026-10-16T13-00-00Z_feature_world_apps_structured_migration.md).

## Intent

1. Fix a host allocator trap that made any Roc program requesting a non power-of-two allocation alignment die inside `std.mem.Alignment.fromByteUnits`.
2. Render `territory` as a portable, styled screen: per-seat colours, an inverted cursor, a boxed grid, and a scoreboard, using the same validated cell grid in SSH and the browser.

## Decisions

- **Alignment is clamped, not rejected.** Roc may request alignments that are not powers of two (the failing trace showed one). `DefaultAllocators` now rounds the requested alignment down to the nearest power of two with `std.math.floorPowerOfTwo`, and uses that same value as the metadata header size in `rocAlloc`, `rocDealloc`, and `rocRealloc`, so the three always agree on where the user pointer starts even when Roc's requested alignment differs between calls. The fix lives in `host/src/roc_platform_abi.zig`, shared by both the Wasm host and the native worker.
- **A batched row draw.** `Screen.put_row` replaces one row of cells from a starting column in a single list update. Row-oriented redraws no longer pay one whole-list copy per cell. `put_text` stays for borders, titles, and short lines.
- **A screen sized to fit the default fuel.** Territory renders a 24x19 screen: the 16x8 grid in an 18x10 box, a one-line control hint, and one line per each of the eight seats. At the sandbox's default 10,000,000 fuel it uses roughly 7,100,000, so the default budget continues to suffice and no server-wide limit was raised.
- **One structured view, no legacy fallback inside the program.** Territory always returns `Screen.render_json`; the host validates it into `WorldSurface::Screen`, and presenters render the same cells.

## Acceptance

- `checked_in_territory_renders_a_styled_screen_within_the_default_fuel` compiles the checked-in module with `SandboxLimits::default()`, plays one move, and asserts a 24x19 structured screen whose cursor cell is styled.
- `checked_in_territory_plays_persists_and_rejoins` asserts the rendered text carries `free=124`, both scoreboard lines, and no escape sequences, and that snapshots, restore, and rejoin still behave.
- All 20 sandbox tests pass, and the full library suite passes.
- `territory.wasm` is rebuilt from `main.roc` with `roc` and validated import-free with `wasm-tools`.

## Follow-ups

- Browser-side rendering of the cursor background is limited to the xterm-256 indices the contract already carries.
- Tile-grid helpers beyond `put_row` (fills, lines) remain planned for the data/game library slice.
