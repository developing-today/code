#!/usr/bin/env python3
"""Regression tests for pkgs/mcpx/scripts/ci/e2e-shard-packer.py discovery.

Issue #6906: the packer discovered tests by regex over raw file text,
ignoring Go build constraints, so tests behind an unsatisfied
//go:build tag (e.g. realsoftware) were dealt into shards whose untagged
`go test` legs could never report them — the leg's count check then
failed. The oracle for every case here is the default build context the
shard legs run under (no -tags, linux/amd64): a fixture file's tests are
discovered iff an untagged linux/amd64 `go test` build would contain
them.

Run: python3 -m unittest discover -s pkgs/mcpx/scripts/tests -p 'test_e2e_shard_packer.py' -v
"""

import importlib.util
import os
import tempfile
import unittest

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
