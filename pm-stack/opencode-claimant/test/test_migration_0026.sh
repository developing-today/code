#!/bin/bash
# Runner for the migration-0026 focused tests (#5400).
#
# Starts a throwaway Postgres cluster in a mktemp directory (socket
# only, no TCP), applies the minimal schema, then ../migration-0026.sql,
# then runs the scenario file in migration-0026.test.sql with
# ON_ERROR_STOP. Exit 0 = all scenarios passed. The cluster and its
# data directory are removed on exit; nothing touches any live DB.
set -euo pipefail

cd "$(dirname "$0")"

PGROOT="$(mktemp -d)"
PORT=55439
cleanup() {
  pg_ctl -D "$PGROOT/data" -m immediate stop >/dev/null 2>&1 || true
  rm -rf "$PGROOT"
}
trap cleanup EXIT

initdb -D "$PGROOT/data" -U postgres --auth=trust >/dev/null
pg_ctl -D "$PGROOT/data" -l "$PGROOT/pg.log" -w \
  -o "-k $PGROOT -p $PORT -c listen_addresses=''" start >/dev/null
createdb -h "$PGROOT" -p "$PORT" -U postgres migration_0026_test

psql -h "$PGROOT" -p "$PORT" -U postgres -d migration_0026_test \
  -v ON_ERROR_STOP=1 -q \
  -f migration-0026.schema.sql \
  -f ../migration-0026.sql \
  -f migration-0026.test.sql

echo "test_migration_0026.sh: PASS"
