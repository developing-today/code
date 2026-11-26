#!/usr/bin/env bash
# Final census for code#803, batch A: root crate `id` (default features,
# web/ssh/dht feature pass) plus the -D gate confirmation on the root crate.
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

census id-default -p id
census id-feats -p id --features web,ssh,dht

( cargo clippy -p id --all-targets -- \
    -D clippy::assert_is_empty -D clippy::chunks_exact_to_as_chunks \
    > /tmp/gate-id.log 2>&1; echo "GATE id D_RC=$?" )
grep -e 'clippy::assert_is_empty' -e 'clippy::chunks_exact_to_as_chunks' /tmp/gate-id.log | head -20 || true
echo "CENSUS-A DONE"
