#!/usr/bin/env bash
# F11.10 regression guard (#1411, split tracked by #808).
#
# Fails when any production .rs file under pkgs/ (or apps/, when it
# exists) exceeds 2500 production lines. Production lines = all lines
# except test code, where test code is any item carrying #[cfg(test)]
# (an inline `mod tests { ... }` block, an out-of-line `mod tests;`
# declaration, a #[cfg(test)] fn/use, ...). This refines the #1078
# audit method ("lines before the first #[cfg(test)]"), which
# undercounts files that declare out-of-line test modules before
# their production code (e.g. pkgs/id/src/world_session/mod.rs
# declares its test modules at the top) — Codex finding on PR #4709.
# Test files are out of scope entirely: files under a tests/
# directory and files named tests.rs are skipped.
#
# Grandfather list: files that already exceeded the limit when the
# guard landed. Each entry names the issue that removes it; when the
# file drops under the limit, delete its entry in the same PR.
#
# Usage: check-production-line-limit.sh [ROOT]   (default: repo root)
set -euo pipefail

LIMIT=2500
ROOT="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"

# path -> removal issue
declare -A GRANDFATHERED=(
  # 5,325 production lines when the guard landed; split tracked by
  # https://github.com/developing-today/code/issues/4704
  ["pkgs/id/src/web/markdown.rs"]="https://github.com/developing-today/code/issues/4704"
  # 2,598 production lines; the F11.10 file itself, regrown. Split:
  # https://github.com/developing-today/code/issues/4715
  ["pkgs/id/src/cli.rs"]="https://github.com/developing-today/code/issues/4715"
  # 2,573 production lines; split tracked by
  # https://github.com/developing-today/code/issues/4716
  ["pkgs/id/src/web/routes.rs"]="https://github.com/developing-today/code/issues/4716"
  # 2,821 production lines; split tracked by
  # https://github.com/developing-today/code/issues/4717
  ["pkgs/id/src/world_session/mod.rs"]="https://github.com/developing-today/code/issues/4717"
)

# Count production lines of one file: every line that is not part of
# a #[cfg(test)] item. Brace counting finds the end of an inline
# test item; a cfg(test) item terminated by ";" (out-of-line module
# declaration, use, ...) excludes only its own lines.
production_lines() {
  awk '
    function braces(s,   t) { t = s; return gsub(/\{/, "{", t) - gsub(/\}/, "}", t) }
    skipdepth > 0 {
      skipdepth += braces($0)
      if (skipdepth <= 0) skipdepth = 0
      next
    }
    pending {
      pending = 0
      d = braces($0)
      if (d > 0) { skipdepth = d; next }
      if ($0 ~ /;[[:space:]]*$/) next
      pending = 1   # multi-line item header: keep waiting for { or ;
      next
    }
    /#\[cfg\(test\)\]/ { pending = 1; next }
    { count++ }
    END { print count + 0 }
  ' "$1"
}

fail=0
checked=0
for base in pkgs apps; do
  [ -d "$ROOT/$base" ] || continue
  while IFS= read -r -d "" file; do
    rel="${file#"$ROOT"/}"
    case "/$rel" in
      */tests/*) continue ;;
    esac
    case "$(basename "$file")" in
      tests.rs) continue ;;
    esac
    prod=$(production_lines "$file")
    checked=$((checked + 1))
    if [ "$prod" -gt "$LIMIT" ]; then
      if [ -n "${GRANDFATHERED[$rel]:-}" ]; then
        echo "::warning::GRANDFATHERED $rel: $prod production lines (limit $LIMIT) - split tracked by ${GRANDFATHERED[$rel]}"
      else
        echo "::error::OVER LIMIT $rel: $prod production lines (limit $LIMIT) - split the module or, if it predates this guard, grandfather it with a linked split issue (#1411)"
        fail=1
      fi
    fi
  done < <(find "$ROOT/$base" -name "*.rs" -type f -print0 | sort -z)
done

echo "production-line-guard: checked $checked production .rs files, limit $LIMIT"
if [ "$fail" -ne 0 ]; then
  echo "production-line-guard: FAILED - file(s) over the $LIMIT production-line limit listed above"
  exit 1
fi
echo "production-line-guard: OK"
