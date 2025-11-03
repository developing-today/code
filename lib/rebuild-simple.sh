#!/usr/bin/env bash
set -Eexuo pipefail # https://vaneyckt.io/posts/safer_bash_scripts_with_set_euxo_pipefail

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

if systemctl is-active --quiet tailscaled; then # this is a hack: https://github.com/NixOS/nixpkgs/issues/180175#issuecomment-2134547782
  echo "stopping tailscaled..."                 # this is a hack: https://github.com/NixOS/nixpkgs/issues/180175#issuecomment-2134547782
  sudo systemctl stop tailscaled                # this is a hack: https://github.com/NixOS/nixpkgs/issues/180175#issuecomment-2134547782
  echo "tailscaled service stopped."            # this is a hack: https://github.com/NixOS/nixpkgs/issues/180175#issuecomment-2134547782
else
  echo "tailscaled service not found or not active." # hack not needed
fi
# --refresh
# --offline
# --no-build-nix // --fast
# --use-substitutes
# --no-net
# -vvvv
nixos-rebuild --use-remote-sudo --accept-flake-config --json switch --json --upgrade --json --print-build-logs --verbose --keep-going --log-format internal-json --fallback --show-trace --flake '.' |& nom --json
current=$(nixos-rebuild list-generations | grep current)
if [[ -z "$current" ]]; then
  echo "Could not find current, possibly using nixos-25.11, seeking first Current tab = true"
  current="$(nixos-rebuild list-generations --json | jq -r 'to_entries[] | select(.value.current == true) | "\(.value.generation)"')"
fi
echo "current: $current"
hostname=$(hostname)
echo "hostname: $hostname"
git commit --no-verify --allow-empty -m "$hostname $current"
# don't add after rebuild to prevent mismatched file content vs version
