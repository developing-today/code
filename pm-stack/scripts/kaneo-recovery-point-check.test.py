#!/usr/bin/env python3
"""Oracle: plan §2/§4, age interval [0,3600] seconds on captured DATA.

Deterministic generated-age property tests supplement driver mutation cases.
No live services; all archive verification observations are fixtures.
"""
from datetime import datetime, timedelta, timezone
import json
from pathlib import Path
import random
import runpy
import tempfile
import unittest

check = runpy.run_path(str(Path(__file__).with_name("kaneo-recovery-point-check.py")))["check_recovery"]


class RecoveryPointTest(unittest.TestCase):
    def test_age_interval(self):
        now = datetime(2026, 11, 25, tzinfo=timezone.utc)
        rng = random.Random(6816)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "record.json"
            dsn = Path(directory) / "source.dsn"
            dsn.write_text("postgresql://u:pw@SOURCE-db:5432/kan%65o")
            for seconds in [-1, 0, 1, 3599, 3600, 3601] + [rng.randrange(-7200, 7201) for _ in range(200)]:
                record = {
                    "date": "2026-11-25", "finishedAt": now.isoformat(),
                    "result": "complete", "archiveSha256": "a" * 64,
                    "destinations": {name: "ok" for name in ("local", "r2", "drive")},
                    "source": {"host": "source-db", "port": 5432, "database": "kaneo"},
                    "recoveryPointAt": (now - timedelta(seconds=seconds)).isoformat(),
                }
                path.write_text(json.dumps(record))
                with self.subTest(seconds=seconds):
                    if 0 <= seconds <= 3600:
                        check(path, dsn, now)
                    else:
                        with self.assertRaises(ValueError):
                            check(path, dsn, now)


if __name__ == "__main__":
    unittest.main()
