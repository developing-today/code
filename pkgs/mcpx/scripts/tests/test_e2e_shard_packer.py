#!/usr/bin/env python3
"""Regression tests for pkgs/mcpx/scripts/ci/e2e-shard-packer.py.

Weight loading (#6822): load_weights must reject malformed timing
seeds -- a non-object `tests` field and non-finite / non-positive /
boolean per-test weights -- instead of crashing or poisoning the
median.

Discovery (#6906): the packer discovered tests by regex over raw file
text, ignoring Go build constraints, so tests behind an unsatisfied
//go:build tag were dealt into shards whose untagged `go test` legs
could never report them. The oracle for the discovery cases is the
default build context the shard legs run under (no -tags,
linux/amd64).

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
        for invalid_tests in ([["TestA", 10]], "TestA=10"):
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
            "TestLow",
            "TestHigh",
            "TestInfinity",
            "TestNegativeInfinity",
            "TestNaN",
            "TestZero",
            "TestNegative",
            "TestTrue",
            "TestFalse",
        ]
        weights, note = packer.load_weights(self.write_seed(value), names)

        invalid_names = [
            "TestInfinity",
            "TestNegativeInfinity",
            "TestNaN",
            "TestZero",
            "TestNegative",
            "TestTrue",
            "TestFalse",
        ]
        for name in invalid_names:
            with self.subTest(name=name):
                self.assertEqual(weights[name], 20.0)
                self.assertIn(name, note)
        self.assertEqual(weights["TestLow"], 10.0)
        self.assertEqual(weights["TestHigh"], 30.0)
        self.assertTrue(
            all(math.isfinite(weight) and weight > 0 for weight in weights.values())
        )

        # Every accepted weight remains usable by median/packing arithmetic.
        shards, buckets = packer.pack(names, weights, target_seconds=40, max_shards=6)
        self.assertEqual(shards, 5)
        self.assertEqual(
            sorted(name for bucket in buckets for name in bucket), sorted(names)
        )

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
                sys.executable,
                PACKER_PATH,
                "--tests-dir",
                os.path.join(MCPX_DIR, "internal", "e2e"),
                "--timings",
                TIMINGS_PATH,
                "--target-seconds",
                "170",
                "--max-shards",
                "6",
                "--github-output",
                output_path,
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


PACKER = os.path.abspath(
    os.path.join(os.path.dirname(__file__), "..", "ci", "e2e-shard-packer.py")
)


def load_packer():
    spec = importlib.util.spec_from_file_location("e2e_shard_packer", PACKER)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def go_file(body: str, build_line: str | None = None) -> str:
    header = f"{build_line}\n\n" if build_line else ""
    return f"{header}package e2e_test\n\n{body}\n"


def test_fn(name: str) -> str:
    return f"func {name}(t *testing.T) {{}}"


class DiscoverBuildConstraintTests(unittest.TestCase):
    def setUp(self):
        self.packer = load_packer()
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)

    def write(self, filename: str, content: str):
        with open(os.path.join(self.tmp.name, filename), "w") as fh:
            fh.write(content)

    def discover(self):
        return self.packer.discover(self.tmp.name)

    def test_untagged_tests_discovered(self):
        self.write(
            "plain_test.go",
            go_file(test_fn("TestPlain") + "\n" + test_fn("TestAlsoPlain")),
        )
        self.assertEqual(self.discover(), ["TestAlsoPlain", "TestPlain"])

    def test_unsatisfied_tag_excluded(self):
        self.write(
            "real_test.go",
            go_file(test_fn("TestRealSoftware"), "//go:build realsoftware"),
        )
        self.write("plain_test.go", go_file(test_fn("TestPlain")))
        self.assertEqual(self.discover(), ["TestPlain"])

    def test_ignore_tag_excluded(self):
        self.write(
            "ignored_test.go", go_file(test_fn("TestIgnored"), "//go:build ignore")
        )
        self.assertEqual(self.discover(), [])

    def test_other_os_excluded_and_default_os_included(self):
        self.write("win_test.go", go_file(test_fn("TestWindows"), "//go:build windows"))
        self.write(
            "lin_test.go", go_file(test_fn("TestLinux"), "//go:build linux && amd64")
        )
        self.write(
            "notwin_test.go", go_file(test_fn("TestNotWindows"), "//go:build !windows")
        )
        self.write(
            "either_test.go",
            go_file(test_fn("TestEitherOS"), "//go:build linux || darwin"),
        )
        self.assertEqual(
            self.discover(), ["TestEitherOS", "TestLinux", "TestNotWindows"]
        )

    def test_parenthesised_and_negated_custom_tag(self):
        self.write(
            "combo_test.go",
            go_file(
                test_fn("TestCombo"), "//go:build (linux && amd64) && !realsoftware"
            ),
        )
        self.write(
            "combo2_test.go",
            go_file(test_fn("TestComboTagged"), "//go:build linux && realsoftware"),
        )
        self.assertEqual(self.discover(), ["TestCombo"])

    def test_legacy_plus_build_lines(self):
        self.write(
            "legacy_win_test.go", go_file(test_fn("TestLegacyWin"), "// +build windows")
        )
        self.write(
            "legacy_lin_test.go",
            go_file(test_fn("TestLegacyLin"), "// +build linux,amd64"),
        )
        self.assertEqual(self.discover(), ["TestLegacyLin"])

    def test_filename_suffix_rules(self):
        self.write("thing_windows_test.go", go_file(test_fn("TestFileWindows")))
        self.write("thing_linux_test.go", go_file(test_fn("TestFileLinux")))
        self.write("thing_arm64_test.go", go_file(test_fn("TestFileArm64")))
        self.write("thing_amd64_test.go", go_file(test_fn("TestFileAmd64")))
        self.assertEqual(self.discover(), ["TestFileAmd64", "TestFileLinux"])

    def test_testmain_is_not_a_test(self):
        self.write(
            "main_test.go",
            go_file("func TestMain(m *testing.M) {}\n" + test_fn("TestReal")),
        )
        self.assertEqual(self.discover(), ["TestReal"])

    def test_mixed_fixture_pack_counts_match_discovery(self):
        self.write("plain_test.go", go_file(test_fn("TestA") + "\n" + test_fn("TestB")))
        self.write(
            "tagged_test.go", go_file(test_fn("TestTagged"), "//go:build realsoftware")
        )
        self.write("lin_test.go", go_file(test_fn("TestC"), "//go:build linux"))
        names = self.discover()
        self.assertEqual(names, ["TestA", "TestB", "TestC"])
        weights, _ = self.packer.load_weights(None, names)
        shards, buckets = self.packer.pack(names, weights, 170.0, 6)
        self.packer.assert_covered(names, buckets)
        self.assertEqual(sum(len(b) for b in buckets), len(names))
        self.assertGreaterEqual(shards, 1)


if __name__ == "__main__":
    unittest.main()
