# Back-of-envelope monthly cost for a Worker + Durable Object pkarr relay on Workers Paid.
# Inputs are from the pricing pages in sources/ (read 2026-10-11). The per-request CPU
# and the 2 ms assumption are NOT measured. Only idle 5-minute republishes are modelled
# (no on-change publishes, no GETs).
REPUBLISH_S = 300            # iroh DEFAULT_REPUBLISH_INTERVAL (iroh-1.3.0 pkarr.rs:146)
DAYS = 30
CPU_MS_PER_REQ = 2.0         # ASSUMPTION
WORKER_INCL, WORKER_RATE = 10_000_000, 0.30 / 1_000_000   # $/request beyond included
DO_INCL, DO_RATE = 1_000_000, 0.15 / 1_000_000
CPU_INCL_MS, CPU_RATE = 30_000_000, 0.02 / 1_000_000      # $/CPU ms beyond included
BASE = 5.00
FREE_DAILY = 100_000         # Free plan daily Worker and DO request caps

def monthly(endpoints):
    req = endpoints * (86_400 // REPUBLISH_S) * DAYS
    w = max(0, req - WORKER_INCL) * WORKER_RATE
    d = max(0, req - DO_INCL) * DO_RATE
    cpu = max(0, req * CPU_MS_PER_REQ - CPU_INCL_MS) * CPU_RATE
    return req, w, d, cpu, BASE + w + d + cpu

for e in (100, 1_000, 10_000):
    req, w, d, cpu, total = monthly(e)
    print(f"endpoints={e:>6}  PUT/month={req:>12,}  worker=${w:7.2f}  do=${d:7.2f}  cpu=${cpu:6.2f}  total=${total:8.2f}")
print(f"Free plan: endpoints whose idle republish fits in {FREE_DAILY:,}/day = {FREE_DAILY // (86_400 // REPUBLISH_S)}")
