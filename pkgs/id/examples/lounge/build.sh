#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
ROC="${ROC:-roc}"
WASM_TOOLS="${WASM_TOOLS:-nix run nixpkgs#wasm-tools --}"

(cd ../roc-world && ./build-host.sh >/dev/null)
"$ROC" build main.roc --target=wasm32 --debug --wasm-memory=8454144 --output=lounge.wasm
$WASM_TOOLS validate lounge.wasm
echo "Built lounge.wasm (import-free Roc world module)"
