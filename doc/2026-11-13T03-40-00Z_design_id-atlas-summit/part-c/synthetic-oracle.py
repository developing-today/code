#!/usr/bin/env python3
"""Synthetic-only cost arithmetic oracle; no provider calls.

CLI contract: no arguments; reject extras before calculating or emitting output.
"""
import json
import sys


FIXTURE = {
    "label": "synthetic-not-measured",
    "d1": {"rows_read": 2_000_000_000, "rows_written": 20_000_000,
           "read_included": 25_000_000_000, "write_included": 50_000_000,
           "read_usd_per_million": 0.001, "write_usd_per_million": 1.0},
    "r2": {"gb_month": 100, "class_a": 2_000_000, "class_b": 30_000_000,
           "storage_usd_per_gb_month": 0.015, "class_a_usd_per_million": 4.50,
           "class_b_usd_per_million": 0.36},
}


def cost_per_million(units, included, rate):
    assert units >= 0 and included >= 0 and rate >= 0
    return max(0, units - included) * rate / 1_000_000


def calculate(data):
    d1, r2 = data["d1"], data["r2"]
    assert d1["rows_read"] >= 0 and d1["rows_written"] >= 0
    assert r2["gb_month"] >= 0 and r2["class_a"] >= 0 and r2["class_b"] >= 0
    # Oracle: stated synthetic units, included allowances, and published-rate arithmetic.
    d1_usd = cost_per_million(d1["rows_read"], d1["read_included"],
                              d1["read_usd_per_million"])
    d1_usd += cost_per_million(d1["rows_written"], d1["write_included"],
                               d1["write_usd_per_million"])
    r2_usd = r2["gb_month"] * r2["storage_usd_per_gb_month"]
    r2_usd += r2["class_a"] / 1_000_000 * r2["class_a_usd_per_million"]
    r2_usd += r2["class_b"] / 1_000_000 * r2["class_b_usd_per_million"]
    # Fixture sanity: all displayed totals must be nonnegative and exactly reproducible.
    assert d1_usd == 0 and abs(r2_usd - 21.3) < 1e-9
    return {"label": data["label"], "measurement_status": "not measured",
            "basis": "synthetic counters; rates copied from cited docs, not a quote",
            "d1_synthetic_usd": round(d1_usd, 4),
            "r2_synthetic_usd": round(r2_usd, 4),
            "winner": None}


if __name__ == "__main__":
    if len(sys.argv) != 1:
        print("usage: synthetic-oracle.py", file=sys.stderr)
        raise SystemExit(2)
    print(json.dumps(calculate(FIXTURE), sort_keys=True, indent=2))
