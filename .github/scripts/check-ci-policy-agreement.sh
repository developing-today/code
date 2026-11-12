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
  # Empty as of 2026-11-11: Drew ruled re-arm on #7089 and both
  # Phase 2-3 disagreements landed in the same PR - the
  # opencode-config-guard main leg (#6953, push restored; venue
  # re-enabled via the Actions API) and the production-line-guard
  # PR leg (#6959, pull_request restored alongside push to main).
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
    inblock && /^  ["\047]?[a-z_]+["\047]?[[:space:]]*:/ {
      key = $0
      sub(/^  /, "", key)
      sub(/:.*/, "", key)
      gsub(/["\047]/, "", key)
      sub(/[[:space:]]+$/, "", key)
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
    function hexval(str,   v, i, d) {
      v = 0
      for (i = 1; i <= length(str); i++) {
        d = index("0123456789abcdef", tolower(substr(str, i, 1))) - 1
        if (d < 0) return -1
        v = v * 16 + d
      }
      return v
    }
    function codepoint_char(cp,   b1, b2, b3, b4) {
      if (cp < 128) return sprintf("%c", cp)
      if (cp < 2048) return sprintf("%c%c", 192 + int(cp / 64), 128 + (cp % 64))
      if (cp < 65536) return sprintf("%c%c%c", 224 + int(cp / 4096), 128 + (int(cp / 64) % 64), 128 + (cp % 64))
      return sprintf("%c%c%c%c", 240 + int(cp / 262144), 128 + (int(cp / 4096) % 64), 128 + (int(cp / 64) % 64), 128 + (cp % 64))
    }
    function decode_dq(str,   out, i, c, n, dn, cp) {
      # Decode the escapes of a YAML double-quoted scalar: the simple
      # escapes and the \xXX / \uXXXX / \UXXXXXXXX forms. An
      # unrecognized or truncated escape keeps its backslash;
      # decoding never invents characters.
      out = ""
      for (i = 1; i <= length(str); i++) {
        c = substr(str, i, 1)
        if (c != "\\") { out = out c; continue }
        if (i == length(str)) { out = out "\\"; break }
        n = substr(str, i + 1, 1)
        if (n == "0") { out = out sprintf("%c", 0); i++ }
        else if (n == "a") { out = out sprintf("%c", 7); i++ }
        else if (n == "b") { out = out sprintf("%c", 8); i++ }
        else if (n == "t") { out = out "\t"; i++ }
        else if (n == "n") { out = out "\n"; i++ }
        else if (n == "v") { out = out sprintf("%c", 11); i++ }
        else if (n == "f") { out = out sprintf("%c", 12); i++ }
        else if (n == "r") { out = out "\r"; i++ }
        else if (n == "e") { out = out sprintf("%c", 27); i++ }
        else if (n == " ") { out = out " "; i++ }
        else if (n == "\"") { out = out "\""; i++ }
        else if (n == "/") { out = out "/"; i++ }
        else if (n == "\\") { out = out "\\"; i++ }
        else if (n == "N") { out = out codepoint_char(133); i++ }
        else if (n == "_") { out = out codepoint_char(160); i++ }
        else if (n == "L") { out = out codepoint_char(8232); i++ }
        else if (n == "P") { out = out codepoint_char(8233); i++ }
        else if (n == "x" || n == "u" || n == "U") {
          dn = (n == "x") ? 2 : (n == "u") ? 4 : 8
          if (i + 1 + dn <= length(str) && (cp = hexval(substr(str, i + 2, dn))) >= 0) { out = out codepoint_char(cp); i += 1 + dn }
          else out = out "\\"
        }
        else out = out "\\"
      }
      return out
    }
    function clean(tok) {
      sub(/^[[:space:]]+/, "", tok)
      sub(/[[:space:]]+$/, "", tok)
      if (length(tok) >= 2) {
        if (substr(tok, 1, 1) == "\047" && substr(tok, length(tok), 1) == "\047") { tok = substr(tok, 2, length(tok) - 2); gsub(/\047\047/, "\047", tok) }
        else if (substr(tok, 1, 1) == "\"" && substr(tok, length(tok), 1) == "\"") tok = decode_dq(substr(tok, 2, length(tok) - 2))
      }
      return tok
    }
    function add_tokens(str,   i, c, q, tok, seg, out) {
      sub(/^[[:space:]]*\[/, "", str)
      sub(/\][[:space:]]*$/, "", str)
      out = ""
      tok = ""
      seg = ""
      q = ""
      for (i = 1; i <= length(str); i++) {
        c = substr(str, i, 1)
        if (q == "\"") {
          if (c == "\"") { tok = tok decode_dq(seg); seg = ""; q = "" }
          else if (c == "\\" && i < length(str)) { seg = seg c substr(str, i + 1, 1); i++ }
          else seg = seg c
        } else if (q != "") {
          # Single-quoted scalar: a doubled '' is one literal
          # apostrophe (Codex P2, PR #8513 round 16); dropping
          # both rewrites an apostrophe pattern as plain main.
          if (c == q && substr(str, i + 1, 1) == q) { tok = tok q; i++ }
          else if (c == q) q = ""
          else tok = tok c
        } else if (c == "\047" || c == "\"") {
          q = c
        } else if (c ~ /[[:space:],]/) {
          if (tok != "") { out = out " " tok; tok = "" }
        } else {
          tok = tok c
        }
      }
      if (q == "\"") tok = tok decode_dq(seg)
      if (tok != "") out = out " " tok
      return out
    }
    # Quote discipline for the structural scanners (Codex P2, PR
    # #8513 round 12): inside a double-quoted scalar a backslash
    # escapes the next character, so \" does not end the scalar;
    # inside a single-quoted scalar a doubled '' is an escaped
    # quote, not the end. The scalar decoder (decode_dq) already
    # honors the escapes — the scanners that decide where scalars
    # and structures end must honor them too, or a delimiter inside
    # a quoted scalar is mistaken for structure.
    # A YAML comment starts at an unquoted # that begins the line
    # or follows whitespace (Codex P2, PR #8513 round 13): braces
    # and brackets inside comments are not structure, so every
    # structural scanner stops there.
    function unquoted_close(str,   j, c, q) {
      q = ""
      for (j = 1; j <= length(str); j++) {
        c = substr(str, j, 1)
        if (q == "\"") {
          if (c == "\\") j++
          else if (c == "\"") q = ""
        } else if (q == "\047") {
          if (c == "\047" && substr(str, j + 1, 1) == "\047") j++
          else if (c == "\047") q = ""
        }
        else if (c == "\047" || c == "\"") q = c
        else if (c == "#" && (j == 1 || substr(str, j - 1, 1) ~ /[[:space:]]/)) return 0
        else if (c == "]") return j
      }
      return 0
    }
    # strip_comment: the text of a line up to (not including) an
    # unquoted comment start, with the same quote discipline as
    # unquoted_close. Joining physical lines into one logical line
    # (inline_collect) must strip each line first (Codex P2, PR
    # #8513 round 14): flattening first puts a closing bracket
    # that follows a comment inside that comment, and the filter
    # it closes is silently discarded. Every filter-value path
    # strips through this helper, never a quote-naive substitution
    # (Codex P2, round 15): a # inside a quoted pattern is data,
    # not a comment, and truncating there discards the rest of
    # the filter.
    function strip_comment(str,   j, c, q) {
      q = ""
      for (j = 1; j <= length(str); j++) {
        c = substr(str, j, 1)
        if (q == "\"") {
          if (c == "\\") j++
          else if (c == "\"") q = ""
        } else if (q == "\047") {
          if (c == "\047" && substr(str, j + 1, 1) == "\047") j++
          else if (c == "\047") q = ""
        }
        else if (c == "\047" || c == "\"") q = c
        else if (c == "#" && (j == 1 || substr(str, j - 1, 1) ~ /[[:space:]]/)) return substr(str, 1, j - 1)
      }
      return str
    }
    function flow_unclosed(str,   j, c, q, open) {
      q = ""
      open = 0
      for (j = 1; j <= length(str); j++) {
        c = substr(str, j, 1)
        if (q == "\"") {
          if (c == "\\") j++
          else if (c == "\"") q = ""
        } else if (q == "\047") {
          if (c == "\047" && substr(str, j + 1, 1) == "\047") j++
          else if (c == "\047") q = ""
        }
        else if (c == "\047" || c == "\"") q = c
        else if (c == "#" && (j == 1 || substr(str, j - 1, 1) ~ /[[:space:]]/)) return open
        else if (c == "[") open = 1
        else if (c == "]" && open) open = 0
      }
      return open
    }
    function has_unquoted_brace(str,   j, c, q) {
      q = ""
      for (j = 1; j <= length(str); j++) {
        c = substr(str, j, 1)
        if (q == "\"") {
          if (c == "\\") j++
          else if (c == "\"") q = ""
        } else if (q == "\047") {
          if (c == "\047" && substr(str, j + 1, 1) == "\047") j++
          else if (c == "\047") q = ""
        }
        else if (c == "\047" || c == "\"") q = c
        else if (c == "#" && (j == 1 || substr(str, j - 1, 1) ~ /[[:space:]]/)) return 0
        else if (c == "}") return 1
      }
      return 0
    }
    function flow_key_colon(str, key,   i, j, c, q, prev, word) {
      q = ""
      for (i = 1; i <= length(str); i++) {
        c = substr(str, i, 1)
        if (q == "\"") {
          if (c == "\\") i++
          else if (c == "\"") q = ""
          continue
        }
        if (q == "\047") {
          if (c == "\047" && substr(str, i + 1, 1) == "\047") i++
          else if (c == "\047") q = ""
          continue
        }
        if (c == "#" && (i == 1 || substr(str, i - 1, 1) ~ /[[:space:]]/)) return 0
        if (c == "\"" || c == "\047") {
          q = c
          j = i + 1
          word = ""
          while (j <= length(str)) {
            c = substr(str, j, 1)
            if (q == "\"" && c == "\\") { j++; word = word substr(str, j, 1); j++; continue }
            if (c == q) {
              if (q == "\047" && substr(str, j + 1, 1) == q) { word = word q; j += 2; continue }
              break
            }
            word = word c
            j++
          }
          if (word == key && j <= length(str)) {
            j++
            while (substr(str, j, 1) ~ /[[:space:]]/) j++
            if (substr(str, j, 1) == ":") return j
          }
          i = j
          q = ""
          continue
        }
        prev = (i == 1) ? "" : substr(str, i - 1, 1)
        if ((prev == "" || prev == "{" || prev == "," || prev ~ /[[:space:]]/) && substr(str, i, length(key)) == key) {
          j = i + length(key)
          while (substr(str, j, 1) ~ /[[:space:]]/) j++
          if (substr(str, j, 1) == ":") return j
        }
      }
      return 0
    }
    function flow_span(str, key,   colon, start, seg, i) {
      colon = flow_key_colon(str, key)
      if (colon == 0) return ""
      start = colon + 1
      while (substr(str, start, 1) ~ /[[:space:]]/) start++
      if (substr(str, start, 1) != "[") return ""
      seg = substr(str, start)
      i = unquoted_close(seg)
      if (i == 0) return ""
      return substr(seg, 1, i)
    }
    function resolve_sequence_value(val,   name) {
      sub(/^[[:space:]]+/, "", val)
      sub(/[[:space:]]+$/, "", val)
      if (val !~ /^\*[A-Za-z0-9_-]+$/) return val
      name = substr(val, 2)
      if (name in sequence_anchor) return sequence_anchor[name]
      alias_unknown = 1
      return ""
    }
    function apply_inline_filters(str,   seg, colon) {
      seg = flow_span(str, "branches-ignore")
      if (seg == "") {
        colon = flow_key_colon(str, "branches-ignore")
        if (colon) seg = resolve_sequence_value(substr(str, colon + 1))
      }
      if (seg != "") {
        seen_branch_filter = 1
        ignore_list = ignore_list add_tokens(seg)
      }
      seg = flow_span(str, "branches")
      if (seg == "") {
        colon = flow_key_colon(str, "branches")
        if (colon) seg = resolve_sequence_value(substr(str, colon + 1))
      }
      if (seg != "") {
        seen_branches = 1
        seen_branch_filter = 1
        branches_list = branches_list add_tokens(seg)
      }
      if (flow_key_colon(str, "tags-ignore") || flow_key_colon(str, "tags")) seen_tags = 1
    }
    function resolve_push_alias(name) {
      # An aliased push inherits the anchored mapping filters
      # (Codex P2, PR #8513 round 13). An alias that resolves to
      # no recorded filter is unsupported syntax: mark it so the
      # verdict never certifies it as an unfiltered push.
      if (name in anchor_body) {
        apply_inline_filters(anchor_body[name])
      } else if ((name in anchor_mapping_seen) || (name in anchor_seen_branches) || (name in anchor_seen_ignore) || (name in anchor_seen_tags)) {
        if (name in anchor_seen_ignore) {
          seen_branch_filter = 1
          ignore_list = ignore_list anchor_ignore[name]
        }
        if (name in anchor_seen_branches) {
          seen_branches = 1
          seen_branch_filter = 1
          branches_list = branches_list anchor_branches[name]
        }
        if (name in anchor_seen_tags) seen_tags = 1
      } else {
        alias_unknown = 1
      }
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
    inblock && /^  ["\047]?[a-z_]+["\047]?[[:space:]]*:/ {
      key = $0
      sub(/^  /, "", key)
      sub(/:.*/, "", key)
      gsub(/["\047]/, "", key)
      sub(/[[:space:]]+$/, "", key)
      inpush = (key == "push")
      if (!inpush) child_indent = 0
      rest = $0
      sub(/^  ["\047]?[a-z_]+["\047]?[[:space:]]*:[[:space:]]*/, "", rest)
      # YAML anchors (Codex P2, PR #8513 round 13): a mapping
      # value may carry &name and be referenced later as *name.
      # Record anchored bodies so an aliased push resolves to
      # its filters instead of looking like an unfiltered push.
      anchor_here = ""
      if (rest ~ /^&[A-Za-z0-9_-]+([[:space:]]|$)/) {
        anchor_here = rest
        sub(/^&/, "", anchor_here)
        sub(/[[:space:]].*/, "", anchor_here)
        sub(/^&[A-Za-z0-9_-]+[[:space:]]*/, "", rest)
        if (rest ~ /^\{/) anchor_body[anchor_here] = rest
      }
      cur_anchor = anchor_here
      anchor_child_indent = 0
      anchor_flow_key = ""
      anchor_seq_indent = 0
      if (inpush) {
        seen_push = 1
        if (rest ~ /^\*[A-Za-z0-9_-]+/) {
          alias = substr(rest, 2)
          sub(/[^A-Za-z0-9_-].*/, "", alias)
          resolve_push_alias(alias)
        } else if (rest ~ /^\{/) {
          if (has_unquoted_brace(rest)) apply_inline_filters(rest)
          else { inline_collect = 1; inline_buf = strip_comment(rest); inline_anchor = anchor_here }
        }
      }
      subkey = ""
      next
    }
    inblock && !inpush && cur_anchor == "" && /^ +["\047]?[a-z_-]+["\047]?[[:space:]]*:/ {
      line = $0
      match(line, /^ +/)
      sequence_key_indent = RLENGTH
      keyval = substr(line, sequence_key_indent + 1)
      sub(/:.*/, "", keyval)
      gsub(/["\047]/, "", keyval)
      val = substr(line, sequence_key_indent + 1)
      sub(/^[^:]*:[[:space:]]*/, "", val)
      sequence_anchor_pending = ""
      if (val ~ /^&[A-Za-z0-9_-]+([[:space:]]|$)/) {
        sequence_anchor_pending = val
        sub(/^&/, "", sequence_anchor_pending)
        sub(/[[:space:]].*/, "", sequence_anchor_pending)
        sub(/^&[A-Za-z0-9_-]+[[:space:]]*/, "", val)
        if (val ~ /^\[/) {
          sequence_anchor[sequence_anchor_pending] = add_tokens(val)
          sequence_anchor_pending = ""
        }
      }
      next
    }
    inblock && !inpush && cur_anchor == "" && sequence_anchor_pending != "" && /^ +-[[:space:]]/ {
      item = $0
      match(item, /^ +/)
      if (RLENGTH > sequence_key_indent) {
        sub(/^ +-[[:space:]]*/, "", item)
        sequence_anchor[sequence_anchor_pending] = sequence_anchor[sequence_anchor_pending] " " clean(item)
        next
      }
      sequence_anchor_pending = ""
    }
    inblock && /^[^[:space:]#]/ { inblock = 0; inpush = 0; child_indent = 0; cur_anchor = "" }
    inline_collect {
      if ($0 ~ /^[^[:space:]#]/) {
        apply_inline_filters(inline_buf)
        if (inline_anchor != "") { anchor_body[inline_anchor] = inline_buf; inline_anchor = "" }
        inline_collect = 0
      } else {
        inline_line = strip_comment($0)
        inline_buf = inline_buf " " inline_line
        if (has_unquoted_brace(inline_line)) {
          apply_inline_filters(inline_buf)
          if (inline_anchor != "") { anchor_body[inline_anchor] = inline_buf; inline_anchor = "" }
          inline_collect = 0
        }
        next
      }
    }

    !inpush && cur_anchor != "" && /^ +["\047]?[a-z-]+["\047]?[[:space:]]*:/ {
      line = $0
      match(line, /^ +/)
      ind = RLENGTH
      if (anchor_child_indent == 0) anchor_child_indent = ind
      if (ind != anchor_child_indent) next
      anchor_seq_key_indent = ind
      anchor_seq_indent = 0
      subkey_a = substr(line, ind + 1)
      sub(/:.*/, "", subkey_a)
      gsub(/["\047]/, "", subkey_a)
      sub(/[[:space:]]+$/, "", subkey_a)
      anchor_mapping_seen[cur_anchor] = 1
      val = substr(line, ind + 1)
      sub(/^[^:]*:[[:space:]]*/, "", val)
      if (subkey_a == "branches" || subkey_a == "branches-ignore" || subkey_a == "tags" || subkey_a == "tags-ignore") val = strip_comment(val)
      if ((subkey_a == "branches" || subkey_a == "branches-ignore") && flow_unclosed(val)) anchor_flow_key = subkey_a
      if (subkey_a == "branches") {
        anchor_seen_branches[cur_anchor] = 1
        anchor_branches[cur_anchor] = add_tokens(val)
      } else if (subkey_a == "branches-ignore") {
        anchor_seen_ignore[cur_anchor] = 1
        anchor_ignore[cur_anchor] = add_tokens(val)
      } else if (subkey_a == "tags" || subkey_a == "tags-ignore") {
        anchor_seen_tags[cur_anchor] = 1
      }
      next
    }
    !inpush && cur_anchor != "" && anchor_flow_key != "" {
      seg = strip_comment($0)
      k = unquoted_close(seg)
      if (k > 0) {
        seg = substr(seg, 1, k - 1)
        if (anchor_flow_key == "branches") anchor_branches[cur_anchor] = anchor_branches[cur_anchor] add_tokens(seg)
        else if (anchor_flow_key == "branches-ignore") anchor_ignore[cur_anchor] = anchor_ignore[cur_anchor] add_tokens(seg)
        anchor_flow_key = ""
      } else if (anchor_flow_key == "branches") {
        anchor_branches[cur_anchor] = anchor_branches[cur_anchor] add_tokens(seg)
      } else if (anchor_flow_key == "branches-ignore") {
        anchor_ignore[cur_anchor] = anchor_ignore[cur_anchor] add_tokens(seg)
      }
      next
    }
    !inpush && cur_anchor != "" && /^ +-[[:space:]]/ {
      item = $0
      match(item, /^ +/)
      ind = RLENGTH
      if (anchor_seq_indent == 0) {
        if (ind < anchor_seq_key_indent) next
        anchor_seq_indent = ind
      }
      if (ind != anchor_seq_indent) next
      sub(/^ +-[[:space:]]*/, "", item)
      if (subkey_a == "branches" || subkey_a == "branches-ignore" || subkey_a == "tags" || subkey_a == "tags-ignore") item = strip_comment(item)
      if (subkey_a == "branches") {
        anchor_seen_branches[cur_anchor] = 1
        anchor_branches[cur_anchor] = anchor_branches[cur_anchor] " " clean(item)
      } else if (subkey_a == "branches-ignore") {
        anchor_seen_ignore[cur_anchor] = 1
        anchor_ignore[cur_anchor] = anchor_ignore[cur_anchor] " " clean(item)
      } else if (subkey_a == "tags" || subkey_a == "tags-ignore") {
        anchor_seen_tags[cur_anchor] = 1
      }
      next
    }
    inpush && /^ +["\047]?[a-z-]+["\047]?[[:space:]]*:/ {
      line = $0
      match(line, /^ +/)
      ind = RLENGTH
      if (child_indent == 0) child_indent = ind
      if (ind != child_indent) next
      seq_key_indent = ind
      seq_indent = 0
      subkey = substr(line, ind + 1)
      sub(/:.*/, "", subkey)
      gsub(/["\047]/, "", subkey)
      sub(/[[:space:]]+$/, "", subkey)
      val = substr(line, ind + 1)
      sub(/^[^:]*:[[:space:]]*/, "", val)
      if (subkey == "branches" || subkey == "branches-ignore" || subkey == "tags" || subkey == "tags-ignore") val = strip_comment(val)
      if ((subkey == "branches" || subkey == "branches-ignore") && flow_unclosed(val)) flow_key = subkey
      if (subkey == "branches" || subkey == "branches-ignore") val = resolve_sequence_value(val)
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
      seg = strip_comment($0)
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
      if (seq_indent == 0) {
        if (ind < seq_key_indent) next
        seq_indent = ind
      }
      if (ind != seq_indent) next
      sub(/^ +-[[:space:]]*/, "", item)
      if (subkey == "branches" || subkey == "branches-ignore" || subkey == "tags" || subkey == "tags-ignore") item = strip_comment(item)
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
      if (alias_unknown) {
        print "no"
      } else if (list_matches_main(ignore_list)) {
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
