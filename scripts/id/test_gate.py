#!/usr/bin/env python3
"""Unit tests for scripts/id/gate.py, the pkgs/id batched-gate planner.

Oracle: brute force. For bisect, the culprits of a batch are exactly
the PRs whose individual verdict is red; the planner's halving walk
must return that same set. For planning, batches must partition the
input in order. For the log, every record must carry the batch's
command and result, because issue #12229's second acceptance
criterion reads them back. The count toward ten consecutive green
batches is the criterion's reader: a landed record names its
consecutive-green count, bisect probes carry none, and ``status``
reports the count against the target.
"""

from __future__ import annotations

import contextlib
import importlib.util
import io
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
                path,
                gate.log_record(
                    1, [101, 102], "cargo check --workspace", "green",
                    consecutive_green=1,
                ),
            )
            gate.append_log(
                path,
                gate.log_record(
                    2, [103], "cargo check --workspace", "red",
                    consecutive_green=0,
                ),
            )
            records = gate.read_log(path)
        self.assertEqual(len(records), 2)
        self.assertEqual(records[0]["batch"], 1)
        self.assertEqual(records[0]["prs"], [101, 102])
        self.assertEqual(records[0]["command"], "cargo check --workspace")
        self.assertEqual(records[0]["result"], "green")
        self.assertEqual(records[0]["consecutive_green"], 1)
        self.assertEqual(records[0]["probe"], False)
        self.assertEqual(records[1]["result"], "red")
        self.assertEqual(records[1]["consecutive_green"], 0)
        for record in records:
            self.assertIn("ts", record)

    def test_log_round_trips_through_json_lines(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "gate-log.jsonl"
            gate.append_log(
                path,
                gate.log_record(1, [101], "cmd", "green", consecutive_green=1),
            )
            raw = path.read_text(encoding="utf-8").strip().splitlines()
        self.assertEqual(len(raw), 1)
        self.assertEqual(json.loads(raw[0])["prs"], [101])

    def test_probe_records_carry_no_green_count(self) -> None:
        record = gate.log_record(
            1, [101], "cmd", "green", probe=True, consecutive_green=None
        )
        self.assertEqual(record["probe"], True)
        self.assertIsNone(record["consecutive_green"])
        with self.assertRaises(ValueError):
            gate.log_record(
                1, [101], "cmd", "green", probe=True, consecutive_green=1
            )

    def test_landed_record_without_a_count_is_a_usage_error(self) -> None:
        with self.assertRaises(ValueError):
            gate.log_record(1, [101], "cmd", "green")

    def test_appended_records_advance_and_reset_the_green_count(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "gate-log.jsonl"
            first = gate.append_batch_record(path, 1, [101], "cmd", "green")
            second = gate.append_batch_record(path, 2, [102], "cmd", "green")
            probe = gate.append_batch_record(
                path, 3, [103], "cmd", "green", probe=True
            )
            third = gate.append_batch_record(path, 3, [103, 104], "cmd", "red")
            fourth = gate.append_batch_record(path, 4, [105], "cmd", "green")
            # The probe never counted as a landed batch; read the count
            # back from the file while the temp dir still exists.
            filed = gate.consecutive_green(gate.read_log(path))
        self.assertEqual(first["consecutive_green"], 1)
        self.assertEqual(second["consecutive_green"], 2)
        self.assertIsNone(probe["consecutive_green"])
        self.assertEqual(third["consecutive_green"], 0)
        self.assertEqual(fourth["consecutive_green"], 1)
        self.assertEqual(filed, 1)

    def test_status_reports_the_count_against_ten(self) -> None:
        records = [
            gate.log_record(1, [101], "cmd", "green", consecutive_green=1),
            gate.log_record(2, [102], "cmd", "green", consecutive_green=2),
            gate.log_record(
                3, [103], "cmd", "red", probe=True, consecutive_green=None
            ),
            gate.log_record(3, [103], "cmd", "red", consecutive_green=0),
        ]
        status = gate.gate_status(records)
        self.assertEqual(status["consecutive_green"], 0)
        self.assertEqual(status["target"], 10)
        self.assertEqual(status["batches"], 3)
        self.assertEqual(status["complete"], False)

        done = [
            gate.log_record(
                b, [100 + b], "cmd", "green", consecutive_green=b
            )
            for b in range(1, 11)
        ]
        self.assertEqual(gate.gate_status(done)["complete"], True)
        self.assertEqual(gate.gate_status(done)["consecutive_green"], 10)

    def test_status_reads_records_written_before_the_count_existed(self) -> None:
        # Old records have no probe/consecutive_green keys; read by result.
        records = [
            {"batch": 1, "prs": [101], "command": "cmd", "result": "green"},
            {"batch": 2, "prs": [102], "command": "cmd", "result": "green"},
            {"batch": 3, "prs": [103], "command": "cmd", "result": "red"},
        ]
        self.assertEqual(gate.consecutive_green(records), 0)
        self.assertEqual(gate.gate_status(records)["batches"], 3)


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
                [12470],
                1,
                affected={12470: ["id-core"]},
                log_path=Path(tmp) / "gate.jsonl",
                repo=Path(tmp),
                git=git,
                shell=shell,
                gh=gh,
            )
            self.assertEqual(result, "green")
            self.assertTrue(any(name == "shell" for name, _ in events))
            self.assertTrue(
                any(name == "gh" and "merge" in args for name, args in events)
            )
            (record,) = gate.read_log(Path(tmp) / "gate.jsonl")
            self.assertEqual(record["result"], "green")
            self.assertEqual(record["prs"], [12470])
            # A landed batch names its green count; it is not a probe.
            self.assertEqual(record["probe"], False)
            self.assertEqual(record["consecutive_green"], 1)

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
                [1, 2],
                7,
                affected={1: ["a"], 2: ["b"]},
                log_path=Path(tmp) / "gate.jsonl",
                repo=Path(tmp),
                git=git,
                shell=verdict,
                gh=gh,
            )
            self.assertEqual(result, "red")
            records = gate.read_log(Path(tmp) / "gate.jsonl")
            self.assertEqual(
                [(r["prs"], r["result"]) for r in records],
                [([1, 2], "red"), ([1], "green"), ([2], "red")],
            )
            # Only the whole-batch verdict counts: the root red resets the
            # count to 0, and the bisect probes carry no count at all.
            self.assertEqual(
                [(r["probe"], r["consecutive_green"]) for r in records],
                [(False, 0), (True, None), (True, None)],
            )
            self.assertEqual(gate.consecutive_green(records), 0)
            self.assertTrue(any("merge" in args and "1" in args for args in gh_calls))
            self.assertFalse(any("merge" in args and "2" in args for args in gh_calls))

    def test_gate_off_runs_nothing(self) -> None:
        with mock.patch.dict(os.environ, {"ID_GATE_OFF": "1"}):
            result = gate.run_batch(
                [1],
                1,
                affected={},
                log_path=Path("unused.jsonl"),
                repo=Path("."),
                git=lambda *a, **k: self.fail("git ran"),
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
                [1, 2],
                8,
                affected={1: [], 2: []},
                log_path=Path(tmp) / "gate.jsonl",
                repo=Path(tmp),
                git=lambda *args, **kwargs: "",
                shell=shell,
                gh=gh,
            )
            self.assertEqual(result, "red")
            self.assertEqual(merges, [])
            self.assertEqual(len(gate.read_log(Path(tmp) / "gate.jsonl")), 3)


class DevShellRunnerTests(unittest.TestCase):
    """Oracle: the batch-1 failure record on #12229. The fleet build shell
    lacks the wayland client libraries, so a batch command run there dies in
    the wayland-sys build script before any PR code is evaluated. The runner
    must therefore enter pkgs/id's own nix dev shell — the flake that wires
    wayland/X11 via pkgs/id/nix-common.nix — while passing the batch command
    through verbatim (the JSONL log records that exact command)."""

    def invoke(self, command: str, cwd: Path, returncode: int = 0) -> list[str]:
        calls: list[list[str]] = []

        def fake_run(argv: list[str], **kwargs: object) -> mock.Mock:
            calls.append(argv)
            self.assertEqual(kwargs.get("cwd"), cwd)
            return mock.Mock(returncode=returncode)

        with mock.patch.object(gate.subprocess, "run", fake_run):
            result = gate._run_devshell(command, cwd=cwd)
        self.assertEqual(result, returncode)
        return calls[0]

    def test_runner_enters_the_pkgs_id_dev_shell(self) -> None:
        candidate = Path("/candidate")
        argv = self.invoke("cargo check --workspace", candidate)
        self.assertEqual(argv[0], "nix")
        self.assertIn("develop", argv)
        # The dev shell entered is pkgs/id's flake, not the repo root's.
        self.assertIn(str(candidate / "pkgs" / "id"), argv)

    def test_runner_passes_the_batch_command_verbatim_to_a_shell(self) -> None:
        command = gate.batch_command([101], {101: ["id-core"]})
        argv = self.invoke(command, Path("/candidate"))
        # bash -c <command>: the && chain needs a shell, and the command the
        # log records must be the command that ran, byte for byte.
        self.assertEqual(argv[-3:], ["bash", "-c", command])

    def test_runner_propagates_the_exit_code(self) -> None:
        argv = self.invoke(
            "cargo check --workspace", Path("/candidate"), returncode=101
        )
        self.assertEqual(argv[0], "nix")


class StatusCommandTests(unittest.TestCase):
    def test_status_command_prints_the_count_as_json(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "gate.jsonl"
            gate.append_batch_record(path, 1, [101], "cmd", "green")
            gate.append_batch_record(path, 2, [102], "cmd", "green")
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                rc = gate.main(["status", "--file", str(path)])
        self.assertEqual(rc, 0)
        self.assertEqual(
            json.loads(out.getvalue()),
            {
                "consecutive_green": 2,
                "target": 10,
                "batches": 2,
                "complete": False,
            },
        )

    def test_log_command_appends_a_counted_record(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "gate.jsonl"
            rc = gate.main(
                [
                    "log",
                    "--file", str(path),
                    "--batch", "1",
                    "--prs", "101",
                    "--command", "cmd",
                    "--result", "green",
                ]
            )
            (record,) = gate.read_log(path)
        self.assertEqual(rc, 0)
        self.assertEqual(record["consecutive_green"], 1)
        self.assertEqual(record["probe"], False)


if __name__ == "__main__":
    unittest.main()
