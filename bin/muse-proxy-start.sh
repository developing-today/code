#!/bin/bash
# Laptop start wrapper for muse-proxy (fleet pattern, mirrors the sandbox
# wrapper minus the sandbox-only egress pins).
export HOME=/home/user
export MUSE_PROXY_PORT=${MUSE_PROXY_PORT:-18747}
# Empty egress override: on the laptop the muse CLI reaches Meta directly.
export MUSE_PROXY_EGRESS=
# META_API_KEY takes priority over the CLI's OAuth account login and bills
# a separate pay-as-you-go wallet — it must be absent.
unset META_API_KEY
export PATH="$HOME/.local/bin:/run/current-system/sw/bin:/usr/local/bin:$PATH"
exec /run/current-system/sw/bin/node "$HOME/code/bin/muse-proxy.mjs"
