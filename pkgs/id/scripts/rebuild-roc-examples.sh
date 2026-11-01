#!/usr/bin/env bash
set -euo pipefail

examples=(lounge roc-chat roc-counter roc-grid roc-screen territory tic-tac-toe)

for example in "${examples[@]}"; do
  echo "=== Rebuilding Roc example: $example ==="
  (cd "examples/$example" && bash ./build.sh)
done
