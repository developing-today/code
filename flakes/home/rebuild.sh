#!/usr/bin/env bash
set -euo pipefail

# Get the directory of the script
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Git add for the script's directory
cd "${script_dir}" || exit 1
git add .

# Loop through each directory in programs
for dir in "${script_dir}"/programs/*; do
  if [[ -d ${dir} ]]; then
    # Skip the directory if it doesn't contain a flake.nix file
    if [[ ! -f "${dir}/flake.nix" ]]; then
      continue
    fi
    cd "${dir}" || exit 1
    # If a rebuild script exists, execute it
    if [[ -f "./rebuild.sh" ]]; then
      chmod +x ./rebuild.sh
      ./rebuild.sh
    fi
    # TODO: do update-ref sometimes instead of update
    nix flake update --print-build-logs --verbose --show-trace
    cd "${script_dir}" || exit 1
  fi
done

git add .
# TODO: do update-ref sometimes instead of update
nix flake update --print-build-logs --verbose --show-trace
git add .

#TODO: setup cachix, skip cachix if not setup
