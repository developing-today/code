#!/usr/bin/env python3
"""D2 slice 1 reference: embed objects, render resolution, and bounds.

Stdlib only. Reads fixtures/graph.json, checks every embed link against the
rules in the D2 record (§3 to §6), renders each fixture case for each viewer,
and compares the result with fixtures/expected.json.

  python3 -I embed_reference.py check     # exit 0 when every check passes
  python3 -I embed_reference.py render    # print the render trees as JSON

The renderer is a reference for the semantics, not a host renderer: it says
which object a host shows in each slot and with which state and badges (D2 §5).
"""

import hashlib
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent

MODES = {"live", "snapshot"}
SCOPES = {"object", "collection"}
# Host-wide render budget (§6): no render expands more than this many slots,
# whatever the bounds on individual embeds say.
RENDER_BUDGET = 64
# Defaults when an embed omits a bound (§6). Collection embeds must state
# max_count; depth defaults to this.
DEFAULT_MAX_DEPTH = 3

# Cases rendered by the checker: (case name, root object, viewer).
CASES = [
    ("post-embeds-photo", "post-photo-published", "anon"),
    ("note-embeds-thread", "note-recap", "anon"),
    ("page-embeds-collection", "page-meetup", "ana"),
    ("page-embeds-collection-owner", "page-meetup", "cyp"),
    ("private-target-outsider", "post-budget-pointer", "ana"),
    ("private-target-owner", "post-budget-pointer", "bo"),
    ("cycle", "note-a", "anon"),
    ("pruned-snapshot", "post-old-ref", "anon"),
]


class Graph:
    def __init__(self, data):
        self.areas = data["areas"]
        self.authors = data["authors"]
        self.objects = {o["object_id"]: o for o in data["objects"]}
        self.pruned = set(data.get("pruned", []))
        self.links = data["links"]

    def can_read(self, viewer, object_id):
        obj = self.objects.get(object_id)
        if obj is None:
            return False
        readers = self.areas[obj["area"]]
        return "*" in readers or viewer in readers

    def links_from(self, source, verb):
        out = [l for l in self.links if l["source"] == source and l["verb"] == verb]
        return sorted(out, key=lambda l: (l["created_claim"], l["object_id"]))

    def head(self, object_id, viewer):
        """Follow supersede links to the newest readable head (D3 §5, reader
        rule 3). Returns (head, forked)."""
        current, forked = object_id, False
        seen = {current}
        while True:
            nexts = [
                l["source"]
                for l in self.links
                if l["verb"] == "supersede"
                and l["target"] == current
                and self.can_read(viewer, l["source"])
            ]
            if not nexts:
                return current, forked
            if len(nexts) > 1:
                forked = True
            nexts.sort(key=lambda i: (self.objects[i]["created_claim"], i))
            nxt = nexts[-1]
            if nxt in seen:
                return current, True
            seen.add(nxt)
            current = nxt

    def members(self, container, viewer, at=None):
        """The nest projection of a container (D3 §4), restricted to what the
        viewer can read, oldest first. `at` limits to links created at or
        before that claim (used only by tests)."""
        out = []
        for l in self.links:
            if l["verb"] != "nest" or l["target"] != container:
                continue
            if at is not None and l["created_claim"] > at:
                continue
            if self.can_read(viewer, l["source"]):
                out.append((l["created_claim"], l["source"]))
        return [m for _, m in sorted(out)]


def members_digest(members):
    return "sha256:" + hashlib.sha256("\n".join(sorted(members)).encode()).hexdigest()


def check_links(g):
    """Structural and create-rule checks on every embed link (§3, §4)."""
    problems = []
    for l in g.links:
        if l["verb"] != "embed":
            continue
        name = l["object_id"]
        e = l.get("embed")
        if not isinstance(e, dict):
            problems.append(f"{name}: embed link without an embed block")
            continue
        if e.get("mode") not in MODES:
            problems.append(f"{name}: mode {e.get('mode')!r} not in {sorted(MODES)}")
        if e.get("scope") not in SCOPES:
            problems.append(f"{name}: scope {e.get('scope')!r} not in {sorted(SCOPES)}")
        unknown = set(e) - {"mode", "scope", "pin", "bounds"}
        if unknown:
            problems.append(f"{name}: unknown embed fields {sorted(unknown)}")
        pin = e.get("pin")
        if e.get("mode") == "snapshot":
            if not isinstance(pin, dict):
                problems.append(f"{name}: snapshot embed without a pin")
            elif e.get("scope") == "object":
                if pin.get("object") is None:
                    problems.append(f"{name}: object snapshot must pin an object")
            else:
                mem = pin.get("members")
                if pin.get("root") != l["target"] or not isinstance(mem, list):
                    problems.append(
                        f"{name}: collection pin needs root=target and members[]"
                    )
                elif pin.get("members_digest") != members_digest(mem):
                    problems.append(f"{name}: members_digest does not match members")
                elif len(mem) > e.get("bounds", {}).get("max_count", 0):
                    problems.append(f"{name}: pinned members exceed max_count")
        elif pin is not None:
            problems.append(f"{name}: a live embed carries no pin")
        if e.get("scope") == "collection":
            b = e.get("bounds")
            if not isinstance(b, dict) or not isinstance(b.get("max_count"), int):
                problems.append(f"{name}: collection embed must state bounds.max_count")
            elif b["max_count"] < 1:
                problems.append(f"{name}: max_count must be at least 1")
        # Create rule (§4): the embedder must have been able to read the
        # target. A pruned target cannot be re-checked and is recorded as such.
        embedder = g.authors[l["author"]]
        if l["target"] in g.pruned:
            continue
        if l["target"] not in g.objects:
            problems.append(
                f"{name}: target {l['target']!r} is neither stored nor pruned"
            )
        elif not g.can_read(embedder, l["target"]):
            problems.append(f"{name}: embedder {embedder} cannot read the target")
        if l["source"] not in g.objects:
            problems.append(f"{name}: source {l['source']!r} is not stored")
        elif g.authors[g.objects[l["source"]]["author"]] != embedder:
            problems.append(f"{name}: an embed is authored by its source's author")
    return problems


class Renderer:
    def __init__(self, g, viewer):
        self.g, self.viewer, self.budget = g, viewer, RENDER_BUDGET

    def obj(self, object_id, path, depth_left):
        """Render a content object and its own outbound embeds."""
        self.budget -= 1
        node = {"object": object_id, "kind": self.g.objects[object_id]["kind"]}
        embeds = []
        for l in self.g.links_from(object_id, "embed"):
            embeds.append(self.embed(l, path + [object_id], depth_left))
        if embeds:
            node["embeds"] = embeds
        return node

    def unavailable(self, link):
        # One state for "not authorised", "not stored" and "pruned" (§5).
        return {"link": link["object_id"], "state": "unavailable"}

    def embed(self, l, path, depth_left):
        g, v, e = self.g, self.viewer, l["embed"]
        bounds = e.get("bounds", {})
        depth_left = min(depth_left, bounds.get("max_depth", DEFAULT_MAX_DEPTH))
        base = {"link": l["object_id"], "mode": e["mode"], "scope": e["scope"]}
        target = l["target"]
        if not g.can_read(v, target):
            return self.unavailable(l)
        if target in path:
            return {**base, "state": "cycle-cut", "object": target}
        if depth_left <= 0:
            return {**base, "state": "depth-cut", "object": target}
        if self.budget <= 0:
            return {**base, "state": "budget-cut", "object": target}
        if e["scope"] == "object":
            return self.embed_object(l, base, path, depth_left)
        return self.embed_collection(l, base, path, depth_left)

    def embed_object(self, l, base, path, depth_left):
        g, v, e = self.g, self.viewer, l["embed"]
        head, forked = g.head(l["target"], v)
        badges = ["fork"] if forked else []
        if e["mode"] == "live":
            shown = head
            if shown in path:
                return {**base, "state": "cycle-cut", "object": shown}
            if head != l["target"]:
                badges.append("edited")
        else:
            shown = e["pin"]["object"]
            if not g.can_read(v, shown):
                return self.unavailable(l)
            if head != shown:
                badges.append("changed-since-embed")
        if g.objects[shown]["kind"] == "tombstone":
            return {**base, "state": "removed", "object": shown}
        node = {**base, "state": "shown", **self.obj(shown, path, depth_left - 1)}
        if badges:
            node["badges"] = badges
        return node

    def embed_collection(self, l, base, path, depth_left):
        g, v, e = self.g, self.viewer, l["embed"]
        cap = e["bounds"]["max_count"]
        live_members = g.members(l["target"], v)
        badges = []
        if e["mode"] == "live":
            members = live_members
        else:
            # Pinned members the viewer can no longer read are dropped, like
            # unreadable members of a live collection (§5).
            members = [m for m in e["pin"]["members"] if g.can_read(v, m)]
            added = [m for m in live_members if m not in e["pin"]["members"]]
            if added:
                badges.append("changed-since-embed")
        shown, more = members[:cap], max(0, len(members) - cap)
        rendered = []
        for m in shown:
            if self.budget <= 0:
                more += len(shown) - len(rendered)
                break
            if m in path:
                rendered.append({"object": m, "state": "cycle-cut"})
            else:
                rendered.append(self.obj(m, path + [l["target"]], depth_left - 1))
        node = {**base, "state": "shown", "object": l["target"], "members": rendered}
        if more:
            node["more"] = more
        if badges:
            node["badges"] = badges
        return node


def render_case(g, root, viewer):
    return Renderer(g, viewer).obj(root, [], DEFAULT_MAX_DEPTH)


def render_all(g):
    return {name: render_case(g, root, viewer) for name, root, viewer in CASES}


def main(argv):
    g = Graph(json.loads((HERE / "graph.json").read_text(encoding="utf-8")))
    cmd = argv[1] if len(argv) > 1 else "check"
    rendered = render_all(g)
    if cmd == "render":
        print(json.dumps(rendered, indent=2))
        return 0
    problems = check_links(g)
    expected = json.loads((HERE / "expected.json").read_text(encoding="utf-8"))
    for name in sorted(set(expected) | set(rendered)):
        if expected.get(name) != rendered.get(name):
            problems.append(f"case {name}: render differs from expected.json")
    # Negative checks: each mutation must be refused by check_links.
    bad = [
        (
            "live embed with a pin",
            "lnk-embed-photo-live",
            {"pin": {"object": "img-group-photo"}},
        ),
        ("collection without bounds", "lnk-embed-album-live", {"bounds": None}),
        (
            "tampered member digest",
            "lnk-embed-thread-snapshot",
            {
                "pin": {
                    "root": "post-meetup-thread",
                    "members": ["msg-photo-answer"],
                    "members_digest": "sha256:00",
                }
            },
        ),
        ("unknown mode", "lnk-embed-photo-live", {"mode": "mirror"}),
        (
            "embedder cannot read target",
            "lnk-embed-album-live",
            {
                "target": "img-private-draft",
                "embed": {"mode": "live", "scope": "object"},
            },
        ),
    ]
    for label, link_id, change in bad:
        data = json.loads((HERE / "graph.json").read_text(encoding="utf-8"))
        for l in data["links"]:
            if l["object_id"] == link_id:
                if "embed" in change or "target" in change:
                    l.update(change)
                else:
                    for k, val in change.items():
                        if val is None:
                            l["embed"].pop(k, None)
                        else:
                            l["embed"][k] = val
        if not check_links(Graph(data)):
            problems.append(f"negative check not refused: {label}")
    for p in problems:
        print(f"FAIL: {p}")
    if problems:
        return 1
    print(
        f"ok: {sum(1 for l in g.links if l['verb'] == 'embed')} embeds, {len(CASES)} cases, {len(bad)} refusals"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
