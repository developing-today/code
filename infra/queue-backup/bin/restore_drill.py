#!/usr/bin/env python3
"""Restore drill for the fleet-queue D1 backup: D1 copy -> restored store.

Command line: --dump PATH --manifest PATH --out PATH [--worker ID]
[--allow-empty]. The drill loads the export dump into a scratch database
(the D1 copy), checks it against the manifest (dump hash, per-table
counts, schema fingerprint), restores it into --out (the scratch Turso
stand-in: a libSQL local database file), re-checks counts and schema,
then proves the restored store is claimable by running the queue's claim
shape — the conditional UPDATE of turso_queue/claim.rs (queued branch of
the pick predicate in lifecycle.rs) plus its leases row — against it:
exactly the first claimable task is claimed, exactly once. The drill
writes only to scratch paths it is handed; it never touches a live store.
The production form of this drill (a broker claiming against a real
scratch Turso database loaded from the real D1 copy) is in the runbook.
"""

import argparse
import hashlib
import json
import os
import sqlite3
import sys
import tempfile
from datetime import datetime, timedelta, timezone

import queue_backup_common as qb


class DrillError(Exception):
    """A drill check failed; the backup path is not proven."""


LEASE_SECONDS = 2700  # the broker's default claim lease (queue_broker.rs)

PICK_SQL = (
    "SELECT id, attempts FROM tasks"
    " WHERE (status = 'queued' AND attempts < max_attempts"
    " AND (not_before IS NULL OR not_before <= ?))"
    " ORDER BY priority DESC, created_at ASC, id ASC LIMIT 1"
)

CLAIM_SQL = (
    "UPDATE tasks SET status = 'claimed', worker_id = ?, claimed_at = ?,"
    " lease_until = ?, attempts = attempts + 1,"
    " lane_id = COALESCE(lane_id, ?), park_kind = NULL, wait_kind = NULL,"
    " not_before = NULL, required_budget_minutes = NULL, updated_at = ?"
    " WHERE id = ?"
    " AND (status = 'queued' AND attempts < max_attempts"
    " AND (not_before IS NULL OR not_before <= ?))"
)


def _now_strings():
    now = datetime.now(timezone.utc)
    fmt = "%Y-%m-%dT%H:%M:%SZ"
    return now.strftime(fmt), (now + timedelta(seconds=LEASE_SECONDS)).strftime(fmt)


def _check_counts(conn, expected, label):
    actual = qb.table_counts(conn)
    if actual != expected:
        raise DrillError(f"{label} table counts differ: {actual} != {expected}")


def _check_schema(conn, expected_sha, label):
    if qb.schema_sha256(conn) != expected_sha:
        raise DrillError(f"{label} schema fingerprint differs from the manifest")


def drill(dump_path, manifest_path, out_path, worker="restore-drill",
          allow_empty=False):
    if os.path.exists(out_path):
        raise DrillError(f"refusing to overwrite existing drill target: {out_path}")
    try:
        with open(manifest_path, "r", encoding="utf-8") as handle:
            manifest = json.load(handle)
    except (OSError, ValueError) as exc:
        raise DrillError(f"cannot read manifest {manifest_path}: {exc}") from exc
    with open(dump_path, "rb") as handle:
        raw = handle.read()
    if hashlib.sha256(raw).hexdigest() != manifest["dump_sha256"]:
        raise DrillError("dump SHA-256 does not match the manifest")
    dump_text = raw.decode("utf-8")
    expected_counts = manifest["tables"]
    expected_schema = manifest["schema_sha256"]

    tmp = tempfile.TemporaryDirectory(prefix="queue-drill-")
    try:
        d1_copy = qb.load_dump_text(dump_text, os.path.join(tmp.name, "d1.db"))
        try:
            _check_counts(d1_copy, expected_counts, "D1 copy")
            _check_schema(d1_copy, expected_schema, "D1 copy")
            restored_dump = qb.dump_database(d1_copy)
        finally:
            d1_copy.close()

        restored = qb.load_dump_text(restored_dump, out_path)
        try:
            _check_counts(restored, expected_counts, "restored store")
            _check_schema(restored, expected_schema, "restored store")
            claimed_id = _claim_check(restored, worker, allow_empty)
            restored.commit()
        finally:
            restored.close()
    except sqlite3.Error as exc:
        raise DrillError(f"sqlite error during drill: {exc}") from exc
    finally:
        tmp.cleanup()

    return {
        "ok": True,
        "tables": expected_counts,
        "claimed_task_id": claimed_id,
        "worker": worker,
        "restored": out_path,
    }


def _claim_check(conn, worker, allow_empty):
    now, lease_until = _now_strings()
    pick = conn.execute(PICK_SQL, (now,)).fetchone()
    if pick is None:
        if allow_empty:
            return None
        raise DrillError("no claimable task in the restored store")
    task_id, attempts_before = pick
    cursor = conn.execute(
        CLAIM_SQL, (worker, now, lease_until, worker, now, task_id, now)
    )
    if cursor.rowcount != 1:
        raise DrillError(f"claim UPDATE matched {cursor.rowcount} rows, want 1")
    conn.execute(
        "INSERT INTO leases (task_id, worker_id, lease_until, heartbeat_at)"
        " VALUES (?, ?, ?, ?)",
        (task_id, worker, lease_until, now),
    )
    row = conn.execute(
        "SELECT status, worker_id, attempts, lease_until FROM tasks WHERE id = ?",
        (task_id,),
    ).fetchone()
    if row != ("claimed", worker, attempts_before + 1, lease_until):
        raise DrillError(f"claimed row reads back wrong: {row}")
    again = conn.execute(
        CLAIM_SQL, ("other-worker", now, lease_until, "other-worker", now,
                    task_id, now)
    )
    if again.rowcount != 0:
        raise DrillError("second claim on the same task matched a row")
    return task_id


def _parser():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--dump", required=True, help="export SQL dump")
    parser.add_argument("--manifest", required=True, help="export manifest JSON")
    parser.add_argument(
        "--out", required=True, help="scratch restored-store DB file to create"
    )
    parser.add_argument("--worker", default="restore-drill", help="drill worker id")
    parser.add_argument(
        "--allow-empty", action="store_true",
        help="pass when the store holds no claimable task",
    )
    return parser


def main(argv=None):
    args = _parser().parse_args(argv)
    try:
        report = drill(
            args.dump, args.manifest, args.out,
            worker=args.worker, allow_empty=args.allow_empty,
        )
    except (DrillError, OSError) as exc:
        print(f"queue-backup: drill: {exc}", file=sys.stderr)
        return 1
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
