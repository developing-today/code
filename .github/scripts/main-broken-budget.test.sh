#!/usr/bin/env bash
# Tests for main-broken-budget.sh (#1782): a fake gh on PATH serves
# and mutates a canned issue state (JSON file), so the scenarios
# run hermetically. The oracle is the acceptance addition recorded
# on #1782 (2026-11-05): two concurrent failures starting from two
# active remediations must never leave more than three
# `dispatch/remediate` labels active, and the budget decision is
# serialized on the budget scope, not the run id -- the workflow
# wiring assertions at the end pin that.
set -euo pipefail

SCRIPT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/main-broken-budget.sh"
WATCHER_YML="$(cd "$(dirname "${BASH_SOURCE[0]}")/../workflows" && pwd)/main-broken.yml"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP/bin"

# Fake gh over a mutable state file ($FAKE_STATE): a JSON array of
# {number, state, labels: [names], comments: [{body}]}.
#   gh issue list ... --json number,labels   -> projected records
#   gh issue edit N --add-label/--remove-label L
#   gh issue comment N --body TEXT
#   gh api repos/<r>/issues/N/comments?...   -> [{body}, ...]
cat >"$TMP/bin/gh" <<'GH'
#!/usr/bin/env bash
set -euo pipefail
state="$FAKE_STATE"
case "$1 $2" in
"issue list")
  jq '[.[] | select(.state == "OPEN")
    | {number, labels: [.labels[] | {name: .}]}]' "$state"
  ;;
"issue edit")
  n="$3"; shift 3
  while [ $# -gt 0 ]; do
    case "$1" in
    --add-label)
      jq --argjson n "$n" --arg l "$2" \
        'map(if .number == $n then .labels |= (. + [$l] | unique) else . end)' \
        "$state" >"$state.tmp" && mv "$state.tmp" "$state"
      shift 2
      ;;
    --remove-label)
      jq --argjson n "$n" --arg l "$2" \
        'map(if .number == $n then .labels |= map(select(. != $l)) else . end)' \
        "$state" >"$state.tmp" && mv "$state.tmp" "$state"
      shift 2
      ;;
    *) shift ;;
    esac
  done
  ;;
"issue comment")
  n="$3"; shift 3
  body=""
  while [ $# -gt 0 ]; do
    case "$1" in
    --body) body="$2"; shift 2 ;;
    *) shift ;;
    esac
  done
  jq --argjson n "$n" --arg b "$body" \
    'map(if .number == $n then .comments |= (. + [{body: $b}]) else . end)' \
    "$state" >"$state.tmp" && mv "$state.tmp" "$state"
  ;;
"api repos/"*)
  # $2 is repos/<r>/issues/<n>/comments?per_page=100
  n=$(sed -n 's|.*/issues/\([0-9]*\)/.*|\1|p' <<<"$2")
  jq --argjson n "$n" '[.[] | select(.number == $n) | .comments[]]' "$state"
  ;;
*)
  echo "fake gh: unsupported invocation: $*" >&2
  exit 1
  ;;
esac
GH
chmod +x "$TMP/bin/gh"

fail() {
  echo "FAIL: $1" >&2
  exit 1
}

run_budget() { # $1 = state file; extra env via DRY_RUN when set by caller
  PATH="$TMP/bin:$PATH" FAKE_STATE="$1" \
    GH_REPO=developing-today/code DRY_RUN="${DRY_RUN:-false}" \
    bash "$SCRIPT" >/dev/null
}

labelled() { # $1 = state file -> sorted labelled issue numbers
  jq -r '[.[] | select(.labels | index("dispatch/remediate")) | .number] | sort | join(",")' "$1"
}

has_comment() { # $1 = state, $2 = number, $3 = substring
  jq -e --argjson n "$2" --arg s "$3" \
    '[.[] | select(.number == $n) | .comments[].body | select(contains($s))] | length > 0' \
    "$1" >/dev/null
}

mkissue() { # number, labels csv -> jq object on stdout
  jq -n --argjson n "$1" --arg l "$2" \
    '{number: $n, state: "OPEN", labels: ($l | split(",") | map(select(length > 0))), comments: []}'
}

# --- Scenario 1: grant under budget --------------------------------
{
  mkissue 10 "main-broken,queue/queued"
} | jq -s '.' >"$TMP/s1.json"
run_budget "$TMP/s1.json"
[ "$(labelled "$TMP/s1.json")" = "10" ] || fail "s1: undecided candidate must be granted the label"
has_comment "$TMP/s1.json" 10 "holder: dispatch" || fail "s1: grant must post the holder: dispatch note"

# --- Scenario 2: budget full -> exhausted note, and idempotent -----
{
  mkissue 1 "main-broken,dispatch/remediate"
  mkissue 2 "main-broken,dispatch/remediate"
  mkissue 3 "main-broken,dispatch/remediate"
  mkissue 11 "main-broken,queue/queued"
} | jq -s '.' >"$TMP/s2.json"
run_budget "$TMP/s2.json"
[ "$(labelled "$TMP/s2.json")" = "1,2,3" ] || fail "s2: a full budget must grant nothing"
has_comment "$TMP/s2.json" 11 "remediation budget exhausted" ||
  fail "s2: the overflow candidate must get the budget-exhausted note"
run_budget "$TMP/s2.json"
count=$(jq '[.[] | select(.number == 11) | .comments | length] | .[0]' "$TMP/s2.json")
[ "$count" = "1" ] || fail "s2: a decided candidate must not be re-noted on the next reconcile"

# --- Scenario 3 (the #1782 acceptance case): two active labels, ---
# --- two distinct failures decided by two serialized runs ---------
{
  mkissue 1 "main-broken,dispatch/remediate"
  mkissue 2 "main-broken,dispatch/remediate"
  mkissue 12 "main-broken,queue/queued"
  mkissue 13 "main-broken,queue/queued"
} | jq -s '.' >"$TMP/s3.json"
run_budget "$TMP/s3.json" # first firing's budget job
run_budget "$TMP/s3.json" # second firing's budget job (serialized)
[ "$(labelled "$TMP/s3.json")" = "1,2,12" ] ||
  fail "s3: exactly three labels may be active; the oldest candidate wins the last slot"
has_comment "$TMP/s3.json" 13 "remediation budget exhausted" ||
  fail "s3: the candidate that misses the budget must be decided as exhausted"

# --- Scenario 4: cap enforcement over a pre-serialized race --------
{
  mkissue 1 "main-broken,dispatch/remediate"
  mkissue 2 "main-broken,dispatch/remediate"
  mkissue 3 "main-broken,dispatch/remediate"
  mkissue 4 "main-broken,dispatch/remediate"
} | jq -s '.' >"$TMP/s4.json"
run_budget "$TMP/s4.json"
[ "$(labelled "$TMP/s4.json")" = "1,2,3" ] ||
  fail "s4: the cap must hold after the run; the highest-numbered holder yields"
has_comment "$TMP/s4.json" 4 "remediation budget exhausted" ||
  fail "s4: the yielded holder must be told why"

# --- Scenario 5: decided candidates are never revisited -----------
{
  mkissue 14 "main-broken,queue/queued"
} | jq -s '.' >"$TMP/s5.json"
jq 'map(if .number == 14 then .comments = [{body: "agent-note:v1 — holder: released by chain at 2026-11-20T00:00:00Z, reason: no remediation PR within 24h"}] else . end)' \
  "$TMP/s5.json" >"$TMP/s5b.json" && mv "$TMP/s5b.json" "$TMP/s5.json"
run_budget "$TMP/s5.json"
[ "$(labelled "$TMP/s5.json")" = "" ] ||
  fail "s5: a released candidate must not be re-granted when budget frees up"

# --- Scenario 6: no queue/queued -> not a candidate ----------------
{
  mkissue 15 "main-broken"
} | jq -s '.' >"$TMP/s6.json"
run_budget "$TMP/s6.json"
[ "$(labelled "$TMP/s6.json")" = "" ] || fail "s6: an issue with no fix task queued is not a budget candidate"
count=$(jq '[.[] | .comments | length] | add' "$TMP/s6.json")
[ "$count" = "0" ] || fail "s6: a non-candidate must not be noted"

# --- Scenario 7: dry-run mutates nothing ---------------------------
{
  mkissue 16 "main-broken,queue/queued"
} | jq -s '.' >"$TMP/s7.json"
DRY_RUN=true run_budget "$TMP/s7.json"
[ "$(labelled "$TMP/s7.json")" = "" ] || fail "s7: dry-run must not label"
count=$(jq '[.[] | .comments | length] | add' "$TMP/s7.json")
[ "$count" = "0" ] || fail "s7: dry-run must not comment"

# --- Workflow wiring: serialization keys derive from the ---------
# --- Breakage-ID / budget scope, never the run id ------------------
grep -Fq 'group: main-broken-file-${{ needs.identify.outputs.bid }}' "$WATCHER_YML" ||
  fail "filing must be serialized per Breakage-ID"
grep -Fq 'group: main-broken-budget' "$WATCHER_YML" ||
  fail "the budget decision must be serialized on the budget scope"
grep -Fq 'main-broken-budget.sh' "$WATCHER_YML" ||
  fail "the watcher must run the budget reconcile script"
if grep -Fq 'github.event_name }}-${{ github.event.workflow_run.id || github.run_id }}' "$WATCHER_YML"; then
  fail "the old per-run concurrency group must be gone"
fi

echo "main-broken-budget.test: all tests passed"
