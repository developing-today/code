# Extrapolation table for RESULTS.md. Everything here is EXTRAPOLATED from the K=100 and K=1000 means.
# Full-history welcome and build/encode time: straight line in secrets (K + 1), through the two means.
# Recent-N welcome, build and encode time: flat in K once the bundle holds N secrets (checked at K=100 and
# K=1000 where N <= K + 1). Re-add churn: the per-re-add model of analysis/churn_model.py (c0 + c1 * secrets,
# summed over K/2 re-adds with min(N, k + 1) secrets each).
import glob
from collections import defaultdict

rows = []
for path in sorted(glob.glob("logs/run-seed*.txt")):
    for line in open(path):
        if line.startswith("ROW|"):
            rows.append(line.rstrip("\n").split("|"))

agg = defaultdict(list)
for r in rows:
    agg[(r[1], int(r[3]), r[4])].append(r)

def mean(key, idx):
    xs = [float(r[idx]) for r in agg[key]]
    return sum(xs) / len(xs)

INF = 10**9
MODES = {"full": INF, "recent:1": 1, "recent:8": 8, "recent:32": 32, "recent:128": 128, "recent:512": 512}

def secrets_sum(K, N):
    return sum(min(N, 2 * i + 1) for i in range(1, K // 2 + 1))

print("| build | history | welcome B at K=100 | welcome B at K=1,000 | welcome B at K=10,000 (extrap.) | build µs at K=1,000 | build µs at K=10,000 (extrap.) | re-add churn B at K=1,000 | re-add churn B at K=10,000 (extrap.) |")
print("|---|---|---|---|---|---|---|---|---|")
for build in ["stock", "serde-bytes"]:
    for mode, N in MODES.items():
        k1, k2 = (build, 100, mode), (build, 1000, mode)
        w1, w2 = mean(k1, 5), mean(k2, 5)
        b1, b2 = mean(k1, 9), mean(k2, 9)
        n1, n2 = mean(k1, 8), mean(k2, 8)
        t1, t2 = mean(k1, 17), mean(k2, 17)
        if mode == "full":
            # welcome bytes and build time against secrets (K + 1), line through the two K points
            slope = (w2 - w1) / (n2 - n1)
            intercept = w1 - slope * n1
            w10k = intercept + slope * 10001
            bslope = (b2 - b1) / (n2 - n1)
            b10k = (b1 - bslope * n1) + bslope * 10001
            w10k_s = f"{w10k:,.0f}"
            b10k_s = f"{b10k / 1000:.1f} ms"
        else:
            w10k_s = f"{w2:,.0f} (flat)"
            b10k_s = f"{b2:,.0f} µs (flat)"
        # re-add churn model: fit c0 and c1 on the two K points, then extrapolate to K = 10,000
        c100, s100 = 50, secrets_sum(100, N)
        c1000, s1000 = 500, secrets_sum(1000, N)
        det = c100 * s1000 - c1000 * s100
        if det == 0:
            c1, c0 = 0.0, t2 / c1000
        else:
            c1 = (c100 * t2 - c1000 * t1) / det
            c0 = (t1 - c1 * s100) / c100
        churn10k = c0 * 5000 + c1 * secrets_sum(10000, N)
        print(f"| {build} | {mode} | {w1:,.0f} | {w2:,.0f} | {w10k_s} | {b2:,.0f} | {b10k_s} | {t2:,.0f} | {churn10k / 1e6:,.1f} MB |")

print()
print("Frame check (inferred: one welcome per 1 MiB frame, `design…md:519`):")
for build, slope_key in [("stock", None), ("serde-bytes", None)]:
    k1, k2 = (build, 100, "full"), (build, 1000, "full")
    w1, w2 = mean(k1, 5), mean(k2, 5)
    n1, n2 = mean(k1, 8), mean(k2, 8)
    slope = (w2 - w1) / (n2 - n1)
    intercept = w1 - slope * n1
    cross = (1048576 - intercept) / slope
    print(f"- {build}: full welcome passes 1,048,576 B at about {cross:,.0f} secrets, i.e. about {cross - 1:,.0f} rotations")
