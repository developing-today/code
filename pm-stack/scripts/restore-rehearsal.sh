#!/usr/bin/env bash
# Isolated cold-restore rehearsal for the pm-stack (#6816).
#
# Restores one #1514 daily backup generation into scratch containers on
# loopback-only alternate ports and verifies each store (pg_restore +
# counts, SQLite integrity_check, HTTP smoke). Production containers and
# volumes are never touched. This is a rehearsal driver, NOT the
# laptop-off drill: it runs on the backup host and uses the local copy.
#
# Usage: restore-rehearsal.sh [YYYY-MM-DD]   (default: today's generation)
# Contract: at most one calendar date; flags/unknown/excess args exit 2
# before any subprocess or write. No options are accepted.
# Requires: docker, sqlite3, zstd, curl; images postgres:16-alpine,
# gristlabs/grist-oss, ghcr.io/perpetualsoftware/pad,
# ghcr.io/usekaneo/kaneo (present on the laptop).
set -euo pipefail
usage_error() { echo "invalid argument: $1; usage: restore-rehearsal.sh [YYYY-MM-DD]" >&2; exit 2; }
if (($# > 1)); then usage_error "$2"; fi
if (($# == 1)); then
  [[ $1 =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || usage_error "$1"
  year=$((10#${1:0:4})); month=$((10#${1:5:2})); day=$((10#${1:8:2}))
  ((year >= 1 && month >= 1 && month <= 12 && day >= 1)) || usage_error "$1"
  max_day=31
  case $month in
    4|6|9|11) max_day=30 ;;
    2) max_day=28; if ((year % 4 == 0 && (year % 100 != 0 || year % 400 == 0))); then max_day=29; fi ;;
  esac
  ((day <= max_day)) || usage_error "$1"
  DAY=$1
else
  DAY=$(date +%F)
fi
SCRATCH="$HOME/restore-rehearsal-6816"
SRC="$SCRATCH/src"
BK="$HOME/backups/pm-stack/$DAY/pm-stack-$DAY.tar.zst"
T_ALL=$(date +%s)
say() { echo "== $* =="; }
elapsed() { echo "[$1] $(($(date +%s) - $2))s"; }
fail() { echo "$*" >&2; exit 1; }
cleanup() {
  local rc=$?
  trap - EXIT
  say "cleanup"
  docker rm -f rehearsal-pg rehearsal-grist rehearsal-pad rehearsal-kaneo >/dev/null 2>&1 || rc=1
  docker network rm rehearsal-net >/dev/null 2>&1 || rc=1
  echo "scratch kept at $SCRATCH (src + logs); remove when the record is written"
  exit "$rc"
}
integrity_check() {
  local result
  result=$(sqlite3 "file:$1?mode=ro" "PRAGMA integrity_check;") || fail "SQLITE CHECK FAILED: $1"
  [[ $result == ok ]] || fail "SQLITE INTEGRITY FAILED: $1: $result"
}
http_ready() {
  local label=$1 attempts=$2 url=$3 allow_redirect=${4:-no} code i
  for ((i=0; i<attempts; i++)); do
    if code=$(curl --connect-timeout 2 --max-time 5 -s -o /dev/null -w "%{http_code}" "$url" 2>/dev/null); then
      if [[ $code == 200 || ( $allow_redirect == yes && $code == 302 ) ]]; then
        echo "$label http: $code"
        return 0
      fi
    fi
    sleep 2
  done
  fail "$label HTTP TIMEOUT (last status: ${code:-none})"
}
[ -f "$BK" ] || {
  echo "backup not found: $BK"
  exit 1
}

trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
rm -rf "$SCRATCH"
mkdir -p "$SRC"
T=$(date +%s)
tar --zstd -xf "$BK" -C "$SRC" || {
  echo "EXTRACT FAILED"
  exit 1
}
elapsed "extract" "$T"

say "kaneo restore"
T=$(date +%s)
docker rm -f rehearsal-pg >/dev/null 2>&1 || true
docker run -d --name rehearsal-pg -e POSTGRES_USER=kaneo -e POSTGRES_PASSWORD=rehearsal -e POSTGRES_DB=kaneo -p 127.0.0.1:55433:5432 postgres:16-alpine >/dev/null || {
  echo "PG START FAILED"
  exit 1
}
PG_READY=no
for ((i=0; i<30; i++)); do
  if docker exec rehearsal-pg pg_isready -U kaneo -d kaneo >/dev/null 2>&1; then PG_READY=yes; break; fi
  sleep 1
done
[[ $PG_READY == yes ]] || fail "PG READINESS TIMEOUT"
# The scratch bootstrap role already exists; keep legacy globals SQL tolerance.
# Connection/process failures still fail, and the database restore is strict.
docker exec -i rehearsal-pg psql -U kaneo -d postgres -v ON_ERROR_STOP=0 <"$SRC/kaneo/globals.sql" >/dev/null 2>&1
docker exec -i rehearsal-pg pg_restore -U kaneo -d kaneo --no-owner <"$SRC/kaneo/kaneo.dump" >"$SCRATCH/pg_restore.log" 2>&1 || fail "PG RESTORE FAILED (see $SCRATCH/pg_restore.log)"
echo "pg_restore exit: 0"
docker exec rehearsal-pg psql -U kaneo -d kaneo -t -c "select 'user_tables=' || count(*) from information_schema.tables where table_schema='public';"
docker exec rehearsal-pg psql -U kaneo -d kaneo -t -c "select 'total_rows=' || coalesce(sum(n_live_tup),0) from pg_stat_user_tables;"
elapsed "kaneo" "$T"

say "grist integrity + boot"
T=$(date +%s)
integrity_check "$SRC/grist/home.sqlite3"
[[ -d "$SRC/grist/docs" ]] || fail "GRIST DOCS MISSING: $SRC/grist/docs"
DOC_COUNT=$(ls "$SRC/grist/docs" | wc -l) || fail "GRIST DOCS COUNT FAILED"
echo "grist doc files: $DOC_COUNT"
docker rm -f rehearsal-grist >/dev/null 2>&1 || true
docker run -d --name rehearsal-grist -e GRIST_DATA_DIR=/persist -p 127.0.0.1:8488:8484 -v "$SRC/grist":/persist gristlabs/grist-oss:latest >/dev/null || fail "GRIST START FAILED"
http_ready grist 45 http://127.0.0.1:8488/ yes
elapsed "grist" "$T"

say "pad integrity + boot"
T=$(date +%s)
cp "$SRC/pad/pad.db" "$SCRATCH/pad-check.db"
integrity_check "$SCRATCH/pad-check.db"
docker rm -f rehearsal-pad >/dev/null 2>&1 || true
docker run -d --name rehearsal-pad -e PAD_PORT=7777 -e PAD_DB=/data/pad.db -w /data -p 127.0.0.1:7778:7777 -v "$SRC/pad":/data ghcr.io/perpetualsoftware/pad:latest >/dev/null || fail "PAD START FAILED"
http_ready 'pad health' 30 http://127.0.0.1:7778/api/v1/health
elapsed "pad" "$T"

say "kaneo app boot"
T=$(date +%s)
docker network create rehearsal-net >/dev/null 2>&1
docker network connect rehearsal-net rehearsal-pg >/dev/null 2>&1
docker rm -f rehearsal-kaneo >/dev/null 2>&1 || true
# AUTH_SECRET is a rehearsal-only dummy here; a real restore takes the
# rendered stack env by name from Bitwarden (runbook step 6).
docker run -d --name rehearsal-kaneo --network rehearsal-net \
  -e DATABASE_URL=postgres://kaneo:rehearsal@rehearsal-pg:5432/kaneo \
  -e AUTH_SECRET=rehearsal-only-secret-not-real-0123456789 \
  -p 127.0.0.1:5174:5173 ghcr.io/usekaneo/kaneo:2.47.0 >/dev/null 2>&1 || fail "KANEO START FAILED"
http_ready 'kaneo health' 40 http://127.0.0.1:5174/api/health
elapsed "kaneo-app" "$T"

elapsed "TOTAL" "$T_ALL"
