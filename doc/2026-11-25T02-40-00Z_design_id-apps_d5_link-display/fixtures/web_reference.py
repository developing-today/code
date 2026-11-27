#!/usr/bin/env python3
"""D5 slice 2 reference: how the web renderer projects, hit-tests and walks a frame.

Stdlib only. Builds on view_reference.py (slice 1) in this folder.

  python3 -I web_reference.py check   # exit 0 when every check passes
  python3 -I web_reference.py scale   # time a 10 000-node graph at the budget cap

It checks the renderer obligations of slice-2-web.md: the projection keeps
ring order and clockwise angular order, a pointer at a node or an edge hits
that target, keyboard order equals the list/tree floor's line order, a
refocus keeps node identities for the transition, and a graph far larger
than the budget still yields a frame within the cap.
"""

import itertools
import json
import math
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import view_reference as vr

NODE_RADIUS = 6.0  # px at zoom 1: the hit radius of a node
EDGE_SLOP = 4.0  # px: how far from an edge's line a pointer still hits it
MARGIN = 24.0  # px kept free at the viewport edge


def ring_spacing(frame, width, height):
    """Pixels between rings, so the outermost ring fits the viewport."""
    outer = max(node["ring"] for node in frame["nodes"])
    usable = min(width, height) / 2 - MARGIN
    return usable / max(outer + 0.5, 1)


def project(frame, width, height, zoom=1.0, pan=(0.0, 0.0)):
    """Screen position of every node: angle 0 at 12 o'clock, clockwise."""
    spacing = ring_spacing(frame, width, height) * zoom
    cx, cy = width / 2 + pan[0], height / 2 + pan[1]
    points = {}
    for node in frame["nodes"]:
        theta = 2 * math.pi * node["angle"] / vr.TURN
        r = node["ring"] * spacing
        points[node["id"]] = (cx + r * math.sin(theta), cy - r * math.cos(theta))
    return points


def hit(frame, points, x, y, zoom=1.0):
    """The intent target under the pointer: nodes win over edges."""
    best = None
    for node in frame["nodes"]:
        px, py = points[node["id"]]
        d = math.hypot(x - px, y - py)
        if d <= NODE_RADIUS * max(zoom, 1.0) and (best is None or d < best[0]):
            best = (d, "node", node["id"])
    if best:
        return best[1:]
    for edge in frame["edges"]:
        (ax, ay), (bx, by) = points[edge["source"]], points[edge["target"]]
        length = math.hypot(bx - ax, by - ay)
        if length == 0:
            continue
        t = max(
            0.0, min(1.0, ((x - ax) * (bx - ax) + (y - ay) * (by - ay)) / length**2)
        )
        d = math.hypot(x - (ax + t * (bx - ax)), y - (ay + t * (by - ay)))
        if d <= EDGE_SLOP and (best is None or d < best[0]):
            best = (d, "edge", edge["id"])
    return best[1:] if best else None


def keyboard_order(frame):
    """Arrow-key order: the spanning tree depth first in angle order."""
    kids = {node["id"]: [] for node in frame["nodes"]}
    for node in frame["nodes"]:
        if node["parent"] is not None:
            kids[node["parent"]].append(node)
    order = []

    def walk(node_id):
        order.append(node_id)
        for child in sorted(kids[node_id], key=lambda n: (n["angle"], n["id"])):
            walk(child["id"])

    walk(frame["focus"])
    return order


GESTURES = {
    # pointer
    ("pointer", "hover-node"): "identify",
    ("pointer", "hover-edge"): "identify-link",
    ("pointer", "click-node"): "open",
    ("pointer", "click-edge"): "inspect-link",
    ("pointer", "double-click-node"): "focus",
    ("pointer", "click-more-chip"): "focus",
    # touch
    ("touch", "tap-node"): "identify",
    ("touch", "tap-edge"): "identify-link",
    ("touch", "tap-card-open"): "open",
    ("touch", "tap-card-inspect"): "inspect-link",
    ("touch", "long-press-node"): "focus",
    ("touch", "tap-card-focus"): "focus",
    # keyboard
    ("keyboard", "arrow"): "identify",
    ("keyboard", "enter"): "open",
    ("keyboard", "f"): "focus",
    ("keyboard", "l"): "identify-link",
    ("keyboard", "shift-enter"): "inspect-link",
}
INTENTS = {"identify", "identify-link", "open", "inspect-link", "focus"}


def synthetic(nodes, fanout):
    """A public notes graph with `nodes` notes, each replied to by `fanout`."""
    objects = [
        {
            "object_id": f"s-{i}",
            "kind": "note",
            "author": "a",
            "area": "public",
            "created_claim": f"2026-11-01T00:00:{i % 60:02d}Z",
            "title": f"note {i}",
        }
        for i in range(nodes)
    ]
    links = []
    for i in range(1, nodes):
        parent = (i - 1) // fanout
        links.append(
            {
                "object_id": f"l-{i}",
                "verb": "reply",
                "source": f"s-{i}",
                "target": f"s-{parent}",
                "author": "a",
                "area": "public",
                "created_claim": "2026-11-01T00:00:00Z",
            }
        )
    return vr.Graph({"areas": {"public": "public"}, "objects": objects, "links": links})


def check():
    data = json.loads((Path(vr.HERE) / "graph.json").read_text())
    graph = vr.Graph(data)
    failures = []
    frames = vr.all_frames(graph)
    for name, frame in frames.items():
        for width, height, zoom in (
            (1280, 800, 1.0),
            (390, 844, 1.0),
            (1280, 800, 2.5),
        ):
            points = project(frame, width, height, zoom)
            cx, cy = width / 2, height / 2
            # Ring order: a deeper ring is never nearer the centre.
            by_ring = {}
            for node in frame["nodes"]:
                px, py = points[node["id"]]
                by_ring.setdefault(node["ring"], []).append(
                    math.hypot(px - cx, py - cy)
                )
            rings = sorted(by_ring)
            for inner, outer in itertools.pairwise(rings):
                if max(by_ring[inner]) >= min(by_ring[outer]):
                    failures.append(f"{name}: ring {inner} not inside ring {outer}")
            # Angular order: clockwise from 12 o'clock matches the model angle.
            for ring in rings[1:]:
                members = sorted(
                    (n for n in frame["nodes"] if n["ring"] == ring),
                    key=lambda n: n["angle"],
                )
                screen = [
                    math.atan2(points[n["id"]][0] - cx, cy - points[n["id"]][1])
                    % (2 * math.pi)
                    for n in members
                ]
                if screen != sorted(screen):
                    failures.append(f"{name}: angular order broken on ring {ring}")
            # Fit: at zoom 1 every node is inside the viewport.
            if zoom == 1.0:
                for node_id, (px, py) in points.items():
                    if not (0 <= px <= width and 0 <= py <= height):
                        failures.append(
                            f"{name}: {node_id} off screen at {width}x{height}"
                        )
            # Hit tests: each node's centre hits the node.
            for node in frame["nodes"]:
                if hit(frame, points, *points[node["id"]], zoom=zoom) != (
                    "node",
                    node["id"],
                ):
                    failures.append(f"{name}: node {node['id']} not hit at its centre")
        # Edge hits: an edge's midpoint hits it when no node is that close.
        points = project(frame, 1280, 800)
        for edge in frame["edges"]:
            (ax, ay), (bx, by) = points[edge["source"]], points[edge["target"]]
            mx, my = (ax + bx) / 2, (ay + by) / 2
            near_node = any(
                math.hypot(mx - px, my - py) <= NODE_RADIUS
                for px, py in points.values()
            )
            got = hit(frame, points, mx, my)
            if not near_node and (got is None or got[0] != "edge"):
                failures.append(f"{name}: edge {edge['id']} not hit at its midpoint")
        # Keyboard order is the floor's line order.
        tree = vr.tree_lines(frame)
        by_id = {node["id"]: node for node in frame["nodes"]}
        order = keyboard_order(frame)
        if order[0] != frame["focus"] or len(order) != len(frame["nodes"]):
            failures.append(
                f"{name}: keyboard order does not start at the focus or skips nodes"
            )
        for index, node_id in enumerate(order):
            node = by_id[node_id]
            if (
                index >= len(tree)
                or f"{node['label']} [{node['kind']}]" not in tree[index]
            ):
                failures.append(
                    f"{name}: keyboard stop {index} differs from the tree floor"
                )
                break
    # Refocus keeps identities so the transition can interpolate.
    frame = frames["notes-replies-first"]
    for node in frame["nodes"]:
        refocused = vr.respond(graph, frame, "focus", node["id"])["frame"]
        if node["id"] not in {n["id"] for n in refocused["nodes"]}:
            failures.append(f"refocus on {node['id']} lost the node")
    # Every gesture maps onto a slice 1 intent, and every intent is reachable
    # from the keyboard.
    if set(GESTURES.values()) - INTENTS:
        failures.append("a gesture maps to an unknown intent")
    for intent in INTENTS:
        if ("keyboard", intent) not in {(k[0], v) for k, v in GESTURES.items()}:
            failures.append(f"{intent} is not reachable from the keyboard")
    if GESTURES[("pointer", "click-node")] != "open":
        failures.append("plain click must be open")
    # Scale: 10 000 notes, the frame stays within the cap.
    big = synthetic(10_000, 4)
    frame = vr.build_frame(
        big, "anon", {"focus": "s-0", "radius": 4, "budget": vr.MAX_BUDGET}
    )
    if len(frame["nodes"]) > vr.MAX_BUDGET or vr.invariants(big, frame):
        failures.append("10 000-node graph broke the budget or the invariants")
    for failure in failures:
        print("FAIL", failure)
    print(f"{len(frames)} frames x 3 viewports, {len(failures)} failures")
    return 1 if failures else 0


def scale():
    for nodes, fanout, radius in ((10_000, 4, 4), (10_000, 30, 2), (50_000, 8, 4)):
        graph = synthetic(nodes, fanout)
        started = time.perf_counter()
        frame = vr.build_frame(
            graph, "anon", {"focus": "s-0", "radius": radius, "budget": vr.MAX_BUDGET}
        )
        took = time.perf_counter() - started
        print(
            f"{nodes} notes, fan-out {fanout}, radius {radius}: "
            f"{len(frame['nodes'])} nodes, {len(frame['edges'])} edges, "
            f"budget cut {frame['cuts']['budget']}, {took * 1000:.0f} ms (reference, not a target)"
        )
    return 0


if __name__ == "__main__":
    if sys.argv[1:] == ["check"]:
        sys.exit(check())
    if sys.argv[1:] == ["scale"]:
        sys.exit(scale())
    print(__doc__)
    sys.exit(2)
