# Markdown tables for RESULTS.md, generated from logs/run-seed*.txt so the numbers are copied, not retyped.
import glob
from collections import defaultdict

rows, pre = [], []
for path in sorted(glob.glob("logs/run-seed*.txt")):
    for line in open(path):
        if line.startswith("ROW|"):
            rows.append(line.rstrip("\n").split("|"))
        elif line.startswith("PRE|"):
            pre.append(line.rstrip("\n").split("|"))

MODES = ["full", "recent:1", "recent:8", "recent:32", "recent:128", "recent:512"]
BUILDS = ["stock", "serde-bytes"]
KS = [100, 1000]

def stats(xs):
    n = len(xs)
    m = sum(xs) / n
    sd = (sum((x - m) ** 2 for x in xs) / (n - 1)) ** 0.5 if n > 1 else 0.0
    return m, sd

agg = defaultdict(list)
for r in rows:
    agg[(r[1], int(r[3]), r[4])].append(r)
pre_by = defaultdict(list)
for p in pre:
    pre_by[(p[1], int(p[3]), p[4])].append(p[5].startswith("welcome ok"))

print("### Table A: welcome, build time, joiner check (5 seeds; mean (sd))")
print()
print("| build | K (epochs) | history | bundle (secrets) | welcome B | build µs (median of reps) | encode µs | decrypt, expected | refused, expected | fail | welcome processed | pre-welcome delivery OK |")
print("|---|---|---|---|---|---|---|---|---|---|---|---|")
for b in BUILDS:
    for k in KS:
        for mode in MODES:
            rs = agg[(b, k, mode)]
            w = stats([float(r[5]) for r in rs])
            bu = stats([float(r[9]) for r in rs])
            en = stats([float(r[10]) for r in rs])
            bundle = stats([float(r[8]) for r in rs])[0]
            dec = stats([float(r[11]) for r in rs])[0]
            ref = stats([float(r[12]) for r in rs])[0]
            fail = sum(int(r[13]) for r in rs)
            ok = sum(int(r[16]) for r in rs)
            pok = sum(pre_by[(b, k, mode)])
            wsd = f"{w[0]:,.1f} ({w[1]:.1f})" if w[1] > 0 else f"{w[0]:,.0f} (0)"
            bsd = f"{bu[0]:,.0f} ({bu[1]:.0f})"
            print(f"| {b} | {k} ({k+1}) | {mode} | {bundle:.0f} | {wsd} | {bsd} | {en[0]:,.1f} | {dec:.0f} of {int(rs[0][15])} | {ref:.0f} | {fail} | {ok}/{len(rs)} | {pok}/{len(rs)} |")
print()
print("### Table B: re-add (churn) welcomes, cumulative bytes over all re-adds (5-seed mean (sd))")
print()
print("| build | history | K=100 (50 re-adds) | K=1000 (500 re-adds) |")
print("|---|---|---|---|")
for b in BUILDS:
    for mode in MODES:
        cells = []
        for k in KS:
            v = stats([float(r[17]) for r in agg[(b, k, mode)]])
            cells.append(f"{v[0]:,.0f} ({v[1]:,.0f})")
        print(f"| {b} | {mode} | {cells[0]} | {cells[1]} |")
