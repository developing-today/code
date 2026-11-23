# Rust Sketch guest

This standalone crate implements the domain portion of
`sketch/redo-as-new-stroke` (#13608) on the Plaza ABI v1 Rust SDK. It keeps
draft vertices transient, commits immutable strokes, tombstones an owner's
undo, and copies retained geometry under a fresh ID with provenance. Runtime
participant IDs come from the host ABI; the guest never treats a user-provided
author string as a grant.

Run the independent literal-vector tests with:

```sh
cargo test
```

Build the import-free guest with `./build.sh` after installing the
`wasm32-unknown-unknown` target. The checked-in `sketch.wasm` is what the
host-side fixture `pkgs/id/tests/sketch_host_play.rs` loads: it plays the
six-step vector through the regular ID host and asserts the literal
projections, portable view, snapshot/restore and next-operation equivalence,
with draft UI state, guest-domain records and host permissions kept distinct.
Run it from `pkgs/id/` with `just test-sketch`.
