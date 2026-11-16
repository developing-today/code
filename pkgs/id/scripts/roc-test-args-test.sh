#!/usr/bin/env bash
# Regression test for roc-test.sh argument contract (#10500).
# The script takes no positional arguments; extra args must be rejected
# before tool checks or test execution.
set -euo pipefail

script="$(cd "$(dirname "$0")" && pwd)/roc-test.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

fail() {
  echo "roc-test-args-test: FAIL: $1" >&2
  exit 1
}

# Stub roc/cargo markers to prove no test execution happens if arg
# rejection is broken (rejection must occur before command checks).
mkdir -p "$tmp/bin" "$tmp/examples/roc-world" "$tmp/examples/roc-cap-demo" "$tmp/examples/roc-import-policy"
cat >"$tmp/bin/roc" <<'SH'
#!/usr/bin/env bash
echo "roc-ran" >>"$TRACE"
exit 0
SH
cat >"$tmp/bin/cargo" <<'SH'
#!/usr/bin/env bash
echo "cargo-ran" >>"$TRACE"
exit 0
SH
chmod +x "$tmp/bin/roc" "$tmp/bin/cargo"
cp "$script" "$tmp/roc-test.sh"

export TRACE="$tmp/trace"
PATH="$tmp/bin:$PATH"
if (cd "$tmp" && bash ./roc-test.sh unexpected >out 2>err); then
  fail "extra argument was accepted"
fi
[ ! -f "$TRACE" ] || fail "test/tool work ran despite extra argument"
grep -q 'Usage: roc-test.sh' "$tmp/err" || fail "usage not printed"
grep -q 'unaccounted argument' "$tmp/err" || fail "unaccounted argument not reported"

echo "roc-test-args tests passed"
