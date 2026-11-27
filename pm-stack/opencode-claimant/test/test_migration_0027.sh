#!/bin/bash
# Contract: no arguments. Validate before any file/process effects.
# Oracle: #16066 compare-clear contract, plus all migration-0026 guards.
set -euo pipefail
if (( $# != 0 )); then
  echo 'usage: test_migration_0027.sh (no arguments)' >&2
  exit 2
fi
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
createdb -h "$PGROOT" -p "$PORT" -U postgres migration_0027_test
# Run the old contract against the new definition, then exercise the same
# success/refusal scenarios with the new error-stopped marker and reason.
sed -e 's/opencode-claimant-recovery:gone-session/opencode-claimant-recovery:error-stopped/g' \
    -e 's/bound session verified absent before recovery/verified error-stopped session deleted before recovery/g' \
    migration-0026.test.sql > "$PGROOT/error-stopped.test.sql"
psql -h "$PGROOT" -p "$PORT" -U postgres -d migration_0027_test \
  -v ON_ERROR_STOP=1 -q \
  -f migration-0026.schema.sql \
  -f ../migration-0026.sql \
  -f ../migration-0027.sql \
  -f migration-0026.test.sql \
  -f "$PGROOT/error-stopped.test.sql"
echo 'test_migration_0027.sh: PASS'
