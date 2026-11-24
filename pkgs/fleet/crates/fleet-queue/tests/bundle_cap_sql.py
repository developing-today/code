"""Behavioural test for the #11333 bundle-cap counting oracle.

The broker counts a launch's ordinary claims with
BUNDLE_CLAIM_COUNT_SQL (turso_queue/lifecycle.rs) over the
events table. This test extracts that exact SQL from the Rust
source and runs it against a fabricated events table shaped
like the 2026-11-17 incident (#11333): a scheduled session
that claimed past the 10-task bundle cap, surrounded by the
exempt traffic (handoff picks, by-id claims, transfers) that
must never enter the count.

Oracle: the broker's claim count for a (lane, session) pair.
A count at or past BUNDLE_CLAIM_CAP (10) refuses the launch's
next ordinary claim; the claim loop therefore stops at
exactly 10.
"""

import json
import pathlib
import re
import sqlite3
import unittest

SRC = pathlib.Path(__file__).resolve().parents[1] / "src"
RUST = (SRC / "turso_queue.rs").read_text() + "".join(
    p.read_text() for p in sorted((SRC / "turso_queue").glob("*.rs"))
)

BUNDLE_CLAIM_CAP = 10


def rust_string(s):
    return re.sub(r"\\\n\s*", "", s).replace('\\"', '"')


def production_sql():
    m = re.search(r'BUNDLE_CLAIM_COUNT_SQL: &str =\s*"(.*?)";', RUST, re.S)
    assert m, "BUNDLE_CLAIM_COUNT_SQL not found in the Rust source"
    return rust_string(m.group(1))


def claimed(detail):
    return ("claimed", json.dumps(detail))


def pick(lane, session, mode="default"):
    return claimed(
        {
            "via": "claim_repo_task_turso",
            "pick": mode,
            "lane_id": lane,
            "session_id": session,
            "lease_seconds": 14400,
        }
    )


class BundleCapCount(unittest.TestCase):
    def setUp(self):
        self.db = sqlite3.connect(":memory:")
        self.db.execute(
            "CREATE TABLE events ("
            "id INTEGER PRIMARY KEY AUTOINCREMENT, worker_id TEXT, "
            "task_id INTEGER, kind TEXT NOT NULL, "
            "detail TEXT NOT NULL DEFAULT '{}', created_at TEXT NOT NULL)"
        )
        rows = []
        # The incident shape: 12 ordinary claims by one launch —
        # two past the cap the runbook promised.
        rows += [pick("any-n", "ses_incident")] * 12
        # A fresh launch of the same lane: 9 claims, one left.
        rows += [pick("any-n", "ses_fresh")] * 9
        # The same session id under another lane is another
        # launch: the key is the (lane, session) pair.
        rows += [pick("any-q", "ses_incident")] * 10
        # Exempt traffic for the incident launch — none of it
        # may enter the count:
        rows += [pick("any-n", "ses_incident", mode="handed_off")] * 3
        # By-id claims write 'claimed' with by_id=true and no
        # pick mark (the session arrives via row marks).
        rows += [
            claimed(
                {
                    "via": "claim_repo_task_turso",
                    "by_id": True,
                    "lane_id": "any-n",
                    "session_id": "ses_incident",
                }
            )
        ] * 2
        # Transfers write 'claimed' with via=..._transfer.
        rows += [
            claimed(
                {
                    "via": "claim_repo_task_transfer",
                    "transfer": True,
                    "by_id": True,
                    "lane_id": "any-n",
                    "session_id": "ses_incident",
                }
            )
        ]
        # Other kinds are not claims at all.
        rows += [
            (
                "completed",
                json.dumps(
                    {
                        "pick": "default",
                        "lane_id": "any-n",
                        "session_id": "ses_incident",
                    }
                ),
            )
        ]
        for i, (kind, detail) in enumerate(rows):
            self.db.execute(
                "INSERT INTO events (worker_id, task_id, kind, detail, created_at) "
                "VALUES ('repo-worker-code', ?, ?, ?, '2026-11-17T00:00:00Z')",
                (1000 + i, kind, detail),
            )

    def count(self, lane, session):
        cur = self.db.execute(production_sql(), (lane, session))
        return cur.fetchone()[0]

    def test_incident_launch_counted_in_full(self):
        self.assertEqual(self.count("any-n", "ses_incident"), 12)
        # At or past the cap: the next ordinary claim refuses.
        self.assertGreaterEqual(self.count("any-n", "ses_incident"), BUNDLE_CLAIM_CAP)

    def test_fresh_launch_has_one_claim_left(self):
        self.assertEqual(self.count("any-n", "ses_fresh"), 9)
        self.assertLess(self.count("any-n", "ses_fresh"), BUNDLE_CLAIM_CAP)
        # One more ordinary claim lands the launch exactly at
        # the cap; the claim after that is the refused one — a
        # claim loop stops at exactly 10.
        kind, detail = pick("any-n", "ses_fresh")
        self.db.execute(
            "INSERT INTO events (worker_id, task_id, kind, detail, created_at) "
            "VALUES ('repo-worker-code', 2000, ?, ?, '2026-11-17T01:00:00Z')",
            (kind, detail),
        )
        self.assertEqual(self.count("any-n", "ses_fresh"), BUNDLE_CLAIM_CAP)

    def test_key_is_the_lane_session_pair(self):
        self.assertEqual(self.count("any-q", "ses_incident"), 10)
        self.assertEqual(self.count("any-n", "ses_absent"), 0)
        self.assertEqual(self.count("any-absent", "ses_incident"), 0)


if __name__ == "__main__":
    unittest.main()
