"""PoC 1 (#14037): Rummager-like organising over the D3 ID graph (stdlib only).

Loads the D3 cross-app fixtures, lets a curator who authored none of them
collect them into a collection object, and answers discovery queries over
verb links: backlinks, neighbourhood, recent changes.

Model gap: D3 v1 has no collection kind and no collect verb. The PoC admits
`collect` provisionally (see README.md) and records the gap on #13885.
"""

import json
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
FIXTURES = (
    REPO / "doc" / "2026-11-24_design_id-graph-layer-typed-links-verbs" / "fixtures"
)

CONTENT_FILES = [
    "chat-message.json",
    "forum-post.json",
    "image.json",
    "cross-app-chat-answer.json",
    "cross-app-forum-published.json",
]
LINK_FILES = [
    "link-reply-chat-to-forum.json",
    "link-attach-photo.json",
    "link-quote-chat-ask.json",
    "link-nest-thread-in-board.json",
    "link-mention-photo.json",
]

CURATOR = "ed25519:dddd4444curator"
COLLECTION_ID = "coll-meetup-2026-11-22"
D3_VERBS = {"reply", "mention", "quote", "attach", "supersede", "nest"}
POC_VERBS = D3_VERBS | {"collect"}


def load(name):
    with open(FIXTURES / name, encoding="utf-8") as f:
        return json.load(f)


def build_graph():
    objects = {}
    links = {}
    for name in CONTENT_FILES:
        o = load(name)
        objects[o["object_id"]] = o
    for name in LINK_FILES:
        o = load(name)
        links[o["object_id"]] = o
    return objects, links


def collect(objects, links, member_ids, created_claim):
    """Curator-owned collection: a collection object plus one `collect` link per member."""
    objects[COLLECTION_ID] = {
        "object_id": COLLECTION_ID,
        "author": CURATOR,
        "created_claim": created_claim,
        "kind": "collection",
        "body": {"title": "Saturday meetup, all in one place"},
        "schema_version": 1,
    }
    for mid in member_ids:
        lid = f"lnk-collect-{mid}"
        links[lid] = {
            "object_id": lid,
            "author": CURATOR,
            "created_claim": created_claim,
            "verb": "collect",
            "source": COLLECTION_ID,
            "target": mid,
            "schema_version": 1,
        }
    return objects, links


def backlinks(links, target_id, verb=None):
    """Derived index: links whose target is target_id (what links here)."""
    return sorted(
        (
            l
            for l in links.values()
            if l["target"] == target_id and (verb is None or l["verb"] == verb)
        ),
        key=lambda l: l["object_id"],
    )


def neighbourhood(objects, links, object_id):
    """Everything one hop away over any verb, in either direction."""
    out = []
    for l in links.values():
        if l["source"] == object_id:
            out.append((l["verb"], "out", l["target"]))
        elif l["target"] == object_id:
            out.append((l["verb"], "in", l["source"]))
    return sorted(out)


def recent_changes(objects, links, limit=10):
    items = list(objects.values()) + list(links.values())
    items.sort(key=lambda o: (o["created_claim"], o["object_id"]), reverse=True)
    return [o["object_id"] for o in items[:limit]]


def collection_members(links, collection_id):
    return sorted(
        l["target"]
        for l in links.values()
        if l["verb"] == "collect" and l["source"] == collection_id
    )


def verbs_in(links):
    return {l["verb"] for l in links.values()}
