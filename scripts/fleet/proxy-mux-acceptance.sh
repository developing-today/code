#!/usr/bin/env bash
# Repeatable step-5 proxy-mux acceptance harness (#5756 §6, task 25686).
#
# Contract: --manifest PATH (optional; defaults to pkgs/fleet/Cargo.toml),
# --output PATH (optional; tee full output to PATH), --list (assert the
# exact acceptance test exists, print its name, exit 0), --help. No
# positional arguments are accepted; unknown flags are a usage error.
#
# What it runs: the merged ignored probe
# `mux_acceptance::proxy_mux_acceptance_probe` (PR #15865) — in-process
# UnixStream pair covering all four §6 dimensions (warm p50, 128
# concurrent, tunnel-kill recovery, client/broker trip correlation).
# Loopback timings are a regression guard only; the report explicitly
# labels live thresholds UNVERIFIED. Live measurements require a
# reachable paired proxy endpoint (see resume condition on #5756).
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: scripts/fleet/proxy-mux-acceptance.sh [--manifest PATH] [--output PATH] [--list] [--help]
Runs the merged mux acceptance probe; loopback does not prove live thresholds.
USAGE
}

manifest="pkgs/fleet/Cargo.toml"
output=""
list_only=0
while (($#)); do
  case "$1" in
  --manifest)
    (($# >= 2)) || {
      echo "usage error: --manifest requires a value" >&2
      exit 2
    }
    manifest=$2
    shift 2
    ;;
  --output)
    (($# >= 2)) || {
      echo "usage error: --output requires a value" >&2
      exit 2
    }
    output=$2
    shift 2
    ;;
  --list)
    list_only=1
    shift
    ;;
  --help | -h)
    usage
    exit 0
    ;;
  --*)
    echo "usage error: unknown option '$1'" >&2
    exit 2
    ;;
  *)
    echo "usage error: unexpected positional argument '$1'" >&2
    exit 2
    ;;
  esac
done

name="proxy_mux_acceptance_probe"
repo_root=$(git rev-parse --show-toplevel)
manifest_abs=$(cd "$repo_root" && realpath "$manifest")
cargo_args=(--manifest-path "$manifest_abs" -p fleet-proxy --test mux_acceptance)
listing=$(cargo test "${cargo_args[@]}" -- --list)
grep -Fq "$name: test" <<<"$listing" || {
  echo "acceptance test missing from target: $name" >&2
  exit 1
}
if ((list_only)); then
  printf '%s\n' "$name"
  exit 0
fi

run() {
  printf 'acceptance_sha=%s\n' "$(git -C "$repo_root" rev-parse HEAD)"
  printf 'environment=%s\n' "$(uname -a)"
  printf 'acceptance_tests=%s\n' "$name"
  printf 'live_proxy_pair=UNVERIFIED (loopback harness only; thresholds require live pair)\n'
  cargo test "${cargo_args[@]}" -- --ignored --nocapture
}
if [[ -n $output ]]; then
  mkdir -p "$(dirname "$output")"
  run 2>&1 | tee "$output"
else
  run
fi
