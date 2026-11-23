#!/usr/bin/env python3
"""Fixture tests for reconcile.py (#1270 acceptance: a planted mismatch
in a test fixture is caught). Oracle: the diff output must contain
exactly the planted findings and no others."""

import json, subprocess, sys, tempfile
from pathlib import Path

TOOL = Path(__file__).parent / "reconcile.py"


def run(store, tracker_text, measured):
    with tempfile.TemporaryDirectory() as d:
        d = Path(d)
        sp, tp, mp = d / "plans.json", d / "registry.md", d / "snap.json"
        if store is not None:
            sp.write_text(json.dumps(store))
        tp.write_text(tracker_text)
        mp.write_text(json.dumps(measured))
        r = subprocess.run(
            [
                sys.executable,
                str(TOOL),
                "--store",
                str(sp),
                "--tracker",
                str(tp),
                "--measured",
                str(mp),
            ],
            capture_output=True,
            text=True,
        )
        return r


def tracker(rows):
    return "```plan-facts\n" + json.dumps({"rows": rows}) + "\n```\n"


STORE = {
    # NB (#13375): the store fixture carries "monthly" — the value the
    # real store schema admits (monthly/yearly/once). The old "month"
    # masked the vocabulary mismatch: the tracker says month/year while
    # the store says monthly/yearly, and the diff must not report that
    # as drift. Test 1 below is the no-drift proof.
    "plans": [
        {
            "id": "go-personal",
            "providerID": "opencode-go",
            "plan": "go",
            "fee": {"amount": 10, "period": "monthly"},
            "quotas": [
                {
                    "name": "monthly",
                    "limit": 100,
                    "windowHours": 720,
                    "resetRule": "calendar",
                }
            ],
            "status": "active",
        }
    ]
}
TRACK_OK = tracker(
    [
        {
            "plan": "go-personal",
            "provider": "opencode-go",
            "fee_amount": 10,
            "fee_period": "month",
            "window": 720,
            "limit": 100,
            "reset": "calendar",
            "status": "active",
        }
    ]
)

# 1. agreeing legs -> no-drift
r = run(STORE, TRACK_OK, {"providers": {}})
out = json.loads(r.stdout)
assert r.returncode == 0 and out["result"] == "no-drift", out

# 2. PLANTED MISMATCH: tracker fee 20 vs store fee 10 -> caught, store wins
bad = tracker(
    [
        {
            "plan": "go-personal",
            "provider": "opencode-go",
            "fee_amount": 20,
            "fee_period": "month",
            "window": 720,
            "limit": 100,
            "reset": "calendar",
            "status": "active",
        }
    ]
)
r = run(STORE, bad, {"providers": {}})
out = json.loads(r.stdout)
assert out["result"] == "findings" and len(out["findings"]) == 1, out
f = out["findings"][0]
assert f["field"] == "fee_amount" and f["winner"] == "plan-store", f

# 3. missing store file -> FAILURE (ruling: not an empty plan set)
r = run(None, TRACK_OK, {"providers": {}})
out = json.loads(r.stdout)
assert r.returncode == 2 and out["result"] == "FAILURE", out

# 4. plan only in tracker -> presence finding
r = run({"plans": []}, TRACK_OK, {"providers": {}})
out = json.loads(r.stdout)
assert any(x["field"] == "presence" for x in out["findings"]), out

# 5. GENUINE PERIOD MISMATCH (#13375): store monthly vs tracker year is
# real drift, not vocabulary noise -> caught, store wins
mismatch = tracker(
    [
        {
            "plan": "go-personal",
            "provider": "opencode-go",
            "fee_amount": 10,
            "fee_period": "year",
            "window": 720,
            "limit": 100,
            "reset": "calendar",
            "status": "active",
        }
    ]
)
r = run(STORE, mismatch, {"providers": {}})
out = json.loads(r.stdout)
assert out["result"] == "findings" and len(out["findings"]) == 1, out
f = out["findings"][0]
assert f["field"] == "fee_period" and f["winner"] == "plan-store", f

print("all reconcile fixture tests pass")
