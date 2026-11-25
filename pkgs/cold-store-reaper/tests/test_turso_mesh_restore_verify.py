#!/usr/bin/env python3
"""Tests for scripts/turso-mesh-restore-verify.py — #14251 (#13455 slice 4).

Oracles:
- Fake-restic oracle: a fixture ``restic`` executable answers
  ``snapshots --json`` with one tagged snapshot per class-A database
  and materializes that database's staged payload (compressed dump +
  ``manifest.json``, the exact artifact set the #14247 runner files)
  on ``restore``. The verifier's report is asserted against the
  fixture: per-database pass, the snapshot id it selected, per-table
  row counts equal to the manifest built by the slice-1 builder
  (``turso-mesh-class-a.py`` ``build_manifest``) from the fixture
  SQLite file, and the fleet-queue read-only queue query answering
  with the fixture's row count.
- Discrepancy oracle: a manifest whose row count is tampered with
  for one database fails that database only — the other two still
  pass — so a restore discrepancy is attributable per database.
- Scratch-discipline oracle: a non-empty per-database scratch target
  is refused (no restore is issued for it), per the house rule that
  drills restore to an isolated scratch target only.

Run:  python3 -m pytest pkgs/cold-store-reaper/tests/test_turso_mesh_restore_verify.py
"""

import importlib.util
import json
import os
import sqlite3
import sys
import tempfile
import unittest
from pathlib import Path

PKG = Path(__file__).resolve().parent.parent
SCRIPT = PKG / "scripts" / "turso-mesh-restore-verify.py"
CLASS_A_SCRIPT = PKG / "scripts" / "turso-mesh-class-a.py"

spec = importlib.util.spec_from_file_location("turso_mesh_restore_verify", SCRIPT)
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)

ca_spec = importlib.util.spec_from_file_location("turso_mesh_class_a", CLASS_A_SCRIPT)
ca = importlib.util.module_from_spec(ca_spec)
ca_spec.loader.exec_module(ca)

DUMP = (
    "CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT);\n"
    "INSERT INTO t VALUES (1, 'a');\n"
    "INSERT INTO t VALUES (2, 'b');\n"
    "COMMIT;\n"
)
EXPORTED_AT = "2026-11-24T00:00:00Z"


class VerifyFixture(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        root = Path(self.tmp.name)
        self.repo = root / "repo"
        self.repo.mkdir()
        self.payload = root / "payload"
        self.log = root / "restic-log.jsonl"
        # One staged payload per database, exactly as the runner files
        # it: <db>.sql.zst (here: plain text, decompress seam is `cat`)
        # plus manifest.json built by the slice-1 builder.
        for db in ca.CLASS_A_DATABASES:
            stage = self.payload / db
            stage.mkdir(parents=True)
            sqlite_path = stage / "fixture.db"
            conn = sqlite3.connect(str(sqlite_path))
            conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT)")
            conn.executemany("INSERT INTO t VALUES (?, ?)", [(1, "a"), (2, "b")])
            conn.commit()
            conn.close()
            manifest = ca.build_manifest(db, str(sqlite_path), DUMP.encode(), EXPORTED_AT)
            (stage / "manifest.json").write_text(json.dumps(manifest))
            (stage / ("%s.sql.zst" % db)).write_text(DUMP)
            sqlite_path.unlink()
        bin_dir = root / "bin"
        bin_dir.mkdir()
        self.restic = bin_dir / "restic"
        self.restic.write_text(
            "#!/usr/bin/env python3\n"
            "import json, os, shutil, sys\n"
            "from pathlib import Path\n"
            "args = sys.argv[1:]\n"
            "with open(os.environ['FAKE_RESTIC_LOG'], 'a') as fh:\n"
            "    fh.write(json.dumps(args) + '\\n')\n"
            "if 'snapshots' in args:\n"
            "    db = [a[3:] for a in args if a.startswith('db=')][0]\n"
            "    print(json.dumps([{'id': 'snap-' + db, 'time': '2026-11-24T05:10:00Z', 'tags': ['class-a', 'mesh=turso-mesh', 'db=' + db]}]))\n"
            "elif 'restore' in args:\n"
            "    snap = args[args.index('restore') + 1]\n"
            "    db = snap[len('snap-'):]\n"
            "    target = Path(args[args.index('--target') + 1])\n"
            "    src = Path(os.environ['FAKE_RESTIC_PAYLOAD']) / db\n"
            "    for p in src.iterdir():\n"
            "        shutil.copy2(p, target / p.name)\n"
        )
        self.restic.chmod(0o755)
        self.scratch = root / "scratch"
        self.env = dict(os.environ)
        self.env["FAKE_RESTIC_LOG"] = str(self.log)
        self.env["FAKE_RESTIC_PAYLOAD"] = str(self.payload)
        self.env["RESTIC_PASSWORD"] = "fixture-only"

    def tearDown(self):
        self.tmp.cleanup()

    def run_verify(self):
        old = dict(os.environ)
        os.environ.update(self.env)
        try:
            return mod.verify(
                list(ca.CLASS_A_DATABASES),
                str(self.repo),
                str(self.scratch),
                restic_bin=str(self.restic),
                decompress_cmd="cat",
            )
        finally:
            os.environ.clear()
            os.environ.update(old)

    def entries(self):
        if not self.log.exists():
            return []
        return [json.loads(l) for l in self.log.read_text().splitlines() if l.strip()]


class RestoreVerifyAcceptance(VerifyFixture):
    def test_all_three_pass_against_manifest_with_queue_query(self):
        report = self.run_verify()
        self.assertEqual(report["status"], "pass")
        by_db = {r["database"]: r for r in report["databases"]}
        self.assertEqual(set(by_db), set(ca.CLASS_A_DATABASES))
        for db, entry in by_db.items():
            self.assertEqual(entry["status"], "pass", entry)
            self.assertEqual(entry["snapshot"], "snap-" + db)
            self.assertTrue(entry["dump_sha256_match"])
            self.assertEqual(
                entry["tables"],
                [{"name": "t", "manifest_rows": 2, "restored_rows": 2, "match": True}],
            )
        self.assertEqual(
            by_db["fleet-queue"]["queue_query"], {"table": "t", "rows": 2}
        )
        restores = [a for a in self.entries() if "restore" in a]
        self.assertEqual(len(restores), 3)

    def test_tampered_manifest_fails_only_that_db(self):
        manifest_path = self.payload / "atlas" / "manifest.json"
        manifest = json.loads(manifest_path.read_text())
        manifest["tables"][0]["rows"] = 99
        manifest_path.write_text(json.dumps(manifest))
        report = self.run_verify()
        self.assertEqual(report["status"], "fail")
        by_db = {r["database"]: r for r in report["databases"]}
        self.assertEqual(by_db["atlas"]["status"], "fail")
        self.assertEqual(by_db["fleet-queue"]["status"], "pass")
        self.assertEqual(by_db["fleet-platform"]["status"], "pass")

    def test_nonempty_scratch_target_is_refused(self):
        target = self.scratch / "atlas"
        target.mkdir(parents=True)
        (target / "leftover.txt").write_text("previous drill evidence")
        report = self.run_verify()
        by_db = {r["database"]: r for r in report["databases"]}
        self.assertEqual(by_db["atlas"]["status"], "fail")
        self.assertIn("not empty", by_db["atlas"]["error"])
        self.assertEqual(by_db["fleet-queue"]["status"], "pass")
        restores = [a for a in self.entries() if "restore" in a]
        self.assertEqual(len(restores), 2)


if __name__ == "__main__":
    unittest.main()
