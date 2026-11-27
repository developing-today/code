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
  printf 'inactive' >"$1/units/pm-kaneo.service"
  printf 'inactive' >"$1/units/pm-kaneo.timer"
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
  if [ "$?" -eq 1 ] && [[ $out == UNFENCED:*container:pm-grist* ]]; then
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
  if [ "$?" -eq 1 ] && [[ $out == UNFENCED:*unit:docker-pm-pad.service* ]]; then
    pass 'active unit reports UNFENCED and names the writer'
  else
    fail "active unit reports UNFENCED and names the writer (got: $out)"
  fi
fi

# 4. Oracle: unreadable state fails closed (UNFENCED, exit 1), never FENCED.
if out="$("$BASH_BIN" "$CHECK" --units-dir "$TMP/no-such-dir" --containers-file "$TMP/no-such-file" 2>&1)"; then
  fail "unreadable fixtures fail closed (exit 0, got: $out)"
else
  if [ "$?" -eq 1 ] && [[ $out == UNFENCED:* ]]; then
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

# 10. Per-store fencing: Grist is fenced while the other stores are live.
d="$TMP/per-store-grist"
make_fixtures "$d"
printf 'pm-pad running\npm-stack-kaneo-1 running\n' >"$d/containers.txt"
if out="$("$BASH_BIN" "$CHECK" --store grist --units-dir "$d/units" --containers-file "$d/containers.txt" 2>&1)"; then
  case "$out" in
  FENCED:*) pass 'grist fence ignores Kaneo and Pad writers' ;;
  *) fail "grist fence ignores Kaneo and Pad writers (got: $out)" ;;
  esac
else
  fail "grist fence ignores Kaneo and Pad writers (exit $?, got: $out)"
fi

# 11. Per-store fencing: an active Pad unit is named and blocks Pad.
d="$TMP/per-store-pad"
make_fixtures "$d"
printf 'active' >"$d/units/docker-pm-pad.service"
if out="$("$BASH_BIN" "$CHECK" --store pad --units-dir "$d/units" --containers-file "$d/containers.txt" 2>&1)"; then
  fail "pad fence names its own active writer (exit 0, got: $out)"
else
  if [ "$?" -eq 1 ] && [[ $out == UNFENCED:*unit:docker-pm-pad.service* ]]; then
    pass 'pad fence names its own active writer'
  else
    fail "pad fence names its own active writer (got: $out)"
  fi
fi

# 12. Per-store unit fixtures fail closed when that store's unit is missing.
d="$TMP/per-store-missing-unit"
make_fixtures "$d"
rm "$d/units/docker-pm-grist.service"
if out="$("$BASH_BIN" "$CHECK" --store grist --units-dir "$d/units" --containers-file "$d/containers.txt" 2>&1)"; then
  fail "missing Grist unit fixture fails closed (exit 0, got: $out)"
else
  [ "$?" -eq 1 ] && [[ $out == UNFENCED:*docker-pm-grist.service* ]] && pass 'missing Grist unit fixture fails closed' || fail "missing Grist unit fixture fails closed (got: $out)"
fi

# 13. Unknown store is a usage error.
if "$BASH_BIN" "$CHECK" --store bogus >/dev/null 2>&1; then
  fail '--store bogus exits 2'
else
  [ "$?" -eq 2 ] && pass '--store bogus exits 2' || fail '--store bogus exits 2'
fi

# 14. Oracle: an active pm-kaneo.service blocks the Kaneo fence even with
# every container stopped — the manager (`deploy.sh` via `compose up`)
# revives stopped containers on its next run, so containers-stopped
# alone is not a fence (#3989 criterion 3).
d="$TMP/kaneo-manager-active"
make_fixtures "$d"
printf 'active' >"$d/units/pm-kaneo.service"
if out="$("$BASH_BIN" "$CHECK" --store kaneo --units-dir "$d/units" --containers-file "$d/containers.txt" 2>&1)"; then
  fail "active pm-kaneo.service blocks the kaneo fence (exit 0, got: $out)"
else
  if [ "$?" -eq 1 ] && [[ $out == UNFENCED:*unit:pm-kaneo.service* ]]; then
    pass 'active pm-kaneo.service blocks the kaneo fence and names the unit'
  else
    fail "active pm-kaneo.service blocks the kaneo fence and names the unit (got: $out)"
  fi
fi

# 15. Oracle: an armed pm-kaneo.timer blocks the Kaneo fence even with the
# service stopped — the 30 min retry elapse re-runs `deploy.sh` and
# restarts the stack, so the fence must pin the timer too.
d="$TMP/kaneo-timer-armed"
make_fixtures "$d"
printf 'active' >"$d/units/pm-kaneo.timer"
if out="$("$BASH_BIN" "$CHECK" --store kaneo --units-dir "$d/units" --containers-file "$d/containers.txt" 2>&1)"; then
  fail "armed pm-kaneo.timer blocks the kaneo fence (exit 0, got: $out)"
else
  if [ "$?" -eq 1 ] && [[ $out == UNFENCED:*unit:pm-kaneo.timer* ]]; then
    pass 'armed pm-kaneo.timer blocks the kaneo fence and names the unit'
  else
    fail "armed pm-kaneo.timer blocks the kaneo fence and names the unit (got: $out)"
  fi
fi

# 16. Oracle: the armed timer blocks --store all as well (the cutover
# promote path checks the full writer set, not a per-store subset).
d="$TMP/all-timer-armed"
make_fixtures "$d"
printf 'active' >"$d/units/pm-kaneo.timer"
if out="$("$BASH_BIN" "$CHECK" --units-dir "$d/units" --containers-file "$d/containers.txt" 2>&1)"; then
  fail "armed pm-kaneo.timer blocks the full fence (exit 0, got: $out)"
else
  if [ "$?" -eq 1 ] && [[ $out == UNFENCED:*unit:pm-kaneo.timer* ]]; then
    pass 'armed pm-kaneo.timer blocks the full fence and names the unit'
  else
    fail "armed pm-kaneo.timer blocks the full fence and names the unit (got: $out)"
  fi
fi

if ((FAILURES)); then
  printf 'fence-check.test.sh: %d check(s) failed\n' "$FAILURES"
  exit 1
fi
echo 'LIMITATION: fixtures replace docker/systemctl; these checks verify the fencing rule against fixture state, not a live old host.'
echo 'fence-check.test.sh: all checks passed'
