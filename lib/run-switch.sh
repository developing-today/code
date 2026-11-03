#!/usr/bin/env bash
# Canonical source for ~/run-switch.sh; deploy the laptop wrapper from this file.
set -uo pipefail
umask 077

home=${HOME:?HOME must be set}
lock_file=${RUN_SWITCH_LOCK_FILE:-"${home}/.run-switch.lock"}
log_dir=${RUN_SWITCH_LOG_DIR:-"${home}"}
flake=${RUN_SWITCH_FLAKE:-"${home}/code#amd"}

if ! mkdir -p "${log_dir}"; then
  printf 'run-switch: cannot create log directory: %s\n' "${log_dir}" >&2
  exit 1
fi

timestamp=$(date -u +%Y%m%dT%H%M%SZ)
log_file="${log_dir}/switch-${timestamp}-$$.log"

# Append-open so a contender cannot truncate the active invocation's metadata.
if ! exec {lock_fd}>>"${lock_file}"; then
  printf 'run-switch: cannot open lock file: %s\n' "${lock_file}" >&2
  exit 1
fi

if ! flock -n "${lock_fd}"; then
  holder_log=$(sed -n 's/^log=//p' "${lock_file}" | tail -n 1)
  if [[ -n ${holder_log} ]]; then
    printf 'run-switch: another switch is already running (active log: %s)\n' "${holder_log}" >&2
  else
    printf 'run-switch: another switch is already running (lock: %s)\n' "${lock_file}" >&2
  fi
  exit 1
fi

# Store holder details only after acquiring the lock. The file is informational;
# flock on the open descriptor is the authority.
printf 'pid=%s\nlog=%s\n' "$$" "${log_file}" >"${lock_file}"
printf '%s\n' "${log_file}"

# shellcheck disable=SC2024 # The log file is user-owned; the shell opens it intentionally.
sudo -S -p '' nixos-rebuild switch --flake "${flake}" >"${log_file}" 2>&1
switch_status=$?
printf 'SWITCH_EXIT=%s\n' "${switch_status}" >>"${log_file}"
exit "${switch_status}"
