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
#   3. scripts/export-turso.sh        file a portable SQLite export of
#                                     every Turso database in
#                                     db-backups/turso/ — only when the
#                                     Turso platform token below is set;
#                                     skipped (not an error) otherwise
#   4. cold-store prune-backups --apply
#                                     enforce 30 daily / 12 weekly /
#                                     12 monthly retention across all DBs
#   5. cold-store sync-manifest       push the local manifest (including
#                                     tonight's ledger rows) to D1
#   6. cold-store status              one-screen summary (head)
#
# Steps are ISOLATED: a step that fails never starves the steps after
# it (the 2026-10-27 run lost retention, manifest sync, and status to
# one failed Turso export). Each step prints a per-step outcome line
# (ok / FAILED / skipped), the status summary always prints, and the
# script exits non-zero AT THE END if any step failed — failure stays
# loud, it just stops being contagious. Guarded skips are not failures.
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
#   TURSO_PLATFORM_API_TOKEN
#                         optional. Turso Platform API token (Bitwarden:
#                         turso.platform-api-token-muse-cli). When unset,
#                         step 3 is skipped with a one-line notice
#                         instead of failing the run. See
#                         scripts/export-turso.sh for its own env.
#   COLD_STORE_HOME       optional; the reaper home (default
#                         ~/.config/cold-store). That directory's
#                         credentials.json holds the Drive grant.
#   COLD_STORE_BIN        optional; the cold-store executable
#                         (default: ../cold-store next to this script).
#                         Test seam, mirrors export-turso.sh.
#   EXPORT_TURSO_BIN      optional; the export-turso script
#                         (default: scripts/export-turso.sh next to
#                         this script). Test seam.
set -uo pipefail

: "${CLOUDFLARE_API_TOKEN:?CLOUDFLARE_API_TOKEN must be set — materialize it from Bitwarden into the environment first}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
COLD_STORE="${COLD_STORE_BIN:-${SCRIPT_DIR}/../cold-store}"
EXPORT_TURSO="${EXPORT_TURSO_BIN:-${SCRIPT_DIR}/export-turso.sh}"

FAILED_STEPS=()

# run_step <name> <command...>: run one step, record (never propagate)
# its failure, and print the per-step outcome.
run_step() {
  local name="$1"
  shift
  echo "nightly-backup: step ${name}: running"
  if "$@"; then
    echo "nightly-backup: step ${name}: ok"
  else
    local rc=$?
    echo "nightly-backup: step ${name}: FAILED (exit ${rc})" >&2
    FAILED_STEPS+=("${name}")
  fi
}

run_step backup-d1 "${COLD_STORE}" backup-d1
if [[ -n ${R2_ACCESS_KEY_ID:-} && -n ${R2_SECRET_ACCESS_KEY:-} ]]; then
  run_step backup-supabase "${COLD_STORE}" backup-supabase
else
  echo "nightly-backup: step backup-supabase: skipped (R2 credentials not set)"
fi
run_step export-turso "${EXPORT_TURSO}"
run_step prune-backups "${COLD_STORE}" prune-backups --apply
run_step sync-manifest "${COLD_STORE}" sync-manifest

echo "nightly-backup: step status: running"
# Never pipe status directly into head: head exits after 20 lines, closing
# the pipe while cold-store is still writing. Under `set -o pipefail` that
# SIGPIPE/BrokenPipeError turns a healthy run into FAILED. Capture the full
# output first so the verdict reflects only cold-store's own exit code, then
# truncate purely for display.
status_out="$(mktemp)"
status_rc=0
"${COLD_STORE}" status >"${status_out}" || status_rc=$?
head -n 20 "${status_out}"
rm -f "${status_out}"
if ((status_rc == 0)); then
  echo "nightly-backup: step status: ok"
else
  echo "nightly-backup: step status: FAILED (exit ${status_rc})" >&2
  FAILED_STEPS+=("status")
fi

if [[ ${#FAILED_STEPS[@]} -gt 0 ]]; then
  echo "nightly-backup: FAILED steps: ${FAILED_STEPS[*]} (${#FAILED_STEPS[@]} of 6 steps failed; all steps were attempted)" >&2
  exit 1
fi
echo "nightly-backup: all steps ok"
