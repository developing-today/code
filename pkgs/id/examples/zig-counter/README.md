# Zig counter world

A tiny pure Zig world program for `id`'s multiplayer service — the
F12.8 (#12596) twin of `../roc-counter` and `../rust-counter`, built
on the Zig guest SDK in `../zig-sdk`. The host owns the authoritative
state; guests send `inc`, and every participant sees the returned
`count=N` presentation. It demonstrates the SDK's screen helper (the
same boxed 20x3 screen JSON as the Roc and Rust programs), the
`records` export encoded with the SDK's json helper, and
snapshot/restore.

## Build

Requires the pinned Zig 0.16.0 (enforced by
`../zig-sdk/zig-toolchain.sh`):

```sh
ZIG=/path/to/zig ./build.sh        # or put Zig 0.16.0 on PATH first
```

`build.sh` runs the host tests, builds the wasm32-freestanding
module, and refreshes the checked-in `counter.wasm`, which the host's
F12.7 conformance suite (`pkgs/id/crates/id-worlds/src/sandbox.rs`)
can load as the `zig-counter` fixture — no Zig toolchain is needed
to run the suite.

## Run a hosted world

```sh
id serve --world --world-admin-token "$WORLD_ADMIN" --world-module counter.wasm
```

The module has no Wasm imports and exports the full Plaza ABI v1
surface (see `../roc-world/ABI.md`): `memory`, `plaza_alloc`,
`plaza_free`, `plaza_init`, `plaza_update`, `plaza_view`,
`plaza_layout`, `plaza_records`, `plaza_wants`, `plaza_snapshot`,
`plaza_restore`, `plaza_out_len`, `plaza_abi_version`, and the
`plaza_error_ptr`/`plaza_error_len` diagnostic pair. Adding
`zig-counter` to the conformance suite's module list is the
remaining F12.7 step on #12596.
