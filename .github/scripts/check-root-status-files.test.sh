#!/usr/bin/env bash
# Contract tests for check-root-status-files.sh (#12136).
# Builds fixture roots in a temp dir: a clean root passes (including
# a *COMPLETE* file nested in a subdirectory - only the root is
# guarded), and a root *COMPLETE* / *SUMMARY* file fails, named.
# Path-only, so it runs offline.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHECK="$SCRIPT_DIR/check-root-status-files.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

failures=0
note_fail() {
  echo "FAIL: $1"
  failures=$((failures + 1))
}

# A clean root passes: ordinary root files, plus a *COMPLETE* file
# nested in a subdirectory (out of scope - the guard is root-only).
CLEAN="$TMP/clean"
mkdir -p "$CLEAN/doc"
: >"$CLEAN/README.md"
: >"$CLEAN/Cargo.toml"
: >"$CLEAN/doc/TASK_COMPLETE.md"
if bash "$CHECK" "$CLEAN" >/dev/null 2>&1; then
  echo "ok: a clean root passes (nested *COMPLETE* is out of scope)"
else
  note_fail "a clean root should pass"
fi

# A root *COMPLETE* file fails, and the failure names it.
BAD="$TMP/complete"
mkdir -p "$BAD"
: >"$BAD/README.md"
: >"$BAD/TASK_COMPLETE.md"
if out=$(bash "$CHECK" "$BAD" 2>&1); then
  note_fail "a root TASK_COMPLETE.md should fail"
elif ! printf '%s' "$out" | grep -qF 'TASK_COMPLETE.md'; then
  note_fail "the failure should name the offending file"
  printf '%s\n' "$out" | sed 's/^/       /'
else
  echo "ok: a root TASK_COMPLETE.md fails, naming the file"
fi

# Matching is case-insensitive, like the lowercase slice-1 names
# #12136 removed (slice1_final_verification.md and friends).
LOWER="$TMP/lower"
mkdir -p "$LOWER"
: >"$LOWER/implementation_summary.txt"
if bash "$CHECK" "$LOWER" >/dev/null 2>&1; then
  note_fail "a lowercase root implementation_summary.txt should fail"
else
  echo "ok: a lowercase root *summary* file fails"
fi

# The exact shape #12136 cleaned up fails too.
SLICE="$TMP/slice"
mkdir -p "$SLICE"
: >"$SLICE/SLICE1_COMPLETION_SUMMARY.txt"
if bash "$CHECK" "$SLICE" >/dev/null 2>&1; then
  note_fail "a root SLICE1_COMPLETION_SUMMARY.txt should fail"
else
  echo "ok: a root SLICE1_COMPLETION_SUMMARY.txt fails"
fi

# Scope boundary: the issue's guard bullet names *COMPLETE* and
# *SUMMARY* only, so a *STATUS* name alone still passes.
STATUS="$TMP/status"
mkdir -p "$STATUS"
: >"$STATUS/IMPLEMENTATION_STATUS.md"
if bash "$CHECK" "$STATUS" >/dev/null 2>&1; then
  echo "ok: a root *STATUS*-only name passes (guard covers *COMPLETE* / *SUMMARY*)"
else
  note_fail "a *STATUS*-only name should pass"
fi

if bash "$CHECK" "$TMP/missing" >/dev/null 2>&1; then
  note_fail "a missing root should fail"
else
  echo "ok: a missing root fails"
fi

# Argument contract (#3352): anything beyond the one optional ROOT
# positional is a usage error, exit 2, with the usage message.
rc=0
out=$(bash "$CHECK" "$CLEAN" extra 2>&1) || rc=$?
if [ "$rc" -ne 2 ]; then
  note_fail "an extra argument should exit 2 (usage error), got $rc"
elif ! printf '%s' "$out" | grep -qF 'Usage:'; then
  note_fail "the extra-argument failure should print the usage message"
  printf '%s\n' "$out" | sed 's/^/       /'
else
  echo "ok: an extra argument is a usage error (exit 2, usage printed)"
fi

if [ "$failures" -ne 0 ]; then
  echo "check-root-status-files.test: $failures failure(s)"
  exit 1
fi
echo "check-root-status-files.test: OK"
