#!/usr/bin/env bash
# nightly-backup.sh — the cold store's nightly DB-backup run
# (DESIGN.md section 5: nightly dumps into Drive db-backups/<db>/,
# retention enforced by trashing, never deleting).
#
# Steps, in order:
#   1. cold-store backup-d1           export the D1 cold-store-manifest
#                                     database and file it in db-backups/d1/
#   2. cold-store prune-backups --apply
#                                     enforce 30 daily / 12 weekly /
#                                     12 monthly retention across all DBs
#   3. cold-store sync-manifest       push the local manifest (including
#                                     tonight's ledger rows) to D1
#   4. cold-store status              one-screen summary (head)
#
# Environment — the operator/cron supplies these; the script itself
# stores nothing and echoes no secret values:
#   CLOUDFLARE_API_TOKEN  required. Materialize it from Bitwarden
#                         (cloudflare.api-token-day-to-day) into the
#                         cron environment; backup-d1 and sync-manifest
#                         read it from the environment only.
#   COLD_STORE_HOME       optional; the reaper home (default
#                         ~/.config/cold-store). That directory's
#                         credentials.json holds the Drive grant.
#
# Fails loudly: any failing step aborts the run non-zero (set -e).
set -euo pipefail

: "${CLOUDFLARE_API_TOKEN:?CLOUDFLARE_API_TOKEN must be set — materialize it from Bitwarden into the environment first}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
COLD_STORE="${SCRIPT_DIR}/../cold-store"

"${COLD_STORE}" backup-d1
"${COLD_STORE}" prune-backups --apply
"${COLD_STORE}" sync-manifest
"${COLD_STORE}" status | head -n 20
