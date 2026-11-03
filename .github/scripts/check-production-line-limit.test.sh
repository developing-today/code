#!/usr/bin/env bash
# Contract tests for check-production-line-limit.sh (#1411): the guard
# passes on a compliant tree (grandfathered file aside), fails naming
# the offender on a violating tree, and does not count test code.
set -euo pipefail

SCRIPT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/check-production-line-limit.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

gen() { # gen <path> <production-lines> [test-lines]
  local path="$1" prod="$2" test_lines="${3:-0}"
  mkdir -p "$(dirname "$path")"
  { for i in $(seq 1 "$prod"); do echo "fn f$i() {}"; done
    if [ "$test_lines" -gt 0 ]; then
      echo "#[cfg(test)]"
      echo "mod tests {"
      for i in $(seq 1 "$test_lines"); do echo "fn t$i() {}"; done
      echo "}"
    fi
  } > "$path"
}

# Compliant tree: under-limit file + a file whose PRODUCTION part is
# under the limit even though the file is long (test module appended).
mkdir -p "$TMP/ok/pkgs/id/src/web"
gen "$TMP/ok/pkgs/id/src/small.rs" 100
gen "$TMP/ok/pkgs/id/src/long_with_tests.rs" 100 5000
# The real grandfathered path, over the limit: warning, not failure.
gen "$TMP/ok/pkgs/id/src/web/markdown.rs" 3000
bash "$SCRIPT" "$TMP/ok" > "$TMP/ok.out"
grep -q "GRANDFATHERED pkgs/id/src/web/markdown.rs" "$TMP/ok.out"
grep -q "production-line-guard: OK" "$TMP/ok.out"
echo "pass: compliant tree passes, grandfather reported"

# Violating tree: one synthetic over-limit production file.
mkdir -p "$TMP/bad/pkgs/id/src"
gen "$TMP/bad/pkgs/id/src/huge.rs" 2501
if bash "$SCRIPT" "$TMP/bad" > "$TMP/bad.out" 2>&1; then
  echo "FAIL: guard accepted an over-limit file" >&2; exit 1
fi
grep -q "OVER LIMIT pkgs/id/src/huge.rs: 2501 production lines" "$TMP/bad.out"
echo "pass: violating tree fails and names the offender with its count"

# Production code AFTER an early out-of-line test-module declaration
# still counts (Codex finding on PR #4709): a file that declares
# #[cfg(test)] mod tests; at the top and then runs 2,501 production
# lines must fail, not be truncated at the declaration.
mkdir -p "$TMP/early/pkgs/id/src"
{ echo "#[cfg(test)]"
  echo "mod tests;"
  for i in $(seq 1 2501); do echo "fn p$i() {}"; done
} > "$TMP/early/pkgs/id/src/early.rs"
if bash "$SCRIPT" "$TMP/early" > "$TMP/early.out" 2>&1; then
  echo "FAIL: guard truncated counting at an out-of-line test declaration" >&2; exit 1
fi
grep -q "OVER LIMIT pkgs/id/src/early.rs: 2501 production lines" "$TMP/early.out"
echo "pass: production after an out-of-line test declaration is counted"

# Production code after a test item whose braces balance on one line
# still counts (Codex finding on PR #4709): #[cfg(test)] fn helper() {}
# excludes only the helper; skipping must not swallow what follows.
mkdir -p "$TMP/sameline/pkgs/id/src"
{ echo "#[cfg(test)] fn inline_helper() {}"
  echo "#[cfg(test)]"
  echo "fn helper() {}"
  for i in $(seq 1 2501); do echo "fn p$i() {}"; done
} > "$TMP/sameline/pkgs/id/src/sameline.rs"
if bash "$SCRIPT" "$TMP/sameline" > "$TMP/sameline.out" 2>&1; then
  echo "FAIL: guard kept skipping after a one-line test item" >&2; exit 1
fi
grep -q "OVER LIMIT pkgs/id/src/sameline.rs: 2501 production lines" "$TMP/sameline.out"
echo "pass: production after a one-line test item is counted"

# Items gated by a compound cfg predicate that requires test —
# #[cfg(all(test, feature = "world"))] — are test code (Codex finding
# on PR #4709): a 3,000-line module under that gate must not count.
mkdir -p "$TMP/compound/pkgs/id/src"
{ for i in $(seq 1 100); do echo "fn f$i() {}"; done
  echo "#[cfg(all(test, feature = \"world\"))]"
  echo "mod world_only_tests {"
  for i in $(seq 1 3000); do echo "fn t$i() {}"; done
  echo "}"
} > "$TMP/compound/pkgs/id/src/compound.rs"
bash "$SCRIPT" "$TMP/compound" > "$TMP/compound.out"
grep -q "production-line-guard: OK" "$TMP/compound.out"
echo "pass: compound cfg(all(test, ...)) module is excluded"

# Control: a module gated WITHOUT test — #[cfg(all(feature = "world"))]
# — is production: the same shape must fail the guard.
mkdir -p "$TMP/prodcfg/pkgs/id/src"
{ echo "#[cfg(all(feature = \"world\"))]"
  echo "mod world_prod {"
  for i in $(seq 1 2501); do echo "fn p$i() {}"; done
  echo "}"
} > "$TMP/prodcfg/pkgs/id/src/prodcfg.rs"
if bash "$SCRIPT" "$TMP/prodcfg" > "$TMP/prodcfg.out" 2>&1; then
  echo "FAIL: guard excluded a non-test compound cfg module" >&2; exit 1
fi
grep -q "OVER LIMIT pkgs/id/src/prodcfg.rs" "$TMP/prodcfg.out"
echo "pass: compound cfg without test stays production"

# Test code is out of scope: tests/ trees and tests.rs modules.
mkdir -p "$TMP/tests/pkgs/id/tests" "$TMP/tests/pkgs/id/src/encryption"
gen "$TMP/tests/pkgs/id/tests/cli_integration.rs" 4000
gen "$TMP/tests/pkgs/id/src/encryption/tests.rs" 4000
bash "$SCRIPT" "$TMP/tests" > "$TMP/tests.out"
grep -q "checked 0 production" "$TMP/tests.out"
echo "pass: test files are not counted"

echo "check-production-line-limit.test.sh: all contract tests passed"
