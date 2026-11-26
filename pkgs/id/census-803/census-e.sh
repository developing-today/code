#!/usr/bin/env bash
# Final census for code#803, batch E (probe): disambiguation pass.
# (1) Show the actual human-mode lines matching the lint names for a small
#     crate (id-config) to identify what the batch A-D counts matched.
# (2) JSON-message-format census (one JSON line per diagnostic; the lint
#     code string appears only in real diagnostics) for representative and
#     error-flagged crates.
# (3) Characterize the '^error' lines seen for id-caps / id-events /
#     id-native-shell / id-bevy-presenter in batches B and D.
# Throwaway verification script; not part of the product.
if [ "$#" -ne 0 ]; then echo "usage: $0 (takes no arguments)" >&2; exit 2; fi
cd "$(dirname "$0")/.." || exit 1
export CARGO_TARGET_DIR="$PWD/.census-target"
export CARGO_BUILD_JOBS=2

( cargo clippy -p id-config --all-targets --keep-going -- \
    -W clippy::assert_is_empty -W clippy::chunks_exact_to_as_chunks \
    > /tmp/probe-config.log 2>&1 || true )
echo "PROBE id-config lines matching clippy::assert_is_empty:"
grep -e 'clippy::assert_is_empty' /tmp/probe-config.log | head -3 | cut -c1-200 || true
echo "PROBE id-config lines starting with warning:"
grep -c '^warning' /tmp/probe-config.log || true

jcensus() {
  name="$1"; shift
  log="/tmp/jcensus-${name}.log"
  ( cargo clippy "$@" --all-targets --keep-going --message-format json -- \
      -W clippy::assert_is_empty -W clippy::chunks_exact_to_as_chunks \
      > "$log" 2>&1 || true )
  echo "JCENSUS ${name} assert=$(grep -c 'clippy::assert_is_empty' "$log" || true) chunks=$(grep -c 'clippy::chunks_exact_to_as_chunks' "$log" || true) errordiags=$(grep -c '"level":"error"' "$log" || true)"
}

jcensus id-config -p id-config
jcensus id-caps -p id-caps
jcensus id-events -p id-events
jcensus id-native-shell -p id-native-shell
jcensus id-bevy-presenter -p id-bevy-presenter
jcensus id-store -p id-store
jcensus id-worlds -p id-worlds --features ssh,sandbox
jcensus id-feats -p id --features web,ssh,dht

for c in id-caps id-events; do
  ( cargo clippy -p "$c" --all-targets --keep-going -- \
      -W clippy::assert_is_empty -W clippy::chunks_exact_to_as_chunks \
      > "/tmp/probe-${c}.log" 2>&1 || true )
  echo "PROBE ${c} error lines (first 6):"
  grep '^error' "/tmp/probe-${c}.log" | head -6 | cut -c1-200 || true
  echo "PROBE ${c} log tail:"
  tail -3 "/tmp/probe-${c}.log" | cut -c1-200 || true
done
echo "CENSUS-E DONE"
