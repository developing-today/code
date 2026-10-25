#!/bin/bash
# Laptop start wrapper for commandcode-proxy (fleet pattern, mirrors the
# sandbox wrapper minus the sandbox-only egress pins).
export HOME=/home/user
export COMMANDCODE_PROXY_PORT=${COMMANDCODE_PROXY_PORT:-18748}
export COMMANDCODE_PROXY_MAX_TURNS=1
export COMMANDCODE_PROXY_HOST=127.0.0.1
# Empty egress override: on the laptop the CLI reaches CommandCode
# directly; the proxy must NOT pin the sandbox egress proxy for children.
export COMMANDCODE_PROXY_EGRESS=
# IPv6 fix (2026-10-25): LAN v6 is ULA-only/no-upstream and Node 24
# fetch (undici autoSelectFamily) times out on it. Load per-process net
# defaults (autoSelectFamily OFF) for the whole proxy tree; CLI children
# inherit env via the proxy's childEnv().
export NODE_OPTIONS="--import $HOME/code/node-net-defaults.mjs${NODE_OPTIONS:+ $NODE_OPTIONS}"
export PATH="/run/current-system/sw/bin:$HOME/.local/bin:/run/current-system/sw/bin:/usr/local/bin:$PATH"
exec /run/current-system/sw/bin/node "$HOME/code/bin/commandcode-proxy.mjs"
