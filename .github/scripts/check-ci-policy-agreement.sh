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
    inblock && /^  ["\047]?[a-z_]+["\047]?:/ {
      key = $0
      sub(/^  /, "", key)
      sub(/:.*/, "", key)
      gsub(/["\047]/, "", key)
      print key
      next
    }
    inblock && /^[^[:space:]#]/ { inblock = 0 }
  ' "$file"
}

# push_targets_main <workflow-file>: print "yes" when the workflow
# declares a push trigger that runs on main, "no" when it declares a
# push trigger that cannot run on main, and "absent" when it declares
# no push trigger at all. The main tier's contract is a push on
# branches: [main] (ci-policy.md), so the trigger *name* is not
# enough (#7950): a bare/unfiltered push counts; an explicit
# branches: filter must match main as a pattern (an ordered list,
# so a later '!main' negation unmatches it); branches-ignore: with
# any pattern matching main, a branches: list whose patterns do not
# match main, and a tags-only push (tags: or tags-ignore:, with no
# branch filter) never run on main and do not satisfy the leg.
# Trailing YAML comments on filter lines are not filter entries.
# Path gates do not affect the verdict ("path gates as before").
push_targets_main() {
  local file="$1"
  awk '
    function clean(tok) {
      sub(/^[[:space:]]+/, "", tok)
      sub(/[[:space:]]+$/, "", tok)
      if (length(tok) >= 2) {
        if ((substr(tok, 1, 1) == "\047" && substr(tok, length(tok), 1) == "\047") || (substr(tok, 1, 1) == "\"" && substr(tok, length(tok), 1) == "\"")) tok = substr(tok, 2, length(tok) - 2)
      }
      return tok
    }
    function add_tokens(str,   i, c, q, tok, out) {
      sub(/^[[:space:]]*\[/, "", str)
      sub(/\][[:space:]]*$/, "", str)
      out = ""
      tok = ""
      q = ""
      for (i = 1; i <= length(str); i++) {
        c = substr(str, i, 1)
        if (q != "") {
          if (c == q) q = ""
          else tok = tok c
        } else if (c == "\047" || c == "\"") {
          q = c
        } else if (c ~ /[[:space:],]/) {
          if (tok != "") { out = out " " tok; tok = "" }
        } else {
          tok = tok c
        }
      }
      if (tok != "") out = out " " tok
      return out
    }
    function unquoted_close(str,   j, c, q) {
      q = ""
      for (j = 1; j <= length(str); j++) {
        c = substr(str, j, 1)
        if (q != "") { if (c == q) q = "" }
        else if (c == "\047" || c == "\"") q = c
        else if (c == "]") return j
      }
      return 0
    }
    function flow_unclosed(str,   j, c, q, open) {
      q = ""
      open = 0
      for (j = 1; j <= length(str); j++) {
        c = substr(str, j, 1)
        if (q != "") { if (c == q) q = "" }
        else if (c == "\047" || c == "\"") q = c
        else if (c == "[") open = 1
        else if (c == "]" && open) open = 0
      }
      return open
    }
    function flow_span(str, key,   i, start, seg, from, prev) {
      from = 1
      while ((i = index(substr(str, from), key)) > 0) {
        i = from + i - 1
        prev = (i == 1) ? "" : substr(str, i - 1, 1)
        if (prev == "" || prev == "{" || prev == "," || prev == " ") break
        from = i + length(key)
        i = 0
      }
      if (i == 0) return ""
      start = index(substr(str, i), "[")
      if (start == 0) return ""
      start = i + start - 1
      seg = substr(str, start)
      i = unquoted_close(seg)
      if (i == 0) return ""
      return substr(seg, 1, i)
    }
    function apply_inline_filters(str,   seg) {
      seg = flow_span(str, "branches-ignore:")
      if (seg != "") {
        seen_branch_filter = 1
        ignore_list = ignore_list add_tokens(seg)
      }
      seg = flow_span(str, "branches:")
      if (seg != "") {
        seen_branches = 1
        seen_branch_filter = 1
        branches_list = branches_list add_tokens(seg)
      }
      if (str ~ /(^|[{,][[:space:]]*)["\047]?tags-ignore["\047]?[[:space:]]*:/) seen_tags = 1
      if (str ~ /(^|[{,][[:space:]]*)["\047]?tags["\047]?[[:space:]]*:/) seen_tags = 1
    }
    function glob_regex(pat,   out, i, c, cls) {
      # GitHub filter-pattern syntax (workflow-syntax cheat sheet),
      # not shell glob: ? and + quantify the preceding character,
      # * matches within a path segment, ** across segments.
      out = "^"
      for (i = 1; i <= length(pat); i++) {
        c = substr(pat, i, 1)
        if (c == "*") {
          if (substr(pat, i + 1, 1) == "*") {
            if (substr(pat, i + 2, 1) == "/") { out = out "(.*/)?"; i += 2 }
            else { out = out ".*"; i++ }
          }
          else out = out "[^/]*"
        }
        else if (c == "?" || c == "+") out = out c
        else if (c == "[") {
          cls = "["
          i++
          if (substr(pat, i, 1) == "!") { cls = cls "^"; i++ }
          for (; i <= length(pat); i++) {
            cls = cls substr(pat, i, 1)
            if (substr(pat, i, 1) == "]") break
          }
          out = out cls
        }
        else if (c ~ /[.^$()|{}]/) out = out "\\" c
        else out = out c
      }
      return out "$"
    }
    function list_matches_main(list,   n, parts, i, tok) {
      n = split(list, parts, /[[:space:]]+/)
      for (i = 1; i <= n; i++) {
        tok = parts[i]
        if (tok != "" && ("main" ~ glob_regex(tok))) return 1
      }
      return 0
    }
    function branches_allow_main(list,   n, parts, i, tok, allow) {
      allow = 0
      n = split(list, parts, /[[:space:]]+/)
      for (i = 1; i <= n; i++) {
        tok = parts[i]
        if (tok == "") continue
        if (substr(tok, 1, 1) == "!") {
          if ("main" ~ glob_regex(substr(tok, 2))) allow = 0
        } else if ("main" ~ glob_regex(tok)) {
          allow = 1
        }
      }
      return allow
    }
    /^"?on"?:[[:space:]]*\[/ {
      line = $0
      sub(/^"?on"?:[[:space:]]*\[/, "", line)
      sub(/\].*/, "", line)
      if (line ~ /(^|,)[[:space:]]*push[[:space:]]*(,|$)/) verdict = "yes"
      else verdict = "absent"
      done = 1
      exit
    }
    /^"?on"?:[[:space:]]*[a-z_]/ {
      line = $0
      sub(/^"?on"?:[[:space:]]*/, "", line)
      sub(/[[:space:]].*/, "", line)
      verdict = (line == "push") ? "yes" : "absent"
      done = 1
      exit
    }
    /^"?on"?:[[:space:]]*$/ { inblock = 1; next }
    inblock && /^  ["\047]?[a-z_]+["\047]?:/ {
      key = $0
      sub(/^  /, "", key)
      sub(/:.*/, "", key)
      gsub(/["\047]/, "", key)
      inpush = (key == "push")
      if (!inpush) child_indent = 0
      if (inpush) {
        seen_push = 1
        rest = $0
        sub(/^  ["\047]?[a-z_]+["\047]?:[[:space:]]*/, "", rest)
        if (rest ~ /^\{/) {
          if (rest ~ /\}/) apply_inline_filters(rest)
          else { inline_collect = 1; inline_buf = rest }
        }
      }
      subkey = ""
      next
    }
    inblock && /^[^[:space:]#]/ { inblock = 0; inpush = 0; child_indent = 0 }
    inline_collect {
      if ($0 ~ /^[^[:space:]#]/) {
        apply_inline_filters(inline_buf)
        inline_collect = 0
      } else {
        inline_buf = inline_buf " " $0
        if ($0 ~ /\}/) {
          apply_inline_filters(inline_buf)
          inline_collect = 0
        }
        next
      }
    }

    inpush && /^ +["\047]?[a-z-]+["\047]?:/ {
      line = $0
      match(line, /^ +/)
      ind = RLENGTH
      if (child_indent == 0) child_indent = ind
      if (ind != child_indent) next
      subkey = substr(line, ind + 1)
      sub(/:.*/, "", subkey)
      gsub(/["\047]/, "", subkey)
      val = substr(line, ind + 1)
      sub(/^[^:]*:[[:space:]]*/, "", val)
      if (subkey == "branches" || subkey == "branches-ignore" || subkey == "tags" || subkey == "tags-ignore") sub(/[[:space:]]#.*/, "", val)
      if ((subkey == "branches" || subkey == "branches-ignore") && flow_unclosed(val)) flow_key = subkey
      if (subkey == "branches") {
        seen_branches = 1
        seen_branch_filter = 1
        branches_list = add_tokens(val)
      } else if (subkey == "branches-ignore") {
        seen_branch_filter = 1
        ignore_list = add_tokens(val)
      } else if (subkey == "tags" || subkey == "tags-ignore") {
        seen_tags = 1
      }
      next
    }
    inpush && flow_key != "" {
      seg = $0
      sub(/[[:space:]]#.*/, "", seg)
      k = unquoted_close(seg)
      if (k > 0) {
        seg = substr(seg, 1, k - 1)
        if (flow_key == "branches") branches_list = branches_list add_tokens(seg)
        else if (flow_key == "branches-ignore") ignore_list = ignore_list add_tokens(seg)
        flow_key = ""
      } else if (flow_key == "branches") {
        branches_list = branches_list add_tokens(seg)
      } else if (flow_key == "branches-ignore") {
        ignore_list = ignore_list add_tokens(seg)
      }
      next
    }
    inpush && /^ +-[[:space:]]/ {
      item = $0
      match(item, /^ +/)
      ind = RLENGTH
      if (child_indent == 0 || (ind != child_indent && ind != child_indent + 2)) next
      sub(/^ +-[[:space:]]*/, "", item)
      if (subkey == "branches" || subkey == "branches-ignore" || subkey == "tags" || subkey == "tags-ignore") sub(/[[:space:]]#.*/, "", item)
      if (subkey == "branches") {
        seen_branches = 1
        seen_branch_filter = 1
        branches_list = branches_list " " clean(item)
      } else if (subkey == "branches-ignore") {
        seen_branch_filter = 1
        ignore_list = ignore_list " " clean(item)
      } else if (subkey == "tags" || subkey == "tags-ignore") {
        seen_tags = 1
      }
      next
    }
    END {
      if (done) {
        print verdict
        exit
      }
      if (!seen_push) {
        print "absent"
        exit
      }
      if (list_matches_main(ignore_list)) {
        print "no"
      } else if (seen_branches) {
        if (branches_allow_main(branches_list)) print "yes"
        else print "no"
      } else if (seen_branch_filter) {
        print "yes"
      } else if (seen_tags) {
        print "no"
      } else {
        print "yes"
      }
    }
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
    agrees=0
    mistargeted=0
    if [ "$tier" = "main" ]; then
      # The main leg agrees only when the push trigger targets main
      # (#7950): a mis-filtered push keeps the leg in disagreement,
      # so a waiver for it is recorded, never stale.
      case "$(push_targets_main "$wf")" in
        yes) agrees=1 ;;
        no) mistargeted=1 ;;
      esac
    elif printf '%s\n' "$triggers" | grep -qx "$req"; then
      agrees=1
    fi
    if [ "$agrees" -eq 1 ]; then
      if [ -n "$(waiver_for "$file" "$tier")" ]; then
        echo "::error::STALE WAIVER $file: $tier leg now agrees (trigger '$req' declared) — delete the waiver for issue #$(waiver_for "$file" "$tier") in this PR"
        fail=1
      fi
    else
      issue="$(waiver_for "$file" "$tier")"
      if [ -n "$issue" ]; then
        SEEN_WAIVER["$file|$tier"]=1
        waived=$((waived + 1))
        if [ "$mistargeted" -eq 1 ]; then
          echo "::warning::WAIVED $file: $tier tier requires a push trigger on branches: [main]; the declared push does not target main — disagreement recorded against issue #$issue (Phase 2-3 of #7089, awaiting Drew's ruling)"
        else
          echo "::warning::WAIVED $file: $tier tier requires trigger '$req', not declared — disagreement recorded against issue #$issue (Phase 2-3 of #7089, awaiting Drew's ruling)"
        fi
      else
        if [ "$mistargeted" -eq 1 ]; then
          echo "::error::DISAGREEMENT $file: $tier tier requires a push trigger on branches: [main]; the declared push does not target main and no waiver records it — retarget the push to main, strike the tier from ci-policy.md, or record a waiver naming its phase issue (#7089)"
        else
          echo "::error::DISAGREEMENT $file: $tier tier requires trigger '$req', not declared and no waiver records it — restore the trigger, strike the tier from ci-policy.md, or record a waiver naming its phase issue (#7089)"
        fi
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
