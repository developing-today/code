#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

examples=(
  examples/lounge
  examples/roc-chat
  examples/roc-counter
  examples/roc-grid
  examples/roc-screen
  examples/roc-text-counter
  examples/territory
  apps/tic-tac-toe/program
)

for example in "${examples[@]}"; do
  echo "=== Rebuilding Roc example: $example ==="
  (cd "$example" && bash ./build.sh)
done
