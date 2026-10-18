#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
ROC="${ROC:-roc}"
(cd ../roc-world && ./build-host.sh >/dev/null)
"$ROC" build main.roc --target=wasm32 --debug --wasm-memory=8454144 --output=roc-chat.wasm
echo "Built roc-chat.wasm (import-free Roc world module)"
