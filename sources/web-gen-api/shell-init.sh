#!/usr/bin/env bash

# nix-shell

rustup toolchain install nightly
rustup default nightly
rustup target add wasm32-unknown-unknown

cargo install cargo-leptos trunk

./db-init-create.sh

./shell-enter.sh
