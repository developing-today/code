#!/usr/bin/env bash
# Exercises the real sweep against fixture trees: opencode tmp
# entries older than 72h and mcpx-bin dirs older than 24h must be
# removed; anything younger, anything outside the managed roots,
# and anything in /tmp without the mcpx-bin- prefix must survive.
set -uo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$SELF_DIR/laptop-scratch-sweep.sh"
[ -f "$SCRIPT" ] || {
  echo "FAIL: script not found at $SCRIPT" >&2
  exit 1
}

TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT
failures=0
fail() {
  echo "FAIL: $1" >&2
  failures=$((failures + 1))
}
check_gone() { [ ! -e "$1" ] || fail "should be removed: $1"; }
check_kept() { [ -e "$1" ] || fail "should be kept: $1"; }

# --- fixtures ---------------------------------------------------------------
OC_TMP="$TMP_ROOT/opencode/tmp"
mkdir -p "$OC_TMP/old-dir" "$OC_TMP/fresh-dir"
printf 'x\n' >"$OC_TMP/old-dir/payload"
printf 'x\n' >"$OC_TMP/old-file"
printf 'x\n' >"$OC_TMP/fresh-dir/payload"
printf 'x\n' >"$OC_TMP/fresh-file"
# A sibling of tmp/ (stands in for the session store): never managed.
mkdir -p "$TMP_ROOT/opencode/sessions"
printf 'x\n' >"$TMP_ROOT/opencode/sessions/keep.db"
touch -d '100 hours ago' "$OC_TMP/old-dir/payload" "$OC_TMP/old-dir" "$OC_TMP/old-file"
touch -d '100 hours ago' "$TMP_ROOT/opencode/sessions/keep.db"

FAKE_TMP="$TMP_ROOT/faketmp"
mkdir -p "$FAKE_TMP/mcpx-bin-old" "$FAKE_TMP/mcpx-bin-fresh" "$FAKE_TMP/unrelated-old"
printf 'x\n' >"$FAKE_TMP/mcpx-bin-old/mcpx"
printf 'x\n' >"$FAKE_TMP/mcpx-bin-fresh/mcpx"
printf 'x\n' >"$FAKE_TMP/unrelated-old/keep"
touch -d '30 hours ago' "$FAKE_TMP/mcpx-bin-old/mcpx" "$FAKE_TMP/mcpx-bin-old"
touch -d '30 hours ago' "$FAKE_TMP/unrelated-old/keep" "$FAKE_TMP/unrelated-old"

# --- run --------------------------------------------------------------------
out="$(OPENCODE_TMP_DIR="$OC_TMP" MCPX_TMP_DIR="$FAKE_TMP" bash "$SCRIPT" 2>&1)" || {
  echo "FAIL: sweep exited non-zero: $out" >&2
  exit 1
}
echo "$out"

case "$out" in
  *"opencode-tmp removed=2"*mcpx-bin\ removed=1*) ;;
  *) fail "unexpected counts line: $out" ;;
esac

check_gone "$OC_TMP/old-dir"
check_gone "$OC_TMP/old-file"
check_kept "$OC_TMP/fresh-dir"
check_kept "$OC_TMP/fresh-file"
check_kept "$OC_TMP"
check_kept "$TMP_ROOT/opencode/sessions/keep.db"
check_gone "$FAKE_TMP/mcpx-bin-old"
check_kept "$FAKE_TMP/mcpx-bin-fresh"
check_kept "$FAKE_TMP/unrelated-old"

# Missing roots are a no-op success, not an error.
out="$(OPENCODE_TMP_DIR="$TMP_ROOT/nope" MCPX_TMP_DIR="$TMP_ROOT/also-nope" bash "$SCRIPT" 2>&1)" || {
  echo "FAIL: sweep with missing roots exited non-zero: $out" >&2
  exit 1
}
case "$out" in
  *"removed=0"*) ;;
  *) fail "missing-root run should report removed=0: $out" ;;
esac

if [ "$failures" -gt 0 ]; then
  echo "laptop-scratch-sweep.test: $failures failure(s)" >&2
  exit 1
fi
echo "laptop-scratch-sweep.test: ok"
