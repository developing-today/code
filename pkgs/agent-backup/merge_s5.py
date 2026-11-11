#!/usr/bin/env python3
"""Merge S5 runs+sessions saved-output pages and load mirror_concurrency_hourly.
Usage: merge_s5.py runs=<file> [runs=<file>...] sessions=<file> [...]
Missing side for an hour = 0. Loads via load_rows.py in 200-row chunks."""
import json, subprocess, sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from load_from_output import extract_rows

runs, sessions = {}, {}
for arg in sys.argv[1:]:
    kind, path = arg.split("=", 1)
    target = runs if kind == "runs" else sessions
    col = "max_concurrent_runs" if kind == "runs" else "max_concurrent_sessions"
    for r in extract_rows(path):
        target[int(r["hour_epoch"])] = int(r[col])
hours = sorted(set(runs) | set(sessions))
rows = [{"hour_epoch": h, "max_concurrent_runs": runs.get(h, 0),
         "max_concurrent_sessions": sessions.get(h, 0)} for h in hours]
spec = json.load(open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "spec_s5.json")))
total = 0
for i in range(0, len(rows), 200):
    spec["rows"] = rows[i:i + 200]
    p = subprocess.run([sys.executable, os.path.expanduser("~/tools/runtime-db-export/load_rows.py")],
                       input=json.dumps(spec), text=True, capture_output=True)
    if p.returncode != 0:
        print(p.stdout, p.stderr); sys.exit(1)
    total += len(spec["rows"])
print(f"merged+loaded {total} concurrency rows over {len(hours)} hours")
