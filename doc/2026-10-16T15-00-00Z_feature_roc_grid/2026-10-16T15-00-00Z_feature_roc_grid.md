# RocGrid: a bounds-checked grid library for world programs

See the [Phase 9 plan](../../.opencode/plans/phase-9-portable-platform.md) and the [structured apps migration record](../2026-10-16T13-00-00Z_feature_world_apps_structured_migration/2026-10-16T13-00-00Z_feature_world_apps_structured_migration.md).

## Intent

Game and data worlds all keep a bounded grid: a board, a map, a canvas, an
inventory. Before this slice each app re-derived index arithmetic, bounds
checks, and neighbour walks in its own code. `RocGrid.roc` gives them one
tested library, exposed by the world platform as `pf.RocGrid`.

## Decisions

- **Byte cells, row-major.** A grid is `{ width : U64, height : U64, cells :
List(U8) }`. Bytes keep the representation small and interoperate with
  `Screen.roc`, which draws the same cells as text.
- **Never trap.** `get` takes a caller fallback for out-of-bounds reads and
  `set` returns the grid unchanged for out-of-bounds writes, so a game can
  clip a cursor or a move without branching on every coordinate.
- **Neighbours in a stable order.** `neighbors4` walks up, down, left, right
  and omits off-grid neighbours. `touches`-style adjacency rules (territory's
  claim rule is one) read directly from it.
- **A namespace module.** `RocGrid` is an opaque record type with its
  functions in one namespace, matching `Screen.roc`'s shape. One trap cost
  this slice: a module file named `RocGrid.roc` may not contain a type alias
  of the same name, and the platform's `exposes` list only resolves names the
  module also imports.

## Acceptance

- `pkgs/id/examples/roc-grid` is the checked-in example. It builds an
  import-free Wasm module and shows count, in-bounds get, and out-of-bounds
  fallback in its view.
- `checked_in_grid_example_uses_bounded_get_set` compiles the checked-in
  module in the sandbox, asserts `cells=12 cursor=46 outside=63`, plays one
  `set`, and asserts `cells=11 cursor=65 outside=63`.
- The platform still builds every example; all 21 sandbox tests pass.

## Follow-ups

- `Screen.roc` and `RocGrid.roc` still duplicate index arithmetic; a later
  slice can draw a `RocGrid` directly.
- A text renderer for grids (`RocGrid.render`) was left out because the
  available string helpers differ across Roc nightlies; add it once the
  pinned nightly can build it.
