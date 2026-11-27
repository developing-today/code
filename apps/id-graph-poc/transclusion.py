"""Fixture-backed live transclusion renderer for PoC 2 (#14037 slice 2).

This is a small host renderer, independent of D2's reference renderer. It reads
D2's graph fixtures and implements only the object/collection views exercised
by this local proof. Delegated view grants are deliberately not implemented.
"""

import json
from pathlib import Path

DELEGATED_VIEW_GRANTS = False
REPO = Path(__file__).resolve().parents[2]
D2_FIXTURES = REPO / "doc" / "2026-11-25T01-40-00Z_design_id-apps_d2_transclusion" / "fixtures"


def load_graph():
    """Load the D2 fixture graph into indexes suitable for a host renderer."""
    graph = json.loads((D2_FIXTURES / "graph.json").read_text(encoding="utf-8"))
    graph["objects"] = {o["object_id"]: o for o in graph["objects"]}
    return graph


def can_read(graph, viewer, object_id):
    """Composition-A read check: only the object's area grants access."""
    obj = graph["objects"].get(object_id)
    if obj is None:
        return False
    readers = graph["areas"][obj["area"]]
    return "*" in readers or viewer in readers


def live_head(graph, object_id, viewer):
    """Follow readable supersede links to the newest live object head."""
    current = object_id
    seen = {current}
    while True:
        heads = [
            link["source"]
            for link in graph["links"]
            if link["verb"] == "supersede"
            and link["target"] == current
            and can_read(graph, viewer, link["source"])
        ]
        if not heads:
            return current
        next_head = max(
            heads,
            key=lambda item: (graph["objects"][item]["created_claim"], item),
        )
        if next_head in seen:
            return current
        seen.add(next_head)
        current = next_head


def _unavailable(link_id):
    # Do not include even mode, scope, or target when the viewer lacks access.
    return {"link": link_id, "state": "unavailable"}


def _render_embed(graph, link, viewer):
    target = link["target"]
    if not can_read(graph, viewer, target):
        return _unavailable(link["object_id"])

    embed = link["embed"]
    mode, scope = embed["mode"], embed["scope"]
    base = {"link": link["object_id"], "mode": mode, "scope": scope}

    if scope == "object":
        head = live_head(graph, target, viewer)
        shown = head if mode == "live" else embed["pin"]["object"]
        if not can_read(graph, viewer, shown):
            return _unavailable(link["object_id"])
        result = {
            **base,
            "state": "shown",
            "object": shown,
            "kind": graph["objects"][shown]["kind"],
        }
        if mode == "live" and shown != target:
            result["badges"] = ["edited"]
        elif mode == "snapshot" and head != shown:
            result["badges"] = ["changed-since-embed"]
        return result

    # A live collection is the readable projection of current nest links;
    # private members are omitted before applying max_count and computing more.
    cap = embed["bounds"]["max_count"]
    members = sorted(
        (
            (link["created_claim"], link["source"])
            for link in graph["links"]
            if link["verb"] == "nest"
            and link["target"] == target
            and can_read(graph, viewer, link["source"])
        )
    )
    readable = [object_id for _, object_id in members]
    shown = readable[:cap]
    result = {
        **base,
        "state": "shown",
        "object": target,
        "members": [
            {"object": object_id, "kind": graph["objects"][object_id]["kind"]}
            for object_id in shown
        ],
    }
    more = max(0, len(readable) - len(shown))
    if more:
        result["more"] = more
    return result


def render_object(graph, object_id, viewer):
    """Render one source object and its authored embeds."""
    obj = graph["objects"][object_id]
    rendered = {"object": object_id, "kind": obj["kind"]}
    embeds = sorted(
        (
            link
            for link in graph["links"]
            if link["verb"] == "embed" and link["source"] == object_id
        ),
        key=lambda link: (link["created_claim"], link["object_id"]),
    )
    if embeds:
        rendered["embeds"] = [_render_embed(graph, link, viewer) for link in embeds]
    return rendered


def render_composed(graph, viewer):
    """Compose two live source objects into one local page for PoC 2."""
    return {
        "object": "composed-meetup",
        "sections": [
            render_object(graph, "page-meetup", viewer),
            render_object(graph, "post-photo-published", viewer),
        ],
    }
