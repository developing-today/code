#!/usr/bin/env bash
WT=$HOME/git/fleet/5378-3195-release-audit
LOG=$WT/fuel-repeat-20261102.log
exec > "$LOG" 2>&1
cd "$WT/pkgs/id" || exit 1
echo "=== FUEL TEST START $(date -u +%FT%TZ) ==="
echo "load: $(cat /proc/loadavg)"
export CARGO_TARGET_DIR="$WT/target-a-baseline"
s=$SECONDS
~/tools/fleet-build-lock cargo test --release --offline --lib checked_in_examples_stay_within_fuel_budgets 2>&1 | tail -8
echo "FUEL_SECONDS=$((SECONDS-s))"
echo "=== FUEL TEST END $(date -u +%FT%TZ) ==="
run_bench() {
  local name="$1" exe="$2"
  echo "=== REPEAT BENCH $name START $(date -u +%FT%TZ) ==="
  echo "load: $(cat /proc/loadavg)"
  local bs=$SECONDS
  "$exe" 2>&1 | grep '^bench '
  echo "BENCH_SECONDS[$name]=$((SECONDS-bs))"
  echo "load_after: $(cat /proc/loadavg)"
  echo "=== REPEAT BENCH $name END $(date -u +%FT%TZ) ==="
}
run_bench b-fat "$WT/target-b-fat/release/deps/world_bench-4988b560e85aaad7"
run_bench c-cg16 "$WT/target-c-cg16/release/deps/world_bench-04adad38f1012e24"
echo "FUEL_REPEAT_DONE $(date -u +%FT%TZ)"
