#!/bin/sh
# Reproduce the S1 golden hash vector (composition-hash-spec.md).
# Step 1 (always works): re-derive the canonical CBOR bytes from
# inputs.json with the stdlib-only encoder and compare to canonical.hex.
# Step 2 needs a BLAKE3 implementation: `b3sum` if present, else the
# estate's offline cargo helper (blake3 1.8.7 is in the workspace
# lockfile and the cargo cache, so --offline builds with no network).
set -eu
cd "$(dirname "$0")"

CANON="$(python3 -I make_vector.py)"
if [ "$CANON" != "$(cat canonical.hex)" ]; then
  echo "FAIL: re-encoded canonical bytes differ from canonical.hex"
  exit 1
fi
echo "canonical bytes reproduce: $(($(wc -c < canonical.hex) / 2)) bytes"

if command -v b3sum >/dev/null 2>&1; then
  HASH="$(printf '%s' "$CANON" | xxd -r -p | b3sum | cut -d' ' -f1)"
elif [ -x /home/user/.local/share/opencode/tmp/opencode/s1-blake3/target/release/s1-blake3 ]; then
  HASH="$(/home/user/.local/share/opencode/tmp/opencode/s1-blake3/target/release/s1-blake3 "$CANON")"
else
  echo "no BLAKE3 tool available; canonical bytes verified, hash step skipped"
  echo "build one: cargo build --offline in a crate depending on blake3 = \"=1.8.7\""
  exit 0
fi
if [ "$HASH" != "$(cat hash.hex)" ]; then
  echo "FAIL: recomputed hash $HASH differs from hash.hex"
  exit 1
fi
echo "hash reproduces: $HASH"
