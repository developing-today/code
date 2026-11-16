#!/usr/bin/env python3
"""CEN-05 — offline reference model and experiment runner tests (#9435).

Conformance tests for `experiment.py` against the pinned fixture
contract. The oracles here are independent of the runner's internals:
golden vectors (the fixture's own expected order, the RFC 8785 framing
vectors in contracts/digest-vectors.json), a reference sort and gate
implemented in this file, a BFS reachability oracle for the cycle
insertion rule, a permutations enumeration for exact inclusion
probabilities, and the fixture digest recomputed from raw bytes.
No production code is read, imported, or used as its own oracle; the
runner is exercised through its CLI (subprocess, `python3 -I`) and
through its documented model functions.

Run:  python3 -m unittest -v test_experiment   (lists every test)
      python3 test_experiment.py               (same suite)
"""

from __future__ import annotations

import collections
import decimal
import hashlib
import importlib.util
import itertools
import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
EXPERIMENT = HERE / "experiment.py"
FIXTURES = HERE / "fixtures.json"
DIGEST_VECTORS = HERE.parent / "contracts" / "digest-vectors.json"
DECIMAL = decimal.Decimal


def _load_runner():
    spec = importlib.util.spec_from_file_location("experiment", EXPERIMENT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


runner = _load_runner()


def _run_cli(exp_path: Path, *extra: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, "-I", str(exp_path), *extra],
        capture_output=True,
        timeout=300,
    )


def _fixture_bytes() -> bytes:
    return FIXTURES.read_bytes()


def _fixtures() -> dict:
    return json.loads(_fixture_bytes())


class ReplayAndContractTest(unittest.TestCase):
    def test_experiment_replays_same_fixture_and_rejects_bad_contract(self):
        # Replay: the same fixture and seed produce a byte-stable report,
        # and the report's pinned inputs match independently computed ones.
        args = ("--seed", "9196", "--cases", "25", "--draws", "200")
        first = _run_cli(EXPERIMENT, *args)
        second = _run_cli(EXPERIMENT, *args)
        self.assertEqual(first.returncode, 0, first.stdout.decode("utf-8", "replace"))
        self.assertEqual(second.returncode, 0)
        self.assertEqual(first.stdout, second.stdout, "report is not byte-stable")
        report = json.loads(first.stdout)
        self.assertEqual(report["schema"], "pmo-offline-experiment/v1")
        self.assertEqual(report["result"], "pass")
        self.assertEqual(report["inputs"]["seed"], 9196)
        self.assertEqual(
            report["inputs"]["fixture_sha256"],
            hashlib.sha256(_fixture_bytes()).hexdigest(),
        )

        # Bad contracts fail closed: nonzero exit and a machine-readable
        # fail result, never a pass report or a silent coercion.
        text = _fixture_bytes().decode("utf-8")
        mutations = {
            "duplicate_key": text.replace(
                '"fixture_schema": "pmo-offline-fixtures/v1"',
                '"fixture_schema": "pmo-offline-fixtures/v1", '
                '"fixture_schema": "pmo-offline-fixtures/v1"',
                1,
            ).encode(),
            "unknown_major_schema": text.replace(
                "pmo-offline-fixtures/v1", "pmo-offline-fixtures/v2", 1
            ).encode(),
            "lossy_numeric_id": text.replace(
                '"canonical_id": "9007199254741001"', '"canonical_id": 9007199254741001', 1
            ).encode(),
            "invalid_utf8": b'{"fixture_schema": "pmo-offline-fixtures/v1", "x": "\xff"}',
            "missing_graph_section": json.dumps(
                {k: v for k, v in _fixtures().items() if k != "graph"}
            ).encode(),
        }
        for name, blob in mutations.items():
            with self.subTest(mutation=name):
                self.assertNotEqual(
                    blob, _fixture_bytes(), f"mutation {name} did not change the fixture"
                )
                with tempfile.TemporaryDirectory() as tmp:
                    workdir = Path(tmp)
                    shutil.copy(EXPERIMENT, workdir / "experiment.py")
                    (workdir / "fixtures.json").write_bytes(blob)
                    proc = _run_cli(workdir / "experiment.py", *args)
                self.assertEqual(proc.returncode, 1, proc.stdout.decode("utf-8", "replace"))
                verdict = json.loads(proc.stdout)
                self.assertEqual(verdict["result"], "fail")
                self.assertEqual(verdict["schema"], "pmo-offline-experiment/v1")


class EligibilityGateTest(unittest.TestCase):
    @staticmethod
    def _reference_gate(task: dict, claimer: dict, explicit_filter) -> bool:
        requirements = all(claimer.get(k) == v for k, v in task.get("requires", {}).items())
        predicate = (
            True
            if explicit_filter is None
            else all(task.get(k) == v for k, v in explicit_filter.items())
        )
        return requirements and predicate

    def test_requirements_and_filter_are_independent_conjunctive_gates(self):
        cases = _fixtures()["priority_and_eligibility"]["eligibility"]
        self.assertGreaterEqual(len(cases), 3)
        for case in cases:
            expected = self._reference_gate(case["task"], case["claimer"], case.get("filter"))
            self.assertEqual(expected, case["expected"], "golden case disagrees with reference gate")
            self.assertEqual(
                runner.task_eligible(case["task"], case["claimer"], case.get("filter")),
                case["expected"],
            )
        # Each gate alone can veto: requirement mismatch with a passing
        # filter, and filter mismatch with passing requirements.
        task = {"repo": "code", "requires": {"platform": "linux"}}
        self.assertFalse(runner.task_eligible(task, {"repo": "code", "platform": "windows"}, {"repo": "code"}))
        self.assertFalse(runner.task_eligible(task, {"repo": "code", "platform": "linux"}, {"repo": "opencode"}))
        self.assertTrue(runner.task_eligible(task, {"repo": "code", "platform": "linux"}, None))
        self.assertTrue(runner.task_eligible({"repo": "code"}, {"repo": "code"}, {}))


class PriorityOrderingTest(unittest.TestCase):
    def test_numeric_work_id_ordering_rank_9_before_10(self):
        golden = _fixtures()["priority_and_eligibility"]
        items = golden["items"]
        # Independent reference sort: urgency, exact integer peer rank,
        # creation time, then canonical ID UTF-8 bytes (README oracle).
        reference = sorted(
            items,
            key=lambda x: (
                x["urgency"],
                int(x["peer_rank"]),
                x["created_at"],
                x["canonical_id"].encode("utf-8"),
            ),
        )
        self.assertEqual([x["canonical_id"] for x in reference], golden["expected_order"])
        ordered_ranks = [x["peer_rank"] for x in reference]
        self.assertLess(ordered_ranks.index("9"), ordered_ranks.index("10"))
        # Decimal-string IDs above 2**53 survive exactly; a float would not.
        for item in items:
            self.assertEqual(str(int(item["canonical_id"])), item["canonical_id"])
        self.assertGreater(int(items[0]["canonical_id"]), 1 << 53)
        self.assertNotEqual(int(float(items[0]["canonical_id"])), int(items[0]["canonical_id"]))


class DigestVectorTest(unittest.TestCase):
    def test_rfc8785_dto_bytes_match_golden_vectors(self):
        doc = json.loads(DIGEST_VECTORS.read_text())
        self.assertEqual(doc["schema_version"], "fleet.digest-framing-fixtures/v1")
        self.assertGreaterEqual(len(doc["vectors"]), 3)
        for vector in doc["vectors"]:
            with self.subTest(domain=vector["domain"]):
                # Framing per the contract: ASCII domain, one NUL byte,
                # then the RFC 8785 canonical UTF-8 bytes of the DTO.
                framed = (
                    vector["domain"].encode("ascii")
                    + b"\x00"
                    + vector["canonical_json"].encode("utf-8")
                )
                self.assertEqual(framed.hex(), vector["framed_hex"])
                self.assertEqual(
                    "sha256:" + hashlib.sha256(framed).hexdigest(), vector["digest"]
                )


def _bfs_reachable(graph: list[set[int]], start: int, goal: int) -> bool:
    seen = {start}
    queue = collections.deque([start])
    while queue:
        node = queue.popleft()
        if node == goal:
            return True
        for nxt in graph[node]:
            if nxt not in seen:
                seen.add(nxt)
                queue.append(nxt)
    return False


class CycleRaceTest(unittest.TestCase):
    def test_cycle_race_insertion_rule(self):
        chain = [{1}, {2}, set()]
        # Inserting u->v cycles iff u == v or v already reaches u.
        candidates = {(2, 0): True, (0, 2): False, (1, 1): True, (2, 1): True, (0, 1): False}
        for (u, v), expected in candidates.items():
            with self.subTest(edge=(u, v)):
                rule = u == v or _bfs_reachable(chain, v, u)
                self.assertEqual(rule, expected)
                graph = [set(row) for row in chain]
                graph[u].add(v)
                self.assertEqual(runner.has_cycle_color_dfs(graph), expected)

    def test_fixture_graph_blocker_orientation(self):
        graph_fixture = _fixtures()["graph"]
        index = {name: i for i, name in enumerate(graph_fixture["nodes"])}
        graph = [set() for _ in graph_fixture["nodes"]]
        for edge in graph_fixture["edges"]:
            if edge["kind"] == "hard_blocker":
                graph[index[edge["from"]]].add(index[edge["to"]])
        self.assertEqual(
            runner.reachable_dfs(graph, index["blocker"], index["dependent"]),
            graph_fixture["expected_blocker_to_dependent"],
        )
        self.assertEqual(
            runner.reachable_dfs(graph, index["parent"], index["advisory_target"]),
            graph_fixture["expected_parent_and_advisory_blocking"],
        )


class CompletionModelTest(unittest.TestCase):
    def test_cursor_crash_recovery_at_every_cut(self):
        model = runner.CompletionModel()

        def cut(m):
            return runner.CompletionModel.reload(m.serialize())

        self.assertTrue(
            model.prepare("worker-A", 1, authorized=True, deps_clear=True, operation="op-1")
        )
        model = cut(model)
        self.assertTrue(model.send("worker-A", 1))
        self.assertEqual(model.op["status"], "unknown")
        model = cut(model)  # crash while the send outcome is unknown
        self.assertEqual(model.op["status"], "unknown")
        self.assertFalse(model.finalize("op-1"))
        model.apply_remote("op-1")
        model = cut(model)
        self.assertTrue(model.readback("op-1"))
        model = cut(model)
        self.assertTrue(model.finalize("op-1"))
        self.assertEqual(model.state, "completed")

    def test_operation_apply_then_timeout_no_blind_resend(self):
        model = runner.CompletionModel()
        # Negative preconditions reject without mutation.
        self.assertFalse(
            model.prepare("worker-B", 1, authorized=True, deps_clear=True, operation="op-1")
        )
        self.assertFalse(
            model.prepare("worker-A", 1, authorized=False, deps_clear=True, operation="op-1")
        )
        self.assertFalse(
            model.prepare("worker-A", 1, authorized=True, deps_clear=False, operation="op-1")
        )
        self.assertIsNone(model.op)
        self.assertEqual(model.state, "claimed")

        self.assertTrue(
            model.prepare("worker-A", 1, authorized=True, deps_clear=True, operation="op-1")
        )
        self.assertTrue(model.send("worker-A", 1))  # response lost: outcome unknown
        self.assertFalse(model.send("worker-A", 1))  # no blind resend while unknown
        model.apply_remote("op-1")  # the request had in fact landed before the timeout
        # A webhook is a hint only: duplicate deliveries are rejected and a
        # webhook alone never verifies or finalizes.
        self.assertTrue(model.webhook("delivery-1", "op-1"))
        self.assertFalse(model.webhook("delivery-1", "op-1"))
        self.assertFalse(model.finalize("op-1"))
        self.assertTrue(model.readback("op-1"))  # same-operation readback confirms
        self.assertTrue(model.finalize("op-1"))

        # Fencing: after a takeover, the stale owner's generation is dead.
        fenced = runner.CompletionModel()
        fenced.takeover()
        self.assertFalse(fenced.mutate_local("worker-A", 1, "claimed"))
        self.assertFalse(
            fenced.prepare("worker-A", 1, authorized=True, deps_clear=True, operation="op-2")
        )
        self.assertTrue(
            fenced.prepare("worker-B", 2, authorized=True, deps_clear=True, operation="op-2")
        )


class SamplerTest(unittest.TestCase):
    def test_seeded_sampler_probabilities(self):
        # The SHA-256 counter stream is seed-deterministic and portable.
        stream_a = [runner.CounterRng(9196).u53() for _ in range(1)]
        rng_a, rng_b, rng_c = runner.CounterRng(9196), runner.CounterRng(9196), runner.CounterRng(9197)
        seq_a = [rng_a.u53() for _ in range(50)]
        self.assertEqual(seq_a, [rng_b.u53() for _ in range(50)])
        self.assertNotEqual(seq_a, [rng_c.u53() for _ in range(50)])
        self.assertEqual(stream_a[0], seq_a[0])
        self.assertTrue(all(0.0 <= runner.CounterRng(s).unit() < 1.0 for s in range(5)))

        # Sampling is without replacement and bounded by the population.
        drawn = runner.draw_without_replacement(
            ["a", "b", "c"], [0.5, 0.3, 0.2], 2, runner.CounterRng(7)
        )
        self.assertEqual(len(drawn), 2)
        self.assertEqual(len(set(drawn)), 2)
        self.assertLessEqual(set(drawn), {"a", "b", "c"})
        self.assertEqual(
            sorted(runner.draw_without_replacement(["a", "b", "c"], [0.5, 0.3, 0.2], 99, runner.CounterRng(7))),
            ["a", "b", "c"],
        )

        # Exact inclusion probabilities vs an independent enumeration of
        # every ordered weighted sequence (Decimal arithmetic throughout).
        masses = [DECIMAL("0.5"), DECIMAL("0.3"), DECIMAL("0.2")]
        inclusion = [DECIMAL(0) for _ in masses]
        for seq in itertools.permutations(range(3), 2):
            prob = DECIMAL(1)
            remaining = list(range(3))
            for pick in seq:
                total = sum((masses[i] for i in remaining), DECIMAL(0))
                prob *= masses[pick] / total
                remaining.remove(pick)
            for pick in set(seq):
                inclusion[pick] += prob
        self.assertEqual(runner.exact_inclusion(masses, 2), inclusion)

        # Analytic first-draw oracle: masses sum to one, a lower urgency
        # score draws more mass at equal density, epsilon 1 is uniform.
        first = runner.decimal_first_draw([30, 10, 20], [100, 100, 100], "0")
        self.assertLess(abs(sum(first, DECIMAL(0)) - 1), DECIMAL("1e-40"))
        self.assertGreater(first[1], first[2])
        self.assertGreater(first[2], first[0])
        uniform = runner.decimal_first_draw([30, 10, 20], [100, 100, 100], "1")
        self.assertTrue(all(abs(p - DECIMAL(1) / 3) < DECIMAL("1e-50") for p in uniform))


class CliContractTest(unittest.TestCase):
    def test_cli_rejects_unknown_arguments_before_reading_fixtures(self):
        for argv in (("--bogus",), ("--cases", "0"), ("--draws", "-1"), ("--see", "1")):
            with self.subTest(argv=argv):
                proc = _run_cli(EXPERIMENT, *argv)
                self.assertEqual(proc.returncode, 2)
                self.assertEqual(proc.stdout, b"", "no report may be emitted on a usage error")
        # Parse precedes fixture access: even with a broken fixture in
        # place, an unknown argument is a usage error (exit 2), not a
        # fixture failure (exit 1).
        with tempfile.TemporaryDirectory() as tmp:
            workdir = Path(tmp)
            shutil.copy(EXPERIMENT, workdir / "experiment.py")
            (workdir / "fixtures.json").write_bytes(b"not json")
            proc = _run_cli(workdir / "experiment.py", "--bogus")
            self.assertEqual(proc.returncode, 2)


if __name__ == "__main__":
    unittest.main()
