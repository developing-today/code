#!/usr/bin/env bash
# Internal Home Manager activation helper.
# Usage: opencode-deploy.sh BUILD_KEY STATE_FILE SYSTEMCTL PROC_ROOT
# Parse the complete argument list before running systemctl or writing state.
set -euo pipefail

if [[ $# -ne 4 ]]; then
  echo "usage: $0 BUILD_KEY STATE_FILE SYSTEMCTL PROC_ROOT" >&2
  exit 2
fi

build_key=$1
state_file=$2
systemctl=$3
proc_root=$4
if [[ -z $build_key || -z $state_file || -z $systemctl || -z $proc_root ]]; then
  echo "usage: $0 BUILD_KEY STATE_FILE SYSTEMCTL PROC_ROOT (arguments must be non-empty)" >&2
  exit 2
fi

previous_key="$(cat "$state_file" 2>/dev/null || true)"
if [[ $previous_key == "$build_key" ]]; then
  exit 0
fi

# `is-system-running` exits nonzero for `degraded`, even though the manager can
# still control services. Do not record a key while it is unavailable; a later
# activation must retry instead of treating an unperformed restart as done.
manager_state="$("$systemctl" --user is-system-running 2>/dev/null || true)"
if [[ $manager_state == running || $manager_state == degraded ]]; then
  # On the first activation there is no saved deployment key. If the active
  # process already names this exact store path, seed the key without restarting.
  if [[ -z $previous_key ]]; then
    pid="$("$systemctl" --user show --property=MainPID --value opencode.service)"
    if [[ $pid =~ ^[1-9][0-9]*$ ]] && grep -zFq -- "$build_key" "$proc_root/$pid/cmdline"; then
      mkdir -p "$(dirname "$state_file")"
      printf '%s\n' "$build_key" >"$state_file"
      exit 0
    fi
  fi

  if "$systemctl" --user is-active --quiet opencode.service; then
    "$systemctl" --user restart opencode.service
  else
    "$systemctl" --user start opencode.service
  fi

  mkdir -p "$(dirname "$state_file")"
  printf '%s\n' "$build_key" >"$state_file"
fi
