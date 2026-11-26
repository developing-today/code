#!/usr/bin/env python3
"""Due-cohort export pass and run record for the Turso backup mesh (#13456, slice 2).

Consumes the slice-1 planner's dry-run plan (``turso-mesh-cohorts.py``)
and runs one read-only export per database in the due cohort. Every
database either lands in the run record's ``exported`` list with its
snapshot id and export time, or in its ``retry`` list with the reason.
A database that fails, or that the exporter cannot find, is never a
silent skip: it stays in the retry list, and the next pass includes it
again when a previous run record is supplied with ``--retry``.

This slice writes nothing to R2 or Drive and holds no credential. The
exporter is an external command supplied with ``--exporter``; the pass
runs it with the database name as its only argument, never through a
shell. Its contract:

* exit 0 and one JSON object on stdout: ``{"snapshot_id": "...",
  "exported_at": "<ISO-8601>"}``;
* any other outcome (non-zero exit, unparseable output, timeout,
  missing executable) is a miss for that database.

Run record (JSON on stdout or ``--out``)::

    {"date": "2026-11-24", "cohort": 3, "ok": false,
     "databases": ["..."], "exported": [{"database", "snapshot_id",
     "exported_at"}], "retry": [{"database", "reason"}],
     "excluded_class_a": ["atlas", ...]}

Exit 0 when nothing is in the retry list, 1 when something is, and 2 on
a usage error (unreadable plan or record, bad arguments).

Usage:
    turso-mesh-cohorts.py --inventory dbs.txt --date 2026-11-24 > plan.json
    turso-mesh-export-pass.py --plan plan.json --exporter ./export-one.sh \\
        [--retry previous-run.json] [--timeout 3600] [--out run.json]

Argument contract (#3352): the whole command line is declared in
``main()`` and parsed strictly with argparse before any subprocess; an
unknown flag or stray positional is a usage error, exit 2.
"""

import argparse
import json
import subprocess
import sys
from datetime import datetime, timezone

CLASS_A = ("atlas", "fleet-platform", "fleet-queue")


class Miss(Exception):
    """One database's export did not produce a snapshot."""


def _run_exporter(exporter, name, timeout):
    try:
        result = subprocess.run(
            list(exporter) + [name],
            capture_output=True,
            timeout=timeout,
            check=False,
        )
    except subprocess.TimeoutExpired:
        raise Miss("exporter timed out after %ss" % timeout)
    except OSError as exc:
        raise Miss("exporter could not start: %s" % exc.strerror)
    if result.returncode != 0:
        raise Miss("exporter exited %d" % result.returncode)
    try:
        payload = json.loads(result.stdout.decode("utf-8"))
    except (ValueError, UnicodeDecodeError):
        raise Miss("exporter output is not JSON")
    snapshot = payload.get("snapshot_id") if isinstance(payload, dict) else None
    exported_at = payload.get("exported_at") if isinstance(payload, dict) else None
    if not snapshot or not exported_at:
        raise Miss("exporter output lacks snapshot_id or exported_at")
    return {"snapshot_id": str(snapshot), "exported_at": str(exported_at)}


def run_pass(plan, prior, export, now):
    """Export the due cohort plus any carried-over retries; build the record.

    ``export(name)`` returns ``{"snapshot_id", "exported_at"}`` or raises
    ``Miss``. Any other exception is recorded as a miss too, so one
    database cannot abort the pass.
    """
    due = list(plan["databases"])
    carried = [entry["database"] for entry in (prior or {}).get("retry", [])]
    names = sorted(set(due) | set(carried))
    exported = []
    retry = []
    for name in names:
        if name in CLASS_A:
            retry.append({"database": name, "reason": "class-A database in a class-B pass"})
            continue
        try:
            got = export(name)
        except Miss as exc:
            retry.append({"database": name, "reason": str(exc)})
        except Exception as exc:
            retry.append({"database": name, "reason": "unexpected error: %s" % type(exc).__name__})
        else:
            exported.append({"database": name, **got})
    return {
        "date": plan["date"],
        "cohort": plan["cohort"],
        "generated_at": now,
        "ok": not retry,
        "databases": names,
        "exported": exported,
        "retry": retry,
        "excluded_class_a": plan.get("excluded_class_a", []),
    }


def _load_json(path, label):
    try:
        with open(path, encoding="utf-8") as handle:
            return json.load(handle)
    except (OSError, ValueError) as exc:
        raise ValueError("cannot read %s %s: %s" % (label, path, exc))


def main(argv=None):
    parser = argparse.ArgumentParser(
        description="Run one due-cohort Turso export pass and write its run record (#13456).",
        allow_abbrev=False,
    )
    parser.add_argument("--plan", required=True, metavar="FILE", help="planner JSON (turso-mesh-cohorts.py output)")
    parser.add_argument("--exporter", required=True, metavar="CMD", help="read-only exporter executable; run as CMD DATABASE")
    parser.add_argument("--retry", metavar="FILE", help="previous run record whose retry list is included in this pass")
    parser.add_argument("--timeout", type=int, default=3600, metavar="SECS", help="per-database exporter timeout (default 3600)")
    parser.add_argument("--out", metavar="FILE", help="write the run record here instead of stdout")
    args = parser.parse_args(argv)

    if args.timeout <= 0:
        print("turso-mesh-export-pass: --timeout must be positive", file=sys.stderr)
        return 2
    try:
        plan = _load_json(args.plan, "plan")
        prior = _load_json(args.retry, "run record") if args.retry else None
    except ValueError as exc:
        print("turso-mesh-export-pass: %s" % exc, file=sys.stderr)
        return 2
    if not isinstance(plan, dict) or "databases" not in plan or "date" not in plan or "cohort" not in plan:
        print("turso-mesh-export-pass: plan is not a planner dry-run output", file=sys.stderr)
        return 2

    exporter = [args.exporter]
    record = run_pass(
        plan,
        prior,
        lambda name: _run_exporter(exporter, name, args.timeout),
        datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    )
    text = json.dumps(record, indent=2, sort_keys=True) + "\n"
    if args.out:
        with open(args.out, "w", encoding="utf-8") as handle:
            handle.write(text)
    else:
        sys.stdout.write(text)
    return 0 if record["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())
