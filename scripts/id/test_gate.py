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
import os
import tempfile
import unittest
from unittest import mock
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
            gate.append_log(
                path, gate.log_record(1, [101, 102], "cargo check --workspace", "green")
            )
            gate.append_log(
                path, gate.log_record(2, [103], "cargo check --workspace", "red")
            )
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


class RunBatchTests(unittest.TestCase):
    """Oracle: injected command/GitHub fakes capture the exact gate state transitions."""

    def test_green_batch_checks_then_merges_and_logs(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            events: list[tuple[str, object]] = []

            def git(args: list[str], **kwargs: object) -> str:
                events.append(("git", args))
                return ""

            def shell(command: str, **kwargs: object) -> int:
                events.append(("shell", command))
                return 0

            def gh(args: list[str], **kwargs: object) -> str:
                events.append(("gh", args))
                if args[:2] == ["pr", "view"]:
                    return '{"headRefOid":"deadbeef"}'
                return ""

            result = gate.run_batch(
                [12470], 1, affected={12470: ["id-core"]},
                log_path=Path(tmp) / "gate.jsonl", repo=Path(tmp),
                git=git, shell=shell, gh=gh,
            )
            self.assertEqual(result, "green")
            self.assertTrue(any(name == "shell" for name, _ in events))
            self.assertTrue(any(name == "gh" and "merge" in args for name, args in events))
            record, = gate.read_log(Path(tmp) / "gate.jsonl")
            self.assertEqual(record["result"], "green")
            self.assertEqual(record["prs"], [12470])

    def test_red_batch_bisects_merges_green_and_logs_all_runs(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            shell_calls: list[str] = []
            gh_calls: list[list[str]] = []

            def git(args: list[str], **kwargs: object) -> str:
                return ""

            def shell(command: str, **kwargs: object) -> int:
                shell_calls.append(command)
                # Oracle fixture: PR 2 is the sole culprit.
                prs = kwargs.get("prs", [])
                return int(2 in prs)

            def gh(args: list[str], **kwargs: object) -> str:
                gh_calls.append(args)
                return '{"headRefOid":"deadbeef"}' if args[:2] == ["pr", "view"] else ""

            def verdict(command: str, *, prs: list[int], **kwargs: object) -> int:
                return int(2 in prs)

            result = gate.run_batch(
                [1, 2], 7, affected={1: ["a"], 2: ["b"]},
                log_path=Path(tmp) / "gate.jsonl", repo=Path(tmp),
                git=git, shell=verdict, gh=gh,
            )
            self.assertEqual(result, "red")
            records = gate.read_log(Path(tmp) / "gate.jsonl")
            self.assertEqual([(r["prs"], r["result"]) for r in records],
                             [([1, 2], "red"), ([1], "green"), ([2], "red")])
            self.assertTrue(any("merge" in args and "1" in args for args in gh_calls))
            self.assertFalse(any("merge" in args and "2" in args for args in gh_calls))

    def test_gate_off_runs_nothing(self) -> None:
        with mock.patch.dict(os.environ, {"ID_GATE_OFF": "1"}):
            result = gate.run_batch(
                [1], 1, affected={}, log_path=Path("unused.jsonl"),
                repo=Path("."), git=lambda *a, **k: self.fail("git ran"),
                shell=lambda *a, **k: self.fail("shell ran"),
                gh=lambda *a, **k: self.fail("gh ran"),
            )
        self.assertEqual(result, "off")

    def test_red_only_when_combined_leaves_all_prs_open(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            merges: list[list[str]] = []

            def shell(command: str, *, prs: list[int], **kwargs: object) -> int:
                return int(prs == [1, 2])

            def gh(args: list[str], **kwargs: object) -> str:
                if args[:2] == ["pr", "view"]:
                    return '{"headRefOid":"deadbeef"}'
                merges.append(args)
                return ""

            result = gate.run_batch(
                [1, 2], 8, affected={1: [], 2: []},
                log_path=Path(tmp) / "gate.jsonl", repo=Path(tmp),
                git=lambda *args, **kwargs: "", shell=shell, gh=gh,
            )
            self.assertEqual(result, "red")
            self.assertEqual(merges, [])
            self.assertEqual(len(gate.read_log(Path(tmp) / "gate.jsonl")), 3)


if __name__ == "__main__":
    unittest.main()
