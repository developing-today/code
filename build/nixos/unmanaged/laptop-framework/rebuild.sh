#!/usr/bin/env bash
set -euo pipefail
#nix-shell -p nixVersions.nix_2_18 git cachix jq
#cat /mnt/c/wsl/cachix.key | cachix authtoken --stdin
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
    # todo: update-ref instead of update sometimes
    nix flake update
    cd "${script_dir}" || exit 1
  fi
done

git add .
# todo: update-ref instead of update sometimes
nix flake update
git add .
sudo nixos-rebuild switch

#TODO: don't do cachix if not setup
nix flake archive --json | jq -r '.path,(.inputs|to_entries[].value.path)' | cachix push binary # todo: make optional

for dir in "${script_dir}"/flakes/*; do
  if [[ -d ${dir} ]]; then
    cd "${dir}" || exit 1
    if [[ -f "./rebuild.sh" ]]; then
      nix flake archive --json | jq -r '.path,(.inputs|to_entries[].value.path)' | cachix push binary # todo: make optional
    fi
    cd "${script_dir}" || exit 1
  fi
done
