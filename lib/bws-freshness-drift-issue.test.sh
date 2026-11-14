#!/usr/bin/env bash
# Tests for `lib/bws-freshness-drift-issue.sh` (#6644).
#
# Oracle: canned `gh issue list` JSON documents with known identity
# signals. The finder must return the workflow-owned issue (exact title
# + hidden body marker), never an unrelated same-words title — in
# particular not when the unrelated issue sorts first, which is the
# case that made the old `.[0]` title-search lookup comment on, and
# close, the wrong issue.
#
# gh is stubbed on PATH: it prints the fixture document ($GH_FIXTURE)
# regardless of arguments. No live GitHub access is used.
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"
UNDER_TEST="$SCRIPT_DIR/bws-freshness-drift-issue.sh"
[ -f "$UNDER_TEST" ] || {
  echo "FAIL: $UNDER_TEST not found" >&2
  exit 1
}
command -v jq >/dev/null 2>&1 || {
  echo "FAIL: jq not found in PATH" >&2
  exit 1
}

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
STUB_BIN="$TMP/bin"
mkdir -p "$STUB_BIN"

cat >"$STUB_BIN/gh" <<'STUB'
#!/usr/bin/env bash
# Fixture stub: prints the canned issue-list document for any invocation.
cat "$GH_FIXTURE"
STUB
chmod +x "$STUB_BIN/gh"
export PATH="$STUB_BIN:$PATH"

FAILURES=0

# check <name> <fixture-json> <expected-output>
check() {
  local name="$1" fixture="$2" expected="$3"
  local file="$TMP/$name.json"
  printf '%s' "$fixture" >"$file"
  local got
  got="$(GH_FIXTURE="$file" bash "$UNDER_TEST")"
  if [ "$got" = "$expected" ]; then
    echo "PASS $name"
  else
    echo "FAIL $name: expected [$expected], got [$got]" >&2
    FAILURES=$((FAILURES + 1))
  fi
}

OWNED='{"number": 4242, "title": "bws.yaml mirror drift detected", "createdAt": "2026-11-02T06:23:00Z", "body": "<!-- bws-freshness-drift-issue v1 -->\n\nThe scheduled names-only freshness check (#5451) found drift."}'
UNRELATED_FIRST='{"number": 9001, "title": "bws.yaml mirror drift detected in staging export", "createdAt": "2026-11-01T00:00:00Z", "body": "A human issue about a different mirror."}'
SAME_TITLE_NO_MARKER='{"number": 9002, "title": "bws.yaml mirror drift detected", "createdAt": "2026-10-01T00:00:00Z", "body": "A human issue that happens to use the exact title."}'
OWNED_NEWER='{"number": 4300, "title": "bws.yaml mirror drift detected", "createdAt": "2026-11-03T06:23:00Z", "body": "<!-- bws-freshness-drift-issue v1 -->\n\nA second episode opened in error."}'

# The workflow's own issue is found even when an unrelated same-words
# title sorts ahead of it (the old lookup's failure mode).
check unrelated_first "[$UNRELATED_FIRST, $OWNED]" "4242"
# An exact-title human issue without the marker is not the drift issue.
check same_title_no_marker "[$SAME_TITLE_NO_MARKER]" ""
# No candidates at all: no-op (empty output), never an error.
check none_open "[]" ""
# Only unrelated candidates: no-op.
check only_unrelated "[$UNRELATED_FIRST]" ""
# Two verified issues (should not happen): the oldest episode wins.
check two_owned "[$OWNED_NEWER, $OWNED]" "4242"

if [ "$FAILURES" -gt 0 ]; then
  echo "FAILED: $FAILURES case(s)" >&2
  exit 1
fi
echo "OK: all bws-freshness-drift-issue cases passed"
