#!/usr/bin/env python3
"""Regression tests for the measured-time e2e shard packer (#6822).

Run: python3 -m unittest discover -s pkgs/mcpx/scripts/tests -p 'test_e2e_shard_packer.py' -v
"""

import importlib.util
import json
import math
import os
import subprocess
import sys
import tempfile
import unittest


TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
MCPX_DIR = os.path.abspath(os.path.join(TESTS_DIR, "..", ".."))
REPO_ROOT = os.path.abspath(os.path.join(MCPX_DIR, "..", ".."))
PACKER_PATH = os.path.join(MCPX_DIR, "scripts", "ci", "e2e-shard-packer.py")
TIMINGS_PATH = os.path.join(MCPX_DIR, "scripts", "ci", "e2e-timings.json")

SPEC = importlib.util.spec_from_file_location("e2e_shard_packer", PACKER_PATH)
packer = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(packer)


class WeightLoadingTest(unittest.TestCase):
    def write_seed(self, value):
        # Write-close-read: close the temp file before anything reopens
        # its path (an open NamedTemporaryFile cannot be reopened on
        # Windows).
        with tempfile.NamedTemporaryFile(
            mode="w", suffix=".json", delete=False
        ) as seed:
            json.dump(value, seed)
            path = seed.name
        self.addCleanup(os.unlink, path)
        return path

    def load_seed(self, value):
        return packer.load_weights(self.write_seed(value), ["TestA", "TestB"])

    def test_non_object_tests_field_falls_back_with_named_reason(self):
        for invalid_tests in ([ ["TestA", 10] ], "TestA=10"):
            with self.subTest(tests=invalid_tests):
                weights, note = self.load_seed({"tests": invalid_tests})
                self.assertEqual(weights, {"TestA": 3.0, "TestB": 3.0})
                self.assertIn("'tests'", note)
                self.assertIn("expected object", note)
                self.assertIn("ignoring seed", note)

    def test_non_finite_and_boolean_weights_are_skipped_by_name(self):
        value = {
            "tests": {
                "TestLow": 10,
                "TestHigh": 30,
                "TestInfinity": float("inf"),
                "TestNegativeInfinity": float("-inf"),
                "TestNaN": float("nan"),
                "TestZero": 0,
                "TestNegative": -1,
                "TestTrue": True,
                "TestFalse": False,
            }
        }
        names = [
            "TestLow", "TestHigh", "TestInfinity", "TestNegativeInfinity",
            "TestNaN", "TestZero", "TestNegative", "TestTrue", "TestFalse",
        ]
        weights, note = packer.load_weights(self.write_seed(value), names)

        invalid_names = [
            "TestInfinity", "TestNegativeInfinity", "TestNaN", "TestZero",
            "TestNegative", "TestTrue", "TestFalse"
        ]
        for name in invalid_names:
            with self.subTest(name=name):
                self.assertEqual(weights[name], 20.0)
                self.assertIn(name, note)
        self.assertEqual(weights["TestLow"], 10.0)
        self.assertEqual(weights["TestHigh"], 30.0)
        self.assertTrue(all(math.isfinite(weight) and weight > 0 for weight in weights.values()))

        # Every accepted weight remains usable by median/packing arithmetic.
        shards, buckets = packer.pack(names, weights, target_seconds=40, max_shards=6)
        self.assertEqual(shards, 5)
        self.assertEqual(sorted(name for bucket in buckets for name in bucket), sorted(names))

    def test_all_invalid_seed_falls_back_and_names_rejected_entries(self):
        names = ["TestTrue", "TestZero"]
        weights, note = packer.load_weights(
            self.write_seed({"tests": {"TestTrue": True, "TestZero": 0}}), names
        )

        self.assertEqual(weights, {name: 3.0 for name in names})
        self.assertIn("no usable timing seed", note)
        self.assertIn("TestTrue", note)
        self.assertIn("TestZero", note)

    def test_well_formed_seed_preserves_weights_and_lpt_assignment(self):
        weights, note = packer.load_weights(
            self.write_seed({"tests": {"TestA": 12, "TestB": 8}}),
            ["TestA", "TestB", "TestC"],
        )

        self.assertEqual(weights, {"TestA": 12.0, "TestB": 8.0, "TestC": 10.0})
        self.assertEqual(note, "seed: 2 measured, median 10.0s for the rest")
        self.assertEqual(
            packer.pack(["TestA", "TestB", "TestC"], weights, 15, 3),
            (2, [["TestA"], ["TestB", "TestC"]]),
        )

    def test_no_seed_fallback_preserves_default_matrix_and_note(self):
        weights, note = packer.load_weights(None, ["TestA", "TestB", "TestC"])
        self.assertEqual(weights, {"TestA": 3.0, "TestB": 3.0, "TestC": 3.0})
        self.assertEqual(
            note,
            "equal weights at the run-41530429335 rate (3s/test; no timing seed)",
        )
        self.assertEqual(
            packer.pack(["TestA", "TestB", "TestC"], weights, 4, 6),
            (3, [["TestA"], ["TestB"], ["TestC"]]),
        )

    def test_committed_timing_seed_cli_emits_a_finite_matrix(self):
        # Write-close-read: close the output file before the subprocess
        # opens it by path (an open NamedTemporaryFile cannot be reopened
        # on Windows).
        with tempfile.NamedTemporaryFile(
            mode="w", suffix=".out", delete=False
        ) as output:
            output_path = output.name
        self.addCleanup(os.unlink, output_path)
        result = subprocess.run(
            [
                sys.executable, PACKER_PATH,
                "--tests-dir", os.path.join(MCPX_DIR, "internal", "e2e"),
                "--timings", TIMINGS_PATH,
                "--target-seconds", "170",
                "--max-shards", "6",
                "--github-output", output_path,
            ],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
            timeout=60,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        with open(output_path) as output:
            values = dict(line.rstrip("\n").split("=", 1) for line in output)

        matrix = json.loads(values["matrix"])
        include = matrix["include"]
        names = packer.discover(os.path.join(MCPX_DIR, "internal", "e2e"))
        assigned = [name for shard in include for name in shard["run"].split("|")]
        self.assertTrue(include)
        self.assertEqual(sorted(assigned), names)
        self.assertEqual(int(values["total"]), len(names))
        self.assertEqual(int(values["shards"]), len(include))
        for shard in include:
            self.assertTrue(math.isfinite(shard["seconds"]))
            self.assertTrue(math.isfinite(shard["longest"]))


if __name__ == "__main__":
    unittest.main()
