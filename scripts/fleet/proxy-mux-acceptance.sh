#!/usr/bin/env bash
# Repeatable step-5 proxy-mux acceptance harness (#5756).
# Contract: --manifest PATH (optional; defaults to pkgs/fleet/Cargo.toml),
# --output PATH (optional; tee full output to PATH), --list (list/assert
# exact tests and exit), or --help. No positional arguments are accepted.
# Live proxy-pair criteria are deliberately not inferred from loopback;
# the report explicitly labels live thresholds UNVERIFIED unless a live
# pair and live measurement procedure are separately available.
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: scripts/fleet/proxy-mux-acceptance.sh [--manifest PATH] [--output PATH] [--list] [--help]
Runs the four named loopback acceptance checks; loopback does not prove live thresholds.
USAGE
}
manifest="pkgs/fleet/Cargo.toml"
output=""
list_only=0
while (($#)); do
  case "$1" in
    --manifest)
      (($# >= 2)) || { echo "usage error: --manifest requires a value" >&2; exit 2; }
      manifest=$2; shift 2 ;;
    --output)
      (($# >= 2)) || { echo "usage error: --output requires a value" >&2; exit 2; }
      output=$2; shift 2 ;;
    --list) list_only=1; shift ;;
    --help|-h) usage; exit 0 ;;
    --*) echo "usage error: unknown option '$1'" >&2; exit 2 ;;
    *) echo "usage error: unexpected positional argument '$1'" >&2; exit 2 ;;
  esac
done

names=(
  acceptance_warm_trivial_exec_p50
  acceptance_concurrent_128_zero_failures
  acceptance_failure_recovery_within_2s
  acceptance_trip_correlation_both_sides
)
manifest_dir=$(dirname "$manifest")
repo_root=$(git rev-parse --show-toplevel)
manifest_abs=$(cd "$repo_root" && realpath "$manifest")
cargo_args=(--manifest-path "$manifest_abs" -p fleet-proxy --test mux_acceptance)
listing=$(cargo test "${cargo_args[@]}" -- --list)
for name in "${names[@]}"; do
  grep -Fxq "$name: test" <<<"$listing" || {
    echo "acceptance test missing from target: $name" >&2
    exit 1
  }
done
if ((list_only)); then
  printf '%s\n' "${names[@]}"
  exit 0
fi

run() {
  printf 'acceptance_sha=%s\n' "$(git -C "$repo_root" rev-parse HEAD)"
  printf 'environment=%s\n' "$(uname -a)"
  printf 'acceptance_tests=%s\n' "${names[*]}"
  printf 'live_proxy_pair=UNVERIFIED (loopback harness only; thresholds require live pair)\n'
  cargo test "${cargo_args[@]}" -- --nocapture --test-threads=4
}
if [[ -n "$output" ]]; then
  mkdir -p "$(dirname "$output")"
  run 2>&1 | tee "$output"
else
  run
fi
