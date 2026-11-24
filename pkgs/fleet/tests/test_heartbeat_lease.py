"""Run the broker's heartbeat SQL against an isolated SQLite database."""

from pathlib import Path
import sqlite3
import unittest

SQL = Path(__file__).resolve().parents[1] / "crates/fleet-queue/src/turso_queue"
HEARTBEAT = (SQL / "heartbeat.sql").read_text()
MIRROR = (SQL / "heartbeat_lease_mirror.sql").read_text()


class HeartbeatLeaseTest(unittest.TestCase):
    def setUp(self):
        self.db = sqlite3.connect(":memory:")
        self.addCleanup(self.db.close)
        self.db.executescript("""
            CREATE TABLE tasks (id INTEGER PRIMARY KEY, worker_id TEXT,
                lane_id TEXT, claimed_at TEXT, attempts INTEGER, status TEXT,
                progress TEXT, updated_at TEXT, lease_until TEXT);
            CREATE TABLE leases (task_id INTEGER PRIMARY KEY, worker_id TEXT,
                lease_until TEXT, heartbeat_at TEXT);
            INSERT INTO tasks VALUES (1,'worker','lane','start',2,'claimed',
                '{}','2026-01-01T00:00:00Z','2026-01-01T04:00:00Z');
        """)
        self.db.execute(MIRROR)

    def beat(self, proposed, now="2026-01-01T00:05:00Z"):
        return self.db.execute(
            HEARTBEAT,
            (proposed, '{"step":"review"}', now, 1, "worker", "start", "lane", 2),
        ).rowcount

    def deadline(self):
        return self.db.execute("SELECT lease_until FROM tasks").fetchone()[0]

    def test_later_deadline_and_heartbeat_clock_are_mirrored(self):
        initial = self.deadline()
        later = "2026-01-01T04:10:00Z"
        for now, proposed, expected in [
            ("2026-01-01T00:05:00Z", "2026-01-01T00:20:00Z", initial),
            ("2026-01-01T00:05:00Z", "2026-01-01T00:20:00Z", initial),
            ("2026-01-01T03:55:00Z", later, later),
            ("2026-01-01T03:56:00Z", "2026-01-01T04:00:00Z", later),
        ]:
            with self.subTest(proposed=proposed, now=now):
                self.assertEqual(self.beat(proposed, now), 1)
                self.assertEqual(self.deadline(), expected)
                self.assertEqual(
                    self.db.execute(
                        "SELECT worker_id, lease_until, heartbeat_at FROM leases"
                    ).fetchone(),
                    ("worker", expected, now),
                )

    def test_changed_claim_cannot_be_renewed(self):
        for column, value in [
            ("status", "parked"),
            ("worker_id", "other"),
            ("lane_id", None),
            ("claimed_at", "next claim"),
            ("attempts", 3),
        ]:
            with self.subTest(column=column):
                self.db.execute("SAVEPOINT replacement")
                self.db.execute(f"UPDATE tasks SET {column} = ?", (value,))
                self.assertEqual(self.beat("2026-01-01T09:00:00Z"), 0)
                self.assertEqual(self.deadline(), "2026-01-01T04:00:00Z")
                self.assertIsNone(self.db.execute("SELECT * FROM leases").fetchone())
                self.db.execute("ROLLBACK TO replacement")
                self.db.execute("RELEASE replacement")

    def test_lease_mirror_failure_rolls_back_task_update(self):
        self.db.execute("""CREATE TRIGGER fail_lease BEFORE INSERT ON leases
                           BEGIN SELECT RAISE(ABORT, 'injected mirror failure'); END""")
        with self.assertRaisesRegex(sqlite3.IntegrityError, "injected mirror failure"):
            self.beat("2026-01-01T09:00:00Z")
        self.assertEqual(self.deadline(), "2026-01-01T04:00:00Z")
        self.assertEqual(
            self.db.execute("SELECT progress FROM tasks").fetchone(), ("{}",)
        )


if __name__ == "__main__":
    unittest.main()
