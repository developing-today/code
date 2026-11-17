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
#
# Matrix mode (#5896): when either input is a `results.jsonl` bundle
# (the world_bench matrix output / committed baseline.jsonl), cells
# are compared by the full matrix identity — (machine_class,
# workload_id, interface_id, configuration_id, metric, statistic) —
# never by workload name alone: two cells that share a workload but
# differ in interface or configuration are different rows and can
# never collide. A machine-class mismatch makes every row
# incomparable (results are never compared across machine classes,
# per the #5896 decision record). Cells the current run records with
# an explicit non-measured status (unsupported / not_applicable) are
# reported with that status, not as "not run". The JSONL parser
# reads the harness's own emitted shape (fixed key order, one cell
# per line); it is not a general JSON parser.
set -euo pipefail

threshold="${BENCH_REGRESSION_THRESHOLD_PCT:-20}"

if [ "$#" -lt 1 ] || [ "$#" -gt 2 ]; then
  echo "usage: id-bench-compare.sh <bench-output-file> [baseline-file]" >&2
  exit 2
fi

if ! [[ $threshold =~ ^[0-9]+([.][0-9]+)?$ ]]; then
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

case "$current_file:$baseline_file" in
*.jsonl:* | *:*.jsonl)
  awk -v threshold="$threshold" '
    function getstr(line, key,    pat, s) {
      pat = "\"" key "\":\""
      s = index(line, pat)
      if (s == 0) return ""
      line = substr(line, s + length(pat))
      s = index(line, "\"")
      if (s == 0) return ""
      return substr(line, 1, s - 1)
    }
    function parse_jsonl(file, val, unit, order, present, cellstatus, bad, mclass,    line, type, w, i, c, st, rest, np, parts, j, seg, metric, stat, u, v, key) {
      mclass = ""
      while ((getline line < file) > 0) {
        type = getstr(line, "type")
        if (type == "manifest") { mclass = getstr(line, "machine_class"); continue }
        if (type != "cell") continue
        w = getstr(line, "workload_id"); i = getstr(line, "interface_id")
        c = getstr(line, "configuration_id"); st = getstr(line, "status")
        cellstatus[w SUBSEP i SUBSEP c] = st
        if (st != "measured") continue
        # Metric entries are nested objects on the same line; split
        # on their opening marker and parse each segment.
        np = split(line, parts, "{\"metric\":\"")
        for (j = 2; j <= np; j++) {
          seg = parts[j]
          metric = seg; sub(/",".*/, "", metric)
          stat = seg; sub(/^[^"]*","statistic":"/, "", stat); sub(/".*/, "", stat)
          u = seg; sub(/^.*"unit":"/, "", u); sub(/".*/, "", u)
          v = seg; sub(/^.*"value":/, "", v); sub(/[^0-9.].*/, "", v)
          key = w SUBSEP i SUBSEP c SUBSEP metric SUBSEP stat
          if (key in present) { if (!(key in bad)) bad[key] = "duplicate cell metric"; continue }
          present[key] = 1
          order[++n] = key
          unit[key] = u
          if (v ~ /^[0-9]+([.][0-9]+)?$/) val[key] = v + 0
          else if (!(key in bad)) bad[key] = "non-numeric value \"" v "\""
        }
      }
      close(file)
      return mclass
    }
    function disp(key,    f) {
      split(key, f, SUBSEP)
      return f[1] "/" f[2] "/" f[3] " " f[4] "/" f[5]
    }
    BEGIN {
      n = 0
      base_mc = parse_jsonl(ARGV[1], base_val, base_unit, base_order, base_present, base_cell, bad_base, "")
      base_n = n
      n = 0
      cur_mc = parse_jsonl(ARGV[2], cur_val, cur_unit, cur_order, cur_present, cur_cell, bad_cur, "")

      print "## Benchmark matrix report (vs committed baselines)"
      print ""
      printf "Identity: (machine_class, workload, interface, configuration, metric, statistic). Baseline machine class: `%s`; current machine class: `%s`. Threshold: a cell is flagged when its value is more than %s%% above baseline.\n", base_mc, cur_mc, threshold
      print ""
      print "| cell (workload/interface/configuration metric/statistic) | unit | baseline | current | change | status |"
      print "| --- | --- | ---: | ---: | ---: | --- |"
      regressions = 0; incomparable = 0; malformed = 0
      mc_diff = (base_mc != cur_mc)
      for (k = 1; k <= base_n; k++) {
        key = base_order[k]
        split(key, f, SUBSEP)
        cellkey = f[1] SUBSEP f[2] SUBSEP f[3]
        if ((key in bad_base) || (key in bad_cur)) {
          malformed++
          reason = ""
          if (key in bad_base) reason = "baseline: " bad_base[key]
          if (key in bad_cur) reason = reason (reason == "" ? "" : "; ") "current: " bad_cur[key]
          mal_list[++mal_n] = sprintf("%s (%s)", disp(key), reason)
          printf "| %s | %s | %s | %s | — | incomparable (malformed cell) |\n", disp(key), ((key in base_unit) ? base_unit[key] : "—"), ((key in base_val) ? sprintf("%.3f", base_val[key]) : "—"), ((key in cur_val) ? sprintf("%.3f", cur_val[key]) : "—")
        } else if (mc_diff) {
          incomparable++
          inc_list[++inc_n] = sprintf("%s (machine class differs: %s → %s)", disp(key), base_mc, cur_mc)
          printf "| %s | %s | %.3f | %s | — | incomparable (machine class differs) |\n", disp(key), base_unit[key], base_val[key], ((key in cur_val) ? sprintf("%.3f", cur_val[key]) : "—")
        } else if (key in cur_val) {
          if (base_unit[key] != cur_unit[key]) {
            incomparable++
            inc_list[++inc_n] = sprintf("%s (unit changed: %s → %s)", disp(key), base_unit[key], cur_unit[key])
            printf "| %s | %s → %s | %.3f | %.3f | — | incomparable (unit changed) |\n", disp(key), base_unit[key], cur_unit[key], base_val[key], cur_val[key]
          } else if (base_val[key] == 0) {
            incomparable++
            inc_list[++inc_n] = sprintf("%s (zero baseline)", disp(key))
            printf "| %s | %s | %.3f | %.3f | — | incomparable (zero baseline) |\n", disp(key), base_unit[key], base_val[key], cur_val[key]
          } else {
            pct = (cur_val[key] - base_val[key]) / base_val[key] * 100
            if (pct > threshold) { status = "⚠️ regression"; regressions++; reg_list[++reg_n] = sprintf("%s (+%.1f%%)", disp(key), pct) } else status = "ok"
            printf "| %s | %s | %.3f | %.3f | %+.1f%% | %s |\n", disp(key), base_unit[key], base_val[key], cur_val[key], pct, status
          }
        } else if (cellkey in cur_cell) {
          printf "| %s | %s | %.3f | — | — | %s |\n", disp(key), base_unit[key], base_val[key], cur_cell[cellkey]
        } else {
          printf "| %s | %s | %.3f | — | — | not run |\n", disp(key), base_unit[key], base_val[key]
        }
      }
      for (k = 1; k <= n; k++) {
        key = cur_order[k]
        if (!(key in base_present)) {
          if (key in bad_cur) {
            malformed++
            mal_list[++mal_n] = sprintf("%s (current: %s)", disp(key), bad_cur[key])
            printf "| %s | %s | — | %s | — | incomparable (malformed cell) |\n", disp(key), ((key in cur_unit) ? cur_unit[key] : "—"), ((key in cur_val) ? sprintf("%.3f", cur_val[key]) : "—")
          } else {
            printf "| %s | %s | — | %.3f | — | no baseline |\n", disp(key), cur_unit[key], cur_val[key]
          }
        }
      }
      print ""
      if (regressions > 0) {
        printf "⚠️ **%d cell(s) regressed more than %s%%:** ", regressions, threshold
        for (k = 1; k <= reg_n; k++) printf "%s%s", reg_list[k], (k < reg_n ? ", " : "")
        print ""
      } else {
        printf "✅ No matrix cell regressed more than %s%% against the committed baselines.\n", threshold
      }
      if (incomparable > 0) {
        printf "⚠️ **%d cell(s) could not be compared:** ", incomparable
        for (k = 1; k <= inc_n; k++) printf "%s%s", inc_list[k], (k < inc_n ? ", " : "")
        print ""
        print "Incomparable rows carry no percent change and no verdict: a unit or machine-class change means the two values measure different things, and a zero baseline has no meaningful ratio. Re-baseline (see `pkgs/id/benches/BASELINES.md`) to make them comparable again."
      }
      if (malformed > 0) {
        printf "⚠️ **%d cell(s) had malformed records:** ", malformed
        for (k = 1; k <= mal_n; k++) printf "%s%s", mal_list[k], (k < mal_n ? ", " : "")
        print ""
        print "Malformed rows carry no percent change and no verdict: the cell record was present but a metric value was missing or not a number, or the same cell identity appeared more than once in one bundle."
      }
    }
  ' "$baseline_file" "$current_file"
  exit 0
  ;;
esac

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
