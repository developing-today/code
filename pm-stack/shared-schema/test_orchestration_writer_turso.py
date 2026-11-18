"""Oracle: SQLite atomic snapshot, first-seen identity and append-only runs."""

import importlib.util
import json
from pathlib import Path
import sqlite3
import sys
import tempfile
import unittest
from unittest.mock import patch

path = Path(__file__).with_name("orchestration-writer.py")
spec = importlib.util.spec_from_file_location("writer", path)
writer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(writer)


class Database:
    def __init__(self):
        self.db = sqlite3.connect(":memory:")
        self.db.row_factory = sqlite3.Row
        self.db.executescript(
            "CREATE TABLE orch_runs(run_id TEXT PRIMARY KEY,generated_at,document_version,provenance,document,item_count); CREATE TABLE orch_items(id TEXT PRIMARY KEY,tier,kind,title,source,state,reason,ref,since,last_seen_at,item,first_seen_run_id,last_seen_run_id);"
        )

    def query(self, sql, args=()):
        args = [json.dumps(a) if isinstance(a, (dict, list)) else a for a in args]
        return [dict(r) for r in self.db.execute(sql, args)]

    def batch(self, stmts, transaction=False):
        with self.db:
            return [self.query(sql, args) for sql, args in stmts]


class WriterTests(unittest.TestCase):
    def test_snapshot_and_conflicting_replay(self):
        db = Database()
        item = dict(
            id="a",
            kind="decision",
            title="a",
            source="test",
            reason="test",
            since="2026-11-18",
            lastSeenAt="2026-11-18",
        )
        doc = dict(
            runId="r1", generatedAt="2026-11-18", version=1, tiers={"needsDrew": [item]}
        )
        with tempfile.TemporaryDirectory() as directory:
            p = Path(directory) / "doc.json"

            def run(d):
                p.write_text(json.dumps(d))
                with (
                    patch.object(sys, "argv", [str(path), str(p)]),
                    patch.object(writer, "Turso", return_value=db),
                ):
                    writer.main()

            run(doc)
            # Recovery JSONB serializes equivalent objects with a different order.
            db.db.execute(
                "UPDATE orch_runs SET document=? WHERE run_id=?",
                (json.dumps(doc, sort_keys=True, indent=2), "r1"),
            )
            db.db.commit()
            run(doc)
            nextdoc = {**doc, "runId": "r2"}
            run(nextdoc)
            self.assertEqual(
                db.query("select first_seen_run_id from orch_items")[0][
                    "first_seen_run_id"
                ],
                "r1",
            )
            with self.assertRaises(SystemExit):
                run({**nextdoc, "tiers": {}})
            self.assertEqual(db.query("select count(*) n from orch_items")[0]["n"], 1)
            run({**doc, "runId": "r3", "tiers": {}})
            self.assertEqual(db.query("select count(*) n from orch_items")[0]["n"], 0)
            self.assertEqual(db.query("select count(*) n from orch_runs")[0]["n"], 3)
            db.db.close()


if __name__ == "__main__":
    unittest.main()
