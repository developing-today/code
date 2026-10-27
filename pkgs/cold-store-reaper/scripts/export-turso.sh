#!/usr/bin/env bash
# export-turso.sh — file a portable SQLite export of every Turso
# database into the cold store's Drive db-backups/turso/ folder
# (DESIGN.md section 5), via Turso's HTTP APIs — no turso CLI needed.
#
# How: the Platform API (api.turso.tech) has no dump route, but each
# database's own libsql endpoint serves one: GET https://<hostname>/dump
# with a per-database auth token returns the complete SQL dump. So per
# database: mint a token (POST .../databases/<name>/auth/tokens, with
# the platform token), download /dump with curl, rebuild a standalone
# SQLite file with python3, require PRAGMA integrity_check = ok,
# gzip -6, and file it with `cold-store put-backup turso`.
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
#                             1800). Large databases (atlas is
#                             ~500 MB of SQL) outgrow the old fixed
#                             300 s cap; the download also gets one
#                             whole-request retry on transport
#                             failure. Small databases are unaffected.
#   TURSO_API_BASE            optional; Platform API base URL
#                             (default https://api.turso.tech).
#                             Test seam, mirrors COLD_STORE_S3_BASE.
#   TURSO_DUMP_BASE           optional; when set, dumps are fetched
#                             from <TURSO_DUMP_BASE>/<name>/dump
#                             instead of https://<hostname>/dump.
#                             Test seam.
#   COLD_STORE_BIN            optional; the cold-store executable
#                             (default: ../cold-store next to this
#                             script).
#
# A database that fails any stage prints a loud ERROR line naming it;
# the run continues with the remaining databases and exits non-zero
# at the end if any database failed.
set -euo pipefail

if [[ -z ${TURSO_PLATFORM_API_TOKEN:-} ]]; then
  echo "turso export: skipped (TURSO_PLATFORM_API_TOKEN not set)"
  exit 0
fi

ORG="${TURSO_ORG:-dezren39}"
DUMP_MAX_TIME="${TURSO_DUMP_MAX_TIME_SECONDS:-1800}"
case "${DUMP_MAX_TIME}" in
  '' | *[!0-9]*)
    echo "turso export: ERROR: TURSO_DUMP_MAX_TIME_SECONDS must be a positive integer (got '${DUMP_MAX_TIME}')" >&2
    exit 1
    ;;
  *) ;;
esac
API_BASE="${TURSO_API_BASE:-https://api.turso.tech}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
COLD_STORE="${COLD_STORE_BIN:-${SCRIPT_DIR}/../cold-store}"

WORK="$(mktemp -d)"
trap 'rm -rf "${WORK}"' EXIT

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

# ------------------------------------------------------------ one database
export_one() {
  local name="$1" hostname="$2"
  local dump_sql="${WORK}/${name}.sql" db_file="${WORK}/${name}.db"
  local jwt dump_url

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
  # (curl: the proxy-safe transport in the cron sandbox).
  if [[ -n ${TURSO_DUMP_BASE:-} ]]; then
    dump_url="${TURSO_DUMP_BASE}/${name}/dump"
  else
    dump_url="https://${hostname}/dump"
  fi
  if ! curl_auth "${jwt}" -fsS --max-time "${DUMP_MAX_TIME}" \
    --retry 1 --retry-all-errors \
    "${dump_url}" -o "${dump_sql}"; then
    echo "turso export: ERROR: ${name}: dump download failed (${dump_url})" >&2
    return 1
  fi
  unset jwt

  # Rebuild a standalone SQLite file; integrity must check out.
  if ! python3 - "${dump_sql}" "${db_file}" <<'PY'
import sqlite3
import sys

src, dst = sys.argv[1], sys.argv[2]
with open(src) as fh:
    sql = fh.read()
con = sqlite3.connect(dst)
try:
    con.executescript(sql)
    row = con.execute("PRAGMA integrity_check").fetchone()
finally:
    con.close()
if row is None or row[0] != "ok":
    print(f"integrity_check returned {row}", file=sys.stderr)
    sys.exit(1)
PY
  then
    echo "turso export: ERROR: ${name}: dump did not rebuild to an integrity-clean SQLite file" >&2
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

FAILURES=0
for row in "${DB_ROWS[@]}"; do
  name="${row%%$'\t'*}"
  hostname="${row##*$'\t'}"
  # shellcheck disable=SC2310 # by design: export_one guards every
  # stage itself and returns 1, so one database's failure is counted
  # here instead of aborting the run via set -e.
  if ! export_one "${name}" "${hostname}"; then
    FAILURES=$((FAILURES + 1))
  fi
done

if [[ ${FAILURES} -gt 0 ]]; then
  echo "turso export: ${FAILURES} of ${#DB_ROWS[@]} database(s) FAILED" >&2
  exit 1
fi
echo "turso export: all ${#DB_ROWS[@]} database(s) filed"
