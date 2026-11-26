#!/usr/bin/env python3
"""D2 slice 2 reference: the capability matrix and delegated view grants.

Stdlib only. Loads the slice 1 graph plus the slice 2 overlay
(view_grants.json), then runs every check that capability_matrix.json names:
one per matrix cell and one per worked attack. The list and the checks cannot
drift apart: a cell or attack naming an unknown check fails, and so does a
check that nothing names.

  python3 -I capability_reference.py check    # exit 0 when every check passes

Delegated view grants are specified here but are off by default (record §7,
open question 4). The reference renders with them only when a check turns
them on, and one check proves that with the switch off every grant is
ignored.
"""

import copy
import importlib.util
import json
import sys
from datetime import datetime, timedelta
from pathlib import Path

HERE = Path(__file__).resolve().parent
# Loading the slice 1 reference must not leave a __pycache__ in the record.
sys.dont_write_bytecode = True


def load_reference():
    spec = importlib.util.spec_from_file_location(
        "embed_reference", HERE / "embed_reference.py"
    )
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


ref = load_reference()
BASE = json.loads((HERE / "graph.json").read_text(encoding="utf-8"))
OVERLAY = json.loads((HERE / "view_grants.json").read_text(encoding="utf-8"))
MATRIX = json.loads((HERE / "capability_matrix.json").read_text(encoding="utf-8"))
OUTSIDER = "ana"


def when(text):
    return datetime.fromisoformat(text.replace("Z", "+00:00"))


def data():
    """A fresh copy of the slice 1 graph with the slice 2 overlay added."""
    d = copy.deepcopy(BASE)
    d["objects"] += copy.deepcopy(OVERLAY["objects"])
    d["links"] += copy.deepcopy(OVERLAY["links"])
    return d


def link(d, link_id):
    return next(l for l in d["links"] if l["object_id"] == link_id)


def obj(d, object_id):
    return next(o for o in d["objects"] if o["object_id"] == object_id)


def everyone_but(d, name):
    return sorted(set(d["authors"].values()) - {name}) + ["anon"]


# --- create rules -----------------------------------------------------------


def create_problems(g):
    """Slice 1's create rules plus the slice 2 pin rule: a collection pin
    holds only members the embedder could read, because the pin's member ids
    are readable by everyone who reads the source."""
    problems = ref.check_links(g)
    for l in g.links:
        e = l.get("embed") or {}
        if l["verb"] != "embed" or e.get("scope") != "collection":
            continue
        pin = e.get("pin") or {}
        embedder = g.authors[l["author"]]
        for m in pin.get("members", []):
            if m in g.objects and not g.can_read(embedder, m):
                problems.append(
                    f"{l['object_id']}: pin names {m}, which the embedder cannot read"
                )
    return problems


# --- grants -----------------------------------------------------------------


def grant_problems(g, grant, managers, max_days):
    """Why a grant is invalid, or [] when it is valid (record §7.3)."""
    out = []
    gid = grant.get("grant_id", "?")
    l = next((x for x in g.links if x["object_id"] == grant.get("link")), None)
    if l is None or l["verb"] != "embed":
        return [f"{gid}: names no embed link"]
    e = l["embed"]
    target = g.objects.get(l["target"])
    if target is None:
        return [f"{gid}: target is not stored"]
    issuer = g.authors.get(grant.get("issuer"))
    if issuer not in managers.get(target["area"], []):
        out.append(f"{gid}: issuer does not manage {target['area']}")
    if e["mode"] != "snapshot":
        out.append(f"{gid}: a live embed takes no grant")
    else:
        pin = e["pin"]
        if e["scope"] == "object":
            want = {"object": pin["object"]}
        else:
            want = {"root": pin["root"], "members_digest": pin["members_digest"]}
        if grant.get("covers") != want:
            out.append(f"{gid}: covers differs from the embed's pin")
    if grant.get("rights") != ["read"]:
        out.append(f"{gid}: a grant carries read and nothing else")
    source = g.objects.get(l["source"])
    if source is None or grant.get("area") != source["area"]:
        out.append(f"{gid}: a grant is stored in its source's area")
    start, end = when(grant["not_before"]), when(grant["not_after"])
    if end <= start:
        out.append(f"{gid}: empty window")
    elif end - start > timedelta(days=max_days):
        out.append(f"{gid}: window longer than {max_days} days")
    if when(grant["issued_claim"]) < when(l["created_claim"]):
        out.append(f"{gid}: issued before its embed existed")
    return out


class GrantRenderer(ref.Renderer):
    """The slice 1 renderer plus delegated views. With `enabled` false it
    renders exactly as slice 1 does."""

    def __init__(self, g, viewer, grants, managers, now, max_days, enabled):
        super().__init__(g, viewer)
        self.enabled = enabled
        self.grants = {}
        for gr in grants:
            if grant_problems(g, gr, managers, max_days):
                continue
            if when(gr["not_before"]) <= now < when(gr["not_after"]):
                self.grants[gr["link"]] = gr

    def embed(self, l, path, depth_left):
        gr = self.grants.get(l["object_id"]) if self.enabled else None
        if gr is None or self.g.can_read(self.viewer, l["target"]):
            # The viewer's own right always wins; a grant never adds to it.
            return super().embed(l, path, depth_left)
        e = l["embed"]
        depth_left = min(
            depth_left, e.get("bounds", {}).get("max_depth", ref.DEFAULT_MAX_DEPTH)
        )
        base = {"link": l["object_id"], "mode": e["mode"], "scope": e["scope"]}
        target = l["target"]
        if target in path:
            return {**base, "state": "cycle-cut", "object": target}
        if depth_left <= 0:
            return {**base, "state": "depth-cut", "object": target}
        if self.budget <= 0:
            return {**base, "state": "budget-cut", "object": target}
        via = {"via": gr["grant_id"]}
        if e["scope"] == "object":
            shown = e["pin"]["object"]
            # No badges: a grant covers the pin, not the target's history.
            return {
                **base,
                "state": "shown",
                **via,
                **self.obj(shown, path, depth_left - 1),
            }
        members = e["pin"]["members"]
        cap = e["bounds"]["max_count"]
        rendered = [self.obj(m, path + [target], depth_left - 1) for m in members[:cap]]
        node = {**base, "state": "shown", **via, "object": target, "members": rendered}
        if len(members) > cap:
            node["more"] = len(members) - cap
        return node


def render(d, root, viewer, *, enabled=False, grants=None, now=None, max_days=None):
    """Render `root` for `viewer`. Returns None when the viewer cannot read the
    root: traversal starts from what the viewer can read (matrix 'traverse')."""
    g = ref.Graph(d)
    if not g.can_read(viewer, root):
        return None
    r = GrantRenderer(
        g,
        viewer,
        OVERLAY["grants"] if grants is None else grants,
        OVERLAY["managers"],
        when(OVERLAY["render_at"]) if now is None else now,
        OVERLAY["max_window_days"] if max_days is None else max_days,
        enabled,
    )
    return r.obj(root, [], ref.DEFAULT_MAX_DEPTH)


def slot(tree, link_id):
    """The embed node for `link_id` anywhere in a render tree."""
    stack = [tree]
    while stack:
        n = stack.pop()
        if n is None:
            continue
        if n.get("link") == link_id:
            return n
        stack += n.get("embeds", []) + n.get("members", [])
    return None


def fixture_links():
    return MATRIX["fixture_links"]


# --- checks: matrix cells ---------------------------------------------------


def create_needs_target_read():
    out = []
    for kind, lid in fixture_links().items():
        d = data()
        l = link(d, lid)
        embedder = d["authors"][l["author"]]
        area = obj(d, l["target"])["area"]
        d["areas"][area] = everyone_but(d, embedder)
        if not any(
            "cannot read the target" in p for p in create_problems(ref.Graph(d))
        ):
            out.append(f"{kind}: embed accepted from an embedder who cannot read")
    return out


def pin_holds_only_readable_members():
    d = data()
    l = link(d, "lnk-embed-thread-snapshot")
    members = l["embed"]["pin"]["members"] + ["img-p1"]
    l["embed"]["pin"]["members"] = members
    l["embed"]["pin"]["members_digest"] = ref.members_digest(members)
    probs = create_problems(ref.Graph(d))
    if not any("img-p1" in p and "cannot read" in p for p in probs):
        return ["a pin naming a member the embedder cannot read was accepted"]
    if create_problems(ref.Graph(data())):
        return ["the unmutated graph is refused"]
    return []


def source_unreadable_hides_embed():
    out = []
    for kind, lid in fixture_links().items():
        d = data()
        l = link(d, lid)
        src = obj(d, l["source"])
        d["areas"][src["area"]] = [d["authors"][src["author"]]]
        viewer = "anon" if d["authors"][src["author"]] != "anon" else OUTSIDER
        if render(d, l["source"], viewer) is not None:
            out.append(f"{kind}: source rendered for a viewer who cannot read it")
    return out


def slot_needs_own_read():
    out = []
    for kind, lid in fixture_links().items():
        d = data()
        l = link(d, lid)
        owner = d["authors"][obj(d, l["target"])["author"]]
        embedder = d["authors"][l["author"]]
        area = obj(d, l["target"])["area"]
        d["areas"][area] = sorted({owner, embedder})
        viewer = next(
            v for v in ["anon", "ana", "bo", "cyp", "dee"] if v not in d["areas"][area]
        )
        if not ref.Graph(d).can_read(viewer, l["source"]):
            continue
        n = slot(render(d, l["source"], viewer), lid)
        if n != {"link": lid, "state": "unavailable"}:
            out.append(f"{kind}: slot for {viewer} is {n}")
    # Member level: an unreadable member is neither shown nor counted.
    for viewer in ["ana", "cyp"]:
        d = data()
        g = ref.Graph(d)
        n = slot(render(d, "page-meetup", viewer), "lnk-embed-album-live")
        shown = [m["object"] for m in n["members"]]
        if any(not g.can_read(viewer, m) for m in shown):
            out.append(f"{viewer}: an unreadable member is shown")
        if len(shown) + n.get("more", 0) != len(g.members("album-meetup", viewer)):
            out.append(f"{viewer}: shown plus more is not the readable count")
    return out


def embedder_rights_do_not_flow():
    out = []
    for kind, lid in fixture_links().items():
        d = data()
        l = link(d, lid)
        embedder = d["authors"][l["author"]]
        root = l["source"]
        viewers = [v for v in ["anon", "ana", "bo", "cyp", "dee"] if v != embedder]
        before = {v: render(d, root, v) for v in viewers}
        area = obj(d, l["target"])["area"]
        if "*" in d["areas"][area]:
            d["areas"][area] = everyone_but(d, embedder)
        else:
            d["areas"][area] = [x for x in d["areas"][area] if x != embedder]
        for v in viewers:
            if render(d, root, v) != before[v]:
                out.append(f"{kind}: {v}'s render changed when the embedder lost read")
    return out


def nested_embeds_use_own_rights():
    tree = render(data(), "post-plan-public", OUTSIDER, enabled=True)
    outer = slot(tree, "lnk-embed-plan-snapshot")
    inner = slot(tree, "lnk-embed-plan-budget")
    if outer is None or outer.get("via") != "grant-plan":
        return [f"granted slot not shown: {outer}"]
    if inner != {"link": "lnk-embed-plan-budget", "state": "unavailable"}:
        return [f"inner private embed reached through a grant: {inner}"]
    return []


def revocation_empties_slot():
    out = []
    for kind, lid in fixture_links().items():
        d = data()
        l = link(d, lid)
        before = slot(render(d, l["source"], "anon"), lid)
        if before is None or before.get("state") != "shown":
            out.append(f"{kind}: not shown before revocation: {before}")
            continue
        area = obj(d, l["target"])["area"]
        d["areas"][area] = [d["authors"][obj(d, l["target"])["author"]]]
        after = slot(render(d, l["source"], "anon"), lid)
        if after != {"link": lid, "state": "unavailable"}:
            out.append(f"{kind}: still {after} after revocation")
    return out


def grant_issuer_must_manage():
    gr = copy.deepcopy(OVERLAY["grants"])
    gr[0]["issuer"] = "ed25519:aaaa1111chat"
    g = ref.Graph(data())
    if not grant_problems(g, gr[0], OVERLAY["managers"], OVERLAY["max_window_days"]):
        return ["a grant from a non-manager was accepted"]
    n = slot(
        render(data(), "post-plan-public", OUTSIDER, enabled=True, grants=gr),
        "lnk-embed-plan-snapshot",
    )
    if n != {"link": "lnk-embed-plan-snapshot", "state": "unavailable"}:
        return [f"a non-manager's grant filled the slot: {n}"]
    return []


def grant_scope_is_exact():
    g = ref.Graph(data())
    good = OVERLAY["grants"][0]
    m, days = OVERLAY["managers"], OVERLAY["max_window_days"]
    out = []
    bad = {
        "live embed": {"link": "lnk-embed-budget-live", "area": "forum-public"},
        "other object": {"covers": {"object": "doc-bo-budget"}},
        "whole area": {"covers": {"area": "bo-private"}},
        "write right": {"rights": ["read", "write"]},
        "stored elsewhere": {"area": "bo-private"},
    }
    for label, change in bad.items():
        if not grant_problems(g, {**good, **change}, m, days):
            out.append(f"grant accepted: {label}")
    col = OVERLAY["grants"][1]
    widened = {**col, "covers": {**col["covers"], "members_digest": "sha256:00"}}
    if not grant_problems(g, widened, m, days):
        out.append("grant accepted: different member set")
    for gr in OVERLAY["grants"]:
        if grant_problems(g, gr, m, days):
            out.append(f"fixture grant refused: {grant_problems(g, gr, m, days)}")
    return out


def grant_bound_to_one_link():
    d = data()
    d["objects"].append(
        {
            "object_id": "post-plan-repost",
            "area": "forum-public",
            "author": "ed25519:bbbb2222forum",
            "kind": "forum-post",
            "created_claim": "2026-11-24T07:00:00Z",
        }
    )
    second = copy.deepcopy(link(d, "lnk-embed-plan-snapshot"))
    second.update(
        object_id="lnk-embed-plan-repost",
        source="post-plan-repost",
        created_claim="2026-11-24T07:00:00Z",
    )
    d["links"].append(second)
    if create_problems(ref.Graph(d)):
        return ["the second embed itself is invalid"]
    n = slot(render(d, "post-plan-repost", OUTSIDER, enabled=True), second["object_id"])
    if n != {"link": "lnk-embed-plan-repost", "state": "unavailable"}:
        return [f"a grant filled a second embed's slot: {n}"]
    return []


def grant_shows_pinned_object():
    out = []
    d = data()
    n = slot(
        render(d, "post-plan-public", OUTSIDER, enabled=True), "lnk-embed-plan-snapshot"
    )
    want = {
        "link": "lnk-embed-plan-snapshot",
        "mode": "snapshot",
        "scope": "object",
        "state": "shown",
        "via": "grant-plan",
        "object": "doc-bo-plan",
        "kind": "note",
    }
    if {k: v for k, v in (n or {}).items() if k != "embeds"} != want:
        out.append(f"object grant slot: {n}")
    n = slot(
        render(d, "post-album-public", OUTSIDER, enabled=True),
        "lnk-embed-private-album-snapshot",
    )
    if n is None or [m["object"] for m in n.get("members", [])] != ["img-p1", "img-p2"]:
        out.append(f"collection grant slot: {n}")
    elif n.get("via") != "grant-private-album" or "more" in n or "badges" in n:
        out.append(f"collection grant slot carries extra keys: {sorted(n)}")
    # The owner reads by their own right: no `via`.
    n = slot(
        render(d, "post-plan-public", "bo", enabled=True), "lnk-embed-plan-snapshot"
    )
    if n is None or "via" in n:
        out.append(f"owner's slot names a grant: {n}")
    return out


def grant_expires():
    out = []
    lid = "lnk-embed-plan-snapshot"
    gone = {"link": lid, "state": "unavailable"}
    for label, now in {
        "after not_after": when("2026-11-30T00:00:00Z"),
        "before not_before": when("2026-11-24T05:00:00Z"),
    }.items():
        n = slot(
            render(data(), "post-plan-public", OUTSIDER, enabled=True, now=now), lid
        )
        if n != gone:
            out.append(f"{label}: {n}")
    long = copy.deepcopy(OVERLAY["grants"])
    long[0]["not_after"] = "2026-12-24T00:00:00Z"
    n = slot(
        render(data(), "post-plan-public", OUTSIDER, enabled=True, grants=long), lid
    )
    if n != gone:
        out.append(f"over-long window accepted: {n}")
    return out


# --- checks: attacks only ---------------------------------------------------


def leak_without_grant():
    out = []
    n = slot(render(data(), "post-budget-pointer", OUTSIDER), "lnk-embed-budget-live")
    if n != {"link": "lnk-embed-budget-live", "state": "unavailable"}:
        out.append(f"slice 1 leak case: {n}")
    n = slot(
        render(data(), "post-plan-public", OUTSIDER, enabled=True, grants=[]),
        "lnk-embed-plan-snapshot",
    )
    if n != {"link": "lnk-embed-plan-snapshot", "state": "unavailable"}:
        out.append(f"ungranted snapshot of a private note: {n}")
    return out


def grant_only_fills_its_slot():
    if render(data(), "doc-bo-plan", OUTSIDER, enabled=True) is not None:
        return ["the granted object rendered as a root for an outsider"]
    return []


def grants_off_by_default():
    out = []
    for root, lid in [
        ("post-plan-public", "lnk-embed-plan-snapshot"),
        ("post-album-public", "lnk-embed-private-album-snapshot"),
    ]:
        n = slot(render(data(), root, OUTSIDER), lid)
        if n != {"link": lid, "state": "unavailable"}:
            out.append(f"{lid} shown with grants switched off: {n}")
    # With the switch off, slice 1's own render trees are unchanged.
    g = ref.Graph(BASE)
    for name, root, viewer in ref.CASES:
        if render(copy.deepcopy(BASE), root, viewer) != ref.render_case(
            g, root, viewer
        ):
            out.append(f"case {name} differs from slice 1")
    return out


CHECKS = {
    f.__name__: f
    for f in [
        create_needs_target_read,
        pin_holds_only_readable_members,
        source_unreadable_hides_embed,
        slot_needs_own_read,
        embedder_rights_do_not_flow,
        nested_embeds_use_own_rights,
        revocation_empties_slot,
        grant_issuer_must_manage,
        grant_scope_is_exact,
        grant_bound_to_one_link,
        grant_shows_pinned_object,
        grant_expires,
        leak_without_grant,
        grant_only_fills_its_slot,
        grants_off_by_default,
    ]
}


def main(argv):
    if (argv[1] if len(argv) > 1 else "check") != "check":
        print(__doc__)
        return 2
    problems = []
    named = set()
    kinds = set(MATRIX["kinds"])
    if set(fixture_links()) != kinds:
        problems.append("fixture_links does not name every kind")
    covered = set()
    for c in MATRIX["cells"]:
        covered |= kinds if c["kinds"] == "all" else set(c["kinds"])
        if not set(c["kinds"] if c["kinds"] != "all" else []) <= kinds:
            problems.append(f"cell {c['phase']}/{c['party']}: unknown kind")
    if covered != kinds:
        problems.append(f"kinds with no cell: {sorted(kinds - covered)}")
    entries = [("cell " + c["phase"] + "/" + c["party"], c) for c in MATRIX["cells"]]
    entries += [("attack " + a["id"], a) for a in MATRIX["attacks"]]
    for label, entry in entries:
        name = entry.get("check")
        if name not in CHECKS:
            problems.append(f"{label}: unknown check {name!r}")
        named.add(name)
    for name in sorted(set(CHECKS) - named):
        problems.append(f"check {name} is named by no cell or attack")
    if create_problems(ref.Graph(data())):
        problems += [f"overlay: {p}" for p in create_problems(ref.Graph(data()))]
    for name, fn in CHECKS.items():
        problems += [f"{name}: {p}" for p in fn()]
    for p in problems:
        print(f"FAIL: {p}")
    if problems:
        return 1
    print(
        f"ok: {len(MATRIX['cells'])} cells, {len(MATRIX['attacks'])} attacks, "
        f"{len(CHECKS)} checks, 0 failures"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
