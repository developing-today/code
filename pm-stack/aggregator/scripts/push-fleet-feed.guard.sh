#!/usr/bin/env bash
# Grep-level guard for push-fleet-feed.sh (issue #948).
# Fails if the proxy credential is ever handed to a process as an
# argument again, or if SSH host-key verification is disabled again.
# Forbidden patterns are assembled from fragments so this guard file
# itself does not contain them literally.
set -euo pipefail
cd "$(dirname "$0")"
TARGET="push-fleet-feed.sh"
fail=0

check_absent() {
  local pat="$1" why="$2"
  if grep -nE "$pat" "$TARGET" >/dev/null; then
    echo "guard: FORBIDDEN pattern present ($why): $pat" >&2
    grep -nE "$pat" "$TARGET" >&2 || true
    fail=1
  fi
}

check_present() {
  local pat="$1" why="$2"
  if ! grep -qE "$pat" "$TARGET"; then
    echo "guard: REQUIRED pattern missing ($why): $pat" >&2
    fail=1
  fi
}

check_absent "proxy""auth" "proxy credential must never be an argv token (e.g. to socat)"
check_absent "Proxy""Command" "no proxy helper subprocess; the tunnel is in-process"
check_absent "StrictHostKeyChecking=no" "host-key verification must stay on"
check_absent "UserKnownHostsFile=/dev/null" "host keys must come from the pinned file"
check_absent "so""cat" "socat carried the credential in argv; do not reintroduce it"

check_present "StrictHostKeyChecking=yes" "host-key verification is required"
check_present "laptop_known_hosts" "the pinned known_hosts file must be used"
check_present "Proxy-Authorization" "credential goes on the proxy wire only"

if [ ! -s "laptop_known_hosts" ]; then
  echo "guard: laptop_known_hosts is missing or empty" >&2
  fail=1
fi

if [ "$fail" -ne 0 ]; then
  exit 1
fi
echo "guard: push-fleet-feed.sh OK (no argv credential, host key pinned)"
