#!/usr/bin/env bash
# Rendered-unit shape test for issue #8933 (pm + lab Cloudflare tunnels).
#
# Oracle: issue #8933's acceptance criteria. In these NixOS modules
# the systemd unit's command line is the literal `ExecStart` string
# in the module (modulo store paths), so asserting on that string is
# asserting on the rendered unit:
#   - neither tunnel's ExecStart carries a token: no `--token <value>`
#     argument and no TUNNEL_TOKEN expansion anywhere in either module;
#   - both ExecStarts pass the token via `--token-file` pointing at a
#     dedicated token file;
#   - lab-tunnel serves a loopback metrics//ready endpoint on a port
#     distinct from pm-tunnel's 20242 and has a watchdog service +
#     2-minute timer; pm-tunnel's watchdog is untouched;
#   - pm-stack/deploy.sh renders the raw tunnel.token (mode 600) the
#     pm unit now points at.
set -uo pipefail

here="$(dirname -- "$(realpath -- "$0")")"
pm_module="$here/../pm-stack/default.nix"
lab_module="$here/default.nix"
deploy_sh="$here/../../../pm-stack/deploy.sh"
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

for module in "$pm_module" "$lab_module"; do
  expect_absent "$module" 'TUNNEL_TOKEN' \
    "$module: no TUNNEL_TOKEN expansion remains"
  expect_absent "$module" 'run --token ' \
    "$module: no --token <value> argument in ExecStart"
  expect_present "$module" '--token-file' \
    "$module: ExecStart uses --token-file"
done

expect_present "$pm_module" '--token-file ${stackDir}/tunnel.token' \
  "pm-tunnel reads ~/pm-stack/tunnel.token"
expect_present "$lab_module" '--token-file ${stateDir}/tunnel.token' \
  "lab-tunnel reads ~/lab-tunnel/tunnel.token"
expect_present "$lab_module" '--metrics 127.0.0.1:${toString tunnelMetricsPort}' \
  "lab-tunnel serves loopback metrics//ready"
expect_present "$lab_module" 'tunnelMetricsPort = 20243' \
  "lab metrics port 20243 is distinct from pm-tunnel's 20242"
expect_present "$lab_module" 'lab-tunnel-watchdog' \
  "lab-tunnel watchdog service is defined"
expect_present "$lab_module" 'OnUnitActiveSec = "2min"' \
  "lab-tunnel watchdog runs on a 2-minute timer"
expect_present "$pm_module" 'pm-tunnel-watchdog' \
  "pm-tunnel watchdog is preserved"

expect_present "$deploy_sh" '> tunnel.token' \
  "deploy.sh renders the raw tunnel.token"
expect_present "$deploy_sh" 'chmod 600 tunnel.token' \
  "deploy.sh renders tunnel.token at mode 600"

if [[ $failures -gt 0 ]]; then
  echo "tunnel-units.test.sh: $failures check(s) failed" >&2
  exit 1
fi
echo "tunnel-units.test.sh: all checks passed"
