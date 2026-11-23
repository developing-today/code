#!/usr/bin/env python3
"""Daily plan reconciliation diff (#1270, developing-today/code).

Three records describe the same plans and drift independently:
  (1) plan store  — opencode plans.json (model-plan.ts): entitlements,
      fees, windows, status.
  (2) tracker     — the fenced ```plan-facts JSON block in
      provider-quota-registry.md (keeper-maintained; schema approved
      2026-11-16, ledger extracted-blocked-review-drew-decision-01-13).
  (3) measured    — provider-usage-snapshot.json (check-usage.py):
      the latest measured reality.

Rulings (2026-11-16): a MISSING plan store is a failure, not an empty
plan set. A present-but-empty store is a valid (empty) leg.

Evidence order (winning source): console/measured > plan store >
tracker prose. The diff reports ONLY mismatches — plan identity, fee,
window limit/reset, status — each naming the winning source. It never
edits any source: corrections flow through normal edit/PR paths with
the diff attached.

Exit codes: 0 = ran (findings or no-drift printed); 2 = missing plan
store (failure finding); 3 = unreadable input.
"""

import argparse, json, re, sys
from pathlib import Path

FIELDS = ("fee_amount", "fee_period", "window", "limit", "reset", "status")

# The plan-store Effect schema only admits monthly/yearly/once, while
# the tracker vocabulary is month/year (#13375). Normalise the store
# leg to the tracker vocabulary at load, so the diff compares the same
# fact instead of reporting the spelling as permanent drift. Values
# outside the map (e.g. "once", or anything new) pass through
# untouched, so a genuinely new period still reports drift.
STORE_PERIOD_TO_TRACKER = {"monthly": "month", "yearly": "year"}


def load_store(path):
    p = Path(path)
    if not p.exists():
        return None  # missing store = failure per 2026-11-16 ruling
    d = json.loads(p.read_text())
    rows = {}
    for plan in d.get("plans", []):
        quotas = plan.get("quotas") or [{}]
        q = quotas[0]
        fee = plan.get("fee") or {}
        rows[plan["id"]] = {
            "plan": plan.get("plan") or plan["id"],
            "provider": plan.get("providerID"),
            "fee_amount": fee.get("amount"),
            "fee_period": STORE_PERIOD_TO_TRACKER.get(fee.get("period"), fee.get("period")),
            "window": q.get("windowHours"),
            "limit": q.get("limit"),
            "reset": q.get("resetRule"),
            "status": plan.get("status"),
        }
    return rows


def load_tracker(path):
    text = Path(path).read_text()
    m = re.search(r"```plan-facts\s*\n(.*?)```", text, re.S)
    if not m:
        return {}
    rows = {}
    for r in json.loads(m.group(1)).get("rows", []):
        rows[r["plan"]] = {k: r.get(k) for k in ("provider",) + FIELDS} | {
            "plan": r["plan"]
        }
    return rows


def load_measured(path):
    p = Path(path)
    if not p.exists():
        return {}
    d = json.loads(p.read_text())
    rows = {}
    for pid, prov in (d.get("providers") or {}).items():
        if isinstance(prov, dict) and prov.get("plan"):
            rows[prov["plan"]] = {
                "plan": prov["plan"],
                "provider": pid,
                "status": prov.get("status"),
            }
    return rows


def diff(store, tracker, measured):
    findings = []
    keys = sorted(set(store) | set(tracker) | set(measured))
    for k in keys:
        s, t, m = store.get(k), tracker.get(k), measured.get(k)
        present = [
            n for n, r in (("measured", m), ("plan-store", s), ("tracker", t)) if r
        ]
        if len(present) < 2:
            only = present[0] if present else "none"
            findings.append(
                {
                    "plan": k,
                    "field": "presence",
                    "detail": f"present only in {only}",
                    "winner": present[0] if present else None,
                }
            )
            continue
        # measured wins over store wins over tracker, field by field
        for f in FIELDS:
            vals = {
                n: r.get(f)
                for n, r in (("measured", m), ("plan-store", s), ("tracker", t))
                if r and r.get(f) is not None
            }
            if len(set(map(str, vals.values()))) > 1:
                winner = next(
                    n for n in ("measured", "plan-store", "tracker") if n in vals
                )
                findings.append(
                    {"plan": k, "field": f, "values": vals, "winner": winner}
                )
    return findings


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--store", required=True)
    ap.add_argument("--tracker", required=True)
    ap.add_argument("--measured", required=True)
    a = ap.parse_args()
    store = load_store(a.store)
    if store is None:
        print(
            json.dumps(
                {
                    "result": "FAILURE",
                    "findings": [
                        {
                            "plan": "*",
                            "field": "plan-store",
                            "detail": f"plan store missing at {a.store} — failure, not an empty plan set (ruling 2026-11-16)",
                            "winner": None,
                        }
                    ],
                }
            )
        )
        return 2
    try:
        tracker, measured = load_tracker(a.tracker), load_measured(a.measured)
    except Exception as e:
        print(f"unreadable input: {e}", file=sys.stderr)
        return 3
    findings = diff(store, tracker, measured)
    print(
        json.dumps(
            {
                "result": "findings" if findings else "no-drift",
                "legs": {
                    "plan-store": len(store),
                    "tracker": len(tracker),
                    "measured": len(measured),
                },
                "findings": findings,
            },
            indent=1,
        )
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
