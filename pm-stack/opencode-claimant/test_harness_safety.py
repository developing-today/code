#!/usr/bin/env python3
"""Black-box regression tests for the offline harness, never the live claimant."""

import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent


class HarnessSafetyTests(unittest.TestCase):
    def run_harness(self, script, arguments=(), injected_test=None):
        with tempfile.TemporaryDirectory(prefix="claimant-harness-check-") as tmp:
            base = Path(tmp)
            home = base / "operator-home"
            mode = home / ".local/state/opencode-claimant/mode"
            mode.parent.mkdir(parents=True)
            mode.write_bytes(b"live\noperator-sentinel\x00")
            before = {
                str(p.relative_to(home)): p.read_bytes()
                for p in home.rglob("*")
                if p.is_file()
            }
            source = ROOT
            if injected_test is not None:
                source = base / "fixture"
                source.mkdir()
                for name in (script, "run_hermetic_tests.py"):
                    shutil.copyfile(ROOT / name, source / name)
                (source / "test").mkdir()
                (source / "test_claimant.py").write_text(injected_test)
                (source / "test_poll.py").write_text("")
                (source / "test/test_claimant.py").write_text("")
                (source / "test_prompt_constants.py").write_text("")
            env = dict(
                os.environ,
                HOME=str(home),
                XDG_STATE_HOME=str(home / "state"),
                OPENCODE_AUTH_FILE=str(home / "auth-must-not-be-read"),
                OPENCODE_URL="http://127.0.0.1:1",
            )
            proc = subprocess.run(
                ["bash", str(source / script), *arguments],
                env=env,
                capture_output=True,
                text=True,
                timeout=30,
            )
            after = {
                str(p.relative_to(home)): p.read_bytes()
                for p in home.rglob("*")
                if p.is_file()
            }
            self.assertEqual(before, after, proc.stderr)
            return proc

    def test_existing_suites_pass_and_leave_operator_home_unchanged(self):
        for script in ("test_slice1.sh", "verify_slice1.sh"):
            with self.subTest(script=script):
                result = self.run_harness(script)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn("Ran ", result.stderr)

    def test_arguments_rejected_before_tests(self):
        for script in ("test_slice1.sh", "verify_slice1.sh"):
            result = self.run_harness(script, ("--live",))
            self.assertEqual(result.returncode, 2, result.stderr)
            self.assertNotIn("Ran ", result.stderr)

    def test_test_failure_preserves_operator_home(self):
        fixture = "import unittest\nclass Failure(unittest.TestCase):\n def test_fail(self): self.fail('forced failure')\n"
        for script in ("test_slice1.sh", "verify_slice1.sh"):
            result = self.run_harness(script, injected_test=fixture)
            self.assertEqual(result.returncode, 1, result.stderr)
            self.assertIn("forced failure", result.stderr)

    def test_caught_network_or_process_attempt_still_fails_runner(self):
        for statement in (
            "import socket; socket.socket()",
            "import subprocess; subprocess.run(['must-not-execute'])",
        ):
            fixture = (
                "import unittest\nclass Guard(unittest.TestCase):\n"
                " def test_attempt(self):\n  try:\n   "
                + statement
                + "\n  except RuntimeError: pass\n"
            )
            result = self.run_harness("test_slice1.sh", injected_test=fixture)
            self.assertEqual(result.returncode, 1, result.stderr)
            self.assertIn("Forbidden external I/O attempted", result.stderr)


if __name__ == "__main__":
    unittest.main()
