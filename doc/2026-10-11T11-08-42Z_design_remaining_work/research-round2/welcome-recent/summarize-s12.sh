#!/bin/sh
# Summarises logs/run-seed*.txt into logs/summary.txt: one row per build and seed, then the mean and
# sample standard deviation over seeds for each build.
cd "$(dirname "$0")/logs" || exit 1
awk '
function grab(s, re,    t) {
  if (match(s, re)) {
    t = substr(s, RSTART, RLENGTH)
    gsub(/[^0-9.]/, "", t)
    return t + 0
  }
  return 0
}
/^## build:/ { build = $3; sub(/,$/, "", build); seed = $5 }
/^\| 1000 \|/ { split($0, f, "|"); vals["probe"] = f[4] + 0 }
/^final EncryptionGroup::add/ { vals["final"] = grab($0, "welcome [0-9]+ B") }
/^fit welcome/ {
  vals["slope"] = grab($0, "b = [0-9.]+")
  vals["icpt"] = grab($0, "a = [0-9.]+")
  vals["resid"] = grab($0, "max residual = [0-9.]+")
}
/^time at K=/ {
  vals["add"] = grab($0, "Dcgka::add median [0-9]+")
  vals["enc"] = grab($0, "encode median [0-9]+")
  vals["gadd"] = grab($0, "EncryptionGroup::add median [0-9]+")
}
/^re-adds of Carol/ {
  vals["readd"] = grab($0, "[0-9]+ B in total")
  split("probe final slope icpt resid add enc gadd readd", names, " ")
  line = build " " seed
  for (i = 1; i <= 9; i++) {
    m = names[i]
    v = vals[m]
    line = line " " v
    sum[build, m] += v
    sq[build, m] += v * v
  }
  n[build]++
  rows[++nrows] = line
}
END {
  print "build seed | probe_B final_B slope_B_per_secret intercept_B max_resid_B add_med_us encode_med_us groupadd_med_us readd_total_B"
  for (i = 1; i <= nrows; i++) print rows[i]
  split("probe final slope icpt resid add enc gadd readd", names, " ")
  for (b = 1; b <= 3; b++) {
    build = (b == 1) ? "stock" : (b == 2) ? "serde-bytes" : "found-as-is"
    if (!(build in n)) continue
    out = build " mean"
    sd = build " sd"
    for (i = 1; i <= 9; i++) {
      m = names[i]
      mean = sum[build, m] / n[build]
      s = (n[build] > 1) ? sqrt((sq[build, m] - n[build] * mean * mean) / (n[build] - 1)) : 0
      out = out sprintf(" %.1f", mean)
      sd = sd sprintf(" %.1f", s)
    }
    print out
    print sd
  }
}
' run-seed*.txt > summary.txt
cat summary.txt
