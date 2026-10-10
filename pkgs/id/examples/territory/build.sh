#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
ROC="${ROC:-roc}"
WASM_TOOLS="${WASM_TOOLS:-nix run nixpkgs#wasm-tools --}"

(cd ../roc-world && ./build-host.sh >/dev/null)
# Roc's default initial wasm memory (64 MiB) is above the sandbox's 16 MiB limit.
"$ROC" build main.roc --target=wasm32 --debug --wasm-memory=8454144 --output=territory.wasm
$WASM_TOOLS validate territory.wasm
echo "Built territory.wasm (import-free Roc world module)"
