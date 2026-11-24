#!/usr/bin/env python3
"""Tests for scripts/turso-mesh-restic-runner.py — #14247 (#13455 slice 2).

Oracles:
- Fake-restic oracle: a fixture ``restic`` executable records every
  invocation (argv + the staged files it was handed) into a JSON-lines
  log inside a fixture repository directory. The runner's behaviour is
  asserted against that log: exactly one ``backup`` per database, with
  tags ``class-a``, ``mesh=turso-mesh`` and ``db=<name>``; the staged
  snapshot directory restic received contains the compressed dump and
  a ``manifest.json`` whose per-table row counts equal a recount of
  the fixture SQLite file the export seam produced (the slice-1
  manifest oracle); exactly one ``forget`` carrying the design §5
  retention numbers (keep-daily 7 / keep-weekly 8 / keep-monthly 12)
  with ``--prune``.
- Failure-isolation oracle: an export seam that fails for one named
  database still yields snapshots for the other two, the failing
  database's own report entry, and a non-zero runner exit.

Run:  python3 -m pytest pkgs/cold-store-reaper/tests/test_turso_mesh_restic_runner.py
"""

import importlib.util
import json
import os
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

PKG = Path(__file__).resolve().parent.parent
SCRIPT = PKG / "scripts" / "turso-mesh-restic-runner.py"

spec = importlib.util.spec_from_file_location("turso_mesh_restic_runner", SCRIPT)
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)

class RunnerFixture(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        root = Path(self.tmp.name)
        self.repo = root / "repo"
        self.repo.mkdir()
        self.log = root / "restic-log.jsonl"
        bin_dir = root / "bin"
        bin_dir.mkdir()
        self.restic = bin_dir / "restic"
        self.restic.write_text(
            "#!/usr/bin/env python3\n"
            "import json, os, sys\n"
            "from pathlib import Path\n"
            "log = os.environ['FAKE_RESTIC_LOG']\n"
            "args = sys.argv[1:]\n"
            "entry = {'argv': args}\n"
            "if 'backup' in args:\n"
            "    stage = Path(args[args.index('backup') + 1])\n"
            "    entry['staged'] = sorted(p.name for p in stage.iterdir())\n"
            "    manifest = stage / 'manifest.json'\n"
            "    if manifest.is_file():\n"
            "        entry['manifest'] = json.loads(manifest.read_text())\n"
            "    for p in stage.iterdir():\n"
            "        if p.name.endswith('.sql.zst'):\n"
            "            entry['dump_zst'] = p.read_text()\n"
            "with open(log, 'a') as fh:\n"
            "    fh.write(json.dumps(entry) + '\\n')\n"
        )
        self.restic.chmod(0o755)
        self.exporter = bin_dir / "export-one.py"
        self.exporter.write_text(
            "#!/usr/bin/env python3\n"
            "import os, sqlite3, sys\n"
            "from pathlib import Path\n"
            "db, dump, sqlite = sys.argv[1], sys.argv[2], sys.argv[3]\n"
            "if db == os.environ.get('FAKE_EXPORT_FAIL', ''):\n"
            "    sys.exit(3)\n"
            "conn = sqlite3.connect(sqlite)\n"
            "conn.execute('CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT)')\n"
            "conn.executemany('INSERT INTO t VALUES (?, ?)', [(1, 'a'), (2, 'b')])\n"
            "conn.commit(); conn.close()\n"
            "Path(dump).write_text(\"CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT);\\nINSERT INTO t VALUES (1, 'a');\\nINSERT INTO t VALUES (2, 'b');\\nCOMMIT;\\n\")\n"
        )
        self.exporter.chmod(0o755)
        self.work = root / "work"
        self.env = dict(os.environ)
        self.env["FAKE_RESTIC_LOG"] = str(self.log)
        self.env["RESTIC_PASSWORD"] = "fixture-only"

    def tearDown(self):
        self.tmp.cleanup()

    def run_runner(self, extra_env=None):
        env = dict(self.env)
        if extra_env:
            env.update(extra_env)
        old = dict(os.environ)
        os.environ.update(env)
        try:
            return mod.run(
                list(mod.CLASS_A_DATABASES),
                str(self.repo),
                "%s {db} {dump} {sqlite}" % self.exporter,
                str(self.work),
                restic_bin=str(self.restic),
                compress_cmd="cat",
                exported_at="2026-11-24T00:00:00Z",
            )
        finally:
            os.environ.clear()
            os.environ.update(old)

    def entries(self):
        if not self.log.exists():
            return []
        return [json.loads(l) for l in self.log.read_text().splitlines() if l.strip()]


class RunnerAcceptance(RunnerFixture):
    def test_one_tagged_snapshot_per_db_with_manifest_and_retention(self):
        report = self.run_runner()
        self.assertEqual(report, {db: "ok" for db in mod.CLASS_A_DATABASES})
        entries = self.entries()
        backups = [e for e in entries if "backup" in e["argv"]]
        self.assertEqual(len(backups), 3)
        seen_dbs = set()
        for e in backups:
            argv = e["argv"]
            self.assertIn("class-a", argv)
            self.assertIn("mesh=turso-mesh", argv)
            db_tags = [a for a in argv if a.startswith("db=")]
            self.assertEqual(len(db_tags), 1)
            seen_dbs.add(db_tags[0][3:])
            self.assertIn("manifest.json", e["staged"])
            self.assertTrue(any(n.endswith(".sql.zst") for n in e["staged"]))
            self.assertIn("COMMIT;", e["dump_zst"])
            tables = {t["name"]: t["rows"] for t in e["manifest"]["tables"]}
            self.assertEqual(tables, {"t": 2})
        self.assertEqual(seen_dbs, set(mod.CLASS_A_DATABASES))
        forgets = [e for e in entries if "forget" in e["argv"]]
        self.assertEqual(len(forgets), 1)
        argv = forgets[0]["argv"]
        for flag, value in (("--keep-daily", "7"), ("--keep-weekly", "8"),
                            ("--keep-monthly", "12")):
            self.assertEqual(argv[argv.index(flag) + 1], value)
        self.assertIn("--prune", argv)

    def test_one_db_failure_does_not_block_others(self):
        report = self.run_runner(extra_env={"FAKE_EXPORT_FAIL": "atlas"})
        self.assertEqual(report["fleet-queue"], "ok")
        self.assertEqual(report["fleet-platform"], "ok")
        self.assertTrue(report["atlas"].startswith("failed:"))
        backups = [e for e in self.entries() if "backup" in e["argv"]]
        self.assertEqual(len(backups), 2)


if __name__ == "__main__":
    unittest.main()
