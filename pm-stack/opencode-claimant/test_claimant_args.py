# lane-scratch: code-b /tmp/lane-code-b-20261115-0248 task 17353
"""Regression tests for claimant.py argument handling (#10435).

Oracle: the operator-script argument contract (#3352) — claimant.py
declares NO command-line arguments, so any argument (unknown flag,
--help, stray positional) is a usage error with exit 2 BEFORE any
claimant work (no mode-file read, no supervision/gate/launch steps).

Run: python3 -m unittest pm-stack/opencode-claimant/test_claimant_args.py
"""

import importlib.util
import subprocess
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
SCRIPT = HERE / "claimant.py"

_spec = importlib.util.spec_from_file_location("claimant", SCRIPT)
claimant = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(claimant)


def run_claimant(*args):
    return subprocess.run(
        [sys.executable, "-I", str(SCRIPT), *args],
        capture_output=True,
        text=True,
        timeout=60,
    )


class TestClaimantArgsSubprocess(unittest.TestCase):
    def assert_usage_error(self, *args):
        result = run_claimant(*args)
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertIn("unrecognized arguments", result.stderr)
        # No claimant work happened: the startup banner is main()'s
        # first print and must never appear.
        self.assertNotIn("OpenCode claimant starting", result.stdout)

    def test_unknown_flag_rejected(self):
        self.assert_usage_error("--bogus")

    def test_help_rejected(self):
        self.assert_usage_error("--help")

    def test_positional_rejected(self):
        self.assert_usage_error("extra")


class TestClaimantParseArgs(unittest.TestCase):
    def test_no_arguments_accepted(self):
        # Control: the empty command line parses, preserving the
        # script's no-argument behavior.
        claimant.parse_args([])

    def test_any_argument_raises_exit_2(self):
        for argv in (["--bogus"], ["--help"], ["extra"], ["--mode", "live"]):
            with self.subTest(argv=argv):
                with self.assertRaises(SystemExit) as ctx:
                    claimant.parse_args(argv)
                self.assertEqual(ctx.exception.code, 2)


if __name__ == "__main__":
    unittest.main()
