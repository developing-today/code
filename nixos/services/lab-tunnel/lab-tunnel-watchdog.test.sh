#!/usr/bin/env bash
# Test for nixos/services/lab-tunnel/lab-tunnel-watchdog.sh (#8933).
#
# Oracle: pm-tunnel-watchdog semantics (code#6992), which the lab
# watchdog mirrors exactly:
#   - /ready success: counter resets to 0, never a restart.
#   - /ready failure (HTTP error or probe/connection error alike):
#     the consecutive-failure counter increments by one per run.
#   - At 3 consecutive failures: exactly one
#     `systemctl --user restart lab-tunnel.service`, counter resets.
#
# curl and systemctl are stubs; the watchdog under test is this
# repo's script with its @curl@/@systemctl@/@readyUrl@ placeholders
# substituted by the stubs, exactly as the NixOS module substitutes
# the real paths. No network, no systemd, no credential is touched.
set -uo pipefail

here="$(dirname -- "$(realpath -- "$0")")"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/stubbin" "$work/home"

cat >"$work/stubbin/curl" <<'STUB'
#!/usr/bin/env bash
# Behavior is driven by $CURL_MODE_FILE: ok | httpfail | connerr
mode="$(cat "$CURL_MODE_FILE")"
case "$mode" in
ok) exit 0 ;;
httpfail) exit 22 ;;
connerr) exit 7 ;;
*) exit 99 ;;
esac
STUB

cat >"$work/stubbin/systemctl" <<'STUB'
#!/usr/bin/env bash
echo "$@" >>"$RESTART_LOG"
exit 0
STUB
chmod +x "$work/stubbin/curl" "$work/stubbin/systemctl"

export CURL_MODE_FILE="$work/curl-mode"
export RESTART_LOG="$work/restarts.log"
export HOME="$work/home"

sed -e "s|@curl@|$work/stubbin/curl|g" \
  -e "s|@systemctl@|$work/stubbin/systemctl|g" \
  -e "s|@readyUrl@|http://127.0.0.1:9/ready|g" \
  "$here/lab-tunnel-watchdog.sh" >"$work/watchdog.sh"

countFile="$HOME/.local/state/lab-tunnel-watchdog/consecutive-failures"
failures=0

check() { # check <description> <expected> <actual>
  if [[ "$2" == "$3" ]]; then
    echo "PASS: $1"
  else
    echo "FAIL: $1 (expected [$2], got [$3])"
    failures=$((failures + 1))
  fi
}

counter() { cat "$countFile" 2>/dev/null || echo "missing"; }
restarts() { grep -c '' "$RESTART_LOG" 2>/dev/null || echo 0; }
run_watchdog() { bash "$work/watchdog.sh" >/dev/null 2>&1; }

# 1. Healthy: /ready succeeds — counter 0, no restart.
echo ok >"$CURL_MODE_FILE"
run_watchdog
check "healthy run leaves counter at 0" "0" "$(counter)"
check "healthy run never restarts" "0" "$(restarts)"

# 2. Degraded: HTTP failure on /ready counts up; restart at 3.
echo httpfail >"$CURL_MODE_FILE"
run_watchdog
check "first failure counts 1" "1" "$(counter)"
check "first failure does not restart" "0" "$(restarts)"
run_watchdog
check "second failure counts 2" "2" "$(counter)"
run_watchdog
check "third failure resets counter" "0" "$(counter)"
check "third failure restarts exactly once" "1" "$(restarts)"
check "restart targets lab-tunnel.service" \
  "--user restart lab-tunnel.service" "$(cat "$RESTART_LOG")"

# 3. Probe error (connection refused) counts as a failure too.
echo connerr >"$CURL_MODE_FILE"
run_watchdog
check "probe error counts as a failure" "1" "$(counter)"
check "probe error alone does not restart" "1" "$(restarts)"

# 4. A success after failures resets the counter, no restart.
echo httpfail >"$CURL_MODE_FILE"
run_watchdog
check "failure after probe error counts 2" "2" "$(counter)"
echo ok >"$CURL_MODE_FILE"
run_watchdog
check "success resets the counter" "0" "$(counter)"
check "success does not restart" "1" "$(restarts)"

if [[ $failures -gt 0 ]]; then
  echo "lab-tunnel-watchdog.test.sh: $failures check(s) failed" >&2
  exit 1
fi
echo "lab-tunnel-watchdog.test.sh: all checks passed"
