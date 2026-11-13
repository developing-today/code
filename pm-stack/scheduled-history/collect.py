#!/usr/bin/env python3
"""collect.py — per-run history for OpenChamber scheduled tasks (#2174 Gap 1).

Design: a POLLING RECONCILER, not an event consumer. OpenChamber's
scheduled-task-ran events are ephemeral and emitter errors are
swallowed, and the events that do exist report enqueue/send rather
than assistant completion — an SSE-only collector cannot guarantee a
durable terminal record (recon on issue #2174, task 12585). The
authoritative surface is the scheduled-task record itself
(`openchamber schedule list --json`), which retains the latest run's
fields. Polling it on a short tick and keying runs by
(task id, lastScheduledFor) yields exactly one durable row per run:

- a run key first seen is inserted with its observed status;
- while a newer run has not appeared, the row's outcome follows the
  record until the status is terminal, then the row is frozen;
- when a newer run for the same task appears, any older non-terminal
  row is finalized with its last observed status
  (finalized_by=superseded) — it can never update again, because the
  record no longer describes it;
- token/cost fields are enriched from the OpenCode session store
  (session_v2 in the opencode.db the #2072 exporter reads), keyed by
  the run's session id, whenever that row exists; otherwise they
  stay null — absence is recorded, never invented.

The store is one JSON document (rows keyed by run key) written
atomically (tmp file + os.replace). At fleet scale (tens of tasks,
tens of runs per day) this is far below any size where a database
earns its keep; the row shape is the contract, not the container.

Read path: `collect.py list [--task ID] [--since ISO] [--until ISO]
[--json]` — the input the #2174 comparison and the Gap 3 watchdog
check consume.

This tool changes NO existing schedule: it only reads the
scheduled-task records. Installing its own timer is the deployment
step recorded in README.md.
"""

import argparse, json, os, sqlite3, subprocess, sys, tempfile
from datetime import datetime, timezone

TERMINAL_STATUSES = {
    "success", "error", "failed", "failure", "cancelled",
    "canceled", "timeout", "timed_out", "disabled", "skipped",
}

DEFAULT_STORE = os.path.join(
    os.path.expanduser("~/.local/state/openchamber-scheduled-history"),
    "history.json",
)
DEFAULT_DB = os.path.expanduser("~/.local/share/opencode/opencode.db")


def iso(ms):
    if ms is None:
        return None
    return datetime.fromtimestamp(ms / 1000, tz=timezone.utc).isoformat()


def now_iso(clock=None):
    t = clock() if clock else datetime.now(timezone.utc)
    return t.isoformat()


def fetch_tasks(project_dir, runner=subprocess.run):
    out = runner(
        ["openchamber", "schedule", "list", "--json", "--dir", project_dir],
        capture_output=True, text=True, timeout=120,
    )
    if out.returncode != 0:
        raise RuntimeError(f"openchamber schedule list failed for {project_dir}: {out.stderr.strip()[:200]}")
    doc = json.loads(out.stdout)
    if doc.get("status") != "ok":
        raise RuntimeError(f"openchamber schedule list error for {project_dir}: {doc.get('error')}")
    return doc.get("tasks", [])


def session_lookup(db_path, session_id):
    """Token/cost totals for one session from session_v2, or None."""
    if not session_id or not db_path or not os.path.exists(db_path):
        return None
    try:
        con = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
        try:
            row = con.execute(
                "SELECT cost, tokens_input, tokens_output, tokens_reasoning,"
                " tokens_cache_read, tokens_cache_write, time_created, time_updated"
                " FROM session_v2 WHERE id = ?", (session_id,),
            ).fetchone()
        finally:
            con.close()
    except sqlite3.Error:
        return None
    if row is None:
        return None
    cost, ti, to, tr, tcr, tcw, created, updated = row
    return {
        "cost": cost,
        "tokens": {
            "input": ti, "output": to, "reasoning": tr,
            "cache_read": tcr, "cache_write": tcw,
        },
        "session_created_at": iso(created),
        "session_updated_at": iso(updated),
        "token_source": "opencode session_v2",
    }


def reconcile(rows, tasks, project_dir, clock=None, lookup=None):
    """Fold one poll of one project's tasks into the row store.

    rows: {run_key: row} mutated in place. Returns (inserted, updated,
    finalized) counts. lookup(session_id) -> enrichment dict or None.
    """
    seen_at = now_iso(clock)
    inserted = updated = finalized = 0
    latest_key_by_task = {}
    for task in tasks:
        state = task.get("state") or {}
        run_at = state.get("lastRunAt")
        scheduled_for = state.get("lastScheduledFor") or run_at
        if run_at is None and scheduled_for is None:
            continue  # never ran — no run, no row
        run_key = f"{task.get('id')}@{scheduled_for}"
        latest_key_by_task[task.get("id")] = run_key
        status = (state.get("lastStatus") or "unknown").lower()
        terminal = status in TERMINAL_STATUSES
        row = rows.get(run_key)
        if row is None:
            row = {
                "run_key": run_key,
                "task_id": task.get("id"),
                "task_name": task.get("name"),
                "project_dir": project_dir,
                "enabled": task.get("enabled"),
                "scheduled_for": iso(scheduled_for),
                "started_at": iso(run_at),
                "finished_at": None,
                "duration_ms": state.get("lastDurationMs"),
                "session_id": state.get("lastSessionId"),
                "outcome": status,
                "outcome_terminal": terminal,
                "finalized_by": "terminal-status" if terminal else None,
                "cost": None,
                "tokens": None,
                "session_created_at": None,
                "session_updated_at": None,
                "token_source": None,
                "first_seen_at": seen_at,
                "last_seen_at": seen_at,
            }
            rows[run_key] = row
            inserted += 1
        elif not row.get("outcome_terminal"):
            row["outcome"] = status
            row["outcome_terminal"] = terminal
            row["duration_ms"] = state.get("lastDurationMs")
            row["session_id"] = state.get("lastSessionId") or row.get("session_id")
            row["last_seen_at"] = seen_at
            if terminal:
                row["finalized_by"] = "terminal-status"
            updated += 1
        if row.get("outcome_terminal") and row.get("finished_at") is None:
            if row.get("started_at") and row.get("duration_ms") is not None:
                start = datetime.fromisoformat(row["started_at"])
                row["finished_at"] = datetime.fromtimestamp(
                    start.timestamp() + row["duration_ms"] / 1000, tz=timezone.utc
                ).isoformat()
        if lookup is not None and row.get("tokens") is None and row.get("session_id"):
            got = lookup(row["session_id"])
            if got:
                row.update(got)
    # Finalize non-terminal rows superseded by a newer run of the task.
    for key, row in rows.items():
        if row.get("outcome_terminal") or row.get("project_dir") != project_dir:
            continue
        latest = latest_key_by_task.get(row.get("task_id"))
        if latest is not None and latest != key:
            row["outcome_terminal"] = True
            row["finalized_by"] = "superseded"
            row["last_seen_at"] = seen_at
            finalized += 1
    return inserted, updated, finalized


def load_store(path):
    if os.path.exists(path):
        with open(path) as f:
            doc = json.load(f)
        return doc.get("rows", {})
    return {}


def save_store(path, rows):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    doc = {"schema": "scheduled-history/v1", "rows": rows}
    fd, tmp = tempfile.mkstemp(dir=os.path.dirname(path), suffix=".tmp")
    try:
        with os.fdopen(fd, "w") as f:
            json.dump(doc, f, indent=1, sort_keys=True)
        os.replace(tmp, path)
    except BaseException:
        if os.path.exists(tmp):
            os.unlink(tmp)
        raise


def cmd_collect(args):
    rows = load_store(args.store)
    totals = [0, 0, 0]
    for project_dir in args.dir:
        tasks = fetch_tasks(project_dir)
        counts = reconcile(
            rows, tasks, project_dir,
            lookup=lambda sid: session_lookup(args.opencode_db, sid),
        )
        totals = [a + b for a, b in zip(totals, counts)]
    save_store(args.store, rows)
    print(json.dumps({
        "rows_total": len(rows), "inserted": totals[0],
        "updated": totals[1], "finalized_superseded": totals[2],
        "store": args.store,
    }))
    return 0


def cmd_list(args):
    rows = load_store(args.store)
    out = []
    for row in rows.values():
        if args.task and row.get("task_id") != args.task:
            continue
        started = row.get("started_at") or ""
        if args.since and started < args.since:
            continue
        if args.until and started > args.until:
            continue
        out.append(row)
    out.sort(key=lambda r: r.get("started_at") or "")
    if args.json:
        print(json.dumps(out, indent=1))
    else:
        for r in out:
            tok = r.get("tokens") or {}
            print(
                f"{r.get('started_at')}  {r.get('task_name')}  outcome={r.get('outcome')}"
                f"  terminal={r.get('outcome_terminal')}  session={r.get('session_id')}"
                f"  tokens_in={tok.get('input')} tokens_out={tok.get('output')}"
            )
        print(f"{len(out)} run(s)")
    return 0


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--store", default=DEFAULT_STORE)
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("collect", help="one polling tick over the given projects")
    c.add_argument("--dir", action="append", required=True,
                   help="project directory (repeatable)")
    c.add_argument("--opencode-db", default=DEFAULT_DB)
    c.set_defaults(fn=cmd_collect)
    l = sub.add_parser("list", help="query stored runs")
    l.add_argument("--task", help="scheduled task id")
    l.add_argument("--since", help="ISO timestamp, inclusive lower bound on started_at")
    l.add_argument("--until", help="ISO timestamp, inclusive upper bound on started_at")
    l.add_argument("--json", action="store_true")
    l.set_defaults(fn=cmd_list)
    args = ap.parse_args(argv)
    return args.fn(args)


if __name__ == "__main__":
    sys.exit(main())
