#!/usr/bin/env bash
set -Eexuo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "${script_dir}" || exit 1

git_root="$(git rev-parse --show-toplevel)"
cd "${git_root}" || exit 1

git add .

if systemctl is-active --quiet tailscaled; then
  sudo systemctl stop tailscaled
fi

nixos-rebuild --use-remote-sudo --accept-flake-config switch --upgrade --keep-going --fallback --show-trace --flake '.'

git add flake.lock
set +e
current=$(nixos-rebuild list-generations | grep current)
set -e
if [[ -z "$current" ]]; then
  current="$(nixos-rebuild list-generations --json | jq -r 'to_entries[] | select(.value.current == true) | "\(.value.generation)"')"
fi
hostname=$(hostname)
git commit --no-verify --allow-empty -m "$hostname $current"
