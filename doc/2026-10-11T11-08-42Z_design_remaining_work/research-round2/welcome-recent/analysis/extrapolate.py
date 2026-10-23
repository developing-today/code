# Totals from the raw ROW/PRE lines, and linear/quadratic extrapolation from the K=100 and K=1000
# means. Every extrapolated value is labelled as such in RESULTS.md.
import glob, re, math, sys
from collections import defaultdict

rows = []
pre = []
for path in sorted(glob.glob("logs/run-seed*.txt")):
    for line in open(path):
        f = line.rstrip("\n").split("|")
        if f[0] == "ROW":
            rows.append(f)
        elif f[0] == "PRE":
            pre.append(f)

# ROW fields: ROW|build|seed|K|mode|welcome|history|cipher|bundle|build_us|encode_us|decrypt|refuse|fail|expected|total|welcome_ok|readd_total|readd_count
def num(x):
    return float(x)

pass_n = sum(int(r[11]) + int(r[12]) for r in rows)
fail_n = sum(int(r[13]) for r in rows)
total_n = sum(int(r[15]) for r in rows)
welcome_ok = sum(int(r[16]) for r in rows)
print(f"ROW lines: {len(rows)}; joiner checks: total {total_n}, pass {pass_n}, fail {fail_n}; welcomes processed {welcome_ok}/{len(rows)}")

pre_ok = sum(1 for p in pre if p[5].startswith("welcome ok"))
print(f"PRE lines: {len(pre)}; pre-welcome delivery ok {pre_ok}, fails {len(pre) - pre_ok}")
by = defaultdict(lambda: [0, 0])
for p in pre:
    key = (p[1], int(p[3]), p[4])
    by[key][0] += p[5].startswith("welcome ok")
    by[key][1] += 1
for key in sorted(by, key=lambda k: (k[0], k[1], k[2])):
    print(f"  pre-welcome {key}: ok {by[key][0]}/{by[key][1]}")

# Per (build, K, mode) means over seeds.
agg = defaultdict(list)
for r in rows:
    agg[(r[1], int(r[3]), r[4])].append(r)

def mean(xs):
    return sum(xs) / len(xs)

def m(key, idx):
    return mean([num(r[idx]) for r in agg[key]])

print()
print("## Extrapolation to K = 10,000 (labelled extrapolation; fitted through the 5-seed means at K=100 and K=1000)")
for build in ["stock", "serde-bytes"]:
    print(f"build {build}")
    for mode in ["full", "recent:1", "recent:8", "recent:32", "recent:128", "recent:512"]:
        k1, k2 = (build, 100, mode), (build, 1000, mode)
        if k1 not in agg or k2 not in agg:
            continue
        s1, s2 = m(k1, 5), m(k2, 5)            # welcome bytes
        n1, n2 = m(k1, 8), m(k2, 8)            # bundle size (secrets)
        b1, b2 = m(k1, 9), m(k2, 9)            # build time, us
        e1, e2 = m(k1, 10), m(k2, 10)          # encode time, us
        t1, t2 = m(k1, 17), m(k2, 17)          # re-add total bytes
        if mode == "full":
            # welcome bytes = a + b * secrets, secrets = K + 1
            b = (s2 - s1) / (n2 - n1)
            a = s1 - b * n1
            s10k = a + b * 10001
            # build and encode time per secret, same two-point line
            bt = (b2 - b1) / (n2 - n1)
            bt0 = b1 - bt * n1
            bt10k = bt0 + bt * 10001
            et = (e2 - e1) / (n2 - n1)
            et0 = e1 - et * n1
            et10k = et0 + et * 10001
            # 1 MiB frame: first secret count whose full welcome exceeds 1,048,576 bytes
            cross = (1048576 - a) / b
            print(f"  full: welcome = {a:.1f} + {b:.2f} * secrets B; at 10,001 secrets: {s10k:,.0f} B; "
                  f"frame (1 MiB) crossed at ~{cross:,.0f} secrets (~{cross - 1:,.0f} rotations)")
            print(f"        build = {bt0:.0f} + {bt:.3f} * secrets us; at 10,001: {bt10k/1000:.1f} ms; "
                  f"encode at 10,001: {et10k/1000:.1f} ms")
        else:
            print(f"  {mode}: welcome flat at {s2:,.0f} B (bundle {n2:.0f}); build {b2:.0f} us; encode {e2:.0f} us")
        # Re-add (churn) total: T(K) = a*K + b*K^2 through the two points.
        # Solve 100a + 1e4 b = t1 and 1000a + 1e6 b = t2.
        det = 100 * 1e6 - 1e4 * 1000
        A = (t1 * 1e6 - 1e4 * t2) / det
        B = (100 * t2 - 1000 * t1) / det
        t10k = A * 10000 + B * 1e8
        print(f"        re-add churn total: K=100 {t1:,.0f} B, K=1000 {t2:,.0f} B; "
              f"fit aK+bK^2 (a={A:.1f}, b={B:.4f}) -> K=10,000 {t10k/1e6:,.1f} MB")
