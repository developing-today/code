#!/usr/bin/env bash
# Self-test for prod-lines-check.sh: proves the guard passes a clean
# tree and fails on each violation shape (new over-limit file,
# grandfather growth, broken/empty scan). Runs inside the
# prod-lines-check just recipe, so CI continuously proves the guard
# fails, not just that it passes.
set -euo pipefail

script="$(cd "$(dirname "$0")" && pwd)/prod-lines-check.sh"
tmp="$(mktemp -d)"
out="$(mktemp)"
trap 'rm -rf "$tmp" "$out"' EXIT

# A small production file: 3 production lines + a test module.
write_small() {
  mkdir -p "$tmp/$1"
  {
    echo "pub fn f() {}"
    echo "pub fn g() {}"
    echo "pub fn h() {}"
    echo "#[cfg(test)]"
    echo "mod tests { }"
  } >"$tmp/$1/lib.rs"
}

reset_tree() {
  rm -rf "$tmp"
  mkdir -p "$tmp"
}

check_pass() {
  if ! PROD_LINES_ROOT="$tmp" PROD_LINES_LIMIT="$1" bash "$script" >"$out" 2>&1; then
    cat "$out"
    echo "unexpected failure: $2" >&2
    exit 1
  fi
}

check_fail() {
  if PROD_LINES_ROOT="$tmp" PROD_LINES_LIMIT="$1" bash "$script" >"$out" 2>&1; then
    echo "unexpected success: $2" >&2
    exit 1
  fi
  if ! grep -q "$3" "$out"; then
    cat "$out"
    echo "failure did not name '$3': $2" >&2
    exit 1
  fi
}

# Clean tree: one small file, integration tests, vendored code and the
# module-gated encryption and world-session tests are all out of scope and must not trip
# a limit of 3.
reset_tree
write_small src
mkdir -p "$tmp/tests" "$tmp/vendor/dep" "$tmp/src/encryption" "$tmp/src/web" "$tmp/src/world_session"
printf 'line1\nline2\nline3\nline4\n' >"$tmp/tests/big_integration.rs"
printf 'line1\nline2\nline3\nline4\n' >"$tmp/vendor/dep/big_vendor.rs"
printf 'line1\nline2\nline3\nline4\n' >"$tmp/src/encryption/tests.rs"
printf 'line1\nline2\nline3\nline4\n' >"$tmp/src/world_session/tests.rs"
printf 'line1\nline2\nline3\n#[cfg(test)]\nmod tests { }\n' >"$tmp/src/web/markdown.rs"
check_pass 3 'clean tree passes'

# New over-limit production file fails and is named with its count.
reset_tree
write_small src
for i in 1 2 3 4 5; do echo "pub fn f$i() {}"; done >"$tmp/src/grown.rs"
check_fail 3 'new over-limit file fails' 'src/grown.rs has 5 production lines (limit 3)'

# Grandfathered markdown.rs at its allowance passes; one line more fails.
reset_tree
mkdir -p "$tmp/src/web"
awk 'BEGIN { for (i = 0; i < 5312; i++) print "x" }' >"$tmp/src/web/markdown.rs"
check_pass 2500 'grandfather at allowance passes'
printf 'one more line\n' >>"$tmp/src/web/markdown.rs"
check_fail 2500 'grandfather growth fails' 'grandfathered file grew: 5313 production lines (allowance 5312'

# Empty scan is a broken scan, not a pass.
reset_tree
mkdir -p "$tmp/empty"
if PROD_LINES_ROOT="$tmp/empty" bash "$script" >"$out" 2>&1; then
  echo "unexpected success: empty scan passed" >&2
  exit 1
fi

echo "prod-lines-check tests passed"
