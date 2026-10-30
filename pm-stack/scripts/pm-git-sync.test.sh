#!/usr/bin/env bash
# pm-git-sync.test.sh — harness for pm-git-sync.sh owed-redeploy
# handling (#2391). Builds a throwaway origin + clone in a temp dir,
# stubs systemctl and bun via the PM_GIT_SYNC_* env seams, and drives:
#   1. dep step fails -> unit NOT restarted, owed record exists,
#      run exits non-zero
#   2. next run with HEAD unmoved (old==new) retries the owed
#      redeploy; still failing -> still owed, still no restart
#   3. dep step recovers -> retry restarts the unit, owed cleared,
#      run exits 0
#   4. restart itself fails -> owed recorded; next old==new run
#      retries and clears it
# Run: bash pm-stack/scripts/pm-git-sync.test.sh
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SYNC="$SCRIPT_DIR/pm-git-sync.sh"
SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT

FAILURES=0
check() { # <description> <condition...>
  local desc="$1"
  shift
  if "$@" >/dev/null 2>&1; then
    echo "ok - $desc"
  else
    echo "FAIL - $desc"
    FAILURES=$((FAILURES + 1))
  fi
}
log_has() { grep -q "$1" "$STUB_LOG"; }
log_lacks() { ! grep -q "$1" "$STUB_LOG"; }

# --- fixture repos -----------------------------------------------------------
export GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@test
export GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=test@test
ORIGIN="$SCRATCH/origin.git"
SEED="$SCRATCH/seed"
REPO="$SCRATCH/repo"
git init -q -b main --bare "$ORIGIN"
git clone -q "$ORIGIN" "$SEED"
cd "$SEED" || exit 1
mkdir -p pm-stack/aggregator pm-stack/projector/scripts pm-stack/dispatch
echo '{"name":"agg","version":"1"}' >pm-stack/aggregator/package.json
echo 'lock-v1' >pm-stack/aggregator/bun.lock
echo 'agg v1' >pm-stack/aggregator/index.ts
echo '{"name":"proj","version":"1"}' >pm-stack/projector/package.json
cat >pm-stack/projector/scripts/setup-deps.sh <<'EOS'
#!/usr/bin/env bash
if [ -e "$PM_TEST_PROJ_DEP_FAIL" ]; then exit 1; fi
echo "setup-deps ok"
EOS
chmod +x pm-stack/projector/scripts/setup-deps.sh
echo 'dispatch v1' >pm-stack/dispatch/index.ts
git add -A && git commit -qm v1 && git push -q origin main
git clone -q "$ORIGIN" "$REPO"

# --- stubs -------------------------------------------------------------------
STUB="$SCRATCH/stub"
mkdir -p "$STUB"
STUB_LOG="$SCRATCH/stub.log"
: >"$STUB_LOG"
cat >"$STUB/systemctl" <<'EOS'
#!/usr/bin/env bash
echo "systemctl $*" >>"$PM_TEST_STUB_LOG"
if [ -e "$PM_TEST_RESTART_FAIL" ]; then exit 1; fi
exit 0
EOS
cat >"$STUB/bun" <<'EOS'
#!/usr/bin/env bash
echo "bun $*" >>"$PM_TEST_STUB_LOG"
if [ -e "$PM_TEST_BUN_FAIL" ]; then exit 1; fi
exit 0
EOS
chmod +x "$STUB/systemctl" "$STUB/bun"

export PM_TEST_STUB_LOG="$STUB_LOG"
export PM_TEST_BUN_FAIL="$SCRATCH/bun-fail"
export PM_TEST_RESTART_FAIL="$SCRATCH/restart-fail"
export PM_TEST_PROJ_DEP_FAIL="$SCRATCH/proj-dep-fail"
STATE_DIR="$SCRATCH/state"

run_sync() {
  PM_GIT_SYNC_REPO="$REPO" \
    PM_GIT_SYNC_BUN="$STUB/bun" \
    PM_GIT_SYNC_SYSTEMCTL="$STUB/systemctl" \
    PM_GIT_SYNC_STATE_DIR="$STATE_DIR" \
    bash "$SYNC"
}

push_change() { # <file> <content>
  cd "$SEED" || exit 1
  echo "$2" >"$1"
  git add -A && git commit -qm "change $1" && git push -q origin main
  cd "$SCRATCH" || exit 1
}

OWED_AGG="$STATE_DIR/owed-pm-aggregator.service"
OWED_DISPATCH="$STATE_DIR/owed-pm-dispatch.service"

# --- 0. up-to-date run is a quiet success ------------------------------------
run_sync >/dev/null 2>&1
check "up-to-date run exits 0" test $? -eq 0
check "up-to-date run restarts nothing" log_lacks "restart"

# --- 1. dep step fails: no restart, owed recorded, loud exit ------------------
push_change pm-stack/aggregator/package.json '{"name":"agg","version":"2"}'
: >"$STUB_LOG"
touch "$PM_TEST_BUN_FAIL"
run_sync >/dev/null 2>&1
check "dep-fail run exits non-zero" test $? -ne 0
check "dep step was attempted" log_has "bun install"
check "aggregator NOT restarted after dep failure" log_lacks "restart pm-aggregator"
check "owed record exists for aggregator" test -e "$OWED_AGG"

# --- 2. old==new run retries the owed redeploy --------------------------------
: >"$STUB_LOG"
run_sync >/dev/null 2>&1
check "old==new retry run exits non-zero while dep still fails" test $? -ne 0
check "owed retry re-ran the dep step" log_has "bun install"
check "aggregator still NOT restarted" log_lacks "restart pm-aggregator"
check "owed record still exists" test -e "$OWED_AGG"

# --- 3. dep recovers: retry restarts and clears the owed record ---------------
rm -f "$PM_TEST_BUN_FAIL"
: >"$STUB_LOG"
run_sync >/dev/null 2>&1
check "recovery run exits 0" test $? -eq 0
check "owed retry restarted the aggregator" log_has "restart pm-aggregator"
check "owed record cleared" test ! -e "$OWED_AGG"

# --- 4. restart failure is owed and retried on old==new ------------------------
push_change pm-stack/dispatch/index.ts 'dispatch v2'
touch "$PM_TEST_RESTART_FAIL"
run_sync >/dev/null 2>&1
check "restart-fail run exits non-zero" test $? -ne 0
check "owed record exists for dispatch" test -e "$OWED_DISPATCH"
rm -f "$PM_TEST_RESTART_FAIL"
: >"$STUB_LOG"
run_sync >/dev/null 2>&1
check "old==new retry after restart failure exits 0" test $? -eq 0
check "dispatch restarted by retry" log_has "restart pm-dispatch"
check "dispatch owed record cleared" test ! -e "$OWED_DISPATCH"

if [ "$FAILURES" -ne 0 ]; then
  echo "pm-git-sync.test: $FAILURES check(s) failed"
  exit 1
fi
echo "pm-git-sync.test: all checks passed"
