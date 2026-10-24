#!/usr/bin/env bash
# Deploy Pad (pm-stack Phase 2, planning layer) on this host.
#
# Renders .env (mode 600) from the SOPS-encrypted secrets in
# ../../secrets/pm-stack.sops.yaml (relative to this repo checkout),
# then pulls and (re)creates the container, then waits for the
# health endpoint.
#
# Usage:
#   ./deploy.sh             render env, pull image, start/refresh, health gate
#   ./deploy.sh render      only render .env
#   ./deploy.sh restart     docker compose restart (keeps current env;
#                           note: env changes need a full deploy.sh run,
#                           compose only applies env on recreate)
#
# Host specifics (the ONLY host-specific values):
#   PM_STACK_SECRETS  path to the SOPS file
#                     (default: ../../secrets/pm-stack.sops.yaml next to
#                     this script's repo checkout)
#   SOPS_AGE_KEY_FILE age key for decryption
#                     (default: ~/.config/sops/age/keys.txt)
# The stack binds 127.0.0.1:7777 and the host tailnet address :7777
# (see compose.yml) and stores data in the Docker named volume
# pad_pad_data on this host.
set -euo pipefail

cd "$(dirname "$0")"
STACK_DIR="$PWD"
SECRETS_FILE="${PM_STACK_SECRETS:-$STACK_DIR/../../secrets/pm-stack.sops.yaml}"
export SOPS_AGE_KEY_FILE="${SOPS_AGE_KEY_FILE:-$HOME/.config/sops/age/keys.txt}"

render() {
  if [[ ! -f $SECRETS_FILE ]]; then
    echo "secrets file not found: $SECRETS_FILE" >&2
    exit 1
  fi
  # Decrypt to a variable; values never touch disk in plaintext except
  # in the rendered file below (mode 600).
  local plain
  plain="$(sops --decrypt "$SECRETS_FILE")"
  local get
  get() { printf '%s\n' "$plain" | sed -n "s/^$1: //p" | tr -d "\"'"; }

  umask 077
  {
    echo "# Rendered by deploy.sh from secrets/pm-stack.sops.yaml — do not edit, do not commit."
    echo "# Pad needs no server-env secrets on the pinned version: the API"
    echo "# authenticates with minted pad_ tokens (SOPS: pad_api_token)."
  } >.env
  chmod 600 .env
  echo "rendered .env from $SECRETS_FILE"
}

case "${1:-deploy}" in
render)
  render
  ;;
restart)
  docker compose restart
  ;;
deploy)
  render
  docker compose pull
  docker compose up -d --force-recreate
  echo "waiting for pad health..."
  for i in $(seq 1 30); do
    if curl --fail --silent http://127.0.0.1:7777/api/v1/health >/dev/null; then
      echo "pad is healthy: $(curl --silent http://127.0.0.1:7777/api/v1/health)"
      exit 0
    fi
    sleep 5
  done
  echo "pad did not become healthy in time; check: docker compose logs" >&2
  exit 1
  ;;
*)
  echo "usage: $0 [deploy|render|restart]" >&2
  exit 2
  ;;
esac
