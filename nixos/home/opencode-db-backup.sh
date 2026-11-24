#!/usr/bin/env bash
# opencode-db-backup.sh — daily backup of the OpenCode session database
# (developing-today/code#10297): ~/.local/share/opencode/opencode.db is
# the fleet's working memory (every live and historical OpenCode
# session) and existed in exactly one copy on one disk; the same-disk
# .bak copies next to it are not a backup, and laptop-state-backup
# covers ~/.local/state/opencode (a different, tiny directory).
#
# DESIGN: take a transaction-consistent snapshot of the live database
# with VACUUM INTO (a single WAL snapshot: writers proceed against the
# live DB while the copy reads one fixed snapshot), verify it with
# PRAGMA integrity_check, then restic-backup the snapshot file.
# VACUUM INTO rather than the sqlite backup API (.backup): on
# 2026-11-17 the first run's .backup pass restarted indefinitely --
# the destination stayed pinned at 12,415,795,200 bytes for 10+
# minutes because .backup restarts its copy whenever the source
# changes mid-pass, and this database is written continuously by
# session traffic. A WAL snapshot read cannot be invalidated that way.
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
#
# ZSTD FAST-RESTORE LEG (code#14515, research #9094 scheme (b)):
# after the restic legs, the staged opencode.db image is ALSO
# published as one dated zstd -3 full copy -- locally at
# ~/backups/opencode-db-zstd/<date>/ and on Google Drive at
# rclone:drive:Cloud/cold-store/db-backups/opencode/<date>/ (the
# per-DB dated layout the openclaw-backup pattern uses) -- with a
# sha256 manifest, and the Drive copy verified by re-reading it
# before success is recorded. Retention applies to the dated zstd
# directories only, on both targets, with restic's GFS semantics
# over calendar dates: keep-daily 7, keep-weekly 4, keep-monthly 12.
# The leg writes its own success-epoch marker
# (.opencode-db-zstd-last-success-epoch) only after both copies
# verify and retention has run, and the watchdog checks that marker
# separately: a successful restic run must not mask a failed or
# stale zstd leg. Measured economics (#9094): 31.03 GiB -> 5.22 GiB
# in 44 s; restore = download + ~56 s decompress + integrity_check,
# with no restic/rclone stack needed. Drive upload throughput is
# the weak point (a direct single-file rclone upload measured
# ~56 KiB/s on 2026-11-24 vs ~1.5-2 MiB/s for restic's chunked
# traffic), so the leg's freshness SLO budgets a >=1-day
# off-laptop lag until re-measured; same-day off-laptop coverage
# is NOT promised.
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
ZSTD_ROOT="$HOME/backups/opencode-db-zstd"
ZSTD_DRIVE_DIR="Cloud/cold-store/db-backups/opencode"
ZSTD_MARKER="$ROOT/.opencode-db-zstd-last-success-epoch"

log() { echo "opencode-db-backup: $*"; }
fail() {
  echo "opencode-db-backup: FAILED $*" >&2
  exit 1
}
ZSTD_TMP=""
trap 'rc=$?; if [ -n "$ZSTD_TMP" ]; then rm -f "$ZSTD_TMP"; fi; if [ $rc -ne 0 ]; then echo "opencode-db-backup: FAILED exit=$rc" >&2; fi' EXIT

mkdir -p "$ROOT"

# The zstd leg's artifact date. OPENCODE_DB_BACKUP_DATE overrides it
# for the test suite (a fixed historical date makes the retention
# expectations deterministic); the systemd timer never sets it.
RUN_DATE="${OPENCODE_DB_BACKUP_DATE:-$(date +%F)}"
if [[ ! $RUN_DATE =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] ||
  [ "$(date -d "$RUN_DATE" +%F 2>/dev/null)" != "$RUN_DATE" ]; then
  fail "invalid RUN_DATE '$RUN_DATE' - expected a real calendar date as YYYY-MM-DD"
fi

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
ZSTD="$(resolve_tool zstd)"
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
  "$SQLITE" "$SRC_DIR/$db" "VACUUM INTO '$STAGE/$db';" ||
    fail "sqlite VACUUM INTO snapshot failed for $db"
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

# --- zstd fast-restore leg (code#14515) -------------------------------------
# GFS retention over calendar dates (restic's keep-daily/keep-weekly/
# keep-monthly semantics, one artifact per day): keep the 7 newest
# days, the latest day of each of the 4 newest ISO weeks present, and
# the latest day of each of the 12 newest months present; the run
# date is always kept. Reads: $1 run date, $2 space-separated
# candidate dates (trailing slashes tolerated). Prints the dates to
# prune, one per line.
zstd_retention_prune() {
  python3 - "$1" "$2" <<'PYEOF'
import sys
from datetime import date

today = date.fromisoformat(sys.argv[1])
dates = []
for tok in sys.argv[2].split():
    try:
        dates.append(date.fromisoformat(tok.strip("/")))
    except ValueError:
        continue  # not a dated artifact dir; never pruned by this leg
dates = sorted(set(dates))
keep = set(dates[-7:])
weeks = {}
for d in dates:
    weeks.setdefault(d.isocalendar()[:2], []).append(d)
for w in sorted(weeks)[-4:]:
    keep.add(max(weeks[w]))
months = {}
for d in dates:
    months.setdefault((d.year, d.month), []).append(d)
for m in sorted(months)[-12:]:
    keep.add(max(months[m]))
keep.add(today)
for d in dates:
    if d not in keep:
        print(d.isoformat())
PYEOF
}

zstd_retention_local() {
  local candidates="" d prune day
  for d in "$ZSTD_ROOT"/20??-??-??; do
    [ -d "$d" ] || continue
    candidates+="$(basename "$d") "
  done
  prune="$(zstd_retention_prune "$RUN_DATE" "$candidates")"
  while IFS= read -r day; do
    [ -n "$day" ] || continue
    rm -rf "${ZSTD_ROOT:?}/$day"
    log "zstd retention: pruned local $day"
  done <<<"$prune"
}

zstd_retention_drive() {
  local listing prune day
  # A failed listing fails the run (the openclaw-backup #1035
  # lesson): an unreachable Drive must not read as an empty remote
  # with nothing to prune while the run reports success.
  listing="$(rclone lsf --dirs-only "drive:$ZSTD_DRIVE_DIR/")" ||
    fail "zstd retention: Drive listing failed"
  prune="$(zstd_retention_prune "$RUN_DATE" "$(printf '%s' "$listing" | tr '\n' ' ')")"
  while IFS= read -r day; do
    [ -n "$day" ] || continue
    rclone purge "drive:$ZSTD_DRIVE_DIR/$day" ||
      fail "zstd retention: Drive prune failed for $day"
    log "zstd retention: pruned Drive $day"
  done <<<"$prune"
}

zstd_leg() {
  local src="$STAGE/opencode.db"
  local artifact="opencode-db-$RUN_DATE.db.zst"
  local outdir="$ZSTD_ROOT/$RUN_DATE"
  local drive_dir="drive:$ZSTD_DRIVE_DIR/$RUN_DATE"
  local src_sha src_bytes zst_sha zst_bytes dec_sha got
  mkdir -p "$outdir"
  src_sha="$(sha256sum "$src" | cut -d' ' -f1)"
  src_bytes="$(stat -c%s "$src")"
  ZSTD_TMP="$outdir/.tmp-$artifact"
  rm -f "$ZSTD_TMP"
  "$ZSTD" -3 -q -f -o "$ZSTD_TMP" "$src" || fail "zstd compression failed for $src"
  # Verify BEFORE publish: the compressed bytes must decompress to
  # exactly the staged image (sha256 over the decompressed stream),
  # so a torn compression can never become the dated artifact.
  dec_sha="$("$ZSTD" -dc "$ZSTD_TMP" | sha256sum | cut -d' ' -f1)"
  [ "$dec_sha" = "$src_sha" ] ||
    fail "zstd verify failed: decompressed sha256 $dec_sha != staged image $src_sha"
  zst_sha="$(sha256sum "$ZSTD_TMP" | cut -d' ' -f1)"
  zst_bytes="$(stat -c%s "$ZSTD_TMP")"
  mv "$ZSTD_TMP" "$outdir/$artifact"
  ZSTD_TMP=""
  ZSTD_DATE="$RUN_DATE" ZSTD_CREATED="$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    ZSTD_FILE="$artifact" ZSTD_SHA256="$zst_sha" ZSTD_BYTES="$zst_bytes" \
    ZSTD_SRC_SHA256="$src_sha" ZSTD_SRC_BYTES="$src_bytes" \
    python3 - "$outdir/manifest.json" <<'PYEOF' || fail "zstd manifest generation failed"
import json, os, sys

manifest = {
    "archive": f"opencode-db-{os.environ['ZSTD_DATE']}",
    "created_at": os.environ["ZSTD_CREATED"],
    "host": os.uname().nodename,
    "scope": "laptop",
    "source_db": "opencode.db",
    "uncompressed_sha256": os.environ["ZSTD_SRC_SHA256"],
    "uncompressed_size": int(os.environ["ZSTD_SRC_BYTES"]),
    "files": [{
        "path": os.environ["ZSTD_FILE"],
        "sha256": os.environ["ZSTD_SHA256"],
        "size": int(os.environ["ZSTD_BYTES"]),
    }],
}
with open(sys.argv[1], "w") as fh:
    json.dump(manifest, fh, indent=2)
PYEOF
  log "zstd local OK $outdir/$artifact ($zst_bytes bytes, sha256 $zst_sha)"
  rclone copyto "$outdir/$artifact" "$drive_dir/$artifact" ||
    fail "zstd Drive upload failed ($artifact)"
  rclone copyto "$outdir/manifest.json" "$drive_dir/manifest.json" ||
    fail "zstd Drive upload failed (manifest.json)"
  # || got="": a failed cat must surface as the sha mismatch fail
  # below (clear message), not as a bare pipefail abort.
  got="$(rclone cat "$drive_dir/$artifact" 2>/dev/null | sha256sum | cut -d' ' -f1)" || got=""
  [ "$got" = "$zst_sha" ] || fail "zstd Drive copy sha256 mismatch (got $got)"
  log "zstd Drive OK sha256 verified ($drive_dir/$artifact)"
  zstd_retention_local
  zstd_retention_drive
  # The zstd success marker advances ONLY now: both copies verified
  # and retention applied. The watchdog reads it separately from the
  # restic marker, so neither leg can mask the other (#14515).
  date +%s >"$ZSTD_MARKER"
  export ZSTD_DATE="$RUN_DATE" ZSTD_FILE="$artifact" ZSTD_SHA256="$zst_sha" ZSTD_BYTES="$zst_bytes"
}

backup_backend "local" "$LOCAL_REPO"
backup_backend "r2" "$R2_REPO"
backup_backend "drive" "$DRIVE_REPO"

# The restic success marker is written as soon as the restic legs
# are verified, BEFORE the zstd leg: a zstd failure must not make a
# successful restic run look stale (and the zstd leg's own marker,
# written at its end, keeps the reverse true).
date +%s >"$MARKER"

zstd_leg

# --- completion evidence: run record (the watchdog reads the markers) ---
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
zstd_file = os.environ.get("ZSTD_FILE")
if zstd_file:
    record["zstd"] = {
        "date": os.environ.get("ZSTD_DATE", ""),
        "file": zstd_file,
        "bytes": int(os.environ.get("ZSTD_BYTES", "0")),
        "sha256": os.environ.get("ZSTD_SHA256", ""),
        "drive_dir": "Cloud/cold-store/db-backups/opencode/"
        + os.environ.get("ZSTD_DATE", ""),
    }
with open(record_path, "w") as fh:
    json.dump(record, fh, indent=1)
PYEOF
rm -rf "$STAGE"
log "OK opencode session DB backed up to local + R2 + Drive (restic dedup) + zstd fast-restore leg ($ZSTD_FILE); record $RECORD"
