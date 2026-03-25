#!/usr/bin/env bash
set -euo pipefail

# update-nixpkgs-inputs.sh
#
# Discover NixOS/nixpkgs inputs from flake.lock and update them.
# Uses nix/nixpkgs-inputs.nix to parse the lock file.
#
# Usage:
#   ./scripts/update-nixpkgs-inputs.sh [ref]
#
# Arguments:
#   ref  - optional branch filter: "master", "nixos-unstable", or empty for all
#
# Examples:
#   ./scripts/update-nixpkgs-inputs.sh              # all NixOS/nixpkgs inputs
#   ./scripts/update-nixpkgs-inputs.sh master        # master (or no ref) inputs
#   ./scripts/update-nixpkgs-inputs.sh nixos-unstable # nixos-unstable inputs

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

ref="${1:-}"

# Discover inputs from flake.lock
if [ -z "$ref" ]; then
  inputs=$(nix eval --raw --impure --expr "import $REPO_DIR/nix/nixpkgs-inputs.nix {}")
else
  inputs=$(nix eval --raw --impure --expr "import $REPO_DIR/nix/nixpkgs-inputs.nix { ref = \"$ref\"; }")
fi

if [ -z "$inputs" ]; then
  label="${ref:-all}"
  echo "No NixOS/nixpkgs $label inputs found in flake.lock"
  exit 0
fi

label="${ref:-all}"
echo "Updating NixOS/nixpkgs ($label) inputs: $(echo "$inputs" | tr '\n' ' ')"
echo "$inputs" | while IFS= read -r input; do
  [ -z "$input" ] && continue
  echo "--- Updating $input ---"
  nix flake update "$input"
done
