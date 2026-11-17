#!/usr/bin/env bash
# Test suite for opencode-db-backup.sh (code#10297).
# Drives the REAL script (no pasted copy) with a scratch HOME holding a
# fake ~/.local/share/opencode/opencode.db, a stub bws printing fake
# credential JSON, a stub sqlite3 that performs .backup as a file copy
# and answers integrity_check with "ok", and a stub restic that records
# its invocations and maintains a fake local repo (config + snapshots).
# rclone is stubbed too: the Drive leg runs restic against an rclone:
# URL, so the stub restic only records it.
# Scenarios:
#   * happy path — exit 0, one restic backup per backend (local + r2 +
#     drive) with --tag opencode-db-daily over the staged snapshots,
#     check + forget+prune (with keep-monthly 12) per backend, staging
#     removed, success marker + run record written.
#   * missing DB — an HOME without opencode.db exits non-zero before
#     any restic invocation.
#   * lock skip — a second instance holding the flock prints SKIP and
#     exits 0, invoking restic zero times.
set -uo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$SELF_DIR/opencode-db-backup.sh"

TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT

failures=0
note_fail() {
  echo "FAIL: $*" >&2
  failures=$((failures + 1))
}

make_home() { # $1 dir; fake opencode share dir + bws token
  local h="$1"
  mkdir -p "$h/.local/share/opencode" "$h/.config/bws"
  printf 'fake-sqlite-db-bytes' >"$h/.local/share/opencode/opencode.db"
  printf 'fake-local-db-bytes' >"$h/.local/share/opencode/opencode-local.db"
  echo "fake-token" >"$h/.config/bws/access-token"
}

make_bin() { # $1 bin dir, $2 restic calls file
  local b="$1" calls="$2"
  mkdir -p "$b"
  cat >"$b/bws" <<'EOF'
#!/bin/sh
if [ "$1" = "secret" ] && [ "$2" = "list" ]; then
  printf '[{"key":"restic.repo-archive-password","value":"pw"},{"key":"cloudflare.r2-backups-access-key-id","value":"id"},{"key":"cloudflare.r2-backups-secret-access-key","value":"secret"},{"key":"cloudflare.r2-endpoint","value":"https://example.r2.test"}]'
  exit 0
fi
exit 1
EOF
  cat >"$b/sqlite3" <<'EOF'
#!/bin/sh
db="$1"; sql="$2"
case "$sql" in
  "VACUUM INTO "*)
    dest="$(printf '%s' "$sql" | sed "s/^VACUUM INTO '//; s/';$//")"
    cp "$db" "$dest" ;;
  "PRAGMA integrity_check;") echo "ok" ;;
  *) echo "stub sqlite3: unexpected sql: $sql" >&2; exit 1 ;;
esac
exit 0
EOF
  cat >"$b/restic" <<EOF
#!/bin/sh
echo "\$*" >> "$calls"
repo="\$(printf '%s\n' "\$@" | grep -A1 '^-r\$' | tail -1)"
if [ "\$1" = "-r" ]; then shift 2; fi
case "\$repo" in
  *:* ) :;; # remote URL (s3:/rclone:) — record invocation only
  *)
    case "\$1" in
      cat|init) mkdir -p "\$repo"; touch "\$repo/config";;
      backup) mkdir -p "\$repo"; echo "\$*" >> "\$repo/snapshots"; echo "Added to the repository: 1.000 KiB";;
      check|forget) :;;
    esac;;
esac
exit 0
EOF
  cat >"$b/rclone" <<'EOF'
#!/bin/sh
exit 0
EOF
  chmod +x "$b/bws" "$b/sqlite3" "$b/restic" "$b/rclone"
}

run_script() { # $1 home, $2 bin
  HOME="$1" PATH="$2:$PATH" bash "$SCRIPT"
}

# --- happy path -------------------------------------------------------------
H1="$TMP_ROOT/h1" B1="$TMP_ROOT/b1" C1="$TMP_ROOT/calls1"
make_home "$H1"
make_bin "$B1" "$C1"
out="$(run_script "$H1" "$B1" 2>&1)" || note_fail "happy path exited non-zero: $out"
echo "$out" | grep -q "OK opencode session DB backed up" || note_fail "happy path missing OK line: $out"
backups="$(grep -c ' backup ' "$C1" 2>/dev/null || true)"
[ "$backups" = "3" ] || note_fail "expected 3 restic backup invocations, got $backups"
grep -q -- '--tag opencode-db-daily' "$C1" || note_fail "backup invocations missing --tag opencode-db-daily"
grep -q -- '--keep-monthly 12' "$C1" || note_fail "forget invocations missing --keep-monthly 12"
grep -q 'opencode.db' "$C1" || note_fail "backup invocations missing staged opencode.db"
[ -f "$H1/backups/restic/.opencode-db-last-success-epoch" ] || note_fail "success marker not written"
[ -f "$H1/backups/restic/.opencode-db-last-run.json" ] || note_fail "run record not written"
grep -q '"result": "complete"' "$H1/backups/restic/.opencode-db-last-run.json" 2>/dev/null ||
  note_fail "run record does not say complete"
[ ! -e "$H1/backups/opencode-db-staging" ] || note_fail "staging dir not cleaned after success"

# --- missing DB ---------------------------------------------------------------
H2="$TMP_ROOT/h2" B2="$TMP_ROOT/b2" C2="$TMP_ROOT/calls2"
mkdir -p "$H2/.config/bws"
echo "fake-token" >"$H2/.config/bws/access-token"
make_bin "$B2" "$C2"
if run_script "$H2" "$B2" >/dev/null 2>&1; then
  note_fail "missing DB should exit non-zero"
fi
[ ! -f "$C2" ] || note_fail "missing DB invoked restic"

# --- lock skip ------------------------------------------------------------------
H3="$TMP_ROOT/h3" B3="$TMP_ROOT/b3" C3="$TMP_ROOT/calls3"
make_home "$H3"
make_bin "$B3" "$C3"
mkdir -p "$H3/backups/restic"
flock "$H3/backups/restic/.opencode-db-backup.lock" sleep 10 &
lock_pid=$!
sleep 0.5
out="$(run_script "$H3" "$B3" 2>&1)" || note_fail "lock skip exited non-zero: $out"
echo "$out" | grep -q "SKIP" || note_fail "lock skip missing SKIP line: $out"
[ ! -f "$C3" ] || note_fail "lock skip invoked restic"
kill "$lock_pid" 2>/dev/null || true
wait "$lock_pid" 2>/dev/null || true

if [ "$failures" -gt 0 ]; then
  echo "opencode-db-backup.test: $failures failure(s)" >&2
  exit 1
fi
echo "opencode-db-backup.test: all scenarios passed"
