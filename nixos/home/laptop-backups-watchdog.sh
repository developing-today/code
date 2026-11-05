# laptop-backups-watchdog — freshness check for the laptop backups
# (#1026, design #1024 phase 3; the #976 acceptance "a freshness check
# exists so a stopped backup gets named").
#
# Read-only: it changes nothing and attempts no repair. It exits 0
# only when BOTH hold, and otherwise exits 1 with one line per
# violation naming the job/timer:
#   (a) pm-dashboard-backup's run record ($HOME/backups/dashboard/
#       .last-run.json, written by modules/pm-dashboard-backup.sh on
#       every exit path) says result=complete and finished within
#       the threshold — a failed record is a violation however fresh,
#       so a nightly failure can never masquerade as freshness;
#   (b) both backup timers (pm-dashboard-backup.timer and
#       repo-archive-backup.timer) last triggered within the same
#       threshold, per `systemctl --user show -p LastTriggerUSec`;
#   (c) when LAPTOP_BACKUPS_OFFSITE_CHECK=1 (the module sets it for
#       the service; #3914), the dashboard backup's own date - the
#       run record's `date` - is present on the R2 offsite:
#       r2bak:supabase-backups/dashboard/<date>/dashboard-<date>.tar.zst
#       exists. A complete run record whose upload silently landed
#       nothing (2026-10-29..2026-11-01, #3914) passes (a) and (b);
#       only an object listing sees the absence. Credentials come
#       from Bitwarden exactly as in pm-dashboard-backup.sh, and
#       rclone/bws resolve via PATH so tests can stub both.
# repo-archive-backup is covered via its timer's last-trigger time
# only: its script and restic credentials live outside this repo, so
# no run record is required of it (stated in laptop-backups.nix too).
#
# Usage: laptop-backups-watchdog [max-age-hours] [jobs]
#   max-age-hours   (default 26, a little over one daily period; the
#                   module option services.laptop-backups.watchdog
#                   .maxAgeHours feeds this argument).
#   jobs            comma-separated subset of dashboard,repo-archive
#                   (default both). The module passes only the jobs
#                   whose enable flag is on: a disabled job has no
#                   record and no timer, so requiring them would fail
#                   the watchdog forever in a supported configuration.
# Test hooks: LAPTOP_BACKUPS_RECORD overrides the record path and
# `systemctl` is resolved via PATH, so the suite can stub both.
MAX_AGE_HOURS="${1:-26}"
JOBS="${2:-dashboard,repo-archive}"
want_job() { case ",$JOBS," in *",$1,"*) return 0 ;; *) return 1 ;; esac }
RECORD="${LAPTOP_BACKUPS_RECORD:-$HOME/backups/dashboard/.last-run.json}"
NOW_EPOCH="$(date +%s)"
MAX_AGE_SEC=$((MAX_AGE_HOURS * 3600))
# A timestamp further in the future than this tolerance cannot be a
# fresh run: after a backward clock correction it would read as a
# negative age and pass, masking a stopped backup until real time
# caught up. Small skew (NTP steps, VM resume) is tolerated.
SKEW_TOLERANCE_SEC=300
violations=0

complain() {
  echo "laptop-backups-watchdog: FAIL $*" >&2
  violations=$((violations + 1))
}

# --- (a) the dashboard run record -------------------------------------------
if want_job dashboard; then
  if [ ! -f "$RECORD" ]; then
    complain "pm-dashboard-backup: no run record at $RECORD"
  else
    verdict="$(
      RECORD="$RECORD" NOW_EPOCH="$NOW_EPOCH" MAX_AGE_SEC="$MAX_AGE_SEC" SKEW_TOLERANCE_SEC="$SKEW_TOLERANCE_SEC" \
        python3 - <<'PYEOF' 2>/dev/null
import json, os
from datetime import datetime, timezone
try:
    rec = json.load(open(os.environ["RECORD"]))
    finished = datetime.strptime(rec["finished_at"], "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=timezone.utc)
except Exception:
    print("unreadable")
    raise SystemExit
age = int(os.environ["NOW_EPOCH"]) - int(finished.timestamp())
if rec.get("result") != "complete":
    print(f"failed result={rec.get('result')} failing_step={rec.get('failing_step')} finished_at={rec.get('finished_at')}")
elif age < -int(os.environ["SKEW_TOLERANCE_SEC"]):
    print(f"future finished_at={rec.get('finished_at')} is {-age}s ahead of now (skew tolerance {os.environ['SKEW_TOLERANCE_SEC']}s)")
elif age > int(os.environ["MAX_AGE_SEC"]):
    print(f"stale age_hours={age // 3600} finished_at={rec.get('finished_at')}")
else:
    print("ok")
PYEOF
    )"
    case "$verdict" in
    ok) ;;
    unreadable | "") complain "pm-dashboard-backup: run record at $RECORD is unreadable" ;;
    *) complain "pm-dashboard-backup: run record $verdict (threshold ${MAX_AGE_HOURS}h)" ;;
    esac
  fi
fi

# --- (b) enabled timers' last triggers ---------------------------------------
timers=()
want_job dashboard && timers+=(pm-dashboard-backup.timer)
want_job repo-archive && timers+=(repo-archive-backup.timer)
for timer in "${timers[@]}"; do
  job="${timer%.timer}"
  raw="$(systemctl --user show -p LastTriggerUSec --value "$timer" 2>/dev/null || true)"
  if [ -z "$raw" ]; then
    complain "$job: timer $timer has never triggered"
    continue
  fi
  trig_epoch="$(date -d "$raw" +%s 2>/dev/null || echo "")"
  if [ -z "$trig_epoch" ]; then
    complain "$job: timer $timer last trigger unparseable: '$raw'"
    continue
  fi
  age=$((NOW_EPOCH - trig_epoch))
  if [ "$age" -lt "$((-SKEW_TOLERANCE_SEC))" ]; then
    complain "$job: timer $timer last trigger is $((-age))s in the future (skew tolerance ${SKEW_TOLERANCE_SEC}s)"
  elif [ "$age" -gt "$MAX_AGE_SEC" ]; then
    complain "$job: timer $timer last triggered $((age / 3600))h ago (threshold ${MAX_AGE_HOURS}h)"
  fi
done

# --- (c) offsite presence on R2 ----------------------------------------------
if want_job dashboard && [ "${LAPTOP_BACKUPS_OFFSITE_CHECK:-0}" = "1" ]; then
  offsite_date="$(
    RECORD="$RECORD" python3 - <<'PYEOF' 2>/dev/null
import json, os
try:
    print(json.load(open(os.environ["RECORD"]))["date"])
except Exception:
    pass
PYEOF
  )"
  if [ -z "$offsite_date" ]; then
    : # (a) already named the missing or unreadable record
  elif ! command -v rclone >/dev/null 2>&1 || ! command -v bws >/dev/null 2>&1; then
    complain "pm-dashboard-backup: cannot verify offsite presence for $offsite_date: rclone/bws not on PATH"
  else
    if [ -z "${BWS_ACCESS_TOKEN:-}" ] && [ -f "$HOME/.config/bws/access-token" ]; then
      BWS_ACCESS_TOKEN="$(cat "$HOME/.config/bws/access-token")"
      export BWS_ACCESS_TOKEN
    fi
    BWS_LIST="$(bws secret list 2>/dev/null || true)"
    bws_get() {
      printf '%s' "$BWS_LIST" | python3 -c 'import json,sys
k = sys.argv[1]
for s in json.load(sys.stdin):
    if s.get("key") == k:
        print(s.get("value", "")); break' "$1" 2>/dev/null
    }
    R2_ACCESS_KEY_ID="$(bws_get cloudflare.r2-backups-access-key-id)"
    R2_SECRET_ACCESS_KEY="$(bws_get cloudflare.r2-backups-secret-access-key)"
    R2_ENDPOINT="$(bws_get cloudflare.r2-endpoint)"
    if [ -z "$R2_ACCESS_KEY_ID" ] || [ -z "$R2_SECRET_ACCESS_KEY" ] || [ -z "$R2_ENDPOINT" ]; then
      complain "pm-dashboard-backup: cannot verify offsite presence for $offsite_date: R2 credentials incomplete in Bitwarden"
    else
      case "$R2_ENDPOINT" in
      http://* | https://*) ;;
      *) R2_ENDPOINT="https://$R2_ENDPOINT" ;;
      esac
      R2_ENDPOINT="${R2_ENDPOINT%/}"
      export RCLONE_CONFIG_R2BAK_TYPE="s3"
      export RCLONE_CONFIG_R2BAK_PROVIDER="Cloudflare"
      export RCLONE_CONFIG_R2BAK_ACCESS_KEY_ID="$R2_ACCESS_KEY_ID"
      export RCLONE_CONFIG_R2BAK_SECRET_ACCESS_KEY="$R2_SECRET_ACCESS_KEY"
      export RCLONE_CONFIG_R2BAK_ENDPOINT="$R2_ENDPOINT"
      export RCLONE_CONFIG_R2BAK_REGION="auto"
      # The r2-backups token cannot HeadBucket (see pm-dashboard-backup.sh).
      export RCLONE_CONFIG_R2BAK_NO_CHECK_BUCKET="true"
      offsite_listing="$(rclone lsf "r2bak:supabase-backups/dashboard/$offsite_date/" 2>/dev/null || true)"
      case "$offsite_listing" in
      *"dashboard-$offsite_date.tar.zst"*) ;;
      *) complain "pm-dashboard-backup: offsite absence on R2: dashboard/$offsite_date/dashboard-$offsite_date.tar.zst not found (run record says complete)" ;;
      esac
    fi
  fi
fi

if [ "$violations" -gt 0 ]; then
  echo "laptop-backups-watchdog: $violations violation(s)" >&2
  exit 1
fi
echo "laptop-backups-watchdog: OK jobs [$JOBS] fresh within ${MAX_AGE_HOURS}h"
