#!/usr/bin/env bash
# opencode-db-backup.sh — daily backup of the OpenCode session database
# (developing-today/code#10297): ~/.local/share/opencode/opencode.db is
# the fleet's working memory (every live and historical OpenCode
# session) and existed in exactly one copy on one disk; the same-disk
# .bak copies next to it are not a backup, and laptop-state-backup
# covers ~/.local/state/opencode (a different, tiny directory).
#
# DESIGN: take a transaction-consistent snapshot of the live database
# with the sqlite backup API (sqlite3 .backup — safe against the
# running server, no lock stall beyond the copy), verify the snapshot
# with PRAGMA integrity_check, then restic-backup the snapshot file
# (content-defined chunking + dedup across days, the
# laptop-state-backup.sh pattern) to one restic repository per backend:
#   local  ~/backups/restic/opencode-db
#   R2     s3:<cloudflare.r2-endpoint>/supabase-backups/restic-opencode-db
#   Drive  rclone:drive:Cloud/backups/restic-opencode-db
# opencode-local.db rides along when present. The regenerable tmp/ and
# storage/ trees are out of scope (#10297). Retention per backend,
# applied ONLY after that backend's backup and check succeed:
# keep-last 7, keep-daily 7, keep-weekly 4, keep-monthly 12 (the
# monthly tier is Drew's 2026-11-03 retention ruling on #1258).
# The restic password reuses the existing restic.repo-archive-password
# Bitwarden key (fetched by name at runtime, never logged), as in
# laptop-state-backup.sh; the repositories are separate (independent
# retention/listing). On success the run writes a completion record
# (~/backups/restic/.opencode-db-last-run.json) and a success-epoch
# marker the laptop-backups watchdog reads (#1841 coverage).
# Runs unattended via the opencode-db-backup systemd user timer
# (nixos/home/laptop-backups.nix). Any failure exits non-zero with a
# loud FAILED line; success ends with one OK line.
set -euo pipefail

DB_TAG="opencode-db-daily"
SRC_DIR="$HOME/.local/share/opencode"
ROOT="$HOME/backups/restic"
STAGE="$HOME/backups/opencode-db-staging"
LOCAL_REPO="$ROOT/opencode-db"
R2_BUCKET="supabase-backups"
R2_PREFIX="restic-opencode-db"
DRIVE_REPO="rclone:drive:Cloud/backups/restic-opencode-db"
MARKER="$ROOT/.opencode-db-last-success-epoch"
RECORD="$ROOT/.opencode-db-last-run.json"
LOCK="$ROOT/.opencode-db-backup.lock"

log() { echo "opencode-db-backup: $*"; }
fail() {
  echo "opencode-db-backup: FAILED $*" >&2
  exit 1
}
trap 'rc=$?; if [ $rc -ne 0 ]; then echo "opencode-db-backup: FAILED exit=$rc" >&2; fi' EXIT

mkdir -p "$ROOT"

# --- single instance ---
exec 9>"$LOCK"
if ! flock -n 9; then
  log "SKIP another opencode-db-backup run holds the lock"
  exit 0
fi

# --- resolve the tool fallbacks (the systemd unit runs with a minimal
# --- PATH; the profile PATH normally provides these, the nix-store
# --- fallbacks keep the run working when it does not) ---
resolve_tool() { # $1 name; prints path or fails
  local name="$1" bin=""
  if bin="$(command -v "$name" 2>/dev/null)"; then
    printf '%s' "$bin"
    return 0
  fi
  bin="$(ls -d /nix/store/*-"$name"-[0-9]*/bin/"$name" 2>/dev/null | sort -V | tail -1)"
  [ -n "$bin" ] && [ -x "$bin" ] || fail "$name not found in PATH or nix store"
  printf '%s' "$bin"
}
RESTIC="$(resolve_tool restic)"
BWS="$(resolve_tool bws)"
SQLITE="$(resolve_tool sqlite3)"
if ! command -v rclone >/dev/null 2>&1; then
  RCLONE_BIN="$(resolve_tool rclone)"
  mkdir -p "$ROOT/.bin"
  ln -sf "$RCLONE_BIN" "$ROOT/.bin/rclone"
  export PATH="$ROOT/.bin:$PATH"
  log "rclone resolved via nix store: $RCLONE_BIN"
fi

# --- sources: the session DB is required, the local DB optional ---
[ -f "$SRC_DIR/opencode.db" ] || fail "session database $SRC_DIR/opencode.db is missing"
DBS=("opencode.db")
[ -f "$SRC_DIR/opencode-local.db" ] && DBS+=("opencode-local.db")
log "sources: ${DBS[*]}"

# --- consistent snapshots via the sqlite backup API ---
rm -rf "$STAGE"
mkdir -p "$STAGE"
SNAPSHOTS=()
for db in "${DBS[@]}"; do
  log "snapshot start: $db"
  "$SQLITE" "$SRC_DIR/$db" ".backup $STAGE/$db" ||
    fail "sqlite .backup failed for $db"
  verdict="$("$SQLITE" "$STAGE/$db" "PRAGMA integrity_check;" | head -1)"
  [ "$verdict" = "ok" ] || fail "integrity_check on staged $db returned: $verdict"
  SNAPSHOTS+=("$STAGE/$db")
  log "snapshot ok: $db ($(du -h "$STAGE/$db" | cut -f1))"
done

# --- credentials from Bitwarden (names only in code/logs) ---
if [ -z "${BWS_ACCESS_TOKEN:-}" ] && [ -f "$HOME/.config/bws/access-token" ]; then
  BWS_ACCESS_TOKEN="$(cat "$HOME/.config/bws/access-token")"
  export BWS_ACCESS_TOKEN
fi
BWS_LIST="$("$BWS" secret list 2>/dev/null)" || fail "bws secret list failed"
bws_get() {
  printf '%s' "$BWS_LIST" | python3 -c 'import json,sys
k = sys.argv[1]
for s in json.load(sys.stdin):
    if s.get("key") == k:
        print(s.get("value", "")); break' "$1"
}
RESTIC_PASSWORD="$(bws_get restic.repo-archive-password)"
[ -n "$RESTIC_PASSWORD" ] || fail "restic.repo-archive-password missing in Bitwarden"
export RESTIC_PASSWORD
AWS_ACCESS_KEY_ID="$(bws_get cloudflare.r2-backups-access-key-id)"
AWS_SECRET_ACCESS_KEY="$(bws_get cloudflare.r2-backups-secret-access-key)"
R2_ENDPOINT="$(bws_get cloudflare.r2-endpoint)"
[ -n "$AWS_ACCESS_KEY_ID" ] && [ -n "$AWS_SECRET_ACCESS_KEY" ] && [ -n "$R2_ENDPOINT" ] ||
  fail "R2 credentials incomplete in Bitwarden"
export AWS_ACCESS_KEY_ID AWS_SECRET_ACCESS_KEY AWS_EC2_METADATA_DISABLED=true
R2_REPO="s3:${R2_ENDPOINT}/${R2_BUCKET}/${R2_PREFIX}"

init_if_needed() { # $1 repo
  if ! "$RESTIC" -r "$1" cat config >/dev/null 2>&1; then
    log "initializing restic repository $1"
    "$RESTIC" -r "$1" init || fail "restic init failed for $1"
  fi
}

backup_backend() { # $1 label, $2 repo
  local label="$1" repo="$2" out added
  init_if_needed "$repo"
  log "backup start: $label ($repo)"
  out="$("$RESTIC" -r "$repo" backup "${SNAPSHOTS[@]}" --tag "$DB_TAG" 2>&1)" ||
    {
      printf '%s\n' "$out" >&2
      fail "restic backup failed for $label"
    }
  printf '%s\n' "$out" | grep -E '^(Files:|Dirs:|Added to the repository|Data Blobs|Tree Blobs|total )' || true
  added="$(printf '%s\n' "$out" | grep '^Added to the repository' || true)"
  log "backup done: $label — $added"
  "$RESTIC" -r "$repo" check >/dev/null 2>&1 || fail "restic check failed for $label"
  log "check ok: $label"
  # retention only ever runs after a successful, verified backup
  "$RESTIC" -r "$repo" forget --tag "$DB_TAG" \
    --keep-last 7 --keep-daily 7 --keep-weekly 4 --keep-monthly 12 --prune >/dev/null 2>&1 ||
    fail "restic forget/prune failed for $label"
  log "retention applied: $label (keep-last 7, keep-daily 7, keep-weekly 4, keep-monthly 12)"
}

backup_backend "local" "$LOCAL_REPO"
backup_backend "r2" "$R2_REPO"
backup_backend "drive" "$DRIVE_REPO"

# --- completion evidence: success marker + run record (watchdog) ---
date +%s >"$MARKER"
python3 - "$RECORD" "${DBS[@]}" <<'PYEOF'
import json, os, sys
from datetime import datetime, timezone

record_path = sys.argv[1]
dbs = sys.argv[2:]
home = os.path.expanduser("~")
sizes = {}
for db in dbs:
    staged = os.path.join(home, "backups", "opencode-db-staging", db)
    if os.path.exists(staged):
        sizes[db] = os.path.getsize(staged)
record = {
    "finished_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "result": "complete",
    "databases": dbs,
    "snapshot_bytes": sizes,
    "backends": ["local", "r2", "drive"],
    "tag": "opencode-db-daily",
}
with open(record_path, "w") as fh:
    json.dump(record, fh, indent=1)
PYEOF
rm -rf "$STAGE"
log "OK opencode session DB backed up to local + R2 + Drive (restic dedup; record $RECORD)"
