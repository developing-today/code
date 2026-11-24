"""D3 fixture checker (stdlib only): validates the doc/ fixtures against the
registry rules stated in D3-graph-layer.md §2.4 / §5 / §6.

Checks:
  1. every content fixture has the required ContentObject fields and a v1 kind
  2. every link fixture has the required LinkObject fields and a v1 verb
  3. verb-specific structure: reply requires context{root,previous[]};
     quote requires snapshot{digest}
  4. referential integrity: every link source/target resolves to a fixture
     object_id within the fixture set
  5. the four acceptance fixtures are present: a chat-message, a forum-post,
     an image, and a cross-app reply chain (reply link whose source and
     target have different authors)

Usage: python3 -I fixtures/check.py   (exit 0 = all pass)
"""
import json
import sys
from pathlib import Path

FIX = Path(__file__).resolve().parent
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
KINDS = {"chat-message", "forum-post", "image"}
VERBS = {"reply", "mention", "quote", "attach", "supersede", "nest"}
CONTENT_REQUIRED = {"object_id", "author", "created_claim", "kind", "body", "schema_version"}
LINK_REQUIRED = {"object_id", "author", "created_claim", "verb", "source", "target", "schema_version"}

failures = []


def fail(msg):
    failures.append(msg)
    print(f"FAIL: {msg}")


def load(name):
    with open(FIX / name, encoding="utf-8") as f:
        return json.load(f)


def main():
    objects = {}
    for name in CONTENT_FILES:
        try:
            o = load(name)
        except (OSError, json.JSONDecodeError) as e:
            fail(f"{name}: unreadable ({e})")
            continue
        missing = CONTENT_REQUIRED - set(o)
        if missing:
            fail(f"{name}: missing fields {sorted(missing)}")
        if o.get("kind") not in KINDS:
            fail(f"{name}: kind {o.get('kind')!r} not in v1 registry {sorted(KINDS)}")
        if not isinstance(o.get("body"), dict):
            fail(f"{name}: body must be an object")
        if o.get("kind") == "image":
            for k in ("digest", "size", "mime"):
                if k not in o.get("body", {}):
                    fail(f"{name}: image body missing {k}")
        objects[o.get("object_id", name)] = (name, o)

    links = []
    for name in LINK_FILES:
        try:
            o = load(name)
        except (OSError, json.JSONDecodeError) as e:
            fail(f"{name}: unreadable ({e})")
            continue
        missing = LINK_REQUIRED - set(o)
        if missing:
            fail(f"{name}: missing fields {sorted(missing)}")
        if o.get("verb") not in VERBS:
            fail(f"{name}: verb {o.get('verb')!r} not in v1 registry {sorted(VERBS)}")
        if o.get("verb") == "reply":
            ctx = o.get("context")
            if not isinstance(ctx, dict) or "root" not in ctx or not isinstance(ctx.get("previous"), list):
                fail(f"{name}: reply link requires context{{root, previous[]}}")
        if o.get("verb") == "quote":
            snap = o.get("snapshot")
            if not isinstance(snap, dict) or not snap.get("digest"):
                fail(f"{name}: quote link requires snapshot{{digest}}")
        for end in ("source", "target"):
            if o.get(end) not in objects:
                fail(f"{name}: {end} {o.get(end)!r} resolves to no fixture object")
        objects[o.get("object_id", name)] = (name, o)
        links.append((name, o))

    # Acceptance: the four fixture shapes from #13885.
    kinds_present = {o.get("kind") for _, o in objects.values() if "kind" in o}
    for need in ("chat-message", "forum-post", "image"):
        if need not in kinds_present:
            fail(f"acceptance: no {need} fixture present")
    chain = False
    for name, o in links:
        if o.get("verb") != "reply":
            continue
        src = objects.get(o["source"])
        tgt = objects.get(o["target"])
        if src and tgt and src[1].get("author") != tgt[1].get("author"):
            chain = True
            print(f"OK: cross-app reply chain in {name}: "
                  f"{o['source']} ({src[1]['author']}) -> {o['target']} ({tgt[1]['author']})")
    if not chain:
        fail("acceptance: no cross-app reply chain (reply link across authors) found")

    # SSB annex presence is a record-level check, reported here for completeness.
    record = FIX.parent / "D3-graph-layer.md"
    text = record.read_text(encoding="utf-8").lower() if record.exists() else ""
    for needle in ("## 7. ssb", "single-device", "append-only", "hops", "capability scoping",
                   "first sync", "adopt", "reject"):
        if needle not in text:
            fail(f"acceptance: record missing SSB annex marker {needle!r}")

    print(f"\n{len(CONTENT_FILES)} content + {len(LINK_FILES)} link fixtures checked, "
          f"{len(failures)} failure(s).")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
