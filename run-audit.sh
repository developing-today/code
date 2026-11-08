#!/usr/bin/env bash
cd ~/git/fleet/5378-3195-release-audit/pkgs/id || exit 1
LOG=~/git/fleet/5378-3195-release-audit/audit-5378.log
exec > "$LOG" 2>&1
run_variant() {
  local name="$1"; shift
  echo "=== VARIANT $name START $(date -u +%FT%TZ) ==="
  echo "load: $(cat /proc/loadavg)"
  local td="$HOME/git/fleet/5378-3195-release-audit/target-$name"
  local start=$SECONDS
  CARGO_TARGET_DIR="$td" cargo bench --bench world_bench --no-run "$@" 2>&1 | tail -3
  echo "BUILD_SECONDS[$name]=$((SECONDS-start))"
  echo "load_after_build: $(cat /proc/loadavg)"
  local exe
  exe=$(ls "$td"/release/deps/world_bench-* 2>/dev/null | grep -v '\.d$' | head -1)
  echo "BENCH_EXE[$name]=$exe"
  local bstart=$SECONDS
  "$exe" 2>&1 | grep '^bench ' 
  echo "BENCH_SECONDS[$name]=$((SECONDS-bstart))"
  echo "=== VARIANT $name END $(date -u +%FT%TZ) ==="
}
run_variant a-baseline
run_variant b-fat --config 'profile.bench.lto="fat"'
run_variant c-cg16 --config 'profile.bench.codegen-units=16'
run_variant d-lld --config 'target.x86_64-unknown-linux-gnu.rustflags=["-C","link-arg=-fuse-ld=lld"]'
echo "ALL_DONE $(date -u +%FT%TZ)"
