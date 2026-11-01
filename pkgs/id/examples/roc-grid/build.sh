#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
export ROC="${ROC:-roc}"
source ../roc-toolchain.sh

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

if [ -z "${WASM_OPT:-}" ]; then
  if command -v wasm-opt >/dev/null 2>&1; then
    WASM_OPT="wasm-opt"
  else
    STORE_OPT=$(ls -d /nix/store/*-binaryen-*/bin/wasm-opt 2>/dev/null | head -n1 || true)
    if [ -n "$STORE_OPT" ] && [ -x "$STORE_OPT" ]; then
      WASM_OPT="$STORE_OPT"
    else
      WASM_OPT="nix shell nixpkgs#binaryen --command wasm-opt"
    fi
  fi
fi
(cd ../roc-world && ./build-host.sh >/dev/null)
"$ROC" build main.roc --target=wasm32 --wasm-memory=8454144 --output=roc-grid.wasm

# Shrink the checked-in module (F11.15, #555): optimize with wasm-opt
# and strip DWARF/custom sections so modules stay <= 400 KB. --all-features:
# Roc nightly emits post-MVP features that binaryen rejects by default. The name
# section is kept so Wasmtime trap traces still resolve.
$WASM_OPT -Os --all-features roc-grid.wasm -o roc-grid.wasm.opt
mv roc-grid.wasm.opt roc-grid.wasm
$WASM_TOOLS strip roc-grid.wasm -o roc-grid.wasm.stripped
mv roc-grid.wasm.stripped roc-grid.wasm
echo "Built roc-grid.wasm (import-free Roc world module)"
