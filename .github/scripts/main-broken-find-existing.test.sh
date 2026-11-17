#!/usr/bin/env bash
# Tests for main-broken-find-existing.sh (#9546): a fake gh on PATH
# serves a canned issue list; the replay case at the end is the
# acceptance scenario from #9546 -- two watcher firings for the same
# run id must produce exactly one issue and one dispatch decision.
set -euo pipefail

SCRIPT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/main-broken-find-existing.sh"
WATCHER_YML="$(cd "$(dirname "${BASH_SOURCE[0]}")/../workflows" && pwd)/main-broken.yml"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP/bin"

cat >"$TMP/bin/gh" <<'GH'
#!/usr/bin/env bash
# Fake gh: only `gh issue list ...` is supported; it prints the JSON
# stored in $FAKE_ISSUES_JSON.
if [ "$1" = "issue" ] && [ "$2" = "list" ]; then
  cat "$FAKE_ISSUES_JSON"
  exit 0
fi
echo "fake gh: unsupported invocation: $*" >&2
exit 1
GH
chmod +x "$TMP/bin/gh"

fail() {
  echo "FAIL: $1" >&2
  exit 1
}

find_existing() { # $1 = issues json file
  PATH="$TMP/bin:$PATH" FAKE_ISSUES_JSON="$1" \
    GH_REPO=developing-today/code BID=f846e6189265 RUN_ID=43392267205 \
    bash "$SCRIPT"
}

# Case 1: nothing filed yet -> no match (the firing creates the issue).
echo '[]' >"$TMP/none.json"
[ -z "$(find_existing "$TMP/none.json")" ] || fail "empty list must match nothing"

# Case 2: open issue carrying the Breakage-ID -> append target.
cat >"$TMP/open-bid.json" <<'JSON'
[{"number": 9528, "state": "OPEN", "body": "Run: https://github.com/developing-today/code/actions/runs/43399999999\n- Breakage-ID: f846e6189265\n"}]
JSON
[ "$(find_existing "$TMP/open-bid.json")" = "$(printf '9528\topen')" ] || fail "open Breakage-ID match"

# Case 3: open issue carrying this run's URL (different Breakage-ID) -> append.
cat >"$TMP/open-run.json" <<'JSON'
[{"number": 9531, "state": "OPEN", "body": "Run: https://github.com/developing-today/code/actions/runs/43392267205\n- Breakage-ID: 000000000000\n"}]
JSON
[ "$(find_existing "$TMP/open-run.json")" = "$(printf '9531\topen')" ] || fail "open run-id match"

# Case 4: closed issue carrying this run's URL -> comment, never file.
cat >"$TMP/closed-run.json" <<'JSON'
[{"number": 9523, "state": "CLOSED", "body": "Run: https://github.com/developing-today/code/actions/runs/43392267205\n- Breakage-ID: f846e6189265\n"}]
JSON
[ "$(find_existing "$TMP/closed-run.json")" = "$(printf '9523\tclosed')" ] || fail "closed run-id match"

# Case 5: closed issue with only the Breakage-ID (a NEW run recurring)
# must NOT match -- a genuine recurrence files fresh.
cat >"$TMP/closed-bid.json" <<'JSON'
[{"number": 9523, "state": "CLOSED", "body": "Run: https://github.com/developing-today/code/actions/runs/43300000000\n- Breakage-ID: f846e6189265\n"}]
JSON
[ -z "$(find_existing "$TMP/closed-bid.json")" ] || fail "closed Breakage-ID-only must not match"

# Case 6: priority -- an open Breakage-ID match beats a closed run match.
cat >"$TMP/both.json" <<'JSON'
[{"number": 9523, "state": "CLOSED", "body": "Run: https://github.com/developing-today/code/actions/runs/43392267205\n"},
 {"number": 9528, "state": "OPEN", "body": "- Breakage-ID: f846e6189265\n"}]
JSON
[ "$(find_existing "$TMP/both.json")" = "$(printf '9528\topen')" ] || fail "open bid must outrank closed run"

# Acceptance replay (#9546): firing 1 sees no issue and creates #9528;
# firing 2 for the SAME run id must find #9528 open (append), so the
# run yields exactly one issue; and the workflow's dispatch keys are
# keyed on the failing run id, so both firings carry the same
# dedupe_key and the queue dispatches exactly one fix session.
first="$(find_existing "$TMP/none.json")"
[ -z "$first" ] || fail "replay firing 1 must find nothing"
cat >"$TMP/after-create.json" <<'JSON'
[{"number": 9528, "state": "OPEN", "body": "Run: https://github.com/developing-today/code/actions/runs/43392267205\n- Breakage-ID: f846e6189265\n"}]
JSON
second="$(find_existing "$TMP/after-create.json")"
[ "$second" = "$(printf '9528\topen')" ] || fail "replay firing 2 must append to the issue firing 1 created"
grep -Fq 'dedupe_key: main-broken-run-${{ steps.identify.outputs.run_id }}-dispatch-append' "$WATCHER_YML" ||
  fail "append dispatch is not keyed on the failing run id"
grep -Fq 'dedupe_key: main-broken-run-${{ steps.identify.outputs.run_id }}-dispatch-create' "$WATCHER_YML" ||
  fail "create dispatch is not keyed on the failing run id"
grep -Fq "steps.identify.outputs.action == 'comment'" "$WATCHER_YML" ||
  fail "watcher has no comment-only path for closed run matches"

echo "main-broken-find-existing.test: all tests passed"
