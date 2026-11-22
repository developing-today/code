#!/usr/bin/env python3
"""The D1 export of the fleet-queue store is faithful and D1-loadable.

Oracle: a dump produced from a queue-shaped fixture database loads into a
fresh vanilla SQLite database (the D1 stand-in: D1 is SQLite) with the
identical schema fingerprint and identical per-table row counts, and the
exporter's manifest records those same counts.

Run: python3 -I infra/queue-backup/tests/test_export_d1.py
"""

import hashlib
import json
import os
import sqlite3
import sys
import tempfile
import unittest

BIN = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "bin"))
sys.path.insert(0, BIN)

import export_d1
import queue_backup_common as qb


# The queue store schema, copied from the DDL of record in
# pkgs/fleet/crates/fleet-queue/src/turso_queue/core.rs (SCHEMA) and
# pkgs/fleet/crates/fleet-queue/src/merge_intent.rs (MERGE_INTENT_DDL).
QUEUE_SCHEMA = [
    "CREATE TABLE IF NOT EXISTS tasks ("
    " id INTEGER PRIMARY KEY AUTOINCREMENT,"
    " queue TEXT NOT NULL, title TEXT, body TEXT,"
    " payload TEXT NOT NULL DEFAULT '{}', dedupe_key TEXT,"
    " priority INTEGER, status TEXT NOT NULL DEFAULT 'queued',"
    " not_before TEXT, worker_id TEXT, claimed_at TEXT, lease_until TEXT,"
    " lane_id TEXT, platform TEXT, attempts INTEGER NOT NULL DEFAULT 0,"
    " max_attempts INTEGER NOT NULL DEFAULT 1000,"
    " claim_provider TEXT, claim_model TEXT, claim_variant TEXT,"
    " claim_session_id TEXT,"
    " progress TEXT NOT NULL DEFAULT '{}', result TEXT NOT NULL DEFAULT '{}',"
    " last_error TEXT, park_kind TEXT, wait_kind TEXT, ready_condition TEXT,"
    " required_budget_minutes INTEGER,"
    " last_checked_at TEXT, check_outcome TEXT,"
    " created_at TEXT NOT NULL, updated_at TEXT NOT NULL,"
    " completed_at TEXT,"
    " blocked_deps TEXT, dep_next_recheck_at TEXT, dep_blocked_until TEXT,"
    " dep_recheck_seconds INTEGER)",
    "CREATE INDEX IF NOT EXISTS tasks_queue_dedupe"
    " ON tasks(queue, dedupe_key) WHERE dedupe_key IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS tasks_claim_scan"
    " ON tasks(queue, status, priority, created_at)",
    "CREATE TABLE IF NOT EXISTS leases ("
    " task_id INTEGER PRIMARY KEY, worker_id TEXT NOT NULL,"
    " lease_until TEXT NOT NULL, heartbeat_at TEXT NOT NULL)",
    "CREATE TABLE IF NOT EXISTS lanes ("
    " lane_id TEXT PRIMARY KEY, worker_id TEXT NOT NULL,"
    " queue TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 1)",
    "CREATE TABLE IF NOT EXISTS workers ("
    " worker_id TEXT PRIMARY KEY, role TEXT NOT NULL DEFAULT '',"
    " display_name TEXT, status TEXT NOT NULL DEFAULT 'idle',"
    " last_heartbeat_at TEXT, created_at TEXT)",
    "CREATE TABLE IF NOT EXISTS checkpoints ("
    " worker_id TEXT NOT NULL, key TEXT NOT NULL,"
    " value TEXT NOT NULL DEFAULT '{}', updated_at TEXT NOT NULL,"
    " PRIMARY KEY (worker_id, key))",
    "CREATE TABLE IF NOT EXISTS repo_task_lane_ledger ("
    " id INTEGER PRIMARY KEY AUTOINCREMENT, queue TEXT NOT NULL,"
    " task_id INTEGER NOT NULL UNIQUE, assigned_lane TEXT,"
    " executed_lane TEXT, executed_lane_id TEXT,"
    " exclusion_reason TEXT, opencode_reason TEXT, platform_limit TEXT,"
    " platform_limit_reason TEXT, assigned_by TEXT, session_id TEXT,"
    " fallback_reason TEXT, assigned_at TEXT, executed_at TEXT,"
    " updated_at TEXT NOT NULL)",
    "CREATE TABLE IF NOT EXISTS events ("
    " id INTEGER PRIMARY KEY AUTOINCREMENT, worker_id TEXT, task_id INTEGER,"
    " kind TEXT NOT NULL, detail TEXT NOT NULL DEFAULT '{}',"
    " created_at TEXT NOT NULL)",
    "CREATE INDEX IF NOT EXISTS events_task ON events(task_id, id)",
    "CREATE INDEX IF NOT EXISTS events_kind_ts ON events(kind, created_at)",
    "CREATE INDEX IF NOT EXISTS events_created ON events(created_at)",
    "CREATE TABLE IF NOT EXISTS creation_outbox ("
    " id TEXT PRIMARY KEY, repo TEXT NOT NULL, kind TEXT NOT NULL,"
    " payload TEXT NOT NULL DEFAULT '{}', dedup_key TEXT NOT NULL,"
    " payload_hash TEXT NOT NULL, producer TEXT,"
    " state TEXT NOT NULL DEFAULT 'pending',"
    " created_at TEXT NOT NULL, available_at TEXT NOT NULL,"
    " attempt_count INTEGER NOT NULL DEFAULT 0,"
    " max_attempts INTEGER NOT NULL DEFAULT 8,"
    " last_error_class TEXT, lease_owner TEXT, lease_until TEXT,"
    " github_number INTEGER, github_url TEXT, completed_at TEXT)",
    "CREATE UNIQUE INDEX IF NOT EXISTS creation_outbox_dedupe"
    " ON creation_outbox(repo, dedup_key)",
    "CREATE INDEX IF NOT EXISTS creation_outbox_due"
    " ON creation_outbox(state, available_at, created_at)",
    "CREATE TABLE IF NOT EXISTS creation_outbox_circuit ("
    " id INTEGER PRIMARY KEY CHECK (id = 1),"
    " consecutive_rate_limits INTEGER NOT NULL DEFAULT 0,"
    " open_until TEXT, last_error_class TEXT, updated_at TEXT NOT NULL)",
    "CREATE TABLE IF NOT EXISTS merge_intents ("
    " id INTEGER PRIMARY KEY,"
    " stack_id TEXT NOT NULL, position INTEGER NOT NULL,"
    " repo TEXT NOT NULL, branch TEXT NOT NULL, base_branch TEXT NOT NULL,"
    " head_sha TEXT, base_head_sha TEXT,"
    " issue_number INTEGER, pr_number INTEGER,"
    " owner_lane TEXT NOT NULL, owner_worker TEXT NOT NULL,"
    " owner_session TEXT, state TEXT NOT NULL DEFAULT 'intended',"
    " conflict_detail TEXT, created_at TEXT NOT NULL,"
    " updated_at TEXT NOT NULL,"
    " UNIQUE (stack_id, position), UNIQUE (repo, branch))",
]

TS = "2026-11-22T00:00:00Z"


def make_fixture(path):
    conn = sqlite3.connect(path)
    try:
        for statement in QUEUE_SCHEMA:
            conn.execute(statement)
        conn.execute(
            "INSERT INTO lanes VALUES ('drill-lane', 'seed-worker',"
            " 'repo-health-code-pilot', 1)"
        )
        conn.execute(
            "INSERT INTO workers VALUES ('seed-worker', 'lane', 'Seed',"
            " 'idle', ?, ?)",
            (TS, TS),
        )
        tasks = [
            # id, priority, status, not_before, attempts, max_attempts
            (1, 90, "queued", None, 0, 1000),
            (2, 10, "queued", "2999-01-01T00:00:00Z", 0, 1000),
            (3, 80, "queued", None, 1000, 1000),
            (4, 70, "done", None, 1, 1000),
            (5, 60, "parked", "2999-01-01T00:00:00Z", 0, 1000),
        ]
        for task_id, priority, status, not_before, attempts, max_attempts in tasks:
            conn.execute(
                "INSERT INTO tasks (id, queue, title, priority, status,"
                " not_before, attempts, max_attempts, created_at, updated_at)"
                " VALUES (?, 'repo-health-code-pilot', ?, ?, ?, ?, ?, ?, ?, ?)",
                (task_id, f"task {task_id}", priority, status, not_before,
                 attempts, max_attempts, TS, TS),
            )
        conn.execute(
            "INSERT INTO events (worker_id, task_id, kind, created_at)"
            " VALUES ('seed-worker', 4, 'completed', ?)",
            (TS,),
        )
        conn.execute(
            "INSERT INTO checkpoints VALUES ('seed-worker', 'cursor',"
            " '{\"n\": 1}', ?)",
            (TS,),
        )
        conn.execute(
            "INSERT INTO creation_outbox (id, repo, kind, dedup_key,"
            " payload_hash, created_at, available_at)"
            " VALUES ('ob1', 'developing-today/code', 'issue', 'k1', 'h1',"
            " ?, ?)",
            (TS, TS),
        )
        conn.execute(
            "INSERT INTO creation_outbox_circuit (id, updated_at)"
            " VALUES (1, ?)",
            (TS,),
        )
        conn.execute(
            "INSERT INTO repo_task_lane_ledger (queue, task_id,"
            " assigned_lane, updated_at)"
            " VALUES ('repo-health-code-pilot', 4, 'drill-lane', ?)",
            (TS,),
        )
        conn.commit()
    finally:
        conn.close()


class ExportD1Tests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.source = os.path.join(self.tmp.name, "queue.db")
        make_fixture(self.source)
        self.out = os.path.join(self.tmp.name, "queue-d1.sql")
        self.manifest_path = os.path.join(self.tmp.name, "manifest.json")

    def test_export_loads_identical_in_vanilla_sqlite(self):
        manifest = export_d1.export(self.source, self.out, self.manifest_path)
        conn = sqlite3.connect(self.source)
        try:
            expected_counts = qb.table_counts(conn)
            expected_schema = qb.schema_fingerprint(conn)
        finally:
            conn.close()
        self.assertEqual(manifest["tables"], expected_counts)
        self.assertEqual(manifest["tables"]["tasks"], 5)
        loaded = qb.load_dump_file(self.out, os.path.join(self.tmp.name, "d1.db"))
        try:
            self.assertEqual(qb.table_counts(loaded), expected_counts)
            self.assertEqual(qb.schema_fingerprint(loaded), expected_schema)
            self.assertEqual(
                manifest["schema_sha256"],
                hashlib.sha256(expected_schema.encode("utf-8")).hexdigest(),
            )
        finally:
            loaded.close()

    def test_dump_is_d1_hygienic_and_manifest_matches_bytes(self):
        manifest = export_d1.export(self.source, self.out, self.manifest_path)
        with open(self.out, "r", encoding="utf-8") as handle:
            dump = handle.read()
        self.assertNotIn("PRAGMA", dump.upper())
        with open(self.out, "rb") as handle:
            raw = handle.read()
        self.assertEqual(
            manifest["dump_sha256"], hashlib.sha256(raw).hexdigest()
        )
        self.assertEqual(manifest["dump_bytes"], len(raw))
        with open(self.manifest_path, "r", encoding="utf-8") as handle:
            on_disk = json.load(handle)
        self.assertEqual(on_disk["tables"], manifest["tables"])

    def test_export_refuses_to_clobber_without_force(self):
        export_d1.export(self.source, self.out, self.manifest_path)
        with self.assertRaises(FileExistsError):
            export_d1.export(self.source, self.out, self.manifest_path)
        rc = export_d1.main(
            ["--source", self.source, "--out", self.out,
             "--manifest", self.manifest_path]
        )
        self.assertEqual(rc, 1)
        rc = export_d1.main(
            ["--source", self.source, "--out", self.out,
             "--manifest", self.manifest_path, "--force"]
        )
        self.assertEqual(rc, 0)

    def test_export_rejects_a_non_database_source(self):
        bogus = os.path.join(self.tmp.name, "bogus.db")
        with open(bogus, "w", encoding="utf-8") as handle:
            handle.write("this is not a sqlite database")
        rc = export_d1.main(
            ["--source", bogus, "--out", self.out,
             "--manifest", self.manifest_path]
        )
        self.assertEqual(rc, 1)
        self.assertFalse(os.path.exists(self.out))


if __name__ == "__main__":
    unittest.main()
