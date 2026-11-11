#!/usr/bin/env bash
# id-bench-compare.sh — diff pkgs/id benchmark medians against the
# committed F11.8 baselines and print a Markdown regression report
# (slice 3 of #539, issue #1513).
#
# Usage: id-bench-compare.sh <bench-output-file> [baseline-file]
#
# Both inputs use the harness's own line format (world_bench.rs):
#   bench <name> unit=<unit> median=<value> min=<value> reps=<n>
# baseline.txt is literally a captured harness run plus comments, so
# one parser serves both sides.
#
# A benchmark is flagged when its current median is more than
# BENCH_REGRESSION_THRESHOLD_PCT percent (default 20) above baseline.
# The report goes to stdout; the caller appends it to the GitHub job
# summary. This script exits 0 whenever it produces a report — slice 3
# is non-blocking by design: the report informs, it never gates a PR.
# The only non-zero exits are usage errors and a non-numeric
# BENCH_REGRESSION_THRESHOLD_PCT (both exit 2, before any report):
# a broken configuration must be loud, never silently read as
# "flag nothing" (#1619).
#
# Rows that cannot be compared — the unit changed between baseline
# and current, or the baseline median is zero — are reported as
# incomparable: no percent change, no ok/regression verdict (#1619).
#
# Malformed bench lines are loud, never results (#6876): a bench line
# whose median field is missing or not a number, or a benchmark that
# appears twice in one file, makes that benchmark's row incomparable
# (malformed bench line) with the reason named — never a coerced 0,
# never "not run" for a line that is present, never a silent
# overwrite.
set -euo pipefail

threshold="${BENCH_REGRESSION_THRESHOLD_PCT:-20}"

if [ "$#" -lt 1 ] || [ "$#" -gt 2 ]; then
  echo "usage: id-bench-compare.sh <bench-output-file> [baseline-file]" >&2
  exit 2
fi

if ! [[ "$threshold" =~ ^[0-9]+([.][0-9]+)?$ ]]; then
  echo "id-bench-compare.sh: error: BENCH_REGRESSION_THRESHOLD_PCT must be a non-negative number, got '$threshold'" >&2
  exit 2
fi

current_file="$1"
baseline_file="${2:-pkgs/id/benches/baseline.txt}"

if [ ! -f "$current_file" ]; then
  echo "## Benchmark regression report"
  echo
  echo "⚠️ Benchmark output file \`$current_file\` not found — the bench run produced no results to compare."
  exit 0
fi

if [ ! -f "$baseline_file" ]; then
  echo "## Benchmark regression report"
  echo
  echo "⚠️ Baseline file \`$baseline_file\` not found — nothing to compare against."
  exit 0
fi

awk -v threshold="$threshold" '
  function parse(file, median, unit, order, present, bad,    line, i, f, kv, name, med, has_med) {
    while ((getline line < file) > 0) {
      if (line !~ /^bench /) continue
      split(line, f, /[ \t]+/)
      name = f[2]
      if (name in present) {
        # A repeated bench line is malformed input (#6876): the first
        # line stands, the duplicate is named in the report — never a
        # silent overwrite.
        if (!(name in bad)) bad[name] = "duplicate bench line"
        continue
      }
      present[name] = 1
      order[++n] = name
      has_med = 0
      for (i = 3; i in f; i++) {
        split(f[i], kv, "=")
        if (kv[1] == "unit") unit[name] = kv[2]
        if (kv[1] == "median") {
          has_med = 1
          med = kv[2]
          # Never coerce: awk would read a non-numeric median as 0 and
          # the row would report a fabricated -100% "ok" (#6876). The
          # accepted shape matches the threshold check in the shell.
          if (med ~ /^[0-9]+([.][0-9]+)?$/) median[name] = med + 0
          else if (!(name in bad)) bad[name] = "non-numeric median \"" med "\""
        }
      }
      if (!has_med && !(name in bad)) bad[name] = "missing median field"
    }
    close(file)
  }
  BEGIN {
    n = 0
    parse(ARGV[1], base_median, base_unit, base_order, base_present, bad_base)
    base_n = n
    n = 0
    parse(ARGV[2], cur_median, cur_unit, cur_order, cur_present, bad_cur)

    print "## Benchmark regression report (vs committed baselines)"
    print ""
    printf "Threshold: a benchmark is flagged when its median is more than %s%% above baseline. Baselines were taken on the reference machine in `pkgs/id/benches/BASELINES.md`; shared CI runners differ, so treat flags as a prompt to investigate, not a verdict.\n", threshold
    print ""
    print "| benchmark | unit | baseline median | current median | change | status |"
    print "| --- | --- | ---: | ---: | ---: | --- |"
    regressions = 0
    incomparable = 0
    malformed = 0
    for (i = 1; i <= base_n; i++) {
      name = base_order[i]
      if ((name in bad_base) || (name in bad_cur)) {
        malformed++
        reason = ""
        if (name in bad_base) reason = "baseline: " bad_base[name]
        if (name in bad_cur) reason = reason (reason == "" ? "" : "; ") "current: " bad_cur[name]
        mal_list[++mal_n] = sprintf("%s (%s)", name, reason)
        bmed = (name in base_median) ? sprintf("%.3f", base_median[name]) : "—"
        cmed = (name in cur_median) ? sprintf("%.3f", cur_median[name]) : "—"
        u = (name in base_unit) ? base_unit[name] : ((name in cur_unit) ? cur_unit[name] : "—")
        printf "| %s | %s | %s | %s | — | incomparable (malformed bench line) |\n", name, u, bmed, cmed
      } else if (name in cur_median) {
        if (base_unit[name] != cur_unit[name]) {
          incomparable++
          inc_list[++inc_n] = sprintf("%s (unit changed: %s → %s)", name, base_unit[name], cur_unit[name])
          printf "| %s | %s → %s | %.3f | %.3f | — | incomparable (unit changed) |\n", name, base_unit[name], cur_unit[name], base_median[name], cur_median[name]
        } else if (base_median[name] == 0) {
          incomparable++
          inc_list[++inc_n] = sprintf("%s (zero baseline median)", name)
          printf "| %s | %s | %.3f | %.3f | — | incomparable (zero baseline) |\n", name, base_unit[name], base_median[name], cur_median[name]
        } else {
          pct = (cur_median[name] - base_median[name]) / base_median[name] * 100
          if (pct > threshold) {
            status = "⚠️ regression"
            regressions++
            reg_list[++reg_n] = sprintf("%s (+%.1f%%)", name, pct)
          } else {
            status = "ok"
          }
          printf "| %s | %s | %.3f | %.3f | %+.1f%% | %s |\n", name, base_unit[name], base_median[name], cur_median[name], pct, status
        }
      } else {
        printf "| %s | %s | %.3f | — | — | not run |\n", name, base_unit[name], base_median[name]
      }
    }
    for (i = 1; i <= n; i++) {
      name = cur_order[i]
      if (!(name in base_present)) {
        if (name in bad_cur) {
          malformed++
          mal_list[++mal_n] = sprintf("%s (current: %s)", name, bad_cur[name])
          cmed = (name in cur_median) ? sprintf("%.3f", cur_median[name]) : "—"
          u = (name in cur_unit) ? cur_unit[name] : "—"
          printf "| %s | %s | — | %s | — | incomparable (malformed bench line) |\n", name, u, cmed
        } else {
          printf "| %s | %s | — | %.3f | — | no baseline |\n", name, cur_unit[name], cur_median[name]
        }
      }
    }
    print ""
    if (regressions > 0) {
      printf "⚠️ **%d benchmark(s) regressed more than %s%%:** ", regressions, threshold
      for (i = 1; i <= reg_n; i++) printf "%s%s", reg_list[i], (i < reg_n ? ", " : "")
      print ""
    } else {
      printf "✅ No benchmark regressed more than %s%% against the committed baselines.\n", threshold
    }
    if (incomparable > 0) {
      printf "⚠️ **%d benchmark(s) could not be compared:** ", incomparable
      for (i = 1; i <= inc_n; i++) printf "%s%s", inc_list[i], (i < inc_n ? ", " : "")
      print ""
      print "Incomparable rows carry no percent change and no verdict: a unit change means the two medians measure different things, and a zero baseline has no meaningful ratio. Re-baseline (see `pkgs/id/benches/BASELINES.md`) to make them comparable again."
    }
    if (malformed > 0) {
      printf "⚠️ **%d benchmark(s) had malformed bench lines:** ", malformed
      for (i = 1; i <= mal_n; i++) printf "%s%s", mal_list[i], (i < mal_n ? ", " : "")
      print ""
      print "Malformed rows carry no percent change and no verdict: the bench line was present but its median was missing or not a number, or the benchmark appeared more than once in one file (#6876). Fix the harness output or the baseline file; never read these rows as results."
    }
  }
' "$baseline_file" "$current_file"
