"""Contract tests for the orchestration-pass retirement record (#2136, slice 1).

Oracle: the duty names and the operational/macro owner split come from
the #2136 issue text; the enabled/disabled ground truth is the
orchestration-pass row in doc/fleet-schedule-registry.md. The record
script is read-only, so every test is hermetic except the CLI pass,
which reads the real tree the way test_program_tending.py reads its
real workflow file.
"""

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

SCRIPT_DIR = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPT_DIR))

import orchestration_pass_retirement as opr

REPO_ROOT = Path(__file__).resolve().parents[2]

EXPECTED_RESIDUE = (
    "cross-queue priority calls",
    "stuck-work escalation",
    "hourly goal measurement",
    "goal-measurement rollup vs roadmaps and priorities",
)


def _write_registry(tmp: Path, state: str) -> Path:
    doc = tmp / "doc"
    doc.mkdir(parents=True)
    (doc / "fleet-schedule-registry.md").write_text(
        "| Name | Duty | Cadence | State |\n"
        "| --- | --- | --- | --- |\n"
        f"| `orchestration-pass` | triage | `interval@1h` | {state} |\n"
    )
    return tmp


class RetirementRecordTest(unittest.TestCase):
    def test_residue_matches_issue_2136(self):
        residue = sorted(
            e["duty"] for e in opr.DUTIES if e.get("status") == "residue"
        )
        self.assertEqual(residue, sorted(EXPECTED_RESIDUE))

    def test_every_duty_has_exactly_one_known_owner(self):
        for entry in opr.DUTIES:
            self.assertIn(entry.get("owner"), opr.KNOWN_OWNERS, entry)

    def test_residue_is_tend_owned_with_issue_split(self):
        for entry in opr.DUTIES:
            if entry.get("status") != "residue":
                continue
            self.assertIn(entry["owner"], opr.RESIDUE_OWNERS, entry)
            self.assertEqual(
                entry["owner"] == "stewardship-cycle",
                entry["class"] == "operational",
                entry,
            )

    def test_check_passes_on_clean_tree(self):
        report = opr.check(REPO_ROOT)
        self.assertEqual(report["violations"], [], report)
        self.assertFalse(report["retired"])
        self.assertEqual(report["registry_state"], "enabled")

    def test_missing_owner_fails(self):
        duties = [dict(e) for e in opr.DUTIES]
        duties[0]["owner"] = "nobody"
        with mock.patch.object(opr, "DUTIES", duties):
            report = opr.check(REPO_ROOT)
        self.assertTrue(
            any("no known owner" in v for v in report["violations"]),
            report,
        )

    def test_non_tend_residue_owner_fails(self):
        duties = [dict(e) for e in opr.DUTIES]
        for entry in duties:
            if entry.get("duty") == "stuck-work escalation":
                entry["owner"] = "fleet-monitor"
        with mock.patch.object(opr, "DUTIES", duties):
            report = opr.check(REPO_ROOT)
        self.assertTrue(
            any("not Tend-owned" in v for v in report["violations"]), report
        )

    def test_retired_record_requires_disabled_registry(self):
        root = _write_registry(Path(tempfile.mkdtemp()), "enabled")
        with mock.patch.object(opr, "RETIRED", True):
            report = opr.check(root)
        self.assertTrue(
            any("says retired" in v for v in report["violations"]), report
        )

    def test_registry_drift_detected_while_active(self):
        root = _write_registry(Path(tempfile.mkdtemp()), "disabled")
        report = opr.check(root)
        self.assertTrue(
            any("says active" in v for v in report["violations"]), report
        )

    def test_cli_check_reports_json(self):
        proc = subprocess.run(
            [sys.executable, "-I", str(SCRIPT_DIR / "orchestration_pass_retirement.py"),
             "--check", "--json", "--root", str(REPO_ROOT)],
            capture_output=True,
            text=True,
            timeout=60,
            cwd=str(REPO_ROOT),
        )
        self.assertEqual(proc.returncode, 0, proc.stderr)
        report = json.loads(proc.stdout)
        self.assertEqual(report["violations"], [])

    def test_cli_unknown_flag_exits_2(self):
        proc = subprocess.run(
            [sys.executable, "-I", str(SCRIPT_DIR / "orchestration_pass_retirement.py"),
             "--bogus"],
            capture_output=True,
            text=True,
            timeout=60,
            cwd=str(REPO_ROOT),
        )
        self.assertEqual(proc.returncode, 2)

    def test_cli_without_check_exits_2(self):
        proc = subprocess.run(
            [sys.executable, "-I", str(SCRIPT_DIR / "orchestration_pass_retirement.py")],
            capture_output=True,
            text=True,
            timeout=60,
            cwd=str(REPO_ROOT),
        )
        self.assertEqual(proc.returncode, 2)


if __name__ == "__main__":
    unittest.main()
