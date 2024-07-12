#!/usr/bin/env bash
set -euo pipefail

# Get the directory of the script
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Git add for the script's directory
cd "${script_dir}" || exit 1
git add .

# Loop through each directory in config
for dir in "${script_dir}"/config/*; do
  if [[ -d ${dir} ]]; then
    # Skip the directory if it doesn't contain a flake.nom file
    if [[ ! -f "${dir}/flake.nix" ]]; then
      continue
    fi
    cd "${dir}" || exit 1
    # If a rebuild --json script exists, execute it
    if [[ -f "./rebuild.sh" ]]; then
      chmod +x ./rebuild.sh
      ./rebuild.sh
    fi
    nix flake update --print-build-logs --verbose --keep-going --log-format internal-json --fallback --show-trace |& nom --json
    nix build --json --print-out-paths --print-build-logs --verbose --keep-going --log-format internal-json --fallback --show-trace |& nom --json
    #nom build --json --print-out-paths --json --print-build-logs --verbose --keep-going --log-format internal-json --fallback   --show-trace --json |
    #  jq -r '.[].outputs | to_entries[].value' |
    #  cachix push binary
    # TODO: skip cachix if not setup
    #nom flake archive --print-build-logs --verbose --keep-going --log-format internal-json --fallback  --show-trace --json | jq -r '.path,(.inputs|to_entries[].value.path)' | cachix push binary # todo: make optional

    cd "${script_dir}" || exit 1
  fi
done

git add .

if [[ -f "./flake.nix" ]]; then
# TODO: sometimes do update-ref instead of update
nix flake update --print-build-logs --verbose --keep-going --log-format internal-json --fallback --show-trace |& nom --json
# TODO: skip cachix if not setup
nix build --json --print-out-paths --json --print-build-logs --verbose --keep-going --log-format internal-json --fallback --show-trace |& nom --json
#nom build --json --print-out-paths --json --print-build-logs --verbose --keep-going --log-format internal-json --fallback   --show-trace --json |
#  jq -r '.[].outputs | to_entries[].value' |
#  cachix push binary
#nom flake archive --print-build-logs --verbose --keep-going --log-format internal-json --fallback   --show-trace --json |
#  jq -r '.path,(.inputs|to_entries[].value.path)' |
#  cachix push binary # todo: make optional

git add .
fi
