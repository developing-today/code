#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
ROC="${ROC:-roc}"
if [ -z "${WASM_TOOLS:-}" ]; then
  if command -v wasm-tools >/dev/null 2>&1; then
    WASM_TOOLS="wasm-tools"
  else
    STORE_TOOL=$(ls -d /nix/store/*-wasm-tools-*/bin/wasm-tools 2>/dev/null | head -n1 || true)
    if [ -n "$STORE_TOOL" ] && [ -x "$STORE_TOOL" ]; then
      WASM_TOOLS="$STORE_TOOL"
    else
      WASM_TOOLS="nix run nixpkgs#wasm-tools --"
    fi
  fi
fi

(cd ../roc-world && ./build-host.sh >/dev/null)
# Roc's default initial wasm memory (64 MiB) is above the sandbox's 16 MiB limit.
"$ROC" build main.roc --target=wasm32 --debug --wasm-memory=8454144 --output=territory.wasm
$WASM_TOOLS validate territory.wasm
echo "Built territory.wasm (import-free Roc world module)"
