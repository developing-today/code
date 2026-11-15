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
# A property section at the end generates mixed log sequences and
# checks the parser against an independent model of its invariants
# (oracle: only success rows file dates; the dedupe digest is the
# last success row's sha).
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

# --- property checks: generated sequences vs an independent model --------
# Oracle (stated independently of the parser's code): a row files a date
# iff its verdict is a success marker (canonical "OK" or legacy "ok");
# PARTIAL and unchanged rows never file. The dedupe digest is the sha of
# the LAST success row, or empty when no row succeeded. The generator
# builds sequences from row kinds directly, so the model's expectation
# never re-reads the log text the parser consumes.
PROP="$TMP_ROOT/property.py"
cat >"$PROP" <<'PYEOF'
import random, subprocess, sys

parser, tmp = sys.argv[1], sys.argv[2]
KINDS = ["OK", "ok", "PARTIAL", "unchanged"]
DATES = ["2026-11-01", "2026-11-02", "2026-11-03"]
SHAS = ["aaa111", "bbb222", "ccc333"]

def render(kind, date, sha):
    if kind == "OK":
        return f"{date} | sha256={sha} | OK laptop/r2/drive | size=10 | counts x"
    if kind == "ok":
        return f"{date} | sha256={sha} | ok laptop/r2/drive | size=10 | counts x"
    if kind == "PARTIAL":
        return f"{date} | sha256={sha} | PARTIAL laptop=ok r2=FAIL drive=ok | size=10"
    return f"{date} | sha256={sha} | unchanged \u2014 legs skipped | size=10"

def model(rows):
    sha, dates = "", []
    for kind, date, s in rows:
        if kind in ("OK", "ok"):
            sha = s
            dates.append(date)
    return sha, " ".join(dates)

cases = mixed = dup_dates = no_success = 0
for seed in range(300):
    rng = random.Random(seed)
    rows = [(rng.choice(KINDS), rng.choice(DATES), rng.choice(SHAS))
            for _ in range(rng.randrange(0, 13))]
    log = f"{tmp}/prop-{seed}.log"
    with open(log, "w") as fh:
        fh.write("\n".join(render(*r) for r in rows) + ("\n" if rows else ""))
    out = subprocess.run(["python3", "-I", parser, log],
                         capture_output=True, text=True, check=True).stdout
    lines = out.splitlines()
    got = (lines[0] if lines else "", lines[1] if len(lines) > 1 else "")
    exp = model(rows)
    if got != exp:
        print(f"seed {seed}: parser {got!r} != model {exp!r} rows={rows}", file=sys.stderr)
        sys.exit(1)
    cases += 1
    kinds = {r[0] for r in rows}
    if len(kinds) >= 3:
        mixed += 1
    if len([r[1] for r in rows]) != len({r[1] for r in rows}):
        dup_dates += 1
    if not any(r[0] in ("OK", "ok") for r in rows):
        no_success += 1

# Coverage guards: the property is meaningless if the generator never
# produced the interesting shapes.
if not (mixed >= 100 and dup_dates >= 50 and no_success >= 10):
    print(f"weak generation: mixed={mixed} dup_dates={dup_dates} no_success={no_success}",
          file=sys.stderr)
    sys.exit(1)
print(f"{cases} generated sequences (mixed={mixed}, dup_dates={dup_dates}, no_success={no_success})")
PYEOF
if prop_out="$(python3 "$PROP" "$PARSER" "$TMP_ROOT" 2>&1)"; then
  pass "property: parser matches the independent invariant model on $prop_out"
else
  fail "property: $prop_out"
fi

if [ "$failures" -ne 0 ]; then
  echo "$failures failure(s)" >&2
  exit 1
fi
echo "all atlas-backup parser checks passed"
