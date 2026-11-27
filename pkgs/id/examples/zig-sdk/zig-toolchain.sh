#!/usr/bin/env bash
# Shared compiler guard for the Zig guest SDK (#12596). Mirrors
# ../roc-toolchain.sh: the pinned Zig version lives in
# pkgs/id/nix/zig-version and the SDK refuses to build under any other
# compiler, so a drifted toolchain fails loudly instead of producing a
# guest module the host may reject.
#
# Usage: zig-toolchain.sh
#   Accepts no positional arguments.
set -euo pipefail

usage() {
  echo "Usage: zig-toolchain.sh" >&2
}

if [[ $# -ne 0 ]]; then
  usage
  echo "Error: unaccounted argument(s): $*" >&2
  exit 1
fi

ID_ZIG_VERSION_FILE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../nix" && pwd)/zig-version"
ID_ZIG_EXPECTED_VERSION="$(<"$ID_ZIG_VERSION_FILE")"
ID_ZIG_BIN="${ZIG:-zig}"

if ! ID_ZIG_ACTUAL_OUTPUT="$("$ID_ZIG_BIN" version 2>&1)"; then
  printf 'Zig toolchain check failed: could not run %s version:\n%s\n' \
    "$ID_ZIG_BIN" "$ID_ZIG_ACTUAL_OUTPUT" >&2
  exit 1
fi

if [[ $ID_ZIG_ACTUAL_OUTPUT != "$ID_ZIG_EXPECTED_VERSION" ]]; then
  printf 'Zig toolchain mismatch: expected %s from %s, got %s from %s; refusing to build.\n' \
    "$ID_ZIG_EXPECTED_VERSION" "$ID_ZIG_VERSION_FILE" "$ID_ZIG_ACTUAL_OUTPUT" "$ID_ZIG_BIN" >&2
  exit 1
fi

echo "Using pinned Zig compiler $ID_ZIG_EXPECTED_VERSION"
