#!/usr/bin/env bash
# Install/update pm-dispatch on the laptop.
#
# Mirrors the aggregator's install.sh: copies this package to
# ~/pm-stack/dispatch, renders the service env file from
# ~/secrets/pm-stack.sops.yaml (keys by name; values never printed or
# committed), installs the user systemd unit from pm-stack/systemd/,
# restarts ONLY when the deployed tree or deployment inputs changed,
# and fails loudly unless /health reports the build version it just
# deployed.
#
# Secrets layout:
# - From the sops file: github_app_id, github_private_key_base64
#   (rendered into ~/pm-stack/dispatch/.env, mode 600).
# - NOT in sops: SUPABASE_SERVICE_ROLE_KEY and DISPATCH_API_TOKEN live
#   in ~/secrets/dispatch.env (mode 600, laptop-only), which the unit
#   also reads. This script ensures that file exists and carries a
#   generated DISPATCH_API_TOKEN when one is absent; the Supabase key is
#   placed there by the operator (Bitwarden: supabase.service-role-key)
#   and is never generated or printed here.
#
# Usage (on the laptop, from the repo checkout):  bash install.sh
set -euo pipefail
cd "$(dirname "$0")"
SRC="$PWD"
DST="$HOME/pm-stack/dispatch"
SOPS="$HOME/secrets/pm-stack.sops.yaml"
SECRETS_ENV="$HOME/secrets/dispatch.env"
UNIT_SRC="$HOME/pm-stack/systemd/pm-dispatch.service"
[ -f "$UNIT_SRC" ] || UNIT_SRC="$SRC/../systemd/pm-dispatch.service"
UNIT_DST="$HOME/.config/systemd/user/pm-dispatch.service"
TAILNET_IP="${TAILNET_IP:-100.99.113.103}"
PORT="${DISPATCH_PORT:-4879}"

mkdir -p "$DST" "$HOME/.config/systemd/user" "$HOME/secrets"

# --- Operator secrets file (never printed) ------------------------------
if [ ! -f "$SECRETS_ENV" ]; then
  umask 077
  : >"$SECRETS_ENV"
  chmod 600 "$SECRETS_ENV"
  echo "created $SECRETS_ENV (mode 600)"
fi
if ! grep -q '^DISPATCH_API_TOKEN=' "$SECRETS_ENV"; then
  umask 077
  printf 'DISPATCH_API_TOKEN=%s\n' "$(openssl rand -hex 32)" >>"$SECRETS_ENV"
  chmod 600 "$SECRETS_ENV"
  echo "generated a DISPATCH_API_TOKEN into $SECRETS_ENV (value not printed)"
fi
if ! grep -q '^SUPABASE_SERVICE_ROLE_KEY=' "$SECRETS_ENV"; then
  echo "WARNING: $SECRETS_ENV has no SUPABASE_SERVICE_ROLE_KEY — queue endpoints will 502 until the operator adds it (Bitwarden: supabase.service-role-key)" >&2
fi

# --- Change detection + build marker ------------------------------------
DEPLOY_COMMIT="$(git -C "$SRC" rev-parse HEAD 2>/dev/null || true)"
tree_hash() {
  local root="$1"
  (cd "$root" && {
    for f in package.json bun.lock tsconfig.json README.md; do
      [ -f "$f" ] && echo "$f"
    done
    find src test examples -type f 2>/dev/null
  } | LC_ALL=C sort | while IFS= read -r f; do sha256sum "$f"; done | sha256sum | cut -d' ' -f1)
}
SRC_HASH="$(tree_hash "$SRC")"
DST_HASH=""
[ -f "$DST/.deploy-tree-hash" ] && DST_HASH="$(cat "$DST/.deploy-tree-hash")"
# Deployment inputs the tree hash cannot see: the tailnet bind address,
# the SOPS ciphertext, the unit file, and the operator secrets file.
# Hashing file contents keeps the comparison secret-free (a hash is not
# the value, and only 12 chars of it are ever echoed).
deploy_inputs_hash() {
  {
    printf 'TAILNET_IP=%s\n' "$TAILNET_IP"
    if [ -f "$SOPS" ]; then sha256sum "$SOPS"; else echo "sops:absent"; fi
    if [ -f "$UNIT_SRC" ]; then sha256sum "$UNIT_SRC"; else echo "unit:absent"; fi
    if [ -f "$SECRETS_ENV" ]; then sha256sum "$SECRETS_ENV"; else echo "secrets:absent"; fi
  } | sha256sum | cut -d' ' -f1
}
SRC_INPUTS="$(deploy_inputs_hash)"
DST_INPUTS=""
[ -f "$DST/.deploy-inputs-hash" ] && DST_INPUTS="$(cat "$DST/.deploy-inputs-hash")"
BUILD_VERSION="${DEPLOY_COMMIT:-tree:${SRC_HASH}}"
live_version() {
  curl -s --max-time 3 "http://127.0.0.1:${PORT}/health" | bun -e 'const t = await Bun.stdin.text(); try { console.log(JSON.parse(t).version ?? "") } catch { console.log("") }' || true
}
CHANGED=1
if [ "$SRC_HASH" = "$DST_HASH" ] && [ "$SRC_INPUTS" = "$DST_INPUTS" ] && systemctl --user is-active --quiet pm-dispatch.service && [ "$(live_version)" = "$BUILD_VERSION" ]; then
  CHANGED=0
  echo "dispatch unchanged (tree ${SRC_HASH:0:12}, inputs ${SRC_INPUTS:0:12}, version ${BUILD_VERSION}) - no copy, no restart"
elif [ "$SRC_HASH" = "$DST_HASH" ] && [ "$SRC_INPUTS" != "$DST_INPUTS" ]; then
  echo "dispatch tree unchanged but deployment inputs changed (tailnet/SOPS/unit/secrets) - re-rendering env and restarting"
fi

if [ "$CHANGED" -eq 1 ]; then
  for f in package.json bun.lock tsconfig.json README.md; do
    [ -f "$SRC/$f" ] && cp "$SRC/$f" "$DST/$f"
  done
  rm -rf "$DST/src" "$DST/test" "$DST/examples"
  cp -r "$SRC/src" "$DST/src"
  cp -r "$SRC/test" "$DST/test"
  cp -r "$SRC/examples" "$DST/examples"

  # Service env: non-secret config + the GitHub App material from sops.
  umask 077
  ENV_FILE="$DST/.env"
  {
    echo "DISPATCH_BUILD_VERSION=${BUILD_VERSION}"
    echo "DISPATCH_HOSTS=127.0.0.1,${TAILNET_IP}"
    echo "DISPATCH_PORT=${PORT}"
    if [ -f "$SOPS" ] && sops -d --extract '["github_app_id"]' "$SOPS" >/dev/null 2>&1; then
      echo "GITHUB_APP_ID=$(sops -d --extract '["github_app_id"]' "$SOPS")"
      echo "GITHUB_APP_PRIVATE_KEY_BASE64=$(sops -d --extract '["github_private_key_base64"]' "$SOPS")"
    else
      echo "WARNING: github app keys not readable from $SOPS — issue creation will use the gh CLI fallback" >&2
    fi
  } >"$ENV_FILE"
  chmod 600 "$ENV_FILE"

  cp "$UNIT_SRC" "$UNIT_DST"

  cd "$DST"
  bun install --silent
  echo "$SRC_HASH" > "$DST/.deploy-tree-hash"
  echo "$SRC_INPUTS" > "$DST/.deploy-inputs-hash"
fi

systemctl --user daemon-reload
systemctl --user enable pm-dispatch.service
if [ "$CHANGED" -eq 1 ]; then
  systemctl --user restart pm-dispatch.service
elif ! systemctl --user is-active --quiet pm-dispatch.service; then
  systemctl --user start pm-dispatch.service
fi

# Self-verify: poll /health until it reports the deployed BUILD_VERSION.
echo "== verifying deployed version =="
LIVE_VERSION=""
for _ in $(seq 1 15); do
  LIVE_VERSION="$(live_version)"
  [ "$LIVE_VERSION" = "$BUILD_VERSION" ] && break
  sleep 2
done
if [ "$LIVE_VERSION" != "$BUILD_VERSION" ]; then
  echo "ERROR: /health reports version '${LIVE_VERSION:-<no response>}' but this install deployed '${BUILD_VERSION}' - the running dispatch is NOT the deployed tree" >&2
  exit 1
fi
echo "health version verified: ${LIVE_VERSION}"
echo "== health (loopback) =="
curl -s "http://127.0.0.1:${PORT}/health"
echo
echo "== health (tailnet ${TAILNET_IP}) =="
curl -s "http://${TAILNET_IP}:${PORT}/health"
echo
systemctl --user is-active pm-dispatch.service
