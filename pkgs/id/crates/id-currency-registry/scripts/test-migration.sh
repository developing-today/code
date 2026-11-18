#!/usr/bin/env bash
# test-migration.sh — prove migration 0001 against a real Postgres.
#
# Oracle: issue #2317's acceptance list.
#   - the migration creates the four tables with the uniqueness
#     constraints the design names;
#   - the allocator function hands out non-zero u32s monotonically
#     and refuses to recycle (allocation rows cannot be deleted);
#   - two issuers may both register symbol ABC, but a duplicate
#     currency_id is rejected.
#
# Spins up a throwaway Postgres cluster in a mktemp dir owned by
# this script, runs the assertions, and tears it down. Requires
# initdb/pg_ctl/psql on PATH. Exits non-zero on the first failure.
set -euo pipefail

CRATE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
UP="$CRATE_DIR/migrations/0001_currency_registry.up.sql"
DOWN="$CRATE_DIR/migrations/0001_currency_registry.down.sql"

PGDATA="$(mktemp -d /tmp/id-currency-registry-pg.XXXXXX)"
PGPORT="${PGPORT:-55439}"
export PGDATA

cleanup() {
  pg_ctl -D "$PGDATA" -m immediate stop >/dev/null 2>&1 || true
  case "$PGDATA" in
  /tmp/id-currency-registry-pg.*) rm -rf "$PGDATA" ;;
  esac
}
trap cleanup EXIT

initdb -D "$PGDATA" --auth=trust >/dev/null
pg_ctl -D "$PGDATA" -o "-k $PGDATA -p $PGPORT" -l "$PGDATA/pg.log" -w start >/dev/null
PSQL="psql -h $PGDATA -p $PGPORT -d postgres -v ON_ERROR_STOP=1 -qAt"

$PSQL -f "$UP" >/dev/null
echo "migration applied"

# Allocator: non-zero, monotonic, range-split.
first="$($PSQL -c "SELECT allocate_ledger('currency', 'abc')")"
second="$($PSQL -c "SELECT allocate_ledger('currency', 'def')")"
system="$($PSQL -c "SELECT allocate_ledger('system', 'fees')")"
[ "$first" = "1048576" ] || {
  echo "FAIL: first currency ledger = $first, want 1048576"
  exit 1
}
[ "$second" = "1048577" ] || {
  echo "FAIL: second currency ledger = $second, want 1048577"
  exit 1
}
[ "$system" = "1" ] || {
  echo "FAIL: first system ledger = $system, want 1"
  exit 1
}
echo "allocator: monotonic, non-zero, range-split OK"

# Never recycled: allocation rows cannot be deleted.
if $PSQL -c "DELETE FROM ledger_allocation WHERE ledger_id = $first" >/dev/null 2>&1; then
  echo "FAIL: ledger_allocation delete succeeded; ids could be recycled"
  exit 1
fi
echo "allocator: delete (recycle) refused OK"

# Registry: two issuers may both register symbol ABC.
$PSQL >/dev/null <<'SQL'
INSERT INTO currency (ledger_id, currency_id, name, symbol, scale, issuer_id, graph_object_id)
VALUES (1048576, 'alice-abc', 'Alice ABC', 'ABC', 2, 'alice', 'graph-alice-abc');
INSERT INTO currency (ledger_id, currency_id, name, symbol, scale, issuer_id, graph_object_id)
VALUES (1048577, 'bob-abc', 'Bob ABC', 'ABC', 2, 'bob', 'graph-bob-abc');
INSERT INTO currency_issuer (ledger_id, account_id, role)
VALUES (1048576, 'alice', 'issuer'), (1048577, 'bob', 'issuer');
INSERT INTO account (account_id, ledger_id, owner_id, kind)
VALUES ('0123456789abcdef0123456789abcdef', 1048576, 'alice', 'issuer');
SQL
echo "registry: two issuers with symbol ABC OK"

# Duplicate currency_id is rejected (globally unique, decision 3).
third="$($PSQL -c "SELECT allocate_ledger('currency', 'abc-again')")"
if $PSQL -c "INSERT INTO currency (ledger_id, currency_id, name, symbol, scale, issuer_id)
             VALUES ($third, 'alice-abc', 'Copy', 'XYZ', 2, 'mallory')" >/dev/null 2>&1; then
  echo "FAIL: duplicate currency_id was accepted"
  exit 1
fi
echo "registry: duplicate currency_id rejected OK"

# Duplicate symbol for the same issuer is rejected.
fourth="$($PSQL -c "SELECT allocate_ledger('currency', 'alice-abc-2')")"
if $PSQL -c "INSERT INTO currency (ledger_id, currency_id, name, symbol, scale, issuer_id)
             VALUES ($fourth, 'alice-abc-2', 'Copy', 'ABC', 2, 'alice')" >/dev/null 2>&1; then
  echo "FAIL: duplicate (issuer, symbol) was accepted"
  exit 1
fi
echo "registry: duplicate (issuer, symbol) rejected OK"

# Range split is a schema constraint, not a convention.
if $PSQL -c "INSERT INTO ledger_allocation (ledger_id, kind, owner_ref)
             VALUES (7, 'currency', 'out-of-range')" >/dev/null 2>&1; then
  echo "FAIL: out-of-range currency allocation was accepted"
  exit 1
fi
echo "schema: range split enforced OK"

# Down migration removes the schema.
$PSQL -f "$DOWN" >/dev/null
remaining="$($PSQL -c "SELECT count(*) FROM information_schema.tables WHERE table_name IN ('ledger_allocation','currency','currency_issuer','account')")"
[ "$remaining" = "0" ] || {
  echo "FAIL: down migration left $remaining tables"
  exit 1
}
echo "down migration OK"

echo "PASS: migration 0001 satisfies the #2317 acceptance checks"
