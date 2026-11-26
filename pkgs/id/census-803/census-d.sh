#!/usr/bin/env bash
# Final census for code#803, batch D: non-default workspace members
# (shells, android-remote, bevy bridge/presenter).
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

for c in id-native-shell id-hw-shell id-tui-shell id-android-remote id-bevy-bridge id-bevy-presenter; do
  census "$c" -p "$c"
done
echo "CENSUS-D DONE"
