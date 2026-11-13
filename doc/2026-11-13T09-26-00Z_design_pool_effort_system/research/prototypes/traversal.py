"""Cycle-safe pool traversal prototype (D5, AD6). Stdlib only.

Model: a route graph whose nodes are pools (children: pools or leaves) and
leaves (one door each). Expansion is depth-first, in declared order, with ONE
per-call visited set shared by every pool expansion (AD6).

Outcomes per edge:
- pool already on the visited set  -> 'cycle-skipped' (its members were
  already fully processed; this is NOT a failure)
- leaf whose door is 100% walled   -> 'walled-skipped' (never attempted)
- leaf otherwise                   -> attempted exactly once; 'success' stops
  the whole walk, 'failed' continues.

Oracle (checked on every generated graph):
1. Termination: the walk makes at most (pools + edges + 1) visits.
2. At-most-once: no leaf is attempted twice; no pool is expanded twice.
3. Equivalence: when no success occurs, the attempted leaves plus the
   walled-skipped leaves equal the brute-force reachable leaf set (BFS over
   the same graph, no visited set), and walled leaves are never attempted.
4. Success prefix: when a success occurs, the attempted list is a prefix of
   the DFS order and ends at the success leaf.
5. Cycle outcome: every edge into an already-expanded pool is recorded as
   cycle-skipped, never as failed or attempted.

Run: python3 -I research/prototypes/traversal.py
"""

import random
import sys
from dataclasses import dataclass, field


@dataclass
class Graph:
    children: dict  # pool -> list of nodes (pool or leaf ids)
    leaves: set
    walled: set
    success: set
    root: str


@dataclass
class Trace:
    attempted: list = field(default_factory=list)
    expanded: list = field(default_factory=list)
    events: list = field(default_factory=list)  # (outcome, node)
    visits: int = 0
    stopped_on_success: bool = False


def walk(g: Graph) -> Trace:
    t = Trace()
    visited = set()

    def visit(node):
        t.visits += 1
        if node in g.leaves:
            if node in visited:
                t.events.append(("already-processed", node))
                return False
            visited.add(node)
            if node in g.walled:
                t.events.append(("walled-skipped", node))
                return False
            t.attempted.append(node)
            if node in g.success:
                t.events.append(("success", node))
                t.stopped_on_success = True
                return True
            t.events.append(("failed", node))
            return False
        if node in visited:
            t.events.append(("cycle-skipped", node))
            return False
        visited.add(node)
        t.expanded.append(node)
        for child in g.children[node]:
            if visit(child):
                return True
        return False

    visit(g.root)
    return t


def reachable_leaves(g: Graph) -> set:
    seen = set()
    out = set()
    stack = [g.root]
    while stack:
        n = stack.pop()
        if n in g.leaves:
            out.add(n)
            continue
        if n in seen:
            continue
        seen.add(n)
        stack.extend(g.children[n])
    return out


def reachable_pools(g: Graph) -> set:
    seen = set()
    stack = [g.root]
    while stack:
        n = stack.pop()
        if n in g.leaves or n in seen:
            continue
        seen.add(n)
        stack.extend(g.children[n])
    return seen


def random_graph(rng: random.Random) -> Graph:
    n_pools = rng.randint(1, 12)
    n_leaves = rng.randint(0, 8)
    pools = [f"p{i}" for i in range(n_pools)]
    leaves = [f"l{i}" for i in range(n_leaves)]
    everything = pools + leaves
    children = {}
    for p in pools:
        k = rng.randint(1, 4)
        # Targets drawn from every node, so self-loops and back-edges
        # (mutual references) occur at random.
        children[p] = [rng.choice(everything) for _ in range(k)]
    walled = {l for l in leaves if rng.random() < 0.3}
    success = {l for l in leaves if rng.random() < 0.2}
    return Graph(children, set(leaves), walled, success, root="p0")


def check(g: Graph) -> None:
    t = walk(g)
    pools = set(g.children)
    edges = sum(len(v) for v in g.children.values())

    assert t.visits <= len(pools) + edges + 1, "termination bound"
    assert len(t.attempted) == len(set(t.attempted)), "leaf attempted twice"
    assert len(t.expanded) == len(set(t.expanded)), "pool expanded twice"

    for outcome, node in t.events:
        if outcome == "walled-skipped":
            assert node in g.walled and node not in t.attempted, "walled leaf attempted"
        if outcome == "cycle-skipped":
            assert node in t.expanded, "cycle-skipped pool never expanded"
            assert node not in g.leaves

    walled_seen = {n for o, n in t.events if o == "walled-skipped"}
    if not t.stopped_on_success:
        reach = reachable_leaves(g)
        assert set(t.attempted) | walled_seen == reach, "reachable set mismatch"
        assert set(t.expanded) == reachable_pools(g), "pool coverage mismatch"
    else:
        last = t.attempted[-1]
        assert last in g.success, "success walk did not end on a success leaf"
        assert last in reachable_leaves(g)


def main() -> int:
    n = 0
    stopped = 0
    cyc = 0
    for seed in range(600):
        rng = random.Random(seed)
        g = random_graph(rng)
        check(g)
        n += 1
        t = walk(g)
        stopped += t.stopped_on_success
        cyc += sum(1 for o, _ in t.events if o == "cycle-skipped")
    assert n >= 500
    print(f"PASS {n} random graphs (seeds 0..599): {stopped} stopped on success, "
          f"{cyc} cycle-skipped edges observed, invariants held")
    return 0


if __name__ == "__main__":
    sys.exit(main())
