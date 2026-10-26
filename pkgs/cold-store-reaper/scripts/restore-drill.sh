#!/usr/bin/env bash
# restore-drill.sh — the cold store's quarterly restore drill
# (DESIGN.md section 8: an untested archive is a rumour, not a backup).
#
# What it proves, end to end:
#   1. Three pseudo-random cold objects from the local manifest are
#      fetched back through the rehydrator Worker
#      (GET https://cold-store-rehydrator.developing-today.workers.dev/fetch/<sha256>)
#      and each response body's sha256 must equal the object hash —
#      the same check the reaper's own `fetch` performs, but through
#      the production read path (D1 lookup -> Durable Object -> Drive
#      Range pull -> zstd decode -> hash verify).
#   2. The newest d1 backup in Drive is downloaded with get-backup and
#      checked against the live manifest by scripts/drill_compare.py:
#      the backup is loaded into in-memory SQLite and must be a
#      content-equal subset of live D1 (queried over the same REST API
#      backup-d1 uses) — every backup row present with identical values,
#      keyed by each table's primary key. A backup is point-in-time
#      and the manifest is append-mostly, so live rows appended after
#      the backup (the nightly run's own ledger rows, growth since)
#      are reported as a count note, never a failure. If D1 is
#      unreachable, the fallback oracle is the local manifest.
#
# Prints "DRILL PASS:" / "DRILL FAIL:" per item and a final verdict;
# exits non-zero if anything failed.
#
# Environment — the operator supplies these; the script stores nothing
# and echoes no secret values:
#   REHYDRATOR_API_KEY    required. Bearer key for the Worker
#                         (Bitwarden: cold-store.rehydrator-api-key).
#   CLOUDFLARE_API_TOKEN  required. For the live-D1 count check
#                         (Bitwarden: cloudflare.api-token-day-to-day).
#   COLD_STORE_HOME       optional; the reaper home (default
#                         ~/.config/cold-store), home of manifest.sqlite.
set -euo pipefail

: "${REHYDRATOR_API_KEY:?REHYDRATOR_API_KEY must be set — materialize it from Bitwarden into the environment first}"
: "${CLOUDFLARE_API_TOKEN:?CLOUDFLARE_API_TOKEN must be set — materialize it from Bitwarden into the environment first}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
COLD_STORE="${SCRIPT_DIR}/../cold-store"
STORE_HOME="${COLD_STORE_HOME:-${HOME}/.config/cold-store}"
MANIFEST="${STORE_HOME}/manifest.sqlite"
WORKER_BASE="https://cold-store-rehydrator.developing-today.workers.dev"

WORK="$(mktemp -d)"
trap 'rm -rf "${WORK}"' EXIT

FAILURES=0
pass() { echo "DRILL PASS: $1"; }
fail() {
  echo "DRILL FAIL: $1"
  FAILURES=$((FAILURES + 1))
}

# ------------------------------------------------------------ cold objects
python3 - "${MANIFEST}" >"${WORK}/cold-hashes.txt" <<'PY'
import sqlite3
import sys

try:
    conn = sqlite3.connect(f"file:{sys.argv[1]}?mode=ro", uri=True)
    rows = conn.execute(
        "SELECT hash FROM objects WHERE state='cold' ORDER BY hash"
    ).fetchall()
except sqlite3.Error:
    rows = []
for (h,) in rows:
    print(h)
PY
mapfile -t COLD_HASHES <"${WORK}/cold-hashes.txt"

if [[ ${#COLD_HASHES[@]} -eq 0 ]]; then
  fail "no cold objects in the local manifest — nothing to rehydrate"
else
  PICKED="$(printf '%s\n' "${COLD_HASHES[@]}" | shuf | head -n 3)"
  while IFS= read -r obj_hash; do
    [[ -n ${obj_hash} ]] || continue
    out="${WORK}/object.bin"
    if curl -fsS --max-time 120 \
      -H "Authorization: Bearer ${REHYDRATOR_API_KEY}" \
      "${WORKER_BASE}/fetch/${obj_hash}" -o "${out}"; then
      actual="$(sha256sum "${out}" | awk '{print $1}')"
      if [[ ${actual} == "${obj_hash}" ]]; then
        pass "object ${obj_hash} rehydrated via Worker, sha256 verified"
      else
        fail "object ${obj_hash} sha256 mismatch (Worker returned ${actual})"
      fi
    else
      fail "object ${obj_hash} fetch via Worker failed"
    fi
  done <<<"${PICKED}"
fi

# ------------------------------------------------------------ newest d1 backup
BACKUP_NAME="$("${COLD_STORE}" list-backups --db d1 2>/dev/null |
  sed -n 's/.*\(d1-[0-9]*T[0-9]*Z-[^ ]*\).*/\1/p' | head -n 1)" || true
if [[ -z ${BACKUP_NAME} ]]; then
  fail "no d1 backup found in Drive"
elif ! "${COLD_STORE}" get-backup d1 "${BACKUP_NAME}" "${WORK}/backup.sql" >/dev/null; then
  fail "get-backup could not download ${BACKUP_NAME}"
elif python3 "${SCRIPT_DIR}/drill_compare.py" "${WORK}/backup.sql" "${MANIFEST}"; then
  pass "d1 backup ${BACKUP_NAME} loads and is a content-equal subset of the live manifest"
else
  fail "d1 backup ${BACKUP_NAME} does not match the live manifest (see rows above)"
fi

# ------------------------------------------------------------ verdict
if [[ ${FAILURES} -eq 0 ]]; then
  echo "DRILL VERDICT: PASS"
else
  echo "DRILL VERDICT: FAIL (${FAILURES} failure(s))"
  exit 1
fi
