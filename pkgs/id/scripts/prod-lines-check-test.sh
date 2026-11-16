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

check_usage_fail() {
  local label="$1"
  shift
  local status=0
  if bash "$script" "$@" >"$out" 2>&1; then
    echo "unexpected success: $label" >&2
    exit 1
  else
    status=$?
  fi
  if [[ $status -ne 2 ]]; then
    cat "$out"
    echo "unexpected exit status $status (expected 2): $label" >&2
    exit 1
  fi
  if ! grep -q "unexpected argument" "$out"; then
    cat "$out"
    echo "missing unexpected-argument message: $label" >&2
    exit 1
  fi
}

# Extra positional arguments must be rejected before any scan work.
check_usage_fail 'extra positional argument' extra-arg-test

# Clean tree: one small file, integration tests, vendored code and the
# module-gated encryption, world-session and cli test modules are all out of
# scope and must not trip a limit of 3. Both the pre-split `src/...` layout
# and the post-split `crates/<crate>/src/...` layout are covered: the guard
# normalizes the crate prefix before matching (#10603).
reset_tree
write_small src
mkdir -p "$tmp/tests" "$tmp/vendor/dep" "$tmp/src/encryption" "$tmp/src/web" "$tmp/src/world_session"
mkdir -p "$tmp/crates/id-groups/src/encryption" "$tmp/crates/id-worlds/src/world_session" "$tmp/crates/id-cli/src/cli"
printf 'line1\nline2\nline3\nline4\n' >"$tmp/tests/big_integration.rs"
printf 'line1\nline2\nline3\nline4\n' >"$tmp/vendor/dep/big_vendor.rs"
printf 'line1\nline2\nline3\nline4\n' >"$tmp/src/encryption/tests.rs"
printf 'line1\nline2\nline3\nline4\n' >"$tmp/src/world_session/tests.rs"
printf 'line1\nline2\nline3\nline4\n' >"$tmp/crates/id-groups/src/encryption/tests.rs"
printf 'line1\nline2\nline3\nline4\n' >"$tmp/crates/id-worlds/src/world_session/tests.rs"
printf 'line1\nline2\nline3\nline4\n' >"$tmp/crates/id-cli/src/cli/tests.rs"
printf 'line1\nline2\nline3\n#[cfg(test)]\nmod tests { }\n' >"$tmp/src/web/markdown.rs"
check_pass 3 'clean tree passes'

# New over-limit production file fails and is named with its count — in the
# pre-split layout and under a crate.
reset_tree
write_small src
for i in 1 2 3 4 5; do echo "pub fn f$i() {}"; done >"$tmp/src/grown.rs"
check_fail 3 'new over-limit file fails' 'src/grown.rs has 5 production lines (limit 3)'
reset_tree
write_small crates/x/src
for i in 1 2 3 4 5; do echo "pub fn f$i() {}"; done >"$tmp/crates/x/src/grown.rs"
check_fail 3 'new over-limit crate file fails' 'crates/x/src/grown.rs has 5 production lines (limit 3)'

# A grandfathered crate file at its allowance passes; one line more fails,
# and the allowance is reported against the real crate path.
reset_tree
mkdir -p "$tmp/crates/id-directory/src"
awk 'BEGIN { for (i = 0; i < 2610; i++) print "x" }' >"$tmp/crates/id-directory/src/directory.rs"
check_pass 2500 'grandfather at allowance passes'
printf 'one more line\n' >>"$tmp/crates/id-directory/src/directory.rs"
check_fail 2500 'grandfather growth fails' 'grandfathered file grew: 2611 production lines (allowance 2610'

# Empty scan is a broken scan, not a pass.
reset_tree
mkdir -p "$tmp/empty"
if PROD_LINES_ROOT="$tmp/empty" bash "$script" >"$out" 2>&1; then
  echo "unexpected success: empty scan passed" >&2
  exit 1
fi

echo "prod-lines-check tests passed"
