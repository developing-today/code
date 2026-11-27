#!/usr/bin/env bash
# nightly-backup.sh — the cold store's nightly DB-backup run
# (DESIGN.md section 5: nightly dumps into Drive db-backups/<db>/,
# retention enforced by trashing, never deleting).
#
# Steps, in order:
#   1. cold-store backup-d1           export the D1 cold-store-manifest
#                                     database and file it in db-backups/d1/
#   2. scripts/export-turso.sh        file a portable SQLite export of
#                                     every Turso database in
#                                     db-backups/turso/ — only when the
#                                     Turso platform token below is set;
#                                     skipped (not an error) otherwise
#   3. cold-store prune-backups --apply
#                                     enforce 30 daily / 12 weekly /
#                                     12 monthly retention across all DBs
#   4. cold-store sync-manifest       push the local manifest (including
#                                     tonight's ledger rows) to D1
#   5. cold-store status              one-screen summary (head)
#
# Steps are ISOLATED: a step that fails never starves the steps after
# it (the 2026-10-27 run lost retention, manifest sync, and status to
# one failed Turso export). Each step prints a per-step outcome line
# (ok / FAILED / skipped), the status summary always prints, and the
# script exits non-zero AT THE END if any step failed — failure stays
# loud, it just stops being contagious. Guarded skips are not failures.
#
# Durable run log (#13115): the whole run's output is tee'd to
# ${COLD_STORE_HOME}/logs/nightly-<timestamp>.log (with a
# nightly-latest.log symlink pointing at the newest run), so a failed
# night names its failing step from the file alone. The systemd
# journal retained NO entries for the 2026-11-22 run, which is why
# that night's failure stage could never be confirmed after the fact.
# stdout and stderr merge into one chronological stream — both in the
# log and for the caller — so the record reads in order; the exit code
# is still the run's own verdict (PIPESTATUS, never tee's). If the log
# directory cannot be created the run proceeds without the file and
# says so loudly: logging must never break the backup.
#
# Environment — the operator/cron supplies these; the script itself
# stores nothing and echoes no secret values:
#   CLOUDFLARE_API_TOKEN  required. Materialize it from Bitwarden
#                         (cloudflare.api-token-day-to-day) into the
#                         cron environment; backup-d1 and sync-manifest
#                         read it from the environment only.
#   TURSO_PLATFORM_API_TOKEN
#                         optional. Turso Platform API token (Bitwarden:
#                         turso.platform-api-token-muse-cli). When unset,
#                         step 2 is skipped with a one-line notice
#                         instead of failing the run. See
#                         scripts/export-turso.sh for its own env.
#   COLD_STORE_HOME       optional; the reaper home (default
#                         ~/.config/cold-store). That directory's
#                         credentials.json holds the Drive grant, and
#                         its logs/ subdirectory holds the run logs.
#   COLD_STORE_BIN        optional; the cold-store executable
#                         (default: ../cold-store next to this script).
#                         Test seam, mirrors export-turso.sh.
#   EXPORT_TURSO_BIN      optional; the export-turso script
#                         (default: scripts/export-turso.sh next to
#                         this script). Test seam.
# Arguments: none. Reject every argument before creating the run log
# directory or running backup steps.
set -uo pipefail

if (($# > 0)); then
  printf 'nightly-backup.sh: unexpected argument: %s (usage: takes no arguments)\n' "$1" >&2
  exit 2
fi

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

main() {
  : "${CLOUDFLARE_API_TOKEN:?CLOUDFLARE_API_TOKEN must be set — materialize it from Bitwarden into the environment first}"

  SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  COLD_STORE="${COLD_STORE_BIN:-${SCRIPT_DIR}/../cold-store}"
  EXPORT_TURSO="${EXPORT_TURSO_BIN:-${SCRIPT_DIR}/export-turso.sh}"

  FAILED_STEPS=()

  run_step backup-d1 "${COLD_STORE}" backup-d1
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
    return 1
  fi
  echo "nightly-backup: all steps ok"
}

# --- durable run log (#13115) -------------------------------------------
COLD_STORE_HOME="${COLD_STORE_HOME:-${HOME}/.config/cold-store}"
LOG_DIR="${COLD_STORE_HOME}/logs"
LOG_FILE=""
if mkdir -p "${LOG_DIR}" 2>/dev/null; then
  LOG_FILE="${LOG_DIR}/nightly-$(date +%Y%m%dT%H%M%S).log"
fi

if [[ -n ${LOG_FILE} ]]; then
  ln -sfn "${LOG_FILE}" "${LOG_DIR}/nightly-latest.log" 2>/dev/null || true
  echo "nightly-backup: durable run log: ${LOG_FILE}" | tee -a "${LOG_FILE}"
  main "$@" 2>&1 | tee -a "${LOG_FILE}"
  exit "${PIPESTATUS[0]}"
fi
echo "nightly-backup: WARNING: cannot create log directory ${LOG_DIR} — running without a durable run log" >&2
main "$@"
