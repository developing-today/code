#!/usr/bin/env python3
"""Unknown-argument contract tests for inventory operator entry points.

Run from repository root:
  python3 -I doc/2026-11-13T01-34-11Z_design_fleet_ca_credentials_secrets/inventory/tests/test_operator_arguments.py

Oracle: every no-option program exits with argparse's usage error before any
canonical/shard/report byte changes; the before/after SHA-256 inventory is the
decidable reference.
"""
from __future__ import annotations

import hashlib
import pathlib
import subprocess
import sys
import unittest


REPO = pathlib.Path(__file__).resolve().parents[4]
INV = pathlib.Path(__file__).resolve().parents[0].parent
SCRIPTS = (
    "validate_inventory.py",
    "build_canonical_inventory.py",
    "compile_refinements.py",
    "write_inventory_report.py",
    "audit_register_anchors.py",
)


def inventory_bytes():
    return {
        path.relative_to(INV).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in INV.rglob("*")
        if path.is_file() and "__pycache__" not in path.parts
    }


class StrictArgumentContract(unittest.TestCase):
    def test_unknown_argument_is_usage_error_before_inventory_changes(self):
        before = inventory_bytes()
        for script in SCRIPTS:
            with self.subTest(script=script):
                proc = subprocess.run(
                    [sys.executable, "-I", str(INV / script), "--typo-option"],
                    cwd=REPO,
                    capture_output=True,
                    text=True,
                    check=False,
                    timeout=30,
                )
                self.assertEqual(proc.returncode, 2, proc.stderr)
                self.assertIn("usage:", proc.stderr.lower())
                self.assertIn("--typo-option", proc.stderr)
                self.assertEqual(inventory_bytes(), before, f"{script} changed canonical/shard bytes")

    def test_positional_argument_is_rejected_before_inventory_changes(self):
        before = inventory_bytes()
        for script in SCRIPTS:
            with self.subTest(script=script):
                proc = subprocess.run(
                    [sys.executable, "-I", str(INV / script), "unexpected"],
                    cwd=REPO,
                    capture_output=True,
                    text=True,
                    check=False,
                    timeout=30,
                )
                self.assertEqual(proc.returncode, 2, proc.stderr)
                self.assertIn("unexpected", proc.stderr)
                self.assertEqual(inventory_bytes(), before, f"{script} changed canonical/shard bytes")


if __name__ == "__main__":
    unittest.main()
