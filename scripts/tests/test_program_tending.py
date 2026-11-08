"""Contract tests for the program-tending scaffold (issue #1352, slice 1/3).

The scaffold is read-only by design: every section reports a verified
no-action, the runner never fails on a per-repo fetch error, and the
run log always covers all five sections with the hard-stop labels named.
"""

import io
import json
import subprocess
import sys
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest import mock

SCRIPT_DIR = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPT_DIR))

import program_tending as pt


class ProgramTendingScaffoldTest(unittest.TestCase):
    def test_run_log_covers_all_five_sections(self):
        with mock.patch.object(pt, "_gh_api", return_value=[]):
            log = pt.run(["developing-today/code"])
        self.assertEqual(
            [s["section"] for s in log["sections"]],
            ["epics", "roadmaps", "projects", "stalled-prs", "loose-ends"],
        )

    def test_scaffold_is_read_only(self):
        with mock.patch.object(pt, "_gh_api", return_value=[{"labels": []}]):
            log = pt.run(["developing-today/code"])
        self.assertEqual(log["actions_taken"], 0)
        self.assertTrue(all(s["status"] == "no-action" for s in log["sections"]))

    def test_hard_stops_named_in_log(self):
        with mock.patch.object(pt, "_gh_api", return_value=[]):
            log = pt.run(["developing-today/code"])
        for stop in ("held", "user/owned", "do-not-merge", "do-not-auto-edit"):
            self.assertIn(stop, log["hard_stops"])

    def test_fetch_failure_reports_never_fails(self):
        with mock.patch.object(pt, "_gh_api", side_effect=RuntimeError("offline")):
            section = pt.stub_stalled_prs(["developing-today/code"])
        self.assertEqual(section["status"], "no-action")
        self.assertIn("error", section["checked"][0])

    def test_cli_run_prints_valid_log(self):
        buf = io.StringIO()
        with mock.patch.object(pt, "_gh_api", return_value=[]), redirect_stdout(buf):
            self.assertEqual(
                pt.main(["--run", "--repo", "developing-today/code"]), 0
            )
        log = json.loads(buf.getvalue())
        self.assertEqual(log["job"], "program-tending")
        self.assertEqual(len(log["sections"]), 5)

    def test_cli_without_run_flag_exits_2(self):
        self.assertEqual(pt.main([]), 2)

    def test_workflow_file_exists_and_is_scheduled(self):
        wf = (
            Path(__file__).resolve().parents[2]
            / ".github/workflows/program-tending.yml"
        )
        text = wf.read_text()
        self.assertIn("cron:", text)
        self.assertIn("workflow_dispatch", text)
        self.assertIn("scripts/program_tending.py", text)

    def test_script_runs_isolated(self):
        bootstrap = "\n".join(
            [
                "import runpy",
                "import sys",
                "from unittest import mock",
                "response = mock.MagicMock()",
                "response.__enter__.return_value = response",
                "response.read.return_value = b'[]'",
                "response.headers.get.return_value = ''",
                "sys.argv = ['program_tending.py', '--run', '--repo', 'developing-today/code']",
                "with mock.patch('urllib.request.urlopen', return_value=response):",
                f"    runpy.run_path({str(SCRIPT_DIR / 'program_tending.py')!r}, run_name='__main__')",
            ]
        )
        proc = subprocess.run(
            [sys.executable, "-I", "-c", bootstrap],
            capture_output=True,
            text=True,
            timeout=120,
            cwd=str(SCRIPT_DIR.parent),
        )
        self.assertEqual(proc.returncode, 0, proc.stderr[-2000:])
        log = json.loads(proc.stdout)
        self.assertEqual(len(log["sections"]), 5)


if __name__ == "__main__":
    unittest.main()
