#!/usr/bin/env bash
# Contract: test.sh (no arguments). Uses only a temporary directory and a fake OpenCode executable.
set -Eeuo pipefail
if (($#)); then echo "usage: $0" >&2; exit 2; fi

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP/bin" "$TMP/state"
cat >"$TMP/bin/opencode" <<'MOCK'
#!/usr/bin/env bash
set -Eeuo pipefail
printf '%s\n' "$@" >"$MOCK_ARGS"
cat "${@: -2:1}" >/dev/null
printf 'fake OpenCode completed\n'
MOCK
chmod +x "$TMP/bin/opencode"
python3 -I - "$TMP/quota-current.json" <<'PY'
import json, sys
from datetime import datetime, timezone
json.dump({
    "version": 1,
    "generatedAt": datetime.now(timezone.utc).isoformat(),
    "heartbeat": {
        "tickAt": (datetime.now(timezone.utc) - __import__("datetime").timedelta(minutes=5)).isoformat(),
        "ok": True,
        "durationMs": 1,
        "perProvider": {},
    },
    "providers": [{
        "providerName": "Example",
        "state": "lapsed",
        "measuredAt": datetime.now(timezone.utc).isoformat(),
        "secret": "must-not-leak",
        "usage": 123456,
        "windows": [{"name": "weekly", "state": "ok", "balance": 99}],
    }],
}, open(sys.argv[1], "w"))
PY
python3 -I - "$TMP/quota-gatherer-heartbeat.json" <<'PY'
import json, sys
from datetime import datetime, timezone
json.dump(
    {
        "tickAt": datetime.now(timezone.utc).isoformat(),
        "ok": False,
        "durationMs": 1,
        "perProvider": {},
        "error": "fixture failure",
    },
    open(sys.argv[1], "w"),
)
PY
export PATH="$TMP/bin:$PATH"
export OPENCODE_BIN="$TMP/bin/opencode" OPENCODE_MODEL=opencode-route/workhorse
export OPENCODE_SCHEDULED_JOB_DIR="$TMP/state"
export QUOTA_CURRENT_PATH="$TMP/quota-current.json" QUOTA_HEARTBEAT_PATH="$TMP/quota-gatherer-heartbeat.json" MOCK_ARGS="$TMP/args"
bash "$ROOT/quota-freshness-summary/run.sh" >"$TMP/stdout"
grep -F -- 'opencode-route/workhorse' "$TMP/args" >/dev/null
grep -F -- '--file' "$TMP/args" >/dev/null
grep -F 'exitCode": 0' "$TMP/state/history.jsonl" >/dev/null
grep -F 'opencode-route/workhorse' "$TMP/state/history.jsonl" >/dev/null
! grep -F 'must-not-leak' "$TMP/state/input.json" >/dev/null
! grep -F '123456' "$TMP/state/input.json" >/dev/null
grep -F '"gathererHealthy": false' "$TMP/state/input.json" >/dev/null
grep -F '"state": "lapsed"' "$TMP/state/input.json" >/dev/null
grep -F 'fake OpenCode completed' "$TMP/state/latest.log" >/dev/null
python3 -I - "$ROOT/quota-freshness-summary" <<'PY'
import importlib.util, sys
from datetime import datetime, timezone, timedelta
spec = importlib.util.spec_from_file_location("summary", sys.argv[1] + "/summarize.py")
summary = importlib.util.module_from_spec(spec)
spec.loader.exec_module(summary)
now = datetime(2026, 11, 21, 12, tzinfo=timezone.utc)
source = {
    "generatedAt": (now - timedelta(minutes=1)).isoformat(),
    "heartbeat": {
        "tickAt": (now - timedelta(minutes=5)).isoformat(),
        "ok": True,
        "durationMs": 1,
        "perProvider": {},
    },
    "providers": [{"providerName": "Example", "state": "lapsed"}],
}
standalone = {
    "tickAt": (now - timedelta(minutes=1)).isoformat(),
    "ok": False,
    "durationMs": 1,
    "perProvider": {},
    "error": "failed",
}
report = summary.build_report(source, now, standalone)
assert report["gathererHealthy"] is False and report["gathererError"] is True
assert report["gathererHeartbeatFresh"] is True
assert report["providerCounts"]["lapsed"] == 1
assert report["providers"][0]["state"] == "lapsed"
PY
if OPENCODE_SCHEDULED_JOB_DIR="$TMP/invalid-state" bash "$ROOT/quota-freshness-summary/run.sh" --dry-run >"$TMP/invalid-run.out" 2>&1; then
  echo 'run.sh accepted an undeclared argument' >&2
  exit 1
fi
grep -F 'usage:' "$TMP/invalid-run.out" >/dev/null
test ! -e "$TMP/invalid-state"
if "$ROOT/install-laptop.sh" --dry-run unexpected >"$TMP/invalid.out" 2>&1; then
  echo 'installer accepted an undeclared argument' >&2
  exit 1
fi
echo 'opencode-scheduled-jobs: PASS'
