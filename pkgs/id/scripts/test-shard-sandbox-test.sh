#!/usr/bin/env bash
# Regression tests for test-shard-sandbox.sh argument and selection safety.
# Use a temporary fixture tree and stub cargo so the oracle is the logged
# invocation set: valid shards partition every target exactly once; invalid
# or empty shards never invoke cargo.
set -euo pipefail

script="$(cd "$(dirname "$0")" && pwd)/test-shard-sandbox.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

mkdir -p "$tmp/bin" "$tmp/work/tests/nested"
cp "$script" "$tmp/work/test-shard-sandbox.sh"
for target in a b c d e f g h; do
  : >"$tmp/work/tests/$target.rs"
done
: >"$tmp/work/tests/nested/main.rs"

cat >"$tmp/bin/cargo" <<'EOF'
#!/usr/bin/env bash
printf 'cargo %s\n' "$*" >>"$CARGO_LOG"
EOF
chmod +x "$tmp/bin/cargo"
export PATH="$tmp/bin:$PATH"
export CARGO_LOG="$tmp/cargo.log"
cd "$tmp/work"

fail() {
  echo "test-shard-sandbox-test.sh: FAIL: $1" >&2
  exit 1
}

check_shard() {
  local shard="$1" expected="$2" expected_calls="$3"
  : >"$CARGO_LOG"
  bash ./test-shard-sandbox.sh "$shard" 4 >"$tmp/out" 2>"$tmp/err" || {
    cat "$tmp/err" >&2
    fail "valid shard $shard/4 exited non-zero"
  }
  grep -Fqx "$expected" "$CARGO_LOG" ||
    fail "shard $shard/4 did not make expected target invocation (log: $(cat "$CARGO_LOG"))"
  [ "$(wc -l <"$CARGO_LOG")" -eq "$expected_calls" ] ||
    fail "shard $shard/4 made an unexpected number of cargo calls (log: $(cat "$CARGO_LOG"))"
}

# Sorted targets are a,b,c,d,e,f,g,h,nested; the reference partition is
# index modulo four. Shard zero also retains the existing bins/doc calls.
check_shard 0 'cargo test --all-features --test a --test e --test nested -- --skip serve_tests' 3
grep -Fqx 'cargo test --all-features --bins -- --skip serve_tests' "$CARGO_LOG" ||
  fail 'shard 0 lost its --bins invocation'
grep -Fqx 'cargo test --all-features --doc' "$CARGO_LOG" ||
  fail 'shard 0 lost its --doc invocation'
check_shard 1 'cargo test --all-features --test b --test f -- --skip serve_tests' 1
check_shard 2 'cargo test --all-features --test c --test g -- --skip serve_tests' 1
check_shard 3 'cargo test --all-features --test d --test h -- --skip serve_tests' 1

# Decimal arguments with leading zeroes remain decimal, not Bash octal.
: >"$CARGO_LOG"
bash ./test-shard-sandbox.sh 00 04 >"$tmp/out" 2>"$tmp/err" || {
  cat "$tmp/err" >&2
  fail 'leading-zero decimal invocation failed'
}
grep -Fqx 'cargo test --all-features --test a --test e --test nested -- --skip serve_tests' "$CARGO_LOG" ||
  fail 'leading-zero decimal invocation selected the wrong targets'
[ "$(wc -l <"$CARGO_LOG")" -eq 3 ] ||
  fail 'leading-zero decimal invocation made an unexpected number of cargo calls'

expect_invalid() {
  local expected="$1"
  shift
  : >"$CARGO_LOG"
  if bash ./test-shard-sandbox.sh "$@" >"$tmp/out" 2>"$tmp/err"; then
    fail "invalid invocation unexpectedly succeeded: $*"
  fi
  [ ! -s "$CARGO_LOG" ] ||
    fail "invalid invocation reached cargo: $* (log: $(cat "$CARGO_LOG"))"
  grep -Fq "$expected" "$tmp/err" || {
    cat "$tmp/err" >&2
    fail "invalid invocation did not name $expected: $*"
  }
}

expect_invalid 'usage:'
expect_invalid 'usage:' 0 4 extra
expect_invalid 'SHARD' x 4
expect_invalid 'COUNT' 0 x
expect_invalid 'COUNT' 0 0
expect_invalid 'COUNT' 0 -1
expect_invalid 'SHARD' -1 4
expect_invalid 'SHARD' 4 4
expect_invalid 'SHARD' 5 4

# A valid range can still select no targets (for example a shard count
# larger than the target set); that must fail safely, never run unfiltered.
rm -rf tests
mkdir -p tests
: >tests/only.rs
: >"$CARGO_LOG"
if bash ./test-shard-sandbox.sh 1 4 >"$tmp/out" 2>"$tmp/err"; then
  fail 'empty valid shard unexpectedly succeeded'
fi
[ ! -s "$CARGO_LOG" ] ||
  fail "empty valid shard reached cargo unfiltered: $(cat "$CARGO_LOG")"
grep -Fq 'no test targets selected for shard 1/4' "$tmp/err" || {
  cat "$tmp/err" >&2
  fail 'empty valid shard did not explain the selection failure'
}

echo 'test-shard-sandbox tests passed'
