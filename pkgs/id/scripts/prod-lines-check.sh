#!/usr/bin/env bash
# Production module-size guard for F11.10 (issue #1411).
#
# Fails when any production Rust file under pkgs/id exceeds 2,500
# production lines, where production = lines before the first
# `#[cfg(test)]` line (the #1078 audit method). Test-only files stay
# out of scope: the package-root `tests/` tree (cargo integration
# tests), vendored third-party code, and the module-gated test modules
# in TEST_ONLY below.
#
# Grandfathered over-limit files live in grandfather_allowance() as
# path|allowance|reason rows. The allowance is a ceiling that must only
# shrink: any growth fails, and deleting the entry (once the split
# lands) makes the plain 2,500 limit apply.
#
# Test seams (used by prod-lines-check-test.sh, not in CI):
#   PROD_LINES_ROOT   scan this tree instead of pkgs/id
#   PROD_LINES_LIMIT  compare against this limit instead of 2,500
set -euo pipefail

pkg_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
root="${PROD_LINES_ROOT:-$pkg_dir}"
root="$(cd "$root" && pwd)"
limit="${PROD_LINES_LIMIT:-2500}"

# Grandfathered over-limit production files: path|allowance|reason.
# Allowance is lines of production code, not total lines.
grandfather_allowance() {
  case "$1" in
  src/cli.rs)
    # Baseline allowance 2026-11-08 at 2,817 production lines (Codex P1
    # baseline disposition on PR #7187: explicit allowances at exact
    # current counts so the guard lands as a ratchet). The F11.10 split
    # that shrinks it is tracked in #1438. Any growth fails.
    printf '2817|F11.10 split tracked in #1438 — allowance must only shrink'
    return 0
    ;;
  src/directory.rs)
    # Baseline allowance 2026-11-08 at 2,610 production lines (Codex P1
    # baseline disposition on PR #7187, as above); the F11.10 split
    # that shrinks it is tracked in #1438. Any growth fails.
    printf '2610|F11.10 split tracked in #1438 — allowance must only shrink'
    return 0
    ;;
  src/web/routes.rs)
    # Baseline allowance 2026-11-08 at 2,890 production lines (Codex P1
    # baseline disposition on PR #7187, as above); the F11.10 split
    # that shrinks it is tracked in #1438. Any growth fails.
    printf '2890|F11.10 split tracked in #1438 — allowance must only shrink'
    return 0
    ;;
  src/web/markdown.rs)
    # Grandfathered 2026-11-03 at 5,312 production lines; baseline
    # re-established 2026-11-08 at 6,232 production lines (growth landed
    # on main before the guard merged; Codex P1 baseline disposition on
    # PR #7187). The F11.10 split that shrinks it is tracked in #1438
    # (queue task 6599). Any growth fails.
    printf '6232|F11.10 split tracked in #1438 — allowance must only shrink'
    return 0
    ;;
  esac
  return 1
}

# Test-only production-tree files with no in-file #[cfg(test)] marker
# (module-gated via `#[cfg(test)] mod <name>;` in the parent module):
# path|reason.
test_only_reason() {
  case "$1" in
  src/encryption/tests.rs)
    printf 'module-gated test module (#[cfg(test)] mod tests in src/encryption/mod.rs)'
    return 0
    ;;
  src/world_session/tests.rs)
    printf 'module-gated test module (#[cfg(test)] mod tests in src/world_session/mod.rs)'
    return 0
    ;;
  esac
  return 1
}

is_skipped_dir() {
  case "$1" in
  tests/* | vendor/* | target/* | node_modules/* | .git/*) return 0 ;;
  esac
  return 1
}

prod_lines() {
  awk 'BEGIN { n = 0 } /^[[:space:]]*#\[cfg\(test\)\]$/ { exit } { n++ } END { print n }' "$1"
}

scanned=0
skipped_test_only=0
violations=0
max_file=""
max_lines=0
grandfather_hits=0

while IFS= read -r file; do
  rel="${file#"$root"/}"
  if is_skipped_dir "$rel"; then
    continue
  fi
  if reason="$(test_only_reason "$rel")"; then
    skipped_test_only=$((skipped_test_only + 1))
    echo "test-only: $rel ($reason)"
    continue
  fi
  count="$(prod_lines "$file")"
  scanned=$((scanned + 1))
  if [ "$count" -gt "$max_lines" ]; then
    max_lines="$count"
    max_file="$rel"
  fi
  if entry="$(grandfather_allowance "$rel")"; then
    grandfather_hits=$((grandfather_hits + 1))
    allowance="${entry%%|*}"
    greason="${entry#*|}"
    if [ "$count" -gt "$allowance" ]; then
      violations=$((violations + 1))
      echo "::error file=$rel::grandfathered file grew: $count production lines (allowance $allowance — $greason)"
    else
      echo "grandfathered: $rel at $count production lines (allowance $allowance — $greason)"
    fi
  elif [ "$count" -gt "$limit" ]; then
    violations=$((violations + 1))
    echo "::error file=$rel::$rel has $count production lines (limit $limit) — split the module per F11.10 (#1411)"
  fi
done < <(find "$root" -name '*.rs' -not -path '*/target/*' -not -path '*/node_modules/*' -not -path '*/.git/*' | sort)

if [ "$scanned" -eq 0 ]; then
  echo "::error::prod-lines-check found no .rs files under $root — the scan is broken, refusing to pass" >&2
  exit 2
fi

echo "prod-lines-check: $scanned production files scanned (limit $limit), largest $max_file at $max_lines lines, $skipped_test_only test-only skips, $grandfather_hits grandfathered"
if [ "$violations" -ne 0 ]; then
  echo "prod-lines-check: $violations file(s) over the limit" >&2
  exit 1
fi
echo "prod-lines-check passed"
