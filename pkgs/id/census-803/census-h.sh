#!/usr/bin/env bash
# Final census for code#803, batch H (gap closure): id-groups was covered
# only by the batch-B human-mode count (cargo unit-summary artifact), so
# run its JSON census plus site extraction here for completeness.
# Throwaway verification script; not part of the product.
if [ "$#" -ne 0 ]; then echo "usage: $0 (takes no arguments)" >&2; exit 2; fi
cd "$(dirname "$0")/.." || exit 1
export CARGO_TARGET_DIR="$PWD/.census-target"
export CARGO_BUILD_JOBS=2

log="/tmp/jcensus-id-groups.log"
( cargo clippy -p id-groups --all-targets --keep-going --message-format json -- \
    -W clippy::assert_is_empty -W clippy::chunks_exact_to_as_chunks \
    > "$log" 2>&1 || true )
echo "JCENSUS id-groups assert=$(grep -c 'clippy::assert_is_empty' "$log" || true) chunks=$(grep -c 'clippy::chunks_exact_to_as_chunks' "$log" || true) errordiags=$(grep -c '"level":"error"' "$log" || true)"
grep -e 'clippy::assert_is_empty' -e 'clippy::chunks_exact_to_as_chunks' "$log" | while IFS= read -r line; do
  f=$(printf '%s' "$line" | grep -o '"file_name":"[^"]*"' | head -1)
  l=$(printf '%s' "$line" | grep -o '"line_start":[0-9]*' | head -1)
  echo "SITE id-groups ${f} ${l}"
done
echo "CENSUS-H DONE"
