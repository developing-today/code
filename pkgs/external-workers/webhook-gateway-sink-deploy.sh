#!/usr/bin/env bash
# Deploys the webhook-gateway-sink Worker whose source lives in
# dezren39/hatch-personal (issue #5767, external-Worker registration
# slice). The worker is dependency-free (no package.json — wrangler
# uploads src/ + wrangler.toml directly), so the deploy needs no build
# and no deploy-time npm install: it stages the pinned source tree and
# runs `wrangler deploy`.
#
# Ordering note: this sink is the stand-in target the webhook-gateway
# reaches through its SINK service binding — deploy the sink before the
# gateway on a fresh account.
#
# Usage:
#   webhook-gateway-sink-deploy.sh [--dry-run] [--rev <sha>]
#
# --dry-run stages the source and runs `wrangler deploy --dry-run`
# (validates wrangler.toml, uploads nothing, needs no credentials).
# A full deploy needs CLOUDFLARE_ACCOUNT_ID (decrypted from sops here,
# like the other cloudflare deploy apps) and a Cloudflare token
# wrangler can use (CLOUDFLARE_API_TOKEN env or wrangler login).
# The sink's KV namespace (SINK_LOG) is provisioned out of band and
# never carried here.
#
# Source overrides (default: the pinned rev in the deploy.nix that
# wraps this script):
#   HATCH_PERSONAL_REPO  owner/repo (default dezren39/hatch-personal)
#   HATCH_PERSONAL_REV   full commit SHA to deploy
#   HATCH_WORKER_PATH    worker dir inside that repo
set -euo pipefail

REPO="${HATCH_PERSONAL_REPO:?HATCH_PERSONAL_REPO must be set by the deploy app}"
PINNED_REV="${HATCH_PERSONAL_REV:?HATCH_PERSONAL_REV must be set by the deploy app}"
SUBPATH="${HATCH_WORKER_PATH:?HATCH_WORKER_PATH must be set by the deploy app}"
WORKER="webhook-gateway-sink"

DRY_RUN=0
REV="$PINNED_REV"
while [ $# -gt 0 ]; do
  case "$1" in
  --dry-run)
    DRY_RUN=1
    shift
    ;;
  --rev)
    REV="${2:?--rev needs a value}"
    shift 2
    ;;
  --rev=*)
    REV="${1#--rev=}"
    shift
    ;;
  -h | --help)
    echo "Usage: $0 [--dry-run] [--rev <sha>]"
    exit 0
    ;;
  *)
    echo "error: unknown argument $1 (usage: $0 [--dry-run] [--rev <sha>])" >&2
    exit 2
    ;;
  esac
done

if [ "$REV" != "$PINNED_REV" ]; then
  echo "warning: deploying $WORKER at $REV, not the pinned $PINNED_REV" >&2
fi

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
echo "staging $REPO $SUBPATH at $REV ..." >&2
git clone --quiet --filter=blob:none --no-checkout "https://github.com/${REPO}.git" "$STAGE/checkout"
git -C "$STAGE/checkout" sparse-checkout set "$SUBPATH"
git -C "$STAGE/checkout" checkout --quiet "$REV"
WORKDIR="$STAGE/checkout/$SUBPATH"
[ -f "$WORKDIR/wrangler.toml" ] || {
  echo "error: $SUBPATH has no wrangler.toml at $REV" >&2
  exit 1
}

if [ "$DRY_RUN" = 1 ]; then
  (cd "$WORKDIR" && wrangler deploy --dry-run)
  exit 0
fi

repo_root="$(git rev-parse --show-toplevel)"
CLOUDFLARE_ACCOUNT_ID="$(sops decrypt --extract '["cloudflare_account_id"]' "${repo_root}/secrets/sops/common/cloudflare.yaml")"
export CLOUDFLARE_ACCOUNT_ID
(cd "$WORKDIR" && wrangler deploy)
