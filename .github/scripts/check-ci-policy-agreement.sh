#!/usr/bin/env bash
# check-ci-policy-agreement.sh — fail when ci-policy's tier promises and
# a workflow's triggers disagree (#7090, Phase 1 of design #7089).
#
# Scope is exactly the "Agreement-tracked guard/build rows" table in
# .github/ci-policy.md: the seven guard/build workflows of Phase 1.
# Workflows ci-policy does not name are not inspected, and test-suite
# registry rows (#6185) are out of scope.
#
# Mapping (stated in ci-policy.md):
#   basic   — pull_request, path-gated where the row names a PR gate
#   main    — push: branches: [main]
#   nightly — schedule
#   workflow_dispatch alone satisfies no tier
#
# Known Phase 2–3 disagreements live in .github/ci-policy-waivers.txt,
# one line per missing leg. A waiver must name one of the four live
# phase issues (#6929, #6953, #6959, #6981); a waiver on a compliant
# leg is stale and fails.
#
# Usage:
#   check-ci-policy-agreement.sh            check the repository tree
#
# Environment:
#   CI_POLICY_ROOT                  repository root (default: this repo)
#   CI_POLICY_REPO                  owner/repo for issue state (default:
#                                   $GITHUB_REPOSITORY or the code repo)
#   CI_POLICY_ISSUE_STATE_FIXTURE   file of "<issue> <state>" lines, used
#                                   by the contract tests instead of gh
#
# Oracle: the mapping above plus GitHub's own workflow-trigger syntax.
# The contract tests beside this script exercise every failure mode.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="${CI_POLICY_ROOT:-$(cd "$SCRIPT_DIR/../.." && pwd)}"
POLICY="$ROOT/.github/ci-policy.md"
WAIVERS="$ROOT/.github/ci-policy-waivers.txt"
REPO="${CI_POLICY_REPO:-${GITHUB_REPOSITORY:-developing-today/code}}"

EXPECTED_ISSUES=(6929 6953 6959 6981)

fail=0
note_fail() {
  echo "ci-policy-agreement: FAIL: $*" >&2
  fail=1
}

trim() {
  local s="$1"
  s="${s#"${s%%[![:space:]]*}"}"
  s="${s%"${s##*[![:space:]]}"}"
  printf '%s' "$s"
}

strip_code() {
  local s="$1"
  s="${s//\`/}"
  trim "$s"
}

# parse_on FILE — emit TSV rows for the top-level on: block.
#   EVENT  <event>
#   PATH   <event> <path>
#   BRANCH <event> <branch>
#   INLINE <unparseable line>
parse_on() {
  awk '
    function trim(s) { sub(/^[[:space:]]+/, "", s); sub(/[[:space:]]+$/, "", s); return s }
    /^on:[[:space:]]*\[/ { print "INLINE\t" $0; next }
    /^on:[[:space:]]*$/ { in_on = 1; next }
    in_on && /^[^[:space:]#]/ { exit }
    !in_on { next }
    /^[[:space:]]*#/ { next }
    /^  [A-Za-z_][A-Za-z0-9_-]*:[[:space:]]*$/ {
      ev = $0; sub(/^  /, "", ev); sub(/:.*/, "", ev)
      print "EVENT\t" ev
      sub_key = ""
      next
    }
    /^  [A-Za-z_][A-Za-z0-9_-]*:[[:space:]]+.+/ {
      print "INLINE\t" trim($0)
      next
    }
    /^    branches:[[:space:]]*\[/ {
      v = $0; sub(/^    branches:[[:space:]]*\[/, "", v); sub(/\].*$/, "", v)
      n = split(v, parts, ",")
      for (i = 1; i <= n; i++) {
        b = trim(parts[i]); gsub(/^"|"$/, "", b); gsub(/^'\''|'\''$/, "", b)
        if (b != "") print "BRANCH\t" ev "\t" b
      }
      sub_key = ""
      next
    }
    /^    branches:/ { sub_key = "branches"; next }
    /^    paths:/    { sub_key = "paths"; next }
    /^    paths-ignore:/ { sub_key = "paths-ignore"; next }
    /^    [A-Za-z_-]+:/ { sub_key = ""; next }
    /^      - / {
      if (ev == "") next
      v = $0; sub(/^      - /, "", v); gsub(/^"|"$/, "", v); v = trim(v)
      if (sub_key == "branches") print "BRANCH\t" ev "\t" v
      else if (sub_key == "paths") print "PATH\t" ev "\t" v
    }
  ' "$1"
}

declare -A WAIVER=()
declare -A WAIVER_SEEN=()

if [ ! -f "$POLICY" ]; then
  echo "ci-policy-agreement: FAIL: policy not found: $POLICY" >&2
  exit 1
fi

# --- waivers (read first: the row pass consults them) ------------------
if [ -f "$WAIVERS" ]; then
  while IFS= read -r line; do
    line="$(trim "$line")"
    case "$line" in ''|'#'*) continue ;; esac
    read -r wf leg issue _ <<<"$line"
    case "$leg" in
      basic|main|nightly) ;;
      *) note_fail "waiver line has unknown leg '$leg': $line"; continue ;;
    esac
    case "$issue" in
      '#'[0-9]*) issue="${issue#\#}" ;;
      [0-9]*) ;;
      *) note_fail "waiver line has no issue number: $line"; continue ;;
    esac
    key="$wf|$leg"
    if [ -n "${WAIVER[$key]:-}" ] && [ "${WAIVER[$key]}" != "$issue" ]; then
      note_fail "conflicting waivers for $wf $leg (#${WAIVER[$key]} and #$issue)"
    fi
    WAIVER["$key"]="$issue"
  done <"$WAIVERS"
else
  note_fail "waiver file not found: $WAIVERS"
fi

# --- policy rows -------------------------------------------------------
rows=0
while IFS= read -r line; do
  case "$line" in
    '| `.github/workflows/'*) ;;
    *) continue ;;
  esac
  IFS='|' read -r _ c_wf c_basic c_main c_nightly c_gate _ <<<"$line"
  wf="$(strip_code "$c_wf")"
  basic="$(trim "$c_basic")"
  main="$(trim "$c_main")"
  nightly="$(trim "$c_nightly")"
  gate="$(strip_code "$c_gate")"
  rows=$((rows + 1))

  case "$wf" in
    .github/workflows/*.yml|.github/workflows/*.yaml) ;;
    *) note_fail "policy row names '$wf', which is not a .github/workflows/*.yml file"; continue ;;
  esac

  if [ ! -f "$ROOT/$wf" ]; then
    note_fail "ci-policy names $wf but the file does not exist"
    continue
  fi

  declare -A EVENTS=() PATHS=() BRANCHES=()
  while IFS=$'\t' read -r kind a b; do
    case "$kind" in
      EVENT) EVENTS["$a"]=1 ;;
      PATH) PATHS["$a|$b"]=1 ;;
      BRANCH) BRANCHES["$a|$b"]=1 ;;
      INLINE) note_fail "$wf: cannot parse inline on: entry: $a" ;;
    esac
  done < <(parse_on "$ROOT/$wf")

  check_leg() { # check_leg LEG PRESENT MISSING-DESCRIPTION
    local leg="$1" present="$2" missing="$3"
    local key="$wf|$1"
    if [ "$present" = "1" ]; then
      if [ -n "${WAIVER[$key]:-}" ]; then
        WAIVER_SEEN["$key"]=1
        note_fail "$wf: stale waiver for $leg (#${WAIVER[$key]}) — the leg is present"
      fi
      return
    fi
    if [ -n "${WAIVER[$key]:-}" ]; then
      echo "ci-policy-agreement: WAIVED $wf $leg (#${WAIVER[$key]}): $missing"
      WAIVER_SEEN["$key"]=1
      return
    fi
    note_fail "$wf missing $leg tier: $missing"
  }

  if [ "$basic" = "yes" ]; then
    present=0
    describe="${gate:+ with a path under $gate}"
    [ "$gate" = "-" ] && describe=""
    if [ -n "${EVENTS[pull_request]:-}" ]; then
      present=1
      if [ "$gate" != "-" ] && [ -n "$gate" ]; then
        gate_ok=0
        for k in "${!PATHS[@]}"; do
          ev="${k%%|*}"
          p="${k#*|}"
          [ "$ev" = "pull_request" ] || continue
          # shellcheck disable=SC2053 # glob match is intended
          if [[ "$p" == $gate ]]; then gate_ok=1; break; fi
        done
        [ "$gate_ok" = "1" ] || present=0
      fi
    fi
    check_leg basic "$present" "no pull_request trigger$describe"
  elif [ -n "${WAIVER[$wf|basic]:-}" ]; then
    WAIVER_SEEN["$wf|basic"]=1
    note_fail "$wf: stale waiver for basic (#${WAIVER[$wf|basic]}) — policy does not assign basic"
  fi

  if [ "$main" = "yes" ]; then
    present=0
    if [ -n "${EVENTS[push]:-}" ] && [ -n "${BRANCHES[push|main]:-}" ]; then
      present=1
    fi
    check_leg main "$present" "no push: branches: [main] trigger"
  elif [ -n "${WAIVER[$wf|main]:-}" ]; then
    WAIVER_SEEN["$wf|main"]=1
    note_fail "$wf: stale waiver for main (#${WAIVER[$wf|main]}) — policy does not assign main"
  fi

  if [ "$nightly" = "yes" ]; then
    present=0
    [ -n "${EVENTS[schedule]:-}" ] && present=1
    check_leg nightly "$present" "no schedule trigger"
  elif [ -n "${WAIVER[$wf|nightly]:-}" ]; then
    WAIVER_SEEN["$wf|nightly"]=1
    note_fail "$wf: stale waiver for nightly (#${WAIVER[$wf|nightly]}) — policy does not assign nightly"
  fi

  unset EVENTS PATHS BRANCHES
done <"$POLICY"

if [ "$rows" -eq 0 ]; then
  note_fail "no agreement-tracked workflow rows found in $POLICY"
fi

# Every waiver in the file must correspond to an actual disagreement.
for k in "${!WAIVER[@]}"; do
  if [ -z "${WAIVER_SEEN[$k]:-}" ]; then
    wf="${k%%|*}"
    leg="${k#*|}"
    note_fail "stale waiver: $wf $leg (#${WAIVER[$k]}) — no disagreement on that leg"
  fi
done

# --- waiver issue liveness --------------------------------------------
declare -A ISSUES_USED=()
for k in "${!WAIVER[@]}"; do
  ISSUES_USED["${WAIVER[$k]}"]=1
done

for issue in "${!ISSUES_USED[@]}"; do
  known=0
  for expected in "${EXPECTED_ISSUES[@]}"; do
    [ "$issue" = "$expected" ] && known=1
  done
  if [ "$known" != "1" ]; then
    note_fail "waiver names #$issue, which is not one of the live Phase 2–3 issues (#6929, #6953, #6959, #6981)"
  fi
done

# Every issue the checker verifies must be open. Unused phase issues are
# fine: closing one retires its waivers, and the next real drift must be
# waived in the same change.

state_of() { # state_of ISSUE
  local issue="$1" state=""
  if [ -n "${CI_POLICY_ISSUE_STATE_FIXTURE:-}" ]; then
    state="$(awk -v n="$issue" '$1 == n { print $2; exit }' "$CI_POLICY_ISSUE_STATE_FIXTURE")"
  elif command -v gh >/dev/null 2>&1; then
    state="$(gh issue view "$issue" --repo "$REPO" --json state --jq .state 2>/dev/null || true)"
  fi
  printf '%s' "$state"
}

for issue in "${!ISSUES_USED[@]}"; do
  state="$(state_of "$issue")"
  case "$state" in
    OPEN|open|Open) ISSUE_STATE="open" ;;
    CLOSED|closed|Closed) ISSUE_STATE="closed" ;;
    *) ISSUE_STATE="unknown" ;;
  esac
  case "$ISSUE_STATE" in
    open) ;;
    closed) note_fail "waiver issue #$issue is closed — remove its waivers in this change" ;;
    *) note_fail "cannot verify waiver issue #$issue is open (set CI_POLICY_ISSUE_STATE_FIXTURE in tests, or check gh/GITHUB_REPOSITORY in CI)" ;;
  esac
done

if [ "$fail" -ne 0 ]; then
  echo "ci-policy-agreement: FAILED" >&2
  exit 1
fi

waived=0
for k in "${!WAIVER_SEEN[@]}"; do waived=$((waived + 1)); done
echo "ci-policy-agreement: checked $rows workflow(s); $waived waived leg(s) across live issues #6929, #6953, #6959, #6981"
echo "ci-policy-agreement: OK"
