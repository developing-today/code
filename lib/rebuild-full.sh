#!/usr/bin/env bash
set -exuo pipefail

#ulimit -n $(ulimit -Hn)
#sudo prlimit --pid $$ --nofile=1000000:1000000
#nix-shell -p nixVersions.nix_2_18 git cachix jq
#cat /mnt/c/wsl/cachix.key | cachix authtoken --stdin

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

cd "${script_dir}" || exit 1
echo "entered: $script_dir"

git_root="$(git rev-parse --show-toplevel)"
cd "${git_root}" || exit 1
echo "entered git root: ${git_root}"

echo "git add ."
git add .

set +x
for dir in "${script_dir}"/pkgs/*; do
  if [[ -d ${dir} ]]; then
    if [[ ! -f "${dir}/rebuild.sh" ]] && [[ ! -f "${dir}/flake.nix" ]]; then
      #echo "invalid target"
      continue
    fi
    cd "${dir}" || exit 1
    echo "entered: ${dir}"
    # todo: update-ref instead of update sometimes
    # or: nix flake lock --update-input
    if [[ -f "./flake.nix" ]]; then
      echo "is a flake: ${dir}"
      nix flake update --print-build-logs --verbose --keep-going --log-format internal-json --fallback --show-trace |& nom --json
      echo "git add flake.lock"
      git add flake.lock
      #else
      #echo "not a flake: ${dir}"
    fi
    # If a rebuild --json script exists, execute it
    if [[ -f "./rebuild.sh" ]]; then
      echo "./rebuild.sh exists, running..."
      chmod +x ./rebuild.sh
      ./rebuild.sh
      #else
      #echo "no ./rebuild.sh exists."
    fi
    echo "exiting: ${dir}"
    cd "${script_dir}" || exit 1
    echo "entered: ${script_dir}"
    #else
    #echo "not a dir: ${dir}"
  fi
done
set -x

# todo: update-ref instead of update sometimes

if [[ -f "./flake.nix" ]]; then
  echo "is a flake: ${dir}"
  echo "updating flake..."
  nix flake update --print-build-logs --verbose --keep-going --log-format internal-json --fallback --show-trace |& nom --json
  echo "git add flake.lock"
  git add flake.lock
  if systemctl is-active --quiet tailscaled; then # this is a hack: https://github.com/NixOS/nixpkgs/issues/180175#issuecomment-2134547782
    echo "stopping tailscaled..." # this is a hack: https://github.com/NixOS/nixpkgs/issues/180175#issuecomment-2134547782
    sudo systemctl stop tailscaled # this is a hack: https://github.com/NixOS/nixpkgs/issues/180175#issuecomment-2134547782
    echo "tailscaled service stopped." # this is a hack: https://github.com/NixOS/nixpkgs/issues/180175#issuecomment-2134547782
  else
    echo "tailscaled service not found or not active." # hack not needed
  fi
  echo "running nixos-rebuild switch..."
  # --refresh
  # --offline
  # --no-build-nix // --fast
  # --use-substitutes
  # --no-net
  # -vvvv
  nixos-rebuild --use-remote-sudo --accept-flake-config --json switch --json --upgrade --json --print-build-logs --verbose --keep-going --log-format internal-json --fallback --show-trace --flake '.' |& nom --json

  current=$(nixos-rebuild list-generations | grep current)
  echo "current: $current"
  hostname=$(hostname)
  echo "hostname: $hostname"
  git commit --no-verify --allow-empty -m "$hostname $current"
  # don't add after rebuild to prevent mismatched file content vs version
  #else
  #echo "not a flake: ${dir}"
fi

#TODO: don't do cachix if not setup
#nom flake archive --print-build-logs --verbose --keep-going --log-format internal-json --fallback  --show-trace --json | jq -r '.path,(.inputs|to_entries[].value.path)' | cachix push binary # todo: make optional

# for dir in "${script_dir}"/pkgs/*; do
#   if [[ -d ${dir} ]]; then
#     cd "${dir}" || exit 1
#     #if [[ -f "./rebuild.sh" ]]; then
#       #echo ""
#       #      nom flake archive --print-build-logs --verbose --keep-going --log-format internal-json --fallback  --show-trace --json | jq -r '.path,(.inputs|to_entries[].value.path)' | cachix push binary # todo: make optional
#     #fi
#     cd "${script_dir}" || exit 1
#   fi
# done

echo "direnv reload"
direnv reload
echo "done: ${script_dir}"
