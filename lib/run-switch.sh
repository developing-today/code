#!/usr/bin/env bash
# Canonical source for ~/run-switch.sh; deploy the laptop wrapper from this file.
#
# Guards (issue #3753, issue #3817):
# - flock: overlapping invocations are refused, naming the running
#   switch's log and evaluated commit, so two switches cannot build
#   concurrently or clobber each other's logs.
# - Evaluated-commit logging: every switch log records the flake
#   source's HEAD commit and tree state (clean/dirty) at eval time.
# - HEAD-moved warning: if the source HEAD moves during the build
#   (the 2026-11-01 gen-524 sequence), the end of the log warns
#   loudly naming both commits — the activated generation was built
#   from the evaluated commit, not the new HEAD.
set -uo pipefail
umask 077

home=${HOME:?HOME must be set}
lock_file=${RUN_SWITCH_LOCK_FILE:-"${home}/.run-switch.lock"}
log_dir=${RUN_SWITCH_LOG_DIR:-"${home}"}
flake=${RUN_SWITCH_FLAKE:-"${home}/code#amd"}
src_dir=${RUN_SWITCH_SRC:-"${home}/code"}

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
  holder_commit=$(sed -n 's/^commit=//p' "${lock_file}" | tail -n 1)
  refusal='run-switch: another switch is already running'
  [[ -n ${holder_log} ]] && refusal+=" (active log: ${holder_log}"
  [[ -n ${holder_commit} ]] && refusal+=", evaluated commit: ${holder_commit}"
  [[ -n ${holder_log} ]] && refusal+=")"
  [[ -z ${holder_log} && -n ${holder_commit} ]] && refusal+=" (evaluated commit: ${holder_commit})"
  [[ -z ${holder_log} && -z ${holder_commit} ]] && refusal+=" (lock: ${lock_file})"
  printf '%s\n' "${refusal}" >&2
  exit 1
fi

# Capture the evaluated source state before the build starts. The flake
# is evaluated from src_dir at invocation time; that commit — not
# whatever HEAD is when the long build finishes — is what lands.
eval_commit=$(git -C "${src_dir}" rev-parse HEAD 2>/dev/null || printf 'unknown')
if [[ ${eval_commit} != unknown ]]; then
  if [[ -z $(git -C "${src_dir}" status --porcelain 2>/dev/null) ]]; then
    tree_state=clean
  else
    tree_state=dirty
  fi
else
  tree_state=unknown
fi

# Store holder details only after acquiring the lock. The file is informational;
# flock on the open descriptor is the authority.
printf 'pid=%s\nlog=%s\ncommit=%s\n' "$$" "${log_file}" "${eval_commit}" >"${lock_file}"
printf '%s\n' "${log_file}"

{
  printf 'EVALUATED_COMMIT=%s\n' "${eval_commit}"
  printf 'TREE_STATE=%s\n' "${tree_state}"
  printf 'SWITCH_STARTED_AT=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} >"${log_file}"

# shellcheck disable=SC2024 # The log file is user-owned; the shell opens it intentionally.
sudo -S -p '' nixos-rebuild switch --flake "${flake}" >>"${log_file}" 2>&1
switch_status=$?

end_commit=$(git -C "${src_dir}" rev-parse HEAD 2>/dev/null || printf 'unknown')
if [[ ${eval_commit} != unknown && ${end_commit} != unknown && ${end_commit} != "${eval_commit}" ]]; then
  {
    printf '*** WARNING: HEAD MOVED DURING BUILD ***\n'
    printf 'WARNING: switch evaluated commit %s but %s HEAD is now %s; the activated generation was built from %s — re-run run-switch.sh to land %s.\n' \
      "${eval_commit}" "${src_dir}" "${end_commit}" "${eval_commit}" "${end_commit}"
  } >>"${log_file}"
  printf 'run-switch: WARNING: HEAD moved during build (evaluated %s, now %s); see %s\n' \
    "${eval_commit}" "${end_commit}" "${log_file}" >&2
fi

printf 'SWITCH_EXIT=%s\n' "${switch_status}" >>"${log_file}"
exit "${switch_status}"
