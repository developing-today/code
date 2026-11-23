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
`wasm32-unknown-unknown` target. The host load/play fixture and its Fleet
verification remain a separate integration step; this crate's unit tests do
not claim host-level permission or deduplication coverage.
