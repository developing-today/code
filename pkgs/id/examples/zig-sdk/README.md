# Zig guest SDK

The third guest language for `id` worlds (#12596, Phase 12 exit
criterion 3, results-first plan step A7): a Zig SDK with `screen`,
`json`, `grid`, and `cap` helpers equal to the Roc libraries in
`../roc-world/`, passing the F12.7 conformance suite, with a playable
example world.

Slice 1 landed the SDK skeleton plus the two pure helpers everything
else builds on (`json`, `grid`); slice 2 adds `screen` and `cap`, so
all four helpers are in. Slice 3 adds the playable example world,
[`../zig-counter/`](../zig-counter/) — the counter twin of
`../roc-counter/` and `../rust-counter/` — leaving the conformance
run as the follow-up slice on #12596.

## Layout

| File               | What it is                                                                                                            |
| ------------------ | --------------------------------------------------------------------------------------------------------------------- |
| `src/json.zig`     | JSON value type: decode/encode, field access, type checks. Zig port of `RocJson`.                                     |
| `src/grid.zig`     | Bounded row-major grids. Zig port of `RocGrid`.                                                                       |
| `src/screen.zig`   | Cell-grid screen: text/boxes/grids, plain + JSON + ANSI renders. Zig port of `Screen`.                                |
| `src/cap.zig`      | Capability-protocol bookkeeping: request ids, pending list, request documents. Zig port of `RocCap`.                  |
| `src/root.zig`     | SDK root: re-exports `json`, `grid`, `screen`, and `cap`.                                                             |
| `build.zig`        | Builds the SDK as a static library and runs `zig build test`.                                                         |
| `build.zig.zon`    | Package manifest; minimum Zig 0.16.0, matching the tree's Zig pin.                                                    |
| `zig-toolchain.sh` | Version guard: refuses to build unless `zig version` matches `../../nix/zig-version` (mirrors `../roc-toolchain.sh`). |
| `zig-sdk-test.sh`  | Runs the guard, `zig fmt --check`, `zig build test`, and a `wasm32-freestanding` build.                               |

## Parity with the Roc libraries

| Roc (`../roc-world/`)                                                                                                 | Zig (here)                                                                                                                                                                                                                     | Notes                                                                                                                                                                                                                                                                                                            |
| --------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `RocJson.decode` / `encode`                                                                                           | `json.decode` / `json.encode`                                                                                                                                                                                                  | Same limits: 64 KiB in/out, depth 64. Numbers keep their exact lexeme. Same error codes via `json.errorCode` (InvalidJson 0 … WrongType 6).                                                                                                                                                                      |
| `RocJson.get`, `as_string`, `as_number`, `as_bool`, `as_array`, `as_object`                                           | `json.get`, `asString`, `asNumber`, `asBool`, `asArray`, `asObject`                                                                                                                                                            | Same `MissingField` / `WrongType` failures.                                                                                                                                                                                                                                                                      |
| `RocGrid.new/get/set/count/neighbors4/row/bytes/dimensions/from_cells`                                                | `grid.Grid.init/get/set/count/neighbors4/row/bytes/dimensions/fromCells`                                                                                                                                                       | Same bounds behaviour: out-of-grid reads return the fallback, out-of-grid writes are no-ops, never traps.                                                                                                                                                                                                        |
| `Screen.new/put_text/put_row/put_grid/fill/rule/boxed/render_plain/render_json/render_rows_json/render_ansi/box_edge` | `screen.Screen.init/putText/putRow/putGrid/fill/rule/boxed/renderPlain/renderJson/renderRowsJson/renderAnsi/boxEdge`                                                                                                           | Same wire behaviour: the JSON screen document is byte-for-byte the Roc one (run-length style spans, default runs omitted, rows padded to width); a column is one Unicode scalar value. Drawing mutates in place, like `Grid.set`. A cell's `ch` is a borrowed slice, so a screen does not own the text it shows. |
| `RocCap.empty/issue/attach/requests/counter/from_pending/answered/done/is_empty` + the `*_request` builders           | `cap.Session.empty/issue/attach/requests/counter/fromPending/answered/done/isEmpty` + `chatRequest`/`nowRequest`/`playersRequest`/`worldRequest`/`historyRequest`/`recordsRequest`/`scheduleAtRequest`/`scheduleCancelRequest` | Same documents on the wire (same field order, ids are `r<counter>`). The session owns its pending entries: ids/caps are copies, and documents passed to `attach`/`fromPending` must be allocator-owned — the session frees them on `done`/`deinit`.                                                              |

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
suites in `src/` (mirrors of the Roc `expect` cases, including the
64-depth and 64 KiB limit cases in `json`), a `wasm32-freestanding`
build proving the SDK compiles for the guest target, and the
`../zig-counter/` example's host tests plus guest-module build. No
Roc toolchain, Rust toolchain, or laptop is needed.

## What remains on #12596

1. A conformance run of the `../zig-counter/` example against the
   F12.7 suite: add `zig-counter` (its checked-in `counter.wasm`) to
   the module list in `pkgs/id/crates/id-worlds/src/sandbox.rs` and
   record the passing run where the Phase 12 exit audit (#12231) can
   cite it.
2. Wiring `zig-sdk-test.sh` into the `roc-test` just recipe once the
   nix-provided Zig version is confirmed to be 0.16.0 (the nix
   `zig` package version was not verified in this slice, so the
   wiring is deliberately left out rather than risk breaking
   `just ci`).
