#!/usr/bin/env bash
# kaneo-cutover — the gated Kaneo → cluster-postgres cutover driver.
#
# Implements the Kaneo half of pm-stack/STORE-MIGRATION-PLAN-6816.md §3-§5
# for issue #6816. Four modes:
#
#   preflight offline target identity check only; no PG/fence calls or writes.
#
#   presync  while live: pg_dump the source `kaneo` database, pg_restore it
#            into the explicitly declared cluster target, then verify schema parity (same
#            public table count). Row drift is expected here — the final
#            delta lands after the writers are fenced — and is reported,
#            not failed.
#   promote  the gate before the DSN flip. Requires BOTH:
#              (a) doc/pm-stack/COLD-RESTORE-RUNBOOK.md step 1: the old
#                  writer is FENCED, proven by pm-stack/scripts/fence-check.sh
#                  (no container running, no unit active, fail closed on
#                  anything unreadable); and
#              (b) final-delta parity: target table count AND exact row
#                  count equal the source's.
#              (c) a complete, verified backup generation finished within
#                  the accepted freshness window (default 36 hours).
#            Only then is a cutover receipt written and PROMOTE OK printed.
#            The script never rewrites the DSN itself: it gates the step
#            and records the evidence, so no configuration can promote
#            while a second writer is alive.
#   rollback the gate before reopening the local writer (§5). Requires the
#            promoted writer FENCED — the DSN flip renames no container,
#            so the existing writer set proves no writer runs against
#            either store — then exports the window's writes (pg_dump of
#            the target) and replays them into the source, demanding exact
#            table/row parity afterwards. Only then is a rollback receipt
#            written and ROLLBACK OK printed. --unreachable is refused
#            here: the writer being fenced is local, not gone.
#            The driver replays the delta but never flips a DSN or
#            restarts a service: the window operator does that per the
#            runbook, attaching this receipt to the window record.
#
# Acceptance language this implements (#6816 criteria 2 and 4): "the old
# writer is fenced before any failover promotion; no configuration permits
# two active writers" and "rollback restores the prior writer without
# losing writes" (the replay half; the DSN flip and restart stay window
# operator steps per the runbook).
#
# Secrets: DSNs are read from files (--source-dsn-file / --target-dsn-file)
# and never echoed, logged, or written to the receipt. Credentials resolve
# by name through the standing secrets route; nothing here stores one.
#
# Argument contract (#3352): flags only, no positionals. `--opt value` and
# `--opt=value` behave identically. Anything else is a usage error (exit 2)
# before any subprocess runs:
#   --mode preflight|presync|promote|rollback required
#   --target-home cluster-postgres required; Supabase is not a supported target
#   --target-host HOST         required; must match target URI's TCP host
# URI-only DSNs: explicit host/database, postgres or postgresql scheme. Only
# sslmode, connect_timeout and application_name query options are supported.
# No service/endpoint overrides, sockets, multi-host or general conninfo.
#   --source-dsn-file FILE     required; file holding the source DSN
#   --target-dsn-file FILE     required; file holding the target DSN
#   --dump-file FILE           required for presync and rollback; the dump to write
#   --receipt FILE             write the receipt here (default: stdout only)
#   --pg-bin-dir DIR           find pg_dump/pg_restore/psql in DIR (tests)
#   --fence-check PATH         fence checker (default: sibling fence-check.sh)
#   --units-dir DIR            passed to fence-check (fixture mode)
#   --containers-file FILE     passed to fence-check (fixture mode)
#   --backup-record-file FILE  latest pm-dashboard-backup .last-run.json;
#                              required for promote
#   --max-backup-age-hours N   freshness bound for the backup (default 36)
#   --unreachable              passed to fence-check: old host unreachable.
#                              Refused in rollback mode (exit 2): the writer
#                              being fenced there is local.
#   -h, --help                 usage, exit 0
#
# Exit codes: 0 pass, 2 usage, 3 not fenced (promotion/rollback refused),
# 4 parity failure, 5 command failure.
set -uo pipefail

MODE=""
TARGET_HOME=""
TARGET_HOST=""
SOURCE_DSN_FILE=""
TARGET_DSN_FILE=""
DUMP_FILE=""
RECEIPT=""
PG_BIN_DIR=""
FENCE_CHECK=""
UNREACHABLE_FLAG=0
FENCE_ARGS=()
BACKUP_RECORD_FILE=""
MAX_BACKUP_AGE_HOURS=36

# Exact counts for every public table, in one query: table count, then the
# sum of per-table exact count(*). Exact (not pg_stat estimates) so the
# promote gate can demand equality instead of a tolerance.
COUNT_SQL="select count(*) || ' ' || coalesce(sum(c), 0) from (select (xpath('/row/c/text()', query_to_xml(format('select count(*) as c from %I.%I', schemaname, relname), false, true, '')))[1]::text::bigint as c from pg_stat_user_tables where schemaname = 'public') s"

usage() {
  cat <<'USAGE'
usage: kaneo-cutover.sh --mode preflight|presync|promote|rollback --source-dsn-file FILE
                        --target-dsn-file FILE --target-home cluster-postgres
                        --target-host HOST [--dump-file FILE]
                        [--receipt FILE] [--pg-bin-dir DIR]
                        [--fence-check PATH] [--units-dir DIR]
                        [--containers-file FILE] [--unreachable]
                        [--backup-record-file FILE]
                        [--max-backup-age-hours N] [-h|--help]

  preflight offline URI identity validation only; no backups/fence/dump/receipt
            required, no database calls or file writes. All modes require the
            same distinct source and declared cluster-postgres target identity.
            Only explicit single TCP host postgres/postgresql URIs supported;
            no Supabase endpoints, muse-dev2 identifiers or endpoint overrides.
  presync  dump the source, restore into the target, verify schema parity.
  promote  require a fresh complete backup, the old writer FENCED, and exact
           table/row parity, then write the cutover receipt. Exit: 0 pass,
           2 usage, 3 promotion refused, 4 parity failure, 5 command failure.
  rollback fence the promoted writer, replay the target delta into the
           source, require exact parity, write the rollback receipt.
           --unreachable is refused. Exit codes as promote.
USAGE
}

fail_usage() {
  echo "kaneo-cutover: $1" >&2
  usage >&2
  exit 2
}

while [ "$#" -gt 0 ]; do
  case "$1" in
  -h | --help)
    usage
    exit 0
    ;;
  --unreachable)
    FENCE_ARGS+=("--unreachable")
    UNREACHABLE_FLAG=1
    shift
    ;;
  --mode | --target-home | --target-host | --source-dsn-file | --target-dsn-file | --dump-file | --receipt | --pg-bin-dir | --fence-check | --units-dir | --containers-file | --backup-record-file | --max-backup-age-hours)
    opt="$1"
    [ "$#" -ge 2 ] || fail_usage "$opt requires a value"
    val="$2"
    [ -n "$val" ] || fail_usage "$opt requires a non-empty value"
    case "$opt" in
    --mode) MODE="$val" ;;
    --target-home) TARGET_HOME="$val" ;;
    --target-host) TARGET_HOST="$val" ;;
    --source-dsn-file) SOURCE_DSN_FILE="$val" ;;
    --target-dsn-file) TARGET_DSN_FILE="$val" ;;
    --dump-file) DUMP_FILE="$val" ;;
    --receipt) RECEIPT="$val" ;;
    --pg-bin-dir) PG_BIN_DIR="$val" ;;
    --fence-check) FENCE_CHECK="$val" ;;
    --backup-record-file) BACKUP_RECORD_FILE="$val" ;;
    --max-backup-age-hours) MAX_BACKUP_AGE_HOURS="$val" ;;
    --units-dir) FENCE_ARGS+=("--units-dir" "$val") ;;
    --containers-file) FENCE_ARGS+=("--containers-file" "$val") ;;
    esac
    shift 2
    ;;
  --mode=* | --target-home=* | --target-host=* | --source-dsn-file=* | --target-dsn-file=* | --dump-file=* | --receipt=* | --pg-bin-dir=* | --fence-check=* | --units-dir=* | --containers-file=* | --backup-record-file=* | --max-backup-age-hours=*)
    opt="${1%%=*}"
    val="${1#*=}"
    [ -n "$val" ] || fail_usage "$opt requires a non-empty value"
    case "$opt" in
    --mode) MODE="$val" ;;
    --target-home) TARGET_HOME="$val" ;;
    --target-host) TARGET_HOST="$val" ;;
    --source-dsn-file) SOURCE_DSN_FILE="$val" ;;
    --target-dsn-file) TARGET_DSN_FILE="$val" ;;
    --dump-file) DUMP_FILE="$val" ;;
    --receipt) RECEIPT="$val" ;;
    --pg-bin-dir) PG_BIN_DIR="$val" ;;
    --fence-check) FENCE_CHECK="$val" ;;
    --backup-record-file) BACKUP_RECORD_FILE="$val" ;;
    --max-backup-age-hours) MAX_BACKUP_AGE_HOURS="$val" ;;
    --units-dir) FENCE_ARGS+=("--units-dir" "$val") ;;
    --containers-file) FENCE_ARGS+=("--containers-file" "$val") ;;
    esac
    shift
    ;;
  --*)
    fail_usage "unexpected argument '$1'"
    ;;
  *)
    fail_usage "unexpected argument '$1' (no positionals)"
    ;;
  esac
done

case "$MODE" in
preflight | presync | promote | rollback) ;;
"") fail_usage "--mode is required (preflight, presync, promote or rollback)" ;;
*) fail_usage "--mode must be preflight, presync, promote or rollback (got '$MODE')" ;;
esac

[ "$TARGET_HOME" = cluster-postgres ] || fail_usage "--target-home cluster-postgres is required"
[ -n "$TARGET_HOST" ] || fail_usage "--target-host is required"
[ -n "$SOURCE_DSN_FILE" ] || fail_usage "--source-dsn-file is required"
[ -n "$TARGET_DSN_FILE" ] || fail_usage "--target-dsn-file is required"
[ -r "$SOURCE_DSN_FILE" ] || fail_usage "source DSN file '$SOURCE_DSN_FILE' is not readable"
[ -r "$TARGET_DSN_FILE" ] || fail_usage "target DSN file '$TARGET_DSN_FILE' is not readable"
if [ "$MODE" = "presync" ]; then
  [ -n "$DUMP_FILE" ] || fail_usage "--dump-file is required for presync"
fi
if [ "$MODE" = "promote" ]; then
  [ -n "$BACKUP_RECORD_FILE" ] || fail_usage "--backup-record-file is required for promote"
  [ -r "$BACKUP_RECORD_FILE" ] || fail_usage "backup record file '$BACKUP_RECORD_FILE' is not readable"
  case "$MAX_BACKUP_AGE_HOURS" in
  '' | *[!0-9]*) fail_usage "--max-backup-age-hours must be a positive integer" ;;
  0) fail_usage "--max-backup-age-hours must be a positive integer" ;;
  esac
fi
if [ "$MODE" = "rollback" ]; then
  [ -n "$DUMP_FILE" ] || fail_usage "--dump-file is required for rollback"
  [ "$UNREACHABLE_FLAG" -eq 0 ] || fail_usage "--unreachable is for failover promote only; rollback fences the local promoted writer"
fi

# Argument validation above must finish before any helper subprocess. This
# check only reads the DSN files; preflight returns before operational gates.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [ "$MODE" = preflight ]; then
  python3 -I "$SCRIPT_DIR/kaneo-target-preflight.py" "$SOURCE_DSN_FILE" "$TARGET_DSN_FILE" "$TARGET_HOST" || exit $?
  exit 0
fi
python3 -I "$SCRIPT_DIR/kaneo-target-preflight.py" "$SOURCE_DSN_FILE" "$TARGET_DSN_FILE" "$TARGET_HOST" || exit $?
[ -n "$FENCE_CHECK" ] || FENCE_CHECK="$SCRIPT_DIR/fence-check.sh"

if [ -n "$PG_BIN_DIR" ]; then
  [ -d "$PG_BIN_DIR" ] || fail_usage "--pg-bin-dir '$PG_BIN_DIR' is not a directory"
  PG_DUMP="$PG_BIN_DIR/pg_dump"
  PG_RESTORE="$PG_BIN_DIR/pg_restore"
  PSQL="$PG_BIN_DIR/psql"
else
  PG_DUMP="pg_dump"
  PG_RESTORE="pg_restore"
  PSQL="psql"
fi

[ -x "$FENCE_CHECK" ] || {
  echo "kaneo-cutover: fence checker '$FENCE_CHECK' is not executable — refusing to gate a promotion on an absent check" >&2
  exit 5
}

read_dsn() {
  # Reads a DSN without echoing it. Trailing newline dropped.
  local v
  v="$(cat "$1")" || return 1
  printf '%s' "$v"
}

counts_of() {
  # $1 = "source"|"target" label, $2 = dsn. Prints "tables rows". Never echoes the DSN.
  local dsn out
  dsn="$2"
  if ! out="$("$PSQL" -Atq -c "$COUNT_SQL" -d "$dsn" 2>&1)"; then
    echo "kaneo-cutover: cannot read counts from the $1 database: $out" >&2
    return 5
  fi
  out="$(printf '%s' "$out" | tr -s '[:space:]' ' ' | sed 's/^ //; s/ $//')"
  case "$out" in
  [0-9]*" "[0-9]*) printf '%s' "$out" ;;
  *)
    echo "kaneo-cutover: unexpected count output from the $1 database: '$out'" >&2
    return 5
    ;;
  esac
}

NOW="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
RECEIPT_BODY=""
write_receipt() {
  # $1 = verdict line, $2 = extra detail lines (optional)
  RECEIPT_BODY="# Kaneo cutover receipt — mode $MODE — $NOW

- Fence: ${FENCE_LINE:-not checked (presync)} 
- Source tables/rows: ${SRC_COUNTS:-unknown}
- Target tables/rows: ${TGT_COUNTS:-unknown}
- Verdict: $1
${2:-}"
  if [ -n "$RECEIPT" ]; then
    printf '%s\n' "$RECEIPT_BODY" >"$RECEIPT" || {
      echo "kaneo-cutover: cannot write receipt '$RECEIPT'" >&2
      exit 5
    }
  fi
}

source_dsn="$(read_dsn "$SOURCE_DSN_FILE")" || {
  echo "kaneo-cutover: cannot read '$SOURCE_DSN_FILE'" >&2
  exit 5
}
target_dsn="$(read_dsn "$TARGET_DSN_FILE")" || {
  echo "kaneo-cutover: cannot read '$TARGET_DSN_FILE'" >&2
  exit 5
}

if [ "$MODE" = "presync" ]; then
  echo "kaneo-cutover: presync — dumping source to '$DUMP_FILE'"
  "$PG_DUMP" -Fc -f "$DUMP_FILE" -d "$source_dsn" || {
    echo "kaneo-cutover: pg_dump failed" >&2
    exit 5
  }
  echo "kaneo-cutover: presync — restoring into the target"
  "$PG_RESTORE" --no-owner --no-privileges -d "$target_dsn" "$DUMP_FILE" || {
    echo "kaneo-cutover: pg_restore failed" >&2
    exit 5
  }

  SRC_COUNTS="$(counts_of source "$source_dsn")" || exit $?
  TGT_COUNTS="$(counts_of target "$target_dsn")" || exit $?
  src_tables="${SRC_COUNTS%% *}"
  tgt_tables="${TGT_COUNTS%% *}"
  if [ "$src_tables" != "$tgt_tables" ]; then
    write_receipt "FAIL — schema parity: source $src_tables public tables, target $tgt_tables"
    echo "kaneo-cutover: FAIL — schema parity: source $src_tables public tables, target $tgt_tables" >&2
    exit 4
  fi
  if [ "$SRC_COUNTS" != "$TGT_COUNTS" ]; then
    write_receipt "PASS (schema) — row drift before the final delta: source $SRC_COUNTS, target $TGT_COUNTS" "- Note: expected pre-fence drift; promote requires exact parity."
    echo "kaneo-cutover: presync OK — $src_tables public tables; row drift until the final delta (source $SRC_COUNTS, target $TGT_COUNTS)"
    exit 0
  fi
  write_receipt "PASS (schema + rows) — source and target both $SRC_COUNTS"
  echo "kaneo-cutover: presync OK — $SRC_COUNTS on both sides"
  exit 0
fi

# --- rollback: fence the promoted writer first, then replay the window's
# writes back into the source. The DSN flip renames no container, so the
# existing writer set proves no writer runs against either store. The
# driver replays the delta but never flips a DSN or restarts a service:
# the window operator does that per the runbook, attaching the receipt.
if [ "$MODE" = "rollback" ]; then
  if ! FENCE_OUT="$("$FENCE_CHECK" ${FENCE_ARGS[@]+"${FENCE_ARGS[@]}"} 2>&1)" || [ "${FENCE_OUT%%:*}" != "FENCED" ]; then
    FENCE_LINE="UNFENCED — ${FENCE_OUT:-fence-check produced no output}"
    write_receipt "REFUSED — rollback requires the promoted writer fenced"
    echo "kaneo-cutover: REFUSED — promoted writer not fenced; no rollback, no replay. $FENCE_OUT" >&2
    exit 3
  fi
  FENCE_LINE="$FENCE_OUT"

  echo "kaneo-cutover: rollback — exporting the target delta to '$DUMP_FILE'"
  "$PG_DUMP" -Fc -f "$DUMP_FILE" -d "$target_dsn" || {
    echo "kaneo-cutover: pg_dump of the target failed" >&2
    exit 5
  }
  echo "kaneo-cutover: rollback — replaying into the source"
  "$PG_RESTORE" --no-owner --no-privileges -d "$source_dsn" "$DUMP_FILE" || {
    echo "kaneo-cutover: pg_restore into the source failed" >&2
    exit 5
  }

  SRC_COUNTS="$(counts_of source "$source_dsn")" || exit $?
  TGT_COUNTS="$(counts_of target "$target_dsn")" || exit $?
  if [ "$SRC_COUNTS" != "$TGT_COUNTS" ]; then
    write_receipt "FAIL — replay parity: source $SRC_COUNTS, target $TGT_COUNTS"
    echo "kaneo-cutover: FAIL — replay parity: source $SRC_COUNTS, target $TGT_COUNTS (re-export and replay before reopening the local writer)" >&2
    exit 4
  fi

  write_receipt "ROLLBACK OK — fenced, delta replayed, source and target both $SRC_COUNTS"
  echo "kaneo-cutover: ROLLBACK OK — $FENCE_OUT; source and target both $SRC_COUNTS"
  if [ -z "$RECEIPT" ]; then
    printf '\n%s\n' "$RECEIPT_BODY"
  fi
  exit 0
fi

# --- promote: fence first, fail closed, before anything touches the stores ---
if ! FENCE_OUT="$("$FENCE_CHECK" ${FENCE_ARGS[@]+"${FENCE_ARGS[@]}"} 2>&1)" || [ "${FENCE_OUT%%:*}" != "FENCED" ]; then
  FENCE_LINE="UNFENCED — ${FENCE_OUT:-fence-check produced no output}"
  write_receipt "REFUSED — promotion requires the old writer fenced"
  echo "kaneo-cutover: REFUSED — old writer not fenced; no promotion, no receipt of pass. $FENCE_OUT" >&2
  exit 3
fi
FENCE_LINE="$FENCE_OUT"

# Reject absent, incomplete, unverified, stale, or future-dated generations
# before querying either database. The collector and gate share the same
# local/R2/Drive definition of a complete backup restore point.
RESTORE_POINT_OUT="$(python3 -I "$SCRIPT_DIR/restore-point-check.py" \
  --record "$BACKUP_RECORD_FILE" --max-age-hours "$MAX_BACKUP_AGE_HOURS" 2>&1)" || {
  RESTORE_POINT_LINE="REFUSED — $RESTORE_POINT_OUT"
  write_receipt "REFUSED — promotion requires a fresh verified backup restore point" "- Restore point: $RESTORE_POINT_LINE"
  echo "kaneo-cutover: REFUSED — promotion requires a fresh verified backup restore point; no database queried. $RESTORE_POINT_OUT" >&2
  exit 3
}
RESTORE_POINT_LINE="$RESTORE_POINT_OUT"

SRC_COUNTS="$(counts_of source "$source_dsn")" || exit $?
TGT_COUNTS="$(counts_of target "$target_dsn")" || exit $?
if [ "$SRC_COUNTS" != "$TGT_COUNTS" ]; then
  write_receipt "FAIL — final-delta parity: source $SRC_COUNTS, target $TGT_COUNTS"
  echo "kaneo-cutover: FAIL — final-delta parity: source $SRC_COUNTS, target $TGT_COUNTS (replay the delta before promoting)" >&2
  exit 4
fi

write_receipt "PASS — fresh verified backup, fenced, and source and target both $SRC_COUNTS" "- Restore point: $RESTORE_POINT_LINE"
echo "kaneo-cutover: PROMOTE OK — $FENCE_OUT; $RESTORE_POINT_OUT; source and target both $SRC_COUNTS"
if [ -z "$RECEIPT" ]; then
  printf '\n%s\n' "$RECEIPT_BODY"
fi
