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
# summary. This script ALWAYS exits 0 — slice 3 is non-blocking by
# design: the report informs, it never gates a PR.
set -euo pipefail

threshold="${BENCH_REGRESSION_THRESHOLD_PCT:-20}"

if [ "$#" -lt 1 ] || [ "$#" -gt 2 ]; then
  echo "usage: id-bench-compare.sh <bench-output-file> [baseline-file]" >&2
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
  function parse(file, median, unit, order,    line, i, f, kv) {
    while ((getline line < file) > 0) {
      if (line !~ /^bench /) continue
      split(line, f, /[ \t]+/)
      name = f[2]
      for (i = 3; i in f; i++) {
        split(f[i], kv, "=")
        if (kv[1] == "median") median[name] = kv[2] + 0
        if (kv[1] == "unit") unit[name] = kv[2]
      }
      if (!(name in seen)) { seen[name] = 1; order[++n] = name }
    }
    close(file)
  }
  BEGIN {
    n = 0
    parse(ARGV[1], base_median, base_unit, base_order)
    base_n = n
    n = 0
    parse(ARGV[2], cur_median, cur_unit, cur_order)

    print "## Benchmark regression report (vs committed baselines)"
    print ""
    printf "Threshold: a benchmark is flagged when its median is more than %s%% above baseline. Baselines were taken on the reference machine in `pkgs/id/benches/BASELINES.md`; shared CI runners differ, so treat flags as a prompt to investigate, not a verdict.\n", threshold
    print ""
    print "| benchmark | unit | baseline median | current median | change | status |"
    print "| --- | --- | ---: | ---: | ---: | --- |"
    regressions = 0
    for (i = 1; i <= base_n; i++) {
      name = base_order[i]
      if (name in cur_median) {
        pct = (cur_median[name] - base_median[name]) / base_median[name] * 100
        if (pct > threshold) {
          status = "⚠️ regression"
          regressions++
          reg_list[++reg_n] = sprintf("%s (+%.1f%%)", name, pct)
        } else {
          status = "ok"
        }
        printf "| %s | %s | %.3f | %.3f | %+.1f%% | %s |\n", name, base_unit[name], base_median[name], cur_median[name], pct, status
      } else {
        printf "| %s | %s | %.3f | — | — | not run |\n", name, base_unit[name], base_median[name]
      }
    }
    for (i = 1; i <= n; i++) {
      name = cur_order[i]
      if (!(name in base_median)) {
        printf "| %s | %s | — | %.3f | — | no baseline |\n", name, cur_unit[name], cur_median[name]
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
  }
' "$baseline_file" "$current_file"
