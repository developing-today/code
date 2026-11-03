#!/usr/bin/env bash
# Contract tests for check-production-line-limit.sh (#1411): the guard
# passes on a compliant tree (grandfathered file aside), fails naming
# the offender on a violating tree, and does not count test code.
set -euo pipefail

SCRIPT="/home/user/git/fleet/6592-1411-line-guard/check-production-line-limit.sh"
TMP="/tmp/tmp.UL8Pow9FpO"
trap "rm -rf """ EXIT

gen() { # gen <path> <production-lines> [test-lines]
  local path="" prod="" test_lines="0"
  mkdir -p "."
  { for i in ; do echo "fn f() {}"; done
    if [ "" -gt 0 ]; then
      echo "#[cfg(test)]"
      for i in ; do echo "fn t() {}"; done
    fi
  } > ""
}

mkdir -p "/ok/pkgs/id/src/web"
gen "/ok/pkgs/id/src/small.rs" 100
gen "/ok/pkgs/id/src/long_with_tests.rs" 100 5000
gen "/ok/pkgs/id/src/web/markdown.rs" 3000
bash "" "/ok" > "/ok.out"
grep -q "GRANDFATHERED pkgs/id/src/web/markdown.rs" "/ok.out"
grep -q "production-line-guard: OK" "/ok.out"
echo "pass: compliant tree passes, grandfather reported"

mkdir -p "/bad/pkgs/id/src"
gen "/bad/pkgs/id/src/huge.rs" 2501
if bash "" "/bad" > "/bad.out" 2>&1; then
  echo "FAIL: guard accepted an over-limit file" >&2; exit 1
fi
grep -q "OVER LIMIT pkgs/id/src/huge.rs: 2501 production lines" "/bad.out"
echo "pass: violating tree fails and names the offender with its count"

mkdir -p "/tests/pkgs/id/tests" "/tests/pkgs/id/src/encryption"
gen "/tests/pkgs/id/tests/cli_integration.rs" 4000
gen "/tests/pkgs/id/src/encryption/tests.rs" 4000
bash "" "/tests" > "/tests.out"
grep -q "checked 0 production" "/tests.out"
echo "pass: test files are not counted"

echo "check-production-line-limit.test.sh: all contract tests passed"
