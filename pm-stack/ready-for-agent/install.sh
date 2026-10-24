#!/usr/bin/env bash
# Install/refresh ready-for-agent (pm-stack Phase 2, dispatch layer)
# on this host.
#
#   ./install.sh            install pinned package + unit, (re)start,
#                           wait for the UI, print status
#   ./install.sh clone      additionally clone (or fetch) the
#                           repositories the harness serves into
#                           ./repos/
#
# Registration note (learned 2026-10-24, first deployment): register
# each clone with the harness through its GraphQL API (loopback
# :6056/graphql, mutation addLocalRepository(path:)). The
# `ready-for-agent add` CLI does its git inspection in-process and
# failed with NOT_A_GIT_REPOSITORY from an interactive SSH shell
# environment on this host; the same inspection succeeds from the
# service's clean environment. A full (non-bare) clone is what the
# first deployment registered and runs with, even though upstream
# documents bare clones as the recommended shape.
#
# The pin: ready-for-agent is an npm package (no container image is
# published). PINNED_VERSION below is the release this stack runs;
# bump it deliberately after reading the upstream release notes —
# never float on @latest.
set -euo pipefail

PINNED_VERSION="1.1.0"

cd "$(dirname "$0")"
DEPLOY_DIR="$PWD"

# 1. Pinned npm package into the user prefix (~/.local). The system
#    npm prefix on this host is the read-only Nix store.
current=""
if [[ -x $HOME/.local/bin/ready-for-agent ]]; then
  current="$("$HOME/.local/bin/ready-for-agent" --version 2>/dev/null | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1 || true)"
fi
if [[ $current != "$PINNED_VERSION" ]]; then
  echo "installing ready-for-agent@$PINNED_VERSION (found: ${current:-none})"
  npm install -g --prefix "$HOME/.local" "ready-for-agent@$PINNED_VERSION"
else
  echo "ready-for-agent@$PINNED_VERSION already installed"
fi

# 2. Repository clones the harness works from (full clones — the shape
#    the first deployment verified; see the registration note above).
#    Only with the explicit `clone` argument — cloning is a one-time,
#    network-heavy step.
if [[ ${1:-} == clone ]]; then
  mkdir -p "$DEPLOY_DIR/repos"
  if [[ -d $DEPLOY_DIR/repos/code ]]; then
    git -C "$DEPLOY_DIR/repos/code" fetch --all --prune
    echo "fetched code"
  else
    GIT_TERMINAL_PROMPT=0 git clone \
      https://github.com/developing-today/code.git "$DEPLOY_DIR/repos/code"
    echo "cloned code"
  fi
fi

# 3. systemd user unit.
install -Dm644 systemd/ready-for-agent.service \
  "$HOME/.config/systemd/user/ready-for-agent.service"
systemctl --user daemon-reload
systemctl --user enable --now ready-for-agent.service

echo "waiting for the harness UI..."
for i in $(seq 1 30); do
  if curl --fail --silent http://127.0.0.1:6056/ >/dev/null; then
    echo "ready-for-agent is up: http://127.0.0.1:6056"
    systemctl --user --no-pager status ready-for-agent.service | head -5
    exit 0
  fi
  sleep 5
done
echo "ready-for-agent did not come up; check: journalctl --user -u ready-for-agent" >&2
exit 1
