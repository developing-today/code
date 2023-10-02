#!/usr/bin/env bash
set -euo pipefail

# Get the directory of the script
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Git add for the script's directory
cd "${script_dir}" || exit 1
git add .

# Loop through each directory in flakes
for dir in "${script_dir}"/flakes/*; do
  if [[ -d ${dir} ]]; then
    cd "${dir}" || exit 1
    # If a rebuild script exists, execute it
    if [[ -f "./rebuild.sh" ]]; then
      chmod +x ./rebuild.sh
      ./rebuild.sh
    fi
    nix flake update --show-trace
    nix flake check --show-trace
    cd "${script_dir}" || exit 1
  fi
done

git add .
nix flake update --show-trace
nix flake check --show-trace
git add .
sudo nixos-rebuild switch
