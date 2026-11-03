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

if [ -z "${WASM_OPT:-}" ]; then
  if command -v wasm-opt >/dev/null 2>&1; then
    WASM_OPT="wasm-opt"
  else
    STORE_OPT=$(ls -d /nix/store/*-binaryen-*/bin/wasm-opt 2>/dev/null | head -n1 || true)
    if [ -n "$STORE_OPT" ] && [ -x "$STORE_OPT" ]; then
      WASM_OPT="$STORE_OPT"
    else
      WASM_OPT="nix run nixpkgs#binaryen --"
    fi
  fi
fi

(cd ../roc-world && ./build-host.sh >/dev/null)
# Roc's default initial wasm memory (64 MiB) is above the sandbox's 16 MiB limit.
"$ROC" build main.roc --target=wasm32 --wasm-memory=8454144 --output=territory.wasm

# Shrink the checked-in module (F11.15, #555): optimize with wasm-opt
# and strip DWARF/custom sections so modules stay <= 400 KB. --all-features:
# Roc nightly emits post-MVP features that binaryen rejects by default. The name
# section is kept so Wasmtime trap traces still resolve.
$WASM_OPT -Os --all-features territory.wasm -o territory.wasm.opt
mv territory.wasm.opt territory.wasm
$WASM_TOOLS strip territory.wasm -o territory.wasm.stripped
mv territory.wasm.stripped territory.wasm
$WASM_TOOLS validate territory.wasm
echo "Built territory.wasm (import-free Roc world module)"
