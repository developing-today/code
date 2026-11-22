#!/usr/bin/env python3
"""Unit tests for scripts/id/gate.py, the pkgs/id batched-gate planner.

Oracle: brute force. For bisect, the culprits of a batch are exactly
the PRs whose individual verdict is red; the planner's halving walk
must return that same set. For planning, batches must partition the
input in order. For the log, every record must carry the batch's
command and result, because issue #12229's second acceptance
criterion reads them back.
"""
from __future__ import annotations

import importlib.util
import json
import math
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("gate.py")
SPEC = importlib.util.spec_from_file_location("gate", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
gate = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(gate)


class PlanBatchesTests(unittest.TestCase):
    def test_batches_partition_the_input_in_order(self) -> None:
        prs = [101, 102, 103, 104, 105, 106, 107, 108, 109]
        batches = gate.plan_batches(prs, batch_size=4)
        self.assertEqual(
            batches,
            [[101, 102, 103, 104], [105, 106, 107, 108], [109]],
        )
        flattened = [pr for batch in batches for pr in batch]
        self.assertEqual(flattened, prs)

    def test_empty_input_plans_no_batches(self) -> None:
        self.assertEqual(gate.plan_batches([], batch_size=4), [])

    def test_default_batch_size_is_the_design_record_value(self) -> None:
        batches = gate.plan_batches(list(range(1, 13)))
        self.assertTrue(all(len(b) <= gate.DEFAULT_BATCH_SIZE for b in batches))
        self.assertEqual(len(batches), math.ceil(12 / gate.DEFAULT_BATCH_SIZE))

    def test_gate_off_plans_nothing(self) -> None:
        self.assertEqual(gate.plan_batches([1, 2, 3], batch_size=2, enabled=False), [])


class BatchCommandTests(unittest.TestCase):
    def test_command_checks_workspace_and_tests_affected_crates(self) -> None:
        affected = {101: ["id-core", "id-store"], 102: ["id-core"]}
        command = gate.batch_command([101, 102], affected)
        self.assertIn("cargo check --workspace", command)
        self.assertIn("cargo test", command)
        self.assertIn("-p id-core", command)
        self.assertIn("-p id-store", command)
        # A crate affected by two PRs in the batch is tested once.
        self.assertEqual(command.count("-p id-core"), 1)

    def test_command_without_affected_crates_still_checks(self) -> None:
        command = gate.batch_command([101], {})
        self.assertIn("cargo check --workspace", command)
        self.assertNotIn("cargo test", command)


class BisectTests(unittest.TestCase):
    def culprits(self, batch: list[int], red: set[int]) -> list[int]:
        return gate.find_culprits(batch, lambda subset: any(pr in red for pr in subset))

    def test_single_red_pr_is_isolated(self) -> None:
        batch = [101, 102, 103, 104, 105, 106, 107, 108]
        self.assertEqual(self.culprits(batch, {105}), [105])

    def test_red_prs_at_both_ends_are_both_found(self) -> None:
        batch = [101, 102, 103, 104, 105]
        self.assertEqual(self.culprits(batch, {101, 105}), [101, 105])

    def test_all_red_returns_the_whole_batch(self) -> None:
        batch = [101, 102, 103]
        self.assertEqual(self.culprits(batch, {101, 102, 103}), batch)

    def test_green_batch_has_no_culprits(self) -> None:
        self.assertEqual(self.culprits([101, 102], set()), [])

    def test_single_red_costs_log_evaluations(self) -> None:
        batch = list(range(200, 216))
        calls = 0

        def is_red(subset: list[int]) -> bool:
            nonlocal calls
            calls += 1
            return 207 in subset

        self.assertEqual(gate.find_culprits(batch, is_red), [207])
        self.assertLessEqual(calls, 2 * math.ceil(math.log2(len(batch))) + 1)


class BatchLogTests(unittest.TestCase):
    def test_log_records_each_batch_command_and_result(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "gate-log.jsonl"
            gate.append_log(path, gate.log_record(1, [101, 102], "cargo check --workspace", "green"))
            gate.append_log(path, gate.log_record(2, [103], "cargo check --workspace", "red"))
            records = gate.read_log(path)
        self.assertEqual(len(records), 2)
        self.assertEqual(records[0]["batch"], 1)
        self.assertEqual(records[0]["prs"], [101, 102])
        self.assertEqual(records[0]["command"], "cargo check --workspace")
        self.assertEqual(records[0]["result"], "green")
        self.assertEqual(records[1]["result"], "red")
        for record in records:
            self.assertIn("ts", record)

    def test_log_round_trips_through_json_lines(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "gate-log.jsonl"
            gate.append_log(path, gate.log_record(1, [101], "cmd", "green"))
            raw = path.read_text(encoding="utf-8").strip().splitlines()
        self.assertEqual(len(raw), 1)
        self.assertEqual(json.loads(raw[0])["prs"], [101])


if __name__ == "__main__":
    unittest.main()
