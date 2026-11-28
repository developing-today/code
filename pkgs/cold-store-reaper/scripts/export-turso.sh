#!/usr/bin/env bash
# export-turso.sh — file a portable SQLite export of every Turso
# database into the cold store's Drive db-backups/turso/ folder
# (DESIGN.md section 5), via Turso's HTTP APIs — no turso CLI needed.
#
# How: the Platform API (api.turso.tech) has no dump route, but each
# database's own libsql endpoint serves one: GET https://<hostname>/dump
# with a per-database auth token returns the complete SQL dump. So per
# database: mint a token (POST .../databases/<name>/auth/tokens, with
# the platform token), download /dump with curl (--compressed: the
# uncompressed stream is large enough to be cut mid-transfer in the
# sandbox; compression shrinks the wire size several-fold), run the
# dump through scripts/repair_dump.py (libsql does not escape single
# quotes inside the sqlite_schema INSERTs that register virtual
# tables, so FTS5 databases are otherwise unparseable — #1391),
# scripts/dedupe_dump.py (libsql can dump a table twice; an exact
# repeat is dropped, a differing one fails the export — #12000) and
# scripts/vector_index_dump.py (stock sqlite3 cannot build a libsql
# vector index, so its CREATE INDEX is kept as a recorded statement;
# the index data stays in its shadow tables — #12000),
# require the filtered dump to end in COMMIT; (a truncated dump
# otherwise rebuilds into an integrity-clean but partial database),
# stream it through
# the sqlite3 CLI to rebuild a standalone SQLite file — constant
# memory at any dump size, unlike reading it into python — require
# PRAGMA integrity_check = ok and at least one user table, gzip -6,
# and file it with `cold-store put-backup turso`.
#
# Databases are discovered from the Platform API
# (GET .../organizations/<org>/databases — items carry Name and
# Hostname), so new databases are picked up automatically.
#
# Environment — supplied by the operator/cron; the script stores
# nothing and never prints a token value (platform or per-database):
#   TURSO_PLATFORM_API_TOKEN  required to run. Materialize it from
#                             Bitwarden (turso.platform-api-token-muse-cli)
#                             into the cron environment. When unset the
#                             script prints a skip line and exits 0:
#                             a missing Turso setup never fails the
#                             nightly run, but the skip is never silent.
#   TURSO_ORG                 optional; the Turso organization slug
#                             (default dezren39).
#   TURSO_DUMP_MAX_TIME_SECONDS
#                             optional; curl --max-time for one
#                             database's /dump download (default
#                             3600). Large databases outgrow a short
#                             cap: atlas's dump passed 10 GB by
#                             2026-11-20 and no longer finished
#                             inside the old 1800 s. Each attempt is
#                             also clamped to what is left of
#                             TURSO_EXPORT_DEADLINE_SECONDS.
#   TURSO_EXPORT_DEADLINE_SECONDS
#                             optional; the whole run's budget from
#                             start (default 10800). No database is
#                             started once less than 60 s of it is
#                             left (it fails loudly instead), and a
#                             dump's attempts and retries are kept
#                             inside what is left, so one slow
#                             database cannot use up the nightly
#                             service's start timeout and starve the
#                             steps after this one (2026-11-20: the
#                             run was killed at 2 h mid-atlas).
#   TURSO_EXPORT_LAST         optional; space-separated database
#                             names exported after all the others
#                             (default "atlas"; empty = list order).
#                             atlas is by far the largest database:
#                             listed first, a slow atlas download on
#                             2026-11-20 left the other 49 unfiled.
#   TURSO_DUMP_RETRY_DELAY_SECONDS
#                             optional; curl --retry-delay between the
#                             3 retries of one /dump download (default
#                             30; must be positive).
#   TURSO_DUMP_RETRY_PASS_DELAY_SECONDS
#                             optional; how long to wait before the
#                             next pass over databases that failed the
#                             previous pass (default 60; 0 = no wait
#                             before the first retry pass). On
#                             2026-11-17 the server closed the stream
#                             mid-transfer (curl 18) on 4 of 42
#                             databases twice in a row while others of
#                             the same size filed fine, so a later pass
#                             is the cheap way past a transient window.
#                             Retry passes repeat while download-stage
#                             failures remain and the run's deadline
#                             still leaves room for the wait plus one
#                             useful attempt (#13115: on 2026-11-23 the
#                             atlas /dump stream was cut on all 4 of
#                             curl's own retries AND on the single
#                             second pass, inside a few minutes, while
#                             hours of deadline budget sat unused).
#                             The wait doubles between spread passes,
#                             capped at 900, so a bad window gets time
#                             to pass instead of being hammered. A
#                             delay of 0 asks for no waiting, so the
#                             spread passes do not engage then — the
#                             run ends after the second pass.
#   TURSO_API_BASE            optional; Platform API base URL
#                             (default https://api.turso.tech).
#                             Test seam, mirrors COLD_STORE_S3_BASE.
#   TURSO_DUMP_BASE           optional; when set, dumps are fetched
#                             from <TURSO_DUMP_BASE>/<name>/dump
#                             instead of https://<hostname>/dump.
#                             Test seam.
#   SQLITE3_BIN               optional; the sqlite3 CLI used to
#                             rebuild and verify each dump (default:
#                             sqlite3 from PATH). The rebuild streams
#                             the dump through this binary; if it is
#                             missing, that database fails loudly —
#                             there is no in-memory fallback.
#   COLD_STORE_BIN            optional; the cold-store executable
#                             (default: ../cold-store next to this
#                             script).
#   EXPORT_TURSO_WORK_DIR     optional; the scratch directory for
#                             dumps and rebuilds (default:
#                             ${TMPDIR:-/tmp}/cold-store-export-turso).
#                             One named directory, wiped at startup
#                             and on exit (including SIGTERM, which
#                             systemd sends at its start timeout) -
#                             never a fresh anonymous mktemp dir
#                             (darwin-nix#373). Created mode 0700:
#                             it holds minted database tokens.
#
# A database that fails any stage prints a loud ERROR line naming it;
# the run continues with the remaining databases and exits non-zero
# at the end if any database failed.
# Arguments: none. Reject every argument before skip handling, lock
# creation, network access or file writes.
set -euo pipefail

if (($# > 0)); then
  printf 'export-turso.sh: unexpected argument: %s (usage: takes no arguments)\n' "$1" >&2
  exit 2
fi

if [[ -z ${TURSO_PLATFORM_API_TOKEN:-} ]]; then
  echo "turso export: skipped (TURSO_PLATFORM_API_TOKEN not set)"
  exit 0
fi

ORG="${TURSO_ORG:-dezren39}"
DUMP_MAX_TIME="${TURSO_DUMP_MAX_TIME_SECONDS:-3600}"
case "${DUMP_MAX_TIME}" in
'' | *[!0-9]*)
  echo "turso export: ERROR: TURSO_DUMP_MAX_TIME_SECONDS must be a positive integer (got '${DUMP_MAX_TIME}')" >&2
  exit 1
  ;;
*[1-9]*) ;;
*)
  # All digits but no non-zero digit (0, 00, ...): curl --max-time 0
  # disables the timeout entirely, so zero is not a positive integer.
  echo "turso export: ERROR: TURSO_DUMP_MAX_TIME_SECONDS must be a positive integer (got '${DUMP_MAX_TIME}')" >&2
  exit 1
  ;;
esac
RETRY_DELAY="${TURSO_DUMP_RETRY_DELAY_SECONDS:-30}"
case "${RETRY_DELAY}" in
*[1-9]*)
  [[ ${RETRY_DELAY} != *[!0-9]* ]] || {
    echo "turso export: ERROR: TURSO_DUMP_RETRY_DELAY_SECONDS must be a positive integer (got '${RETRY_DELAY}')" >&2
    exit 1
  }
  ;;
*)
  # Empty, non-numeric or all zeros: curl --retry-delay 0 silently
  # switches to its own backoff, so zero would not mean "no delay".
  echo "turso export: ERROR: TURSO_DUMP_RETRY_DELAY_SECONDS must be a positive integer (got '${RETRY_DELAY}')" >&2
  exit 1
  ;;
esac
RETRY_PASS_DELAY="${TURSO_DUMP_RETRY_PASS_DELAY_SECONDS:-60}"
case "${RETRY_PASS_DELAY}" in
'' | *[!0-9]*)
  echo "turso export: ERROR: TURSO_DUMP_RETRY_PASS_DELAY_SECONDS must be a non-negative integer (got '${RETRY_PASS_DELAY}')" >&2
  exit 1
  ;;
*) ;;
esac
DEADLINE_SECONDS="${TURSO_EXPORT_DEADLINE_SECONDS:-10800}"
case "${DEADLINE_SECONDS}" in
'' | *[!0-9]*)
  echo "turso export: ERROR: TURSO_EXPORT_DEADLINE_SECONDS must be a positive integer (got '${DEADLINE_SECONDS}')" >&2
  exit 1
  ;;
*[1-9]*) ;;
*)
  echo "turso export: ERROR: TURSO_EXPORT_DEADLINE_SECONDS must be a positive integer (got '${DEADLINE_SECONDS}')" >&2
  exit 1
  ;;
esac
# 10#: a leading zero must not read as octal.
DEADLINE=$((SECONDS + 10#${DEADLINE_SECONDS}))
EXPORT_LAST="${TURSO_EXPORT_LAST-atlas}"
API_BASE="${TURSO_API_BASE:-https://api.turso.tech}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
COLD_STORE="${COLD_STORE_BIN:-${SCRIPT_DIR}/../cold-store}"
SQLITE3="${SQLITE3_BIN:-sqlite3}"

# Scratch lives in ONE named directory, never a fresh anonymous
# mktemp dir (darwin-nix#373): the 2026-11-08/09/10 cold-store-nightly
# runs were killed by the service's 30-minute start timeout with
# ~30 GB of accumulated exports in an anonymous /tmp/tmp.* dir; the
# EXIT trap does not run under SIGKILL, so each killed run leaked
# the whole directory and the next run started a new one. A fixed
# location bounds the damage: a killed run leaves at most this one
# directory, and the next run wipes it before starting. The flock
# keeps two concurrent runs (timer + manual) from wiping each
# other's scratch mid-flight. Per-database files are deleted as
# soon as each database is filed (see export_one), so a live run's
# footprint is its largest single database, not the sum of all of
# them. The .lock file itself is 0 bytes and stays.
WORK="${EXPORT_TURSO_WORK_DIR:-${TMPDIR:-/tmp}/cold-store-export-turso}"
exec 9>"${WORK}.lock"
if ! flock -n 9; then
  echo "turso export: ERROR: another export-turso run holds ${WORK}.lock" >&2
  exit 1
fi
rm -rf "${WORK}"
# 0700 / umask 077: WORK holds each database's minted token response
# for the length of its export (2026-11-20: a killed run left one
# world-readable in /tmp until the next night).
umask 077
mkdir -p "${WORK}"
chmod 700 "${WORK}"
trap 'rm -rf "${WORK}"' EXIT
# systemd stops the unit at its start timeout with SIGTERM; without a
# TERM trap bash dies without running the EXIT trap and the scratch
# (10 GB of atlas dump on 2026-11-20) stays until the next run.
trap 'exit 143' TERM
trap 'exit 130' INT

# curl with an Authorization header that never touches argv (#1027):
# the header travels in a curl config fed on stdin (--config -), the
# pattern cold_store.py's _curl_fetch established for #983 — a token in
# -H argv is world-readable in the process list for the whole transfer.
# Usage: curl_auth <token> <curl args...> (token is $1, rest is argv).
curl_auth() {
  local token="$1"
  shift
  local escaped="${token//\\/\\\\}"
  escaped="${escaped//\"/\\\"}"
  printf 'header = "Authorization: Bearer %s"\n' "${escaped}" |
    curl --config - "$@"
}

# ------------------------------------------------------------ discovery
if ! curl_auth "${TURSO_PLATFORM_API_TOKEN}" -fsS --max-time 60 \
  "${API_BASE}/v1/organizations/${ORG}/databases" \
  -o "${WORK}/databases.json"; then
  echo "turso export: ERROR: could not list databases from ${API_BASE} (org ${ORG})" >&2
  exit 1
fi
# Emit one "Name<TAB>Hostname" line per database.
if ! python3 - "${WORK}/databases.json" >"${WORK}/databases.tsv" <<'PY'
import json
import sys

with open(sys.argv[1]) as fh:
    payload = json.load(fh)
items = payload.get("databases", payload) if isinstance(payload, dict) else payload
for item in items:
    print(f"{item['Name']}\t{item['Hostname']}")
PY
then
  echo "turso export: ERROR: could not parse the Platform API database list" >&2
  exit 1
fi
mapfile -t DB_ROWS <"${WORK}/databases.tsv"
if [[ ${#DB_ROWS[@]} -eq 0 ]]; then
  echo "turso export: ERROR: Platform API returned no databases for org ${ORG}" >&2
  exit 1
fi
# TURSO_EXPORT_LAST: move the named (largest) databases to the end so
# a slow one cannot keep the rest from being filed.
if [[ -n ${EXPORT_LAST// /} ]]; then
  ROWS_FIRST=()
  ROWS_LAST=()
  for row in "${DB_ROWS[@]}"; do
    if [[ " ${EXPORT_LAST} " == *" ${row%%$'\t'*} "* ]]; then
      ROWS_LAST+=("${row}")
    else
      ROWS_FIRST+=("${row}")
    fi
  done
  DB_ROWS=("${ROWS_FIRST[@]}" "${ROWS_LAST[@]}")
fi

# ------------------------------------------------------------ one database
export_one_run() {
  local name="$1" hostname="$2"
  local dump_sql="${WORK}/${name}.sql" db_file="${WORK}/${name}.db"
  local filtered_sql="${WORK}/${name}.filtered.sql"
  local jwt dump_url
  local remaining=$((DEADLINE - SECONDS)) dump_max_time="${DUMP_MAX_TIME}"
  if [[ ${remaining} -lt 60 ]]; then
    echo "turso export: ERROR: ${name}: not started, the run's ${DEADLINE_SECONDS}s deadline (TURSO_EXPORT_DEADLINE_SECONDS) is reached" >&2
    return 1
  fi

  # Mint a per-database auth token (the value is never printed).
  if ! curl_auth "${TURSO_PLATFORM_API_TOKEN}" -fsS --max-time 60 -X POST \
    "${API_BASE}/v1/organizations/${ORG}/databases/${name}/auth/tokens" \
    -o "${WORK}/${name}.token.json"; then
    echo "turso export: ERROR: ${name}: could not mint a database auth token" >&2
    return 1
  fi
  if ! jwt="$(
    python3 - "${WORK}/${name}.token.json" <<'PY'
import json
import sys

with open(sys.argv[1]) as fh:
    print(json.load(fh)["jwt"])
PY
  )" || [[ -z ${jwt} ]]; then
    echo "turso export: ERROR: ${name}: token response carried no jwt" >&2
    return 1
  fi

  # Download the SQL dump from the database's own libsql endpoint
  # (curl: the proxy-safe transport in the cron sandbox). --compressed
  # is load-bearing for GB-class dumps: uncompressed, the atlas dump
  # was cut mid-stream (~525 MB in); compressed its wire size drops to
  # ~507 MB total and completes.
  if [[ -n ${TURSO_DUMP_BASE:-} ]]; then
    dump_url="${TURSO_DUMP_BASE}/${name}/dump"
  else
    dump_url="https://${hostname}/dump"
  fi
  # --retry-max-time keeps the retries inside the run's deadline.
  remaining=$((DEADLINE - SECONDS))
  if [[ ${remaining} -lt 1 ]]; then remaining=1; fi
  if [[ ${remaining} -lt $((10#${dump_max_time})) ]]; then dump_max_time="${remaining}"; fi
  if ! curl_auth "${jwt}" -fsS --compressed --max-time "${dump_max_time}" \
    --retry 3 --retry-delay "${RETRY_DELAY}" --retry-all-errors \
    --retry-max-time "${remaining}" \
    "${dump_url}" -o "${dump_sql}"; then
    echo "turso export: ERROR: ${name}: dump download failed (${dump_url})" >&2
    # The one stage a later retry pass can plausibly fix (#13115):
    # run_pass records it so the deadline-spread passes keep retrying
    # this database while leaving deterministic failures alone.
    FAIL_STAGE=download
    return 1
  fi
  unset jwt

  # Non-sensitive provenance for failed reconstructions. The dump itself
  # remains in the private scratch directory and is never printed.
  local dump_bytes dump_sha256
  dump_bytes="$(stat -c %s "${dump_sql}")" || return 1
  dump_sha256="$(sha256sum "${dump_sql}")" || return 1
  dump_sha256="${dump_sha256%% *}"
  echo "turso export: ${name}: downloaded dump bytes=${dump_bytes} sha256=${dump_sha256}"

  # Repair pass (#1391): libsql registers virtual tables with an
  # INSERT INTO sqlite_schema whose embedded SQL is emitted verbatim
  # — single quotes inside it (an FTS5 content='...' table is the
  # common case) are not doubled, so the literal terminates early and
  # the rebuild below dies with a syntax error. repair_dump.py
  # streams the dump (constant memory, one statement buffered) and
  # rewrites only verifiably mis-escaped schema literals with proper
  # '' doubling; everything else passes through byte-identical.
  #
  # Repeat pass (#12000): libsql's /dump can emit a table twice (the
  # atlas vector table came out whole, then whole again, byte for
  # byte), and the rebuild then stops on the second copy's first
  # primary key. dedupe_dump.py drops a repeated copy only when its
  # CREATE statement and every row match the first copy exactly, and
  # fails (exit 1) when they differ, so no row is dropped silently.
  #
  # Vector index pass (#12000): a libsql vector index is dumped as
  # CREATE INDEX ... (libsql_vector_idx(...)), which stock sqlite3
  # cannot run, so the rebuild died on the dump's last statement.
  # vector_index_dump.py records that statement in the table
  # cold_store_libsql_vector_indexes instead; the index's data is in
  # its shadow tables, which are dumped and rebuilt as plain tables.
  if ! python3 "${SCRIPT_DIR}/repair_dump.py" <"${dump_sql}" |
    python3 "${SCRIPT_DIR}/dedupe_dump.py" |
    python3 "${SCRIPT_DIR}/vector_index_dump.py" >"${filtered_sql}"; then
    echo "turso export: ERROR: ${name}: dump repair filter failed" >&2
    return 1
  fi

  # Truncation guard, applied to the filtered stream: a dump cut
  # mid-stream ends inside its open transaction, yet rebuilds into
  # a database whose integrity_check
  # still says ok (the uncommitted tail silently rolls back). A
  # complete libsql dump's last non-empty line is its COMMIT. The
  # scan streams (awk holds one line), so this is safe at any size.
  local last_line
  last_line="$(awk 'NF { line = $0 } END { print line }' "${filtered_sql}")"
  if [[ ${last_line} != "COMMIT;" ]]; then
    echo "turso export: ERROR: ${name}: dump appears truncated (last non-empty line is not COMMIT;)" >&2
    return 1
  fi

  # Rebuild by streaming the filtered dump through the sqlite3 CLI —
  # constant memory, unlike the previous python3 path which read the whole
  # dump into RAM and was OOM-killed on GB-class dumps. -bail makes
  # any bad statement a hard failure. If the CLI is missing this
  # database fails loudly; there is deliberately no fallback.
  if ! command -v "${SQLITE3}" >/dev/null 2>&1; then
    echo "turso export: ERROR: ${name}: sqlite3 CLI not found (SQLITE3_BIN=${SQLITE3}); cannot rebuild the export" >&2
    return 1
  fi
  local rebuild_err="${WORK}/${name}.rebuild.err"
  if ! "${SQLITE3}" -bail "${db_file}" <"${filtered_sql}" 2>"${rebuild_err}"; then
    cat "${rebuild_err}" >&2
    echo "turso export: ERROR: ${name}: dump did not rebuild into a SQLite file" >&2
    # The dump is deleted with the work dir, so say now what the
    # failing statement was (#12000): its table, and whether the dump
    # repeats it verbatim. Row values are never printed.
    local err_line report
    err_line="$(sed -nE '0,/ near line [0-9]+:/s/^(Parse error|Runtime error|Error) near line ([0-9]+):.*/\2/p' "${rebuild_err}")"
    if [[ -n ${err_line} ]]; then
      report="$(python3 "${SCRIPT_DIR}/dump_line_report.py" "${filtered_sql}" "${err_line}" 2>&1)" ||
        report="line ${err_line}: report failed: ${report}"
      echo "turso export: ${name}: ${report}" >&2
    fi
    return 1
  fi
  # Verify the finished file (the CLI reads it; the dump text is
  # never loaded whole again): integrity_check must say ok, and the
  # file must hold at least one user table — an export that lost its
  # transaction tail is integrity-clean but empty.
  local integrity
  if ! integrity="$("${SQLITE3}" -readonly "${db_file}" "PRAGMA integrity_check;")" ||
    [[ ${integrity} != "ok" ]]; then
    echo "turso export: ERROR: ${name}: rebuilt file failed integrity_check (${integrity:-unreadable})" >&2
    return 1
  fi
  local user_tables
  if ! user_tables="$("${SQLITE3}" -readonly "${db_file}" \
    "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%';")" ||
    [[ ! ${user_tables} =~ ^[0-9]+$ ]] || [[ ${user_tables} -lt 1 ]]; then
    echo "turso export: ERROR: ${name}: rebuilt file has no user tables (dump may be truncated or empty)" >&2
    return 1
  fi

  if ! gzip -6 -f "${db_file}"; then
    echo "turso export: ERROR: ${name}: gzip failed" >&2
    return 1
  fi
  if ! "${COLD_STORE}" put-backup turso "${db_file}.gz"; then
    echo "turso export: ERROR: ${name}: cold-store put-backup failed" >&2
    return 1
  fi
  local gz_size
  if ! gz_size="$(stat -c %s "${db_file}.gz")"; then
    echo "turso export: ERROR: ${name}: could not stat the gzipped export" >&2
    return 1
  fi
  echo "turso export: filed ${name} (${gz_size} bytes gzipped)"
}

# export_one wraps export_one_run with per-database scratch cleanup:
# a database's dump, filtered dump, rebuilt file, gzipped export and
# minted-token response are deleted as soon as its export finishes,
# pass or fail, instead of accumulating in WORK until the run ends.
# Only the run-level files (databases.json / databases.tsv) live for
# the whole run, and WORK itself is wiped at the next run's start.
export_one() {
  local name="$1" hostname="$2"
  local rc=0
  FAIL_STAGE=other
  export_one_run "${name}" "${hostname}" || rc=$?
  rm -f "${WORK}/${name}.sql" "${WORK}/${name}.filtered.sql" \
    "${WORK}/${name}.rebuild.err" \
    "${WORK}/${name}.db" "${WORK}/${name}.db.gz" \
    "${WORK}/${name}.token.json"
  return "${rc}"
}

# One pass over a list of "Name<TAB>Hostname" rows; the rows that
# failed are left in PASS_FAILED for the caller, and the subset whose
# failure was the /dump download stage in PASS_FAILED_DOWNLOAD.
run_pass() {
  PASS_FAILED=()
  PASS_FAILED_DOWNLOAD=()
  local row name hostname
  for row in "$@"; do
    name="${row%%$'\t'*}"
    hostname="${row##*$'\t'}"
    # shellcheck disable=SC2310 # by design: export_one guards every
    # stage itself and returns 1, so one database's failure is
    # recorded here instead of aborting the run via set -e.
    if ! export_one "${name}" "${hostname}"; then
      PASS_FAILED+=("${row}")
      if [[ ${FAIL_STAGE} == download ]]; then
        PASS_FAILED_DOWNLOAD+=("${row}")
      fi
    fi
  done
}

run_pass "${DB_ROWS[@]}"
# Second pass: a database whose download was cut off is tried once
# more at the end of the run, after a pause, rather than failing the
# whole step on one transient window (2026-11-17: 4 of 42 failed
# inside a few minutes, all with curl 18).
if [[ ${#PASS_FAILED[@]} -gt 0 ]]; then
  echo "turso export: retrying ${#PASS_FAILED[@]} failed database(s) after ${RETRY_PASS_DELAY}s"
  sleep "${RETRY_PASS_DELAY}"
  run_pass "${PASS_FAILED[@]}"
fi
# Further passes (#13115): only the download-stage failures keep
# being retried — a rebuild or filing failure is deterministic and a
# later window cannot fix it, but a cut /dump stream can (2026-11-23:
# the atlas stream was cut on curl's own retries AND on the second
# pass, inside a few minutes, while hours of deadline budget sat
# unused). The passes repeat while download failures remain AND the
# run's deadline still leaves room for the wait plus one useful
# attempt window (60s — the same floor export_one applies before
# starting a database). The wait doubles between passes, capped at
# 900. A RETRY_PASS_DELAY of 0 means the operator asked for no
# waiting, so there is nothing to spread with and the spread passes
# do not engage — the run ends after the second pass, as before.
FINAL_FAILED=()
for row in "${PASS_FAILED[@]}"; do
  still_download=0
  for drow in "${PASS_FAILED_DOWNLOAD[@]}"; do
    if [[ ${drow} == "${row}" ]]; then
      still_download=1
      break
    fi
  done
  if [[ ${still_download} -eq 0 ]]; then FINAL_FAILED+=("${row}"); fi
done
retry_delay="${RETRY_PASS_DELAY}"
while [[ ${#PASS_FAILED_DOWNLOAD[@]} -gt 0 && 10#${retry_delay} -gt 0 ]]; do
  remaining=$((DEADLINE - SECONDS))
  if [[ ${remaining} -lt $((10#${retry_delay} + 60)) ]]; then
    echo "turso export: ${#PASS_FAILED_DOWNLOAD[@]} database(s) still failing and only ${remaining}s of the run's deadline remains — no room for another retry pass" >&2
    break
  fi
  echo "turso export: retrying ${#PASS_FAILED_DOWNLOAD[@]} download-failed database(s) after ${retry_delay}s (${remaining}s of the run's deadline remains)"
  sleep "${retry_delay}"
  run_pass "${PASS_FAILED_DOWNLOAD[@]}"
  # A database whose download now succeeds can still fail a later
  # stage; that failure is final, so move it out of the retry set.
  for row in "${PASS_FAILED[@]}"; do
    still_download=0
    for drow in "${PASS_FAILED_DOWNLOAD[@]}"; do
      if [[ ${drow} == "${row}" ]]; then
        still_download=1
        break
      fi
    done
    if [[ ${still_download} -eq 0 ]]; then FINAL_FAILED+=("${row}"); fi
  done
  retry_delay=$((10#${retry_delay} * 2))
  if [[ ${retry_delay} -gt 900 ]]; then retry_delay=900; fi
done
# Whatever download failures remain (no room, or no spreading) are
# final too.
FINAL_FAILED+=("${PASS_FAILED_DOWNLOAD[@]}")
FAILURES=${#FINAL_FAILED[@]}

if [[ ${FAILURES} -gt 0 ]]; then
  echo "turso export: ${FAILURES} of ${#DB_ROWS[@]} database(s) FAILED" >&2
  exit 1
fi
echo "turso export: all ${#DB_ROWS[@]} database(s) filed"
