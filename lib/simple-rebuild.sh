##!/usr/bin/env bash
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
if systemctl is-active --quiet tailscaled; then # this is a hack: https://github.com/NixOS/nixpkgs/issues/180175#issuecomment-2134547782
  echo "stopping tailscaled..." # this is a hack: https://github.com/NixOS/nixpkgs/issues/180175#issuecomment-2134547782
  sudo systemctl stop tailscaled # this is a hack: https://github.com/NixOS/nixpkgs/issues/180175#issuecomment-2134547782
  echo "tailscaled service stopped." # this is a hack: https://github.com/NixOS/nixpkgs/issues/180175#issuecomment-2134547782
else
  echo "tailscaled service not found or not active." # hack not needed
fi
sudo nixos-rebuild --accept-flake-config --json switch --json --upgrade --json --print-build-logs --verbose --keep-going --log-format internal-json --fallback --show-trace --flake '.' |& nom --json
