#!/usr/bin/env bash
set -exuo pipefail
#ulimit -n $(ulimit -Hn)
#sudo prlimit --pid $$ --nofile=1000000:1000000
#nix-shell -p nixVersions.nix_2_18 git cachix jq
#cat /mnt/c/wsl/cachix.key | cachix authtoken --stdin
# Get the directory of the script
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Git add for the script's directory
cd "${script_dir}" || exit 1
echo "entered: $script_dir"
echo "git add ."
git add .

# Loop through each directory in flakes
for dir in "${script_dir}"/pkgs/*; do
  if [[ -d ${dir} ]]; then
    if [[ ! -f "${dir}/rebuild.sh" ]] && [[ ! -f "${dir}/flake.nix" ]]; then
      #echo "invalid target"
      continue
    fi
    cd "${dir}" || exit 1
    echo "entered: ${dir}"
    # If a rebuild --json script exists, execute it
    if [[ -f "./rebuild.sh" ]]; then
      echo "./rebuild.sh exists, running..."
      chmod +x ./rebuild.sh
      ./rebuild.sh
      #else
      #echo "no ./rebuild.sh exists."
    fi
    # todo: update-ref instead of update sometimes
    if [[ -f "./flake.nix" ]]; then
      echo "is a flake: ${dir}"
      nix flake update --print-build-logs --verbose --keep-going --log-format internal-json --fallback --show-trace |& nom --json
      #else
      #echo "not a flake: ${dir}"
    fi
    echo "exiting: ${dir}"
    cd "${script_dir}" || exit 1
    echo "entered: ${script_dir}"
    #else
    #echo "not a dir: ${dir}"
  fi
done

echo "git add ."
git add .
# todo: update-ref instead of update sometimes

if [[ -f "./flake.nix" ]]; then
  echo "is a flake: ${dir}"
  echo "updating flake..."
  nix flake update --print-build-logs --verbose --keep-going --log-format internal-json --fallback --show-trace |& nom --json
  echo "git add ."
  git add .
  if systemctl is-active --quiet tailscaled; then # this is a hack: https://github.com/NixOS/nixpkgs/issues/180175#issuecomment-2134547782
    echo "stopping tailscaled..." # this is a hack: https://github.com/NixOS/nixpkgs/issues/180175#issuecomment-2134547782
    sudo systemctl stop tailscaled # this is a hack: https://github.com/NixOS/nixpkgs/issues/180175#issuecomment-2134547782
    echo "tailscaled service stopped." # this is a hack: https://github.com/NixOS/nixpkgs/issues/180175#issuecomment-2134547782
  else
    echo "tailscaled service not found or not active." # hack not needed
  fi
  echo "running nixos-rebuild switch..."
  sudo nixos-rebuild --accept-flake-config --json switch --json --upgrade --json --print-build-logs --verbose --keep-going --log-format internal-json --fallback --show-trace --flake '.' |& nom --json
  #else
  #echo "not a flake: ${dir}"
fi

#TODO: don't do cachix if not setup
#nom flake archive --print-build-logs --verbose --keep-going --log-format internal-json --fallback  --show-trace --json | jq -r '.path,(.inputs|to_entries[].value.path)' | cachix push binary # todo: make optional

for dir in "${script_dir}"/pkgs/*; do
  if [[ -d ${dir} ]]; then
    cd "${dir}" || exit 1
    #if [[ -f "./rebuild.sh" ]]; then
      #echo ""
      #      nom flake archive --print-build-logs --verbose --keep-going --log-format internal-json --fallback  --show-trace --json | jq -r '.path,(.inputs|to_entries[].value.path)' | cachix push binary # todo: make optional
    #fi
    cd "${script_dir}" || exit 1
  fi
done

echo "direnv reload"
direnv reload
echo "done: ${script_dir}"
