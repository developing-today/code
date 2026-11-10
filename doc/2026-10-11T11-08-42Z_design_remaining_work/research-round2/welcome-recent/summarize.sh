#!/bin/sh
# Aggregates logs/run-seed*.txt into logs/summary.txt: one line per build, K and history mode, with the
# mean and sample standard deviation over seeds (ROW lines), and the count of seeds whose pre-welcome
# delivery succeeded (PRE lines). Columns are tab-separated.
cd "$(dirname "$0")/logs" || exit 1
cat run-seed*.txt | awk -F'|' '
function sd(s, q, c,    m) {
  if (c < 2) return 0
  m = s / c
  return sqrt((q - c * m * m) / (c - 1))
}
/^ROW\|/ {
  key = $2 "\t" $4 "\t" $5
  if (!(key in seen)) { seen[key] = 1; order[++nkeys] = key }
  n[key]++
  for (i = 6; i <= 19; i++) { v = $i + 0; sum[key, i] += v; sq[key, i] += v * v }
  fail[key] += $14
  welcome_ok[key] += $17
}
/^PRE\|/ {
  key = $2 "\t" $4 "\t" $5
  if ($6 ~ /^welcome ok/) pre_ok[key]++
}
END {
  printf "build\tK\tmode\tseeds\twelcome_B_mean\twelcome_B_sd\thistory_B\tciphertext_B\tbundle\tbuild_us_mean\tbuild_us_sd\tencode_us_mean\tdecrypt_pass_mean\trefuse_pass_mean\tfail_total\texpected_decrypt\ttotal\twelcome_ok_total\tpre_welcome_ok_seeds\treadd_total_mean\treadd_total_sd\treadd_count\n"
  for (k = 1; k <= nkeys; k++) {
    key = order[k]; c = n[key]
    printf "%s\t%d", key, c
    printf "\t%.1f\t%.1f", sum[key, 6] / c, sd(sum[key, 6], sq[key, 6], c)
    printf "\t%.1f\t%.1f\t%.1f", sum[key, 7] / c, sum[key, 8] / c, sum[key, 9] / c
    printf "\t%.1f\t%.1f\t%.1f", sum[key, 10] / c, sd(sum[key, 10], sq[key, 10], c), sum[key, 11] / c
    printf "\t%.1f\t%.1f\t%d\t%.1f\t%.1f", sum[key, 12] / c, sum[key, 13] / c, fail[key], sum[key, 15] / c, sum[key, 16] / c
    printf "\t%d\t%d/%d", welcome_ok[key], pre_ok[key] + 0, c
    printf "\t%.1f\t%.1f\t%.1f\n", sum[key, 18] / c, sd(sum[key, 18], sq[key, 18], c), sum[key, 19] / c
  }
}' > summary.txt
cat summary.txt
