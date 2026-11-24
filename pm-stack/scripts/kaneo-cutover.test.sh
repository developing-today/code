#!/usr/bin/env bash
# kaneo-cutover.test.sh — fixture tests for kaneo-cutover.sh (#6816).
#
# Oracle: pm-stack/STORE-MIGRATION-PLAN-6816.md §3-§4 plus
# doc/pm-stack/COLD-RESTORE-RUNBOOK.md step 1 — promotion may happen only
# when the old writer is fenced (proven by fence-check.sh) AND the final
# delta has landed (target table count and exact rows equal the source's).
# A refused promotion must not touch the stores at all.
#
# Every external binary is stubbed (pg_dump, pg_restore, psql on
# --pg-bin-dir; docker/systemctl via fence-check fixtures), so these
# checks verify the driver's ordering and gates, not a live database.
#
# Run from anywhere: bash pm-stack/scripts/kaneo-cutover.test.sh
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DRIVER="$SCRIPT_DIR/kaneo-cutover.sh"
BASH_BIN="${BASH:-$(command -v bash)}"

TMP_BASE="${TMPDIR:-/tmp/opencode}"
mkdir -p "$TMP_BASE"
TMP="$(mktemp -d "$TMP_BASE/kaneo-cutover.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT

FAILURES=0
pass() { printf 'ok - %s\n' "$1"; }
fail() {
  printf 'FAIL - %s\n' "$1"
  FAILURES=$((FAILURES + 1))
}

# --- stubs ---------------------------------------------------------------
mkdir -p "$TMP/bin" "$TMP/fx"
cat >"$TMP/bin/psql" <<'STUB'
#!/usr/bin/env bash
dsn=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -d) dsn="$2"; shift 2 ;;
    *) shift ;;
  esac
done
printf 'psql\n' >>"$CUTOVER_FIXTURE_DIR/log"
case "$dsn" in
  *source*) f="$CUTOVER_FIXTURE_DIR/counts-source.txt" ;;
  *target*) f="$CUTOVER_FIXTURE_DIR/counts-target.txt" ;;
  *) echo "stub psql: unknown dsn" >&2; exit 1 ;;
esac
[ -r "$f" ] || { echo "stub psql: missing fixture $f" >&2; exit 1; }
cat "$f"
STUB
cat >"$TMP/bin/pg_dump" <<'STUB'
#!/usr/bin/env bash
dsn=""; out=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -f) out="$2"; shift 2 ;;
    -d) dsn="$2"; shift 2 ;;
    *) shift ;;
  esac
done
[ -n "$out" ] || { echo "stub pg_dump: no -f" >&2; exit 1; }
case "$dsn" in
  *source*) role="source" ;;
  *target*) role="target" ;;
  *) echo "stub pg_dump: unknown dsn" >&2; exit 1 ;;
esac
printf 'pg_dump:%s\n' "$role" >>"$CUTOVER_FIXTURE_DIR/log"
printf 'dump\n' >"$out"
STUB
cat >"$TMP/bin/pg_restore" <<'STUB'
#!/usr/bin/env bash
dsn=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -d) dsn="$2"; shift 2 ;;
    *) shift ;;
  esac
done
case "$dsn" in
  *source*) role="source" ;;
  *target*) role="target" ;;
  *) echo "stub pg_restore: unknown dsn" >&2; exit 1 ;;
esac
printf 'pg_restore:%s\n' "$role" >>"$CUTOVER_FIXTURE_DIR/log"
STUB
chmod +x "$TMP/bin/psql" "$TMP/bin/pg_dump" "$TMP/bin/pg_restore"

printf 'source-db dsn\n' >"$TMP/source.dsn"
printf 'target-db dsn\n' >"$TMP/target.dsn"
python3 -I - "$TMP/backup.json" <<'PY'
import json, sys
from datetime import datetime, timezone, timedelta
finished = datetime.now(timezone.utc) - timedelta(minutes=5)
json.dump({
    "date": finished.date().isoformat(),
    "finishedAt": finished.isoformat(),
    "result": "complete",
    "archiveSha256": "a" * 64,
    "destinations": {"local": "ok", "r2": "ok", "drive": "ok"},
}, open(sys.argv[1], "w"))
PY

# Fence fixtures: all writers stopped/inactive.
mkdir -p "$TMP/units"
printf 'inactive' >"$TMP/units/docker-pm-grist.service"
printf 'inactive' >"$TMP/units/docker-pm-pad.service"
: >"$TMP/containers-clean.txt"
printf 'pm-stack-kaneo-1 running\n' >"$TMP/containers-kaneo-running.txt"

export CUTOVER_FIXTURE_DIR="$TMP/fx"
set_counts() { # $1 = source "tables rows", $2 = target "tables rows"
  printf '%s\n' "$1" >"$TMP/fx/counts-source.txt"
  printf '%s\n' "$2" >"$TMP/fx/counts-target.txt"
}
reset_log() { : >"$TMP/fx/log"; }
log_is_empty() { [ ! -s "$TMP/fx/log" ]; }

run() { "$BASH_BIN" "$DRIVER" "$@" 2>&1; }

common=(--pg-bin-dir "$TMP/bin" --source-dsn-file "$TMP/source.dsn" --target-dsn-file "$TMP/target.dsn")
promote=(--backup-record-file "$TMP/backup.json")

# 1. presync dumps then restores, and passes on schema parity.
set_counts '45 90139' '45 0'
reset_log
if out="$(run --mode presync --dump-file "$TMP/k.dump" --receipt "$TMP/r1.md" "${common[@]}")"; then
  if [ "$(cat "$TMP/fx/log" | tr '\n' ' ')" = "pg_dump:source pg_restore:target " ]; then
    pass 'presync dumps the source then restores into the target'
  else
    fail "presync dumps the source then restores into the target (log: $(cat "$TMP/fx/log" | tr '\n' ' '))"
  fi
  if grep -q 'PASS (schema)' "$TMP/r1.md" && ! grep -q 'target-db\|source-db' "$TMP/r1.md"; then
    pass 'presync receipt records schema parity and no DSN'
  else
    fail 'presync receipt records schema parity and no DSN'
  fi
else
  fail "presync passes on schema parity (exit $?, got: $out)"
fi

# 2. presync fails on a schema mismatch (fewer target tables).
set_counts '45 90139' '44 90000'
reset_log
if out="$(run --mode presync --dump-file "$TMP/k2.dump" --receipt "$TMP/r2.md" "${common[@]}")"; then
  fail "presync fails on schema mismatch (exit 0, got: $out)"
else
  if [ "$?" -eq 4 ] && grep -q 'FAIL — schema parity' "$TMP/r2.md"; then
    pass 'presync fails on schema mismatch with exit 4'
  else
    fail "presync fails on schema mismatch with exit 4 (got: $out)"
  fi
fi

# 3. promote refuses while a writer is running — and touches nothing.
set_counts '45 90139' '45 90139'
reset_log
if out="$(run --mode promote --receipt "$TMP/r3.md" --units-dir "$TMP/units" --containers-file "$TMP/containers-kaneo-running.txt" "${promote[@]}" "${common[@]}")"; then
  fail "promote refuses while unfenced (exit 0, got: $out)"
else
  if [ "$?" -eq 3 ] && log_is_empty && grep -q 'REFUSED' "$TMP/r3.md"; then
    pass 'promote refuses unfenced, writes REFUSED, touches no store'
  else
    fail "promote refuses unfenced, writes REFUSED, touches no store (got: $out)"
  fi
fi

# 4. promote refuses on final-delta row mismatch even when fenced.
set_counts '45 90139' '45 89000'
reset_log
if out="$(run --mode promote --receipt "$TMP/r4.md" --units-dir "$TMP/units" --containers-file "$TMP/containers-clean.txt" "${promote[@]}" "${common[@]}")"; then
  fail "promote fails on row mismatch (exit 0, got: $out)"
else
  if [ "$?" -eq 4 ] && grep -q 'FAIL — final-delta parity' "$TMP/r4.md"; then
    pass 'promote fails on row mismatch with exit 4'
  else
    fail "promote fails on row mismatch with exit 4 (got: $out)"
  fi
fi

# 5. promote passes when fenced and counts match; receipt carries the fence.
set_counts '45 90139' '45 90139'
reset_log
if out="$(run --mode promote --receipt "$TMP/r5.md" --units-dir "$TMP/units" --containers-file "$TMP/containers-clean.txt" "${promote[@]}" "${common[@]}")"; then
  case "$out" in
  *'PROMOTE OK'*FENCED*) pass 'promote passes when fenced and parity holds' ;;
  *) fail "promote passes when fenced and parity holds (got: $out)" ;;
  esac
  if grep -q 'Fence: FENCED' "$TMP/r5.md" && grep -q '45 90139' "$TMP/r5.md" && grep -q 'RESTORE POINT OK' "$TMP/r5.md"; then
    pass 'promote receipt records the fence, backup restore point, and counts'
  else
    fail 'promote receipt records the fence, backup restore point, and counts'
  fi
else
  fail "promote passes when fenced and parity holds (exit $?, got: $out)"
fi

# 6. An unreadable target is a command failure (exit 5), not a pass.
set_counts '45 90139' '45 90139'
rm -f "$TMP/fx/counts-target.txt"
if out="$(run --mode promote --units-dir "$TMP/units" --containers-file "$TMP/containers-clean.txt" "${promote[@]}" "${common[@]}")"; then
  fail "unreadable target exits 5 (exit 0, got: $out)"
else
  [ "$?" -eq 5 ] && pass 'unreadable target exits 5' || fail "unreadable target exits 5 (got: $out)"
fi
printf '45 90139\n' >"$TMP/fx/counts-target.txt"

# 7. Promotion requires a fresh, complete, off-machine verified restore point.
python3 -I - "$TMP/backup.json" <<'PY'
import json, sys
with open(sys.argv[1]) as f: record = json.load(f)
record["destinations"]["r2"] = "failed"
with open(sys.argv[1], "w") as f: json.dump(record, f)
PY
reset_log
if out="$(run --mode promote --receipt "$TMP/r6.md" --units-dir "$TMP/units" --containers-file "$TMP/containers-clean.txt" "${promote[@]}" "${common[@]}")"; then
  fail "promote refuses backup without verified R2 copy (exit 0, got: $out)"
else
  [ "$?" -eq 3 ] && log_is_empty && grep -q 'restore point' "$TMP/r6.md" && pass 'promote refuses incomplete backup before database access' || fail "promote refuses incomplete backup before database access (got: $out)"
fi
python3 -I - "$TMP/backup.json" <<'PY'
import json, sys
from datetime import datetime, timezone, timedelta
with open(sys.argv[1]) as f: record = json.load(f)
record["destinations"]["r2"] = "ok"
record["finishedAt"] = (datetime.now(timezone.utc) - timedelta(hours=37)).isoformat()
with open(sys.argv[1], "w") as f: json.dump(record, f)
PY
reset_log
if out="$(run --mode promote --receipt "$TMP/r7.md" --units-dir "$TMP/units" --containers-file "$TMP/containers-clean.txt" "${promote[@]}" "${common[@]}")"; then
  fail "promote refuses stale backup (exit 0, got: $out)"
else
  [ "$?" -eq 3 ] && log_is_empty && grep -q 'restore point' "$TMP/r7.md" && pass 'promote refuses stale backup before database access' || fail "promote refuses stale backup before database access (got: $out)"
fi
python3 -I - "$TMP/backup.json" <<'PY'
import json, sys
from datetime import datetime, timezone, timedelta
with open(sys.argv[1]) as f: record = json.load(f)
record["finishedAt"] = (datetime.now(timezone.utc) - timedelta(minutes=5)).isoformat()
with open(sys.argv[1], "w") as f: json.dump(record, f)
PY

# 8. rollback refuses while a writer is running — and touches nothing.
set_counts '45 90139' '45 90500'
reset_log
if out="$(run --mode rollback --dump-file "$TMP/k8.dump" --receipt "$TMP/r8.md" --units-dir "$TMP/units" --containers-file "$TMP/containers-kaneo-running.txt" "${common[@]}")"; then
  fail "rollback refuses while unfenced (exit 0, got: $out)"
else
  if [ "$?" -eq 3 ] && log_is_empty && grep -q 'REFUSED' "$TMP/r8.md"; then
    pass 'rollback refuses unfenced, writes REFUSED, touches no store'
  else
    fail "rollback refuses unfenced, writes REFUSED, touches no store (got: $out)"
  fi
fi

# 9. rollback replays target→source and passes on exact parity.
set_counts '45 90500' '45 90500'
reset_log
if out="$(run --mode rollback --dump-file "$TMP/k9.dump" --receipt "$TMP/r9.md" --units-dir "$TMP/units" --containers-file "$TMP/containers-clean.txt" "${common[@]}")"; then
  if [ "$(cat "$TMP/fx/log" | tr '\n' ' ')" = "pg_dump:target pg_restore:source " ]; then
    pass 'rollback dumps the target then replays into the source'
  else
    fail "rollback dumps the target then replays into the source (log: $(cat "$TMP/fx/log" | tr '\n' ' '))"
  fi
  case "$out" in
  *'ROLLBACK OK'*FENCED*) pass 'rollback passes when fenced and parity holds' ;;
  *) fail "rollback passes when fenced and parity holds (got: $out)" ;;
  esac
  if grep -q 'ROLLBACK OK' "$TMP/r9.md" && grep -q '45 90500' "$TMP/r9.md" && ! grep -q 'target-db\|source-db' "$TMP/r9.md"; then
    pass 'rollback receipt records the fence, the counts, and no DSN'
  else
    fail 'rollback receipt records the fence, the counts, and no DSN'
  fi
else
  fail "rollback passes when fenced and parity holds (exit $?, got: $out)"
fi

# 10. rollback fails when the replayed source still disagrees with the target.
set_counts '45 90400' '45 90500'
reset_log
if out="$(run --mode rollback --dump-file "$TMP/k10.dump" --receipt "$TMP/r10.md" --units-dir "$TMP/units" --containers-file "$TMP/containers-clean.txt" "${common[@]}")"; then
  fail "rollback fails on replay mismatch (exit 0, got: $out)"
else
  if [ "$?" -eq 4 ] && grep -q 'FAIL — replay parity' "$TMP/r10.md"; then
    pass 'rollback fails on replay mismatch with exit 4'
  else
    fail "rollback fails on replay mismatch with exit 4 (got: $out)"
  fi
fi

# 11. rollback rejects --unreachable: the writer being fenced is local.
reset_log
if run --mode rollback --unreachable --dump-file "$TMP/k11.dump" "${common[@]}" >/dev/null 2>&1; then
  fail 'rollback --unreachable exits 2'
else
  [ "$?" -eq 2 ] && log_is_empty && pass 'rollback --unreachable exits 2 with no subprocess' || fail 'rollback --unreachable exits 2 with no subprocess'
fi

# 12. Contract (#3352): usage errors before any subprocess (exit 2).
reset_log
if run --bogus >/dev/null 2>&1; then fail 'unknown flag exits 2'; else
  [ "$?" -eq 2 ] && pass 'unknown flag exits 2' || fail 'unknown flag exits 2'
fi
if run "${common[@]}" >/dev/null 2>&1; then fail 'missing --mode exits 2'; else
  [ "$?" -eq 2 ] && pass 'missing --mode exits 2' || fail 'missing --mode exits 2'
fi
if run --mode presync --source-dsn-file "$TMP/source.dsn" --target-dsn-file "$TMP/target.dsn" >/dev/null 2>&1; then
  fail 'missing --dump-file for presync exits 2'
else
  [ "$?" -eq 2 ] && pass 'missing --dump-file for presync exits 2' || fail 'missing --dump-file for presync exits 2'
fi
if run --mode promote --source-dsn-file "$TMP/source.dsn" --target-dsn-file "$TMP/target.dsn" >/dev/null 2>&1; then
  fail 'missing backup record exits 2'
else
  [ "$?" -eq 2 ] && pass 'missing backup record exits 2' || fail 'missing backup record exits 2'
fi
if run --mode rollback --source-dsn-file "$TMP/source.dsn" --target-dsn-file "$TMP/target.dsn" >/dev/null 2>&1; then
  fail 'missing --dump-file for rollback exits 2'
else
  [ "$?" -eq 2 ] && pass 'missing --dump-file for rollback exits 2' || fail 'missing --dump-file for rollback exits 2'
fi
if run --mode promote --source-dsn-file "$TMP/nope.dsn" --target-dsn-file "$TMP/target.dsn" >/dev/null 2>&1; then
  fail 'unreadable source DSN exits 2'
else
  [ "$?" -eq 2 ] && pass 'unreadable source DSN exits 2' || fail 'unreadable source DSN exits 2'
fi
log_is_empty && pass 'usage errors ran no subprocess' || fail 'usage errors ran no subprocess'

if ((FAILURES)); then
  printf 'kaneo-cutover.test.sh: %d check(s) failed\n' "$FAILURES"
  exit 1
fi
echo 'LIMITATION: stubbed pg_dump/pg_restore/psql and fixture fence state; these checks verify the gated sequence, not a live cutover.'
echo 'kaneo-cutover.test.sh: all checks passed'
