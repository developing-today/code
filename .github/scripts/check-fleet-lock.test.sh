#!/usr/bin/env bash
# Contract tests for check-fleet-lock.sh (#10363).
# Builds a path-only fixture shaped like fleet: a workspace member that
# path-depends on a crate outside the workspace. The guard must pass a
# fresh lock, and fail once the outside crate gains a dependency the
# committed lock does not know. Path-only, so it runs offline.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHECK="$SCRIPT_DIR/check-fleet-lock.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
export FLEET_LOCK_CARGO_FLAGS=--offline
export CARGO_TARGET_DIR="$TMP/target"

failures=0
note_fail() {
  echo "FAIL: $1"
  failures=$((failures + 1))
}

crate() { # crate <dir> <name> [dependency lines...]
  local dir=$1 name=$2
  shift 2
  mkdir -p "$dir/src"
  {
    printf '[package]\nname = "%s"\nversion = "0.1.0"\nedition = "2021"\n\n[dependencies]\n' "$name"
    printf '%s\n' "$@"
  } >"$dir/Cargo.toml"
  : >"$dir/src/lib.rs"
}

# pkgs/{fleet,atlas}: fleet's member reaches into atlas by path.
crate "$TMP/pkgs/atlas/core" atlas-core
crate "$TMP/pkgs/atlas/extra" atlas-extra
crate "$TMP/pkgs/fleet/crates/search" fleet-search 'atlas-core = { path = "../../../atlas/core" }'
printf '[workspace]\nmembers = ["crates/search"]\nresolver = "2"\n' >"$TMP/pkgs/fleet/Cargo.toml"
MANIFEST="$TMP/pkgs/fleet/Cargo.toml"
cargo generate-lockfile --offline --manifest-path "$MANIFEST" >/dev/null 2>&1

if bash "$CHECK" "$MANIFEST" >/dev/null 2>&1; then
  echo "ok: a fresh lock passes"
else
  note_fail "a fresh lock should pass"
fi

# atlas-core gains a dependency; nothing under pkgs/fleet changes.
crate "$TMP/pkgs/atlas/core" atlas-core 'atlas-extra = { path = "../extra" }'
if out=$(bash "$CHECK" "$MANIFEST" 2>&1); then
  note_fail "a lock missing the new atlas edge should fail"
elif ! printf '%s' "$out" | grep -qF 'is stale'; then
  note_fail "the stale-lock failure should name the stale lock"
  printf '%s\n' "$out" | sed 's/^/       /'
else
  echo "ok: a lock missing the new atlas edge fails, naming the stale lock"
fi

# The fix the guard names makes it pass again.
cargo update --workspace --offline --manifest-path "$MANIFEST" >/dev/null 2>&1
if bash "$CHECK" "$MANIFEST" >/dev/null 2>&1; then
  echo "ok: the named fix makes it pass"
else
  note_fail "the lock should pass after cargo update --workspace"
fi

if bash "$CHECK" "$TMP/missing/Cargo.toml" >/dev/null 2>&1; then
  note_fail "a missing manifest should fail"
else
  echo "ok: a missing manifest fails"
fi

if [ "$failures" -ne 0 ]; then
  echo "check-fleet-lock.test: $failures failure(s)"
  exit 1
fi
echo "check-fleet-lock.test: OK"
