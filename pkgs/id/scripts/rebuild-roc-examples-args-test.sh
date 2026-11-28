#!/usr/bin/env bash
# Regression test for rebuild-roc-examples.sh argument contract (#10499).
# The script takes no positional arguments; extra args must be rejected
# before any rebuild work begins.
set -euo pipefail

script="$(cd "$(dirname "$0")" && pwd)/rebuild-roc-examples.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

fail() {
  echo "rebuild-roc-examples-args-test: FAIL: $1" >&2
  exit 1
}

mkdir -p "$tmp/examples/lounge"
cat >"$tmp/examples/lounge/build.sh" <<'SH'
#!/usr/bin/env bash
echo "rebuilt" >"$MARKER"
SH
chmod +x "$tmp/examples/lounge/build.sh"
cp "$script" "$tmp/rebuild-roc-examples.sh"

# Rejection must happen before cd/rebuild work. Run from outside the
# pkgs/id tree with a marker path that would be written if a rebuild ran.
export MARKER="$tmp/marker"
if (cd "$tmp" && bash ./rebuild-roc-examples.sh unexpected >out 2>err); then
  fail "extra argument was accepted"
fi
[ ! -f "$MARKER" ] || fail "rebuild work ran despite extra argument"
grep -q 'Usage: rebuild-roc-examples.sh' "$tmp/err" || fail "usage not printed"
grep -q 'unaccounted argument' "$tmp/err" || fail "unaccounted argument not reported"

echo "rebuild-roc-examples-args tests passed"
