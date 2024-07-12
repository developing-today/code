#!/usr/bin/env bash

printf "%s\n" "done: install-cargo-px script"

SAVED_SHELL_OPTIONS=$(set +o)

# shellcheck disable=SC2317
restore_shell_options() {
  printf "%s\n" "trap start: restoring shell options"
  # printf "%s\n" "SAVED_SHELL_OPTIONS: ${SAVED_SHELL_OPTIONS}"
  # printf "%s\n" "CURRENT_SHELL_OPTIONS: $(set +o)"

  eval "${SAVED_SHELL_OPTIONS}"

  printf "%s\n" "trap done: restoring shell options"
}
trap restore_shell_options EXIT
set -euo pipefail

source ./export-lib.sh

cargo install cargo-px

cd "${1:-./src/pavex/libs}"

cargo update
cargo build --release -p pavex_cli

cargo install sqlx-cli \
    --no-default-features \
    --features native-tls,postgres \
    --version 0.7.0-alpha.3

printf "%s\n" "done: install-cargo-px script"
