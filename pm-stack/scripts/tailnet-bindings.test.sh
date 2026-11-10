#!/usr/bin/env bash
# Regression tests for #781: Pad and Grist tailnet bindings.
#
# The compose files must take the tailnet bind address from
# PM_STACK_TAILNET_IP (rendered into .env by deploy.sh), never from a
# hard-coded host address: unset renders loopback-only, a set address
# renders loopback + that address, and neither case ever renders a
# public (0.0.0.0) binding. GRIST_DEFAULT_EMAIL likewise comes from
# the environment with the documented operator default.
#
# Requires docker compose (config rendering only; nothing is started).
set -euo pipefail

cd "$(dirname "$0")/.."
STACK_DIR="$PWD"

if ! docker compose version >/dev/null 2>&1; then
  echo "SKIP: docker compose not available"
  exit 0
fi

fail=0
note() { echo "$1"; }
bad() {
  echo "FAIL: $1"
  fail=1
}

# --- static: no host address is hard-coded anymore -------------------
if grep -n '100\.99\.113\.103' pad/compose.yml grist/compose.yml README.md; then
  bad "a tracked file still hard-codes the old host's tailnet IP"
fi
grep -q 'PM_STACK_TAILNET_IP' pad/compose.yml || bad "pad/compose.yml does not use PM_STACK_TAILNET_IP"
grep -q 'PM_STACK_TAILNET_IP' grist/compose.yml || bad "grist/compose.yml does not use PM_STACK_TAILNET_IP"
grep -q 'PM_STACK_TAILNET_IP' pad/deploy.sh || bad "pad/deploy.sh does not render PM_STACK_TAILNET_IP"
grep -q 'PM_STACK_TAILNET_IP' grist/deploy.sh || bad "grist/deploy.sh does not render PM_STACK_TAILNET_IP"

# --- render harness ----------------------------------------------------
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
cp pad/compose.yml "$TMP/pad.yml"
cp grist/compose.yml "$TMP/grist.yml"
: >"$TMP/.env"

render() { # <compose-file> [VAR=value ...]
  local file="$1"
  shift
  (cd "$TMP" && env -i PATH="$PATH" HOME="$HOME" "$@" \
    docker compose -f "$file" config 2>/dev/null)
}

host_ips() { grep 'host_ip:' | awk '{print $2}' | sort -u; }

for svc in pad grist; do
  # Unset: loopback only, exactly one binding.
  out="$(render "$svc.yml")"
  ips="$(printf '%s\n' "$out" | host_ips)"
  [[ $ips == "127.0.0.1" ]] || bad "$svc unset PM_STACK_TAILNET_IP: host_ips=[$ips], want [127.0.0.1]"
  [[ "$(printf '%s\n' "$out" | grep -c 'host_ip:')" == "1" ]] || bad "$svc unset: want exactly 1 port binding"
  # Another host's address: loopback + that address, nothing else.
  out="$(render "$svc.yml" PM_STACK_TAILNET_IP=100.64.1.9)"
  ips="$(printf '%s\n' "$out" | host_ips)"
  [[ $ips == "100.64.1.9
127.0.0.1" ]] || bad "$svc PM_STACK_TAILNET_IP=100.64.1.9: host_ips=[$ips]"
  # The current host's address: renders identically to the old
  # hard-coded behaviour (equivalence with the pre-#781 deployment).
  out="$(render "$svc.yml" PM_STACK_TAILNET_IP=100.99.113.103)"
  ips="$(printf '%s\n' "$out" | host_ips)"
  [[ $ips == "100.99.113.103
127.0.0.1" ]] || bad "$svc PM_STACK_TAILNET_IP=100.99.113.103: host_ips=[$ips]"
  # Never a public binding, in any case above.
  if render "$svc.yml" | grep -q 'host_ip: 0.0.0.0'; then
    bad "$svc renders a public 0.0.0.0 binding"
  fi
done

# --- Grist default email: env wins, documented default otherwise ------
out="$(render grist.yml)"
printf '%s\n' "$out" | grep -q 'GRIST_DEFAULT_EMAIL: drewrypope@gmail.com' ||
  bad "grist unset GRIST_DEFAULT_EMAIL: documented default not rendered"
out="$(render grist.yml GRIST_DEFAULT_EMAIL=ops@example.net)"
printf '%s\n' "$out" | grep -q 'GRIST_DEFAULT_EMAIL: ops@example.net' ||
  bad "grist GRIST_DEFAULT_EMAIL override not rendered"

if [[ $fail -ne 0 ]]; then
  note "tailnet-bindings.test.sh: FAILED"
  exit 1
fi
note "tailnet-bindings.test.sh: all checks passed"
