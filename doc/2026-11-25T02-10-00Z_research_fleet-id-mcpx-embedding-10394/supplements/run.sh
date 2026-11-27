#!/usr/bin/env bash
# Build and run the #10394 IPC benchmark. Prints the markdown table and the host facts.
set -euo pipefail
cd "$(dirname "$0")"
out=$(mktemp -d)
rustc --edition 2024 -O --crate-type cdylib ipc_plugin.rs -o "$out/libipc_plugin.so"
rustc --edition 2024 -O ipc_bench.rs -o "$out/ipc_bench"
echo "host: $(uname -srm); cpus: $(nproc); rustc: $(rustc --version)"
grep -m1 'model name' /proc/cpuinfo || true
"$out/ipc_bench" "$out/libipc_plugin.so"
