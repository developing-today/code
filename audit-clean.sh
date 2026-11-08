#!/usr/bin/env bash
WT=$HOME/git/fleet/5378-3195-release-audit
LOG=$WT/audit-clean-20261102.log
exec > "$LOG" 2>&1
echo "CLEAN RUN START $(date -u +%FT%TZ)"
cd "$WT/pkgs/id" || exit 1
run_bench() {
  local name="$1" exe="$2"
  echo "=== BENCH $name START $(date -u +%FT%TZ) ==="
  echo "load: $(cat /proc/loadavg)"
  local s=$SECONDS
  "$exe" 2>&1 | grep '^bench '
  echo "BENCH_SECONDS[$name]=$((SECONDS-s))"
  echo "load_after: $(cat /proc/loadavg)"
  echo "=== BENCH $name END $(date -u +%FT%TZ) ==="
}
run_bench a-baseline "$WT/target-a-baseline/release/deps/world_bench-a79b281dc077277b"
run_bench b-fat "$WT/target-b-fat/release/deps/world_bench-4988b560e85aaad7"
run_bench c-cg16 "$WT/target-c-cg16/release/deps/world_bench-04adad38f1012e24"
echo "=== FUEL TEST START $(date -u +%FT%TZ) ==="
echo "load: $(cat /proc/loadavg)"
"$WT/target-a-baseline/release/deps/id-9dab48ffd28cc624" checked_in_examples_stay_within_fuel_budgets 2>&1 | tail -5
echo "=== FUEL TEST END $(date -u +%FT%TZ) ==="
echo "=== BUILD d-lld START $(date -u +%FT%TZ) ==="
echo "load: $(cat /proc/loadavg)"
export CARGO_TARGET_DIR="$WT/target-d-lld"
s=$SECONDS
~/tools/fleet-build-lock cargo bench --bench world_bench --no-run --config 'target.x86_64-unknown-linux-gnu.rustflags=["-C","link-arg=-fuse-ld=lld"]' 2>&1 | tail -3
echo "BUILD_SECONDS[d-lld]=$((SECONDS-s))"
exe=$(ls "$WT"/target-d-lld/release/deps/world_bench-* 2>/dev/null | grep -v '\.d$' | head -1)
echo "BENCH_EXE[d-lld]=$exe"
if [ -n "$exe" ]; then run_bench d-lld "$exe"; fi
echo "ALL_CLEAN_DONE $(date -u +%FT%TZ)"
