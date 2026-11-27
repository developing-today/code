#!/usr/bin/env bash
# Test-only PostgreSQL + PostgREST fixture (#13764).
# Usage: bash pkgs/id/scripts/test-postgrest-contract.sh
# No arguments are accepted. Docker, curl and the id Rust toolchain must be
# available. The generated network and containers belong only to this run.
set -euo pipefail
if (( $# != 0 )); then
  printf 'Usage: %s (unexpected argument: %s)\n' "$0" "$1" >&2
  exit 2
fi

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
name="id-pg-contract-$$-$RANDOM"
network_created=0
postgres_created=0
postgrest_created=0
cleanup() {
  local result=$?
  if (( result != 0 )); then
    if (( postgrest_created )); then docker logs "$name-rest" >&2 || true; fi
    if (( postgres_created )); then docker logs "$name-db" >&2 || true; fi
  fi
  if (( postgrest_created )); then docker rm -f "$name-rest" >/dev/null || true; fi
  if (( postgres_created )); then docker rm -f "$name-db" >/dev/null || true; fi
  if (( network_created )); then docker network rm "$name" >/dev/null || true; fi
  return "$result"
}
trap cleanup EXIT

docker network create "$name" >/dev/null
network_created=1
docker run -d --name "$name-db" --network "$name" --network-alias postgres \
  -e POSTGRES_PASSWORD=postgres postgres:17.7 >/dev/null
postgres_created=1
ready=0
for attempt in {1..60}; do
  if docker exec "$name-db" pg_isready -U postgres >/dev/null 2>&1; then ready=1; break; fi
  sleep 1
done
if (( ! ready )); then echo 'PostgreSQL did not become ready' >&2; exit 1; fi
docker exec -i "$name-db" psql -v ON_ERROR_STOP=1 -U postgres -d postgres \
  < "$root/pkgs/id/crates/id-pg/tests/fixtures/postgrest/init.sql"

docker run -d --name "$name-rest" --network "$name" -p 127.0.0.1::3000 \
  -e PGRST_DB_URI=postgres://authenticator:fixture_password@postgres:5432/postgres \
  -e PGRST_DB_SCHEMAS=api -e PGRST_DB_ANON_ROLE=fixture_reader \
  -e PGRST_JWT_SECRET=contract-fixture-signing-secret-long-enough \
  postgrest/postgrest:v13.0.7 >/dev/null
postgrest_created=1
port=$(docker port "$name-rest" 3000/tcp)
url="http://${port}"
ready=0
for attempt in {1..60}; do
  if curl --fail --silent "$url/" >/dev/null; then ready=1; break; fi
  sleep 1
done
if (( ! ready )); then echo 'PostgREST did not become ready' >&2; exit 1; fi

export ID_PG_POSTGREST_URL="$url"
cargo test --manifest-path "$root/pkgs/id/Cargo.toml" --locked -p id-pg --tests
cargo test --manifest-path "$root/pkgs/id/Cargo.toml" --locked -p id-pg --test postgrest_contract -- --ignored
