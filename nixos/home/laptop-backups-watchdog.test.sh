#!/usr/bin/env bash
# Tests for laptop-backups-watchdog.sh (#1026): the watchdog must exit
# 0 only when pm-dashboard-backup's run record is fresh + complete,
# both backup timers fired within the threshold, and the newest fresh
# local pm-stack day dir has its named archive; every violation exits
# non-zero naming the failed check. Drives the REAL script with a
# stub `systemctl` (per-timer LastTriggerUSec values from files) and
# generated record files whose timestamps are relative to now.
set -uo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ -f "$SELF_DIR/laptop-backups-watchdog.sh" ]]; then
  SCRIPT="$SELF_DIR/laptop-backups-watchdog.sh"
else
  SCRIPT="$SELF_DIR/modules/laptop-backups-watchdog.sh"
fi

TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT
failures=0
fail() {
  echo "FAIL: $1" >&2
  failures=$((failures + 1))
}

BIN="$TMP_ROOT/bin"
TRIG="$TMP_ROOT/triggers"
HOME_DIR="$TMP_ROOT/home"
mkdir -p "$BIN" "$TRIG" "$HOME_DIR"
{
  printf '#!%s\n' "$BASH"
  cat <<'STUB'
unit="${@: -1}"
f="$WATCHDOG_STUB_TRIGGERS/$unit"
[[ -f "$f" ]] || exit 0
cat "$f"
STUB
} >"$BIN/systemctl"
chmod +x "$BIN/systemctl"

iso_ago() { date -u -d "$1" +%Y-%m-%dT%H:%M:%SZ; }
trig_ago() { date -u -d "$1" '+%a %Y-%m-%d %H:%M:%S UTC'; }

set_triggers() { # $1 pm trigger spec, $2 repo-archive trigger spec, $3 state trigger spec
  trig_ago "$1" >"$TRIG/pm-dashboard-backup.timer"
  trig_ago "$2" >"$TRIG/repo-archive-backup.timer"
  trig_ago "${3:-1 hour ago}" >"$TRIG/laptop-state-backup.timer"
}
MARKER="$TMP_ROOT/state-marker"
write_marker() { # $1 age spec for the laptop-state success marker
  date -d "$1" +%s >"$MARKER"
}
write_record() { # $1 result, $2 finished spec
  python3 - "$1" "$(iso_ago "$2")" <<'PYEOF'
import json, sys
rec = {"job": "pm-dashboard-backup", "date": "2026-10-28",
       "started_at": sys.argv[2], "finished_at": sys.argv[2],
       "result": sys.argv[1], "exit_code": 0 if sys.argv[1] == "complete" else 1,
       "failing_step": None if sys.argv[1] == "complete" else "r2-upload",
       "archive_sha256": "abc123",
       "destinations": {"local": "ok", "r2": "ok", "drive": "ok"}}
with open(__import__("os").environ["RECORD_OUT"], "w") as fh:
    json.dump(rec, fh)
PYEOF
}

# stub bws: `bws secret list` prints dummy entries for the three R2 keys
# (values are never used - the rclone stub does not dial out)
{
  printf '#!%s\n' "$BASH"
  cat <<'STUB'
printf '%s' '[{"key":"cloudflare.r2-backups-access-key-id","value":"stub-id"},{"key":"cloudflare.r2-backups-secret-access-key","value":"stub-secret"},{"key":"cloudflare.r2-endpoint","value":"stub.r2.example"}]'
STUB
} >"$BIN/bws"
chmod +x "$BIN/bws"

# stub rclone: `rclone lsf <path>` prints the fixture listing in
# $WATCHDOG_STUB_R2_LISTING (an absent/empty file = empty offsite)
{
  printf '#!%s\n' "$BASH"
  cat <<'STUB'
[[ -n "${WATCHDOG_STUB_R2_LISTING:-}" && -f "$WATCHDOG_STUB_R2_LISTING" ]] && cat "$WATCHDOG_STUB_R2_LISTING"
exit 0
STUB
} >"$BIN/rclone"
chmod +x "$BIN/rclone"

set_cs_trigger() { trig_ago "$1" >"$TRIG/cold-store-nightly.timer"; }
set_cs_result() { printf '%s\n' "$1" >"$TRIG/cold-store-nightly.service"; }
write_manifest() { # $1 latest backup-upload date spec, or "none"
  mkdir -p "$HOME_DIR/.config/cold-store"
  HOME_DIR="$HOME_DIR" python3 - "$1" <<'PYEOF'
import os, sqlite3, subprocess, sys
path = os.path.join(os.environ["HOME_DIR"], ".config", "cold-store", "manifest.sqlite")
if os.path.exists(path):
    os.remove(path)
con = sqlite3.connect(path)
con.execute("create table transfer_ledger (id integer primary key autoincrement, ts text not null default (datetime('now')), direction text, bytes integer, kind text, ref text)")
if sys.argv[1] != "none":
    ts = subprocess.check_output(["date", "-u", "-d", sys.argv[1], "+%Y-%m-%d %H:%M:%S"]).decode().strip()
    con.execute("insert into transfer_ledger (ts, direction, bytes, kind, ref) values (?, 'upload', 100, 'backup-upload', 'fleet-queue')", (ts,))
con.commit()
PYEOF
}

RECORD="$TMP_ROOT/record.json"
run_watchdog() {
  HOME="$HOME_DIR" LAPTOP_BACKUPS_RECORD="$RECORD" WATCHDOG_STUB_TRIGGERS="$TRIG" \
    LAPTOP_STATE_MARKER="$MARKER" \
    PATH="$BIN:$PATH" bash "$SCRIPT" 26 >"$TMP_ROOT/out.log" 2>&1
  RC=$?
}
run_watchdog_jobs() { # $1 jobs list
  HOME="$HOME_DIR" LAPTOP_BACKUPS_RECORD="$RECORD" WATCHDOG_STUB_TRIGGERS="$TRIG" \
    LAPTOP_STATE_MARKER="$MARKER" \
    PATH="$BIN:$PATH" bash "$SCRIPT" 26 "$1" >"$TMP_ROOT/out.log" 2>&1
  RC=$?
}

expect_ok() {
  if [[ $RC -eq 0 ]]; then
    echo "ok: $1: watchdog exited 0"
  else fail "$1: expected exit 0, got $RC; log: $(cat "$TMP_ROOT/out.log")"; fi
}
expect_named_failure() { # $1 scenario, $2 needle
  if [[ $RC -ne 0 ]]; then
    echo "ok: $1: watchdog exited non-zero ($RC)"
  else fail "$1: expected non-zero exit, got 0"; fi
  if grep -q "$2" "$TMP_ROOT/out.log"; then
    echo "ok: $1: output names $2"
  else fail "$1: output does not name $2: $(cat "$TMP_ROOT/out.log")"; fi
}

# fresh complete record + recently fired timers -> 0
RECORD_OUT="$RECORD" write_record complete "1 hour ago"
set_triggers "2 hours ago" "1 hour ago"
run_watchdog
expect_ok fresh

# stale record -> non-zero naming pm-dashboard-backup
RECORD_OUT="$RECORD" write_record complete "30 hours ago"
set_triggers "2 hours ago" "1 hour ago"
run_watchdog
expect_named_failure stale-record pm-dashboard-backup

# failed record (however fresh) -> non-zero naming pm-dashboard-backup
RECORD_OUT="$RECORD" write_record failed "10 minutes ago"
run_watchdog
expect_named_failure failed-record pm-dashboard-backup

# stale repo-archive timer -> non-zero naming it
RECORD_OUT="$RECORD" write_record complete "1 hour ago"
set_triggers "1 hour ago" "40 hours ago"
run_watchdog
expect_named_failure stale-timer repo-archive-backup

# missing record -> non-zero naming pm-dashboard-backup
rm -f "$RECORD"
set_triggers "1 hour ago" "1 hour ago"
run_watchdog
expect_named_failure missing-record pm-dashboard-backup

# jobs subset: dashboard only -> a stale repo-archive timer is not a
# violation (the job is disabled, so its timer does not exist)
RECORD_OUT="$RECORD" write_record complete "1 hour ago"
set_triggers "1 hour ago" "40 hours ago"
run_watchdog_jobs dashboard
expect_ok jobs-dashboard-only

# jobs subset: repo-archive only -> a missing dashboard record is not
# a violation (the job is disabled, so no record is ever written)
rm -f "$RECORD"
set_triggers "40 hours ago" "1 hour ago"
run_watchdog_jobs repo-archive
expect_ok jobs-repo-only

# jobs subset still names an enabled job's violation
RECORD_OUT="$RECORD" write_record complete "1 hour ago"
set_triggers "1 hour ago" "40 hours ago"
run_watchdog_jobs repo-archive
expect_named_failure jobs-repo-stale repo-archive-backup

# future record (beyond skew tolerance) -> non-zero naming pm-dashboard-backup:
# a negative age must not pass as fresh after a backward clock correction
RECORD_OUT="$RECORD" write_record complete "2 hours"
set_triggers "1 hour ago" "1 hour ago"
run_watchdog
expect_named_failure future-record pm-dashboard-backup

# future timer trigger (beyond skew tolerance) -> non-zero naming the job
RECORD_OUT="$RECORD" write_record complete "1 hour ago"
set_triggers "1 hour ago" "2 hours"
run_watchdog
expect_named_failure future-timer repo-archive-backup

# offsite check: record date is 2026-10-28 (write_record pins it).
R2_LISTING="$TMP_ROOT/r2-listing.txt"
run_watchdog_offsite() {
  HOME="$HOME_DIR" LAPTOP_BACKUPS_RECORD="$RECORD" WATCHDOG_STUB_TRIGGERS="$TRIG" \
    WATCHDOG_STUB_R2_LISTING="$R2_LISTING" LAPTOP_BACKUPS_OFFSITE_CHECK=1 \
    PATH="$BIN:$PATH" bash "$SCRIPT" 26 >"$TMP_ROOT/out.log" 2>&1
  RC=$?
}

# offsite present for the record date -> 0
RECORD_OUT="$RECORD" write_record complete "1 hour ago"
set_triggers "1 hour ago" "1 hour ago"
printf 'dashboard-2026-10-28.tar.zst\n' >"$R2_LISTING"
run_watchdog_offsite
expect_ok offsite-present

# offsite absent (planted) while the record says complete -> non-zero
# naming the R2 absence: this is the #3914 failure shape
printf '' >"$R2_LISTING"
run_watchdog_offsite
expect_named_failure offsite-absent R2

# offsite check disabled -> the same absence is not a violation
# (pre-#3914 behavior for jobs/hosts without the opt-in)
run_watchdog
expect_ok offsite-check-disabled

# A missing pm-stack root is a supported host configuration; skip it.
run_watchdog
expect_ok pm-stack-root-absent

# A fresh date dir with no archive is the failure shape: mkdir happened,
# but the dump never produced its named archive.
# Pin every derived day name to one captured epoch so a midnight
# rollover mid-run cannot make the scenarios disagree about "today".
TEST_EPOCH="$(date +%s)"
TODAY="$(date -d "@$TEST_EPOCH" +%F)"
PM_STACK_ROOT="$HOME_DIR/backups/pm-stack"
YESTERDAY="$(date -d "@$((TEST_EPOCH - 86400))" +%F)"
mkdir -p "$PM_STACK_ROOT/$YESTERDAY"
touch "$PM_STACK_ROOT/$YESTERDAY/pm-stack-$YESTERDAY.tar.zst"
mkdir -p "$PM_STACK_ROOT/$TODAY"
run_watchdog
expect_named_failure pm-stack-empty-day pm-stack-backup
if grep -q "$TODAY" "$TMP_ROOT/out.log"; then
  echo "ok: pm-stack-empty-day: output names date $TODAY"
else fail "pm-stack-empty-day: output does not name date $TODAY: $(cat "$TMP_ROOT/out.log")"; fi
if grep -q "pm-stack-$TODAY.tar.zst" "$TMP_ROOT/out.log"; then
  echo "ok: pm-stack-empty-day: output names missing archive"
else fail "pm-stack-empty-day: output does not name pm-stack-$TODAY.tar.zst: $(cat "$TMP_ROOT/out.log")"; fi

# The same fresh day is healthy once its expected archive is present.
touch "$PM_STACK_ROOT/$TODAY/pm-stack-$TODAY.tar.zst"
run_watchdog
expect_ok pm-stack-archive-present

# Old empty day dirs do not create a stale-day false alarm.
rm -f "$PM_STACK_ROOT/$TODAY/pm-stack-$TODAY.tar.zst"
rmdir "$PM_STACK_ROOT/$TODAY"
rm -f "$PM_STACK_ROOT/$YESTERDAY/pm-stack-$YESTERDAY.tar.zst"
rmdir "$PM_STACK_ROOT/$YESTERDAY"
STALE_DAY="$(date -d "@$((TEST_EPOCH - 172800))" +%F)"
mkdir -p "$PM_STACK_ROOT/$STALE_DAY"
touch -d "@$((TEST_EPOCH - 172800))" "$PM_STACK_ROOT/$STALE_DAY"
run_watchdog
expect_ok pm-stack-stale-day

# --- laptop-state coverage (code#10294) --------------------------------------
# state job enabled, fresh marker + fresh timer -> 0
RECORD_OUT="$RECORD" write_record complete "1 hour ago"
set_triggers "1 hour ago" "1 hour ago" "1 hour ago"
write_marker "2 hours ago"
run_watchdog_jobs dashboard,repo-archive,state
expect_ok state-fresh

# stale marker -> non-zero naming laptop-state-backup (the 12-day stall shape)
write_marker "40 hours ago"
run_watchdog_jobs dashboard,repo-archive,state
expect_named_failure state-stale-marker laptop-state-backup

# missing marker -> non-zero naming laptop-state-backup
rm -f "$MARKER"
run_watchdog_jobs dashboard,repo-archive,state
expect_named_failure state-missing-marker laptop-state-backup

# future marker (beyond skew tolerance) -> non-zero naming laptop-state-backup
write_marker "2 hours"
run_watchdog_jobs dashboard,repo-archive,state
expect_named_failure state-future-marker laptop-state-backup

# stale state timer -> non-zero naming laptop-state-backup
write_marker "1 hour ago"
set_triggers "1 hour ago" "1 hour ago" "40 hours ago"
run_watchdog_jobs dashboard,repo-archive,state
expect_named_failure state-stale-timer laptop-state-backup

# state job not in the jobs list -> its stale marker is not a violation
write_marker "40 hours ago"
set_triggers "1 hour ago" "1 hour ago"
run_watchdog_jobs dashboard,repo-archive
expect_ok state-not-enabled

# cold-store job: fresh trigger + success result + fresh ledger -> 0
set_cs_trigger "1 hour ago"
set_cs_result success
write_manifest "1 hour ago"
run_watchdog_jobs cold-store
expect_ok cs-fresh

# cold-store: stale ledger (nothing filed recently) -> named failure
write_manifest "40 hours ago"
run_watchdog_jobs cold-store
expect_named_failure cs-stale-ledger cold-store-nightly

# cold-store: failed last result, however fresh the ledger -> named
# failure (the 2026-11-12 shape: fired, started, killed mid-export)
set_cs_result failure
write_manifest "10 minutes ago"
run_watchdog_jobs cold-store
expect_named_failure cs-failed-result cold-store-nightly

# cold-store: missing manifest -> named failure
set_cs_result success
rm -f "$HOME_DIR/.config/cold-store/manifest.sqlite"
run_watchdog_jobs cold-store
expect_named_failure cs-missing-manifest cold-store-nightly

# cold-store: stale timer -> named failure
set_cs_trigger "40 hours ago"
write_manifest "1 hour ago"
run_watchdog_jobs cold-store
expect_named_failure cs-stale-timer cold-store-nightly

# cold-store is not in the default job set: its violations stay
# silent for hosts/jobs that did not opt in
RECORD_OUT="$RECORD" write_record complete "1 hour ago"
set_triggers "1 hour ago" "1 hour ago"
set_cs_result failure
run_watchdog
expect_ok cs-not-in-default-jobs

if ((failures > 0)); then
  echo "laptop-backups-watchdog tests: $failures failure(s)" >&2
  exit 1
fi
echo "laptop-backups-watchdog tests: all passed"
