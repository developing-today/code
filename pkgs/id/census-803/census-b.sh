#!/usr/bin/env bash
# Final census for code#803, batch B: leaf crates, plus feature-variant
# passes for id-core (web/test-utils/test-provider) and id-sync (dht/test-utils).
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

for c in id-core id-config id-caps id-store id-events id-groups id-sync id-directory id-currency-registry; do
  census "$c" -p "$c"
done
census id-core-feats -p id-core --features web,test-utils,test-provider
census id-sync-feats -p id-sync --features dht,test-utils
echo "CENSUS-B DONE"
