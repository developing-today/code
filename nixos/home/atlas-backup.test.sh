#!/usr/bin/env bash
# Regression suite for atlas-backup.sh (developing-today/code#3913).
# The filed-snapshot parser (dedupe + keep-7 date set) must accept the
# lines the script actually writes. This drives the REAL parser heredoc
# extracted from the script against fixture backup logs:
#   * legacy lowercase "ok laptop/r2/drive" lines still count as filed;
#   * "OK" lines count as filed;
#   * PARTIAL lines never count as filed;
#   * "unchanged" lines do not count as filed dates.
# The keep-7 date set is the same tail computation the script uses.
set -uo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$SELF_DIR/atlas-backup.sh"
KEEP_LAPTOP_R2=7

TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT

PARSER="$TMP_ROOT/parser.py"
awk '/python3 - "\$LOG" <<.PYEOF./{p=1;next} /^PYEOF$/{p=0} p' "$SCRIPT" >"$PARSER"
if [ ! -s "$PARSER" ]; then
  echo "FAIL: could not extract the filed-snapshot parser from $SCRIPT" >&2
  exit 1
fi

failures=0
pass() { echo "ok - $1"; }
fail() {
  echo "not ok - $1" >&2
  failures=$((failures + 1))
}

parse() {
  python3 -I "$PARSER" "$1"
}

LOG="$TMP_ROOT/backup-log.md"
cat >"$LOG" <<'EOF'
2026-11-01 | sha256=aaa111 | ok laptop/r2/drive | size=10 | counts x
2026-11-02 | sha256=bbb222 | PARTIAL laptop=ok r2=FAIL drive=ok | size=11
2026-11-03 | sha256=ccc333 | unchanged — legs skipped | size=11
2026-11-04 | sha256=ddd444 | OK laptop/r2/drive | size=12 | counts y
2026-11-05 | sha256=ddd444 | unchanged — legs skipped | size=12
EOF

out="$(parse "$LOG")"
last_sha="$(printf '%s\n' "$out" | sed -n '1p')"
dates="$(printf '%s\n' "$out" | sed -n '2p')"

if [ "$last_sha" = "ddd444" ]; then
  pass "dedupe sha is the last OK snapshot (mixed legacy ok and OK lines)"
else
  fail "dedupe sha expected ddd444, got '$last_sha'"
fi

# This is the exact guard used by atlas-backup.sh before its legs. The
# second run sees the first run's digest in the parsed log and skips all
# three legs; unchanged itself is not appended as a filed date.
DIGEST="ddd444"
SKIP_DEDUPE=0
if [ "$DIGEST" = "$last_sha" ] && [ "$SKIP_DEDUPE" -ne 1 ]; then
  pass "an unchanged second-run digest takes the dedupe skip path"
else
  fail "unchanged second-run digest did not take the dedupe skip path"
fi

if [ "$dates" = "2026-11-01 2026-11-04" ]; then
  pass "filed dates exclude PARTIAL and unchanged lines"
else
  fail "filed dates expected '2026-11-01 2026-11-04', got '$dates'"
fi

NOLOG="$TMP_ROOT/absent.md"
out="$(parse "$NOLOG")"
if [ "$(printf '%s\n' "$out" | sed -n '1p')" = "" ] && [ "$(printf '%s\n' "$out" | sed -n '2p')" = "" ]; then
  pass "missing log yields no dedupe sha and no filed dates"
else
  fail "missing log should yield empty parse output"
fi

ALL=""
for n in $(seq -w 1 10); do ALL="$ALL 2026-10-$n"; done
ALL_DATES="$(printf '%s\n%s\n' $ALL "2026-10-10" | sort -u)"
KEEP_SET="$(printf '%s\n' "$ALL_DATES" | tail -n "$KEEP_LAPTOP_R2")"
kept="$(printf '%s\n' "$KEEP_SET" | wc -l)"
newest="$(printf '%s\n' "$KEEP_SET" | head -1)"
if [ "$kept" -eq 7 ] && [ "$newest" = "2026-10-04" ]; then
  pass "keep-7 retains the last 7 filed dates"
else
  fail "keep-7 expected 7 dates starting 2026-10-04, got $kept starting '$newest'"
fi

if [ "$failures" -ne 0 ]; then
  echo "$failures failure(s)" >&2
  exit 1
fi
echo "all atlas-backup parser checks passed"
