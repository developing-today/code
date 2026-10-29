#!/bin/bash
# Laptop start wrapper for opencode-route-proxy (fleet pattern, mirrors
# commandcode-proxy-start.sh). The proxy fronts the standing OpenCode
# server on loopback; the server password is read by the proxy itself
# from ~/.local/state/opencode/service-prod.json at runtime.
export HOME=/home/user
export OPENCODE_ROUTE_PROXY_PORT=${OPENCODE_ROUTE_PROXY_PORT:-18750}
export OPENCODE_ROUTE_PROXY_HOST=127.0.0.1
# IPv6 fix (2026-10-25): LAN v6 is ULA-only/no-upstream and Node 24
# fetch (undici autoSelectFamily) times out on it. Load per-process net
# defaults (autoSelectFamily OFF) for the whole proxy tree.
export NODE_OPTIONS="--import $HOME/code/node-net-defaults.mjs${NODE_OPTIONS:+ $NODE_OPTIONS}"
export PATH="/run/current-system/sw/bin:$HOME/.local/bin:/usr/local/bin:$PATH"
exec /run/current-system/sw/bin/node "$HOME/code/bin/opencode-route-proxy.mjs"
