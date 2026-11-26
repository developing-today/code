"""Done check for PoC 1 (#14037 slice 1): python3 apps/id-graph-poc/check.py"""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from organise import (  # noqa: E402
    CURATOR,
    COLLECTION_ID,
    POC_VERBS,
    backlinks,
    build_graph,
    collect,
    collection_members,
    neighbourhood,
    recent_changes,
    verbs_in,
)

failures = []


def expect(cond, msg):
    if cond:
        print(f"OK: {msg}")
    else:
        failures.append(msg)
        print(f"FAIL: {msg}")


objects, links = build_graph()
members = sorted(objects)
objects, links = collect(objects, links, members, "2026-11-24T02:00:00Z")

authors = {o["author"] for o in objects.values() if o["kind"] != "collection"}
expect(CURATOR not in authors, "curator authored none of the collected objects")
expect(
    collection_members(links, COLLECTION_ID) == sorted(members),
    "collection holds every D3 fixture object via collect links",
)
expect(
    verbs_in(links) <= POC_VERBS,
    "every link verb is in the D3 registry plus the provisional collect verb",
)
expect(
    all(l["author"] == CURATOR for l in links.values() if l["verb"] == "collect"),
    "collect links are authored by the curator, not by the members' authors",
)

replies = backlinks(links, "post-meetup-thread", "reply")
expect(
    [l["source"] for l in replies] == ["msg-photo-answer"],
    "backlinks: what replies to the forum thread (cross-app chat reply)",
)
photo_in = {(l["verb"], l["source"]) for l in backlinks(links, "img-group-photo")}
expect(
    photo_in
    == {
        ("attach", "post-photo-published"),
        ("mention", "msg-photo-answer"),
        ("collect", COLLECTION_ID),
    },
    "backlinks: the photo is attached in the forum post, mentioned by the chat answer, and collected",
)

near = neighbourhood(objects, links, "msg-photo-answer")
expect(
    ("reply", "out", "post-meetup-thread") in near
    and ("nest", "out", "post-meetup-thread") in near,
    "neighbourhood: the chat answer's reply and nest links out to the thread",
)
expect(
    ("mention", "out", "img-group-photo") in near,
    "neighbourhood: the chat answer mentions the photo without containing it",
)
expect(
    ("quote", "in", "lnk-quote-chat-ask") not in near,
    "neighbourhood excludes links that do not touch the object",
)

recent = recent_changes(objects, links, limit=len(members) + 1)
expect(
    recent[0].startswith("lnk-collect-") and COLLECTION_ID in objects,
    "recent changes: the newest change is one of the curator's collect links",
)
expect(
    recent
    == sorted(
        recent,
        key=lambda i: (objects.get(i, links.get(i))["created_claim"], i),
        reverse=True,
    ),
    "recent changes are ordered newest first by (created_claim, object_id)",
)

print(f"\n{len(failures)} failure(s).")
sys.exit(1 if failures else 0)
