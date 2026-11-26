# Colony

Colony is a Plaza world for `colony/two-deliveries` (#13605). A worker `@` hauls ore from a 4×2 board to the stockpile `S` at (0,1). The worker starts at (1,1), with ore `*` at (1,0) and (2,1). The pack holds one ore. Deliver two ore to the stockpile to meet the supply goal.

Each input names an action and carries an `event` id:

- `{"kind":"Mine","direction":"E","event":"m1"}` takes the ore in the adjacent square and leaves the worker in place (`extracted`). A full pack refuses (`pack-full`); no ore there is `empty`.
- `{"kind":"Walk","direction":"W","event":"w1"}` moves one square (`moved`). The board edge and ore squares are `blocked`.
- `{"kind":"Deposit","event":"d1"}` moves the carried ore to the stockpile (`stored`), only on the stockpile (`not-at-stockpile`) and only with ore in the pack (`empty`). The second delivery is `supply-goal-met` and the phase becomes `supplied`; after that every action is `already-supplied`.
- Directions are `N`, `E`, `S` and `W`. An input with a missing or invalid `event`, an unknown kind or a bad direction is `refused` and changes nothing.

The two ore are conserved across the board, the pack and the stockpile. Delivering the same `event` id again is `already-folded` and changes nothing, so a replayed delivery applies once.

The colony asks the host for no timer or other capability, and host events (participant 0) never drive it. `records` publishes the domain state in the vector's own keys plus the last `outcome`. `snapshot` adds a version and the bounded list of folded event ids, which is replay memory rather than domain state. Restoring a snapshot into a fresh instance gives byte-identical records, view and snapshot. A snapshot that describes an unreachable colony (ore under the worker, ore not conserved, a phase that does not match the stockpile) restores the starting layout. The view is presentation only: the grid, the pack and stockpile counts, a legend, then the phase and last outcome.

The pure domain (`Colony::step`) is kept apart from the Plaza entry points, so the oracle tests check it directly as well as through the guest.

## Build and verify

From this directory, on a development machine with the `wasm32-unknown-unknown` target installed:

```sh
cargo test --locked
cargo build --release --locked --target wasm32-unknown-unknown
node verify-two-deliveries.mjs target/wasm32-unknown-unknown/release/rust_colony.wasm
```

The regular ID host plays the same module in the `id-worlds` sandbox test:

```sh
cd ../..
COLONY_WASM="$PWD/examples/rust-colony/target/wasm32-unknown-unknown/release/rust_colony.wasm" \
  cargo test -p id-worlds --all-features --lib built_colony -- --ignored
```

## Run

```sh
id serve --world --world-admin-token "$WORLD_ADMIN" \
  --world-module target/wasm32-unknown-unknown/release/rust_colony.wasm
```

Join with an authorized `id world join` session and send an input such as `{"kind":"Mine","direction":"E","event":"m1"}`.
