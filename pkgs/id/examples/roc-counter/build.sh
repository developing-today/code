#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
ROC="${ROC:-roc}"
WASM_TOOLS="${WASM_TOOLS:-nix run nixpkgs#wasm-tools --}"

# The world platform (host adapter + ABI) is shared by every app.
(cd ../roc-world && ./build-host.sh >/dev/null)

# Keep debug symbols so Wasmtime trap traces can be resolved while developing
# modules. Remove --debug for smaller production modules after validation.
"$ROC" build main.roc --debug --output=counter.wasm

# Reject accidental imports before the module is advertised to a world host.
$WASM_TOOLS validate counter.wasm
echo "Built counter.wasm (import-free Roc world module)"
