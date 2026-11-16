# Plaza guest SDK for Rust

The Rust half of F12.8 (#1065): a guest SDK for `wasm32-unknown-unknown`
with `screen`, `json`, `grid`, `cap` (and `text`) helpers equal to the
Roc platform libraries in `../roc-world`, plus the export plumbing that
turns a pure Rust program into an import-free Plaza ABI v1 world
module (the contract in `../roc-world/ABI.md`).

A guest is a `cdylib` crate that implements `plaza_sdk::World`
(`init` / `update` / `view`, with optional `layout` / `records` /
`wants` / `snapshot` / `restore`) and invokes `plaza_sdk::plaza_guest!`
once. The macro emits all 14 documented `plaza_*` exports with the
ABI's exact signatures; the model is an opaque pointer the host passes
back, outputs go through the one shared output slot, and host inputs
arrive in `plaza_alloc` memory the host frees with `plaza_free`.

`../rust-counter` is the playable example built on this SDK — the Rust
twin of `../roc-counter`, with the same events, screen, records, and
snapshot text, and its checked-in module runs in the host's F12.7
conformance suite (`pkgs/id/src/sandbox.rs`).

## Component model / WIT

F12.8 also asks for a Wasm component-model / WIT evaluation. That
decision is **not** taken here: it is a Drew-owned direction for #1065
(recorded on the issue), and this SDK deliberately targets the landed
ABI v1 module contract so the Rust SDK is usable today. Nothing in the
SDK's helper APIs depends on the raw module boundary, so a future
component-model binding can sit under the same `World` trait.
