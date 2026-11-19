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

# nixpkgs#180175: with tailscaled up, NetworkManager-wait-online (and on some
# hosts systemd-networkd-wait-online) can fail during switch-to-configuration,
# so the switch reports a failed unit. Stopping tailscaled avoids that, but it
# also takes every Tailscale service on this host offline for the whole build
# (on amd that is the fleet queue broker). Only hosts with a wait-online unit
# can hit the bug, so stop tailscaled only there. REBUILD_STOP_TAILSCALE=1 or
# =0 forces the choice either way; through auth.sh, pass it as
#   ./lib/auth.sh env REBUILD_STOP_TAILSCALE=1 ./lib/rebuild-simple.sh
stop_tailscale="${REBUILD_STOP_TAILSCALE:-auto}"
if [[ ${stop_tailscale} == auto ]]; then
  stop_tailscale=0
  for unit in NetworkManager-wait-online.service systemd-networkd-wait-online.service; do
    if systemctl is-enabled --quiet "${unit}" 2>/dev/null; then
      stop_tailscale=1
      echo "${unit} is enabled; tailscaled will be stopped for the rebuild (nixpkgs#180175)"
    fi
  done
fi

was_tailscaled_active=false
if [[ ${stop_tailscale} != 1 ]]; then
  echo "leaving tailscaled running during the rebuild (nixpkgs#180175 workaround not needed here)"
elif systemctl is-active --quiet tailscaled; then # this is a hack: https://github.com/NixOS/nixpkgs/issues/180175#issuecomment-2134547782
  was_tailscaled_active=true
  echo "stopping tailscaled..."      # this is a hack: https://github.com/NixOS/nixpkgs/issues/180175#issuecomment-2134547782
  sudo systemctl stop tailscaled     # this is a hack: https://github.com/NixOS/nixpkgs/issues/180175#issuecomment-2134547782
  echo "tailscaled service stopped." # this is a hack: https://github.com/NixOS/nixpkgs/issues/180175#issuecomment-2134547782
else
  echo "tailscaled service not found or not active." # hack not needed
fi
cleanup_tailscale() {
  if [ "$was_tailscaled_active" = true ]; then
    echo "restarting tailscaled..."
    sudo systemctl start tailscaled || true
  fi
}
trap cleanup_tailscale EXIT
# --refresh
# --offline
# --no-build-nix // --fast
# --use-substitutes
# --no-net
# -vvvv
nixos-rebuild --use-remote-sudo --accept-flake-config --json switch --json --upgrade --json --print-build-logs --verbose --keep-going --log-format internal-json --fallback --show-trace --flake '.' |& nom --json
git add flake.lock
set +e
current=$(nixos-rebuild list-generations | grep current)
set -e
if [[ -z $current ]]; then
  echo "Could not find current, possibly using nixos-25.11, seeking first Current tab = true"
  current="$(nixos-rebuild list-generations --json | jq -r 'to_entries[] | select(.value.current == true) | "\(.value.generation)"')"
fi
echo "current: $current"
hostname=$(hostname)
echo "hostname: $hostname"
git commit --no-verify --allow-empty -m "$hostname $current"
# don't add after rebuild to prevent mismatched file content vs version
