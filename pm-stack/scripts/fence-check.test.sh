#!/usr/bin/env bash
# fence-check.test.sh — fixture tests for fence-check.sh (#3989 criterion 3).
#
# Oracle: runbook step 1 of doc/pm-stack/COLD-RESTORE-RUNBOOK.md — the old
# writer is fenced only when its writer containers are all stopped AND its
# writer units are all inactive; an unreachable host is fenced by
# definition; anything unreadable must report UNFENCED, never FENCED.
# No docker or systemctl is touched: every live path is replaced by
# --units-dir / --containers-file fixtures.
#
# Run from anywhere: bash pm-stack/scripts/fence-check.test.sh
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHECK="$SCRIPT_DIR/fence-check.sh"
BASH_BIN="${BASH:-$(command -v bash)}"

TMP_BASE="${TMPDIR:-/tmp/opencode}"
mkdir -p "$TMP_BASE"
TMP="$(mktemp -d "$TMP_BASE/fence-check.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT

FAILURES=0
pass() { printf 'ok - %s\n' "$1"; }
fail() {
  printf 'FAIL - %s\n' "$1"
  FAILURES=$((FAILURES + 1))
}

make_fixtures() {
  # $1 = dir; writes an all-stopped / all-inactive fixture set.
  mkdir -p "$1/units"
  : >"$1/containers.txt"
  printf 'inactive' >"$1/units/docker-pm-grist.service"
  printf 'inactive' >"$1/units/docker-pm-pad.service"
}

# 1. Oracle: all writers stopped+inactive reports FENCED (exit 0).
d="$TMP/clean"
make_fixtures "$d"
if out="$("$BASH_BIN" "$CHECK" --units-dir "$d/units" --containers-file "$d/containers.txt" 2>&1)"; then
  case "$out" in
    FENCED:*) pass 'all writers stopped reports FENCED' ;;
    *) fail "all writers stopped reports FENCED (got: $out)" ;;
  esac
else
  fail "all writers stopped reports FENCED (exit $?, got: $out)"
fi

# 2. Oracle: one running writer container reports UNFENCED and names it.
d="$TMP/running"
make_fixtures "$d"
printf 'pm-grist running\n' >"$d/containers.txt"
if out="$("$BASH_BIN" "$CHECK" --units-dir "$d/units" --containers-file "$d/containers.txt" 2>&1)"; then
  fail "running container reports UNFENCED (exit 0, got: $out)"
else
  if [ "$?" -eq 1 ] && [[ "$out" == UNFENCED:*container:pm-grist* ]]; then
    pass 'running container reports UNFENCED and names the writer'
  else
    fail "running container reports UNFENCED and names the writer (got: $out)"
  fi
fi

# 3. Oracle: an active writer unit reports UNFENCED and names it.
d="$TMP/active-unit"
make_fixtures "$d"
printf 'active' >"$d/units/docker-pm-pad.service"
if out="$("$BASH_BIN" "$CHECK" --units-dir "$d/units" --containers-file "$d/containers.txt" 2>&1)"; then
  fail "active unit reports UNFENCED (exit 0, got: $out)"
else
  if [ "$?" -eq 1 ] && [[ "$out" == UNFENCED:*unit:docker-pm-pad.service* ]]; then
    pass 'active unit reports UNFENCED and names the writer'
  else
    fail "active unit reports UNFENCED and names the writer (got: $out)"
  fi
fi

# 4. Oracle: unreadable state fails closed (UNFENCED, exit 1), never FENCED.
if out="$("$BASH_BIN" "$CHECK" --units-dir "$TMP/no-such-dir" --containers-file "$TMP/no-such-file" 2>&1)"; then
  fail "unreadable fixtures fail closed (exit 0, got: $out)"
else
  if [ "$?" -eq 1 ] && [[ "$out" == UNFENCED:* ]]; then
    pass 'unreadable fixtures fail closed as UNFENCED'
  else
    fail "unreadable fixtures fail closed as UNFENCED (got: $out)"
  fi
fi

# 5. Oracle: unreachable old host is fenced by definition (exit 0).
if out="$("$BASH_BIN" "$CHECK" --unreachable 2>&1)"; then
  case "$out" in
    FENCED:*) pass 'unreachable host reports FENCED by definition' ;;
    *) fail "unreachable host reports FENCED by definition (got: $out)" ;;
  esac
else
  fail "unreachable host reports FENCED by definition (exit $?, got: $out)"
fi

# 6. Contract (#3352): unknown flag is a usage error (exit 2).
if "$BASH_BIN" "$CHECK" --bogus >/dev/null 2>&1; then
  fail 'unknown flag exits 2'
else
  [ "$?" -eq 2 ] && pass 'unknown flag exits 2' || fail 'unknown flag exits 2'
fi

# 7. Contract (#3352): stray positional is a usage error (exit 2).
if "$BASH_BIN" "$CHECK" someservice >/dev/null 2>&1; then
  fail 'stray positional exits 2'
else
  [ "$?" -eq 2 ] && pass 'stray positional exits 2' || fail 'stray positional exits 2'
fi

# 8. Contract (#3352): --opt=value works identically to --opt value.
d="$TMP/equals-form"
make_fixtures "$d"
if out="$("$BASH_BIN" "$CHECK" --units-dir="$d/units" --containers-file="$d/containers.txt" 2>&1)"; then
  case "$out" in
    FENCED:*) pass '--opt=value form works identically' ;;
    *) fail "--opt=value form works identically (got: $out)" ;;
  esac
else
  fail "--opt=value form works identically (exit $?, got: $out)"
fi

# 9. Contract (#3352): present-but-valueless option is a usage error (exit 2).
if "$BASH_BIN" "$CHECK" --units-dir >/dev/null 2>&1; then
  fail 'valueless option exits 2'
else
  [ "$?" -eq 2 ] && pass 'valueless option exits 2' || fail 'valueless option exits 2'
fi

if ((FAILURES)); then
  printf 'fence-check.test.sh: %d check(s) failed\n' "$FAILURES"
  exit 1
fi
echo 'LIMITATION: fixtures replace docker/systemctl; these checks verify the fencing rule against fixture state, not a live old host.'
echo 'fence-check.test.sh: all checks passed'
