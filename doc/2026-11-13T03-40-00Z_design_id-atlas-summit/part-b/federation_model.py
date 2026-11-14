#!/usr/bin/env python3
"""Small executable oracle model for Part B budgets/dedup (NOT cryptography).

Run: python3 -I doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-b/federation_model.py
The scoped vector model assumes authenticated scope IDs and local metering;
it proves neither fairness nor Byzantine/global budget enforcement.
CLI contract: no arguments; reject all extra arguments before running checks.
"""

from __future__ import annotations

from dataclasses import dataclass, field
import hashlib
import json
import random
import sys


@dataclass
class Ledger:
    request_id: str
    binding: tuple[str, str, int, str]
    expires: int
    total: int
    spent: int = 0
    reservations: dict[str, int] = field(default_factory=dict)
    cache: dict[str, str] = field(default_factory=dict)
    revoked: set[str] = field(default_factory=set)
    completed: set[str] = field(default_factory=set)

    def reserve(self, child: str, amount: int) -> int:
        if child in self.reservations:
            return self.reservations[child]  # retry reuses; cannot mint budget
        remaining = self.total - self.spent - sum(self.reservations.values())
        if amount < 0 or amount > remaining:
            raise ValueError("reservation exceeds remaining parent grant")
        self.reservations[child] = amount
        return amount

    def consume(self, amount: int) -> None:
        if amount < 0 or amount > self.total - self.spent - sum(
            self.reservations.values()
        ):
            raise ValueError("work exceeds unreserved local budget")
        self.spent += amount

    def bind_retry(self, binding: tuple[str, str, int, str], now: int) -> bool:
        if binding != self.binding:
            raise ValueError("request-id binding conflict")
        return now < self.expires

    def release_cached(self, peer: str, now: int) -> str:
        if now >= self.expires:
            return "expired-no-rerun"
        if peer in self.revoked:
            return "reauthorization-denied"
        if peer in self.completed and peer not in self.cache:
            return "result-evicted-no-rerun"
        return self.cache.get(peer, "miss")


@dataclass
class ScopedLedger:
    """Accounting abstraction only; scope IDs are assumed authenticated."""

    limits: dict[tuple[str, str], int]
    held: dict[tuple[str, str], int] = field(default_factory=dict)
    spent: dict[tuple[str, str], int] = field(default_factory=dict)
    reservations: dict[str, tuple[dict[str, int], tuple[str, ...], int]] = field(
        default_factory=dict
    )
    finished: dict[str, tuple[dict[str, int], tuple[str, ...], int, dict[str, int]]] = (
        field(default_factory=dict)
    )

    def reserve(
        self,
        reservation_id: str,
        vector: dict[str, int],
        scopes: tuple[str, ...],
        expires: int,
        now: int,
    ) -> dict[str, int]:
        if len(set(scopes)) != len(scopes):
            raise ValueError("duplicate accounting scope")
        done = self.finished.get(reservation_id)
        if done is not None:
            old_vector, old_scopes, old_expiry, _ = done
            if (old_vector, old_scopes, old_expiry) != (vector, scopes, expires):
                raise ValueError("completed reservation binding conflict")
            raise ValueError("completed reservation tombstone cannot be reopened")
        old = self.reservations.get(reservation_id)
        if old is not None:
            old_vector, old_scopes, old_expiry = old
            if (old_vector, old_scopes, old_expiry) != (vector, scopes, expires):
                raise ValueError("retry reservation binding conflict")
            if now >= old_expiry:
                raise ValueError("expired reservation cannot be retried")
            return dict(old_vector)
        if now >= expires:
            raise ValueError("origin request expired")
        for scope in scopes:
            for dim, amount in vector.items():
                key = (scope, dim)
                limit = self.limits[key]
                used = self.held.get(key, 0) + self.spent.get(key, 0)
                if amount < 0 or used + amount > limit:
                    raise ValueError("scope budget exceeded")
        for scope in scopes:
            for dim, amount in vector.items():
                key = (scope, dim)
                self.held[key] = self.held.get(key, 0) + amount
        self.reservations[reservation_id] = (dict(vector), scopes, expires)
        self._assert_caps()
        return dict(vector)

    def finish(self, reservation_id: str, used: dict[str, int], now: int) -> None:
        done = self.finished.get(reservation_id)
        if done is not None:
            if done[3] != used:
                raise ValueError("completed accounting conflict")
            return  # do not refund/charge the same reservation twice
        vector, scopes, expires = self.reservations[reservation_id]
        # Caller supplies locally measured actual use. Expiry closes the
        # reservation; it refunds only vector minus measured use, never spent work.
        if set(used) - set(vector) or any(
            used.get(dim, 0) < 0 or used.get(dim, 0) > amount
            for dim, amount in vector.items()
        ):
            raise ValueError("actual use exceeds reserved vector")
        spent_before = dict(self.spent)
        for scope in scopes:
            for dim, amount in vector.items():
                key = (scope, dim)
                self.held[key] -= amount
                self.spent[key] = self.spent.get(key, 0) + used.get(dim, 0)
        self.finished[reservation_id] = (dict(vector), scopes, expires, dict(used))
        del self.reservations[reservation_id]
        assert all(
            self.spent.get(key, 0) >= amount for key, amount in spent_before.items()
        )
        self._assert_caps()

    def _assert_caps(self) -> None:
        for key, cap in self.limits.items():
            assert self.held.get(key, 0) >= 0 and self.spent.get(key, 0) >= 0
            assert self.held.get(key, 0) + self.spent.get(key, 0) <= cap


def origin_request_id(
    origin_key: str,
    nonce: str,
    query: str,
    scope: str,
    issued: int,
    expires: int,
    limits: tuple[int, ...],
) -> str:
    """Deterministic model ID. Production canonical CBOR/hash profile is separate."""
    body = json.dumps(
        [origin_key, nonce, query, scope, issued, expires, limits],
        separators=(",", ":"),
    ).encode()
    return hashlib.sha256(b"ID/Atlas/OriginQuery/v1\0" + body).hexdigest()


def check_fixed_oracles() -> None:
    ledger = Ledger("r1", ("digest", "scope-A", 10, "nonce-a"), 20, 10)
    assert ledger.reserve("child-a", 6) == 6
    assert ledger.reserve("child-a", 6) == 6  # retry reuses allocation
    assert (
        ledger.reserve("child-a", 99) == 6
    )  # even a changed retry request cannot mint
    assert origin_request_id(
        "origin", "nonce-1", "q", "s", 1, 20, (3, 5)
    ) != origin_request_id("origin", "nonce-2", "q", "s", 1, 20, (3, 5))
    assert ledger.reserve("child-b", 4) == 4
    try:
        ledger.reserve("child-c", 1)
    except ValueError:
        pass
    else:
        raise AssertionError("copied budget was accepted")
    try:
        ledger.bind_retry(("other-digest", "scope-A", 10, "nonce-a"), 11)
    except ValueError:
        pass
    else:
        raise AssertionError("changed query under same ID was accepted")
    ledger.cache["peer"] = "result"
    ledger.completed.add("peer")
    assert ledger.release_cached("peer", 11) == "result"
    del ledger.cache["peer"]
    assert ledger.release_cached("peer", 11) == "result-evicted-no-rerun"
    ledger.cache["peer"] = "result"
    ledger.revoked.add("peer")
    assert ledger.release_cached("peer", 11) == "reauthorization-denied"
    assert ledger.release_cached("peer", 20) == "expired-no-rerun"

    scopes = (
        "query:q1",
        "client:client-key",
        "peer:peer-key",
        "tenant:tenant-7",
        "node:node-1",
    )
    limits = {
        (scope, dim): cap
        for scope in scopes
        for dim, cap in {"peers": 5, "work": 12, "bytes": 1024}.items()
    }
    scoped = ScopedLedger(limits)
    vector = {"peers": 2, "work": 4, "bytes": 300}
    assert scoped.reserve("reservation-1", vector, scopes, expires=30, now=10) == vector
    assert (
        scoped.reserve("reservation-1", vector, scopes, expires=30, now=11) == vector
    )  # retry idempotent
    try:
        scoped.reserve(
            "reservation-1",
            {"peers": 3, "work": 4, "bytes": 300},
            scopes,
            expires=30,
            now=11,
        )
    except ValueError:
        pass
    else:
        raise AssertionError("mismatched retry minted a changed reservation")
    held_before = dict(scoped.held)
    spent_before = dict(scoped.spent)
    try:
        scoped.reserve(
            "duplicate-scope", {"work": 1}, ("node:node-1", "node:node-1"), 30, 11
        )
    except ValueError:
        pass
    else:
        raise AssertionError("duplicate accounting scopes double-counted one bucket")
    assert scoped.held == held_before and scoped.spent == spent_before
    try:
        scoped.finish("reservation-1", {"work": 1, "bytes": 1_000_000}, now=11)
    except ValueError:
        pass
    else:
        raise AssertionError("usage outside reserved vector was accepted")
    assert scoped.held == held_before and scoped.spent == spent_before
    scoped.finish("reservation-1", {"peers": 1, "work": 2, "bytes": 100}, now=12)
    assert scoped.spent[("node:node-1", "work")] == 2
    assert scoped.held[("node:node-1", "work")] == 0  # unused 2 work units refunded
    try:
        scoped.reserve("reservation-1", vector, scopes, expires=30, now=13)
    except ValueError:
        pass
    else:
        raise AssertionError("completed reservation tombstone was reopened")
    scoped.finish("reservation-1", {"peers": 1, "work": 2, "bytes": 100}, now=13)
    assert scoped.spent[("node:node-1", "work")] == 2
    assert scoped.held[("node:node-1", "work")] == 0
    scoped.reserve("reservation-2", vector, scopes, expires=20, now=12)
    scoped.finish("reservation-2", {"peers": 0, "work": 1, "bytes": 0}, now=20)
    assert (
        scoped.spent[("node:node-1", "work")] == 3
    )  # expiry refunded only unused amount

    scopes_a = (
        "query:q1",
        "query:q2",
        "client:c1",
        "client:c2",
        "peer:p1",
        "peer:p2",
        "tenant:t1",
        "tenant:t2",
        "node:n",
    )
    limits_a = {(scope, "work"): 10 for scope in scopes_a}
    limits_a[("client:c2", "work")] = 5
    isolated = ScopedLedger(limits_a)
    isolated.reserve(
        "c1-r",
        {"work": 4},
        ("query:q1", "client:c1", "peer:p1", "tenant:t1", "node:n"),
        30,
        1,
    )
    isolated.reserve(
        "c2-r",
        {"work": 4},
        ("query:q2", "client:c2", "peer:p2", "tenant:t2", "node:n"),
        30,
        1,
    )
    try:
        isolated.reserve(
            "c2-extra",
            {"work": 2},
            ("query:q2", "client:c2", "peer:p2", "tenant:t2", "node:n"),
            30,
            1,
        )
    except ValueError:
        pass
    else:
        raise AssertionError("authenticated requester cap was exceeded")

    scopes_b = (
        "query:q1",
        "query:q2",
        "client:c1",
        "client:c2",
        "peer:p1",
        "peer:p2",
        "tenant:t1",
        "tenant:t2",
        "node:n",
    )
    limits_b = {(scope, "work"): 20 for scope in scopes_b}
    limits_b[("node:n", "work")] = 10
    node_caps = ScopedLedger(limits_b)
    node_caps.reserve(
        "first",
        {"work": 6},
        ("query:q1", "client:c1", "peer:p1", "tenant:t1", "node:n"),
        30,
        1,
    )
    try:
        node_caps.reserve(
            "second",
            {"work": 6},
            ("query:q2", "client:c2", "peer:p2", "tenant:t2", "node:n"),
            30,
            1,
        )
    except ValueError:
        pass
    else:
        raise AssertionError("node aggregate cap was exceeded")


def check_seeded_graphs() -> None:
    rng = random.Random(15354)
    # Edges/grants are abstract authenticated links; this tests accounting, not crypto.
    for _ in range(2_000):
        total = rng.randrange(0, 100)
        root = Ledger("r", ("d", "s", 3, f"nonce-{_}"), 100, total)
        queue = [("root", root, 0)]
        ledgers = {"root": root}
        edges = 0
        while queue:
            node, ledger, depth = queue.pop(0)
            if ledger.total - ledger.spent - sum(ledger.reservations.values()) > 0:
                ledger.consume(
                    rng.randrange(
                        ledger.total
                        - ledger.spent
                        - sum(ledger.reservations.values())
                        + 1
                    )
                )
            if depth >= 5:
                continue
            count = rng.randrange(0, 4)
            for i in range(count):
                child = f"{node}/{i}"
                remaining = (
                    ledger.total - ledger.spent - sum(ledger.reservations.values())
                )
                grant = rng.randrange(remaining + 1) if remaining else 0
                ledger.reserve(child, grant)
                edges += 1
                child_ledger = Ledger(child, ("d", "s", 3, f"nonce-{_}"), 100, grant)
                ledgers[child] = child_ledger
                queue.append((child, child_ledger, depth + 1))
                if rng.random() < 0.1:
                    assert (
                        ledger.reserve(child, rng.randrange(total + 1)) == grant
                    )  # duplicate delivery/retry
        assert sum(item.spent for item in ledgers.values()) <= total
        assert edges == sum(len(item.reservations) for item in ledgers.values())


if __name__ == "__main__":
    if len(sys.argv) != 1:
        print("usage: federation_model.py", file=sys.stderr)
        raise SystemExit(2)
    check_fixed_oracles()
    check_seeded_graphs()
    print("federation model: fixed and 2000 seeded accounting traces passed")
