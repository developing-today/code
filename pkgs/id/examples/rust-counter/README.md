# Rust counter world

A tiny pure Rust world program for `id`'s multiplayer service — the
F12.8 (#1065) twin of `../roc-counter`, built on the Rust guest SDK in
`../rust-world`. The host owns the authoritative state; guests send
`inc`, and every participant sees the returned `count=N` presentation.
It demonstrates the SDK's screen helper (the same boxed 20x3 screen
JSON as the Roc program), the `records` export, and snapshot/restore.

## Build

Requires the `wasm32-unknown-unknown` Rust target:

```sh
rustup target add wasm32-unknown-unknown
./build.sh        # cargo build --release --target wasm32-unknown-unknown
```

`build.sh` refreshes the checked-in `counter.wasm`, which the host's
F12.7 conformance suite (`pkgs/id/src/sandbox.rs`) loads as the
`rust-counter` fixture — no Rust toolchain is needed to run the suite.

## Run a hosted world

```sh
id serve --world --world-admin-token "$WORLD_ADMIN" --world-module counter.wasm
```

The module has no Wasm imports and exports the full Plaza ABI v1
surface (see `../roc-world/ABI.md`). The remaining F12.8 scope — the
second SDK (AssemblyScript vs Zig) and the component-model/WIT
direction — stays open on #1065 as Drew-owned decisions; this example
covers the Rust SDK slice only.
