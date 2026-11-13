#!/usr/bin/env python3
"""Stats rollup oracle prototype (design-mill code#9399, R3 research).

Run: python3 -I stats_rollup_oracle.py [SEEDS]   (stdlib only; default 200 seeds)

Oracle: exact per-(day, group) statistics computed directly from the raw samples.
Under test: a two-level rollup. Each (hour, group) cell keeps a mergeable partial state
(count, sum, min, max, Welford mean/M2 merged with Chan's formula, a DDSketch-style
log-bucket histogram with relative error ALPHA, and an exact value-count map used for
mode). Hour cells are merged into day cells in a shuffled order, so the test also checks
commutativity and associativity of the merge.

Quantile definition (oracle and sketch use the same rank rule): nearest rank,
the ceil(q*n)-th smallest sample. Sample stddev uses n-1. Mode ties break to the
smaller value.

Invariants (exit 1 if any fails):
  I1 conservation: day counts sum to the number of raw events
  I2 exact-mergeable stats equal the oracle: count, sum, min, max, mode
  I3 float stats match the oracle within rel 1e-9: mean, stddev
  I4 sketch quantiles are within ALPHA relative error of the oracle, for
     Q1, median, Q3, p90, p99, p99.9
Also reports sample-size warnings (p99.9 needs roughly 1000+ samples per cell)
and a storage comparison between raw events and rollup states.
"""
import json
import math
import random
import sys
from collections import Counter, defaultdict

ALPHA = 0.01
GAMMA = (1 + ALPHA) / (1 - ALPHA)
LOG_GAMMA = math.log(GAMMA)
N_EVENTS = 8000
HOURS = 48
PROVIDERS = {
    "deepseek": ("ds-1107", 5.2),
    "luna": ("luna-6", 5.6),
    "openai": ("gpt-mini", 4.9),
    "haiku": ("haiku-5.5", 6.1),
}
EFFORTS = ["low", "medium", "high"]
EFFORT_SHIFT = {"low": 0.0, "medium": 0.15, "high": 0.35}
POOLS = ["workhorse", "limited", "speed"]
QUANTILES = {"q1": 0.25, "median": 0.5, "q3": 0.75, "p90": 0.90, "p99": 0.99, "p999": 0.999}
P999_MIN_SAMPLES = 1000


def gen_events(rng):
    events = []
    provs = list(PROVIDERS)
    for _ in range(N_EVENTS):
        prov = rng.choice(provs)
        model, mu = PROVIDERS[prov]
        effort = rng.choice(EFFORTS)
        pool = rng.choice(POOLS)
        hour = rng.randrange(HOURS)
        ms = max(1, round(rng.lognormvariate(mu + EFFORT_SHIFT[effort], 0.6)))
        events.append({
            "hour": hour, "day": hour // 24, "provider": prov, "model": model,
            "effort": effort, "pool": pool, "ms": ms,
        })
    return events


def group_key(e):
    return (e["provider"], e["model"], e["effort"], e["pool"])


def bucket_index(x):
    return math.ceil(math.log(x) / LOG_GAMMA)


def bucket_value(i):
    return 2 * GAMMA ** i / (GAMMA + 1)


def nearest_rank(sorted_vals, q):
    n = len(sorted_vals)
    return sorted_vals[max(1, math.ceil(q * n)) - 1]


def oracle_stats(vals):
    s = sorted(vals)
    n = len(s)
    total = sum(s)
    mean = total / n
    var = sum((x - mean) ** 2 for x in s) / (n - 1) if n > 1 else None
    counts = Counter(s)
    top = max(counts.values())
    out = {
        "count": n, "sum": total, "mean": mean, "min": s[0], "max": s[-1],
        "stddev": math.sqrt(var) if var is not None else None,
        "mode": min(v for v, c in counts.items() if c == top),
    }
    for name, q in QUANTILES.items():
        out[name] = nearest_rank(s, q)
    out["iqr"] = out["q3"] - out["q1"]
    return out


class State:
    __slots__ = ("n", "sum", "min", "max", "mean", "m2", "hist", "vals")

    def __init__(self):
        self.n = 0
        self.sum = 0
        self.min = None
        self.max = None
        self.mean = 0.0
        self.m2 = 0.0
        self.hist = Counter()
        self.vals = Counter()

    def add(self, x):
        self.n += 1
        self.sum += x
        self.min = x if self.min is None else min(self.min, x)
        self.max = x if self.max is None else max(self.max, x)
        delta = x - self.mean
        self.mean += delta / self.n
        self.m2 += delta * (x - self.mean)
        self.hist[bucket_index(x)] += 1
        self.vals[x] += 1


def merge(a, b):
    """Chan et al. parallel combination of two partial states; returns a new State."""
    m = State()
    m.n = a.n + b.n
    if m.n == 0:
        return m
    m.sum = a.sum + b.sum
    m.min = b.min if a.min is None else (a.min if b.min is None else min(a.min, b.min))
    m.max = b.max if a.max is None else (a.max if b.max is None else max(a.max, b.max))
    if a.n and b.n:
        delta = b.mean - a.mean
        m.mean = a.mean + delta * b.n / m.n
        m.m2 = a.m2 + b.m2 + delta * delta * a.n * b.n / m.n
    else:
        src = a if a.n else b
        m.mean, m.m2 = src.mean, src.m2
    m.hist = a.hist + b.hist
    m.vals = a.vals + b.vals
    return m


def state_stats(st):
    n = st.n
    counts = st.vals
    top = max(counts.values())
    out = {
        "count": n, "sum": st.sum, "min": st.min, "max": st.max,
        "mean": st.sum / n,
        "stddev": math.sqrt(st.m2 / (n - 1)) if n > 1 else None,
        "mode": min(v for v, c in counts.items() if c == top),
    }
    for name, q in QUANTILES.items():
        rank = max(1, math.ceil(q * n))
        cum = 0
        for i in sorted(st.hist):
            cum += st.hist[i]
            if cum >= rank:
                out[name] = bucket_value(i)
                break
    out["iqr"] = out["q3"] - out["q1"]
    return out


def state_bytes(st):
    return len(json.dumps({
        "n": st.n, "sum": st.sum, "min": st.min, "max": st.max,
        "mean": st.mean, "m2": st.m2,
        "hist": {str(k): v for k, v in st.hist.items()},
        "vals": {str(k): v for k, v in st.vals.items()},
    }, separators=(",", ":")))


def run_seed(seed, report):
    rng = random.Random(seed)
    events = gen_events(rng)
    failures = []

    hour_cells = {}
    for e in events:
        key = (e["hour"], group_key(e))
        if key not in hour_cells:
            hour_cells[key] = State()
        hour_cells[key].add(e["ms"])

    day_groups = defaultdict(list)
    for (hour, gk), st in hour_cells.items():
        day_groups[(hour // 24, gk)].append((hour, st))

    raw_by_cell = defaultdict(list)
    for e in events:
        raw_by_cell[(e["day"], group_key(e))].append(e["ms"])

    total_count = 0
    for cell, parts in day_groups.items():
        rng.shuffle(parts)
        merged = State()
        for _, st in parts:
            merged = merge(merged, st)
        samples = raw_by_cell[cell]
        oracle = oracle_stats(samples)
        got = state_stats(merged)
        total_count += got["count"]

        for k in ("count", "sum", "min", "max", "mode"):
            if got[k] != oracle[k]:
                failures.append((seed, cell, k, got[k], oracle[k]))
        for k in ("mean", "stddev"):
            if oracle[k] is None:
                if got[k] is not None:
                    failures.append((seed, cell, k, got[k], None))
            elif not math.isclose(got[k], oracle[k], rel_tol=1e-9, abs_tol=1e-12):
                failures.append((seed, cell, k, got[k], oracle[k]))

        for name in QUANTILES:
            exact = oracle[name]
            approx = got[name]
            rel = abs(approx - exact) / exact
            report["max_rel_err"][name] = max(report["max_rel_err"][name], rel)
            if rel > ALPHA + 1e-9:
                failures.append((seed, cell, name, approx, exact))

        if oracle["count"] < P999_MIN_SAMPLES:
            report["p999_undetermined_cells"] += 1
        report["cells"] += 1

    if total_count != len(events):
        failures.append((seed, "conservation", total_count, len(events)))

    if seed == 0:
        raw_bytes = sum(len(json.dumps(e, separators=(",", ":"))) for e in events)
        hour_bytes = sum(state_bytes(st) for st in hour_cells.values())
        day_states = {}
        for (day, gk), parts in day_groups.items():
            m = State()
            for _, st in parts:
                m = merge(m, st)
            day_states[(day, gk)] = m
        day_bytes = sum(state_bytes(st) for st in day_states.values())
        report["storage"] = {
            "events": len(events),
            "raw_bytes": raw_bytes,
            "hour_cells": len(hour_cells),
            "hour_state_bytes": hour_bytes,
            "day_cells": len(day_states),
            "day_state_bytes": day_bytes,
        }
    return failures


def main():
    seeds = int(sys.argv[1]) if len(sys.argv) > 1 else 200
    if seeds < 200:
        print("note: fewer than 200 seeds requested; the property target is >= 200")
    report = {
        "max_rel_err": {k: 0.0 for k in QUANTILES},
        "p999_undetermined_cells": 0,
        "cells": 0,
        "storage": {},
    }
    all_failures = []
    for seed in range(seeds):
        all_failures.extend(run_seed(seed, report))

    print(f"seeds={seeds} events_per_seed={N_EVENTS} hours={HOURS} alpha={ALPHA}")
    print(f"day-group cells checked={report['cells']}")
    print("invariants: conservation, exact merge (count/sum/min/max/mode), "
          "float merge (mean/stddev, rel 1e-9), quantile rel err <= alpha")
    print("max observed relative error per quantile:")
    for name in QUANTILES:
        print(f"  {name:>6}: {report['max_rel_err'][name]:.6f}  (bound {ALPHA})")
    print(f"cells with n < {P999_MIN_SAMPLES} (p99.9 not resolvable): "
          f"{report['p999_undetermined_cells']} of {report['cells']}")
    st = report["storage"]
    print(f"storage (seed 0): events={st['events']} raw={st['raw_bytes']}B "
          f"hour_cells={st['hour_cells']} hour_states={st['hour_state_bytes']}B "
          f"day_cells={st['day_cells']} day_states={st['day_state_bytes']}B")
    print(f"  raw bytes/event={st['raw_bytes'] / st['events']:.1f}; "
          f"day-state/raw={st['day_state_bytes'] / st['raw_bytes']:.3f}")
    if all_failures:
        print(f"FAIL: {len(all_failures)} invariant violations, first 5:")
        for f in all_failures[:5]:
            print("  ", f)
        sys.exit(1)
    print("PASS")


if __name__ == "__main__":
    main()
