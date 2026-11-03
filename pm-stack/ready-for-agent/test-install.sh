#!/usr/bin/env bash
# Exercise install.sh with isolated HOME and stubbed systemctl/npm/curl.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
ORIGINAL_PATH="${PATH}"
TEST_ROOT="$(mktemp -d)"
trap 'rm -rf "${TEST_ROOT}"' EXIT

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

run_case() {
  local name="$1" initial_version="$2" active="$3" unit_matches="$4"
  local extra_args="${5:-}" git_fail_once="${6:-0}"
  local case_root="${TEST_ROOT}/${name}"
  local home="${case_root}/home" deploy="${case_root}/deploy"
  local stub_bin="${case_root}/bin" state="${case_root}/state"
  local log="${case_root}/systemctl.log" output

  mkdir -p "${home}/.local/bin" "${home}/.config/systemd/user" "${deploy}/systemd" "${stub_bin}" "${state}"
  cp "${REPO_ROOT}/pm-stack/ready-for-agent/install.sh" "${deploy}/install.sh"
  cp "${REPO_ROOT}/pm-stack/ready-for-agent/systemd/ready-for-agent.service" \
    "${deploy}/systemd/ready-for-agent.service"
  if [[ ${unit_matches} == true ]]; then
    cp "${deploy}/systemd/ready-for-agent.service" "${home}/.config/systemd/user/ready-for-agent.service"
  elif [[ ${unit_matches} == false ]]; then
    echo 'stale unit' >"${home}/.config/systemd/user/ready-for-agent.service"
  fi

  if [[ ${initial_version} != missing ]]; then
    printf '%s\n' "${initial_version}" >"${home}/.ready-agent-version"
    cat >"${home}/.local/bin/ready-for-agent" <<'LAUNCHER'
#!/usr/bin/env bash
printf 'ready-for-agent %s\n' "$(cat "${HOME}/.ready-agent-version")"
LAUNCHER
    chmod +x "${home}/.local/bin/ready-for-agent"
  fi

  if [[ ${active} == active ]]; then
    printf 'active\n' >"${state}/active"
    printf '100\n' >"${state}/pid"
    printf '1000\n' >"${state}/start"
  else
    printf 'inactive\n' >"${state}/active"
    printf '0\n' >"${state}/pid"
    printf '0\n' >"${state}/start"
  fi
  printf '%s\n' "${git_fail_once}" >"${state}/git_fail_once"
  : >"${log}"

  cat >"${stub_bin}/systemctl" <<'SYSTEMCTL'
#!/usr/bin/env bash
set -euo pipefail
[[ ${1:-} == --user ]] || exit 2
shift
while [[ ${1:-} == --no-pager ]]; do shift; done
command="${1:-}"
shift || true
case "${command}" in
  is-active)
    [[ $(cat "${STUB_STATE}/active") == active ]]
    ;;
  show)
    property=""
    for arg in "$@"; do
      case "${arg}" in
        --property=*) property="${arg#--property=}" ;;
      esac
    done
    case "${property}" in
      MainPID) cat "${STUB_STATE}/pid" ;;
      ExecMainStartTimestampMonotonic) cat "${STUB_STATE}/start" ;;
      *) exit 2 ;;
    esac
    ;;
  daemon-reload|enable)
    printf '%s\n' "${command}" >> "${STUB_LOG}"
    ;;
  restart|start)
    printf '%s\n' "${command}" >> "${STUB_LOG}"
    printf 'active\n' > "${STUB_STATE}/active"
    printf '%s\n' "$(( $(cat "${STUB_STATE}/pid") + 1 ))" > "${STUB_STATE}/pid"
    printf '%s\n' "$(( $(cat "${STUB_STATE}/start") + 1000 ))" > "${STUB_STATE}/start"
    ;;
  status)
    printf 'Active: active (running)\nMain PID: %s\n' "$(cat "${STUB_STATE}/pid")"
    ;;
  *) echo "unexpected systemctl command: ${command}" >&2; exit 2 ;;
esac
SYSTEMCTL

  cat >"${stub_bin}/npm" <<'NPM'
#!/usr/bin/env bash
set -euo pipefail
printf 'npm install\n' >> "${STUB_LOG}"
printf '1.1.0\n' > "${HOME}/.ready-agent-version"
cat > "${HOME}/.local/bin/ready-for-agent" <<'LAUNCHER'
#!/usr/bin/env bash
printf 'ready-for-agent %s\n' "$(cat "${HOME}/.ready-agent-version")"
LAUNCHER
chmod +x "${HOME}/.local/bin/ready-for-agent"
NPM
  cat >"${stub_bin}/git" <<'GIT'
#!/usr/bin/env bash
set -euo pipefail
# Fail once when the case arms it (npm-ok / git-fail sequence), then
# succeed; clone creates its target dir like the real thing.
if [[ $(cat "${STUB_STATE}/git_fail_once") == 1 ]]; then
  printf '0\n' > "${STUB_STATE}/git_fail_once"
  exit 1
fi
if [[ ${1:-} == clone ]]; then
  mkdir -p "${@: -1}"
fi
exit 0
GIT
  cat >"${stub_bin}/curl" <<'CURL'
#!/usr/bin/env bash
exit 0
CURL
  cat >"${stub_bin}/sleep" <<'SLEEP'
#!/usr/bin/env bash
exit 0
SLEEP
  chmod +x "${stub_bin}/systemctl" "${stub_bin}/npm" "${stub_bin}/git" "${stub_bin}/curl" "${stub_bin}/sleep"

  if [[ ${name} == clone-fail-retry ]]; then
    # #4839: npm upgrades the package, then the clone step fails; the
    # retry must restart (pending marker), not report the pinned
    # version already running, and must clear the marker on success.
    local marker="${home}/.local/state/ready-for-agent.pending-restart" retry_output
    if output="$(HOME="${home}" XDG_STATE_HOME="${home}/.local/state" PATH="${stub_bin}:${ORIGINAL_PATH}" STUB_STATE="${state}" STUB_LOG="${log}" \
      bash "${deploy}/install.sh" ${extra_args} 2>&1)"; then
      printf '%s\n' "${output}" >&2
      fail "${name}: first run (failing clone) unexpectedly succeeded"
    fi
    [[ -f ${marker} ]] || fail "${name}: pending-restart marker missing after failed run"
    ! grep -Fxq restart "${log}" || fail "${name}: restarted before the clone failure"
    : >"${log}"
    if ! retry_output="$(HOME="${home}" XDG_STATE_HOME="${home}/.local/state" PATH="${stub_bin}:${ORIGINAL_PATH}" STUB_STATE="${state}" STUB_LOG="${log}" \
      bash "${deploy}/install.sh" ${extra_args} 2>&1)"; then
      printf '%s\n' "${retry_output}" >&2
      fail "${name}: retry exited unsuccessfully"
    fi
    grep -Fxq restart "${log}" || fail "${name}: retry did not restart"
    printf '%s\n' "${retry_output}" | grep -Fq 'after restart' || fail "${name}: retry missing after-restart report"
    printf '%s\n' "${retry_output}" | grep -Fq 'already running' && fail "${name}: retry falsely reported no-op"
    [[ ! -f ${marker} ]] || fail "${name}: marker not cleared after verified restart"
    echo "PASS: ${name}"
    return 0
  fi

  if ! output="$(HOME="${home}" XDG_STATE_HOME="${home}/.local/state" PATH="${stub_bin}:${ORIGINAL_PATH}" STUB_STATE="${state}" STUB_LOG="${log}" \
    bash "${deploy}/install.sh" ${extra_args} 2>&1)"; then
    printf '%s\n' "${output}" >&2
    fail "${name}: installer exited unsuccessfully"
  fi

  printf '%s\n' "${output}" | grep -Fq 'ready-for-agent@1.1.0'
  case "${name}" in
  package-change-active)
    grep -Fxq restart "${log}" || fail "${name}: expected restart"
    grep -Fxq 'npm install' "${log}" || fail "${name}: expected package install"
    ;;
  unit-change-active)
    grep -Fxq restart "${log}" || fail "${name}: expected restart"
    grep -Fxq daemon-reload "${log}" || fail "${name}: expected daemon-reload"
    ;;
  unchanged-active)
    ! grep -Fxq restart "${log}" || fail "${name}: unexpectedly restarted"
    ! grep -Fxq start "${log}" || fail "${name}: unexpectedly started"
    printf '%s\n' "${output}" | grep -Fq 'no restart' || fail "${name}: missing no-op report"
    ;;
  inactive)
    grep -Fxq start "${log}" || fail "${name}: expected start"
    ! grep -Fxq restart "${log}" || fail "${name}: unexpectedly restarted"
    ;;
  first-install)
    grep -Fxq start "${log}" || fail "${name}: expected start"
    grep -Fxq daemon-reload "${log}" || fail "${name}: expected daemon-reload"
    grep -Fxq 'npm install' "${log}" || fail "${name}: expected package install"
    ! grep -Fxq restart "${log}" || fail "${name}: unexpectedly restarted"
    ;;
  *) fail "unknown test case: ${name}" ;;
  esac
  echo "PASS: ${name}"
}

run_case package-change-active 1.0.0 active true
run_case unit-change-active 1.1.0 active false
run_case unchanged-active 1.1.0 active true
run_case inactive 1.1.0 inactive true
run_case first-install missing inactive missing
run_case clone-fail-retry 1.0.0 active true clone 1
