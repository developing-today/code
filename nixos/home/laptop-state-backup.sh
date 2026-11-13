#!/usr/bin/env bash
# laptop-state-backup.sh — daily backup of the small irreplaceable
# laptop state that no other routine covers (developing-today/code#1849):
#   ~/.local/state/opencode   agent session/auth state (~0.5M)
#   ~/secrets/*.sops.yaml     sops-encrypted secrets (ciphertext only)
#   ~/.ssh                    SSH keys + known_hosts (irreplaceable)
#
# DESIGN: restic with content-defined chunking + dedup, snapshotting
# the source paths directly — the repo-archive-backup.sh
# (~/tools/repo-archive-backup.sh) pattern, no new backup tool. One
# restic repository per backend, same password everywhere:
#   local  ~/backups/restic/laptop-state
#   R2     s3:<cloudflare.r2-endpoint>/supabase-backups/restic-laptop-state
#   Drive  rclone:drive:Cloud/backups/restic-laptop-state
# Retention per backend, applied ONLY after that backend's backup and
# check succeed: keep-last 7, keep-daily 7, keep-weekly 4, with prune.
# The restic password and the R2 credentials are fetched at runtime
# from Bitwarden Secrets Manager, referenced by key name only and
# never logged. The restic password reuses the existing
# restic.repo-archive-password key so no new Bitwarden item is needed;
# the repositories are separate (independent retention/listing).
# Runs unattended via the laptop-state-backup systemd user timer
# (nixos/home/laptop-backups.nix). Any failure exits non-zero with a
# loud FAILED line; success ends with one OK line.
#
# Out of scope by decision on #1849 (do-not-back-up classes, each with
# its reason in the per-class inventory posted there): browser profiles
# (size, file locks, plaintext credential sprawl), password-vault files
# (no local vault files exist; Bitwarden cloud + Secrets Manager are
# already off-machine), ~/atlas trees beyond atlas.db (git-tracked or
# regenerable scratch; atlas.db itself rides atlas-backup.sh).
set -euo pipefail

STATE_TAG="laptop-state-daily"
ROOT="$HOME/backups/restic"
LOCAL_REPO="$ROOT/laptop-state"
R2_BUCKET="supabase-backups"
R2_PREFIX="restic-laptop-state"
DRIVE_REPO="rclone:drive:Cloud/backups/restic-laptop-state"
MARKER="$ROOT/.laptop-state-last-success-epoch"
LOCK="$ROOT/.laptop-state-backup.lock"

log() { echo "laptop-state-backup: $*"; }
fail() { echo "laptop-state-backup: FAILED $*" >&2; exit 1; }
trap 'rc=$?; if [ $rc -ne 0 ]; then echo "laptop-state-backup: FAILED exit=$rc" >&2; fi' EXIT

mkdir -p "$ROOT"

# --- single instance ---
exec 9>"$LOCK"
if ! flock -n 9; then
  log "SKIP another laptop-state-backup run holds the lock"
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
if ! command -v rclone >/dev/null 2>&1; then
  RCLONE_BIN="$(resolve_tool rclone)"
  mkdir -p "$ROOT/.bin"
  ln -sf "$RCLONE_BIN" "$ROOT/.bin/rclone"
  export PATH="$ROOT/.bin:$PATH"
  log "rclone resolved via nix store: $RCLONE_BIN"
fi

# --- collect the source paths (fail only if NOTHING is present) ---
SOURCES=()
[ -d "$HOME/.local/state/opencode" ] && SOURCES+=("$HOME/.local/state/opencode")
[ -d "$HOME/.ssh" ] && SOURCES+=("$HOME/.ssh")
for f in "$HOME"/secrets/*.sops.yaml; do
  [ -f "$f" ] && SOURCES+=("$f")
done
[ "${#SOURCES[@]}" -gt 0 ] || fail "no source paths present (opencode state, .ssh, secrets/*.sops.yaml all missing)"
log "sources: ${SOURCES[*]}"

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
[ -n "$AWS_ACCESS_KEY_ID" ] && [ -n "$AWS_SECRET_ACCESS_KEY" ] && [ -n "$R2_ENDPOINT" ] \
  || fail "R2 credentials incomplete in Bitwarden"
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
  out="$("$RESTIC" -r "$repo" backup "${SOURCES[@]}" --tag "$STATE_TAG" 2>&1)" \
    || { printf '%s\n' "$out" >&2; fail "restic backup failed for $label"; }
  printf '%s\n' "$out" | grep -E '^(Files:|Dirs:|Added to the repository|Data Blobs|Tree Blobs|total )' || true
  added="$(printf '%s\n' "$out" | grep '^Added to the repository' || true)"
  log "backup done: $label — $added"
  "$RESTIC" -r "$repo" check >/dev/null 2>&1 || fail "restic check failed for $label"
  log "check ok: $label"
  # retention only ever runs after a successful, verified backup
  "$RESTIC" -r "$repo" forget --tag "$STATE_TAG" \
    --keep-last 7 --keep-daily 7 --keep-weekly 4 --prune >/dev/null 2>&1 \
    || fail "restic forget/prune failed for $label"
  log "retention applied: $label (keep-last 7, keep-daily 7, keep-weekly 4)"
}

backup_backend "local" "$LOCAL_REPO"
backup_backend "r2" "$R2_REPO"
backup_backend "drive" "$DRIVE_REPO"

date +%s > "$MARKER"
log "OK laptop state backed up to local + R2 + Drive (restic dedup; marker $MARKER)"
