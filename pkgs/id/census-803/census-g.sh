#!/usr/bin/env bash
# Final census for code#803, batch G (site extraction): for the five crates
# where batches E/F found real clippy::assert_is_empty diagnostics
# (id-core 1, id-web 2, id-import-notion 5, id-caps 2, id-events 5), re-run
# the JSON census and print the primary span (file + line) of each
# diagnostic for the two census lints.
# Throwaway verification script; not part of the product.
if [ "$#" -ne 0 ]; then echo "usage: $0 (takes no arguments)" >&2; exit 2; fi
cd "$(dirname "$0")/.." || exit 1
export CARGO_TARGET_DIR="$PWD/.census-target"
export CARGO_BUILD_JOBS=2

sites() {
  crate="$1"; shift
  log="/tmp/sites-${crate}.log"
  ( cargo clippy -p "$crate" "$@" --all-targets --keep-going --message-format json -- \
      -W clippy::assert_is_empty -W clippy::chunks_exact_to_as_chunks \
      > "$log" 2>&1 || true )
  echo "SITES ${crate} assert=$(grep -c 'clippy::assert_is_empty' "$log" || true) chunks=$(grep -c 'clippy::chunks_exact_to_as_chunks' "$log" || true)"
  grep -e 'clippy::assert_is_empty' -e 'clippy::chunks_exact_to_as_chunks' "$log" | while IFS= read -r line; do
    f=$(printf '%s' "$line" | grep -o '"file_name":"[^"]*"' | head -1)
    l=$(printf '%s' "$line" | grep -o '"line_start":[0-9]*' | head -1)
    echo "SITE ${crate} ${f} ${l}"
  done
}

sites id-core
sites id-web
sites id-import-notion
sites id-caps
sites id-events
echo "CENSUS-G DONE"
