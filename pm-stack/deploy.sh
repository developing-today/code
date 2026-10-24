#!/usr/bin/env bash
# Deploy the pm-stack on this host.
#
# Renders .env (mode 600) from the SOPS-encrypted secrets in
# ../secrets/pm-stack.sops.yaml (relative to this repo checkout), then
# pulls and (re)creates the containers, then renders the Cloudflare
# Tunnel token file used by the pm-tunnel systemd user unit.
#
# Usage:
#   ./deploy.sh             render env, pull images, start/refresh stack
#   ./deploy.sh render      only render .env + tunnel token file
#   ./deploy.sh restart     docker compose restart (keeps current env;
#                           note: env changes need a full deploy.sh run,
#                           compose only applies env on recreate)
#
# Host specifics (the ONLY host-specific values in the stack):
#   PM_STACK_SECRETS  path to the SOPS file
#                     (default: ../secrets/pm-stack.sops.yaml next to
#                     this script's repo checkout)
#   SOPS_AGE_KEY_FILE age key for decryption
#                     (default: ~/.config/sops/age/keys.txt)
# The stack itself binds 127.0.0.1:5173 and stores data in the Docker
# named volume pm-stack_postgres_data on this host.
set -euo pipefail

cd "$(dirname "$0")"
STACK_DIR="$PWD"
SECRETS_FILE="${PM_STACK_SECRETS:-$STACK_DIR/../secrets/pm-stack.sops.yaml}"
export SOPS_AGE_KEY_FILE="${SOPS_AGE_KEY_FILE:-$HOME/.config/sops/age/keys.txt}"

render() {
  if [[ ! -f $SECRETS_FILE ]]; then
    echo "secrets file not found: $SECRETS_FILE" >&2
    exit 1
  fi
  # Decrypt to a variable; values never touch disk in plaintext except
  # in the two rendered files below (both mode 600).
  local plain
  plain="$(sops --decrypt "$SECRETS_FILE")"
  local get
  get() { printf '%s\n' "$plain" | sed -n "s/^$1: //p" | tr -d "\"'"; }

  umask 077
  {
    echo "# Rendered by deploy.sh from secrets/pm-stack.sops.yaml — do not edit, do not commit."
    echo "KANEO_CLIENT_URL=https://kaneo.security.cab"
    echo "POSTGRES_DB=kaneo"
    echo "POSTGRES_USER=kaneo"
    echo "POSTGRES_PASSWORD=$(get postgres_password)"
    echo "AUTH_SECRET=$(get auth_secret)"
    echo "NOTIFICATION_SECRET_ENCRYPTION_KEY=$(get notification_secret_encryption_key)"
    # Registration renders CLOSED — that is the fleet posture, and a
    # re-render must never silently reopen it. (Incident 2026-10-24:
    # the previous preserve-if-set logic plus a false default in
    # .env.example dropped the closed posture on a Phase 1 re-render;
    # see the design record's Phase 1 as-built correction.) Opening
    # registration — only ever to bootstrap the first account — is an
    # explicit one-render opt-out:
    #   KANEO_OPEN_REGISTRATION=1 ./deploy.sh
    # Re-run ./deploy.sh without the variable to close it again.
    if [[ ${KANEO_OPEN_REGISTRATION:-} == 1 ]]; then
      echo "DISABLE_REGISTRATION=false"
    else
      echo "DISABLE_REGISTRATION=true"
    fi
    echo "GITHUB_APP_ID=$(get github_app_id)"
    echo "GITHUB_APP_NAME=$(get github_app_name)"
    echo "GITHUB_WEBHOOK_SECRET=$(get github_webhook_secret)"
    echo "GITHUB_PRIVATE_KEY_BASE64=$(get github_private_key_base64)"
  } >.env
  chmod 600 .env

  # cloudflared reads TUNNEL_TOKEN from its environment; the systemd
  # unit (systemd/pm-tunnel.service) loads this file.
  echo "TUNNEL_TOKEN=$(get cloudflare_tunnel_token)" >tunnel.env
  chmod 600 tunnel.env
  echo "rendered .env and tunnel.env from $SECRETS_FILE"
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
  echo "waiting for kaneo health..."
  for i in $(seq 1 30); do
    if curl --fail --silent http://127.0.0.1:5173/api/health >/dev/null; then
      echo "kaneo is healthy: $(curl --silent http://127.0.0.1:5173/api/health)"
      exit 0
    fi
    sleep 5
  done
  echo "kaneo did not become healthy in time; check: docker compose logs" >&2
  exit 1
  ;;
*)
  echo "usage: $0 [deploy|render|restart]" >&2
  exit 2
  ;;
esac
