#!/usr/bin/env bash
# Tests for the Zig guest SDK (#12596): the json, grid, screen, and cap
# helpers with `zig test`, plus a wasm32-freestanding build proving the
# SDK compiles for the guest target. Run from anywhere; the pinned Zig
# version is enforced by zig-toolchain.sh before anything builds.
#
# Usage: zig-sdk-test.sh
#   Accepts no positional arguments.
set -euo pipefail

usage() {
  echo "Usage: zig-sdk-test.sh" >&2
}

if [[ $# -ne 0 ]]; then
  usage
  echo "Error: unaccounted argument(s): $*" >&2
  exit 1
fi

cd "$(dirname "$0")"

ID_ZIG_BIN="${ZIG:-zig}"

if ! command -v "$ID_ZIG_BIN" >/dev/null 2>&1; then
  echo "zig-sdk test: the pinned Zig compiler is not on PATH; refusing to skip silently." >&2
  echo "zig-sdk test: install Zig $(cat ../../nix/zig-version) (see pkgs/id/nix/zig-version) or set ZIG=/path/to/zig." >&2
  exit 1
fi

bash zig-toolchain.sh
"$ID_ZIG_BIN" fmt --check src build.zig
"$ID_ZIG_BIN" build test
"$ID_ZIG_BIN" build -Dtarget=wasm32-freestanding
echo "✓ Zig SDK tests passed (json + grid + screen + cap; wasm32-freestanding build OK)"
