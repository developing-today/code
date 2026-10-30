#!/bin/bash
# Laptop start wrapper for antigravity-direct (fleet pattern, mirrors
# muse-proxy-start.sh). The nix-managed unit in home/common/default.nix
# execs the .mjs directly; this wrapper is the manual-run equivalent.
# Must run inside the user session (or with
# DBUS_SESSION_BUS_ADDRESS set): the proxy reads the agy CLI's OAuth
# credential from the Secret Service keyring via secret-tool.
export HOME=/home/user
export ANTIGRAVITY_DIRECT_PORT=${ANTIGRAVITY_DIRECT_PORT:-18751}
export PATH="$HOME/.local/bin:/run/current-system/sw/bin:/usr/local/bin:$PATH"
exec /run/current-system/sw/bin/node "$HOME/code/bin/antigravity-direct.mjs"
