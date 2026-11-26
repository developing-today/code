#!/usr/bin/env bash
# Final census for code#803, batch F (completion pass):
# (1) JSON-format census for every crate not yet JSON-checked in batch E
#     (batch A-D human-mode counts matched cargo unit-summary lines that
#     repeat the -W flags, so only JSON counts are diagnostic-true).
# (2) Exact file:line sites for the real diagnostics batch E found in
#     id-caps and id-events, via human-mode warning blocks.
# Throwaway verification script; not part of the product.
if [ "$#" -ne 0 ]; then echo "usage: $0 (takes no arguments)" >&2; exit 2; fi
cd "$(dirname "$0")/.." || exit 1
export CARGO_TARGET_DIR="$PWD/.census-target"
export CARGO_BUILD_JOBS=2

jcensus() {
  name="$1"; shift
  log="/tmp/jcensus-${name}.log"
  ( cargo clippy "$@" --all-targets --keep-going --message-format json -- \
      -W clippy::assert_is_empty -W clippy::chunks_exact_to_as_chunks \
      > "$log" 2>&1 || true )
  echo "JCENSUS ${name} assert=$(grep -c 'clippy::assert_is_empty' "$log" || true) chunks=$(grep -c 'clippy::chunks_exact_to_as_chunks' "$log" || true) errordiags=$(grep -c '"level":"error"' "$log" || true)"
}

jcensus id-core -p id-core
jcensus id-core-feats -p id-core --features web,test-utils,test-provider
jcensus id-sync -p id-sync
jcensus id-sync-feats -p id-sync --features dht,test-utils
jcensus id-directory -p id-directory
jcensus id-web -p id-web
jcensus id-web-sandbox -p id-web --features sandbox
jcensus id-render -p id-render
jcensus id-cli -p id-cli
jcensus id-cli-feats -p id-cli --features ssh,web,dht
jcensus id-client -p id-client
jcensus id-pg -p id-pg
jcensus id-grants -p id-grants
jcensus id-import-notion -p id-import-notion
jcensus id-hw-shell -p id-hw-shell
jcensus id-android-remote -p id-android-remote
jcensus id-caps -p id-caps
jcensus id-events -p id-events

for c in id-caps id-events; do
  ( cargo clippy -p "$c" --all-targets --keep-going -- \
      -W clippy::assert_is_empty -W clippy::chunks_exact_to_as_chunks \
      > "/tmp/sites-${c}.log" 2>&1 || true )
  echo "SITES ${c} (warning blocks around the two census lints):"
  grep -B6 -e 'clippy::assert_is_empty' -e 'clippy::chunks_exact_to_as_chunks' "/tmp/sites-${c}.log" | grep -e '^warning' -e '-->' | cut -c1-210 | head -40 || true
done
echo "CENSUS-F DONE"
