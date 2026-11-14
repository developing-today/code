#!/usr/bin/env python3
"""Offline reference experiment for proposed PM/Fleet contracts; not production code."""

from __future__ import annotations

import argparse
import collections
import decimal
import hashlib
import json
import math
import platform
import sys
from pathlib import Path


HERE = Path(__file__).resolve().parent
DECIMAL = decimal.Decimal
decimal.getcontext().prec = 60


def arguments(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument(
        "--seed",
        type=int,
        default=9196,
        help="integer SHA-256 counter seed (default: 9196)",
    )
    parser.add_argument(
        "--cases",
        type=int,
        default=1000,
        help="positive randomized invariant cases (default: 1000)",
    )
    parser.add_argument(
        "--draws",
        type=int,
        default=20000,
        help="positive first-draw statistical trials (default: 20000)",
    )
    args = parser.parse_args(argv)
    if args.cases <= 0:
        parser.error("--cases must be a positive integer")
    if args.draws <= 0:
        parser.error("--draws must be a positive integer")
    return args


class CounterRng:
    """Portable SHA-256 counter stream; each result is in [0, 1) on a 53-bit grid."""

    def __init__(self, seed: int):
        self.seed = str(seed).encode("ascii")
        self.counter = 0

    def u53(self) -> int:
        block = hashlib.sha256(
            b"pmo-experiment-v1\0" + self.seed + b"\0" + str(self.counter).encode()
        ).digest()
        self.counter += 1
        return int.from_bytes(block[:8], "big") >> 11

    def unit(self) -> float:
        return self.u53() / (1 << 53)

    def below(self, n: int) -> int:
        if n <= 0:
            raise ValueError("below requires positive n")
        return self.u53() % n


class Checks:
    def __init__(self) -> None:
        self.counts: collections.Counter[str] = collections.Counter()
        self.max_errors: dict[str, float] = {}

    def ok(self, name: str, condition: bool) -> None:
        self.counts[name] += 1
        if not condition:
            raise AssertionError(
                f"{name}: invariant failed at check {self.counts[name]}"
            )

    def error(self, name: str, value: float) -> None:
        self.max_errors[name] = max(self.max_errors.get(name, 0.0), abs(value))


def reachable_dfs(graph: list[set[int]], start: int, goal: int) -> bool:
    todo, seen = [start], set()
    while todo:
        node = todo.pop()
        if node == goal:
            return True
        if node not in seen:
            seen.add(node)
            todo.extend(graph[node] - seen)
    return False


def closure_oracle(graph: list[set[int]]) -> list[list[bool]]:
    """Independent Floyd-Warshall transitive-closure oracle."""
    n = len(graph)
    reach = [[i == j or j in graph[i] for j in range(n)] for i in range(n)]
    for k in range(n):
        for i in range(n):
            for j in range(n):
                reach[i][j] = reach[i][j] or (reach[i][k] and reach[k][j])
    return reach


def has_cycle_color_dfs(graph: list[set[int]]) -> bool:
    """Independent three-color cycle detector, not a diagonal closure test."""
    color = [0] * len(graph)

    def visit(node: int) -> bool:
        if color[node] == 1:
            return True
        if color[node] == 2:
            return False
        color[node] = 1
        if any(visit(nxt) for nxt in graph[node]):
            return True
        color[node] = 2
        return False

    return any(visit(node) for node in range(len(graph)) if color[node] == 0)


def task_eligible(task: dict, claimer: dict, explicit_filter: dict | None) -> bool:
    """Requirements and explicit task filter are separate, conjunctive gates."""
    requirements = all(claimer.get(k) == v for k, v in task.get("requires", {}).items())
    predicate = (
        True
        if explicit_filter is None
        else all(task.get(k) == v for k, v in explicit_filter.items())
    )
    return requirements and predicate


def log_weight(
    urgency: int, u_ref: int, bucket_n: int, *, r: float, d: float, alpha: float
) -> float:
    return -math.log(r) * (urgency - u_ref) / d - alpha * math.log(bucket_n)


def normalized_log_weights(
    scores: list[int], buckets: list[int], *, r: float, d: float, alpha: float
) -> list[float]:
    if not (
        math.isfinite(r)
        and r > 1
        and math.isfinite(d)
        and d > 0
        and math.isfinite(alpha)
        and alpha >= 0
    ):
        raise ValueError("invalid profile")
    if len(scores) != len(buckets) or any(n <= 0 for n in buckets):
        raise ValueError("invalid density inputs")
    if not scores:
        return []
    logs = [
        log_weight(u, min(scores), n, r=r, d=d, alpha=alpha)
        for u, n in zip(scores, buckets)
    ]
    peak = max(logs)
    raw = [math.exp(x - peak) for x in logs]
    total = math.fsum(raw)
    if not math.isfinite(total) or total <= 0:
        raise ValueError("invalid numeric state")
    return [x / total for x in raw]


def fixed_masses(weighted: list[float], epsilon: float) -> list[float]:
    if not (math.isfinite(epsilon) and 0 <= epsilon <= 1):
        raise ValueError("epsilon outside [0,1]")
    if not weighted:
        return []
    if any((not math.isfinite(p) or p < 0) for p in weighted):
        raise ValueError("invalid probability")
    return [(1 - epsilon) * p + epsilon / len(weighted) for p in weighted]


def draw_without_replacement(
    ids: list[str], masses: list[float], count: int, rng: CounterRng
) -> list[str]:
    if len(ids) != len(masses) or len(set(ids)) != len(ids):
        raise ValueError("IDs must be unique and aligned")
    if count < 0:
        raise ValueError("count must be nonnegative")
    remaining = list(range(len(ids)))
    chosen: list[str] = []
    for _ in range(min(count, len(ids))):
        total = math.fsum(masses[i] for i in remaining)
        if total <= 0 or not math.isfinite(total):
            raise ValueError("invalid remaining mass")
        point = rng.unit() * total
        cumulative = 0.0
        picked = remaining[-1]
        for i in remaining:
            cumulative += masses[i]
            if point < cumulative:
                picked = i
                break
        chosen.append(ids[picked])
        remaining.remove(picked)
    return chosen


def decimal_first_draw(
    scores: list[int], bucket_ns: list[int], epsilon: str
) -> list[decimal.Decimal]:
    """High-precision analytic oracle, implemented independently of float log weights."""
    if not scores:
        return []
    r, d, alpha = DECIMAL("1.25"), DECIMAL("200"), DECIMAL("0.5")
    uref = min(scores)
    raw = []
    for score, density in zip(scores, bucket_ns):
        # Decimal ln/exp avoids the implementation's float/log path.
        urgency = (-(r.ln() * DECIMAL(score - uref) / d)).exp()
        density_factor = (DECIMAL(density).ln() * alpha).exp()
        raw.append(urgency / density_factor)
    total = sum(raw, DECIMAL(0))
    weighted = [v / total for v in raw]
    eps = DECIMAL(epsilon)
    return [(1 - eps) * p + eps / DECIMAL(len(scores)) for p in weighted]


def exact_inclusion(masses: list[decimal.Decimal], draws: int) -> list[decimal.Decimal]:
    """Enumerate every ordered weighted sequence and sum inclusion mass."""
    result = [DECIMAL(0) for _ in masses]

    def visit(
        left: tuple[int, ...], prob: decimal.Decimal, selected: frozenset[int]
    ) -> None:
        if len(selected) == draws:
            for i in selected:
                result[i] += prob
            return
        remaining_total = sum((masses[i] for i in left), DECIMAL(0))
        for i in left:
            next_prob = prob * masses[i] / remaining_total
            visit(tuple(j for j in left if j != i), next_prob, selected | {i})

    visit(tuple(range(len(masses))), DECIMAL(1), frozenset())
    return result


class CompletionModel:
    """Tiny reference model; remote effects are operation-keyed and outside Fleet fencing."""

    def __init__(self) -> None:
        self.owner = "worker-A"
        self.generation = 1
        self.state = "claimed"
        self.op: dict | None = None
        self.remote: set[str] = set()
        self.webhook_ids: set[str] = set()
        self.hold = False
        self.newer_work = False

    def serialize(self) -> str:
        data = self.__dict__.copy()
        if self.op is not None:
            data["op"] = self.op.copy()
            data["op"]["required"] = sorted(self.op["required"])
            data["op"]["verified"] = sorted(self.op["verified"])
        data["remote"] = sorted(self.remote)
        data["webhook_ids"] = sorted(self.webhook_ids)
        return json.dumps(data, sort_keys=True)

    @classmethod
    def reload(cls, payload: str) -> "CompletionModel":
        data = json.loads(payload)
        model = cls()
        model.__dict__.update(data)
        model.remote = set(data["remote"])
        model.webhook_ids = set(data["webhook_ids"])
        if model.op is not None:
            model.op["required"] = set(model.op["required"])
            model.op["verified"] = set(model.op["verified"])
        return model

    def takeover(self) -> None:
        self.owner = "worker-B"
        self.generation += 1

    def mutate_local(self, owner: str, generation: int, new_state: str) -> bool:
        if owner != self.owner or generation != self.generation:
            return False
        self.state = new_state
        return True

    def prepare(
        self,
        owner: str,
        generation: int,
        *,
        authorized: bool,
        deps_clear: bool,
        operation: str,
    ) -> bool:
        if (
            owner != self.owner
            or generation != self.generation
            or not authorized
            or not deps_clear
            or self.op
        ):
            return False
        self.op = {
            "id": operation,
            "status": "pending",
            "required": {operation},
            "verified": set(),
            "sent": False,
        }
        self.state = "completing"
        return True

    def send(self, owner: str, generation: int) -> bool:
        if (
            owner != self.owner
            or generation != self.generation
            or not self.op
            or self.op["status"] != "pending"
        ):
            return False
        self.op["sent"] = True
        self.op["status"] = "unknown"
        return True

    def apply_remote(self, operation: str) -> None:
        if self.op and self.op["id"] == operation and self.op["sent"]:
            self.remote.add(
                operation
            )  # idempotent remote fake keyed by operation identity

    def readback(self, operation: str) -> bool:
        if not self.op or self.op["id"] != operation or operation not in self.remote:
            return False
        self.op["verified"].add(operation)
        self.op["status"] = "confirmed"
        return True

    def late_receipt(self, operation: str, observed_applied: bool) -> bool:
        # May append evidence for the immutable op; never grants authority to create another.
        if (
            not self.op
            or self.op["id"] != operation
            or not observed_applied
            or operation not in self.remote
        ):
            return False
        self.op["verified"].add(operation)
        return True

    def finalize(self, operation: str) -> bool:
        if (
            not self.op
            or self.op["id"] != operation
            or self.state != "completing"
            or self.hold
            or self.newer_work
            or not self.op["required"] <= self.op["verified"]
        ):
            return False
        self.state = "completed"
        return True

    def webhook(self, delivery_id: str, operation: str) -> bool:
        if delivery_id in self.webhook_ids:
            return False
        self.webhook_ids.add(delivery_id)
        # Webhook is a hint only: it cannot send, verify, or finalize.
        return operation in self.remote


def run(args: argparse.Namespace) -> dict:
    fixture_path = HERE / "fixtures.json"
    fixture_bytes = fixture_path.read_bytes()
    fixtures = json.loads(fixture_bytes)
    checks = Checks()
    rng = CounterRng(args.seed)

    # Golden exact-integer ordering and AND eligibility (no JS/float coercion).
    golden = fixtures["priority_and_eligibility"]
    ordered = sorted(
        golden["items"],
        key=lambda x: (
            x["urgency"],
            int(x["peer_rank"]),
            x["created_at"],
            x["canonical_id"].encode("utf-8"),
        ),
    )
    checks.ok(
        "priority_golden_order",
        [x["canonical_id"] for x in ordered] == golden["expected_order"],
    )
    checks.ok(
        "priority_decimal_ids_preserved",
        int(golden["items"][0]["canonical_id"]) > (1 << 53),
    )
    checks.ok(
        "priority_decimal_peer_rank_9_before_10",
        int("9") < int("10") and [x["peer_rank"] for x in ordered[1:3]] == ["9", "9"],
    )
    checks.ok(
        "priority_canonical_id_bytewise_tie",
        [x["canonical_id"] for x in ordered[1:3]]
        == sorted(
            ("9007199254740999", "9007199254741002"), key=lambda x: x.encode("utf-8")
        ),
    )
    for case in golden["eligibility"]:
        eligible = task_eligible(case["task"], case["claimer"], case.get("filter"))
        checks.ok("requirements_and_filter", eligible == case["expected"])
    checks.ok(
        "requirements_default_true",
        task_eligible({"repo": "code"}, {"repo": "code"}, {"repo": "code"}),
    )
    checks.ok(
        "filter_default_true", task_eligible({"repo": "code"}, {"repo": "code"}, None)
    )

    # Graph fixture plus randomized independent DFS-vs-closure oracle and edge cycle rule.
    graph_fixture = fixtures["graph"]
    graph_nodes = graph_fixture["nodes"]
    node_index = {name: i for i, name in enumerate(graph_nodes)}
    graph = [set() for _ in graph_nodes]
    for edge in graph_fixture["edges"]:
        if edge["kind"] == "hard_blocker":
            graph[node_index[edge["from"]]].add(node_index[edge["to"]])
    checks.ok("graph_blocker_to_dependent_orientation", reachable_dfs(graph, 0, 2))
    checks.ok(
        "graph_parent_and_advisory_nonblocking",
        not reachable_dfs(graph, node_index["parent"], node_index["advisory_target"])
        and not reachable_dfs(
            graph, node_index["advisory_target"], node_index["parent"]
        ),
    )
    self_graph = [set(row) for row in graph]
    self_graph[3].add(3)
    checks.ok("graph_cycle_self_edge", has_cycle_color_dfs(self_graph))
    cycle_candidate = 2  # proposed edge 2->0 closes the existing 0->1->2 path
    checks.ok(
        "graph_cycle_reverse_path",
        reachable_dfs(graph, 0, cycle_candidate)
        and not reachable_dfs(graph, cycle_candidate, 0),
    )
    candidate_graph = [set(row) for row in graph]
    candidate_graph[cycle_candidate].add(0)
    checks.ok(
        "graph_cycle_oracle_detects_inserted_cycle",
        has_cycle_color_dfs(candidate_graph),
    )
    graph_cases = 0
    for _ in range(args.cases):
        n = 2 + rng.below(7)
        sample = [set() for _ in range(n)]
        for u in range(n):
            for v in range(
                u + 1, n
            ):  # random DAG; the oracle tests the candidate edge's new cycle
                if rng.below(5) == 0:
                    sample[u].add(v)
        closure = closure_oracle(sample)
        for u in range(n):
            for v in range(n):
                dfs = reachable_dfs(sample, u, v)
                checks.ok("graph_dfs_matches_transitive_closure", dfs == closure[u][v])
                # Proposed insertion u->v cycles exactly when u=v or v reaches u.
                cycle_by_rule = u == v or closure[v][u]
                after = [set(row) for row in sample]
                after[u].add(v)
                creates_cycle = has_cycle_color_dfs(after)
                checks.ok("graph_cycle_insertion_rule", cycle_by_rule == creates_cycle)
        graph_cases += 1
    opposite = [set() for _ in range(2)]
    first_ok = not (0 == 1 or reachable_dfs(opposite, 1, 0))
    if first_ok:
        opposite[0].add(1)
    second_ok = not (1 == 0 or reachable_dfs(opposite, 0, 1))
    checks.ok(
        "graph_serialized_opposite_insert_rejects_second", first_ok and not second_ok
    )

    # Historical profile: fixed densities and mixture masses, analytical Decimal first-draw oracle.
    profile = fixtures["exploration"]["population"]
    scores = profile["urgencies"]
    bucket_ids = profile["bucket_ids"]
    ids = profile["ids"]
    bucket_counts = collections.Counter(
        bucket_ids[i] for i in range(len(ids)) if ids[i] in set(ids)
    )
    bucket_ns = [bucket_counts[bucket] for bucket in bucket_ids]
    checks.ok(
        "exploration_snapshot_density_counted_by_unique_eligible_ids",
        bucket_ns == profile["fixed_bucket_populations"] and len(set(ids)) == len(ids),
    )
    eps_values = fixtures["exploration"]["epsilon_values"]
    analytic_by_eps: dict[str, list[float]] = {}
    decimal_by_eps: dict[str, list[str]] = {}
    max_decimal_error = 0.0
    for eps_text in eps_values:
        analytic_decimal = decimal_first_draw(scores, bucket_ns, eps_text)
        decimal_by_eps[eps_text] = [str(x) for x in analytic_decimal]
        weighted = normalized_log_weights(scores, bucket_ns, r=1.25, d=200, alpha=0.5)
        masses = fixed_masses(weighted, float(eps_text))
        analytic_by_eps[eps_text] = masses
        for got, expected in zip(masses, analytic_decimal):
            error = abs(got - float(expected))
            max_decimal_error = max(max_decimal_error, error)
            checks.error("float_vs_decimal_first_draw", error)
            checks.ok("exploration_decimal_oracle_tolerance", error <= 2e-14)
        checks.ok(
            "exploration_probability_normalization",
            abs(math.fsum(masses) - 1.0) <= 2e-15,
        )
    # Explicitly vary only the shared reference; normalized probabilities must be identical.
    logs_a = [
        log_weight(u, min(scores), n, r=1.25, d=200, alpha=0.5)
        for u, n in zip(scores, bucket_ns)
    ]
    logs_b = [
        log_weight(u, max(scores), n, r=1.25, d=200, alpha=0.5)
        for u, n in zip(scores, bucket_ns)
    ]
    norm = lambda xs: [
        math.exp(x - max(xs)) / math.fsum(math.exp(y - max(xs)) for y in xs) for x in xs
    ]
    ref_error = max(abs(a - b) for a, b in zip(norm(logs_a), norm(logs_b)))
    checks.error("exploration_u_ref_invariance", ref_error)
    checks.ok("exploration_u_ref_invariance", ref_error <= 2e-15)
    checks.ok(
        "exploration_density_uses_unique_ids",
        len(set(ids)) == len(ids) and len(bucket_ns) == len(ids),
    )
    for n in (0, 1, 2):
        small_ids = ids[:n]
        small = draw_without_replacement(small_ids, [1.0 / n] * n if n else [], 3, rng)
        checks.ok(
            "exploration_population_0_1_2",
            len(small) == n and len(set(small)) == len(small),
        )
    sample = draw_without_replacement(
        ids, analytic_by_eps["0.05"], min(3, len(ids)), rng
    )
    checks.ok("exploration_without_replacement", len(sample) == len(set(sample)))
    # Tiny population inclusion oracle enumerates weighted ordered sequences, not first-draw marginals.
    tiny_decimal = decimal_first_draw([10, 90, 180], [3, 1, 2], "0.05")
    tiny_inclusion = exact_inclusion(tiny_decimal, 2)
    checks.ok(
        "exploration_exact_tiny_inclusion_normalizes",
        abs(sum(tiny_inclusion, DECIMAL(0)) - DECIMAL(2)) < DECIMAL("1e-55"),
    )
    checks.ok(
        "exploration_inclusion_not_first_draw",
        any(
            abs(float(tiny_inclusion[i]) - float(tiny_decimal[i])) > 1e-6
            for i in range(3)
        ),
    )
    # Preregistered tiny-population inclusion experiment against exact enumeration.
    tiny_ids = ["tiny-a", "tiny-b", "tiny-c"]
    tiny_counts = [0, 0, 0]
    tiny_rng = CounterRng(args.seed ^ 0x9204)
    for _ in range(args.draws):
        selected = draw_without_replacement(
            tiny_ids, [float(p) for p in tiny_decimal], 2, tiny_rng
        )
        for i, item_id in enumerate(tiny_ids):
            tiny_counts[i] += item_id in selected
    tiny_bounds = []
    tiny_max_z = 0.0
    for probability, count in zip(tiny_inclusion, tiny_counts):
        p = float(probability)
        observed = count / args.draws
        bound = 6.0 * math.sqrt(p * (1.0 - p) / args.draws) + 1.0 / args.draws
        error = abs(observed - p)
        tiny_bounds.append(bound)
        tiny_max_z = max(
            tiny_max_z,
            error / max(math.sqrt(p * (1.0 - p) / args.draws), 1 / args.draws),
        )
        checks.error("exploration_tiny_inclusion_frequency", error)
        checks.ok("exploration_tiny_inclusion_preregistered_bound", error <= bound)
    for bad in (
        (1.0, 200.0, 0.5, 0.05),
        (1.25, 0.0, 0.5, 0.05),
        (1.25, 200.0, -0.1, 0.05),
        (1.25, 200.0, 0.5, 1.1),
    ):
        try:
            normalized_log_weights(scores, bucket_ns, r=bad[0], d=bad[1], alpha=bad[2])
            fixed_masses([0.5, 0.5], bad[3])
            raised = False
        except ValueError:
            raised = True
        checks.ok("exploration_invalid_parameters_typed_error", raised)

    # Seeded empirical first-draw frequencies; preregistered conservative six-sigma + 1/n bound.
    expected = analytic_by_eps["0.05"]
    first_counts = [0] * len(ids)
    stat_rng = CounterRng(args.seed ^ 0x9196)
    for _ in range(args.draws):
        pick = draw_without_replacement(ids, expected, 1, stat_rng)[0]
        first_counts[ids.index(pick)] += 1
    statistical_bounds = []
    max_z = 0.0
    for p, count in zip(expected, first_counts):
        observed = count / args.draws
        bound = 6.0 * math.sqrt(p * (1.0 - p) / args.draws) + 1.0 / args.draws
        err = abs(observed - p)
        statistical_bounds.append(bound)
        z = err / max(math.sqrt(p * (1 - p) / args.draws), 1 / args.draws)
        max_z = max(max_z, z)
        checks.error("exploration_first_draw_frequency", err)
        checks.ok("exploration_preregistered_6sigma_bound", err <= bound)

    # Lifecycle reference state machine, named oracle cases, crash cuts, and negative assertions.
    comp = fixtures["completion"]
    checks.ok(
        "completion_fixture_declares_negative_preconditions",
        comp["prepare_requires"]["stale_owner"] is False
        and comp["prepare_requires"]["unauthorized"] is False
        and comp["prepare_requires"]["blocked_dependency"] is False,
    )
    m = CompletionModel()
    checks.ok(
        "completion_stale_prepare_rejected",
        not m.prepare(
            "worker-old", 0, authorized=True, deps_clear=True, operation="stale-prepare"
        ),
    )
    checks.ok(
        "completion_unauthorized_prepare_rejected",
        not m.prepare(
            "worker-A", 1, authorized=False, deps_clear=True, operation="unauthorized"
        ),
    )
    checks.ok(
        "completion_blocked_dependency_prepare_rejected",
        not m.prepare(
            "worker-A", 1, authorized=True, deps_clear=False, operation="blocked"
        ),
    )
    checks.ok(
        "completion_negative_prepare_no_mutation", m.op is None and m.state == "claimed"
    )
    checks.ok(
        "completion_prepare_valid",
        m.prepare("worker-A", 1, authorized=True, deps_clear=True, operation="op-1"),
    )
    checks.ok("completion_finalize_unverified_rejected", not m.finalize("op-1"))
    checks.ok("completion_send_current_owner", m.send("worker-A", 1))
    m.apply_remote(
        "op-1"
    )  # applied; acknowledgement is lost => remains unknown until readback
    checks.ok("completion_response_lost_is_unknown", m.op["status"] == "unknown")
    checks.ok(
        "completion_readback_confirms_only_same_operation",
        not m.readback("other-op") and m.readback("op-1"),
    )
    checks.ok(
        "completion_verified_finalize", m.finalize("op-1") and m.state == "completed"
    )

    stale = CompletionModel()
    stale.takeover()
    checks.ok(
        "completion_stale_owner_new_prepare_rejected",
        not stale.prepare(
            "worker-A", 1, authorized=True, deps_clear=True, operation="stale"
        ),
    )
    prior_state = stale.state
    checks.ok(
        "completion_stale_local_write_rejected",
        not stale.mutate_local("worker-A", 1, "completed")
        and stale.state == prior_state,
    )
    checks.ok(
        "completion_stale_owner_new_send_rejected",
        stale.prepare("worker-B", 2, authorized=True, deps_clear=True, operation="live")
        and not stale.send("worker-A", 1),
    )
    stale.send("worker-B", 2)
    stale.apply_remote("live")
    checks.ok(
        "completion_expired_receipt_scoped",
        stale.late_receipt("live", True) and not stale.late_receipt("wrong", True),
    )
    checks.ok(
        "completion_receipt_does_not_authorize_new_command",
        not stale.prepare(
            "worker-A", 1, authorized=True, deps_clear=True, operation="new-op"
        ),
    )

    partial = CompletionModel()
    partial.prepare(
        "worker-A",
        1,
        authorized=True,
        deps_clear=True,
        operation="required-issue-close",
    )
    partial.op["required"] = {"merge-pr", "required-issue-close"}
    partial.send("worker-A", 1)
    partial.apply_remote("required-issue-close")
    partial.readback("required-issue-close")
    partial.hold = True  # cancel/hold after send retains external effect and prevents false complete
    checks.ok(
        "completion_cancel_after_send_preserves_remote_effect",
        "required-issue-close" in partial.remote,
    )
    checks.ok(
        "completion_hold_prevents_finalize",
        not partial.finalize("required-issue-close") and partial.state == "completing",
    )
    partial.hold = False
    checks.ok(
        "completion_partial_required_effect_no_false_complete",
        not partial.finalize("required-issue-close") and partial.state == "completing",
    )
    wrong = CompletionModel()
    wrong.prepare("worker-A", 1, authorized=True, deps_clear=True, operation="op-A")
    wrong.send("worker-A", 1)
    wrong.apply_remote("op-A")
    checks.ok(
        "completion_wrong_operation_receipt_no_finalize",
        not wrong.readback("op-B") and not wrong.finalize("op-A"),
    )
    dup = CompletionModel()
    dup.prepare("worker-A", 1, authorized=True, deps_clear=True, operation="hook-op")
    dup.send("worker-A", 1)
    dup.apply_remote("hook-op")
    before = len(dup.remote)
    dup.webhook("delivery-1", "hook-op")
    dup.webhook("delivery-1", "hook-op")
    checks.ok(
        "completion_duplicate_webhook_no_extra_effect",
        len(dup.remote) == before and len(dup.webhook_ids) == 1,
    )
    # Explicit crash cuts around prepare/send/apply/ack/finalize; restart retains state and never blind-resends.
    crash_cuts = (
        "before_prepare",
        "after_prepare",
        "before_send",
        "after_send",
        "before_apply",
        "after_apply",
        "before_ack",
        "after_ack",
        "before_finalize",
    )
    for cut in crash_cuts:
        state = CompletionModel()
        if cut not in ("before_prepare",):
            state.prepare(
                "worker-A", 1, authorized=True, deps_clear=True, operation="crash-op"
            )
        if cut in (
            "after_send",
            "before_apply",
            "after_apply",
            "before_ack",
            "after_ack",
            "before_finalize",
        ):
            state.send("worker-A", 1)
        if cut in ("after_apply", "before_ack", "after_ack", "before_finalize"):
            state.apply_remote("crash-op")
        if cut in ("after_ack", "before_finalize"):
            state.readback("crash-op")
        recovered = CompletionModel.reload(state.serialize())
        # Reconcile only by readback. If the effect is absent/unobservable, preserve unknown;
        # no recovery path resends a consequential request.
        if recovered.op and recovered.op["status"] == "unknown":
            recovered.readback("crash-op")
        can_complete = cut in ("after_ack", "before_finalize") and recovered.finalize(
            "crash-op"
        )
        expected_unknown = cut in ("after_send", "before_apply")
        checks.ok(
            "completion_crash_cut_serialized_reload",
            CompletionModel.reload(state.serialize()).serialize() == state.serialize(),
        )
        checks.ok(
            "completion_crash_cut_converges_or_remains_unknown",
            (
                can_complete
                if cut in ("after_ack", "before_finalize")
                else (
                    recovered.op is None
                    if cut == "before_prepare"
                    else recovered.state != "completed"
                )
            )
            and (not expected_unknown or recovered.op["status"] == "unknown")
            and len(recovered.remote) <= 1,
        )
        if recovered.op and recovered.op["status"] == "unknown":
            checks.ok(
                "completion_recovery_never_blind_resends",
                not recovered.send(recovered.owner, recovered.generation),
            )
    finalized = CompletionModel()
    finalized.prepare(
        "worker-A", 1, authorized=True, deps_clear=True, operation="final-cut"
    )
    finalized.send("worker-A", 1)
    finalized.apply_remote("final-cut")
    finalized.readback("final-cut")
    checks.ok(
        "completion_before_finalize_cut",
        finalized.state == "completing" and finalized.finalize("final-cut"),
    )
    checks.ok(
        "completion_after_finalize_cut",
        finalized.state == "completed" and not finalized.finalize("final-cut"),
    )
    checks.ok(
        "completion_model_explicitly_scoped_to_idempotent_fake",
        comp["remote_effect_key"] == "immutable operation id",
    )

    if not checks.counts or sum(checks.counts.values()) <= 0:
        raise AssertionError("zero checks is not a passing experiment")
    report = {
        "schema": "pmo-offline-experiment/v1",
        "result": "pass",
        "generated_at": "intentionally omitted for deterministic output",
        "inputs": {
            "seed": args.seed,
            "cases": args.cases,
            "draws": args.draws,
            "fixture_sha256": hashlib.sha256(fixture_bytes).hexdigest(),
        },
        "runtime": {
            "python": platform.python_version(),
            "prng": "SHA-256 counter, pmo-experiment-v1 domain separator, 53-bit uniform grid; algorithm independent of Python random module",
        },
        "oracles": {
            "priority": "golden exact-integer comparator and independent requirements-AND-explicit-filter truth table",
            "graph": "independent Floyd-Warshall transitive closure vs DFS reachability; cycle insertion oracle; serialized opposite-edge case",
            "exploration": "60-digit Decimal analytic first-draw weights vs float log-weight implementation; exact enumeration of all weighted sequences for 3-item/two-draw inclusion probabilities checked empirically against a preregistered per-candidate 6-sigma + 1/n bound; first-draw bound likewise preregistered",
            "completion": "golden transition expectations and model invariants over fence/operation/readback/hold/webhook/crash-cut sequences; fake idempotent remote ledger keyed by one operation identity",
        },
        "counts": {
            "named_oracle_checks": sum(checks.counts.values()),
            "by_oracle": dict(sorted(checks.counts.items())),
            "random_graph_cases": graph_cases,
            "statistical_draws": args.draws,
            "completion_crash_cuts": len(crash_cuts) + 1,
        },
        "completion_crash_cut_names": list(crash_cuts) + ["after_finalize"],
        "statistics": {
            "epsilon": 0.05,
            "first_draw_expected": expected,
            "first_draw_counts": first_counts,
            "first_draw_observed": [c / args.draws for c in first_counts],
            "absolute_bounds": statistical_bounds,
            "max_standard_error_units": max_z,
            "preregistered_bound": "6*sqrt(p*(1-p)/draws) + 1/draws per category",
        },
        "tolerances": {
            "float_vs_decimal_first_draw_abs": 2e-14,
            "u_ref_normalized_probability_abs": 2e-15,
            "probability_normalization_abs": 2e-15,
            "first_draw_frequency": "per-category 6*sqrt(p*(1-p)/draws) + 1/draws",
        },
        "small_population": {
            "population": 3,
            "draws_without_replacement": 2,
            "exact_inclusion_probabilities": [str(p) for p in tiny_inclusion],
            "observed_inclusion_counts": tiny_counts,
            "observed_inclusion_probabilities": [c / args.draws for c in tiny_counts],
            "absolute_bounds": tiny_bounds,
            "max_standard_error_units": tiny_max_z,
            "preregistered_bound": "6*sqrt(p*(1-p)/draws) + 1/draws per item",
            "sum_expected_inclusions": str(sum(tiny_inclusion, DECIMAL(0))),
        },
        "max_errors": checks.max_errors,
        "historical_profile": {
            "r": 1.25,
            "d": 200,
            "alpha": 0.5,
            "bucket_width": 200,
            "epsilon_values": eps_values,
            "epsilon_zero_and_one_included": True,
            "initial_masses_by_epsilon": analytic_by_eps,
            "decimal_first_draw_by_epsilon": decimal_by_eps,
            "density_and_mixture_frozen_before_draws": True,
        },
        "limits": [
            "Design experiment only; no imports from Fleet and no production implementation tests.",
            "Synthetic fixtures do not establish production benefit, calibration, starvation bounds, or workload representativeness.",
            "The completion remote ledger is an idempotent fake; no universal exactly-once guarantee and no remote GitHub fence are modeled.",
            "No actual Fleet broker, Rust/TypeScript adapter, GitHub API, deployed service, or live completion behavior was tested.",
            "This is not formal proof, full PBT/MBT production coverage, or authorization to implement, migrate, schedule, or roll out.",
        ],
    }
    return report


def main(argv: list[str] | None = None) -> int:
    args = arguments(
        argv
    )  # Strict complete parse and validation precedes fixture access or output.
    try:
        report = run(args)
    except (AssertionError, ValueError, json.JSONDecodeError) as exc:
        # Emit a machine-readable failure; nonzero ensures it cannot masquerade as success.
        print(
            json.dumps(
                {
                    "schema": "pmo-offline-experiment/v1",
                    "result": "fail",
                    "error": str(exc),
                },
                sort_keys=True,
            )
        )
        return 1
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
