#!/usr/bin/env bash
# Temporary verification script for the #803 final slice. NOT part of the PR.
# Per-crate clippy gate: the two sweep lints denied, crate lint config as-is
# (this matches the root-crate gate the census lane used). A blanket
# -D warnings is NOT the gate: it denies hundreds of unrelated pre-existing
# warn-level lints across these crates.
set -u
cd "$(dirname "$0")/.."
export CARGO_TARGET_DIR="$PWD/.census-target"
export CARGO_BUILD_JOBS=2
mkdir -p "$CARGO_TARGET_DIR"

gate() {
  pkg="$1"
  feats="$2"
  log="/tmp/gate-$pkg.log"
  if [ -n "$feats" ]; then
    cargo clippy -p "$pkg" --all-targets --features "$feats" -- \
      -D clippy::assert_is_empty -D clippy::chunks_exact_to_as_chunks > "$log" 2>&1
  else
    cargo clippy -p "$pkg" --all-targets -- \
      -D clippy::assert_is_empty -D clippy::chunks_exact_to_as_chunks > "$log" 2>&1
  fi
  rc=$?
  echo "GATE $pkg exit=$rc"
  grep -E '^error' "$log" | sort | uniq -c | head -40
  grep -B1 -A6 '^error' "$log" | head -100
}

gate id-core "web,test-utils,test-provider"
gate id-web "sandbox"
gate id-import-notion ""
gate id-caps ""
gate id-events ""
echo "GATES DONE"
