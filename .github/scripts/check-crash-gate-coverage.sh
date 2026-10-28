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
# An ignored test is declared as a #[ignore ...] attribute line followed
# (after any further attribute/doc lines) by `fn <name>(`. Names repeat
# across files (main.rs and identity.rs carry copies); coverage is by
# name, so deduplicate.
ignored_tests="$(
  awk '
    /#\[ignore/ { ignored = 1; next }
    ignored && /^#\[/ { next }
    ignored && /^\/\/\// { next }
    ignored && /^fn [A-Za-z0-9_]+/ {
      name = $2
      sub(/\(.*/, "", name)
      print name
      ignored = 0
      next
    }
    ignored { ignored = 0 }
  ' "$crash_dir"/*.rs | sort -u
)"

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
