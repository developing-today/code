# Zig guest SDK

The third guest language for `id` worlds (#12596, Phase 12 exit
criterion 3, results-first plan step A7): a Zig SDK with `screen`,
`json`, `grid`, and `cap` helpers equal to the Roc libraries in
`../roc-world/`, passing the F12.7 conformance suite, with a playable
example world.

This slice lands the SDK skeleton plus the two pure helpers everything
else builds on. `screen`, `cap`, the playable example, and the
conformance run are follow-up slices on #12596.

## Layout

| File | What it is |
| --- | --- |
| `src/json.zig` | JSON value type: decode/encode, field access, type checks. Zig port of `RocJson`. |
| `src/grid.zig` | Bounded row-major grids. Zig port of `RocGrid`. |
| `src/root.zig` | SDK root: re-exports `json` and `grid`. |
| `build.zig` | Builds the SDK as a static library and runs `zig build test`. |
| `build.zig.zon` | Package manifest; minimum Zig 0.16.0, matching the tree's Zig pin. |
| `zig-toolchain.sh` | Version guard: refuses to build unless `zig version` matches `../../nix/zig-version` (mirrors `../roc-toolchain.sh`). |
| `zig-sdk-test.sh` | Runs the guard, `zig fmt --check`, `zig build test`, and a `wasm32-freestanding` build. |

## Parity with the Roc libraries

| Roc (`../roc-world/`) | Zig (here) | Notes |
| --- | --- | --- |
| `RocJson.decode` / `encode` | `json.decode` / `json.encode` | Same limits: 64 KiB in/out, depth 64. Numbers keep their exact lexeme. Same error codes via `json.errorCode` (InvalidJson 0 … WrongType 6). |
| `RocJson.get`, `as_string`, `as_number`, `as_bool`, `as_array`, `as_object` | `json.get`, `asString`, `asNumber`, `asBool`, `asArray`, `asObject` | Same `MissingField` / `WrongType` failures. |
| `RocGrid.new/get/set/count/neighbors4/row/bytes/dimensions/from_cells` | `grid.Grid.init/get/set/count/neighbors4/row/bytes/dimensions/fromCells` | Same bounds behaviour: out-of-grid reads return the fallback, out-of-grid writes are no-ops, never traps. |

Two deliberate differences, both documented at the use site:

- **Memory is explicit.** Zig has no GC: `json.decode` and
  `grid.Grid.init` allocate from a caller-provided allocator (an arena
  is the natural fit inside a wasm32-freestanding guest) and the
  caller frees with `Value.deinit` / `Grid.deinit`.
- **`Grid.set` mutates in place** (`*Grid`) instead of returning a new
  grid, which is the Zig idiom and avoids a copy per cell write inside
  a guest. Reads still take the grid by value and never mutate it.

## Verify

```sh
ZIG=/path/to/zig ./zig-sdk-test.sh   # or put Zig 0.16.0 on PATH first
```

This runs the version guard, `zig fmt --check`, the `zig test`
suites in `src/json.zig` and `src/grid.zig` (mirrors of the Roc
`expect` cases, including the 64-depth and 64 KiB limit cases), and a
`wasm32-freestanding` build proving the SDK compiles for the guest
target. No Roc toolchain, Rust toolchain, or laptop is needed.

## What remains on #12596

1. `screen` helper (Zig port of `Screen`, including `render_json`).
2. `cap` helper (Zig port of `RocCap`: request ids, pending lists,
   `wants` documents).
3. A playable example world (e.g. `zig-counter`, mirroring
   `../roc-counter/`) exporting the Plaza ABI v1 surface from
   `../roc-world/ABI.md`.
4. A conformance run of that example against the F12.7 suite, and
   wiring `zig-sdk-test.sh` into the `roc-test` just recipe once the
   nix-provided Zig version is confirmed to be 0.16.0 (the nix
   `zig` package version was not verified in this slice, so the
   wiring is deliberately left out rather than risk breaking
   `just ci`).
