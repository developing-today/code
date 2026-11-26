#!/usr/bin/env bash
# Temporary verification script for the #803 final slice. NOT part of the PR.
# Phase B: per-crate clippy gate, all warnings denied.
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
    cargo clippy -p "$pkg" --all-targets --features "$feats" -- -D warnings > "$log" 2>&1
  else
    cargo clippy -p "$pkg" --all-targets -- -D warnings > "$log" 2>&1
  fi
  rc=$?
  echo "GATE $pkg exit=$rc"
  grep -E '^error' "$log" | sort | uniq -c | head -40
  grep -E 'error\[|^error' "$log" | head -5
  grep -B2 -A8 '^error' "$log" | head -80
}

gate id-core "web,test-utils,test-provider"
gate id-web "sandbox"
gate id-import-notion ""
gate id-caps ""
gate id-events ""
echo "GATES DONE"
