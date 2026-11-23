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
# Requires: docker, sqlite3, zstd, curl; images postgres:16-alpine,
# gristlabs/grist-oss, ghcr.io/perpetualsoftware/pad,
# ghcr.io/usekaneo/kaneo (present on the laptop).
set -u
DAY="${1:-$(date +%F)}"
SCRATCH="$HOME/restore-rehearsal-6816"
SRC="$SCRATCH/src"
BK="$HOME/backups/pm-stack/$DAY/pm-stack-$DAY.tar.zst"
T_ALL=$(date +%s)
say() { echo "== $* =="; }
elapsed() { echo "[$1] $(( $(date +%s) - $2 ))s"; }
[ -f "$BK" ] || { echo "backup not found: $BK"; exit 1; }

rm -rf "$SCRATCH"; mkdir -p "$SRC"
T=$(date +%s)
tar --zstd -xf "$BK" -C "$SRC" || { echo "EXTRACT FAILED"; exit 1; }
elapsed "extract" "$T"

say "kaneo restore"
T=$(date +%s)
docker rm -f rehearsal-pg >/dev/null 2>&1
docker run -d --name rehearsal-pg -e POSTGRES_USER=kaneo -e POSTGRES_PASSWORD=rehearsal -e POSTGRES_DB=kaneo -p 127.0.0.1:55433:5432 postgres:16-alpine >/dev/null || { echo "PG START FAILED"; exit 1; }
for i in $(seq 1 30); do docker exec rehearsal-pg pg_isready -U kaneo -d kaneo >/dev/null 2>&1 && break; sleep 1; done
docker exec -i rehearsal-pg psql -U kaneo -d postgres -v ON_ERROR_STOP=0 < "$SRC/kaneo/globals.sql" >/dev/null 2>&1
docker exec -i rehearsal-pg pg_restore -U kaneo -d kaneo --no-owner < "$SRC/kaneo/kaneo.dump" > "$SCRATCH/pg_restore.log" 2>&1
echo "pg_restore exit: $?"
docker exec rehearsal-pg psql -U kaneo -d kaneo -t -c "select 'user_tables=' || count(*) from information_schema.tables where table_schema='public';"
docker exec rehearsal-pg psql -U kaneo -d kaneo -t -c "select 'total_rows=' || coalesce(sum(n_live_tup),0) from pg_stat_user_tables;"
elapsed "kaneo" "$T"

say "grist integrity + boot"
T=$(date +%s)
sqlite3 "file:$SRC/grist/home.sqlite3?mode=ro" "PRAGMA integrity_check;" | head -2
echo "grist doc files: $(ls "$SRC/grist/docs" | wc -l)"
docker rm -f rehearsal-grist >/dev/null 2>&1
docker run -d --name rehearsal-grist -e GRIST_DATA_DIR=/persist -p 127.0.0.1:8488:8484 -v "$SRC/grist":/persist gristlabs/grist-oss:latest >/dev/null || echo "GRIST START FAILED"
GCODE=""
for i in $(seq 1 45); do GCODE=$(curl -s -o /dev/null -w "%{http_code}" http://127.0.0.1:8488/ 2>/dev/null); { [ "$GCODE" = "200" ] || [ "$GCODE" = "302" ]; } && break; sleep 2; done
echo "grist http: $GCODE"
elapsed "grist" "$T"

say "pad integrity + boot"
T=$(date +%s)
cp "$SRC/pad/pad.db" "$SCRATCH/pad-check.db"
sqlite3 "file:$SCRATCH/pad-check.db?mode=ro" "PRAGMA integrity_check;" | head -2
docker rm -f rehearsal-pad >/dev/null 2>&1
docker run -d --name rehearsal-pad -e PAD_PORT=7777 -e PAD_DB=/data/pad.db -w /data -p 127.0.0.1:7778:7777 -v "$SRC/pad":/data ghcr.io/perpetualsoftware/pad:latest >/dev/null || echo "PAD START FAILED"
PCODE=""
for i in $(seq 1 30); do PCODE=$(curl -s -o /dev/null -w "%{http_code}" http://127.0.0.1:7778/api/v1/health 2>/dev/null); [ "$PCODE" = "200" ] && break; sleep 2; done
echo "pad health http: $PCODE"
elapsed "pad" "$T"

say "kaneo app boot"
T=$(date +%s)
docker network create rehearsal-net >/dev/null 2>&1
docker network connect rehearsal-net rehearsal-pg >/dev/null 2>&1
docker rm -f rehearsal-kaneo >/dev/null 2>&1
# AUTH_SECRET is a rehearsal-only dummy here; a real restore takes the
# rendered stack env by name from Bitwarden (runbook step 6).
docker run -d --name rehearsal-kaneo --network rehearsal-net \
  -e DATABASE_URL=postgres://kaneo:rehearsal@rehearsal-pg:5432/kaneo \
  -e AUTH_SECRET=rehearsal-only-secret-not-real-0123456789 \
  -p 127.0.0.1:5174:5173 ghcr.io/usekaneo/kaneo:2.47.0 >/dev/null 2>&1 || echo "KANEO START FAILED"
KCODE=""
for i in $(seq 1 40); do KCODE=$(curl -s -o /dev/null -w "%{http_code}" http://127.0.0.1:5174/api/health 2>/dev/null); [ "$KCODE" = "200" ] && break; sleep 2; done
echo "kaneo health http: ${KCODE:-none}"
elapsed "kaneo-app" "$T"

elapsed "TOTAL" "$T_ALL"
say "cleanup"
docker rm -f rehearsal-pg rehearsal-grist rehearsal-pad rehearsal-kaneo >/dev/null 2>&1
docker network rm rehearsal-net >/dev/null 2>&1
echo "scratch kept at $SCRATCH (src + logs); remove when the record is written"
