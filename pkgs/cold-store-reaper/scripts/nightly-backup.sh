#!/usr/bin/env bash
# nightly-backup.sh — the cold store's nightly DB-backup run
# (DESIGN.md section 5: nightly dumps into Drive db-backups/<db>/,
# retention enforced by trashing, never deleting).
#
# Steps, in order:
#   1. cold-store backup-d1           export the D1 cold-store-manifest
#                                     database and file it in db-backups/d1/
#   2. cold-store backup-supabase     file the newest Supabase backup object
#                                     from R2 in db-backups/supabase/ —
#                                     only when the R2 credentials below
#                                     are set; skipped (not an error)
#                                     otherwise
#   3. cold-store prune-backups --apply
#                                     enforce 30 daily / 12 weekly /
#                                     12 monthly retention across all DBs
#   4. cold-store sync-manifest       push the local manifest (including
#                                     tonight's ledger rows) to D1
#   5. cold-store status              one-screen summary (head)
#
# Environment — the operator/cron supplies these; the script itself
# stores nothing and echoes no secret values:
#   CLOUDFLARE_API_TOKEN  required. Materialize it from Bitwarden
#                         (cloudflare.api-token-day-to-day) into the
#                         cron environment; backup-d1 and sync-manifest
#                         read it from the environment only.
#   R2_ACCESS_KEY_ID      optional. Bucket-scoped S3 credentials for the
#   R2_SECRET_ACCESS_KEY  supabase-backups bucket (Bitwarden:
#                         cloudflare.r2-backups-access-key-id /
#                         cloudflare.r2-backups-secret-access-key).
#                         When either is unset, step 2 is skipped with
#                         a one-line notice instead of failing the run.
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
if [[ -n ${R2_ACCESS_KEY_ID:-} && -n ${R2_SECRET_ACCESS_KEY:-} ]]; then
  "${COLD_STORE}" backup-supabase
else
  echo "nightly-backup: R2 credentials not set — skipping backup-supabase"
fi
"${COLD_STORE}" prune-backups --apply
"${COLD_STORE}" sync-manifest
"${COLD_STORE}" status | head -n 20
