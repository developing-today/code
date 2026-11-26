#!/usr/bin/env python3
"""D5 slice 1 reference: the graph view model and its interaction contract.

Stdlib only. Reads fixtures/graph.json, builds a frame for every case in
CASES, checks the frame invariants of the D5 record (note.md §3 to §6),
and compares each frame with fixtures/expected.json.

  python3 -I view_reference.py check          # exit 0 when every check passes
  python3 -I view_reference.py frame <case>   # print one frame as JSON
  python3 -I view_reference.py tree <case>    # print the list/tree floor
  python3 -I view_reference.py expected       # print expected.json as computed

The reference says which nodes and edges a frame holds, where each node sits
in the shared polar layout, and what each intent answers. It draws nothing:
pixels, cells and escape sequences belong to the renderers (slices 2 and 3).
"""

import json
import sys
from fractions import Fraction
from pathlib import Path

HERE = Path(__file__).resolve().parent

# D3 v1 verbs plus D2's embed. Supersede is history, never an edge (§3.3).
DRAWN_VERBS = ("reply", "quote", "embed", "mention", "attach", "nest")
KNOWN_VERBS = set(DRAWN_VERBS) | {"supersede"}
# A link that is itself linked to shows as a link node joined to its two
# endpoints by these structural edges (§3.2). They are not D3 links.
ENDPOINT = "endpoint"
# Expansion order: replies first, then the other verbs (§3.4).
VERB_RANK = {verb: rank for rank, verb in enumerate(DRAWN_VERBS + (ENDPOINT,))}
# Semantic edge styles, from D3's per-verb rendering hints (§3.5).
EDGE_STYLE = {
    "reply": "conversation",
    "quote": "pinned-reference",
    "embed": "transclusion",
    "mention": "reference",
    "attach": "carry",
    "nest": "containment",
    ENDPOINT: "link-endpoint",
}

TURN = 65536  # angles are integers in 1/65536 of a turn (§4)
DEFAULT_RADIUS = 2
MAX_RADIUS = 4
DEFAULT_BUDGET = 200
MAX_BUDGET = 2000
BRIGHT_HOPS = 1  # hop 0 and 1 are bright; deeper nodes are context (dimmed)
LABEL_CHARS = 60

CASES = {
    "notes-replies-first": {
        "viewer": "ana",
        "query": {
            "focus": "n-spaced",
            "verbs": ["reply", "quote", "mention", "attach"],
        },
    },
    "notes-owner-sees-private": {
        "viewer": "bo",
        "query": {
            "focus": "n-spaced",
            "verbs": ["reply", "quote", "mention", "attach"],
        },
    },
    "notes-budget-cut": {
        "viewer": "ana",
        "query": {
            "focus": "n-spaced",
            "verbs": ["reply", "quote", "mention", "attach"],
            "budget": 4,
        },
    },
    "notes-edited-and-linked-link": {
        "viewer": "anon",
        "query": {"focus": "n-sleep", "radius": 2},
    },
    "notes-time-window": {
        "viewer": "ana",
        "query": {"focus": "n-spaced", "since": "2026-11-04T00:00:00Z"},
    },
    "programme-epic": {
        "viewer": "anon",
        "query": {"focus": "e-13880", "radius": 1},
    },
    "programme-within-epic": {
        "viewer": "anon",
        "query": {
            "focus": "d-5",
            "verbs": ["mention", "embed", "reply"],
            "within": "e-13880",
        },
    },
    "programme-author-filter": {
        "viewer": "anon",
        "query": {
            "focus": "d-5",
            "verbs": ["mention", "embed", "reply"],
            "radius": 1,
            "authors": ["lane-4", "drew"],
        },
    },
}


class QueryError(Exception):
    """A query the view model refuses, with a stable code."""


class Graph:
    def __init__(self, data):
        self.areas = data["areas"]
        self.objects = {}
        for obj in data["objects"]:
            self.objects[obj["object_id"]] = dict(obj, is_link=False)
        for link in data["links"]:
            self.objects[link["object_id"]] = dict(link, is_link=True)
        self.links = [self.objects[link["object_id"]] for link in data["links"]]

    def readable(self, viewer, object_id):
        obj = self.objects.get(object_id)
        if obj is None:
            return False
        readers = self.areas[obj["area"]]
        return readers == "public" or viewer in readers


class View:
    """Everything one viewer can see: readable objects, chains, edges."""

    def __init__(self, graph, viewer):
        self.graph = graph
        self.viewer = viewer
        self.invalid_supersedes = []
        self.unknown_verbs = 0
        readable_links = [
            link
            for link in graph.links
            if graph.readable(viewer, link["object_id"])
            and graph.readable(viewer, link["source"])
            and graph.readable(viewer, link["target"])
        ]
        # Supersede chains (§3.3): source replaces target when both share
        # an author (D3's create-rule without a moderation grant).
        successors = {}
        predecessor = {}
        for link in readable_links:
            if link["verb"] != "supersede":
                continue
            old, new = link["target"], link["source"]
            if graph.objects[old]["author"] != graph.objects[new]["author"]:
                self.invalid_supersedes.append(link["object_id"])
                continue
            successors.setdefault(old, []).append(new)
            predecessor[new] = old
        self.node_of = {}
        self.chains = {}
        for object_id in sorted(graph.objects):
            if not graph.readable(viewer, object_id):
                continue
            root = object_id
            while root in predecessor:
                root = predecessor[root]
            self.node_of[object_id] = root
            self.chains.setdefault(root, []).append(object_id)
        self.heads = {}
        for root, members in self.chains.items():
            heads = sorted(m for m in members if not successors.get(m))
            self.heads[root] = heads
        # Drawable edges between chain nodes.
        self.edges = []
        for link in readable_links:
            verb = link["verb"]
            if verb == "supersede":
                continue
            if verb not in KNOWN_VERBS:
                # D3 reader rule: unknown verbs are kept and skipped.
                self.unknown_verbs += 1
                continue
            source = self.node_of[link["source"]]
            target = self.node_of[link["target"]]
            if source == target:
                continue
            self.edges.append(dict(link, source_node=source, target_node=target))
        # Link nodes: a drawable link whose endpoint is another link object
        # joins that link object to the graph through its own endpoints.
        linked_links = sorted(
            {
                end
                for edge in self.edges
                for end in (edge["source_node"], edge["target_node"])
                if graph.objects[end]["is_link"]
            }
        )
        for link_id in linked_links:
            link = graph.objects[link_id]
            if link["verb"] not in KNOWN_VERBS or link["verb"] == "supersede":
                continue
            for end in ("source", "target"):
                self.edges.append(
                    {
                        "object_id": f"{link_id}/{end}",
                        "verb": ENDPOINT,
                        "of_verb": link["verb"],
                        "author": link["author"],
                        "source_node": link_id,
                        "target_node": self.node_of[link[end]],
                    }
                )
        self.edges.sort(key=lambda edge: edge["object_id"])

    def shown(self, node):
        """The object a node displays: its first chain head."""
        return self.heads[node][0]

    def badges(self, node):
        badges = []
        if len(self.chains[node]) > 1:
            badges.append("edited")
        if len(self.heads[node]) > 1:
            badges.append("fork")
        return badges


def normalise(query):
    radius = query.get("radius", DEFAULT_RADIUS)
    budget = query.get("budget", DEFAULT_BUDGET)
    if not isinstance(radius, int) or not 0 <= radius <= MAX_RADIUS:
        raise QueryError("radius_out_of_range")
    if not isinstance(budget, int) or not 1 <= budget <= MAX_BUDGET:
        raise QueryError("budget_out_of_range")
    verbs = query.get("verbs", list(DRAWN_VERBS))
    for verb in verbs:
        if verb not in DRAWN_VERBS:
            raise QueryError("verb_not_drawable")
    return {
        "focus": query["focus"],
        "radius": radius,
        "budget": budget,
        "verbs": sorted(set(verbs), key=VERB_RANK.get),
        "authors": sorted(set(query["authors"])) if "authors" in query else None,
        "since": query.get("since"),
        "until": query.get("until"),
        "within": query.get("within"),
    }


def nest_descendants(view, container):
    """The container and every node nested under it, transitively."""
    inside = {container}
    frontier = [container]
    while frontier:
        parent = frontier.pop()
        for edge in view.edges:
            if edge["verb"] == "nest" and edge["target_node"] == parent:
                child = edge["source_node"]
                if child not in inside:
                    inside.add(child)
                    frontier.append(child)
    return inside


def label_of(obj):
    text = obj.get("title") or obj.get("body") or obj["object_id"]
    return text if len(text) <= LABEL_CHARS else text[: LABEL_CHARS - 1] + "…"


def build_frame(graph, viewer, query):
    """One frame of the view model (§3): the answer to a focus query."""
    query = normalise(query)
    view = View(graph, viewer)
    focus_object = query["focus"]
    if focus_object not in view.node_of:
        # An unreadable focus and a missing one answer alike (D2 §5).
        raise QueryError("focus_unavailable")
    focus = view.node_of[focus_object]

    allowed = None
    if query["within"] is not None:
        if query["within"] not in view.node_of:
            raise QueryError("within_unavailable")
        allowed = nest_descendants(view, view.node_of[query["within"]])
        if focus not in allowed:
            raise QueryError("focus_outside_within")

    def passes(node):
        if node == focus:
            return True
        if allowed is not None and node not in allowed:
            return False
        shown = graph.objects[view.shown(node)]
        if query["authors"] is not None and shown["author"] not in query["authors"]:
            return False
        claim = shown["created_claim"]
        if query["since"] is not None and claim < query["since"]:
            return False
        return query["until"] is None or claim < query["until"]

    verbs = set(query["verbs"])

    def drawn(edge):
        return edge.get("of_verb", edge["verb"]) in verbs

    incident = {}
    for edge in view.edges:
        if not drawn(edge):
            continue
        if not (passes(edge["source_node"]) and passes(edge["target_node"])):
            continue
        incident.setdefault(edge["source_node"], []).append(
            (edge, edge["target_node"], "out")
        )
        incident.setdefault(edge["target_node"], []).append(
            (edge, edge["source_node"], "in")
        )

    def order(item):
        edge, other, direction = item
        claim = graph.objects[view.shown(other)]["created_claim"]
        return (
            VERB_RANK[edge["verb"]],
            0 if direction == "in" else 1,
            claim,
            other,
            edge["object_id"],
        )

    for items in incident.values():
        items.sort(key=order)

    # Breadth-first expansion in the fixed order, bounded by radius and budget.
    hop = {focus: 0}
    parent = {focus: None}
    via = {focus: None}
    children = {focus: []}
    queue = [focus]
    budget_cut = False
    radius_cut = False
    head = 0
    while head < len(queue):
        node = queue[head]
        head += 1
        for edge, other, direction in incident.get(node, []):
            if other in hop:
                continue
            if hop[node] == query["radius"]:
                radius_cut = True
                continue
            if len(queue) == query["budget"]:
                budget_cut = True
                continue
            hop[other] = hop[node] + 1
            parent[other] = node
            via[other] = (edge, direction)
            children[other] = []
            children[node].append(other)
            queue.append(other)

    in_frame = set(queue)
    more = {
        node: len(
            {other for _, other, _ in incident.get(node, []) if other not in in_frame}
        )
        for node in queue
    }

    # Shared polar layout (§4): ring = hop, angle from the spanning tree,
    # each subtree's sector proportional to its leaf count.
    leaves = {}

    def count_leaves(node):
        total = sum(count_leaves(child) for child in children[node]) or 1
        leaves[node] = total
        return total

    count_leaves(focus)
    angle = {}

    def place(node, start, width):
        angle[node] = 0 if node == focus else int((start + width / 2) * TURN) % TURN
        cursor = start
        for child in children[node]:
            share = width * Fraction(leaves[child], leaves[node])
            place(child, cursor, share)
            cursor += share

    place(focus, Fraction(0), Fraction(1))

    nodes = []
    for node in queue:
        shown = graph.objects[view.shown(node)]
        edge, direction = via[node] or (None, None)
        nodes.append(
            {
                "id": node,
                "shows": shown["object_id"],
                "kind": "link" if shown["is_link"] else shown["kind"],
                "label": label_of(shown)
                if not shown["is_link"]
                else shown["verb"] + " link",
                "author": shown["author"],
                "created_claim": shown["created_claim"],
                "hop": hop[node],
                "role": "focus"
                if hop[node] == 0
                else "neighbour"
                if hop[node] <= BRIGHT_HOPS
                else "context",
                "dim": hop[node] > BRIGHT_HOPS,
                "ring": hop[node],
                "angle": angle[node],
                "parent": parent[node],
                "via": None
                if edge is None
                else {
                    "link": edge["object_id"],
                    "verb": edge["verb"],
                    "direction": direction,
                },
                "badges": view.badges(node),
                "more": more[node],
            }
        )
    nodes.sort(key=lambda n: (n["ring"], n["angle"], n["id"]))

    edges = [
        {
            "id": edge["object_id"],
            "verb": edge["verb"],
            "style": EDGE_STYLE[edge["verb"]],
            "source": edge["source_node"],
            "target": edge["target_node"],
            "author": edge["author"],
            "dim": max(hop[edge["source_node"]], hop[edge["target_node"]])
            > BRIGHT_HOPS,
        }
        for edge in view.edges
        if drawn(edge)
        and edge["source_node"] in in_frame
        and edge["target_node"] in in_frame
    ]

    return {
        "viewer": viewer,
        "query": query,
        "focus": focus,
        "nodes": nodes,
        "edges": edges,
        "cuts": {"budget": budget_cut, "radius": radius_cut},
        "claims_untrusted": query["since"] is not None or query["until"] is not None,
        "skipped_unknown_verbs": view.unknown_verbs,
        "ignored_supersedes": sorted(view.invalid_supersedes),
    }


def tree_lines(frame):
    """The list/tree floor (§6): every frame renders as these lines."""
    by_id = {node["id"]: node for node in frame["nodes"]}
    kids = {node["id"]: [] for node in frame["nodes"]}
    for node in frame["nodes"]:
        if node["parent"] is not None:
            kids[node["parent"]].append(node)
    lines = []

    def describe(node):
        text = f"{node['label']} [{node['kind']}]"
        for badge in node["badges"]:
            text += f" ({badge})"
        if node["more"]:
            text += f" (+{node['more']} more)"
        return text

    def walk(node, depth):
        if node["via"] is None:
            lines.append(describe(node))
        else:
            arrow = "<-" if node["via"]["direction"] == "in" else "->"
            lines.append(
                "  " * depth + f"{arrow} {node['via']['verb']}: " + describe(node)
            )
        for child in sorted(kids[node["id"]], key=lambda n: (n["angle"], n["id"])):
            walk(child, depth + 1)

    walk(by_id[frame["focus"]], 0)
    tree_links = {node["via"]["link"] for node in frame["nodes"] if node["via"]}
    extra = [edge for edge in frame["edges"] if edge["id"] not in tree_links]
    if extra:
        lines.append("also linked:")
        for edge in extra:
            lines.append(
                f"  {by_id[edge['source']]['label']} -{edge['verb']}-> {by_id[edge['target']]['label']}"
            )
    if frame["cuts"]["budget"]:
        lines.append("(node budget reached)")
    return lines


def respond(graph, frame, intent, target):
    """The interaction contract (§5): what a renderer must answer per intent."""
    node_ids = {node["id"] for node in frame["nodes"]}
    edge_ids = {edge["id"] for edge in frame["edges"]}
    if intent in ("identify", "open", "focus") and target not in node_ids:
        raise QueryError("target_not_in_frame")
    if intent in ("identify-link", "inspect-link") and target not in edge_ids:
        raise QueryError("target_not_in_frame")
    if intent == "identify":
        node = next(n for n in frame["nodes"] if n["id"] == target)
        shown = graph.objects[node["shows"]]
        highlight_edges = sorted(
            e["id"] for e in frame["edges"] if target in (e["source"], e["target"])
        )
        highlight_nodes = sorted(
            {e["source"] for e in frame["edges"] if e["target"] == target}
            | {e["target"] for e in frame["edges"] if e["source"] == target}
        )
        return {
            "show": {
                "label": node["label"],
                "kind": node["kind"],
                "author": node["author"],
                "created_claim": node["created_claim"],
                "claim_untrusted": True,
                "badges": node["badges"],
                "preview": shown.get("body", shown.get("title", "")),
            },
            "highlight": {"edges": highlight_edges, "nodes": highlight_nodes},
            "frame_changes": False,
        }
    if intent == "identify-link":
        edge = next(e for e in frame["edges"] if e["id"] == target)
        return {
            "show": {
                "verb": edge["verb"],
                "author": edge["author"],
                "source": edge["source"],
                "target": edge["target"],
            },
            "highlight": {
                "edges": [edge["id"]],
                "nodes": sorted({edge["source"], edge["target"]}),
            },
            "frame_changes": False,
        }
    if intent == "open":
        node = next(n for n in frame["nodes"] if n["id"] == target)
        uri_shape = "head" if "edited" in node["badges"] else "obj"
        return {
            "navigate": {
                "object": node["shows"] if uri_shape == "obj" else node["id"],
                "uri_shape": uri_shape,
            },
            "frame_changes": False,
        }
    if intent == "inspect-link":
        return {
            "navigate": {"object": target, "uri_shape": "obj"},
            "frame_changes": False,
        }
    if intent == "focus":
        query = dict(frame["query"], focus=target)
        query = {k: v for k, v in query.items() if v is not None}
        return {
            "frame": build_frame(graph, frame["viewer"], query),
            "frame_changes": True,
        }
    raise QueryError("unknown_intent")


def invariants(graph, frame):
    """Checks every frame must pass, whatever the case."""
    problems = []
    nodes = frame["nodes"]
    ids = [node["id"] for node in nodes]
    if len(ids) != len(set(ids)):
        problems.append("duplicate node")
    if len(nodes) > frame["query"]["budget"]:
        problems.append("budget exceeded")
    focus = [node for node in nodes if node["role"] == "focus"]
    if len(focus) != 1 or focus[0]["id"] != frame["focus"] or focus[0]["angle"] != 0:
        problems.append("focus not unique at the centre")
    for node in nodes:
        if not graph.readable(frame["viewer"], node["shows"]):
            problems.append(f"unreadable node {node['id']}")
        if node["hop"] > frame["query"]["radius"]:
            problems.append(f"node past radius {node['id']}")
    for ring in {node["ring"] for node in nodes}:
        angles = [node["angle"] for node in nodes if node["ring"] == ring]
        if len(angles) != len(set(angles)):
            problems.append(f"angle collision on ring {ring}")
    node_set = set(ids)
    for edge in frame["edges"]:
        if edge["source"] not in node_set or edge["target"] not in node_set:
            problems.append(f"dangling edge {edge['id']}")
        link_id = edge["id"].split("/")[0]
        verb = graph.objects[link_id]["verb"]
        if verb not in frame["query"]["verbs"]:
            problems.append(f"filtered verb drawn {edge['id']}")
        if not graph.readable(frame["viewer"], link_id):
            problems.append(f"unreadable edge {edge['id']}")
    return problems


def permuted(data):
    """The same graph with every list reversed: frames must not change."""
    return {
        "areas": data["areas"],
        "objects": list(reversed(data["objects"])),
        "links": list(reversed(data["links"])),
    }


def all_frames(graph):
    return {
        name: build_frame(graph, case["viewer"], case["query"])
        for name, case in CASES.items()
    }


def computed_expected(graph):
    frames = all_frames(graph)
    return {
        name: {
            "nodes": [
                [
                    n["id"],
                    n["shows"],
                    n["ring"],
                    n["angle"],
                    n["role"],
                    n["badges"],
                    n["more"],
                ]
                for n in frame["nodes"]
            ],
            "edges": [e["id"] for e in frame["edges"]],
            "cuts": frame["cuts"],
            "claims_untrusted": frame["claims_untrusted"],
            "skipped_unknown_verbs": frame["skipped_unknown_verbs"],
            "ignored_supersedes": frame["ignored_supersedes"],
            "tree": tree_lines(frame),
        }
        for name, frame in frames.items()
    }


def check():
    data = json.loads((HERE / "graph.json").read_text())
    graph = Graph(data)
    failures = []
    frames = all_frames(graph)
    for name, frame in frames.items():
        for problem in invariants(graph, frame):
            failures.append(f"{name}: {problem}")
    # Determinism: input order never changes a frame.
    reordered = all_frames(Graph(permuted(data)))
    for name in frames:
        if json.dumps(frames[name], sort_keys=True) != json.dumps(
            reordered[name], sort_keys=True
        ):
            failures.append(f"{name}: frame depends on input order")
    # Golden frames.
    expected = json.loads((HERE / "expected.json").read_text())
    got = computed_expected(graph)
    for name in sorted(set(expected) | set(got)):
        if expected.get(name) != got.get(name):
            failures.append(f"{name}: differs from expected.json")
    # Interaction contract.
    frame = frames["notes-replies-first"]
    for node in frame["nodes"]:
        refocused = respond(graph, frame, "focus", node["id"])["frame"]
        if refocused["focus"] != node["id"] or invariants(graph, refocused):
            failures.append(f"focus intent on {node['id']} broke the frame")
        if respond(graph, frame, "open", node["id"])["frame_changes"]:
            failures.append("open changed the frame")
        shown = respond(graph, frame, "identify", node["id"])
        if not shown["show"]["claim_untrusted"]:
            failures.append("identify presented a claim as trusted")
    for edge in frame["edges"]:
        if (
            respond(graph, frame, "inspect-link", edge["id"])["navigate"]["object"]
            != edge["id"]
        ):
            failures.append("inspect-link did not open the link object")
    edited = frames["notes-edited-and-linked-link"]
    forget = next(n for n in edited["nodes"] if n["id"] == "n-forget")
    if respond(graph, edited, "open", "n-forget")["navigate"] != {
        "object": "n-forget",
        "uri_shape": "head",
    }:
        failures.append("open on an edited node must name the chain head form")
    if forget["shows"] != "n-forget-v2":
        failures.append("edited node must show its chain head")
    # Refusals answer alike for unreadable and missing focus.
    for focus in ("n-private", "n-does-not-exist"):
        try:
            build_frame(graph, "ana", {"focus": focus})
            failures.append(f"focus {focus} was not refused")
        except QueryError as error:
            if str(error) != "focus_unavailable":
                failures.append(f"focus {focus} refused as {error}")
    for query, code in (
        ({"focus": "n-spaced", "radius": 5}, "radius_out_of_range"),
        ({"focus": "n-spaced", "budget": 0}, "budget_out_of_range"),
        ({"focus": "n-spaced", "verbs": ["supersede"]}, "verb_not_drawable"),
        ({"focus": "x-elsewhere", "within": "e-13880"}, "focus_outside_within"),
    ):
        try:
            build_frame(graph, "anon", query)
            failures.append(f"{query} was not refused")
        except QueryError as error:
            if str(error) != code:
                failures.append(f"{query} refused as {error}, wanted {code}")
    for failure in failures:
        print("FAIL", failure)
    print(f"{len(CASES)} cases, {len(failures)} failures")
    return 1 if failures else 0


def main(argv):
    graph = Graph(json.loads((HERE / "graph.json").read_text()))
    if argv[1:] == ["check"]:
        return check()
    if argv[1:] == ["expected"]:
        print(json.dumps(computed_expected(graph), indent=1, ensure_ascii=False))
        return 0
    if len(argv) == 3 and argv[1] in ("frame", "tree") and argv[2] in CASES:
        case = CASES[argv[2]]
        frame = build_frame(graph, case["viewer"], case["query"])
        if argv[1] == "frame":
            print(json.dumps(frame, indent=1, ensure_ascii=False))
        else:
            print("\n".join(tree_lines(frame)))
        return 0
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
