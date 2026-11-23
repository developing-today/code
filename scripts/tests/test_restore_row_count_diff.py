#!/usr/bin/env python3
"""Contract tests for restore-row-count-diff.sh (#12788).

Oracle: the operator-script argument contract
(doc/agents/operator-script-argument-contract.md, #3352) and the
script's own header — a value option with no value, or with a value
that is itself an option, is a usage error: the usage message on
stderr and exit 2, never a raw bash error. A well-formed invocation
still runs the gate.
"""

import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "fleet" / "restore-row-count-diff.sh"


def run_diff(args):
    return subprocess.run(
        ["bash", str(SCRIPT), *args],
        text=True,
        capture_output=True,
        check=False,
    )


class TestRestoreRowCountDiffArgs(unittest.TestCase):
    def test_missing_option_value_is_a_usage_error(self):
        for option in ("--manifest", "--live", "--inventory"):
            with self.subTest(option=option):
                result = run_diff([option])
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertIn("usage:", result.stderr)
                self.assertNotIn("unbound variable", result.stderr)

    def test_option_looking_value_is_not_consumed(self):
        result = run_diff(["--manifest", "--live", "/path"])
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn("usage:", result.stderr)
        self.assertNotIn("unknown argument: /path", result.stderr)

    def test_unknown_argument_is_a_usage_error(self):
        result = run_diff(["--bogus"])
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn("unknown argument: --bogus", result.stderr)

    def test_matching_counts_pass_the_gate(self):
        with tempfile.TemporaryDirectory(prefix="row-count-diff-") as tmp:
            manifest = Path(tmp) / "manifest.tsv"
            manifest.write_text(
                "table\tsource_count\tdisposition\tallowed_delta\tnote\n"
                "t1\t5\tload\t0\t\n"
            )
            live = Path(tmp) / "live.tsv"
            live.write_text("table\tlive_count\nt1\t5\n")
            result = run_diff(["--manifest", str(manifest), "--live", str(live)])
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("GATE PASSED", result.stdout)


if __name__ == "__main__":
    unittest.main()
