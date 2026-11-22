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
    "heartbeat": {"ok": True},
    "providers": [{
        "providerName": "Example",
        "state": "ok",
        "measuredAt": datetime.now(timezone.utc).isoformat(),
        "secret": "must-not-leak",
        "usage": 123456,
        "windows": [{"name": "weekly", "state": "ok", "balance": 99}],
    }],
}, open(sys.argv[1], "w"))
PY
export PATH="$TMP/bin:$PATH"
export OPENCODE_BIN="$TMP/bin/opencode" OPENCODE_MODEL=opencode-route/workhorse
export OPENCODE_SCHEDULED_JOB_DIR="$TMP/state"
export QUOTA_CURRENT_PATH="$TMP/quota-current.json" MOCK_ARGS="$TMP/args"
bash "$ROOT/quota-freshness-summary/run.sh" >"$TMP/stdout"
grep -F -- 'opencode-route/workhorse' "$TMP/args" >/dev/null
grep -F -- '--file' "$TMP/args" >/dev/null
grep -F 'exitCode": 0' "$TMP/state/history.jsonl" >/dev/null
grep -F 'opencode-route/workhorse' "$TMP/state/history.jsonl" >/dev/null
! grep -F 'must-not-leak' "$TMP/state/input.json" >/dev/null
! grep -F '123456' "$TMP/state/input.json" >/dev/null
grep -F 'fake OpenCode completed' "$TMP/state/latest.log" >/dev/null
if "$ROOT/install-laptop.sh" --dry-run unexpected >"$TMP/invalid.out" 2>&1; then
  echo 'installer accepted an undeclared argument' >&2
  exit 1
fi
echo 'opencode-scheduled-jobs: PASS'
