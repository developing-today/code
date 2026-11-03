#!/usr/bin/env bash
set -uo pipefail

SELF_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
SCRIPT="${SELF_DIR}/run-switch.sh"
TMP_ROOT=$(mktemp -d)
first_pid=

cleanup() {
  if [[ -n ${first_pid} ]] && kill -0 "${first_pid}" 2>/dev/null; then
    kill "${first_pid}" 2>/dev/null || true
    wait "${first_pid}" 2>/dev/null || true
  fi
  rm -rf -- "${TMP_ROOT}"
}
trap cleanup EXIT

failures=0
fail() {
  printf 'FAIL: %s\n' "$1" >&2
  failures=$((failures + 1))
}
ok() { printf 'ok: %s\n' "$1"; }

wait_for_file() {
  local file=$1
  local attempts=0
  while [[ ! -e ${file} ]] && ((attempts < 100)); do
    sleep 0.02
    ((attempts += 1))
  done
  [[ -e ${file} ]]
}

mkdir -p "${TMP_ROOT}/bin" "${TMP_ROOT}/home" "${TMP_ROOT}/logs" "${TMP_ROOT}/state"
export HOME="${TMP_ROOT}/home"
export PATH="${TMP_ROOT}/bin:${PATH}"
export RUN_SWITCH_LOCK_FILE="${TMP_ROOT}/state/run-switch.lock"
export RUN_SWITCH_LOG_DIR="${TMP_ROOT}/logs"
export RUN_SWITCH_FLAKE="${HOME}/test#host"
export CALLS_LOG="${TMP_ROOT}/calls.log"

cat >"${TMP_ROOT}/bin/sudo" <<'STUB'
#!/usr/bin/env bash
[[ ${1:-} == -S ]] && shift
if [[ ${1:-} == -p ]]; then shift 2; fi
exec "$@"
STUB

cat >"${TMP_ROOT}/bin/nixos-rebuild" <<'STUB'
#!/usr/bin/env bash
printf 'pid=%s args=%s\n' "$$" "$*" >>"$CALLS_LOG"
if [[ ${BLOCK_SWITCH:-0} == 1 ]]; then
  : >"${SWITCH_STARTED:?}"
  while [[ ! -e ${SWITCH_RELEASE:?} ]]; do sleep 0.02; done
fi
printf 'stub output for %s\n' "${RUN_NAME:-unnamed}"
exit "${SWITCH_EXIT_CODE:-0}"
STUB
chmod +x "${TMP_ROOT}/bin/sudo" "${TMP_ROOT}/bin/nixos-rebuild"

export BLOCK_SWITCH=1
export SWITCH_STARTED="${TMP_ROOT}/first-started"
export SWITCH_RELEASE="${TMP_ROOT}/release-first"
RUN_NAME=first "${SCRIPT}" >"${TMP_ROOT}/first.stdout" 2>"${TMP_ROOT}/first.stderr" &
first_pid=$!

if ! wait_for_file "${SWITCH_STARTED}"; then
  fail 'first invocation did not reach the stubbed rebuild command'
  exit 1
fi

first_log=$(<"${TMP_ROOT}/first.stdout")
if [[ ! -f ${first_log} ]]; then
  fail 'first invocation did not create the printed log path'
fi

if timeout 2 env RUN_NAME=second "${SCRIPT}" >"${TMP_ROOT}/second.output" 2>&1; then
  second_status=0
else
  second_status=$?
fi
if [[ ${second_status} -eq 0 ]]; then
  fail 'concurrent second invocation was not refused'
fi
if ! grep -Fq 'another switch is already running' "${TMP_ROOT}/second.output"; then
  fail 'concurrent refusal message was missing'
fi
if ! grep -Fq "${first_log}" "${TMP_ROOT}/second.output"; then
  fail 'concurrent refusal did not identify the active invocation log'
fi
log_count=$(find "${TMP_ROOT}/logs" -maxdepth 1 -type f -name 'switch-*.log' | wc -l)
if [[ ${log_count} -ne 1 ]]; then
  fail "concurrent refusal created an unexpected log (found ${log_count})"
fi

touch "${SWITCH_RELEASE}"
if wait "${first_pid}"; then
  first_status=0
else
  first_status=$?
fi
first_pid=
if [[ ${first_status} -ne 0 ]]; then
  fail "first invocation exited ${first_status} instead of 0"
fi
if ! grep -Fxq 'stub output for first' "${first_log}"; then
  fail 'first invocation log did not contain its own stub output'
fi
if grep -Fq 'stub output for second' "${first_log}"; then
  fail 'second invocation output contaminated the first log'
fi
first_exit_count=$(grep -c '^SWITCH_EXIT=' "${first_log}" || true)
if [[ ${first_exit_count} -ne 1 ]] || ! grep -Fxq 'SWITCH_EXIT=0' "${first_log}"; then
  fail 'first invocation log did not contain exactly one SWITCH_EXIT=0 line'
fi
if [[ ! ${first_log} =~ /switch-[0-9]{8}T[0-9]{6}Z-[0-9]+\.log$ ]]; then
  fail 'first log path did not include a timestamp and PID'
fi
ok 'concurrent invocation is refused without creating or contaminating a log'

export BLOCK_SWITCH=0
export SWITCH_EXIT_CODE=23
RUN_NAME=single "${SCRIPT}" >"${TMP_ROOT}/single.stdout" 2>"${TMP_ROOT}/single.stderr"
single_status=$?
single_log=$(<"${TMP_ROOT}/single.stdout")
if [[ ${single_status} -ne 23 ]]; then
  fail "single invocation returned ${single_status} instead of the rebuild status 23"
fi
if [[ ! -f ${single_log} || ${single_log} == "${first_log}" ]]; then
  fail 'single invocation did not create its own distinct log'
fi
if ! grep -Fxq 'stub output for single' "${single_log}"; then
  fail 'single invocation log did not contain its own stub output'
fi
single_exit_count=$(grep -c '^SWITCH_EXIT=' "${single_log}" || true)
if [[ ${single_exit_count} -ne 1 ]] || ! grep -Fxq 'SWITCH_EXIT=23' "${single_log}"; then
  fail 'single invocation log did not contain exactly one SWITCH_EXIT=23 line'
fi
log_count=$(find "${TMP_ROOT}/logs" -maxdepth 1 -type f -name 'switch-*.log' | wc -l)
if [[ ${log_count} -ne 2 ]]; then
  fail 'expected exactly one log per successful invocation and none for the refusal'
fi
call_count=$(wc -l <"${CALLS_LOG}")
if [[ ${call_count} -ne 2 ]] || ! grep -Fq "switch --flake ${RUN_SWITCH_FLAKE}" "${CALLS_LOG}"; then
  fail 'stubbed nixos-rebuild calls did not match the expected invocations'
fi
ok 'single invocation prints a unique log path and preserves the rebuild exit status'

if [[ ${failures} -ne 0 ]]; then
  printf '%s test(s) failed\n' "${failures}" >&2
  exit 1
fi
printf 'all run-switch tests passed\n'
