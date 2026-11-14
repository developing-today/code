#!/bin/sh
# Build the Rust counter guest and refresh the checked-in module the
# host conformance suite loads. Requires the wasm32-unknown-unknown
# target (rustup target add wasm32-unknown-unknown).
set -eu
cd "$(dirname "$0")"
cargo build --release --target wasm32-unknown-unknown
cp target/wasm32-unknown-unknown/release/rust_counter.wasm counter.wasm
echo "wrote counter.wasm"
