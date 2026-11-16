# lane-scratch: code-b /tmp/lane-code-b-20261115-0248 task 17352
"""Regression tests for orchestration-writer.py argument handling (#10432).

Oracle: the operator-script argument contract (#3352) — any unaccounted
argument is a usage error with exit 2 BEFORE any secret fetch, subprocess,
network request, or file read; exactly one optional positional path is
accepted and behaves as before.

Run: python3 -m unittest pm-stack/shared-schema/test_orchestration_writer_args.py
(or pytest). Subprocess-based: importing the writer would run its
module-level secret fetches, so the script is exercised as a CLI.
"""

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("orchestration-writer.py")


def run_writer(*args):
    return subprocess.run(
        [sys.executable, str(SCRIPT), *args],
        capture_output=True,
        text=True,
        timeout=60,
    )


class TestOrchestrationWriterArgs(unittest.TestCase):
    def test_extra_positional_argument_is_usage_error(self):
        result = run_writer("/tmp/ow-a.json", "/tmp/ow-extra.json")
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn("unrecognized arguments", result.stderr)
        self.assertIn("/tmp/ow-extra.json", result.stderr)
        # The usage error must precede any secret fetch.
        self.assertNotIn("secret fetch failed", result.stderr)

    def test_unknown_flag_is_usage_error(self):
        result = run_writer("--bogus")
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn("unrecognized arguments", result.stderr)
        self.assertNotIn("secret fetch failed", result.stderr)

    def test_single_positional_argument_is_accepted(self):
        # Control: one positional path parses fine. The document is an
        # empty object, so a run that gets past parsing fails later
        # (secret fetch or the missing runId key) — never with the
        # exit-2 usage error, and never reaching any SQL step.
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
            json.dump({}, f)
            path = f.name
        result = run_writer(path)
        self.assertNotEqual(result.returncode, 2, result.stderr)
        self.assertNotIn("unrecognized arguments", result.stderr)
        self.assertNotIn("usage:", result.stderr)


if __name__ == "__main__":
    unittest.main()
