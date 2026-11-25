#!/usr/bin/env bash
# Apply the shared-schema migrations to one PostgreSQL store, in
# version order, once each (#14256).
#
# Command line (the whole contract, #3352): apply-migrations.sh
# --database-url <url>
# The URL is the store's connection string, supplied by the caller
# (in CI, from the repository's secret store by the workflow); it is
# never printed. Anything else on the command line is a usage error.
#
# A ledger table (public.pm_schema_migrations) records what has been
# applied. A migration already in the ledger is a no-op and is
# reported as one ("no-op: already applied <file>"); a new one is
# applied in one transaction with its ledger row, so a failed
# migration records nothing and can be run again.
set -euo pipefail

usage() { echo "usage: apply-migrations.sh --database-url <url>" >&2; exit 2; }

database_url=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --database-url)
      [[ $# -ge 2 && -n "$2" ]] || usage
      database_url="$2"; shift 2 ;;
    *) usage ;;
  esac
done
[[ -n "$database_url" ]] || usage
command -v psql >/dev/null 2>&1 || { echo "psql is required" >&2; exit 1; }

schema_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
migrations="$schema_dir/migrations"

psql_store() { psql -X -v ON_ERROR_STOP=1 "$database_url" "$@"; }

psql_store -qAt -c 'CREATE TABLE IF NOT EXISTS public.pm_schema_migrations (
  name text PRIMARY KEY,
  applied_at timestamptz NOT NULL DEFAULT now()
);' >/dev/null

shopt -s nullglob
ups=("$migrations"/*.up.sql)
[[ ${#ups[@]} -gt 0 ]] || { echo "no migrations found in $migrations" >&2; exit 1; }

applied=0
skipped=0
for up in "${ups[@]}"; do
  name="$(basename "$up")"
  already="$(psql_store -qAt -c "SELECT count(*) FROM public.pm_schema_migrations WHERE name = '$name';")"
  if [[ "$already" != "0" ]]; then
    echo "no-op: already applied $name"
    skipped=$((skipped + 1))
    continue
  fi
  # One transaction: the migration and its ledger row land together
  # or not at all.
  { echo "BEGIN;"; cat "$up"; echo "INSERT INTO public.pm_schema_migrations (name) VALUES ('$name');"; echo "COMMIT;"; } \
    | psql_store -q >/dev/null
  echo "applied $name"
  applied=$((applied + 1))
done
echo "migrations: $applied applied, $skipped already applied (no-op)"
