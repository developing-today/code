#!/usr/bin/env python3
"""D2 slice 4 reference: the host rendering contract, run as a checklist.

Stdlib only. Loads host_contract.json, runs every executable item against the
slice 1 reference renderer (embed_reference.py) over its fixtures plus a few
mutations, and checks that the list and the checks cannot drift apart: an
item with neither a check nor a manual procedure fails, and so does a check
no item names.

  python3 -I host_contract_reference.py check    # exit 0 when every item passes

A host does the same with its own adapter: load host_contract.json, take the
items with a check, and compare its render of each case with what these
checks require.
"""

import copy
import importlib.util
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
# Loading the slice 1 reference must not leave a __pycache__ in the record.
sys.dont_write_bytecode = True


def load_reference():
    spec = importlib.util.spec_from_file_location(
        "embed_reference", HERE / "embed_reference.py"
    )
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


ref = load_reference()
CONTRACT = json.loads((HERE / "host_contract.json").read_text(encoding="utf-8"))
GRAPH_DATA = json.loads((HERE / "graph.json").read_text(encoding="utf-8"))


def graph(data=None):
    return ref.Graph(copy.deepcopy(GRAPH_DATA if data is None else data))


def render(g, root, viewer):
    return ref.Renderer(g, viewer).obj(root, [], ref.DEFAULT_MAX_DEPTH)


def cases():
    """Every slice 1 case as (name, graph, root, viewer, render tree)."""
    out = []
    for name, root, viewer in ref.CASES:
        g = graph()
        out.append((name, g, root, viewer, render(g, root, viewer)))
    return out


def slots(node, depth=1):
    """Yield (slot, depth) for every embed slot under a rendered object."""
    for slot in node.get("embeds", []):
        yield slot, depth
        for member in slot.get("members", []):
            if "state" not in member:
                yield from slots(member, depth + 1)
        if "kind" in slot:
            yield from slots(slot, depth + 1)


def members(node):
    """Yield every collection member entry under a rendered object."""
    for slot, _ in slots(node):
        yield from slot.get("members", [])


def link_of(g, slot):
    return next(l for l in g.links if l["object_id"] == slot["link"])


def host_state(state):
    """What a host shows for a state it reads: unknown states are unavailable."""
    return state if state in CONTRACT["states"] else "unavailable"


# --- checks ---------------------------------------------------------------


def states_are_total():
    problems = []
    for name, _, _, _, tree in cases():
        for slot, _ in slots(tree):
            if slot.get("state") not in CONTRACT["states"]:
                problems.append(
                    f"{name}: slot {slot.get('link')} has state {slot.get('state')!r}"
                )
    if host_state("mirror") != "unavailable":
        problems.append("an unknown state is not shown as unavailable")
    return problems


def keys_problem(where, entry, spec):
    keys = set(entry)
    required, allowed = (
        set(spec["required"]),
        set(spec["required"]) | set(spec["optional"]),
    )
    missing, extra = required - keys, keys - allowed
    if missing or extra:
        return [f"{where}: missing {sorted(missing)}, extra {sorted(extra)}"]
    return []


def slot_shapes():
    problems = []
    for name, _, _, _, tree in cases():
        for slot, _ in slots(tree):
            spec = CONTRACT["slot_keys"][slot["state"]]
            problems += keys_problem(f"{name}: {slot['link']}", slot, spec)
        for m in members(tree):
            spec = CONTRACT["member_keys"]["cycle-cut" if "state" in m else "rendered"]
            problems += keys_problem(f"{name}: member {m.get('object')}", m, spec)
    return problems


def unavailable_is_opaque():
    problems, seen = [], 0
    for name, _, _, _, tree in cases():
        for slot, _ in slots(tree):
            if slot["state"] == "unavailable":
                seen += 1
                if set(slot) != {"link", "state"}:
                    problems.append(f"{name}: unavailable slot carries {sorted(slot)}")
    if seen < 2:
        problems.append("the fixtures should hold at least two unavailable slots")
    return problems


def unavailable_causes_match():
    """Unauthorised, not stored and pruned give the same slot, link aside."""
    by_cause = {}
    g = graph()
    tree = render(g, "post-budget-pointer", "ana")
    by_cause["unauthorised"] = tree["embeds"][0]

    g = graph()
    target = g.links_from("post-budget-pointer", "embed")[0]["target"]
    del g.objects[target]
    tree = render(g, "post-budget-pointer", "bo")
    by_cause["not stored"] = tree["embeds"][0]

    g = graph()
    tree = render(g, "post-old-ref", "anon")
    by_cause["pruned"] = tree["embeds"][0]

    stripped = {
        cause: {k: v for k, v in s.items() if k != "link"}
        for cause, s in by_cause.items()
    }
    if any(s != {"state": "unavailable"} for s in stripped.values()):
        return [f"causes differ: {stripped}"]
    return []


CUTS = {"cycle-cut", "depth-cut", "budget-cut"}


def cuts_name_readable_targets():
    problems = []
    for name, g, _, viewer, tree in cases():
        entries = [s for s, _ in slots(tree)] + list(members(tree))
        for e in entries:
            if e.get("state") in CUTS and not g.can_read(viewer, e["object"]):
                problems.append(f"{name}: {e['state']} names unreadable {e['object']}")
    return problems


def readable_members(g, link, viewer):
    e = link["embed"]
    if e["mode"] == "live":
        return g.members(link["target"], viewer)
    return [m for m in e["pin"]["members"] if g.can_read(viewer, m)]


def members_and_more_are_readable():
    problems = []
    for name, g, _, viewer, tree in cases():
        for slot, _ in slots(tree):
            if slot["state"] != "shown" or slot["scope"] != "collection":
                continue
            readable = readable_members(g, link_of(g, slot), viewer)
            shown = [m["object"] for m in slot["members"]]
            hidden = [m for m in shown if not g.can_read(viewer, m)]
            if hidden:
                problems.append(f"{name}: {slot['link']} shows unreadable {hidden}")
            if len(shown) + slot.get("more", 0) > len(readable):
                problems.append(
                    f"{name}: {slot['link']} counts more than the viewer can read"
                )
    return problems


def revocation_takes_effect():
    g = graph()
    before = render(g, "post-budget-pointer", "bo")["embeds"][0]
    target = g.links_from("post-budget-pointer", "embed")[0]["target"]
    g.areas[g.objects[target]["area"]] = []
    after = render(g, "post-budget-pointer", "bo")["embeds"][0]
    if before["state"] != "shown" or after != {
        "link": before["link"],
        "state": "unavailable",
    }:
        return [f"before {before['state']}, after {after}"]
    return []


def snapshot_shows_pin():
    problems, seen = [], 0
    for name, g, _, _, tree in cases():
        for slot, _ in slots(tree):
            if (
                slot["state"] == "shown"
                and slot["mode"] == "snapshot"
                and slot["scope"] == "object"
            ):
                seen += 1
                pin = link_of(g, slot)["embed"]["pin"]["object"]
                if slot["object"] != pin:
                    problems.append(
                        f"{name}: {slot['link']} shows {slot['object']}, pin {pin}"
                    )
    return problems if seen else ["no shown snapshot object slot in the fixtures"]


def live_shows_head():
    problems, seen = [], 0
    for name, g, _, viewer, tree in cases():
        for slot, _ in slots(tree):
            if (
                slot["state"] == "shown"
                and slot["mode"] == "live"
                and slot["scope"] == "object"
            ):
                seen += 1
                head, _ = g.head(link_of(g, slot)["target"], viewer)
                if slot["object"] != head:
                    problems.append(
                        f"{name}: {slot['link']} shows {slot['object']}, head {head}"
                    )
    return problems if seen else ["no shown live object slot in the fixtures"]


def badges_are_known():
    problems = []
    for name, _, _, _, tree in cases():
        for slot, _ in slots(tree):
            badges = slot.get("badges", [])
            if badges and slot["state"] != "shown":
                problems.append(
                    f"{name}: {slot['link']} has badges while {slot['state']}"
                )
            unknown = set(badges) - set(CONTRACT["badges"])
            if unknown or len(set(badges)) != len(badges):
                problems.append(f"{name}: {slot['link']} badges {badges}")
    return problems


def stress_graphs():
    """A deep chain and a wide collection, both public."""
    base = {
        "areas": {"pub": ["*"]},
        "authors": {"ed25519:stress": "sam"},
        "pruned": [],
    }
    obj = lambda i, kind="note": {
        "object_id": i,
        "area": "pub",
        "author": "ed25519:stress",
        "created_claim": "2026-11-25T00:00:00Z",
        "kind": kind,
    }
    embed = lambda src, dst, e: {
        "object_id": f"lnk-{src}-{dst}",
        "author": "ed25519:stress",
        "created_claim": "2026-11-25T00:00:01Z",
        "verb": "embed",
        "source": src,
        "target": dst,
        "embed": e,
    }
    chain = dict(base, objects=[obj(f"n{i}") for i in range(10)])
    chain["links"] = [
        embed(f"n{i}", f"n{i + 1}", {"mode": "live", "scope": "object"})
        for i in range(9)
    ]
    wide = dict(base, objects=[obj("root"), obj("box", "collection")])
    wide["objects"] += [obj(f"m{i:03}", "image") for i in range(200)]
    wide["links"] = [
        {
            "object_id": f"lnk-nest-{i:03}",
            "author": "ed25519:stress",
            "created_claim": f"2026-11-25T00:{i // 60:02}:{i % 60:02}Z",
            "verb": "nest",
            "source": f"m{i:03}",
            "target": "box",
        }
        for i in range(200)
    ]
    wide["links"].append(
        embed(
            "root",
            "box",
            {"mode": "live", "scope": "collection", "bounds": {"max_count": 200}},
        )
    )
    return [("chain", chain, "n0"), ("wide", wide, "root")]


def object_count(node):
    return 1 + sum(
        object_count(child)
        for slot in node.get("embeds", [])
        for child in (
            [slot]
            if "kind" in slot
            else [m for m in slot.get("members", []) if "kind" in m]
        )
    )


def render_is_bounded():
    problems = []
    renders = [(name, tree) for name, _, _, _, tree in cases()]
    for name, data, root in stress_graphs():
        renders.append((name, render(ref.Graph(data), root, "anon")))
    for name, tree in renders:
        if object_count(tree) > ref.RENDER_BUDGET:
            problems.append(f"{name}: {object_count(tree)} objects over the budget")
        # Shown slots nest at most the default depth; the one level below
        # holds only the stubs that say the bound cut there.
        for slot, depth in slots(tree):
            if depth > ref.DEFAULT_MAX_DEPTH + 1 or (
                depth > ref.DEFAULT_MAX_DEPTH
                and slot["state"] not in CUTS | {"unavailable"}
            ):
                problems.append(f"{name}: {slot['state']} slot nests {depth} deep")
    chain = dict(renders)["chain"]
    if not any(s["state"] == "depth-cut" for s, _ in slots(chain)):
        problems.append("chain: the depth bound never cut")
    wide = dict(renders)["wide"]["embeds"][0]
    if len(wide["members"]) + wide.get("more", 0) != 200:
        problems.append(
            "wide: shown plus more does not account for every readable member"
        )
    return problems


def max_count_holds():
    problems = []
    for name, g, _, _, tree in cases():
        for slot, _ in slots(tree):
            if slot["state"] == "shown" and slot["scope"] == "collection":
                cap = link_of(g, slot)["embed"]["bounds"]["max_count"]
                if len(slot["members"]) > cap:
                    problems.append(
                        f"{name}: {slot['link']} shows over max_count {cap}"
                    )
    return problems


def states_ignore_bytes():
    data = copy.deepcopy(GRAPH_DATA)
    for o in data["objects"]:
        o.pop("body", None)
    problems = []
    for name, root, viewer in ref.CASES:
        if render(graph(data), root, viewer) != render(graph(), root, viewer):
            problems.append(f"{name}: withholding bytes changed the render")
    return problems


CHECKS = {
    f.__name__: f
    for f in [
        states_are_total,
        slot_shapes,
        unavailable_is_opaque,
        unavailable_causes_match,
        cuts_name_readable_targets,
        members_and_more_are_readable,
        revocation_takes_effect,
        snapshot_shows_pin,
        live_shows_head,
        badges_are_known,
        render_is_bounded,
        max_count_holds,
        states_ignore_bytes,
    ]
}


def main(argv):
    if (argv[1] if len(argv) > 1 else "check") != "check":
        print(__doc__)
        return 2
    failures, executable, manual = [], 0, 0
    named = set()
    for item in CONTRACT["items"]:
        has_check, has_manual = "check" in item, bool(item.get("manual"))
        if has_check == has_manual:
            failures.append(f"{item['id']}: needs exactly one of check or manual")
            continue
        if has_manual:
            manual += 1
            continue
        executable += 1
        named.add(item["check"])
        check = CHECKS.get(item["check"])
        if check is None:
            failures.append(f"{item['id']}: no check named {item['check']}")
            continue
        failures += [f"{item['id']}: {p}" for p in check()]
    for unused in sorted(set(CHECKS) - named):
        failures.append(f"check {unused} is named by no item")
    for f in failures:
        print(f"FAIL {f}")
    total = len(CONTRACT["items"])
    print(
        f"ok: {total} items, {executable} executable, {manual} manual, {len(failures)} failures"
        if not failures
        else f"{total} items, {executable} executable, {manual} manual, {len(failures)} failures"
    )
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
