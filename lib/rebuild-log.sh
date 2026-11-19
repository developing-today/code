# shellcheck shell=bash
# Rebuild logging, sourced (never executed) by lib/auth.sh and lib/rebuild-*.sh.
#
# Every ./rebuild, ./rebuild-simple, ./rebuild-offline and ./script run writes
# its full output to
#   ${XDG_STATE_HOME:-${HOME}/.local/state}/rebuild/<UTC timestamp>-<hostname>.log
# (directory 0700, file 0600, owned by the user who ran it) and points
# latest.log in that directory at it, so a later session running as the same
# user can read what happened. The newest REBUILD_LOG_KEEP (default 30) logs
# are kept; older ones matching the log name pattern are removed.
#
# Who does what:
#   - lib/auth.sh runs as the user. It creates the directory and the file as
#     the user, writes the header and the final exit status, and prints
#     "rebuild log: <path>". It passes the path to the root side as
#     REBUILD_LOG on the sudo command line (sudo resets the environment).
#   - lib/rebuild-*.sh run as root under that sudo. They only append to the
#     existing user-owned file, so its owner and mode stay as the user made
#     them. rebuild_log_wrap re-runs the script with its stdout and stderr
#     teed into the log, and hands nom the original terminal on fds 3 and 4,
#     so nom's redraws reach the terminal and never the log. rebuild_log_nom
#     tees the raw stream going into nom (nix --log-format internal-json)
#     into the log.
#
# Nothing here logs the environment or traces auth.sh's sudo line, so the
# GitHub token in NIX_CONFIG never reaches the log.

# User side: create the log file and export REBUILD_LOG. Returns non-zero
# (and leaves REBUILD_LOG empty) if the log cannot be created; a rebuild must
# never fail because of logging.
rebuild_log_init() {
  local dir stamp host file keep old
  REBUILD_LOG=
  export REBUILD_LOG
  dir="${XDG_STATE_HOME:-${HOME}/.local/state}/rebuild"
  # shellcheck disable=SC2174 # parents get the default mode; chmod below
  mkdir -p -m 700 -- "${dir}" 2>/dev/null || return 1
  [[ -d ${dir} && ! -L ${dir} && -O ${dir} ]] || return 1
  chmod 700 -- "${dir}" 2>/dev/null || return 1
  stamp="$(date -u +%Y%m%dT%H%M%SZ)"
  host="$(hostname 2>/dev/null || uname -n)"
  host="${host//[^A-Za-z0-9._-]/_}"
  file="${dir}/${stamp}-${host}.log"
  # noclobber: never reuse or follow an existing path; a second run in the
  # same second gets a pid suffix.
  if ! (umask 077 && set -o noclobber && : >"${file}") 2>/dev/null; then
    file="${dir}/${stamp}-${host}-$$.log"
    (umask 077 && set -o noclobber && : >"${file}") 2>/dev/null || return 1
  fi
  ln -sfn -- "${file##*/}" "${dir}/latest.log" 2>/dev/null || true
  {
    printf '# rebuild log: started %s host %s command %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "${host}" "$*"
    echo '# format: plain-text lines are the rebuild script output (its stdout+stderr: echo messages, set -x traces, git); lines starting with "@nix " are the raw nix --log-format internal-json events that nom renders (build logs included); nom display itself is not logged'
  } >>"${file}" 2>/dev/null || true
  REBUILD_LOG="${file}"

  # Prune: names start with the UTC timestamp, so glob order is age order.
  keep="${REBUILD_LOG_KEEP:-30}"
  [[ ${keep} =~ ^[0-9]+$ ]] || keep=30
  local -a logs=()
  for old in "${dir}"/[0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9]T[0-9][0-9][0-9][0-9][0-9][0-9]Z-*.log; do
    [[ -f ${old} && ! -L ${old} ]] && logs+=("${old}")
  done
  if ((${#logs[@]} > keep)); then
    for old in "${logs[@]:0:${#logs[@]}-keep}"; do
      [[ ${old} == "${file}" ]] || rm -f -- "${old}"
    done
  fi
  return 0
}

# User side, from an EXIT trap: record the exit status and print the path.
rebuild_log_finish() {
  local rc="$1"
  [[ -n ${REBUILD_LOG:-} ]] || return 0
  printf '# rebuild log: finished %s exit status %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "${rc}" >>"${REBUILD_LOG}" 2>/dev/null || true
  printf 'rebuild log: %s\n' "${REBUILD_LOG}" >&2
}

# Root side: only append to a regular file (not a symlink) owned by the user
# who ran sudo, so the log stays theirs.
rebuild_log_usable() {
  [[ -n ${REBUILD_LOG:-} && -f ${REBUILD_LOG} && ! -L ${REBUILD_LOG} ]] || return 1
  if [[ -n ${SUDO_UID:-} ]]; then
    local uid
    uid="$(stat -c %u -- "${REBUILD_LOG}" 2>/dev/null)" || return 1
    [[ ${uid} == "${SUDO_UID}" ]] || return 1
  fi
  return 0
}

# Call as `rebuild_log_wrap "$@"` near the top of a rebuild script, before
# it changes directory. If logging is on, it re-runs the script with stdout
# and stderr teed into the log and exits with the script's exit status;
# otherwise (or in the re-run child) it returns and the script carries on.
rebuild_log_wrap() {
  # Keep this bookkeeping out of the set -x trace; restored on return.
  {
    local xtrace=+x self rc owner=
    if [[ $- == *x* ]]; then xtrace=-x; fi
    set +x
  } 2>/dev/null
  rebuild_log_wrap_inner "$@"
  set "${xtrace}"
}
rebuild_log_wrap_inner() {
  [[ -z ${REBUILD_LOG_CHILD:-} ]] || return 0
  if [[ -z ${REBUILD_LOG:-} ]]; then
    # Run without lib/auth.sh: log only as a normal user (a log made here
    # by root would be root's, not the user's).
    ((EUID != 0)) || return 0
    rebuild_log_init "$0" "$@" || return 0
    owner=1
  fi
  if ! rebuild_log_usable; then
    echo "rebuild log: not logging, ${REBUILD_LOG} is not a regular file owned by the caller" >&2
    REBUILD_LOG=
    return 0
  fi
  self="$(realpath -- "$0")"
  # Keep the terminal on fds 3 and 4 for nom (see rebuild_log_nom).
  exec 3>&1 4>&2
  # Survive Ctrl-C long enough to record the status; the child keeps the
  # default SIGINT action because a caught signal is reset on exec.
  trap ':' INT
  set +e
  # The tee ignores SIGINT so the child's EXIT traps (e.g. restarting
  # tailscaled) can still write to it after Ctrl-C instead of dying of
  # SIGPIPE. It ends when the child and everything it started close it.
  REBUILD_LOG_CHILD=1 "${self}" "$@" 2>&1 | (trap '' INT && exec tee -a -- "${REBUILD_LOG}")
  rc="${PIPESTATUS[0]}"
  if [[ -n ${owner} ]]; then
    rebuild_log_finish "${rc}"
  fi
  exit "${rc}"
}

# Root side: use in place of `nom` at the end of a `... |& nom --json`
# pipeline. Tees the raw stream into the log and sends nom's display to the
# terminal saved on fds 3 and 4. The tee never fails the pipeline; with
# pipefail its status is still nixos-rebuild's or nom's, as before.
rebuild_log_nom() {
  if [[ -n ${REBUILD_LOG_CHILD:-} && -n ${REBUILD_LOG:-} ]]; then
    { tee -a -- "${REBUILD_LOG}" || true; } | nom "$@" 1>&3 2>&4
  else
    nom "$@"
  fi
}
