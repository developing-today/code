#!/usr/bin/env python3
"""test_collect.py — reconcile/list tests for the scheduled-history
collector (#2174 Gap 1). Oracle: the acceptance criteria — exactly
one durable row per run, terminal outcome recorded, superseded runs
finalized, token enrichment only from the session store.

Run: python3 pm-stack/scheduled-history/test_collect.py
"""

import json, os, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import collect

PASS = []


def check(name, cond):
    PASS.append((name, bool(cond)))
    print(("PASS " if cond else "FAIL ") + name)
    assert cond, name


T0 = 1794362400000  # a scheduled tick, ms epoch


def task(
    status="success", run_at=T0, sched=T0, session="ses_1", dur=5000, tid="task-1"
):
    return {
        "id": tid,
        "name": "Lane",
        "enabled": True,
        "state": {
            "lastStatus": status,
            "lastRunAt": run_at,
            "lastScheduledFor": sched,
            "lastSessionId": session,
            "lastDurationMs": dur,
        },
    }


# 1. first poll inserts exactly one row with the terminal outcome
rows = {}
ins, upd, fin = collect.reconcile(rows, [task()], "/proj")
check("insert: one row", len(rows) == 1 and ins == 1)
row = rows["task-1@%d" % T0]
check(
    "insert: outcome success + terminal",
    row["outcome"] == "success" and row["outcome_terminal"],
)
check("insert: finished_at = start + duration", row["finished_at"] is not None)
check("insert: session id carried", row["session_id"] == "ses_1")

# 2. re-polling the same run inserts nothing and changes nothing
ins, upd, fin = collect.reconcile(rows, [task()], "/proj")
check("repoll: no duplicate", len(rows) == 1 and ins == 0)

# 3. a running run updates in place to its terminal status
rows = {}
collect.reconcile(rows, [task(status="running")], "/proj")
check("running: not terminal", not rows["task-1@%d" % T0]["outcome_terminal"])
collect.reconcile(rows, [task(status="error")], "/proj")
r = rows["task-1@%d" % T0]
check(
    "running->error: same row, terminal",
    len(rows) == 1 and r["outcome"] == "error" and r["outcome_terminal"],
)

# 4. a newer run supersedes: the older non-terminal row is finalized
rows = {}
collect.reconcile(rows, [task(status="running")], "/proj")
collect.reconcile(
    rows,
    [task(status="running", run_at=T0 + 2400000, sched=T0 + 2400000, session="ses_2")],
    "/proj",
)
old = rows["task-1@%d" % T0]
check(
    "supersede: old row finalized",
    old["outcome_terminal"] and old["finalized_by"] == "superseded",
)
check("supersede: two rows total", len(rows) == 2)

# 5. token enrichment comes from the lookup, only when it has the session
rows = {}
lookup = lambda sid: (
    {
        "cost": 0.5,
        "tokens": {
            "input": 10,
            "output": 3,
            "reasoning": 0,
            "cache_read": 0,
            "cache_write": 0,
        },
        "session_created_at": None,
        "session_updated_at": None,
        "token_source": "opencode session_v2",
    }
    if sid == "ses_1"
    else None
)
collect.reconcile(rows, [task()], "/proj", lookup=lookup)
check(
    "tokens: enriched when session found",
    rows["task-1@%d" % T0]["tokens"]["input"] == 10,
)
rows = {}
collect.reconcile(rows, [task(session="ses_missing")], "/proj", lookup=lookup)
check(
    "tokens: null when session absent (recorded, not invented)",
    rows["task-1@%d" % T0]["tokens"] is None,
)

# 6. a task that never ran produces no row
rows = {}
t = task()
t["state"] = {}
collect.reconcile(rows, [t], "/proj")
check("never-ran: no row", rows == {})

# 7. store round-trips through save/load (atomic JSON document)
import tempfile

p = os.path.join(tempfile.mkdtemp(), "history.json")
rows = {}
collect.reconcile(rows, [task()], "/proj")
collect.save_store(p, rows)
check("store: round-trip", collect.load_store(p) == rows)

# 8. enrichment refreshes on later polls (Codex P1 on PR #8437): a
# session still accumulating tokens must not freeze at first sight,
# even after the run's outcome is terminal
rows = {}
vals = {"input": 10}
lookup = lambda sid: {
    "cost": 0.5,
    "tokens": {
        "input": vals["input"],
        "output": 3,
        "reasoning": 0,
        "cache_read": 0,
        "cache_write": 0,
    },
    "session_created_at": None,
    "session_updated_at": None,
    "token_source": "opencode session_v2",
}
collect.reconcile(rows, [task()], "/proj", lookup=lookup)
check("refresh: first poll enriches", rows["task-1@%d" % T0]["tokens"]["input"] == 10)
vals["input"] = 25
collect.reconcile(rows, [task()], "/proj", lookup=lookup)
check(
    "refresh: later poll updates tokens on a terminal row",
    rows["task-1@%d" % T0]["tokens"]["input"] == 25,
)

# 9. a bare relative store filename saves (Codex P2 on PR #8437)
import tempfile

cwd = os.getcwd()
os.chdir(tempfile.mkdtemp())
try:
    rows = {}
    collect.reconcile(rows, [task()], "/proj")
    collect.save_store("history.json", rows)
    check(
        "store: bare relative filename round-trips",
        collect.load_store("history.json") == rows,
    )
finally:
    os.chdir(cwd)

# 10. list window compares instants, not strings (Codex P2 on PR #8437):
# a bound at 13:00+01:00 is 12:00 UTC, so a 12:30 UTC run is inside
# --since 13:00+01:00 but outside --until 13:00+01:00
import argparse, contextlib, io

sp = os.path.join(tempfile.mkdtemp(), "history.json")
srows = {
    "k": {
        "run_key": "k",
        "task_id": "task-1",
        "task_name": "Lane",
        "started_at": "2026-11-11T12:30:00+00:00",
        "outcome": "success",
        "outcome_terminal": True,
        "session_id": "ses_1",
        "tokens": None,
    }
}
collect.save_store(sp, srows)


def run_list(**kw):
    a = argparse.Namespace(store=sp, task=None, since=None, until=None, json=True)
    for k, v in kw.items():
        setattr(a, k, v)
    buf = io.StringIO()
    with contextlib.redirect_stdout(buf):
        rc = collect.cmd_list(a)
    return rc, json.loads(buf.getvalue())


rc, out = run_list(since="2026-11-11T13:00:00+01:00")
check("window: offset --since compares instants (included)", rc == 0 and len(out) == 1)
rc, out = run_list(until="2026-11-11T13:00:00+01:00")
check("window: offset --until compares instants (excluded)", rc == 0 and len(out) == 0)
rc, out = run_list(since="2026-11-11T12:00:00Z")
check("window: Z --since parses (included)", rc == 0 and len(out) == 1)

print(f"\n{sum(1 for _, ok in PASS if ok)}/{len(PASS)} checks passed")
