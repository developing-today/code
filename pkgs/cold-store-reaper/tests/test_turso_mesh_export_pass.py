#!/usr/bin/env python3
"""Tests for scripts/turso-mesh-export-pass.py (#13456, slice 2).

Oracles:
- The planner's own output is the input, so the due cohort is taken
  from turso-mesh-cohorts.py, not recomputed here.
- Failure injection: a fake exporter fails for names it does not know
  (a misnamed or removed database) and for one with unparseable output;
  each must appear in the run record's retry list, never be dropped.
- Carry-over: a database in a previous run's retry list is exported
  again in the next pass.

Usage: python3 -m unittest discover -s pkgs/cold-store-reaper/tests -v
"""

import hashlib
import json
import subprocess
import sys
import tempfile
import unittest
from datetime import date
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent.parent / "scripts"
PLANNER = SCRIPTS / "turso-mesh-cohorts.py"
PASS = SCRIPTS / "turso-mesh-export-pass.py"

FAKE_EXPORTER = """#!/usr/bin/env python3
import json, sys
name = sys.argv[1]
if name.startswith("removed-"):
    sys.exit(3)
if name.startswith("garbled-"):
    print("not json")
    sys.exit(0)
print(json.dumps({"snapshot_id": "snap-" + name, "exported_at": "2026-11-24T03:00:00Z"}))
"""


def cohort_inventory(day, wanted_count):
    """Names whose hash lands in the due cohort, found by the planner's rule."""
    names = []
    due = (day - date(1970, 1, 1)).days % 14
    index = 0
    while len(names) < wanted_count:
        name = "db-%03d" % index
        digest = hashlib.sha256(name.encode("utf-8")).digest()
        if int.from_bytes(digest[:8], "big") % 14 == due:
            names.append(name)
        index += 1
    return names


def run_planner(day, inventory):
    result = subprocess.run(
        [sys.executable, str(PLANNER), "--date", day.isoformat()],
        input="\n".join(inventory).encode(),
        capture_output=True,
        check=True,
    )
    return json.loads(result.stdout.decode())


class TestExportPass(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self.tmp.name)
        self.exporter = self.dir / "exporter.py"
        self.exporter.write_text(FAKE_EXPORTER)
        self.exporter.chmod(0o755)
        self.day = date(2026, 11, 24)

    def tearDown(self):
        self.tmp.cleanup()

    def run_pass_cli(self, plan, *extra):
        plan_path = self.dir / "plan.json"
        plan_path.write_text(json.dumps(plan))
        return subprocess.run(
            [sys.executable, str(PASS), "--plan", str(plan_path),
             "--exporter", str(self.exporter), *extra],
            capture_output=True,
            check=False,
        )

    def test_due_cohort_is_exported_and_recorded_with_snapshot_ids(self):
        names = cohort_inventory(self.day, 3)
        plan = run_planner(self.day, names)
        self.assertEqual(sorted(plan["databases"]), sorted(names))
        out = self.dir / "run.json"
        result = self.run_pass_cli(plan, "--out", str(out))
        self.assertEqual(result.returncode, 0, result.stderr)
        record = json.loads(out.read_text())
        self.assertTrue(record["ok"])
        self.assertEqual(record["retry"], [])
        self.assertEqual(
            sorted(entry["database"] for entry in record["exported"]), sorted(names)
        )
        for entry in record["exported"]:
            self.assertEqual(entry["snapshot_id"], "snap-" + entry["database"])
            self.assertEqual(entry["exported_at"], "2026-11-24T03:00:00Z")
        self.assertEqual(record["date"], "2026-11-24")

    def test_misnamed_and_garbled_databases_land_in_the_retry_list(self):
        good = cohort_inventory(self.day, 1)[0]
        plan = {
            "date": "2026-11-24",
            "cohort": plan_cohort(self.day),
            "databases": [good, "removed-db", "garbled-db"],
            "excluded_class_a": ["atlas"],
            "class_b_total": 3,
            "cohorts": 14,
        }
        out = self.dir / "run.json"
        result = self.run_pass_cli(plan, "--out", str(out))
        self.assertEqual(result.returncode, 1)
        record = json.loads(out.read_text())
        self.assertFalse(record["ok"])
        self.assertEqual([e["database"] for e in record["exported"]], [good])
        retry = {e["database"]: e["reason"] for e in record["retry"]}
        self.assertEqual(set(retry), {"removed-db", "garbled-db"})
        self.assertIn("exited 3", retry["removed-db"])
        self.assertIn("not JSON", retry["garbled-db"])
        self.assertEqual(record["excluded_class_a"], ["atlas"])

    def test_previous_retry_list_is_carried_into_the_next_pass(self):
        yesterday = {"date": "2026-11-23", "cohort": 0, "ok": False, "databases": [],
                     "exported": [], "retry": [{"database": "db-carried", "reason": "exporter exited 1"}],
                     "excluded_class_a": []}
        prior_path = self.dir / "prior.json"
        prior_path.write_text(json.dumps(yesterday))
        plan = {"date": "2026-11-24", "cohort": plan_cohort(self.day),
                "databases": [], "excluded_class_a": [], "class_b_total": 0, "cohorts": 14}
        result = self.run_pass_cli(plan, "--retry", str(prior_path))
        self.assertEqual(result.returncode, 0, result.stderr)
        record = json.loads(result.stdout.decode())
        self.assertEqual(record["databases"], ["db-carried"])
        self.assertEqual([e["database"] for e in record["exported"]], ["db-carried"])
        self.assertEqual(record["retry"], [])

    def test_class_a_names_never_reach_the_exporter(self):
        plan = {"date": "2026-11-24", "cohort": plan_cohort(self.day),
                "databases": ["atlas"], "excluded_class_a": [], "class_b_total": 0, "cohorts": 14}
        result = self.run_pass_cli(plan)
        record = json.loads(result.stdout.decode())
        self.assertEqual(record["exported"], [])
        self.assertIn("class-A", record["retry"][0]["reason"])

    def test_usage_errors_exit_2(self):
        plan_path = self.dir / "plan.json"
        plan_path.write_text("{}")
        base = [sys.executable, str(PASS), "--plan", str(plan_path), "--exporter", str(self.exporter)]
        self.assertEqual(subprocess.run(base + ["--bogus"], capture_output=True).returncode, 2)
        self.assertEqual(subprocess.run(base + ["extra-positional"], capture_output=True).returncode, 2)
        self.assertEqual(subprocess.run(base + ["--timeout", "0"], capture_output=True).returncode, 2)
        self.assertEqual(subprocess.run(base, capture_output=True).returncode, 2)

    def test_missing_exporter_is_a_recorded_miss_not_a_crash(self):
        plan = {"date": "2026-11-24", "cohort": plan_cohort(self.day),
                "databases": ["db-x"], "excluded_class_a": [], "class_b_total": 1, "cohorts": 14}
        plan_path = self.dir / "plan.json"
        plan_path.write_text(json.dumps(plan))
        result = subprocess.run(
            [sys.executable, str(PASS), "--plan", str(plan_path),
             "--exporter", str(self.dir / "no-such-exporter")],
            capture_output=True, check=False,
        )
        self.assertEqual(result.returncode, 1)
        record = json.loads(result.stdout.decode())
        self.assertEqual(record["retry"][0]["database"], "db-x")
        self.assertIn("could not start", record["retry"][0]["reason"])


def plan_cohort(day):
    return (day - date(1970, 1, 1)).days % 14


if __name__ == "__main__":
    unittest.main()
