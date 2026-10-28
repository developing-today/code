#!/usr/bin/env bash
# Install/update the aggregator on the laptop.
#
# Copies this package to ~/pm-stack/aggregator, renders the service
# env file from ~/secrets/pm-stack.sops.yaml (keys by name; values
# never printed or committed), installs the user systemd unit, and
# restarts the service. Kaneo/Grist stay their own deployments; the
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

systemctl --user daemon-reload
systemctl --user enable --now pm-aggregator.service
sleep 2
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
