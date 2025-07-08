#!/usr/bin/env bash

printf "%s\n" "start: install-cargo-leptos script"

SAVED_SHELL_OPTIONS=$(set +o)

restore_shell_options() {
  printf "%s\n" "trap start: restoring shell options"
  eval "${SAVED_SHELL_OPTIONS}"
  printf "%s\n" "trap done: restoring shell options"
}
trap restore_shell_options EXIT
set -Eexuo pipefail # https://vaneyckt.io/posts/safer_bash_scripts_with_set_euxo_pipefail

source ./export-lib.sh

rustup toolchain install nightly --allow-downgrade
rustup target add wasm32-unknown-unknown

# For NixOS, change .scss to .css in the style directory.
if [[ "${OSTYPE}" == "nixos"* ]]; then
    find ./ -name "*.scss" -exec bash -c 'mv "$0" "${0%.scss}.css"' {} \;
fi

cargo install cargo-generate

# No downloads option for nix, nixos, and buck2
# cargo install --features no_downloads --locked cargo-leptos

# Bleeding edge option
# cargo install --git https://github.com/leptos-rs/cargo-leptos --locked cargo-leptos

cargo install cargo-leptos

printf "%s\n" "done: install-cargo-leptos script"
