#!/usr/bin/env bash
# CI-policy agreement checker (#7089, Phase 1 of the ci-policy
# agreement contract; design: code-ci-policy-agreement.md).
#
# The contract: for every workflow in the registry below, three
# records must agree — the tier(s) .github/ci-policy.md assigns, the
# triggers the workflow declares, and the recorded waiver set. A
# disagreement with no waiver fails, naming the workflow and the
# missing leg; a waiver whose leg now agrees is stale and also
# fails, so the waiver set cannot silently outlive the disagreement
# it records. Enablement (Actions API state, e.g. disabled_manually)
# is not visible from the tree: it is audited on the #6187 recurring
# cadence, and the currently disabled in-scope venues are exactly
# the waiver set below (recorded in ci-policy.md, #7089 section).
#
# Tier -> trigger mapping (pinned in ci-policy.md, #7089 section;
# updated for Drew's 2026-11-08 no-merge-queue ruling, which
# supersedes the design's original merge_group pin):
#   pr      (basic/PR tier)  -> pull_request trigger
#   main    (main tier)      -> push trigger (branches: [main])
#   nightly (nightly tier)   -> schedule trigger
# A workflow_dispatch-only block satisfies NO tier.
#
# Usage: check-ci-policy-agreement.sh [ROOT]   (default: repo root)
set -euo pipefail

ROOT="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
POLICY="$ROOT/.github/ci-policy.md"
WF_DIR="$ROOT/.github/workflows"

# Registry: workflow-file|ci-policy marker|assigned tiers.
# The marker is text that must appear in ci-policy.md (the workflow's
# tier-table row or section), so a policy row cannot silently vanish.
# Scope: the guard/build workflows the #7089 contract covers — the
# Phase 2-3 set (#6929, #6953, #6959, #6981) plus the conforming
# basic-tier guards. Test-suite venues are #6185's registry, not
# this one; Phase 4 of #7089 inventories widening the scope.
REGISTRY=(
  "workflow-lint.yml|workflow-lint|pr"
  "proxy-port-fallbacks.yml|proxy-port-fallbacks|pr"
  "fleet-search-parity-guard.yml|fleet-search-parity-guard|pr"
  "opencode-config-guard.yml|opencode-config-guard|pr,main"
  "mcpx-fix-test-guard.yml|mcpx fix-test guard|pr,main"
  "production-line-guard.yml|production-line guard|pr"
  "go-modules-build.yml|go modules build|pr,main"
  "identity-build.yml|identity build|pr,main"
  "map-build.yml|map build|pr,main"
  "mcpx-nixbuild.yml|mcpx nix build|pr,main"
)

# Waivers: workflow-file|tier-leg|phase issue. Each waiver records a
# Phase 2-3 disagreement of #7089 that awaits Drew's
# re-arm-vs-dispatch-only ruling; it is not a disposition. When a
# phase lands (or is ruled retire), delete the waiver in the same PR
# that changes the triggers or the policy row.
WAIVERS=(
  # Phase 2 (#6953): venue disabled_manually; pull_request leg
  # restored by the 2026-11-08 sweep fix, main-tier push leg missing.
  "opencode-config-guard.yml|main|6953"
  # Phase 2 (#6929): dispatch-only; neither tier leg declared, and
  # the enforcement step is pull_request-gated so even a dispatch
  # never runs the #1969 guard.
  "mcpx-fix-test-guard.yml|pr|6929"
  "mcpx-fix-test-guard.yml|main|6929"
  # Phase 3 (#6959): dispatch-only; the guard's ci-policy row is new
  # in Phase 1 and its PR leg awaits the Phase 3 disposition.
  "production-line-guard.yml|pr|6959"
  # Phase 3 (#6981): all four dispatch-only AND disabled_manually;
  # neither tier leg declared.
  "go-modules-build.yml|pr|6981"
  "go-modules-build.yml|main|6981"
  "identity-build.yml|pr|6981"
  "identity-build.yml|main|6981"
  "map-build.yml|pr|6981"
  "map-build.yml|main|6981"
  "mcpx-nixbuild.yml|pr|6981"
  "mcpx-nixbuild.yml|main|6981"
)

required_trigger() {
  case "$1" in
    pr) echo "pull_request" ;;
    main) echo "push" ;;
    nightly) echo "schedule" ;;
    *) echo "" ;;
  esac
}

# triggers_of <workflow-file>: print the declared trigger names of
# the workflow's on: block, one per line. Handles the block form
# (two-space-indented keys) and the single-line forms
# (on: push, on: [push, pull_request]).
triggers_of() {
  local file="$1"
  awk '
    /^"?on"?:[[:space:]]*\[/ {
      line = $0
      sub(/^"?on"?:[[:space:]]*\[/, "", line)
      sub(/\].*/, "", line)
      n = split(line, parts, ",")
      for (i = 1; i <= n; i++) {
        gsub(/[[:space:]]/, "", parts[i])
        if (parts[i] != "") print parts[i]
      }
      exit
    }
    /^"?on"?:[[:space:]]*[a-z_]/ {
      line = $0
      sub(/^"?on"?:[[:space:]]*/, "", line)
      sub(/[[:space:]].*/, "", line)
      print line
      exit
    }
    /^"?on"?:[[:space:]]*$/ { inblock = 1; next }
    inblock && /^  [a-z_]+:/ {
      key = $0
      sub(/^  /, "", key)
      sub(/:.*/, "", key)
      print key
      next
    }
    inblock && /^[^[:space:]#]/ { inblock = 0 }
  ' "$file"
}

waiver_for() { # waiver_for <file> <leg> -> issue or empty
  local entry
  for entry in "${WAIVERS[@]}"; do
    if [ "${entry%%|*}" = "$1" ]; then
      local rest="${entry#*|}"
      if [ "${rest%%|*}" = "$2" ]; then
        echo "${rest#*|}"
        return 0
      fi
    fi
  done
  echo ""
}

fail=0
checked=0
waived=0
declare -A SEEN_WAIVER=()

if [ ! -f "$POLICY" ]; then
  echo "::error::ci-policy agreement: $POLICY not found"
  exit 1
fi

for row in "${REGISTRY[@]}"; do
  file="${row%%|*}"
  rest="${row#*|}"
  marker="${rest%%|*}"
  tiers="${rest#*|}"
  checked=$((checked + 1))
  wf="$WF_DIR/$file"
  if [ ! -f "$wf" ]; then
    echo "::error::DISAGREEMENT $file: workflow file missing (ci-policy assigns tiers: $tiers)"
    fail=1
    continue
  fi
  if ! grep -qiF "$marker" "$POLICY"; then
    echo "::error::DISAGREEMENT $file: ci-policy.md names no row/section for it (marker '$marker' absent) — every in-scope workflow needs a tier row"
    fail=1
  fi
  triggers="$(triggers_of "$wf")"
  IFS=',' read -ra tier_list <<<"$tiers"
  for tier in "${tier_list[@]}"; do
    req="$(required_trigger "$tier")"
    if printf '%s\n' "$triggers" | grep -qx "$req"; then
      if [ -n "$(waiver_for "$file" "$tier")" ]; then
        echo "::error::STALE WAIVER $file: $tier leg now agrees (trigger '$req' declared) — delete the waiver for issue #$(waiver_for "$file" "$tier") in this PR"
        fail=1
      fi
    else
      issue="$(waiver_for "$file" "$tier")"
      if [ -n "$issue" ]; then
        SEEN_WAIVER["$file|$tier"]=1
        waived=$((waived + 1))
        echo "::warning::WAIVED $file: $tier tier requires trigger '$req', not declared — disagreement recorded against issue #$issue (Phase 2-3 of #7089, awaiting Drew's ruling)"
      else
        echo "::error::DISAGREEMENT $file: $tier tier requires trigger '$req', not declared and no waiver records it — restore the trigger, strike the tier from ci-policy.md, or record a waiver naming its phase issue (#7089)"
        fail=1
      fi
    fi
  done
done

# A waiver for a leg the registry never requires is a defect too:
# it would record a disagreement that cannot exist.
for entry in "${WAIVERS[@]}"; do
  wfile="${entry%%|*}"
  wrest="${entry#*|}"
  wleg="${wrest%%|*}"
  if [ -z "${SEEN_WAIVER["$wfile|$wleg"]:-}" ]; then
    # Either the leg agrees (already reported stale above when the
    # workflow is registered) or the workflow/leg is unknown here.
    known=0
    for row in "${REGISTRY[@]}"; do
      if [ "${row%%|*}" = "$wfile" ]; then
        known=1
      fi
    done
    if [ "$known" -eq 0 ]; then
      echo "::error::UNKNOWN WAIVER $entry: workflow is not in the agreement registry"
      fail=1
    fi
  fi
done

echo "ci-policy-agreement: checked $checked workflows, $waived waived disagreement(s)"
if [ "$fail" -ne 0 ]; then
  echo "ci-policy-agreement: FAILED - disagreement(s) without a recorded waiver listed above"
  exit 1
fi
echo "ci-policy-agreement: OK"
