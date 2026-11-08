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

usage() {
  printf 'usage: %s SHARD COUNT\n' "$(basename "$0")" >&2
}

invalid() {
  printf '%s: %s\n' "$(basename "$0")" "$1" >&2
  usage
  exit 2
}

if [[ $# -ne 2 ]]; then
  invalid 'expected exactly two arguments'
fi

decimal_in_bash_range() {
  local value="$1"
  # Normalize leading zeroes before arithmetic so decimal values such as 08
  # are not interpreted as octal by Bash.
  while [[ ${#value} -gt 1 ]] && [[ ${value:0:1} == 0 ]]; do
    value="${value#0}"
  done
  if [[ ${#value} -gt 19 ]]; then
    return 1
  fi
  if [[ ${#value} -eq 19 ]]; then
    # Compare bounded chunks so the signed-integer limit check itself cannot
    # overflow Bash arithmetic.
    local high="${value:0:9}"
    local low="${value:9}"
    if ((10#${high} > 922337203)); then
      return 1
    fi
    if ((10#${high} == 922337203)) && ((10#${low} > 6854775807)); then
      return 1
    fi
  fi
  printf '%s' "${value}"
}

[[ $1 =~ ^[0-9]+$ ]] || invalid 'SHARD must be a non-negative decimal integer'
[[ $2 =~ ^[0-9]+$ ]] || invalid 'COUNT must be a positive decimal integer'
shard="$(decimal_in_bash_range "$1")" || invalid 'SHARD exceeds the supported integer range'
count="$(decimal_in_bash_range "$2")" || invalid 'COUNT exceeds the supported integer range'

((10#${count} > 0)) || invalid 'COUNT must be greater than zero'
((10#${shard} < 10#${count})) || invalid 'SHARD must be less than COUNT'
shard=$((10#${shard}))
count=$((10#${count}))

targets=()
shopt -s nullglob
for f in tests/*.rs; do
  targets+=("$(basename "${f}" .rs)")
done
for d in tests/*/; do
  if [[ -f "${d}/main.rs" ]]; then
    targets+=("$(basename "${d}")")
  fi
done
if ((${#targets[@]} > 0)); then
  sorted_targets="$(printf '%s\n' "${targets[@]}" | sort)"
  mapfile -t targets <<<"${sorted_targets}"
fi

args=()
for i in "${!targets[@]}"; do
  if ((i % count == shard)); then
    args+=(--test "${targets[${i}]}")
  fi
done

if ((${#args[@]} == 0)); then
  printf '%s: no test targets selected for shard %s/%s\n' \
    "$(basename "$0")" "${shard}" "${count}" >&2
  exit 1
fi

echo "test-shard-sandbox ${shard}/${count} targets: ${args[*]}"
cargo test --all-features "${args[@]}" -- --skip serve_tests

if [[ ${shard} -eq 0 ]]; then
  cargo test --all-features --bins -- --skip serve_tests
  cargo test --all-features --doc
fi
