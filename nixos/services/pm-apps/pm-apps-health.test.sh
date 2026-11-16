#!/usr/bin/env bash
# Behavior test for nixos/services/pm-apps/pm-apps-health.sh (#8940).
#
# Oracle: issue #8940's acceptance criteria, as designed in the probe
# header:
#   - an app whose HTTP endpoint answers at all is READY (any status;
#     the stub's "ok" is curl exit 0, "fail" is exit 7 refused);
#   - the first dark probe starts the app's start-period/grace and
#     still exits 0 (startup is legitimately dark);
#   - an app dark continuously for >= the grace (180s) makes the
#     probe exit 1 and name the app UNHEALTHY on stderr;
#   - a ready probe clears the app's stamp and the probe exits 0
#     again (recovery needs no restart and no manual reset);
#   - apps are tracked independently: one app beyond grace fails the
#     probe while the others stay ready and stampless.
#
# curl is a stub driven by per-port mode files; the probe under test
# is this repo's script with its @curl@/@stateDir@/@graceSeconds@/
# @apps@ placeholders substituted exactly as the NixOS module
# substitutes the real values. No network, no systemd, no container
# is touched. Time is driven through the PM_APPS_HEALTH_NOW hook.
set -uo pipefail

here="$(dirname -- "$(realpath -- "$0")")"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/stubbin" "$work/state" "$work/modes"

cat >"$work/stubbin/curl" <<'STUB'
#!/usr/bin/env bash
# Mode per port: $CURL_STUB_DIR/<port>.mode holds ok|fail (default ok).
url="${@: -1}"
port="$(printf '%s' "$url" | sed -n 's|.*:\([0-9][0-9]*\)/.*|\1|p')"
mode="ok"
if [ -f "$CURL_STUB_DIR/$port.mode" ]; then
  mode="$(cat "$CURL_STUB_DIR/$port.mode")"
fi
case "$mode" in
ok) exit 0 ;;
*) exit 7 ;;
esac
STUB
chmod +x "$work/stubbin/curl"
export CURL_STUB_DIR="$work/modes"

apps="grist=http://127.0.0.1:8484/ pad=http://127.0.0.1:7777/ ready-for-agent=http://127.0.0.1:6056/"
sed \
  -e "s|@curl@|$work/stubbin/curl|g" \
  -e "s|@stateDir@|$work/state|g" \
  -e "s|@graceSeconds@|180|g" \
  -e "s|@apps@|$apps|g" \
  "$here/pm-apps-health.sh" >"$work/probe.sh"

failures=0
run_probe() { # run_probe <now-epoch>; sets rc + out/err files
  PM_APPS_HEALTH_NOW="$1" bash "$work/probe.sh" >"$work/out" 2>"$work/err"
  rc=$?
}
expect_rc() { # expect_rc <want> <description>
  if [ "$rc" -eq "$1" ]; then
    echo "PASS: $2"
  else
    echo "FAIL: $2 (probe exited $rc, wanted $1; stderr: $(cat "$work/err"))"
    failures=$((failures + 1))
  fi
}
expect_stamp() { # expect_stamp <app> <present|absent> <description>
  if [ "$2" = present ] && [ -f "$work/state/first-dark-$1" ]; then
    echo "PASS: $3"
  elif [ "$2" = absent ] && [ ! -f "$work/state/first-dark-$1" ]; then
    echo "PASS: $3"
  else
    echo "FAIL: $3 (stamp first-dark-$1 should be $2)"
    failures=$((failures + 1))
  fi
}

# 1. All apps ready: exit 0, no stamps.
run_probe 1000
expect_rc 0 "all ready -> exit 0"
expect_stamp pad absent "all ready -> no pad stamp"

# 2. Pad goes dark (port 7777 refuses): first dark probe starts the
# grace and still exits 0.
echo fail >"$work/modes/7777.mode"
run_probe 1000
expect_rc 0 "pad first dark -> exit 0 (start-period/grace)"
expect_stamp pad present "pad first dark -> stamp recorded"
expect_stamp grist absent "pad dark -> grist unaffected"

# 3. Pad still dark inside the grace (age 100s < 180s): exit 0.
run_probe 1100
expect_rc 0 "pad dark 100s (< grace) -> exit 0"

# 4. Pad dark for exactly the grace (age 180s): exit 1, app named
# UNHEALTHY on stderr.
run_probe 1180
expect_rc 1 "pad dark 180s (>= grace) -> exit 1"
if grep -q "pad UNHEALTHY" "$work/err"; then
  echo "PASS: pad named UNHEALTHY on stderr"
else
  echo "FAIL: pad not named UNHEALTHY on stderr (got: $(cat "$work/err"))"
  failures=$((failures + 1))
fi

# 5. Pad answers again: stamp cleared, exit 0 (recovery).
echo ok >"$work/modes/7777.mode"
run_probe 1200
expect_rc 0 "pad ready again -> exit 0"
expect_stamp pad absent "pad ready again -> stamp cleared"

# 6. Independence: grist dark beyond grace fails the probe while pad
# and ready-for-agent stay ready.
echo fail >"$work/modes/8484.mode"
run_probe 2000
expect_rc 0 "grist first dark -> exit 0"
run_probe 2200
expect_rc 1 "grist dark beyond grace -> exit 1"
expect_stamp pad absent "grist unhealthy -> pad still stampless"

if [ "$failures" -gt 0 ]; then
  echo "pm-apps-health.test.sh: $failures check(s) failed" >&2
  exit 1
fi
echo "pm-apps-health.test.sh: all checks passed"
