#!/usr/bin/env bash
# dashboard-backup.sh — daily backup of the PM dashboard / orchestration data set.
# Builds ~/backups/dashboard/<YYYY-MM-DD>/dashboard-<YYYY-MM-DD>.tar.zst containing
# the aggregator data files, quota snapshots, the meta-quota export, and the user
# journals of pm-aggregator / quota-gatherer / meta-quota-export (window: since
# the previous successful run, never narrower than 24h), plus manifest.json with
# per-file sha256 + sizes. The tarball is verified against the manifest before
# the script reports success. Idempotent per day: re-running replaces that day's
# archive. Prunes day directories older than 30 days. Runs unattended via the
# dashboard-backup systemd user timer.
set -euo pipefail

DATE="${1:-$(date +%F)}"
CREATED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
AGG="$HOME/.local/state/pm-aggregator"
META="$HOME/atlas/meta-quota-export"
ROOT="$HOME/backups/dashboard"
OUTDIR="$ROOT/$DATE"
ARCHIVE="$OUTDIR/dashboard-$DATE.tar.zst"
MARKER="$ROOT/.last-run-epoch"

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
mkdir -p "$STAGE/aggregator" "$STAGE/meta-quota-export" "$STAGE/journals" "$OUTDIR"

# --- data set (explicit list only; the aggregator .env is deliberately NOT
# --- included — no config/secret files belong in this archive) ---
for f in actions.json comments.json fleet-feed.json quota-current.json \
  registry.json quota-gatherer-state.json quota-gatherer-heartbeat.json \
  events.json views.json coverage-snapshot.json aa-last-good.json; do
  if [ -f "$AGG/$f" ]; then
    cp -a "$AGG/$f" "$STAGE/aggregator/$f"
  else echo "dashboard-backup: WARN missing $AGG/$f" >&2; fi
done
if [ -d "$AGG/snapshots" ]; then cp -a "$AGG/snapshots" "$STAGE/aggregator/snapshots"; fi
if [ -d "$AGG/charge-inputs" ]; then cp -a "$AGG/charge-inputs" "$STAGE/aggregator/charge-inputs"; fi
for f in latest.json history.jsonl; do
  if [ -f "$META/$f" ]; then
    cp -a "$META/$f" "$STAGE/meta-quota-export/$f"
  else echo "dashboard-backup: WARN missing $META/$f" >&2; fi
done

# --- journals: window start = min(last successful run, 24h ago) ---
NOW_EPOCH="$(date +%s)"
LAST=0
if [ -f "$MARKER" ]; then LAST="$(cat "$MARKER" 2>/dev/null || echo 0)"; fi
case "$LAST" in *[!0-9]* | "") LAST=0 ;; esac
W=$((NOW_EPOCH - 86400))
if [ "$LAST" -gt 0 ] && [ "$LAST" -lt "$W" ]; then W="$LAST"; fi
SINCE_STR="$(date -d "@$W" '+%F %T')"
for svc in pm-aggregator quota-gatherer meta-quota-export; do
  journalctl --user -u "$svc.service" --since "$SINCE_STR" --no-pager -o short-iso \
    >"$STAGE/journals/$svc.log" 2>/dev/null ||
    echo "(journal unavailable for $svc)" >"$STAGE/journals/$svc.log"
done

# --- manifest ---
python3 - "$STAGE" "$DATE" "$CREATED_AT" <<'PYEOF'
import hashlib, json, os, sys
stage, date, created = sys.argv[1], sys.argv[2], sys.argv[3]
files = []
for root, _dirs, names in os.walk(stage):
    for n in sorted(names):
        p = os.path.join(root, n)
        rel = os.path.relpath(p, stage)
        if rel == "manifest.json":
            continue
        h = hashlib.sha256()
        with open(p, "rb") as fh:
            for chunk in iter(lambda: fh.read(1 << 20), b""):
                h.update(chunk)
        files.append({"path": rel, "sha256": h.hexdigest(), "size": os.path.getsize(p)})
files.sort(key=lambda f: f["path"])
with open(os.path.join(stage, "manifest.json"), "w") as fh:
    json.dump({"archive": f"dashboard-{date}", "created_at": created,
               "host": os.uname().nodename, "scope": "laptop",
               "files": files}, fh, indent=2)
print(f"dashboard-backup: manifest covers {len(files)} files")
PYEOF

# --- tarball (atomic replace) + verify against manifest ---
TMPARC="$OUTDIR/.dashboard-$DATE.tar.zst.tmp"
tar -I zstd -cf "$TMPARC" -C "$STAGE" .
mv "$TMPARC" "$ARCHIVE"
VDIR="$(mktemp -d)"
tar -I zstd -xf "$ARCHIVE" -C "$VDIR"
if ! (
  cd "$VDIR" && python3 - <<'PYEOF'
import hashlib, json, os, sys
m = json.load(open("manifest.json"))
bad = 0
for f in m["files"]:
    p = f["path"]
    if not os.path.isfile(p):
        print("MISSING", p); bad += 1; continue
    if hashlib.sha256(open(p, "rb").read()).hexdigest() != f["sha256"] \
       or os.path.getsize(p) != f["size"]:
        print("MISMATCH", p); bad += 1
sys.exit(1 if bad else 0)
PYEOF
); then
  echo "dashboard-backup: VERIFY FAILED for $ARCHIVE" >&2
  rm -rf "$VDIR"
  exit 1
fi
rm -rf "$VDIR"

echo "$NOW_EPOCH" >"$MARKER"
echo "dashboard-backup: OK $ARCHIVE ($(stat -c%s "$ARCHIVE") bytes, sha256 $(sha256sum "$ARCHIVE" | cut -d' ' -f1))"

# --- prune day dirs older than 30 days (only this script's own date dirs) ---
CUTOFF="$(date -d '30 days ago' +%F)"
for d in "$ROOT"/20??-??-??; do
  [ -d "$d" ] || continue
  day="$(basename "$d")"
  if [[ $day < $CUTOFF ]]; then
    rm -rf "$d"
    echo "dashboard-backup: pruned $day"
  fi
done
