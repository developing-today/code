#!/usr/bin/env bash
set -euo pipefail

worker=private-laptop-bridge-staging
tmp="$(mktemp -d)"
output="$tmp/dry-run.log"
export XDG_CONFIG_HOME="$tmp/config"
export WRANGLER_SEND_METRICS=false
mkdir -p "$XDG_CONFIG_HOME"
trap 'rm -rf "$tmp"' EXIT

./node_modules/.bin/cf build --mode bridge
./node_modules/.bin/cf -q workers versions create \
  --worker "$worker" --mode bridge --prebuilt --dry-run >"$output" 2>&1
TARGET_WORKER="$worker" TARGET_MODE=bridge DRY_RUN_OUTPUT="$output" \
  node release-artifact.mjs dry-run
