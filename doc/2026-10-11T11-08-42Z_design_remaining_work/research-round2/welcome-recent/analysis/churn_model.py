# Re-add (churn) welcome totals, modelled per re-add. Re-add i (i = 1..K/2) happens at rotation k = 2i and
# carries a bundle of min(N, k + 1) = min(N, 2i + 1) secrets. A re-add welcome of s secrets is c0 + c1*s bytes
# (linear in s, checked against the per-N measurements). Fit c0 and c1 to the 5-seed means at K=100 and
# K=1000, check the fit against both, then extrapolate to K=10,000 (labelled extrapolation).
import glob
from collections import defaultdict

rows = []
for path in sorted(glob.glob("logs/run-seed*.txt")):
    for line in open(path):
        if line.startswith("ROW|"):
            rows.append(line.rstrip("\n").split("|"))

tot = defaultdict(list)
for r in rows:
    tot[(r[1], int(r[3]), r[4])].append(float(r[17]))
mean_tot = {k: sum(v) / len(v) for k, v in tot.items()}

INF = 10**9
modes = {"full": INF, "recent:1": 1, "recent:8": 8, "recent:32": 32, "recent:128": 128, "recent:512": 512}

def model_total(K, N):
    # sum over re-adds i = 1..K/2 of the bundle size min(N, 2i+1), as a count of secrets
    secrets_sum = 0
    count = K // 2
    for i in range(1, count + 1):
        secrets_sum += min(N, 2 * i + 1)
    return count, secrets_sum

out = []
for build in ["stock", "serde-bytes"]:
    for mode, N in modes.items():
        t100, t1000 = mean_tot[(build, 100, mode)], mean_tot[(build, 1000, mode)]
        c100, s100 = model_total(100, N)
        c1000, s1000 = model_total(1000, N)
        # solve c0*c100 + c1*s100 = t100 ; c0*c1000 + c1*s1000 = t1000
        det = c100 * s1000 - c1000 * s100
        if det == 0:
            # Every re-add carries the same number of secrets (N = 1): one parameter, per-re-add size.
            c1 = 0.0
            c0 = t1000 / c1000
        else:
            c1 = (c100 * t1000 - c1000 * t100) / det
            c0 = (t100 - c1 * s100) / c100
        pred100 = c0 * c100 + c1 * s100
        pred1000 = c0 * c1000 + c1 * s1000
        c10k, s10k = model_total(10000, N)
        t10k = c0 * c10k + c1 * s10k
        out.append((build, mode, c0, c1, t100, pred100, t1000, pred1000, t10k))

print("build\tmode\tc0_B\tc1_B_per_secret\tK100_meas\tK100_fit\tK1000_meas\tK1000_fit\tK10000_extrap_MB")
for build, mode, c0, c1, t100, p100, t1000, p1000, t10k in out:
    print(f"{build}\t{mode}\t{c0:.1f}\t{c1:.2f}\t{t100:,.0f}\t{p100:,.0f}\t{t1000:,.0f}\t{p1000:,.0f}\t{t10k/1e6:,.2f}")
