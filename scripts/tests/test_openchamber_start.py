#!/usr/bin/env python3
"""Regression tests for bin/openchamber-start.sh argument handling (issue #10547)."""

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "bin" / "openchamber-start.sh"


def run(*args):
    return subprocess.run(
        ["bash", str(SCRIPT), *args],
        text=True,
        capture_output=True,
    )


class OpenchamberStartArgContractTests(unittest.TestCase):
    def test_missing_argument_rejected(self):
        r = run()
        self.assertNotEqual(r.returncode, 0)
        self.assertEqual(r.returncode, 2)
        self.assertIn("usage:", r.stderr)

    def test_extra_argument_rejected(self):
        with tempfile.NamedTemporaryFile("w", suffix=".sh", delete=False) as f:
            f.write("#!/bin/sh\nexit 0\n")
            path = f.name
        os.chmod(path, 0o755)
        try:
            r = run(path, "extra-bogus-arg")
            self.assertNotEqual(r.returncode, 0)
            self.assertEqual(r.returncode, 2)
            self.assertIn("usage:", r.stderr)
            self.assertIn("unexpected argument", r.stderr)
        finally:
            os.unlink(path)

    def test_valid_single_argument_preserves_behavior(self):
        with tempfile.NamedTemporaryFile("w", suffix=".sh", delete=False) as f:
            f.write("#!/bin/sh\nexit 0\n")
            path = f.name
        os.chmod(path, 0o755)
        try:
            r = run(path)
            # Valid invocation execs the serve script (exit 0) without usage error.
            self.assertEqual(r.returncode, 0)
            self.assertNotIn("usage:", r.stderr)
        finally:
            os.unlink(path)


if __name__ == "__main__":
    unittest.main()
