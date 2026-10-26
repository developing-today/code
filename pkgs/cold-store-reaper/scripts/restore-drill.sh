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
#   2. The newest d1 backup in Drive is downloaded with get-backup,
#      loaded into an in-memory SQLite database, and its per-table row
#      counts must equal the live D1 counts (queried over the same REST
#      API backup-d1 uses). If D1 is unreachable, the fallback oracle is
#      the latest local manifest's row counts.
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

if [[ "${#COLD_HASHES[@]}" -eq 0 ]]; then
  fail "no cold objects in the local manifest — nothing to rehydrate"
else
  PICKED="$(printf '%s\n' "${COLD_HASHES[@]}" | shuf | head -n 3)"
  while IFS= read -r obj_hash; do
    [[ -n "${obj_hash}" ]] || continue
    out="${WORK}/object.bin"
    if curl -fsS --max-time 120 \
      -H "Authorization: Bearer ${REHYDRATOR_API_KEY}" \
      "${WORKER_BASE}/fetch/${obj_hash}" -o "${out}"; then
      actual="$(sha256sum "${out}" | awk '{print $1}')"
      if [[ "${actual}" = "${obj_hash}" ]]; then
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
BACKUP_NAME="$("${COLD_STORE}" list-backups --db d1 2>/dev/null \
  | sed -n 's/.*\(d1-[0-9]*T[0-9]*Z-[^ ]*\).*/\1/p' | head -n 1)" || true
if [[ -z "${BACKUP_NAME}" ]]; then
  fail "no d1 backup found in Drive"
elif ! "${COLD_STORE}" get-backup d1 "${BACKUP_NAME}" "${WORK}/backup.sql" >/dev/null; then
  fail "get-backup could not download ${BACKUP_NAME}"
elif python3 - "${WORK}/backup.sql" "${MANIFEST}" <<'PY'
import json
import os
import sqlite3
import sys
import urllib.request

export_path, manifest_path = sys.argv[1], sys.argv[2]
ACCOUNT = "234b866b99ce2b142d7963d8d139cbfa"
DATABASE = "15d5f460-835c-4f24-b229-222908a5b30c"
TABLES_SQL = (
    "SELECT name, sql FROM sqlite_master WHERE type='table'"
    " AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '_cf_%'"
)


def d1(sql):
    req = urllib.request.Request(
        f"https://api.cloudflare.com/client/v4/accounts/{ACCOUNT}"
        f"/d1/database/{DATABASE}/query",
        data=json.dumps({"sql": sql}).encode(),
        method="POST",
        headers={
            "Authorization": "Bearer " + os.environ["CLOUDFLARE_API_TOKEN"],
            "Content-Type": "application/json",
        },
    )
    with urllib.request.urlopen(req, timeout=60) as resp:
        payload = json.loads(resp.read())
    if not payload.get("success"):
        raise RuntimeError("D1 query failed")
    return payload["result"][0]["results"]


try:
    meta = d1(TABLES_SQL)
    creates = [(r["name"], r["sql"]) for r in meta]
    expected = {
        r["name"]: d1(f'SELECT count(*) AS n FROM "{r["name"]}"')[0]["n"]
        for r in meta
    }
    source = "live D1"
except Exception:
    conn = sqlite3.connect(f"file:{manifest_path}?mode=ro", uri=True)
    creates = conn.execute(TABLES_SQL).fetchall()
    expected = {
        name: conn.execute(f'SELECT count(*) FROM "{name}"').fetchone()[0]
        for name, _sql in creates
    }
    conn.close()
    source = "local manifest (D1 unreachable)"

mem = sqlite3.connect(":memory:")
for _name, sql in creates:
    if sql:
        mem.execute(sql)
with open(export_path) as f:
    mem.executescript(f.read())
ok = True
for name, want in sorted(expected.items()):
    got = mem.execute(f'SELECT count(*) FROM "{name}"').fetchone()[0]
    print(f"  {name}: backup has {got} row(s), {source} has {want}")
    if got != want:
        ok = False
sys.exit(0 if ok else 1)
PY
then
  pass "d1 backup ${BACKUP_NAME} loads and row counts match"
else
  fail "d1 backup ${BACKUP_NAME} row counts do not match"
fi

# ------------------------------------------------------------ verdict
if [[ "${FAILURES}" -eq 0 ]]; then
  echo "DRILL VERDICT: PASS"
else
  echo "DRILL VERDICT: FAIL (${FAILURES} failure(s))"
  exit 1
fi
