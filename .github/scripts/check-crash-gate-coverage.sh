#!/usr/bin/env bash
# Guard the crash-gate PR partition against silent coverage loss (#1635).
#
# PR #1310 split the aggregate `crash-gate` recipe into four PR checks
# partitioned by test-name filter (pkgs/id/justfile): crash-gate-sampled
# runs WITHOUT --include-ignored, and crash-gate-sweep / -control /
# -ingest each run exactly ONE `--include-ignored <substring>` filter.
# An #[ignore]d test in pkgs/id/tests/crash/ therefore gets PR coverage
# only if its name contains one of those substrings; the partition is
# complete today but nothing checked it, so a future ignored crash test
# matching no split would silently run in no PR check at all — only the
# nightly aggregate would ever execute it.
#
# This script makes the partition a checked invariant, at source level
# (no cargo build, so it can run in the id-check.yml guard job):
#
#   1. enumerate the #[ignore]d test names in pkgs/id/tests/crash/*.rs
#   2. subtract the named corpus-writer exclusions below — they rewrite
#      the checked-in corpus files and are run by hand, never in CI
#   3. extract the split filters from the justfile's crash-gate-sweep /
#      -control / -ingest recipes
#   4. fail, naming each test, when an ignored gate test contains none
#      of the filter substrings
#
# Parse drift is a failure too: finding no filters or no ignored tests
# means the inputs changed shape, and a guard that silently passes on
# empty input is the failure this script exists to prevent.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
crash_dir="$repo_root/pkgs/id/tests/crash"
justfile="$repo_root/pkgs/id/justfile"

excluded=(write_corpus write_corpus_identity)

# --- split filters from the justfile recipes -------------------------------
filters=()
for recipe in crash-gate-sweep crash-gate-control crash-gate-ingest; do
  filter="$(
    awk -v header="^${recipe}:" '
      $0 ~ header { in_recipe = 1; next }
      in_recipe && /--include-ignored/ {
        for (i = 1; i <= NF; i++) {
          if ($i == "--include-ignored") { print $(i + 1); exit }
        }
      }
    ' "$justfile"
  )"
  if [[ -z $filter ]]; then
    echo "::error::check-crash-gate-coverage: no --include-ignored filter found in the '$recipe' recipe in pkgs/id/justfile — the guard cannot verify the partition; update the recipe or this script."
    exit 1
  fi
  filters+=("$filter")
done

# --- #[ignore]d test names from the crash suite ----------------------------
# An ignored test is declared as a #[ignore ...] attribute followed
# (after any further attribute/doc lines and blank lines) by a fn item.
# The declaration shape varies — visibility modifiers (pub, pub(crate)),
# async fn, indentation inside a mod block, and #[ignore = "..."] reason
# strings wrapped across lines with a backslash continuation (#2963) —
# so enumeration is a small state machine over those shapes, not one
# exact line pattern. A #[ignore] that cannot be paired with a fn
# declaration is parse drift: the awk names it on stderr and exits
# non-zero rather than silently dropping the test from the enumeration.
# Enumeration is comment- and string-aware (Codex review on PR #4272,
# task 13381): #[ignore] mentions inside // or /* */ comments or
# string literals do not count, raw-string reasons are lexed as raw
# strings, and fn names are preserved whole -- a non-ASCII identifier
# is never truncated into a corpus-writer exclusion.
# Names repeat across files (main.rs and identity.rs carry copies);
# coverage is by name, so deduplicate.
if ! ignored_raw="$(
  awk '
    function fn_name(line,   s, k, ch, nm) {
      s = line
      sub(/^[[:space:]]*/, "", s)
      sub(/^pub[[:space:]]*\([^)]*\)[[:space:]]*/, "", s)
      sub(/^pub[[:space:]]+/, "", s)
      sub(/^async[[:space:]]+/, "", s)
      sub(/^unsafe[[:space:]]+/, "", s)
      sub(/^extern[[:space:]]+"[^"]*"[[:space:]]*/, "", s)
      sub(/^const[[:space:]]+/, "", s)
      if (s !~ /^fn[[:space:]]+/) return ""
      sub(/^fn[[:space:]]+/, "", s)
      sub(/^r#/, "", s)
      nm = ""
      for (k = 1; k <= length(s); k++) {
        ch = substr(s, k, 1)
        if (ch == " " || ch == "\t" || ch == "(" || ch == "<" || ch == "{") break
        nm = nm ch
      }
      return nm
    }
    # scan() is a small lexer for the shapes naive quote counting gets
    # wrong (Codex review on PR #4272): it walks one line tracking
    # normal strings, raw strings (r#"..."#, any hash count), char
    # literals, and // and /* */ comments. It records in code_text the
    # line code with string contents blanked and comments removed,
    # plus began_str / began_blk for the state at the line start, so
    # an #[ignore] mentioned inside a comment or a string is never
    # mistaken for an attribute, and a raw-string reason containing a
    # quote never desynchronises the string state.
    function scan(line,   i, c, n, h, j, closed) {
      sq = sprintf("%c", 39)
      began_str = (str != 0)
      began_blk = (blk > 0)
      code_text = ""
      n = length(line)
      i = 1
      while (i <= n) {
        c = substr(line, i, 1)
        if (blk > 0) {
          if (c == "/" && substr(line, i + 1, 1) == "*") { blk++; i += 2 }
          else if (c == "*" && substr(line, i + 1, 1) == "/") { blk--; i += 2 }
          else i++
          continue
        }
        if (str == 1) {
          if (c == "\\") { i += 2; continue }
          if (c == "\"") { str = 0; code_text = code_text "\"" }
          i++
          continue
        }
        if (str == 2) {
          if (c == "\"") {
            closed = 1
            for (h = 1; h <= rawhash; h++) if (substr(line, i + h, 1) != "#") { closed = 0; break }
            if (closed) { str = 0; code_text = code_text "\""; i += 1 + rawhash; continue }
          }
          i++
          continue
        }
        if (c == "/" && substr(line, i + 1, 1) == "/") break
        if (c == "/" && substr(line, i + 1, 1) == "*") { blk = 1; code_text = code_text " "; i += 2; continue }
        if (c == "\"") { str = 1; code_text = code_text "\""; i++; continue }
        if (c == "r" || (c == "b" && substr(line, i + 1, 1) == "r")) {
          j = i + (c == "b" ? 2 : 1)
          h = 0
          while (substr(line, j, 1) == "#") { h++; j++ }
          if (substr(line, j, 1) == "\"") { str = 2; rawhash = h; code_text = code_text "\""; i = j + 1; continue }
        }
        if (c == sq) {
          if (substr(line, i + 1, 1) == "\\") {
            j = i + 3
            while (j <= n && substr(line, j, 1) != sq) { if (substr(line, j, 1) == "\\") j++; j++ }
            code_text = code_text "0"; i = j + 1; continue
          }
          if (substr(line, i + 2, 1) == sq) { code_text = code_text "0"; i += 3; continue }
          code_text = code_text c; i++; continue
        }
        code_text = code_text c
        i++
      }
    }
    function drift(where) {
      printf "::error::check-crash-gate-coverage: #[ignore] at %s is not followed by a recognizable fn declaration — the enumeration in check-crash-gate-coverage.sh cannot see this test; fix the declaration shape or extend the enumeration.\n", where > "/dev/stderr"
      bad = 1
    }
    FNR == 1 && NR != 1 {
      if (pending) drift(prev_file " (end of file)")
      pending = 0; str = 0; blk = 0
    }
    {
      prev_file = FILENAME
      scan($0)
      if (began_str) next
      t = code_text
      sub(/^[[:space:]]*/, "", t)
      if (began_blk && t == "") next
      if (pending) {
        if (t == "") next
        if (t ~ /^#\[/) next
        name = fn_name(t)
        if (name != "") {
          print name
          pending = 0
          next
        }
        drift(FILENAME ":" FNR)
        pending = 0
        next
      }
      if (t ~ /#\[ignore([^A-Za-z0-9_]|$)/) pending = 1
    }
    END {
      if (pending) drift(FILENAME " (end of file)")
      if (bad) exit 1
    }
  ' "$crash_dir"/*.rs
)"; then
  echo "::error::check-crash-gate-coverage: enumeration of #[ignore]d tests failed (see above) — refusing to assert coverage it did not check."
  exit 1
fi
ignored_tests="$(printf '%s\n' "$ignored_raw" | sort -u)"

if [[ -z $ignored_tests ]]; then
  echo "::error::check-crash-gate-coverage: no #[ignore]d tests found in pkgs/id/tests/crash/*.rs — the suite changed shape; update this guard."
  exit 1
fi

# --- coverage ----------------------------------------------------------------
uncovered=()
covered=0
while IFS= read -r name; do
  skip=0
  for ex in "${excluded[@]}"; do
    if [[ $name == "$ex" ]]; then
      skip=1
      break
    fi
  done
  if [[ $skip -eq 1 ]]; then
    continue
  fi
  hit=0
  for filter in "${filters[@]}"; do
    if [[ $name == *"$filter"* ]]; then
      hit=1
      break
    fi
  done
  if [[ $hit -eq 1 ]]; then
    covered=$((covered + 1))
  else
    uncovered+=("$name")
  fi
done <<<"$ignored_tests"

if [[ ${#uncovered[@]} -ne 0 ]]; then
  for name in "${uncovered[@]}"; do
    echo "::error::#[ignore]d crash test '$name' matches none of the crash-gate PR split filters (${filters[*]}) — it would run in no PR check. Rename it to contain a split filter, extend a split in pkgs/id/justfile, or add it to the exclusion list in check-crash-gate-coverage.sh if it is a corpus writer."
  done
  exit 1
fi

echo "crash-gate partition covers all $covered #[ignore]d gate tests (${#excluded[@]} corpus writers excluded: ${excluded[*]})."
