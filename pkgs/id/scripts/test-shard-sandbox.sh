#!/usr/bin/env bash
# Run one COUNT-th slice of the integration tests (sandbox-safe: excludes
# serve_tests requiring network). Usage: test-shard-sandbox.sh SHARD COUNT
#
# Test targets are globbed from tests/ at run time and dealt round-robin
# across shards, so a new test target joins exactly one shard
# automatically. The shards together cover every target test-sandbox
# covers; shard 0 also runs the bin unit tests and the doc tests, which
# the per-target shards skip. Lib unit tests stay in test-unit. The PR
# gate runs 4 shards in parallel instead of the single serial
# test/test-web pair (#869).
set -euo pipefail

shard=$1
count=$2

targets=()
for f in tests/*.rs; do
  targets+=("$(basename "$f" .rs)")
done
for d in tests/*/; do
  if [ -f "$d/main.rs" ]; then
    targets+=("$(basename "$d")")
  fi
done
mapfile -t targets < <(printf '%s\n' "${targets[@]}" | sort)

args=()
for i in "${!targets[@]}"; do
  if [ $((i % count)) -eq "$shard" ]; then
    args+=(--test "${targets[$i]}")
  fi
done

echo "test-shard-sandbox $shard/$count targets: ${args[*]}"
cargo test --all-features "${args[@]}" -- --skip serve_tests

if [ "$shard" -eq 0 ]; then
  cargo test --all-features --bins -- --skip serve_tests
  cargo test --all-features --doc
fi
