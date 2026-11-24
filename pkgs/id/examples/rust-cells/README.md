# Cells

Cells is a deterministic, manually stepped 5×5 Conway world. Eight neighbors, B3/S23 and a dead exterior define each generation. The initial glider moves one cell east and south in four steps. Players can toggle a seed cell by typing `toggle X Y` while generation is zero, advance with `step`, and restore the original glider with `reset`. Coordinates start at (0,0) in the top left. Rendering, unknown input and reconnecting do not tick the clock.

The example uses the existing Rust Plaza guest SDK to exercise the same import-free Wasm entry points as the Roc examples. Its source is the distributed artifact. It builds with the Rust `wasm32-unknown-unknown` target and only a local SDK dependency. A Roc compiler, Zig host build or separate simulation runtime is unnecessary for this guest.

From this directory, on a development machine with that target installed:

```sh
cargo build --release --locked --target wasm32-unknown-unknown
node verify-glider.mjs target/wasm32-unknown-unknown/release/rust_cells.wasm
id serve --world --world-admin-token "$WORLD_ADMIN" --world-module target/wasm32-unknown-unknown/release/rust_cells.wasm
```

Use an authorized `id world join` session to send `/input 73746570` (`step`) or `/input 7265736574` (`reset`). For example, `/input 746f67676c6520342034` toggles (4,4) in the seed. The `records` export exposes `generation` and a row-major list of live coordinates. The independently specified Wasm oracle compares all four boards, checks the portable screen, and restores the generation-two snapshot into a fresh instance. The version-one snapshot contains the generation and 25 cell glyphs; malformed or unknown versions restore the initial glider. The guest requests no capabilities.
