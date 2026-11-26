#!/usr/bin/env bash
# Temporary verification script for the #803 final slice. NOT part of the PR.
# Phase C: per-crate full test suites with executed counts.
set -u
cd "$(dirname "$0")/.."
export CARGO_TARGET_DIR="$PWD/.census-target"
export CARGO_BUILD_JOBS=2
mkdir -p "$CARGO_TARGET_DIR"

run_tests() {
  pkg="$1"
  feats="$2"
  log="/tmp/test-$pkg.log"
  if [ -n "$feats" ]; then
    cargo test -p "$pkg" --all-targets --features "$feats" > "$log" 2>&1
  else
    cargo test -p "$pkg" --all-targets > "$log" 2>&1
  fi
  rc=$?
  echo "TEST $pkg exit=$rc"
  grep -E '^test result:' "$log"
  grep -E '^test result:' "$log" | awk -F'[.;] ' '{for(i=1;i<=NF;i++){if($i~/passed/)p+=$i+0; if($i~/failed/)f+=$i+0; if($i~/ignored/)ig+=$i+0}} END {print "TOTAL passed=" p " failed=" f " ignored=" ig}'
  grep -E '^error|FAILED|failures:' "$log" | head -20
}

run_tests id-core "web,test-utils,test-provider"
run_tests id-web "sandbox"
run_tests id-import-notion ""
run_tests id-caps ""
run_tests id-events ""
echo "TESTS DONE"
