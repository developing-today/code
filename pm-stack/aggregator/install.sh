#!/usr/bin/env bash
# Install/update the aggregator on the laptop.
#
# Copies this package to ~/pm-stack/aggregator, renders the service
# env file from ~/secrets/pm-stack.sops.yaml (keys by name; values
# never printed or committed), installs the user systemd unit, and
# restarts the service ONLY when the deployed tree changed (#1150);
# the install then fails loudly unless /api/health reports the build
# version it just deployed. Kaneo/Grist stay their own deployments; the
# aggregator only reads them over their local HTTP APIs.
#
# Usage (on the laptop):  bash install.sh
# LAN/tailnet bind is PM_AGG_HOSTS in the rendered env file: loopback +
# the laptop's tailnet address (Drew, 2026-10-25: LAN/tailnet fine,
# public only behind a password — no public route here).
set -euo pipefail
cd "$(dirname "$0")"
SRC="$PWD"
DST="$HOME/pm-stack/aggregator"
SOPS="$HOME/secrets/pm-stack.sops.yaml"
UNIT_SRC="$HOME/pm-stack/systemd/pm-aggregator.service"
UNIT_DST="$HOME/.config/systemd/user/pm-aggregator.service"
TAILNET_IP="${TAILNET_IP:-100.99.113.103}"

mkdir -p "$DST" "$HOME/.config/systemd/user"

# --- Change detection + build marker (#1150) ---------------------------
# BUILD_VERSION is what /api/health must report once this deploy is
# live: the source commit when the package sits in a git checkout,
# else the tree hash. The tree hash covers exactly the files the copy
# below installs; the last deployed hash is kept in $DST so an
# unchanged tree is a true no-op (no copy, no restart). Deployment
# inputs that are not in the tree — the tailnet bind address, the SOPS
# env source and the unit file — are hashed separately (see
# deploy_inputs_hash), so rotating a credential or editing the unit is
# a real change instead of a silent no-op.
DEPLOY_COMMIT="$(git -C "$SRC" rev-parse HEAD 2>/dev/null || true)"
tree_hash() {
  local root="$1"
  (cd "$root" && {
    for f in package.json bun.lock tsconfig.json fleet-feed.json; do
      [ -f "$f" ] && echo "$f"
    done
    find src test scripts opencode-tools -type f 2>/dev/null
  } | LC_ALL=C sort | while IFS= read -r f; do sha256sum "$f"; done | sha256sum | cut -d' ' -f1)
}
SRC_HASH="$(tree_hash "$SRC")"
DST_HASH=""
[ -f "$DST/.deploy-tree-hash" ] && DST_HASH="$(cat "$DST/.deploy-tree-hash")"
# Deployment inputs the tree hash cannot see (#1150, Codex P2 on PR 2432):
# TAILNET_IP is rendered into .env, and both the SOPS file and the unit
# are installed verbatim. Hashing the SOPS *ciphertext* and the unit file
# keeps this comparison secret-free (no value is ever decrypted or
# printed here) while making a rotated credential or an edited unit a
# real change. A missing file hashes as "absent" so the first run that
# gains one is also a change.
deploy_inputs_hash() {
  {
    printf 'TAILNET_IP=%s\n' "$TAILNET_IP"
    if [ -f "$SOPS" ]; then sha256sum "$SOPS"; else echo "sops:absent"; fi
    if [ -f "$UNIT_SRC" ]; then sha256sum "$UNIT_SRC"; else echo "unit:absent"; fi
  } | sha256sum | cut -d' ' -f1
}
SRC_INPUTS="$(deploy_inputs_hash)"
DST_INPUTS=""
[ -f "$DST/.deploy-inputs-hash" ] && DST_INPUTS="$(cat "$DST/.deploy-inputs-hash")"
BUILD_VERSION="${DEPLOY_COMMIT:-tree:${SRC_HASH}}"
live_version() {
  # Parse with bun (already a hard dependency of this script): the API
  # pretty-prints its JSON, so a fixed-string grep for the marker
  # would silently never match (#1150, Codex review on PR 2432).
  curl -s --max-time 3 "http://127.0.0.1:4870/api/health" | bun -e 'const t = await Bun.stdin.text(); try { console.log(JSON.parse(t).version ?? "") } catch { console.log("") }' || true
}
CHANGED=1
if [ "$SRC_HASH" = "$DST_HASH" ] && [ "$SRC_INPUTS" = "$DST_INPUTS" ] && systemctl --user is-active --quiet pm-aggregator.service && [ "$(live_version)" = "$BUILD_VERSION" ]; then
  CHANGED=0
  echo "aggregator unchanged (tree ${SRC_HASH:0:12}, inputs ${SRC_INPUTS:0:12}, version ${BUILD_VERSION}) - no copy, no restart"
elif [ "$SRC_HASH" = "$DST_HASH" ] && [ "$SRC_INPUTS" != "$DST_INPUTS" ]; then
  echo "aggregator tree unchanged but deployment inputs changed (tailnet/SOPS/unit) - re-rendering env and restarting"
fi

if [ "$CHANGED" -eq 1 ]; then
# Package files (no node_modules, no local feed/env artifacts).
for f in package.json bun.lock tsconfig.json fleet-feed.json; do
  [ -f "$SRC/$f" ] && cp "$SRC/$f" "$DST/$f"
done
rm -rf "$DST/src" "$DST/test" "$DST/scripts" "$DST/opencode-tools"
cp -r "$SRC/src" "$DST/src"
cp -r "$SRC/test" "$DST/test"
cp -r "$SRC/scripts" "$DST/scripts"
cp -r "$SRC/opencode-tools" "$DST/opencode-tools"

# Service env from the SOPS file — same pattern as Kaneo's postgres.env.
umask 077
ENV_FILE="$DST/.env"
{
  echo "PM_AGG_BUILD_VERSION=${BUILD_VERSION}"
  echo "PM_AGG_SOURCE=live"
  echo "PM_AGG_HOSTS=127.0.0.1,${TAILNET_IP}"
  echo "PM_AGG_FEED_FILE=${DST}/fleet-feed.json"
  echo "PM_PROJECTOR_URL=http://127.0.0.1:7801"
  for key in kaneo_admin_email:KANEO_ADMIN_EMAIL kaneo_admin_password:KANEO_ADMIN_PASSWORD grist_api_key:GRIST_API_KEY; do
    sops_key="${key%%:*}"
    env_name="${key##*:}"
    value="$(sops -d --extract "[\"${sops_key}\"]" "$SOPS")"
    echo "${env_name}=${value}"
  done
} >"$ENV_FILE"
chmod 600 "$ENV_FILE"

cp "$UNIT_SRC" "$UNIT_DST"

cd "$DST"
bun install --silent
echo "$SRC_HASH" > "$DST/.deploy-tree-hash"
echo "$SRC_INPUTS" > "$DST/.deploy-inputs-hash"
fi

systemctl --user daemon-reload
systemctl --user enable pm-aggregator.service
if [ "$CHANGED" -eq 1 ]; then
  systemctl --user restart pm-aggregator.service
elif ! systemctl --user is-active --quiet pm-aggregator.service; then
  systemctl --user start pm-aggregator.service
fi

# Self-verify (#1150): poll /api/health until it reports the deployed
# BUILD_VERSION. A mismatch - old process still serving, restart
# failed, wrong tree - fails the install loudly instead of printing a
# health body nobody checks.
echo "== verifying deployed version =="
LIVE_VERSION=""
for _ in $(seq 1 15); do
  LIVE_VERSION="$(live_version)"
  [ "$LIVE_VERSION" = "$BUILD_VERSION" ] && break
  sleep 2
done
if [ "$LIVE_VERSION" != "$BUILD_VERSION" ]; then
  echo "ERROR: /api/health reports version '${LIVE_VERSION:-<no response>}' but this install deployed '${BUILD_VERSION}' - the running aggregator is NOT the deployed tree" >&2
  exit 1
fi
echo "health version verified: ${LIVE_VERSION}"
echo "== health (loopback) =="
curl -s "http://127.0.0.1:4870/api/health"
echo
echo "== health (tailnet ${TAILNET_IP}) =="
curl -s "http://${TAILNET_IP}:4870/api/health"
echo
systemctl --user is-active pm-aggregator.service

# Quota gatherer (#1151): a user timer on the same 15-minute cadence
# as the Meta file exporter (#1301), running the deployed package's
# scripts/gather-quota.ts. Units ship with this package (systemd/);
# a ~/pm-stack/systemd copy wins when present, matching how the
# aggregator unit itself is sourced. Guarded: a checkout without
# the units skips this block instead of failing the install.
GATHERER_SVC_SRC="$SRC/systemd/quota-gatherer.service"
GATHERER_TMR_SRC="$SRC/systemd/quota-gatherer.timer"
[ -f "$HOME/pm-stack/systemd/quota-gatherer.service" ] && GATHERER_SVC_SRC="$HOME/pm-stack/systemd/quota-gatherer.service"
[ -f "$HOME/pm-stack/systemd/quota-gatherer.timer" ] && GATHERER_TMR_SRC="$HOME/pm-stack/systemd/quota-gatherer.timer"
if [ -f "$GATHERER_SVC_SRC" ] && [ -f "$GATHERER_TMR_SRC" ]; then
  cp "$GATHERER_SVC_SRC" "$HOME/.config/systemd/user/quota-gatherer.service"
  cp "$GATHERER_TMR_SRC" "$HOME/.config/systemd/user/quota-gatherer.timer"
  systemctl --user daemon-reload
  systemctl --user enable --now quota-gatherer.timer
  echo "quota-gatherer.timer: $(systemctl --user is-active quota-gatherer.timer)"
else
  echo "quota-gatherer units not found — gatherer timer not installed"
fi
