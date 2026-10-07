#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")"
ROC="${ROC:-roc}"
ZIG="${ZIG:-zig}"

# Zig 0.16 builds the freestanding host adapter into the platform input tree.
(cd host && "$ZIG" build)

# The platform target declaration selects wasm32 and merges the host object
# with the Roc application. Keep debug symbols so Wasmtime trap traces can be
# resolved while developing modules. Remove --debug for smaller production
# modules after validation.
"$ROC" build main.roc --debug --output=counter.wasm

# Reject accidental imports before the module is advertised to a world host.
nix run nixpkgs#wasm-tools -- validate counter.wasm
echo "Built counter.wasm (import-free Roc world module)"
