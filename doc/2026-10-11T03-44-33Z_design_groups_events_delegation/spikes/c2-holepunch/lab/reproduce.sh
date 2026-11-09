#!/usr/bin/env bash
set -euo pipefail

LAB=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
SPIKE_ROOT=$(dirname "$LAB")
REPO_ROOT=$(cd "$SPIKE_ROOT/../../../../" && pwd)

for tool in cargo rustc nft ip unshare nsenter openssl python3 timeout; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "missing required tool: $tool; use the documented nix run ...#c2-lab command" >&2
    exit 127
  fi
done

cache_root=${XDG_CACHE_HOME:-${HOME:-$SPIKE_ROOT}/.cache}
CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$cache_root/c2-holepunch}
export CARGO_TARGET_DIR
RELAY_ROOT="$CARGO_TARGET_DIR/iroh-relay"
export NFT="${NFT:-$(command -v nft)}"
export RELAY_BIN="${RELAY_BIN:-$RELAY_ROOT/bin/iroh-relay}"
export C2_BIN="${C2_BIN:-$CARGO_TARGET_DIR/debug/c2-holepunch}"

STAMP=$(date -u +%Y%m%dT%H%M%SZ)
RESULTS_DIR="$SPIKE_ROOT/results"
RESULTS_FILE="$RESULTS_DIR/$STAMP-emulated-c2-matrix.md"
RUN_LOGS="$CARGO_TARGET_DIR/run-logs/$STAMP"
WINDOW=${C2_WINDOW:-8}
mkdir -p "$RESULTS_DIR" "$RUN_LOGS"

exec > >(tee "$RESULTS_FILE") 2>&1
printf '# C2 NAT-emulation run — %s\n\n' "$STAMP"
cat <<EOF
This is Linux netfilter emulation in isolated user/network namespaces, not testing on real NAT devices or real networks.

- Repository: $REPO_ROOT
- Spike: $SPIKE_ROOT
- C2 window per matrix cell: $WINDOW seconds
- Entry point: nix --extra-experimental-features 'nix-command flakes' run ./doc/2026-10-11T03-44-33Z_design_groups_events_delegation/spikes/c2-holepunch#c2-lab

## Build

EOF
cargo build --locked --manifest-path "$SPIKE_ROOT/Cargo.toml" --bin c2-holepunch
cargo install --locked --version 1.3.0 --features server --root "$RELAY_ROOT" iroh-relay

for binary in "$NFT" "$RELAY_BIN" "$C2_BIN"; do
  if [ ! -x "$binary" ]; then
    echo "required executable is missing: $binary" >&2
    exit 127
  fi
done

printf '\n## NAT behaviour probes\n'
for nat in cone restricted symmetric; do
  printf '\n### %s\n\n' "$nat"
  "$LAB/probe-nat.sh" "$nat"
done

printf '\n## C2 endpoint matrix\n'
completed=0
for nat_a in cone restricted symmetric; do
  for nat_b in cone restricted symmetric; do
    tag="$STAMP-$nat_a-$nat_b"
    printf '\n### host-a=%s, host-b=%s\n\n' "$nat_a" "$nat_b"
    NAT_B="$nat_b" LOGS="$RUN_LOGS/$tag" "$LAB/run.sh" "$nat_a" "$tag" "$WINDOW"
    completed=$((completed + 1))
  done
done

successful=$(grep -Ec '^== .*\(dial exit 0\)$' "$RESULTS_FILE" || true)
printf '\nCompleted %s/9 matrix cells; dial exit 0 recorded for %s.\n' "$completed" "$successful"
if [ "$completed" -ne 9 ] || [ "$successful" -ne 9 ]; then
  echo "matrix did not complete successfully; inspect this results file and the temporary logs at $RUN_LOGS" >&2
  exit 1
fi
printf '\nAll nine C2 netfilter-emulation cells completed. Results are emulated, not real-network measurements.\n'
