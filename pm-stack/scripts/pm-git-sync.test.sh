#!/usr/bin/env bash
# pm-git-sync.test.sh — harness for pm-git-sync.sh deployed-head and
# owed-redeploy handling (#2391). Builds a throwaway origin + clone in a
# temp dir, stubs systemctl and bun via the PM_GIT_SYNC_* env seams, and drives:
#   1. hand-pulled and self-pulled HEADs both trigger subtree redeploys
#   2. dep step fails -> unit NOT restarted, owed record exists,
#      run exits non-zero
#   3. next run with HEAD unmoved retries the owed
#      redeploy; still failing -> still owed, still no restart
#   4. dep step recovers -> retry restarts the unit, owed cleared,
#      run exits 0
#   5. restart itself fails -> owed recorded; next old==new run
#      retries and clears it
#   6. unusable or corrupt baseline state fails safely
#   7. partial batch then revert: a unit that restarted successfully
#      at the partial head is rolled back when a later commit reverts
#      its subtree, even though the baseline-range diff is empty
#      (per-unit deployed tracking, #7132)
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
DEPLOYED_HEAD="$STATE_DIR/deployed-head"
deployed_head_is() { test -f "$DEPLOYED_HEAD" && test "$(cat "$DEPLOYED_HEAD")" = "$1"; }

run_sync() {
  PM_GIT_SYNC_REPO="$REPO" \
    PM_GIT_SYNC_BUN="$STUB/bun" \
    PM_GIT_SYNC_SYSTEMCTL="$STUB/systemctl" \
    PM_GIT_SYNC_STATE_DIR="${TEST_STATE_DIR:-$STATE_DIR}" \
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

# --- 0. missing baseline bootstraps even after a hand pull --------------------
push_change pm-stack/aggregator/index.ts 'agg pulled before first sync'
git -C "$REPO" pull --ff-only origin main >/dev/null 2>&1
run_sync >/dev/null 2>&1
check "first run after hand pull exits 0" test $? -eq 0
check "baseline bootstrap redeploys aggregator" log_has "restart pm-aggregator"
check "baseline bootstrap redeploys projector" log_has "restart pm-projector"
check "baseline bootstrap redeploys dispatch" log_has "restart pm-dispatch"
check "successful bootstrap records deployed HEAD" deployed_head_is "$(git -C "$REPO" rev-parse HEAD)"
: >"$STUB_LOG"
run_sync >/dev/null 2>&1
check "repeat run after baseline bootstrap exits 0" test $? -eq 0
check "repeat run after baseline bootstrap restarts nothing" log_lacks "restart"

# --- 0b. a later hand-pulled HEAD still owes its subtree redeploy -------------
push_change pm-stack/dispatch/index.ts 'dispatch hand-pulled'
git -C "$REPO" pull --ff-only origin main >/dev/null 2>&1
HAND_PULLED_HEAD="$(git -C "$REPO" rev-parse HEAD)"
: >"$STUB_LOG"
run_sync >/dev/null 2>&1
check "hand-pulled HEAD run exits 0" test $? -eq 0
check "hand-pulled subtree change redeploys dispatch" log_has "restart pm-dispatch"
check "successful hand-pulled deploy advances baseline" deployed_head_is "$HAND_PULLED_HEAD"
: >"$STUB_LOG"
run_sync >/dev/null 2>&1
check "repeat run after hand-pulled deploy exits 0" test $? -eq 0
check "repeat run does not redeploy the same HEAD" log_lacks "restart"

# --- 0c. a self-pulled HEAD still follows the existing redeploy path ----------
push_change pm-stack/dispatch/index.ts 'dispatch self-pulled'
: >"$STUB_LOG"
run_sync >/dev/null 2>&1
check "self-pulled HEAD run exits 0" test $? -eq 0
check "self-pulled subtree change redeploys dispatch" log_has "restart pm-dispatch"
check "successful self-pulled deploy advances baseline" deployed_head_is "$(git -C "$REPO" rev-parse HEAD)"

# --- 1. dep step fails: no restart, owed recorded, loud exit ------------------
push_change pm-stack/aggregator/package.json '{"name":"agg","version":"2"}'
: >"$STUB_LOG"
BASELINE_BEFORE_FAILURE="$(cat "$DEPLOYED_HEAD")"
touch "$PM_TEST_BUN_FAIL"
run_sync >/dev/null 2>&1
check "dep-fail run exits non-zero" test $? -ne 0
check "dep step was attempted" log_has "bun install"
check "aggregator NOT restarted after dep failure" log_lacks "restart pm-aggregator"
check "owed record exists for aggregator" test -e "$OWED_AGG"
check "failed redeploy does not advance deployed baseline" deployed_head_is "$BASELINE_BEFORE_FAILURE"

# --- 2. old==new run retries the owed redeploy --------------------------------
: >"$STUB_LOG"
run_sync >/dev/null 2>&1
check "old==new retry run exits non-zero while dep still fails" test $? -ne 0
check "owed retry re-ran the dep step" log_has "bun install"
check "aggregator still NOT restarted" log_lacks "restart pm-aggregator"
check "owed record still exists" test -e "$OWED_AGG"

# --- 2b. a newer source-only pull must not skip the owed dep step -------------
# (Codex P1 on PR 2860: the merged obligation survives HEAD moving.)
push_change pm-stack/aggregator/index.ts 'agg v2'
: >"$STUB_LOG"
run_sync >/dev/null 2>&1
check "source-only pull with owed dep exits non-zero" test $? -ne 0
check "owed dep step merged into the new attempt" log_has "bun install"
check "aggregator NOT restarted past the owed dep step" log_lacks "restart pm-aggregator"
check "owed record survives the source-only pull" test -e "$OWED_AGG"

# --- 3. dep recovers: retry restarts and clears the owed record ---------------
rm -f "$PM_TEST_BUN_FAIL"
: >"$STUB_LOG"
run_sync >/dev/null 2>&1
check "recovery run exits 0" test $? -eq 0
check "owed retry restarted the aggregator" log_has "restart pm-aggregator"
check "owed record cleared" test ! -e "$OWED_AGG"
check "recovery advances deployed baseline" deployed_head_is "$(git -C "$REPO" rev-parse HEAD)"

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
check "restart recovery advances deployed baseline" deployed_head_is "$(git -C "$REPO" rev-parse HEAD)"

# --- 4b. partial batch then revert: the successful unit rolls back ------------
push_change pm-stack/aggregator/index.ts 'agg v3-partial'
push_change pm-stack/projector/package.json '{"name":"proj","version":"2"}'
touch "$PM_TEST_PROJ_DEP_FAIL"
: >"$STUB_LOG"
run_sync >/dev/null 2>&1
check "partial-batch run exits non-zero" test $? -ne 0
check "aggregator restarted at the partial head" log_has "restart pm-aggregator"
check "projector NOT restarted past its failing dep step" log_lacks "restart pm-projector"
rm -f "$PM_TEST_PROJ_DEP_FAIL"
push_change pm-stack/aggregator/index.ts 'agg v2'
push_change pm-stack/projector/package.json '{"name":"proj","version":"3"}'
: >"$STUB_LOG"
run_sync >/dev/null 2>&1
check "revert run exits 0" test $? -eq 0
check "aggregator rolled back by the revert" log_has "restart pm-aggregator"
check "projector redeployed by the revert run" log_has "restart pm-projector"
check "revert run advances deployed baseline" deployed_head_is "$(git -C "$REPO" rev-parse HEAD)"
check "aggregator deployed record tracks the revert head" test "$(cat "$STATE_DIR/deployed-pm-aggregator.service")" = "$(git -C "$REPO" rev-parse HEAD)"

# --- 5. an unusable state path must fail before treating HEAD as deployed -----
touch "$SCRATCH/blocker"
: >"$STUB_LOG"
TEST_STATE_DIR="$SCRATCH/blocker/state" run_sync >/dev/null 2>&1
check "run with unusable state dir cannot initialize deployed baseline" test $? -ne 0
check "unwritable baseline storage prevents redeploys" log_lacks "restart"

# A corrupt marker must fail safely instead of silently accepting the
# current HEAD as deployed and dropping the changes represented by it.
printf '%s\n' 'not-a-commit' >"$DEPLOYED_HEAD"
: >"$STUB_LOG"
run_sync >/dev/null 2>&1
check "invalid deployed-head baseline fails the run" test $? -ne 0
check "invalid baseline does not trigger a guessed redeploy" log_lacks "restart"
printf '%s\n' 'HEAD' >"$DEPLOYED_HEAD"
run_sync >/dev/null 2>&1
check "symbolic ref is rejected as a deployed-head baseline" test $? -ne 0

if [ "$FAILURES" -ne 0 ]; then
  echo "pm-git-sync.test: $FAILURES check(s) failed"
  exit 1
fi
echo "pm-git-sync.test: all checks passed"
