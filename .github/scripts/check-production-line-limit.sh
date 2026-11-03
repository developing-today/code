#!/usr/bin/env bash
# F11.10 regression guard (#1411, split tracked by #808).
#
# Fails when any production .rs file under pkgs/ (or apps/, when it
# exists) exceeds 2500 production lines. Production lines = lines
# before the first #[cfg(test)] module, matching the #1078 audit
# method. Test code is out of scope: files under a tests/ directory
# and files named tests.rs (cfg(test) modules) are skipped.
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
)

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
    prod=$(awk "/#\[cfg\(test\)\]/{print NR-1; found=1; exit} END{if (!found) print NR}" "$file")
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
