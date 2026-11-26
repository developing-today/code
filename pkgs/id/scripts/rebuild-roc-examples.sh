#!/usr/bin/env bash
# Rebuild every Roc example under pkgs/id.
# Usage: rebuild-roc-examples.sh
#   Accepts no positional arguments.
set -euo pipefail

usage() {
  echo "Usage: rebuild-roc-examples.sh" >&2
}

if [[ $# -ne 0 ]]; then
  usage
  echo "Error: unaccounted argument(s): $*" >&2
  exit 1
fi

cd "$(dirname "${BASH_SOURCE[0]}")/.."

examples=(
  examples/lounge
  examples/roc-chat
  examples/roc-counter
  examples/roc-grid
  examples/roc-burrow
  examples/roc-mind-map
  examples/roc-screen
  examples/roc-text-counter
  examples/territory
  apps/tic-tac-toe/program
  apps/space-invaders/program
  apps/checkers/program
)

for example in "${examples[@]}"; do
  echo "=== Rebuilding Roc example: $example ==="
  (cd "$example" && bash ./build.sh)
done
