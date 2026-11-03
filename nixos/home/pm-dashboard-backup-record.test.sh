#!/usr/bin/env bash
# Tests for pm-dashboard-backup.sh's run record (#1026, design #1024
# phase 3): every run leaves $HOME/backups/dashboard/.last-run.json.
# Drives the REAL script with the same stubbed bws/journalctl/rclone
# harness as pm-dashboard-backup.test.sh. Scenarios:
#   * happy path — record result=complete, archive_sha256 equals the
#     built archive's sha256, all three destinations "ok", date and
#     job named, started/finished timestamps present
#   * stubbed R2 upload failure — exit non-zero and the record still
#     written: result=failed, failing step names the R2 leg,
#     destinations local=ok / r2=failed / drive=not-attempted
set -uo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ -f "$SELF_DIR/pm-dashboard-backup.sh" ]]; then
  SCRIPT="$SELF_DIR/pm-dashboard-backup.sh"
else
  SCRIPT="$SELF_DIR/modules/pm-dashboard-backup.sh"
fi

TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT
failures=0
fail() { echo "FAIL: $1" >&2; failures=$((failures + 1)); }

BIN="$TMP_ROOT/bin"
mkdir -p "$BIN"
{ printf '#!%s\n' "$BASH"; cat <<'STUB'
if [[ "${BWS_BLOCK:-}" == "1" ]]; then
  [[ -n "${BWS_BLOCK_MARKER:-}" ]] && touch "$BWS_BLOCK_MARKER"
  sleep 60
  exit 0
fi
if [[ "${1:-} ${2:-}" == "secret list" ]]; then
  printf '%s\n' '[{"key":"cloudflare.r2-backups-access-key-id","value":"fake-access-key-id"},{"key":"cloudflare.r2-backups-secret-access-key","value":"fake-secret-access-key"},{"key":"cloudflare.r2-endpoint","value":"https://example.invalid"}]'
  exit 0
fi
echo "stub bws: unsupported args: $*" >&2
exit 1
STUB
} >"$BIN/bws"
{ printf '#!%s\n' "$BASH"; cat <<'STUB'
echo "stub journal line"
exit 0
STUB
} >"$BIN/journalctl"
{ printf '#!%s\n' "$BASH"; cat <<'STUB'
store="${RCLONE_STUB_DIR:?RCLONE_STUB_DIR unset}"
mkdir -p "$store"
key() { printf '%s' "$1" | tr '/:' '__'; }
cmd="${1:-}"
shift || true
case "$cmd" in
  copyto)
    src="$1" dest="$2"
    case "$dest" in
      r2bak:*)
        if [[ "${RCLONE_FAIL:-}" == "R2" ]]; then
          echo "stub rclone: forced R2 failure" >&2
          exit 42
        fi
        ;;
      drive:*)
        if [[ "${RCLONE_FAIL:-}" == "DRIVE" ]]; then
          echo "stub rclone: forced Drive failure" >&2
          exit 42
        fi
        ;;
    esac
    cp "$src" "$store/$(key "$dest")"
    ;;
  cat)
    f="$store/$(key "$1")"
    [[ -f "$f" ]] || exit 1
    cat "$f"
    ;;
  lsf) exit 0 ;;
  purge) exit 0 ;;
  *)
    echo "stub rclone: unsupported command: $cmd" >&2
    exit 1
    ;;
esac
STUB
} >"$BIN/rclone"
chmod +x "$BIN/bws" "$BIN/journalctl" "$BIN/rclone"

run_scenario() {
  local name="$1" fail_mode="$2" dump_mode="${3:-absent}"
  local home="$TMP_ROOT/$name/home"
  mkdir -p "$home/backups/dashboard" "$home/.local/state/pm-aggregator" \
    "$home/atlas/meta-quota-export"
  local dump_script="$home/no-such-dump-script.sh"
  if [[ "$dump_mode" == "ok" ]]; then
    dump_script="$home/dump-ok.sh"
    { printf '#!%s\n' "$BASH"
      printf '%s\n' 'mkdir -p "$HOME/backups/db-dumps/$1"'
      printf '%s\n' 'printf "{}\n" > "$HOME/backups/db-dumps/$1/dump.sql"'
    } >"$dump_script"
    chmod +x "$dump_script"
  elif [[ "$dump_mode" == "fail" ]]; then
    dump_script="$home/dump-fail.sh"
    { printf '#!%s\n' "$BASH"; printf '%s\n' 'exit 3'; } >"$dump_script"
    chmod +x "$dump_script"
  fi
  for f in actions.json comments.json fleet-feed.json quota-current.json registry.json quota-gatherer-state.json quota-gatherer-heartbeat.json; do
    printf '{}\n' >"$home/.local/state/pm-aggregator/$f"
  done
  printf '{}\n' >"$home/atlas/meta-quota-export/latest.json"
  printf '{}\n' >"$home/atlas/meta-quota-export/history.jsonl"
  HOME="$home" PATH="$BIN:$PATH" BWS_ACCESS_TOKEN="<redacted>" \
    RCLONE_FAIL="$fail_mode" RCLONE_STUB_DIR="$TMP_ROOT/$name/rclone-store" \
    PM_DB_DUMP_SCRIPT="$dump_script" \
    bash "$SCRIPT" 2026-10-28 >"$TMP_ROOT/$name.log" 2>&1
  RC=$?
  SCEN_HOME="$home"
}

check_record() { # $1 scenario, $2 python assertions file
  local rec="$SCEN_HOME/backups/dashboard/.last-run.json"
  if [[ -f "$rec" ]]; then echo "ok: $1: run record written"
  else fail "$1: no run record at $rec; log: $(cat "$TMP_ROOT/$1.log")"; return; fi
  if RECORD="$rec" HOME_DIR="$SCEN_HOME" python3 "$2"; then
    echo "ok: $1: record contents as expected"
  else
    fail "$1: record contents wrong: $(cat "$rec")"
  fi
}

cat >"$TMP_ROOT/assert-happy.py" <<'PYEOF'
import hashlib, json, os
rec = json.load(open(os.environ["RECORD"]))
arc = os.path.join(os.environ["HOME_DIR"], "backups/dashboard/2026-10-28/dashboard-2026-10-28.tar.zst")
sha = hashlib.sha256(open(arc, "rb").read()).hexdigest()
assert rec["job"] == "pm-dashboard-backup", rec
assert rec["date"] == "2026-10-28", rec
assert rec["result"] == "complete", rec
assert rec["failing_step"] is None, rec
assert rec["archive_sha256"] == sha, (rec, sha)
assert rec["destinations"] == {"local": "ok", "r2": "ok", "drive": "ok"}, rec
assert rec["started_at"] and rec["finished_at"], rec
assert rec["steps"] == [
    {"step": "validate-date", "outcome": "ok"},
    {"step": "credentials", "outcome": "ok"},
    {"step": "db-dump", "outcome": "not-applicable"},
    {"step": "local-archive", "outcome": "ok"},
    {"step": "r2-upload", "outcome": "ok"},
    {"step": "drive-upload", "outcome": "ok"},
], rec
PYEOF

cat >"$TMP_ROOT/assert-r2fail.py" <<'PYEOF'
import json, os
rec = json.load(open(os.environ["RECORD"]))
assert rec["result"] == "failed", rec
assert rec["failing_step"] == "r2-upload", rec
assert rec["destinations"]["local"] == "ok", rec
assert rec["destinations"]["r2"] == "failed", rec
assert rec["destinations"]["drive"] == "not-attempted", rec
assert rec["archive_sha256"], rec
assert rec["steps"] == [
    {"step": "validate-date", "outcome": "ok"},
    {"step": "credentials", "outcome": "ok"},
    {"step": "db-dump", "outcome": "not-applicable"},
    {"step": "local-archive", "outcome": "ok"},
    {"step": "r2-upload", "outcome": "failed"},
    {"step": "drive-upload", "outcome": "not-attempted"},
], rec
PYEOF

run_scenario happy ""
if [[ "$RC" -eq 0 ]]; then echo "ok: happy: script exited 0"
else fail "happy: expected exit 0, got $RC; log: $(cat "$TMP_ROOT/happy.log")"; fi
check_record happy "$TMP_ROOT/assert-happy.py"

run_scenario r2fail R2
if [[ "$RC" -ne 0 ]]; then echo "ok: r2fail: script exited non-zero ($RC)"
else fail "r2fail: expected non-zero exit, got 0"; fi
check_record r2fail "$TMP_ROOT/assert-r2fail.py"

# signal scenario: SIGTERM to the process group while the script is
# blocked in the credentials step must leave a FAILED record with
# exit_code 143 -- never a fresh "complete" record the watchdog would
# accept as healthy (Codex finding on PR 2989).
sig_home="$TMP_ROOT/sigterm/home"
mkdir -p "$sig_home/backups/dashboard" "$sig_home/.local/state/pm-aggregator" \
  "$sig_home/atlas/meta-quota-export"
for f in actions.json comments.json fleet-feed.json quota-current.json registry.json quota-gatherer-state.json quota-gatherer-heartbeat.json; do
  printf '{}\n' >"$sig_home/.local/state/pm-aggregator/$f"
done
printf '{}\n' >"$sig_home/atlas/meta-quota-export/latest.json"
printf '{}\n' >"$sig_home/atlas/meta-quota-export/history.jsonl"
setsid env HOME="$sig_home" PATH="$BIN:$PATH" BWS_ACCESS_TOKEN="<redacted>" \
  BWS_BLOCK=1 BWS_BLOCK_MARKER="$TMP_ROOT/sigterm.blocked" \
  RCLONE_STUB_DIR="$TMP_ROOT/sigterm/rclone-store" \
  PM_DB_DUMP_SCRIPT="$sig_home/no-such-dump-script.sh" \
  bash "$SCRIPT" 2026-10-28 >"$TMP_ROOT/sigterm.log" 2>&1 &
sig_pid=$!
for _ in $(seq 1 50); do [[ -f "$TMP_ROOT/sigterm.blocked" ]] && break; sleep 0.2; done
if [[ ! -f "$TMP_ROOT/sigterm.blocked" ]]; then
  fail "sigterm: script never reached the blocking credential call; log: $(cat "$TMP_ROOT/sigterm.log")"
  kill -KILL -"$sig_pid" 2>/dev/null || true
else
  kill -TERM -"$sig_pid"
  wait "$sig_pid"; sig_rc=$?
  if [[ "$sig_rc" -eq 143 ]]; then echo "ok: sigterm: script exited 143"
  else fail "sigterm: expected exit 143, got $sig_rc; log: $(cat "$TMP_ROOT/sigterm.log")"; fi
  SCEN_HOME="$sig_home"
  cat >"$TMP_ROOT/assert-sigterm.py" <<'SIGASSERT'
import json, os
rec = json.load(open(os.environ["RECORD"]))
assert rec["result"] == "failed", rec
assert rec["exit_code"] == 143, rec
assert rec["failing_step"] == "credentials", rec
steps = {s["step"]: s["outcome"] for s in rec["steps"]}
assert steps["validate-date"] == "ok", rec
assert steps["credentials"] == "failed", rec
assert steps["db-dump"] == "not-attempted", rec
assert steps["local-archive"] == "not-attempted", rec
assert steps["r2-upload"] == "not-attempted", rec
assert steps["drive-upload"] == "not-attempted", rec
SIGASSERT
  check_record sigterm "$TMP_ROOT/assert-sigterm.py"
fi

cat >"$TMP_ROOT/assert-drivefail.py" <<'PYEOF'
import json, os
rec = json.load(open(os.environ["RECORD"]))
assert rec["result"] == "failed", rec
assert rec["failing_step"] == "drive-upload", rec
assert rec["destinations"] == {"local": "ok", "r2": "ok", "drive": "failed"}, rec
steps = {s["step"]: s["outcome"] for s in rec["steps"]}
assert steps["r2-upload"] == "ok", rec
assert steps["drive-upload"] == "failed", rec
PYEOF

run_scenario drivefail DRIVE
if [[ "$RC" -ne 0 ]]; then echo "ok: drivefail: script exited non-zero ($RC)"
else fail "drivefail: expected non-zero exit, got 0"; fi
check_record drivefail "$TMP_ROOT/assert-drivefail.py"

cat >"$TMP_ROOT/assert-dumpfail.py" <<'PYEOF'
import json, os
rec = json.load(open(os.environ["RECORD"]))
assert rec["result"] == "failed", rec
assert rec["failing_step"] == "db-dump", rec
assert rec["destinations"] == {"local": "not-attempted", "r2": "not-attempted", "drive": "not-attempted"}, rec
steps = {s["step"]: s["outcome"] for s in rec["steps"]}
assert steps["validate-date"] == "ok", rec
assert steps["credentials"] == "ok", rec
assert steps["db-dump"] == "failed", rec
assert steps["local-archive"] == "not-attempted", rec
PYEOF

run_scenario dumpfail "" fail
if [[ "$RC" -ne 0 ]]; then echo "ok: dumpfail: script exited non-zero ($RC)"
else fail "dumpfail: expected non-zero exit, got 0"; fi
check_record dumpfail "$TMP_ROOT/assert-dumpfail.py"

cat >"$TMP_ROOT/assert-dumpok.py" <<'PYEOF'
import json, os
rec = json.load(open(os.environ["RECORD"]))
assert rec["result"] == "complete", rec
assert rec["failing_step"] is None, rec
steps = {s["step"]: s["outcome"] for s in rec["steps"]}
assert steps["db-dump"] == "ok", rec
assert all(o == "ok" for o in steps.values()), rec
PYEOF

run_scenario dumpok "" ok
if [[ "$RC" -eq 0 ]]; then echo "ok: dumpok: script exited 0"
else fail "dumpok: expected exit 0, got $RC; log: $(cat "$TMP_ROOT/dumpok.log")"; fi
check_record dumpok "$TMP_ROOT/assert-dumpok.py"

if (( failures > 0 )); then
  echo "pm-dashboard-backup record tests: $failures failure(s)" >&2
  exit 1
fi
echo "pm-dashboard-backup record tests: all passed"
