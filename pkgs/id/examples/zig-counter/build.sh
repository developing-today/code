#!/usr/bin/env bash
# Build the Zig counter guest (#12596) and refresh the checked-in
# module the host conformance suite loads. Runs the host tests,
# builds the wasm32-freestanding module with the pinned Zig (the
# version guard is ../zig-sdk/zig-toolchain.sh), and copies the
# result to counter.wasm.
#
# Usage: build.sh
#   Accepts no positional arguments.
set -euo pipefail

usage() {
  echo "Usage: build.sh" >&2
}

if [[ $# -ne 0 ]]; then
  usage
  echo "Error: unaccounted argument(s): $*" >&2
  exit 1
fi

cd "$(dirname "${BASH_SOURCE[0]}")"

ID_ZIG_BIN="${ZIG:-zig}"

if ! command -v "$ID_ZIG_BIN" >/dev/null 2>&1; then
  echo "zig-counter build: the pinned Zig compiler is not on PATH." >&2
  echo "zig-counter build: install Zig $(cat ../../nix/zig-version) (see pkgs/id/nix/zig-version) or set ZIG=/path/to/zig." >&2
  exit 1
fi

bash ../zig-sdk/zig-toolchain.sh
"$ID_ZIG_BIN" build test
"$ID_ZIG_BIN" build
cp zig-out/bin/counter.wasm counter.wasm
echo "wrote counter.wasm"
