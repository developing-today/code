#!/usr/bin/env bash
# Regenerate the home-page example's client from the fixture servers.
#
# Acceptance on #1876: the page runs "the real generated client, emitted at
# docs build time by `mcpx client`/`mcpx types` against fixture servers —
# not a hand-written imitation". Until the site build lands (#1941), this
# script is that build step, run by hand and wired into the site pipeline
# later; client.ts and client.browser.js are checked in so the component
# is complete in the docs source.
#
# It builds mcpx and the deterministic fixture server (fakemcp), points an
# isolated mcpx installation at the fixture as the `demo` namespace (the
# arrangement examples/examples_test.go uses), emits the client, strips it
# for the browser, and stops the daemon it started.
#
# Usage: ./generate-client.sh [path-to-pkgs/mcpx]   (default: ../..)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
mcpx_root="${1:-$(cd "$here/../.." && pwd)}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

export TMPDIR="${TMPDIR:-/tmp}"
(cd "$mcpx_root" && go build -o "$work/mcpx" ./cmd/mcpx)
(cd "$mcpx_root" && go build -o "$work/fakemcp" ./internal/testsupport/fakemcp)

mkdir -p "$work/home"
printf '{"mcpServers":{"demo":{"command":"%s"}}}' "$work/fakemcp" > "$work/.mcpx.json"

export HOME="$work/home"
export XDG_CONFIG_HOME="$work/xdg"
# DaemonSearchDirs() also searches $XDG_STATE_HOME/mcpx — isolate it too,
# or the stop below could see (and stop) the caller's own daemons.
export XDG_STATE_HOME="$work/xdg-state"
export MCPX_STATE_DIR="$work/state"
export MCPX_CACHE_DIR="$work/cache"
export MCPX_CONFIG="$work/.mcpx.json"
export MCPX_AUTOSTART_IDLE_EXIT=60s

"$work/mcpx" client --ns demo -o "$here/client.ts"
# Stop ONLY this config's fixture daemon — never `stop --all`, which shuts
# down every live daemon found in the search dirs, including the caller's
# unrelated sessions.
"$work/mcpx" stop >/dev/null 2>&1 || true

node "$here/strip-client.mjs" "$here/client.ts" "$here/client.browser.js"
echo "regenerated client.ts and client.browser.js from the demo fixture"
