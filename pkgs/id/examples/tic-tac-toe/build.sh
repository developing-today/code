#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
ROC="${ROC:-roc}"
WASM_TOOLS="${WASM_TOOLS:-nix run nixpkgs#wasm-tools --}"

(cd ../roc-world && ./build-host.sh >/dev/null)
"$ROC" build main.roc --debug --output=tic-tac-toe.wasm
$WASM_TOOLS validate tic-tac-toe.wasm
echo "Built tic-tac-toe.wasm (import-free Roc world module)"
