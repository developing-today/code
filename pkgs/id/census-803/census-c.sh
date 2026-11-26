#!/usr/bin/env bash
# Final census for code#803, batch C: id-worlds (ssh,sandbox per the brief,
# plus a test-utils variant), id-web, id-render, id-cli (plus ssh/web/dht
# feature pass), id-client, id-pg, id-grants, id-import-notion.
# Throwaway verification script; not part of the product.
if [ "$#" -ne 0 ]; then echo "usage: $0 (takes no arguments)" >&2; exit 2; fi
cd "$(dirname "$0")/.." || exit 1
export CARGO_TARGET_DIR="$PWD/.census-target"
export CARGO_BUILD_JOBS=2

census() {
  name="$1"; shift
  log="/tmp/census-${name}.log"
  ( cargo clippy "$@" --all-targets --keep-going -- \
      -W clippy::assert_is_empty -W clippy::chunks_exact_to_as_chunks \
      > "$log" 2>&1 || true )
  echo "CENSUS ${name} assert=$(grep -c 'clippy::assert_is_empty' "$log" || true) chunks=$(grep -c 'clippy::chunks_exact_to_as_chunks' "$log" || true) errlines=$(grep -c '^error' "$log" || true)"
  grep -B2 -e 'clippy::assert_is_empty' -e 'clippy::chunks_exact_to_as_chunks' "$log" | grep -E -- '-->' || true
}

census id-worlds -p id-worlds --features ssh,sandbox
census id-worlds-tu -p id-worlds --features ssh,sandbox,test-utils
for c in id-web id-render id-cli id-client id-pg id-grants id-import-notion; do
  census "$c" -p "$c"
done
census id-cli-feats -p id-cli --features ssh,web,dht
census id-web-sandbox -p id-web --features sandbox
echo "CENSUS-C DONE"
