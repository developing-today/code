# Roc tic-tac-toe world

A two-player tic-tac-toe world program for `id`, sharing the platform in
`../roc-world`. Players send a cell digit (`0`-`8`); the ply parity decides
whether the move is X or O, so `update` is a pure function of the event and
the journal replays it exactly. Occupied cells, off-board cells, non-digits,
and moves after a win are ignored.

Records are published as JSON: `{"board": "XXXOO----", "plays": 5,
"winner": "X"}`, so `id world records` and `id world mirror` show the board
and can replicate it peer-to-peer.

## Build

```sh
ROC=/path/to/roc ZIG=zig ./build.sh
```

## Run

```sh
id serve --world --world-admin-token "$WORLD_ADMIN" --world-module tic-tac-toe.wasm
id world invite HOST_NODE --admin-token "$WORLD_ADMIN" --name Ada
id world join HOST_NODE --capability TOKEN       # then: /input 34  (plays cell 4)
```
