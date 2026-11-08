# atlas-backup — daily backup of the canonical local atlas.db
# (developing-today/code#1515).
#
# The local atlas.db is gitignored and holds content that exists nowhere
# else (6,941 local-only chunks never published to Turso). The canonical
# atlas.db lives on this laptop (~/atlas/turso-atlas/data/atlas.db, since
# 2026-10-28). This job used to run from the sandbox (a cron executing
# ~/hatch-personal/workspace/turso-atlas/scripts/atlas-backup.py, which
# snapshotted the DB here over SSH and pulled ~500 MB through the sandbox
# for filing); it was migrated into this flake 2026-10-30 per Drew's
# laptop-only execution rule, and now runs natively on the laptop as a
# systemd user service + timer declared in nixos/home/laptop-backups.nix.
#
# Each run snapshots the DB consistently (sqlite3 .backup), gzips it
# deterministically (mtime 0, no stored name), and files the SAME bytes
# to three destinations:
#   1. Laptop: ~/backups/atlas/<DATE>/atlas-<DATE>.db.gz (+ manifest)
#   2. R2:     bucket supabase-backups, key atlas/<DATE>/atlas-<DATE>.db.gz
#   3. Drive:  Cloud/cold-store/db-backups/local-atlas/ (via rclone "drive")
# Every offsite copy is sha256-verified against the local archive.
#
# Dedupe is by content hash: if the gz sha256 equals the last filed
# snapshot's (per ~/backups/atlas/backup-log.md), all legs are skipped
# (UNCHANGED). Rotation touches only this routine's own artifacts:
# newest 7 dates on laptop + R2, newest 4 archives on Drive (rclone
# delete on the drive remote trashes, never hard-deletes).
#
# Every run appends one line to ~/backups/atlas/backup-log.md and prints
# a verdict line:
#   ATLAS-BACKUP <DATE> OK sha256=<hex> size=<n> laptop=ok r2=ok drive=ok ...
#   ATLAS-BACKUP <DATE> UNCHANGED sha256=<hex>
#   ATLAS-BACKUP <DATE> FAILED legs=<names> sha256=<hex> size=<n> ...
#   ATLAS-BACKUP <DATE> FAILED early: <error>
# Exit code 0 on OK/UNCHANGED, 1 if any leg failed.
#
# Credentials come from Bitwarden Secrets Manager at runtime, referenced
# by key name only and never logged (the pm-dashboard-backup pattern):
# cloudflare.r2-backups-access-key-id, cloudflare.r2-backups-secret-
# access-key, cloudflare.r2-endpoint. The Drive leg uses the laptop's
# existing rclone "drive" remote.

DATE="$(date +%F)"
SKIP_DEDUPE=0
while [ $# -gt 0 ]; do
  case "$1" in
    --date)
      DATE="$2"
      shift 2
      ;;
    --skip-dedupe)
      SKIP_DEDUPE=1
      shift
      ;;
    --*)
      echo "atlas-backup: unknown argument: $1" >&2
      exit 2
      ;;
    *)
      DATE="$1"
      shift
      ;;
  esac
done

DB="$HOME/atlas/turso-atlas/data/atlas.db"
ROOT="$HOME/backups/atlas"
OUTDIR="$ROOT/$DATE"
LOG="$ROOT/backup-log.md"
R2_BUCKET="supabase-backups"
R2_PREFIX="atlas"
DRIVE_DIR="Cloud/cold-store/db-backups/local-atlas"
KEEP_LAPTOP_R2=7
KEEP_DRIVE=4

log_line() { printf '%s | %s\n' "$DATE" "$1" >>"$LOG"; }
verdict() { echo "ATLAS-BACKUP $DATE $1"; }

# One EXIT handler for staging cleanup (the pm-dashboard-backup #1010
# lesson: a second trap would replace, not stack). STAGE is empty until
# the staging dir exists.
STAGE=""
on_exit() {
  if [ -n "$STAGE" ]; then rm -rf "$STAGE"; fi
}
trap on_exit EXIT

early_fail() {
  verdict "FAILED early: $1"
  log_line "FAILED early: $1" 2>/dev/null || true
  exit 1
}

# --- DATE validation (pm-dashboard-backup #989 pattern) -------------------
if [[ ! "$DATE" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] \
  || [ "$(date -d "$DATE" +%F 2>/dev/null)" != "$DATE" ]; then
  echo "atlas-backup: invalid DATE '$DATE' - expected a real calendar date as YYYY-MM-DD" >&2
  exit 1
fi

mkdir -p "$ROOT"

# --- single instance -------------------------------------------------------
exec 9>"$ROOT/.atlas-backup.lock"
if ! flock -n 9; then
  echo "atlas-backup: SKIP another atlas-backup run holds the lock"
  exit 0
fi

# --- tools -----------------------------------------------------------------
# rclone/bws may live outside the service PATH on this host (the
# pm-dashboard-backup resolutions, kept): rclone from the newest
# nix-store build, bws from ~/tools/bin.
if ! command -v rclone >/dev/null 2>&1; then
  RCLONE_BIN="$(ls -d /nix/store/*-rclone-[0-9]*/bin/rclone 2>/dev/null | sort -V | tail -1)"
  [ -n "$RCLONE_BIN" ] && [ -x "$RCLONE_BIN" ] || early_fail "rclone not found in PATH or nix store"
  mkdir -p "$ROOT/.bin"
  ln -sf "$RCLONE_BIN" "$ROOT/.bin/rclone"
  export PATH="$ROOT/.bin:$PATH"
fi
if ! command -v bws >/dev/null 2>&1 && [ -x "$HOME/tools/bin/bws" ]; then
  export PATH="$HOME/tools/bin:$PATH"
fi
command -v bws >/dev/null 2>&1 || early_fail "bws not in PATH"
command -v sqlite3 >/dev/null 2>&1 || early_fail "sqlite3 not in PATH"
command -v python3 >/dev/null 2>&1 || early_fail "python3 not in PATH"
[ -f "$DB" ] || early_fail "canonical atlas.db not found at $DB"

# --- credentials from Bitwarden (names only in code/logs) ------------------
if [ -z "${BWS_ACCESS_TOKEN:-}" ] && [ -r "$HOME/.config/bws/access-token" ]; then
  BWS_ACCESS_TOKEN="$(cat "$HOME/.config/bws/access-token")"
  export BWS_ACCESS_TOKEN
fi
BWS_LIST="$(bws secret list 2>/dev/null)" || early_fail "bws secret list failed"
bws_get() {
  printf '%s' "$BWS_LIST" | python3 -c 'import json,sys
k = sys.argv[1]
for s in json.load(sys.stdin):
    if s.get("key") == k:
        print(s.get("value", "")); break' "$1"
}
R2_ACCESS_KEY_ID="$(bws_get cloudflare.r2-backups-access-key-id)"
R2_SECRET_ACCESS_KEY="$(bws_get cloudflare.r2-backups-secret-access-key)"
R2_ENDPOINT="$(bws_get cloudflare.r2-endpoint)"
[ -n "$R2_ACCESS_KEY_ID" ] && [ -n "$R2_SECRET_ACCESS_KEY" ] && [ -n "$R2_ENDPOINT" ] \
  || early_fail "R2 credentials incomplete in Bitwarden"
case "$R2_ENDPOINT" in
  http://*|https://*) ;;
  *) R2_ENDPOINT="https://$R2_ENDPOINT" ;;
esac
R2_ENDPOINT="${R2_ENDPOINT%/}"
# An env-configured rclone remote for R2 (no rclone.conf edit needed).
export RCLONE_CONFIG_R2BAK_TYPE="s3"
export RCLONE_CONFIG_R2BAK_PROVIDER="Cloudflare"
export RCLONE_CONFIG_R2BAK_ACCESS_KEY_ID="$R2_ACCESS_KEY_ID"
export RCLONE_CONFIG_R2BAK_SECRET_ACCESS_KEY="$R2_SECRET_ACCESS_KEY"
export RCLONE_CONFIG_R2BAK_ENDPOINT="$R2_ENDPOINT"
export RCLONE_CONFIG_R2BAK_REGION="auto"
# The r2-backups token cannot HeadBucket (403), so rclone must skip its
# bucket check (the standing fleet rule for this remote).
export RCLONE_CONFIG_R2BAK_NO_CHECK_BUCKET="true"

# --- snapshot + probe -------------------------------------------------------
FREE="$(python3 -c 'import shutil,sys; print(shutil.disk_usage(sys.argv[1]).free)' "$HOME/atlas/turso-atlas/data")"
[ "$FREE" -ge $((5 * 1024 * 1024 * 1024)) ] \
  || early_fail "only $FREE bytes free in atlas data dir"

STAGE="$HOME/atlas/turso-atlas/data/.backup-staging/$DATE"
rm -rf "$STAGE"
mkdir -p "$STAGE"
SNAP="$STAGE/atlas-$DATE.db"
sqlite3 "$DB" ".backup '$SNAP'" || early_fail "sqlite3 .backup failed"

GZ="$STAGE/atlas-$DATE.db.gz"
MANIFEST="$STAGE/atlas-$DATE.manifest.json"
# Integrity check + row counts on the snapshot, deterministic gzip,
# hashes, and the manifest — one python step (the old runner's probe,
# unchanged in substance).
PROBE="$(python3 - "$SNAP" "$GZ" "$MANIFEST" "$DATE" "$DB" <<'PYEOF'
import gzip, hashlib, json, os, shutil, sqlite3, sys
from datetime import datetime, timezone

snap, gz_path, manifest_path, date, db_path = sys.argv[1:6]
con = sqlite3.connect(f"file:{snap}?mode=ro", uri=True)
integ = con.execute("PRAGMA integrity_check").fetchone()[0]
if integ != "ok":
    print(f"snapshot integrity_check: {integ}", file=sys.stderr)
    sys.exit(1)
counts = {}
for t in ("sessions", "messages", "chunks"):
    try:
        counts[t] = con.execute(f"SELECT count(*) FROM {t}").fetchone()[0]
    except Exception:
        counts[t] = None
con.close()
with open(snap, "rb") as fin, open(gz_path, "wb") as raw:
    with gzip.GzipFile(filename="", mode="wb", fileobj=raw,
                       compresslevel=6, mtime=0) as fout:
        shutil.copyfileobj(fin, fout, 1 << 20)
os.remove(snap)

def digest_of(path, algo):
    h = hashlib.new(algo)
    with open(path, "rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()

digest = digest_of(gz_path, "sha256")
md5 = digest_of(gz_path, "md5")
size = os.path.getsize(gz_path)
counts_csv = ",".join(f"{k}:{v}" for k, v in counts.items())
manifest = {
    "dataset": "local-atlas-db",
    "date": date,
    "file": f"atlas-{date}.db.gz",
    "sha256": digest,
    "md5": md5,
    "size": size,
    "source": f"laptop:{db_path}",
    "source_bytes": os.path.getsize(db_path),
    "counts": counts,
    "created_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "issue": "developing-today/code#1515",
}
with open(manifest_path, "w") as fh:
    json.dump(manifest, fh, indent=2)
print(f"{digest} {size} {counts_csv}")
PYEOF
)" || early_fail "snapshot probe failed"
DIGEST="$(printf '%s' "$PROBE" | cut -d' ' -f1)"
SIZE="$(printf '%s' "$PROBE" | cut -d' ' -f2)"
COUNTS_CSV="$(printf '%s' "$PROBE" | cut -d' ' -f3)"
[ -n "$DIGEST" ] && [ -n "$SIZE" ] || early_fail "snapshot probe returned no digest"

# --- dedupe against the last filed snapshot ---------------------------------
# Only a true OK verdict in the log counts as filed (the old runner's
# last_filed rule: a PARTIAL line's leg summary must never read as filed).
LOGINFO="$(python3 - "$LOG" <<'PYEOF'
import os, sys
sha, dates = "", []
if os.path.isfile(sys.argv[1]):
    for line in open(sys.argv[1]):
        parts = [p.strip() for p in line.split("|")]
        if len(parts) >= 3 and "sha256" in parts[1] and parts[2].startswith("OK"):
            sha = parts[1].split("=", 1)[-1] if "=" in parts[1] else parts[1].split()[-1]
            dates.append(parts[0])
print(sha)
print(" ".join(dates))
PYEOF
)"
LAST_SHA="$(printf '%s\n' "$LOGINFO" | sed -n '1p')"
FILED_DATES="$(printf '%s\n' "$LOGINFO" | sed -n '2p')"

if [ "$DIGEST" = "$LAST_SHA" ] && [ "$SKIP_DEDUPE" -ne 1 ]; then
  log_line "sha256=$DIGEST | unchanged — legs skipped | size=$SIZE"
  verdict "UNCHANGED sha256=$DIGEST"
  exit 0
fi

# --- legs -------------------------------------------------------------------
# Each leg records ok or a one-line FAIL note and the run continues with
# the remaining legs (the old runner's semantics); rotation below only
# touches a destination whose leg succeeded.
short_err() { printf '%s' "$1" | head -1 | tr '|' '/' | cut -c1-200; }

LEG_LAPTOP="ok"
LEG_ERR=""
leg_laptop() {
  mkdir -p "$OUTDIR" || { LEG_ERR="mkdir $OUTDIR failed"; return 1; }
  cp "$GZ" "$OUTDIR/.atlas.tmp" || { LEG_ERR="copy to laptop staging failed"; return 1; }
  local got
  got="$(sha256sum "$OUTDIR/.atlas.tmp" | cut -d' ' -f1)"
  [ "$got" = "$DIGEST" ] || { LEG_ERR="laptop sha mismatch: $got != $DIGEST"; return 1; }
  mv "$OUTDIR/.atlas.tmp" "$OUTDIR/atlas-$DATE.db.gz" \
    || { LEG_ERR="laptop rename failed"; return 1; }
  cp "$MANIFEST" "$OUTDIR/atlas-$DATE.manifest.json" \
    || { LEG_ERR="laptop manifest copy failed"; return 1; }
}
if ! leg_laptop; then LEG_LAPTOP="FAIL: $(short_err "$LEG_ERR")"; fi

LEG_R2="ok"
LEG_ERR=""
leg_r2() {
  local dest="r2bak:$R2_BUCKET/$R2_PREFIX/$DATE/atlas-$DATE.db.gz"
  rclone copyto "$GZ" "$dest" 2>&1 || { LEG_ERR="R2 upload failed"; return 1; }
  local got
  got="$(rclone cat "$dest" 2>/dev/null | sha256sum | cut -d' ' -f1)"
  [ "$got" = "$DIGEST" ] || { LEG_ERR="R2 sha mismatch: got $got"; return 1; }
  rclone copyto "$MANIFEST" "r2bak:$R2_BUCKET/$R2_PREFIX/$DATE/atlas-$DATE.manifest.json" 2>&1 \
    || { LEG_ERR="R2 manifest upload failed"; return 1; }
}
if ! leg_r2; then LEG_R2="FAIL: $(short_err "$LEG_ERR")"; fi

LEG_DRIVE="ok"
LEG_ERR=""
leg_drive() {
  local dest="drive:$DRIVE_DIR/atlas-$DATE.db.gz"
  rclone copyto "$GZ" "$dest" 2>&1 || { LEG_ERR="Drive upload failed"; return 1; }
  local got
  got="$(rclone cat "$dest" 2>/dev/null | sha256sum | cut -d' ' -f1)"
  [ "$got" = "$DIGEST" ] || { LEG_ERR="Drive sha mismatch: got $got"; return 1; }
  rclone copyto "$MANIFEST" "drive:$DRIVE_DIR/atlas-$DATE.manifest.json" 2>&1 \
    || { LEG_ERR="Drive manifest upload failed"; return 1; }
}
if ! leg_drive; then LEG_DRIVE="FAIL: $(short_err "$LEG_ERR")"; fi

# --- rotation (only this routine's own artifacts; warnings never fail) ------
ALL_DATES="$(printf '%s\n%s\n' $FILED_DATES "$DATE" | sort -u)"
KEEP_SET="$(printf '%s\n' "$ALL_DATES" | tail -n "$KEEP_LAPTOP_R2")"

if [ "$LEG_LAPTOP" = "ok" ]; then
  for d in "$ROOT"/20??-??-??; do
    [ -d "$d" ] || continue
    day="$(basename "$d")"
    if ! printf '%s\n' "$KEEP_SET" | grep -qx "$day"; then
      rm -rf "$d" || echo "atlas-backup: WARN laptop rotation failed for $day" >&2
    fi
  done
fi

if [ "$LEG_R2" = "ok" ]; then
  if subs="$(rclone lsf --dirs-only "r2bak:$R2_BUCKET/$R2_PREFIX" 2>/dev/null)"; then
    for day in $subs; do
      day="${day%/}"
      if [[ "$day" =~ ^20[0-9]{2}-[0-9]{2}-[0-9]{2}$ ]] \
        && ! printf '%s\n' "$KEEP_SET" | grep -qx "$day"; then
        rclone purge "r2bak:$R2_BUCKET/$R2_PREFIX/$day" >/dev/null 2>&1 \
          || echo "atlas-backup: WARN R2 rotation failed for $day" >&2
      fi
    done
  else
    echo "atlas-backup: WARN R2 rotation listing failed" >&2
  fi
fi

if [ "$LEG_DRIVE" = "ok" ]; then
  if files="$(rclone lsf "drive:$DRIVE_DIR" 2>/dev/null)"; then
    keep_names="$(printf '%s\n' "$files" | grep -E '^atlas-20[0-9]{2}-[0-9]{2}-[0-9]{2}\.db\.gz$' \
      | sort -r | head -n "$KEEP_DRIVE")"
    for name in $files; do
      case "$name" in
        atlas-*.db.gz)
          if ! printf '%s\n' "$keep_names" | grep -qx "$name"; then
            rclone deletefile "drive:$DRIVE_DIR/$name" >/dev/null 2>&1 \
              || echo "atlas-backup: WARN Drive rotation failed for $name" >&2
            rclone deletefile "drive:$DRIVE_DIR/${name%.db.gz}.manifest.json" >/dev/null 2>&1 \
              || true
          fi
          ;;
      esac
    done
  else
    echo "atlas-backup: WARN Drive rotation listing failed" >&2
  fi
fi

# --- verdict ------------------------------------------------------------------
LEG_STR="laptop=$LEG_LAPTOP r2=$LEG_R2 drive=$LEG_DRIVE"
FAILED=""
[ "$LEG_LAPTOP" = "ok" ] || FAILED="laptop"
[ "$LEG_R2" = "ok" ] || FAILED="${FAILED:+$FAILED,}r2"
[ "$LEG_DRIVE" = "ok" ] || FAILED="${FAILED:+$FAILED,}drive"
if [ -n "$FAILED" ]; then
  log_line "sha256=$DIGEST | PARTIAL $LEG_STR | size=$SIZE"
  verdict "FAILED legs=$FAILED sha256=$DIGEST size=$SIZE $LEG_STR counts=$COUNTS_CSV"
  exit 1
fi
log_line "sha256=$DIGEST | ok laptop/r2/drive | size=$SIZE | counts $COUNTS_CSV"
verdict "OK sha256=$DIGEST size=$SIZE laptop=ok r2=ok drive=ok counts=$COUNTS_CSV"
exit 0
