#!/usr/bin/env python3
"""Tests for the tip-hygiene census (code#3205).

Everything runs offline: `--fixture` replaces the git read with a JSON
entry list, so no checkout and no network are needed.

Oracle: the fixture census below is classified BY HAND from the class
definitions in check_tip.py's header (which restate the junk classes
named in code#3205). The expected counts and byte totals in
EXPECTED_CLASSES are that hand computation, not the script's output.
The partition test states the invariant the census must keep for any
entry list: every entry lands in exactly one class, and the class
counts/bytes sum to the tip totals.
"""

import json
import random
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
SCRIPT = HERE / "check_tip.py"
sys.path.insert(0, str(HERE))

import check_tip  # noqa: E402

# Hand-classified fixture. Class per entry, per the header definitions:
FIXTURE_ENTRIES = [
    {"path": "src/main.rs", "size": 1000},            # content
    {"path": "doc/big.pdf", "size": 20_000_000},      # content, large
    {"path": "target/debug/app.o", "size": 500},      # build-output
    {"path": "pkgs/x/target/lib.rlib", "size": 700},  # build-output
    {"path": "logs/run.log", "size": 300},            # log
    {"path": "data/app.db", "size": 4000},            # database
    {"path": "scratch/notes.txt", "size": 50},        # scratch
    {"path": "bin/tool.exe", "size": 2000},           # binary
    {"path": "lib/libfoo.so", "size": 1500},          # binary
    {"path": "vendor/host.rh", "size": 250},          # binary
    {"path": "dump/state.sqlite3", "size": 800},      # scratch (path wins over .sqlite3)
    {"path": "README.md", "size": 100},               # content
]

EXPECTED_CLASSES = {
    "content": {"count": 3, "bytes": 20_001_100},
    "build-output": {"count": 2, "bytes": 1200},
    "binary": {"count": 3, "bytes": 3750},
    "log": {"count": 1, "bytes": 300},
    "database": {"count": 1, "bytes": 4000},
    "scratch": {"count": 2, "bytes": 850},
}
EXPECTED_TOTAL_COUNT = 12
EXPECTED_TOTAL_BYTES = 20_011_200
EXPECTED_JUNK = {"count": 9, "bytes": 10_100}


def run_fixture(entries, *extra_args):
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as fh:
        json.dump({"entries": entries}, fh)
        path = fh.name
    cmd = [sys.executable, str(SCRIPT), "--fixture", path, *extra_args]
    proc = subprocess.run(cmd, capture_output=True, text=True)
    return proc


class FixtureCensusTest(unittest.TestCase):
    def test_fixture_census_matches_hand_computed_oracle(self):
        proc = run_fixture(FIXTURE_ENTRIES)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        report = json.loads(proc.stdout)
        self.assertEqual(report["blob_count"], EXPECTED_TOTAL_COUNT)
        self.assertEqual(report["total_bytes"], EXPECTED_TOTAL_BYTES)
        self.assertEqual(report["classes"], EXPECTED_CLASSES)
        self.assertEqual(report["junk"], EXPECTED_JUNK)

    def test_large_blobs_default_threshold(self):
        proc = run_fixture(FIXTURE_ENTRIES)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        report = json.loads(proc.stdout)
        self.assertEqual(
            report["large_blobs"],
            [{"path": "doc/big.pdf", "size": 20_000_000}],
        )

    def test_large_blob_threshold_flag(self):
        proc = run_fixture(FIXTURE_ENTRIES, "--large-blob-mib", "25")
        self.assertEqual(proc.returncode, 0, proc.stderr)
        report = json.loads(proc.stdout)
        self.assertEqual(report["large_blobs"], [])


class PartitionInvariantTest(unittest.TestCase):
    def test_classes_partition_any_entry_list(self):
        rng = random.Random(3205)
        fragments = [
            "src", "doc", "target", "scratch", "dump", "logs", "data",
            "a.rs", "b.o", "c.log", "d.db", "e.so", "f.md", "g.sqlite",
            "h.rlib", "i.txt", "j.exe",
        ]
        for _ in range(50):
            entries = [
                {
                    "path": "/".join(rng.choice(fragments) for _ in range(rng.randint(1, 3))),
                    "size": rng.randint(0, 5000),
                }
                for _ in range(rng.randint(0, 30))
            ]
            report = check_tip.census(entries)
            self.assertEqual(
                sum(c["count"] for c in report["classes"].values()),
                report["blob_count"],
            )
            self.assertEqual(
                sum(c["bytes"] for c in report["classes"].values()),
                report["total_bytes"],
            )
            self.assertEqual(report["blob_count"], len(entries))
            self.assertEqual(
                report["total_bytes"], sum(e["size"] for e in entries)
            )


class ArgumentContractTest(unittest.TestCase):
    def test_unknown_argument_is_usage_error(self):
        proc = run_fixture(FIXTURE_ENTRIES, "--bogus")
        self.assertEqual(proc.returncode, 2)

    def test_missing_source_is_usage_error(self):
        proc = subprocess.run(
            [sys.executable, str(SCRIPT)], capture_output=True, text=True
        )
        self.assertEqual(proc.returncode, 2)

    def test_both_sources_is_usage_error(self):
        proc = run_fixture(FIXTURE_ENTRIES, "--repo-dir", ".")
        self.assertEqual(proc.returncode, 2)


if __name__ == "__main__":
    unittest.main()
