#!/usr/bin/env bash
# grist-pad-sync.test.sh — fixture tests for Grist/Pad file-store synchronization (#6816).
#
# Oracle: STORE-MIGRATION-PLAN-6816.md §§3–4 — presync copies the selected
# file set and reports pre-final drift; final copies only after the selected
# writer is fenced, the latest backup record describes a fresh verified
# restore point (complete generation, verified local/R2/Drive copies,
# finished within 36h — the same consumer gate kaneo-cutover.sh promote
# carries since #14034), Pad's WAL is empty, and copied content has exact
# SHA-256 parity. Refusal must not copy. Target-only files are preserved and counted.
#
# Run from anywhere: bash pm-stack/scripts/grist-pad-sync.test.sh
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DRIVER="$SCRIPT_DIR/grist-pad-sync.sh"
CHECK="$SCRIPT_DIR/fence-check.sh"
BASH_BIN="${BASH:-$(command -v bash)}"

TMP_BASE="${TMPDIR:-/tmp/opencode}"
mkdir -p "$TMP_BASE"
TMP="$(mktemp -d "$TMP_BASE/grist-pad-sync.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT

FAILURES=0
pass() { printf 'ok - %s\n' "$1"; }
fail() {
  printf 'FAIL - %s\n' "$1"
  FAILURES=$((FAILURES + 1))
}

mkdir -p "$TMP/units"
printf 'inactive' >"$TMP/units/docker-pm-grist.service"
printf 'inactive' >"$TMP/units/docker-pm-pad.service"
: >"$TMP/containers-clean.txt"
printf 'pm-grist running\npm-stack-kaneo-1 running\n' >"$TMP/containers-other-running.txt"
printf 'pm-pad running\n' >"$TMP/containers-pad-running.txt"

setup_grist() {
  mkdir -p "$TMP/grist-source/docs" "$TMP/grist-target/docs"
  printf 'home-db' >"$TMP/grist-source/home.sqlite3"
  printf 'doc-data' >"$TMP/grist-source/docs/doc-1"
  printf 'sessions-are-excluded' >"$TMP/grist-source/grist-sessions.db"
  printf 'stale-target-file' >"$TMP/grist-target/old-file"
}
setup_pad() {
  mkdir -p "$TMP/pad-source/attachments" "$TMP/pad-target/attachments"
  printf 'pad-db' >"$TMP/pad-source/pad.db"
  printf 'secret-key' >"$TMP/pad-source/encryption.key"
  printf 'config' >"$TMP/pad-source/config.toml"
  printf 'attachment' >"$TMP/pad-source/attachments/file-1"
}
run() { "$BASH_BIN" "$DRIVER" "$@" 2>&1; }
common=(--fence-check "$CHECK" --units-dir "$TMP/units")

# Backup restore-point fixtures (oracle: restore-point-check.py — a complete
# generation with verified local/R2/Drive copies finished within 36h).
# Mirrors kaneo-cutover.test.sh; the daily pm-stack generation covers the
# Grist/Pad file sets this driver syncs (home.sqlite3 + docs/, pad.db +
# encryption.key + config.toml + attachments/).
python3 -I - "$TMP/backup.json" <<'PY'
import json, sys
from datetime import datetime, timezone, timedelta
finished = datetime.now(timezone.utc) - timedelta(minutes=5)
json.dump({
    "date": finished.date().isoformat(),
    "finishedAt": finished.isoformat(),
    "result": "complete",
    "archiveSha256": "b" * 64,
    "destinations": {"local": "ok", "r2": "ok", "drive": "ok"},
}, open(sys.argv[1], "w"))
PY
finalgates=(--backup-record-file "$TMP/backup.json")

# 1. Grist presync copies its declared files, verifies checksums and never copies session DB.
setup_grist
if out="$(run --store grist --mode presync --source-dir "$TMP/grist-source" --target-dir "$TMP/grist-target" --receipt "$TMP/grist-presync.md" "${common[@]}")"; then
  if cmp -s "$TMP/grist-source/home.sqlite3" "$TMP/grist-target/home.sqlite3" && cmp -s "$TMP/grist-source/docs/doc-1" "$TMP/grist-target/docs/doc-1" && [ ! -e "$TMP/grist-target/grist-sessions.db" ] && [ -e "$TMP/grist-target/old-file" ] && grep -q 'PASS' "$TMP/grist-presync.md"; then
    pass 'Grist presync copies selected files, verifies parity, excludes sessions, and preserves target extras'
  else
    fail "Grist presync file set and receipt (got: $out)"
  fi
else
  fail "Grist presync succeeds (exit $?, got: $out)"
fi

# 2. Pad final refuses an unfenced Pad writer and must not copy.
setup_pad
if out="$(run --store pad --mode final --source-dir "$TMP/pad-source" --target-dir "$TMP/pad-target" --receipt "$TMP/pad-refused.md" --containers-file "$TMP/containers-pad-running.txt" "${common[@]}" "${finalgates[@]}")"; then
  fail "Pad final refuses an active writer (exit 0, got: $out)"
else
  if [ "$?" -eq 3 ] && [ ! -e "$TMP/pad-target/pad.db" ] && grep -q 'REFUSED' "$TMP/pad-refused.md"; then
    pass 'Pad final refuses an active writer, records REFUSED, and copies nothing'
  else
    fail "Pad final refuses an active writer without copying (got: $out)"
  fi
fi

# 3. Pad final refuses a non-empty WAL even with its writer fenced.
printf 'pending-wal' >"$TMP/pad-source/pad.db-wal"
if out="$(run --store pad --mode final --source-dir "$TMP/pad-source" --target-dir "$TMP/pad-target" --receipt "$TMP/pad-wal-refused.md" --containers-file "$TMP/containers-clean.txt" "${common[@]}" "${finalgates[@]}")"; then
  fail "Pad final refuses non-empty WAL (exit 0, got: $out)"
else
  if [ "$?" -eq 3 ] && [ ! -e "$TMP/pad-target/pad.db" ] && grep -q 'REFUSED' "$TMP/pad-wal-refused.md"; then
    pass 'Pad final refuses non-empty WAL, records REFUSED, and copies nothing'
  else
    fail "Pad final refuses non-empty WAL without copying (got: $out)"
  fi
fi
rm "$TMP/pad-source/pad.db-wal"

# 4. Final uses per-store fencing: Pad can sync while Grist is still live.
if out="$(run --store pad --mode final --source-dir "$TMP/pad-source" --target-dir "$TMP/pad-target" --receipt "$TMP/pad-final.md" --containers-file "$TMP/containers-other-running.txt" "${common[@]}" "${finalgates[@]}")"; then
  if [ -f "$TMP/pad-target/pad.db" ] && [ -f "$TMP/pad-target/attachments/file-1" ] && grep -q 'PASS' "$TMP/pad-final.md" && grep -qi 'restore point' "$TMP/pad-final.md"; then
    pass 'Pad final passes with Pad fenced while Grist remains active and exact parity holds'
  else
    fail "Pad final copies its file set and records parity (got: $out)"
  fi
else
  fail "Pad final passes with only Pad fenced (exit $?, got: $out)"
fi

# 5. Target extras under the synced set are never removed and are counted.
printf 'keep-me' >"$TMP/pad-target/attachments/stale.txt"
if out="$(run --store pad --mode presync --source-dir "$TMP/pad-source" --target-dir "$TMP/pad-target" --receipt "$TMP/pad-extra.md" "${common[@]}")"; then
  if [ -f "$TMP/pad-target/attachments/stale.txt" ] && grep -Eq 'Target extras: 1 kept' "$TMP/pad-extra.md" && grep -Fq 'stale.txt' "$TMP/pad-extra.md"; then
    pass 'Pad presync preserves and counts target-only files'
  else
    fail "Pad target-only files preserved and counted (got: $out; receipt: $(cat "$TMP/pad-extra.md"))"
  fi
else
  fail "Pad presync with target extras succeeds (exit $?, got: $out)"
fi

# 6. Final refuses unaccounted target files with exit 4 and names them.
if out="$(run --store pad --mode final --source-dir "$TMP/pad-source" --target-dir "$TMP/pad-target" --receipt "$TMP/pad-extra-refused.md" --containers-file "$TMP/containers-clean.txt" "${common[@]}" "${finalgates[@]}")"; then
  fail "Pad final refuses target extras (exit 0, got: $out)"
else
  if [ "$?" -eq 4 ] && grep -q 'FAIL' "$TMP/pad-extra-refused.md" && grep -Fq 'stale.txt' "$TMP/pad-extra-refused.md"; then
    pass 'Pad final refuses target extras with exit 4 and names them'
  else
    fail "Pad final refuses target extras with exit 4 and names them (got: $out)"
  fi
fi
rm "$TMP/pad-target/attachments/stale.txt"

# 7. Missing required source file is a command failure (exit 5), not a pass.
rm "$TMP/grist-source/home.sqlite3"
if run --store grist --mode presync --source-dir "$TMP/grist-source" --target-dir "$TMP/grist-target" "${common[@]}" >/dev/null 2>&1; then
  fail 'missing required source file exits 5'
else
  [ "$?" -eq 5 ] && pass 'missing required source file exits 5' || fail 'missing required source file exits 5'
fi

# 8. Contract (#3352): invalid store and stray arguments fail before copy.
if run --store bogus --mode presync --source-dir "$TMP/grist-source" --target-dir "$TMP/grist-target" >/dev/null 2>&1; then
  fail 'invalid store exits 2'
else
  [ "$?" -eq 2 ] && pass 'invalid store exits 2' || fail 'invalid store exits 2'
fi
if run --store grist --mode presync --source-dir "$TMP/grist-source" --target-dir "$TMP/grist-target" stray >/dev/null 2>&1; then
  fail 'stray positional exits 2'
else
  [ "$?" -eq 2 ] && pass 'stray positional exits 2' || fail 'stray positional exits 2'
fi

# 9. Final refuses a stale backup generation (exit 3, copies nothing,
# REFUSED receipt naming the restore point) even with the writer fenced.
python3 -I - "$TMP/stale-backup.json" <<'PY'
import json, sys
from datetime import datetime, timezone, timedelta
finished = datetime.now(timezone.utc) - timedelta(hours=37)
json.dump({
    "date": finished.date().isoformat(),
    "finishedAt": finished.isoformat(),
    "result": "complete",
    "archiveSha256": "b" * 64,
    "destinations": {"local": "ok", "r2": "ok", "drive": "ok"},
}, open(sys.argv[1], "w"))
PY
rm -rf "$TMP/pad-target-stale" && mkdir -p "$TMP/pad-target-stale"
if out="$(run --store pad --mode final --source-dir "$TMP/pad-source" --target-dir "$TMP/pad-target-stale" --receipt "$TMP/pad-stale-refused.md" --containers-file "$TMP/containers-clean.txt" "${common[@]}" --backup-record-file "$TMP/stale-backup.json")"; then
  fail "Pad final refuses a stale backup generation (exit 0, got: $out)"
else
  if [ "$?" -eq 3 ] && [ ! -e "$TMP/pad-target-stale/pad.db" ] && grep -q 'REFUSED' "$TMP/pad-stale-refused.md" && grep -qi 'restore point' "$TMP/pad-stale-refused.md"; then
    pass 'Pad final refuses a stale backup generation, records REFUSED with the restore point, and copies nothing'
  else
    fail "Pad final refuses a stale backup generation without copying (got: $out)"
  fi
fi

# 10. Final refuses an unverified backup generation (R2 copy failed).
# Rebuild the grist source file removed by check 7 so this check proves the
# backup gate, not the missing-file gate.
printf 'home-db' >"$TMP/grist-source/home.sqlite3"
python3 -I - "$TMP/unverified-backup.json" <<'PY'
import json, sys
from datetime import datetime, timezone, timedelta
finished = datetime.now(timezone.utc) - timedelta(minutes=5)
json.dump({
    "date": finished.date().isoformat(),
    "finishedAt": finished.isoformat(),
    "result": "complete",
    "archiveSha256": "b" * 64,
    "destinations": {"local": "ok", "r2": "failed", "drive": "ok"},
}, open(sys.argv[1], "w"))
PY
rm -rf "$TMP/grist-target-unverified" && mkdir -p "$TMP/grist-target-unverified"
if out="$(run --store grist --mode final --source-dir "$TMP/grist-source" --target-dir "$TMP/grist-target-unverified" --receipt "$TMP/grist-unverified-refused.md" --containers-file "$TMP/containers-clean.txt" "${common[@]}" --backup-record-file "$TMP/unverified-backup.json")"; then
  fail "Grist final refuses an unverified backup generation (exit 0, got: $out)"
else
  if [ "$?" -eq 3 ] && [ ! -e "$TMP/grist-target-unverified/home.sqlite3" ] && grep -q 'REFUSED' "$TMP/grist-unverified-refused.md"; then
    pass 'Grist final refuses an unverified backup generation, records REFUSED, and copies nothing'
  else
    fail "Grist final refuses an unverified backup generation without copying (got: $out)"
  fi
fi

# 11. Contract (#3352): final without --backup-record-file exits 2 before any copy.
rm -rf "$TMP/pad-target-norecord" && mkdir -p "$TMP/pad-target-norecord"
if run --store pad --mode final --source-dir "$TMP/pad-source" --target-dir "$TMP/pad-target-norecord" --containers-file "$TMP/containers-clean.txt" "${common[@]}" >/dev/null 2>&1; then
  fail 'final without a backup record exits 2'
else
  if [ "$?" -eq 2 ] && [ ! -e "$TMP/pad-target-norecord/pad.db" ]; then
    pass 'final without a backup record exits 2 and copies nothing'
  else
    fail 'final without a backup record exits 2 and copies nothing'
  fi
fi

if ((FAILURES)); then
  printf 'grist-pad-sync.test.sh: %d check(s) failed\n' "$FAILURES"
  exit 1
fi
echo 'LIMITATION: parity failure after copy is difficult to force with a working cp; these checks verify the post-copy hash pass on matching fixtures, not a deliberately corrupted copy.'
echo 'LIMITATION: fence-check uses fixtures; these checks verify per-store ordering, not a live host.'
echo 'grist-pad-sync.test.sh: all checks passed'
