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

# --- #3817: evaluated-commit logging + HEAD-moved (gen-524) drill ---
# Oracle: a real temp git repo is the flake source; its rev-parse HEAD
# before/after each run decides what the log must name.
src_repo="${TMP_ROOT}/src"
mkdir -p "${src_repo}"
git -C "${src_repo}" init -q
printf 'base\n' >"${src_repo}/flake.txt"
git -C "${src_repo}" add flake.txt
git -C "${src_repo}" -c user.email=drill@example.invalid -c user.name=drill commit -qm base
eval_sha=$(git -C "${src_repo}" rev-parse HEAD)
export RUN_SWITCH_SRC="${src_repo}"

export BLOCK_SWITCH=0
export SWITCH_EXIT_CODE=0
RUN_NAME=guarded "${SCRIPT}" >"${TMP_ROOT}/guarded.stdout" 2>"${TMP_ROOT}/guarded.stderr"
guarded_status=$?
guarded_log=$(<"${TMP_ROOT}/guarded.stdout")
if [[ ${guarded_status} -ne 0 ]]; then
  fail "guarded single invocation exited ${guarded_status} instead of 0"
fi
if ! grep -Fxq "EVALUATED_COMMIT=${eval_sha}" "${guarded_log}"; then
  fail 'switch log did not record the evaluated commit'
fi
if ! grep -Fxq 'TREE_STATE=clean' "${guarded_log}"; then
  fail 'switch log did not record the clean tree state'
fi
if grep -Fq 'HEAD MOVED DURING BUILD' "${guarded_log}"; then
  fail 'HEAD-moved warning fired although HEAD did not move'
fi
ok 'switch log records the evaluated commit and tree state'

# Gen-524 drill: HEAD moves while the (stubbed) build is running.
cat >"${TMP_ROOT}/bin/nixos-rebuild" <<'STUB'
#!/usr/bin/env bash
printf 'pid=%s args=%s\n' "$$" "$*" >>"$CALLS_LOG"
if [[ ${BLOCK_SWITCH:-0} == 1 ]]; then
  : >"${SWITCH_STARTED:?}"
  while [[ ! -e ${SWITCH_RELEASE:?} ]]; do sleep 0.02; done
fi
if [[ ${MOVE_HEAD:-0} == 1 ]]; then
  printf 'drift\n' >>"${RUN_SWITCH_SRC}/flake.txt"
  git -C "${RUN_SWITCH_SRC}" add flake.txt
  git -C "${RUN_SWITCH_SRC}" -c user.email=drill@example.invalid -c user.name=drill commit -qm drift
fi
printf 'stub output for %s\n' "${RUN_NAME:-unnamed}"
exit "${SWITCH_EXIT_CODE:-0}"
STUB
chmod +x "${TMP_ROOT}/bin/nixos-rebuild"

export MOVE_HEAD=1
RUN_NAME=drift "${SCRIPT}" >"${TMP_ROOT}/drift.stdout" 2>"${TMP_ROOT}/drift.stderr"
drift_status=$?
drift_log=$(<"${TMP_ROOT}/drift.stdout")
moved_sha=$(git -C "${src_repo}" rev-parse HEAD)
if [[ ${drift_status} -ne 0 ]]; then
  fail "gen-524 drill invocation exited ${drift_status} instead of 0"
fi
if [[ ${moved_sha} == "${eval_sha}" ]]; then
  fail 'gen-524 drill stub did not move HEAD'
fi
if ! grep -Fxq "EVALUATED_COMMIT=${eval_sha}" "${drift_log}"; then
  fail 'gen-524 drill log did not record the evaluated (stale) commit'
fi
if ! grep -Fq 'HEAD MOVED DURING BUILD' "${drift_log}"; then
  fail 'gen-524 drill log did not warn loudly that HEAD moved during the build'
fi
if ! grep -Fq "${moved_sha}" "${drift_log}"; then
  fail 'gen-524 drill warning did not name the new HEAD commit'
fi
ok 'gen-524 drill: a HEAD move during the build is caught loudly in the log, naming both commits'
export MOVE_HEAD=0

# Refusal must name the running switch's evaluated commit (#3817 acceptance).
export BLOCK_SWITCH=1
export SWITCH_STARTED="${TMP_ROOT}/guarded-started"
export SWITCH_RELEASE="${TMP_ROOT}/release-guarded"
RUN_NAME=holder "${SCRIPT}" >"${TMP_ROOT}/holder.stdout" 2>"${TMP_ROOT}/holder.stderr" &
first_pid=$!
if ! wait_for_file "${SWITCH_STARTED}"; then
  fail 'holder invocation did not reach the stubbed rebuild command'
  exit 1
fi
if timeout 2 env RUN_NAME=contender "${SCRIPT}" >"${TMP_ROOT}/contender.output" 2>&1; then
  contender_status=0
else
  contender_status=$?
fi
if [[ ${contender_status} -eq 0 ]]; then
  fail 'contender was not refused while the holder switch ran'
fi
if ! grep -Fq "${moved_sha}" "${TMP_ROOT}/contender.output"; then
  fail 'refusal did not name the running switch evaluated commit'
fi
touch "${SWITCH_RELEASE}"
wait "${first_pid}" || true
first_pid=
export BLOCK_SWITCH=0
ok 'refusal names the running switch evaluated commit'


if [[ ${failures} -ne 0 ]]; then
  printf '%s test(s) failed\n' "${failures}" >&2
  exit 1
fi
printf 'all run-switch tests passed\n'
