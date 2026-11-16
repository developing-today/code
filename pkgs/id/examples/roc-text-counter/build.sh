#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
export ROC="${ROC:-roc}"
source ../roc-toolchain.sh
WASM_TOOLS="${WASM_TOOLS:-nix run nixpkgs#wasm-tools --}"

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

# The world platform (host adapter + ABI) is shared by every app.
(cd ../roc-world && ./build-host.sh >/dev/null)

# Built without --debug and shrunk below (F11.15, #555) so the checked-in
# module stays <= 1 MiB; the strip step keeps the name section, so
# Wasmtime trap traces still resolve function names.
"$ROC" build main.roc --target=wasm32 --wasm-memory=8454144 --output=text-counter.wasm

# Shrink the checked-in module (F11.15, #555): optimize with wasm-opt
# and strip DWARF/custom sections so modules stay <= 1 MiB. --all-features:
# Roc nightly emits post-MVP features that binaryen rejects by default. The name
# section is kept so Wasmtime trap traces still resolve.
$WASM_OPT -Os --all-features text-counter.wasm -o text-counter.wasm.opt
mv text-counter.wasm.opt text-counter.wasm
$WASM_TOOLS strip text-counter.wasm -o text-counter.wasm.stripped
mv text-counter.wasm.stripped text-counter.wasm

# Reject accidental imports before the module is advertised to a world host.
$WASM_TOOLS validate text-counter.wasm
echo "Built text-counter.wasm (import-free Roc legacy-text world module)"
