#!/usr/bin/env python3
"""Snapshot-set equality tests for turso_mesh_verify_snapshots.py.

Oracle: every target's full restic snapshot-ID set must equal the R2 source
set; missing or extra IDs are a divergence.
"""

import importlib.util
import io
import itertools
import json
from pathlib import Path
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).with_name("turso_mesh_verify_snapshots.py")
SPEC = importlib.util.spec_from_file_location("turso_mesh_verify_snapshots", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC and SPEC.loader
SPEC.loader.exec_module(MODULE)


def completed(repo, ids, returncode=0):
    return type(
        "Result",
        (),
        {
            "returncode": returncode,
            "stdout": json.dumps([{"id": value} for value in ids]),
            "stderr": "",
        },
    )()


class SnapshotSetOracleTests(unittest.TestCase):
    def test_equal_sets_pass_even_when_restic_orders_snapshots_differently(self):
        responses = {
            "r2": completed("r2", ["a", "b", "c"]),
            "laptop": completed("laptop", ["c", "a", "b"]),
            "drive": completed("drive", ["b", "c", "a"]),
        }

        with patch.object(MODULE.subprocess, "run", side_effect=lambda argv, **_: responses[argv[-1]]):
            result = MODULE.verify_snapshot_sets("restic", {"r2": "r2", "laptop": "laptop", "drive": "drive"})

        self.assertEqual(result, {"status": "equal", "source": "r2", "snapshots": 3, "divergence": {}})

    def test_divergence_reports_missing_and_extra_snapshot_ids(self):
        responses = {
            "r2": completed("r2", ["a", "b"]),
            "laptop": completed("laptop", ["a", "c"]),
            "drive": completed("drive", ["a", "b"]),
        }

        with patch.object(MODULE.subprocess, "run", side_effect=lambda argv, **_: responses[argv[-1]]):
            result = MODULE.verify_snapshot_sets("restic", {"r2": "r2", "laptop": "laptop", "drive": "drive"})

        self.assertEqual(
            result,
            {
                "status": "diverged",
                "source": "r2",
                "snapshots": 2,
                "divergence": {"laptop": {"missing": ["b"], "extra": ["c"]}},
            },
        )

    def test_snapshot_set_equality_property_for_all_small_sets(self):
        universe = ("a", "b", "c", "d")
        subsets = [
            {universe[index] for index, bit in enumerate(bits) if bit}
            for bits in itertools.product((False, True), repeat=len(universe))
        ]

        for source in subsets:
            for target in subsets:
                responses = {
                    "r2": completed("r2", sorted(source)),
                    "laptop": completed("laptop", sorted(target)),
                    "drive": completed("drive", sorted(source)),
                }
                with self.subTest(source=source, target=target), patch.object(
                    MODULE.subprocess, "run", side_effect=lambda argv, **_: responses[argv[-1]]
                ):
                    result = MODULE.verify_snapshot_sets(
                        "restic", {"r2": "r2", "laptop": "laptop", "drive": "drive"}
                    )
                self.assertEqual(result["status"] == "equal", source == target)

    def test_invalid_snapshot_id_is_rejected(self):
        responses = {
            "r2": completed("r2", [""]),
        }

        with patch.object(MODULE.subprocess, "run", side_effect=lambda argv, **_: responses[argv[-1]]):
            with self.assertRaises(MODULE.VerificationError):
                MODULE.verify_snapshot_sets("restic", {"r2": "r2", "laptop": "laptop", "drive": "drive"})

    def test_unknown_argument_is_rejected_before_restic_runs(self):
        restic = unittest.mock.Mock()
        environment = {
            "TURSO_MESH_R2_REPOSITORY": "r2",
            "TURSO_MESH_LAPTOP_REPOSITORY": "laptop",
            "TURSO_MESH_DRIVE_REPOSITORY": "drive",
            "PATH": "/usr/bin",
        }

        with patch.object(MODULE.shutil, "which", return_value="/usr/bin/restic"):
            with self.assertRaises(SystemExit) as error:
                MODULE.main(
                    ["--typo"],
                    environ=environment,
                    runner=restic,
                    out=io.StringIO(),
                    err=io.StringIO(),
                )

        self.assertEqual(error.exception.code, 2)
        restic.assert_not_called()


if __name__ == "__main__":
    unittest.main()
