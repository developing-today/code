#!/usr/bin/env bash
# Rendered-unit shape test for issue #8940 (PM app readiness checks).
#
# Oracle: issue #8940's acceptance criteria. In these NixOS modules
# the systemd unit's fields are the literal strings in the module, so
# asserting on them is asserting on the rendered units:
#   - a pm-apps-health oneshot service + a timer firing it every
#     minute (the probe interval), first run 2 minutes after boot;
#   - the probe's baked app list is exactly the three PM apps'
#     loopback HTTP endpoints (Grist 8484, Pad 7777,
#     ready-for-agent 6056);
#   - the probe's state lives in a systemd StateDirectory;
#   - failure semantics ship with the probe: per-app first-dark
#     stamps, the UNHEALTHY report, and a 5-second probe timeout;
#   - report-only: the probe contains no systemctl call, so it can
#     never restart an app;
#   - the pre-existing containers are untouched: same loopback +
#     tailnet port publishes and the same persistent volumes.
set -uo pipefail

here="$(dirname -- "$(realpath -- "$0")")"
module="$here/default.nix"
probe="$here/pm-apps-health.sh"
failures=0

expect_present() { # expect_present <file> <pattern> <description>
  if grep -qF -- "$2" "$1"; then
    echo "PASS: $3"
  else
    echo "FAIL: $3 (pattern not found in $1: $2)"
    failures=$((failures + 1))
  fi
}

expect_absent() { # expect_absent <file> <pattern> <description>
  if grep -qF -- "$2" "$1"; then
    echo "FAIL: $3 (forbidden pattern present in $1: $2)"
    failures=$((failures + 1))
  else
    echo "PASS: $3"
  fi
}

expect_present "$module" 'systemd.services.pm-apps-health' \
  "readiness probe service is defined"
expect_present "$module" 'Type = "oneshot"' \
  "probe service is a oneshot (its failure state is the alert)"
expect_present "$module" 'StateDirectory = "pm-apps-health"' \
  "probe state lives in a systemd StateDirectory"
expect_present "$module" 'systemd.timers.pm-apps-health' \
  "readiness probe timer is defined"
expect_present "$module" 'OnUnitActiveSec = "1min"' \
  "probe interval is 1 minute"
expect_present "$module" 'OnBootSec = "2min"' \
  "first probe runs 2 minutes after boot"
expect_present "$module" 'grist=http://127.0.0.1:8484/' \
  "grist endpoint probed on loopback 8484"
expect_present "$module" 'pad=http://127.0.0.1:7777/' \
  "pad endpoint probed on loopback 7777"
expect_present "$module" 'ready-for-agent=http://127.0.0.1:6056/' \
  "ready-for-agent endpoint probed on loopback 6056"
expect_present "$module" 'healthGraceSeconds = 180' \
  "start-period/grace is 180 seconds"

expect_present "$probe" 'first-dark-' \
  "probe tracks per-app first-dark stamps (start-period/grace)"
expect_present "$probe" 'UNHEALTHY' \
  "probe reports an app UNHEALTHY past its grace"
expect_present "$probe" '-m 5' \
  "probe timeout is 5 seconds per endpoint"
expect_absent "$probe" 'systemctl' \
  "probe is report-only: no systemctl, so it can never restart an app"

expect_present "$module" '"127.0.0.1:8484:8484"' \
  "grist loopback publish unchanged"
expect_present "$module" '"127.0.0.1:7777:7777"' \
  "pad loopback publish unchanged"
expect_present "$module" '"grist_grist_data:/persist"' \
  "grist persistent volume unchanged"
expect_present "$module" '"pad_pad_data:/data"' \
  "pad persistent volume unchanged"

if [ "$failures" -gt 0 ]; then
  echo "pm-apps-units.test.sh: $failures check(s) failed" >&2
  exit 1
fi
echo "pm-apps-units.test.sh: all checks passed"
