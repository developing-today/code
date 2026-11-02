#!/usr/bin/env bash
# id-bench-compare.test.sh — tests for id-bench-compare.sh (#1619).
#
# Covers the three defects fixed for #1619 — unit-mismatched rows and
# zero-baseline rows are reported as incomparable (no percent, no
# verdict), and a non-numeric BENCH_REGRESSION_THRESHOLD_PCT is a
# config error (non-zero exit) — plus the unchanged behaviour:
# same-unit comparisons, not-run / no-baseline rows, and the
# missing-files exit-0 path.
#
# Runs anywhere bash+awk exist:
#   .github/scripts/id-bench-compare.test.sh
set -u

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
compare="$script_dir/id-bench-compare.sh"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

failures=0

pass() { echo "ok - $1"; }
fail() { echo "FAIL - $1" >&2; failures=$((failures + 1)); }

assert_contains() { # <desc> <haystack> <needle>
  case "$2" in
    *"$3"*) pass "$1" ;;
    *) fail "$1 (missing: $3)" ;;
  esac
}

assert_not_contains() { # <desc> <haystack> <needle>
  case "$2" in
    *"$3"*) fail "$1 (unexpected: $3)" ;;
    *) pass "$1" ;;
  esac
}

assert_exit() { # <desc> <expected> <actual>
  if [ "$2" -eq "$3" ]; then pass "$1"; else fail "$1 (exit $3, want $2)"; fi
}

# --- unit mismatch: no percent, no verdict, both units shown --------
printf 'bench alpha unit=us_per_event median=100.000 min=99.000 reps=5\n' > "$tmp/base-unit.txt"
printf 'bench alpha unit=ms_per_event median=0.200 min=0.190 reps=5\n' > "$tmp/cur-unit.txt"
out="$("$compare" "$tmp/cur-unit.txt" "$tmp/base-unit.txt")"
code=$?
assert_exit "unit mismatch exits 0" 0 "$code"
assert_contains "unit mismatch is incomparable" "$out" "incomparable (unit changed)"
assert_contains "unit mismatch shows baseline unit" "$out" "us_per_event"
assert_contains "unit mismatch shows current unit" "$out" "ms_per_event"
assert_not_contains "unit mismatch prints no percent" "$out" "-99.8%"
assert_contains "summary calls out incomparable rows" "$out" "could not be compared"
assert_contains "summary names the unit change" "$out" "unit changed: us_per_event"

# --- zero baseline: incomparable, never inf/nan ----------------------
printf 'bench beta unit=us_per_event median=0.000 min=0.000 reps=5\n' > "$tmp/base-zero.txt"
printf 'bench beta unit=us_per_event median=50.000 min=49.000 reps=5\n' > "$tmp/cur-zero.txt"
out="$("$compare" "$tmp/cur-zero.txt" "$tmp/base-zero.txt")"
code=$?
assert_exit "zero baseline exits 0" 0 "$code"
assert_contains "zero baseline is incomparable" "$out" "incomparable (zero baseline)"
assert_not_contains "zero baseline prints no inf" "$out" "inf"
assert_not_contains "zero baseline prints no nan" "$out" "nan"

# --- bad threshold: config error, non-zero exit, clear message -------
BENCH_REGRESSION_THRESHOLD_PCT=abc "$compare" "$tmp/cur-zero.txt" "$tmp/base-zero.txt" \
  > "$tmp/bad-out.txt" 2> "$tmp/bad-err.txt"
code=$?
if [ "$code" -ne 0 ]; then pass "bad threshold exits non-zero"; else fail "bad threshold exits non-zero (exit 0)"; fi
err="$(cat "$tmp/bad-err.txt")"
assert_contains "bad threshold names the variable" "$err" "BENCH_REGRESSION_THRESHOLD_PCT"

# a decimal threshold is valid and still flags regressions
printf 'bench gamma unit=us_per_event median=100.000 min=99.000 reps=5\n' > "$tmp/base-ok.txt"
printf 'bench gamma unit=us_per_event median=106.000 min=105.000 reps=5\n' > "$tmp/cur-ok.txt"
out="$(BENCH_REGRESSION_THRESHOLD_PCT=5.5 "$compare" "$tmp/cur-ok.txt" "$tmp/base-ok.txt")"
code=$?
assert_exit "decimal threshold exits 0" 0 "$code"
assert_contains "decimal threshold flags regression" "$out" "⚠️ regression"

# --- same-unit behaviour unchanged ------------------------------------
out="$("$compare" "$tmp/cur-ok.txt" "$tmp/base-ok.txt")"
assert_contains "same-unit prints percent" "$out" "+6.0%"
assert_contains "same-unit under default threshold is ok" "$out" "| ok |"

printf 'bench gamma unit=us_per_event median=130.000 min=129.000 reps=5\n' > "$tmp/cur-reg.txt"
out="$("$compare" "$tmp/cur-reg.txt" "$tmp/base-ok.txt")"
assert_contains "same-unit regression percent" "$out" "+30.0%"
assert_contains "same-unit regression flagged" "$out" "⚠️ regression"

# --- not-run / no-baseline rows unchanged ------------------------------
printf 'bench delta unit=us_per_event median=10.000 min=9.000 reps=5\n' >> "$tmp/base-ok.txt"
printf 'bench epsilon unit=us_per_event median=5.000 min=4.000 reps=5\n' >> "$tmp/cur-ok.txt"
out="$("$compare" "$tmp/cur-ok.txt" "$tmp/base-ok.txt")"
assert_contains "missing current row is not run" "$out" "not run"
assert_contains "missing baseline row is no baseline" "$out" "no baseline"

# --- missing files still exit 0 ----------------------------------------
out="$("$compare" "$tmp/does-not-exist.txt" "$tmp/base-ok.txt")"
code=$?
assert_exit "missing current file exits 0" 0 "$code"
assert_contains "missing current file is reported" "$out" "not found"

if [ "$failures" -gt 0 ]; then
  echo "$failures test(s) failed" >&2
  exit 1
fi
echo "all id-bench-compare tests passed"
