#!/usr/bin/env bash
# Exercises the real oc-password-sync.sh (code#11140) in dry-run mode
# against fixture service files, with systemctl stubbed on PATH and a
# stand-in "OpenChamber" process whose /proc environ holds the
# password it was started with. Oracle: the script's log lines, its
# "drift detected" output, and whether it asked systemctl to restart.
#
#   1. a file that is unparseable at trigger time but valid moments
#      later is still read (bounded parse retry);
#   2. a file unparseable on every attempt is skipped with exit 0;
#   3. a periodic run never starts an OpenChamber that is down;
#   4. a periodic run detects drift in a running OpenChamber;
#   5. a periodic run is a no-op when the running process is in sync;
#   6. a path-triggered run with OpenChamber down keeps its old
#      behavior (drift against the last-synced hash).
set -uo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$SELF_DIR/oc-password-sync.sh"
[ -f "$SCRIPT" ] || {
  echo "FAIL: script not found at $SCRIPT" >&2
  exit 1
}

TMP_ROOT="$(mktemp -d)"
STANDIN_PID=""
cleanup() {
  [ -n "$STANDIN_PID" ] && kill "$STANDIN_PID" 2>/dev/null
  rm -rf "$TMP_ROOT"
}
trap cleanup EXIT
failures=0
fail() {
  echo "FAIL: $1" >&2
  failures=$((failures + 1))
}

STUB_BIN="$TMP_ROOT/bin"
mkdir -p "$STUB_BIN"
cat >"$STUB_BIN/systemctl" <<'STUB'
#!/usr/bin/env bash
case " $* " in
  *" show "*) echo "${FAKE_MAINPID:-0}" ;;
  *" restart "*) echo restart >>"$FAKE_CALLS" ;;
esac
exit 0
STUB
chmod +x "$STUB_BIN/systemctl"

JSON="$TMP_ROOT/service-prod.json"
export FAKE_CALLS="$TMP_ROOT/calls"

# Run the script with a fresh HOME; prints its stdout, keeps the log.
run_sync() {
  rm -rf "$TMP_ROOT/home" "$FAKE_CALLS"
  mkdir -p "$TMP_ROOT/home"
  HOME="$TMP_ROOT/home" PATH="$STUB_BIN:$PATH" OC_SERVICE_JSON="$JSON" \
    OC_DRY_RUN=1 OC_PARSE_RETRY_DELAY="${DELAY:-0}" "$@" bash "$SCRIPT"
}
LOGF="$TMP_ROOT/home/.local/state/openchamber/password-sync.log"
STATEF="$TMP_ROOT/home/.local/state/openchamber/password-sync.state"
hash_of() { printf %s "$1" | sha256sum | cut -d" " -f1; }

# 1. mid-write at trigger time, valid a moment later
printf '{"passw' >"$JSON"
(sleep 1 && printf '{"password":"new-secret"}' >"$JSON") &
out=$(DELAY=1 FAKE_MAINPID=0 run_sync env OC_PARSE_ATTEMPTS=5)
wait
[ "$out" = "drift detected (dry run)" ] || fail "1: retried parse should reach drift, got: $out"
grep -q "could not parse" "$LOGF" && fail "1: should not give up while retries remain"

# 2. unparseable on every attempt
printf 'not json' >"$JSON"
out=$(FAKE_MAINPID=0 run_sync env OC_PARSE_ATTEMPTS=3)
rc=$?
[ "$rc" -eq 0 ] || fail "2: exit should be 0, got $rc"
grep -q "after 3 attempts" "$LOGF" || fail "2: log should name the attempts: $(cat "$LOGF")"
[ -z "$out" ] || fail "2: no drift output expected, got: $out"

# 3. periodic run, OpenChamber down: never starts it
printf '{"password":"new-secret"}' >"$JSON"
out=$(FAKE_MAINPID=0 run_sync env OC_SYNC_PERIODIC=1)
[ -z "$out" ] || fail "3: periodic run must not act on a stopped OpenChamber, got: $out"
grep -q "periodic: openchamber not running" "$LOGF" || fail "3: log should say why: $(cat "$LOGF")"
[ ! -e "$FAKE_CALLS" ] || fail "3: systemctl restart must not be called"

# A stand-in OpenChamber process holding the OLD password.
env OPENCODE_PASSWORD=old-secret sleep 300 &
STANDIN_PID=$!
sleep 0.2

# 4. periodic run, running OpenChamber holds a stale password
out=$(FAKE_MAINPID=$STANDIN_PID run_sync env OC_SYNC_PERIODIC=1)
[ "$out" = "drift detected (dry run)" ] || fail "4: periodic run should detect drift, got: $out"

# 5. periodic run, running OpenChamber already in sync
printf '{"password":"old-secret"}' >"$JSON"
out=$(FAKE_MAINPID=$STANDIN_PID run_sync env OC_SYNC_PERIODIC=1)
[ -z "$out" ] || fail "5: in-sync run should take no action, got: $out"
grep -q "in sync; no action" "$LOGF" || fail "5: log should say in sync: $(cat "$LOGF")"
[ "$(cat "$STATEF" 2>/dev/null)" = "$(hash_of old-secret)" ] || fail "5: state should record the synced hash"

# 6. path-triggered run, OpenChamber down: old behavior kept
printf '{"password":"new-secret"}' >"$JSON"
out=$(FAKE_MAINPID=0 run_sync env)
[ "$out" = "drift detected (dry run)" ] || fail "6: path run should still judge drift by state, got: $out"

if [ "$failures" -gt 0 ]; then
  echo "$failures failure(s)" >&2
  exit 1
fi
echo "oc-password-sync: all checks passed"
