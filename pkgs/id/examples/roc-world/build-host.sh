#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
ZIG="${ZIG:-zig}"
# Builds the freestanding wasm host adapter into targets/wasm32/host.wasm.
(cd host && "$ZIG" build)
echo "Built targets/wasm32/host.wasm"
