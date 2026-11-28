#!/usr/bin/env python3
"""Fixture tests for note_promotion_sweep (#8206 acceptance).

Acceptance under test:
  * a promotion performed through the sweep leaves a stub at the old
    path with the new home's identity;
  * a submission-DB record of the old -> new mapping exists;
  * the promoted note is unchanged in content.
Plus the safety rules: no clobber, stubs are never promoted, re-runs
are no-ops.
"""

import hashlib
import importlib.util
import json
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
TOOL = HERE.parent / "note_promotion_sweep.py"

spec = importlib.util.spec_from_file_location("note_promotion_sweep", TOOL)
sweep = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sweep)

NOTE = "# A finding\n\nVerified content that must survive the move verbatim.\n"
NOW = "2026-11-17T00:00:00Z"


class PromotionFixture(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        root = Path(self.tmp.name)
        self.old = root / "research_notes" / "finding.md"
        self.old.parent.mkdir(parents=True)
        self.old.write_text(NOTE, encoding="utf-8")
        self.new = root / "zettel" / "finding.md"
        self.db = root / "submission.db"

    def tearDown(self):
        self.tmp.cleanup()

    def promote(self, **kw):
        return sweep.promote(
            self.old, self.new, self.db, new_home_id="zettel/finding", now=NOW, **kw
        )

    def test_promotion_leaves_stub_record_and_unchanged_note(self):
        receipt = self.promote()
        self.assertEqual(receipt["status"], "promoted")

        # (c) promoted note unchanged in content
        self.assertEqual(self.new.read_bytes(), NOTE.encode())

        # (a) stub at the old path naming the new home's identity
        stub = self.old.read_text(encoding="utf-8")
        self.assertIn(sweep.STUB_MARKER, stub)
        self.assertIn("zettel/finding", stub)
        self.assertIn(str(self.new), stub)
        self.assertIn(hashlib.sha256(NOTE.encode()).hexdigest(), stub)

        # (b) submission-DB record of the old -> new mapping
        with sqlite3.connect(self.db) as conn:
            rows = conn.execute(
                "SELECT old_path, new_path, new_home_id, content_sha256"
                " FROM note_promotions"
            ).fetchall()
        self.assertEqual(
            rows,
            [
                (
                    str(self.old),
                    str(self.new),
                    "zettel/finding",
                    hashlib.sha256(NOTE.encode()).hexdigest(),
                )
            ],
        )

    def test_rerun_is_a_noop(self):
        self.promote()
        receipt = self.promote()
        self.assertEqual(receipt["status"], "already_promoted")
        self.assertEqual(self.new.read_bytes(), NOTE.encode())

    def test_refuses_to_clobber_a_different_new_home(self):
        self.new.parent.mkdir(parents=True)
        self.new.write_text("someone else's note\n", encoding="utf-8")
        with self.assertRaises(sweep.Refusal):
            self.promote()
        # the original note is untouched and no record was written
        self.assertEqual(self.old.read_text(encoding="utf-8"), NOTE)
        self.assertIsNone(sweep.lookup_promotion(self.db, str(self.old)))

    def test_refuses_missing_old_note(self):
        with self.assertRaises(sweep.Refusal):
            sweep.promote(self.old.with_name("nope.md"), self.new, self.db, now=NOW)

    def test_cli_promote_and_sweep_map(self):
        out = subprocess.run(
            [
                sys.executable,
                str(TOOL),
                "promote",
                "--old",
                str(self.old),
                "--new",
                str(self.new),
                "--db",
                str(self.db),
                "--now",
                NOW,
            ],
            capture_output=True,
            text=True,
        )
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertEqual(json.loads(out.stdout)["status"], "promoted")

        old2 = self.old.parent / "second.md"
        old2.write_text(NOTE, encoding="utf-8")
        new2 = self.new.parent / "second.md"
        mapfile = Path(self.tmp.name) / "map.json"
        mapfile.write_text(
            json.dumps([{"old": str(old2), "new": str(new2)}]), encoding="utf-8"
        )
        out = subprocess.run(
            [
                sys.executable,
                str(TOOL),
                "sweep",
                "--map",
                str(mapfile),
                "--db",
                str(self.db),
                "--now",
                NOW,
            ],
            capture_output=True,
            text=True,
        )
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertEqual(json.loads(out.stdout)[0]["status"], "promoted")
        self.assertEqual(new2.read_bytes(), NOTE.encode())


if __name__ == "__main__":
    unittest.main()
