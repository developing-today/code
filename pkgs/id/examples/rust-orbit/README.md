# Orbit Arcade

Orbit Arcade is a manually ticked Plaza world for `orbit-arcade/edge-descent` (#13606). On a 4×4 board the cannon `A` sits on the bottom row at (2,3). One target `T` starts at (3,0), and the formation heads east. The wave starts at the end of tick 3. Steer the cannon by at most one lane and fire before the formation reaches the cannon row.

The level has one cannon seat. Each order is a steer/fire pair for the next logical tick:

- `{"kind":"QueueControls","id":"q4","for_tick":4,"steer":1,"fire":true}` queues an order (`queued`). `steer` is -1, 0 or 1.
- Sending the same order again, or `{"kind":"Redeliver","ref":"q4"}`, leaves exactly one order (`already-folded`), before or after its tick.
- A second distinct order for the same tick is `refused-occupied`. An order for any tick but the next one is `refused-tick`.
- `{"kind":"Tick","number":4}` advances the one logical clock to tick 4. A tick that already ran is `already-advanced`, and skipping ahead is `refused-gap`. Any `event` tag is host journal metadata that the guest does not read.
- `{"kind":"RenderFrames","count":120}` is `presentation-only`. Redraws and views never advance the simulation.
- `{"kind":"NewWave"}` starts the level layout again once a wave is over, keeping the tick and score.

A tick applies the order's steering (clamped at the board edges) and spawns a shot at the cannon for a requested fire. Then every shot moves north one square, and shots leaving the board are discarded. On every fourth tick the formation moves one column. If any target would leave the board, the whole formation descends one row and reverses instead. Collisions are resolved at the resulting positions in stable shot-ID order, and each defeated target scores 10 once. No targets left is `wave-clear`; a target on the cannon row is `overrun`. The outcome names what happened, joined in that order, such as `steer-fire-and-descend` or `hit-T-and-clear`.

The wave asks the host for no timer or other capability, so ticks are manual. Host events (participant 0) never drive it. `records` publishes the domain state in the vector's own keys plus the last `outcome`. `snapshot` adds a version and the bounded list of folded order ids, which is host deduplication memory rather than domain state. Restoring a snapshot into a fresh instance gives byte-identical records, view and snapshot. Malformed or unknown snapshots restore the level. The view is presentation only: a text grid (`A` cannon, `T` target, `|` shot, `.` empty), then the tick, score, heading, next order, phase and last outcome.

## Build and verify

From this directory, on a development machine with the `wasm32-unknown-unknown` target installed:

```sh
cargo test --locked
cargo build --release --locked --target wasm32-unknown-unknown
node verify-edge-descent.mjs target/wasm32-unknown-unknown/release/rust_orbit.wasm
```

The regular ID host plays the same module in the `id-worlds` sandbox test:

```sh
cd ../..
ORBIT_WASM="$PWD/examples/rust-orbit/target/wasm32-unknown-unknown/release/rust_orbit.wasm" \
  cargo test -p id-worlds --lib built_orbit -- --ignored
```

## Run

```sh
id serve --world --world-admin-token "$WORLD_ADMIN" \
  --world-module target/wasm32-unknown-unknown/release/rust_orbit.wasm
```

Join with an authorized `id world join` session and send an input such as `{"kind":"QueueControls","id":"q4","for_tick":4,"steer":1,"fire":true}`, then `{"kind":"Tick","number":4}`.
