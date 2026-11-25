#!/usr/bin/env bash
# stable-entrypoints — create and validate the stable deploy
# entrypoints for the OpenChamber and mcpx systemd user units
# (developing-today/code#14187).
#
# Usage: stable-entrypoints.sh <state-dir> [<link-relpath> <target>]...
#
# For each pair, <state-dir>/<link-relpath> becomes a symlink to
# <target>, repointed atomically (a temporary symlink renamed over
# the old one) and only when it does not already point at <target> —
# an unchanged target leaves the link untouched, so a Home Manager
# switch that does not change a service's build never disturbs the
# entrypoint the running service was started through.
#
# Every target is validated BEFORE any link is touched: a target
# that is missing or not executable fails the whole run with a
# clear message and a non-zero exit, so activation fails instead of
# leaving a unit pointing at a path that cannot start. After
# linking, each link itself is checked executable.
#
# The units' side of the contract (the ExecStart paths that name
# these links) lives in nixos/home/stable-entrypoints.nix; the
# consumer that may repoint a link and restart the service is the
# option-(c) deploy app tracked by dezren39/darwin-nix#1393.
set -euo pipefail

if [[ $# -lt 1 || $((($# - 1) % 2)) -ne 0 ]]; then
  echo "usage: stable-entrypoints.sh <state-dir> [<link-relpath> <target>]..." >&2
  exit 2
fi

state_dir="$1"
shift

pairs=()
while [[ $# -gt 0 ]]; do
  pairs+=("$1" "$2")
  shift 2
done

# Validate every target first: fail clearly, change nothing.
for ((i = 1; i < ${#pairs[@]}; i += 2)); do
  target="${pairs[$i]}"
  if [[ ! -e $target ]]; then
    echo "stable-entrypoints: target for ${pairs[$((i - 1))]} does not exist: $target" >&2
    exit 1
  fi
  if [[ ! -x $target ]]; then
    echo "stable-entrypoints: target for ${pairs[$((i - 1))]} is not executable: $target" >&2
    exit 1
  fi
done

for ((i = 0; i < ${#pairs[@]}; i += 2)); do
  rel="${pairs[$i]}"
  target="${pairs[$((i + 1))]}"
  link="$state_dir/$rel"
  if [[ $(readlink "$link" 2>/dev/null || true) == "$target" ]]; then
    continue
  fi
  mkdir -p "$(dirname "$link")"
  tmp="$link.tmp.$$"
  rm -f "$tmp"
  ln -s "$target" "$tmp"
  mv -Tf "$tmp" "$link"
  echo "stable-entrypoints: $link -> $target"
done

# Validate the links themselves: the unit execs exactly these paths.
for ((i = 0; i < ${#pairs[@]}; i += 2)); do
  link="$state_dir/${pairs[$i]}"
  if [[ ! -x $link ]]; then
    echo "stable-entrypoints: entrypoint is missing or not executable after linking: $link" >&2
    exit 1
  fi
done
