#!/usr/bin/env python3
"""D5 slice 4 reference: the conformance checklist, run against the references.

Stdlib only. Builds on view_reference.py, web_reference.py and
kitty_reference.py in this folder.

  python3 -I conformance_reference.py check         # run every executable item
  python3 -I conformance_reference.py write-floor   # regenerate floor_expected.json

conformance.json is the checklist of slice-4-conformance.md. Each item names
the surfaces it applies to and either a check here or, for an item only a
person or a browser can judge, a manual procedure. `check` fails when an item
has neither, when a check is missing, or when a check fails; it also compares
the floor lines of every case with floor_expected.json byte for byte.
"""

import itertools
import json
import math
import sys
import unicodedata
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import kitty_reference as kr
import view_reference as vr
import web_reference as wr

HERE = Path(__file__).resolve().parent
SURFACES = {"floor", "web", "kitty"}

# ---------------------------------------------------------------------------
# The floor renderer with its one addition in slice 4: label sanitising.

BIDI_CONTROLS = {chr(c) for c in (*range(0x202A, 0x202F), *range(0x2066, 0x206A))}


def sanitise(text):
    """Make a label safe to print: no control, bidi or line-breaking characters."""
    out = []
    for ch in text:
        if ch in BIDI_CONTROLS or unicodedata.category(ch) in ("Cc", "Zl", "Zp"):
            out.append("�")
        else:
            out.append(ch)
    return "".join(out)


def floor_lines(frame):
    """Slice 1's tree lines over sanitised labels."""
    clean = dict(
        frame,
        nodes=[dict(n, label=sanitise(n["label"])) for n in frame["nodes"]],
    )
    return vr.tree_lines(clean)


def no_tty_output(frame):
    return "\n".join(floor_lines(frame)) + "\n"


# ---------------------------------------------------------------------------
# Fixtures.


def load():
    graph = vr.Graph(json.loads((HERE / "graph.json").read_text()))
    return graph, vr.all_frames(graph)


def hostile_graph():
    """One public note whose title tries to drive the terminal and the reader."""
    return vr.Graph(
        {
            "areas": {"public": "public"},
            "objects": [
                {
                    "object_id": "h-1",
                    "kind": "note",
                    "author": "a",
                    "area": "public",
                    "created_claim": "2026-11-25T00:00:00Z",
                    "title": "ok\x1b]52;c;cGF3bmVk\x07\x1b[2J\nline two \u202ereversed",
                },
                {
                    "object_id": "h-2",
                    "kind": "note",
                    "author": "a",
                    "area": "public",
                    "created_claim": "2026-11-25T00:00:01Z",
                    "title": "plain",
                },
            ],
            "links": [
                {
                    "object_id": "h-l",
                    "verb": "reply",
                    "source": "h-2",
                    "target": "h-1",
                    "author": "a",
                    "area": "public",
                    "created_claim": "2026-11-25T00:00:02Z",
                }
            ],
        }
    )


# ---------------------------------------------------------------------------
# Checks. Each returns a list of problems; [] is a pass.


def frames_match_expected(graph, frames):
    expected = json.loads((HERE / "expected.json").read_text())
    got = vr.computed_expected(graph)
    return [f"{name} differs" for name in expected if expected[name] != got.get(name)]


def frame_invariants(graph, frames):
    return [f"{n}: {p}" for n, f in frames.items() for p in vr.invariants(graph, f)]


def unreadable_never_shown(graph, frames):
    owner = {n["id"] for n in frames["notes-owner-sees-private"]["nodes"]}
    other = frames["notes-replies-first"]
    problems = []
    if "n-private" not in owner:
        problems.append("the owner does not see their private note")
    if "n-private" in {n["id"] for n in other["nodes"]}:
        problems.append("a non-owner sees the private note")
    owner_more = {
        n["id"]: n["more"] for n in frames["notes-owner-sees-private"]["nodes"]
    }
    for node in other["nodes"]:
        if node["more"] != owner_more.get(node["id"], node["more"]):
            problems.append(f"{node['id']}: more count differs between viewers")
    return problems


def outside_targets_refused(graph, frames):
    frame = frames["notes-replies-first"]
    problems = []
    for intent in ("identify", "open", "focus", "identify-link", "inspect-link"):
        try:
            vr.respond(graph, frame, intent, "n-private")
            problems.append(f"{intent} on a node outside the frame was answered")
        except vr.QueryError as err:
            if str(err) != "target_not_in_frame":
                problems.append(f"{intent}: refused as {err}")
    return problems


def ring_and_angle_order(graph, frames):
    problems = []
    for name, frame in frames.items():
        points = wr.project(frame, 1280, 800)
        cx, cy = 640, 400
        rings = {}
        for node in frame["nodes"]:
            px, py = points[node["id"]]
            rings.setdefault(node["ring"], []).append(math.hypot(px - cx, py - cy))
        ordered = sorted(rings)
        for inner, outer in itertools.pairwise(ordered):
            if max(rings[inner]) >= min(rings[outer]):
                problems.append(f"{name}: ring {inner} not inside ring {outer}")
        for ring in ordered[1:]:
            on_ring = sorted(
                (n for n in frame["nodes"] if n["ring"] == ring),
                key=lambda n: n["angle"],
            )
            bearings = [
                math.atan2(points[n["id"]][0] - cx, cy - points[n["id"]][1])
                % (2 * math.pi)
                for n in on_ring
            ]
            if bearings != sorted(bearings):
                problems.append(f"{name}: clockwise order broken on ring {ring}")
    return problems


def keyboard_order_is_floor_order(graph, frames):
    problems = []
    for name, frame in frames.items():
        lines = floor_lines(frame)
        by_id = {n["id"]: n for n in frame["nodes"]}
        for index, node_id in enumerate(wr.keyboard_order(frame)):
            node = by_id[node_id]
            if f"{sanitise(node['label'])} [{node['kind']}]" not in lines[index]:
                problems.append(f"{name}: stop {index} is not line {index}")
                break
    return problems


def every_intent_from_keyboard(graph, frames):
    problems = []
    web_keys = {v for (device, _), v in wr.GESTURES.items() if device == "keyboard"}
    if web_keys != wr.INTENTS:
        problems.append(f"web keyboard misses {sorted(wr.INTENTS - web_keys)}")
    if set(kr.KEYS.values()) != wr.INTENTS:
        problems.append("terminal keyboard misses an intent")
    return problems


def plain_click_opens(graph, frames):
    problems = []
    if wr.GESTURES[("pointer", "click-node")] != "open":
        problems.append("web: plain click is not open")
    n = ("node", "a")
    press = {"kind": "press", "button": 0}
    release = {"kind": "release", "button": 0}
    got = kr.gestures([(0, press), (20, release)], [n, n], "hover")
    if got != [("open", "a")]:
        problems.append(f"terminal hover mode: click gave {got}")
    return problems


def focus_is_not_plain_click(graph, frames):
    focus = [g for g, v in wr.GESTURES.items() if v == "focus"]
    return [f"focus mapped to {g}" for g in focus if g == ("pointer", "click-node")]


def look_intents_keep_frame(graph, frames):
    frame = frames["notes-replies-first"]
    problems = []
    for node in frame["nodes"]:
        for intent in ("identify", "open"):
            if vr.respond(graph, frame, intent, node["id"])["frame_changes"]:
                problems.append(f"{intent} on {node['id']} changed the frame")
    for edge in frame["edges"]:
        for intent in ("identify-link", "inspect-link"):
            if vr.respond(graph, frame, intent, edge["id"])["frame_changes"]:
                problems.append(f"{intent} on {edge['id']} changed the frame")
    return problems


def claims_shown_as_claims(graph, frames):
    problems = []
    frame = frames["notes-replies-first"]
    for node in frame["nodes"]:
        shown = vr.respond(graph, frame, "identify", node["id"])["show"]
        if not shown["claim_untrusted"]:
            problems.append(f"{node['id']}: claimed time not marked")
    if not frames["notes-time-window"].get("claims_untrusted"):
        problems.append("a time-window frame does not say claims are untrusted")
    return problems


def edited_opens_head(graph, frames):
    frame = frames["notes-edited-and-linked-link"]
    problems = []
    for node in frame["nodes"]:
        nav = vr.respond(graph, frame, "open", node["id"])["navigate"]
        want = "head" if "edited" in node["badges"] else "obj"
        if nav["uri_shape"] != want:
            problems.append(f"{node['id']}: opens {nav['uri_shape']}, want {want}")
    return problems


def floor_matches_golden(graph, frames):
    expected = json.loads((HERE / "floor_expected.json").read_text())
    got = {name: floor_lines(frame) for name, frame in frames.items()}
    return [
        f"{name}: floor differs" for name in expected if expected[name] != got.get(name)
    ]


def floor_sanitises_labels(graph, frames):
    hostile = hostile_graph()
    frame = vr.build_frame(hostile, "a", {"focus": "h-2"})
    text = no_tty_output(frame)
    problems = []
    for ch in text[:-1]:
        if ch == "\n":
            continue
        if ch in BIDI_CONTROLS or unicodedata.category(ch) in ("Cc", "Zl", "Zp"):
            problems.append(f"control character {ord(ch):#x} reached the output")
    if text.count("\n") != len(floor_lines(frame)):
        problems.append("a label broke a floor line")
    return problems


def no_tty_has_no_escapes(graph, frames):
    problems = []
    for name, frame in frames.items():
        if "\x1b" in no_tty_output(frame):
            problems.append(f"{name}: escape byte in no-TTY output")
    surface, reason = kr.choose_surface({})
    if (surface, reason) != ("lines", "no_tty"):
        problems.append("no TTY does not choose plain lines")
    return problems


def fallback_triggers(graph, frames):
    problems = []
    for name, (profile, expected) in kr.PROFILES.items():
        got = (*kr.choose_surface(profile), kr.pointer_mode(profile))
        if got != expected:
            problems.append(f"{name}: {got} != {expected}")
    return problems


def teardown_restores(graph, frames):
    problems = []
    for name, (profile, _) in kr.PROFILES.items():
        modes = kr.session_modes(profile)
        out = kr.teardown(modes, (kr.IMAGE_ID, kr.IMAGE_ID + 1))
        problems += [
            f"{name}: {m!r} kept" for m in modes if kr.MODE_PAIRS[m] not in out
        ]
    return problems


def graphics_chunking(graph, frames):
    payload = bytes(range(256)) * 40
    commands = kr.graphics_commands(payload, kr.IMAGE_ID, 1, 80, 24, -1)
    parsed = [kr.parse_command(c) for c in commands]
    problems = []
    if any(len(chunk) > kr.CHUNK for _, chunk in parsed):
        problems.append("a chunk is longer than 4096 characters")
    if [k.get("m") for k, _ in parsed] != ["1"] * (len(parsed) - 1) + ["0"]:
        problems.append("m= continuation is wrong")
    return problems


def nodes_win_over_edges(graph, frames):
    problems = []
    for name, frame in frames.items():
        points = wr.project(frame, 1280, 800)
        for node in frame["nodes"]:
            got = wr.hit(frame, points, *points[node["id"]])
            if got is None or got[0] != "node":
                problems.append(f"web {name}: {node['id']} centre hit {got}")
    return problems


def every_node_reachable(graph, frames):
    problems = []
    for name, frame in frames.items():
        if len(wr.keyboard_order(frame)) != len(frame["nodes"]):
            problems.append(f"{name}: a node has no keyboard stop")
    return problems


def budget_cap_and_more(graph, frames):
    problems = []
    frame = frames["notes-budget-cut"]
    if not frame["cuts"]["budget"]:
        problems.append("the budget case did not cut")
    if floor_lines(frame)[-1] != "(node budget reached)":
        problems.append("the floor does not say the budget cut")
    if not any(node["more"] for node in frame["nodes"]):
        problems.append("no node carries a +N more count after a cut")
    big = wr.synthetic(20_000, 8)
    cap = vr.build_frame(big, "anon", {"focus": "s-0", "radius": 4, "budget": 2000})
    if len(cap["nodes"]) > vr.MAX_BUDGET:
        problems.append("a 20 000-note graph broke the 2000-node cap")
    return problems


def refocus_keeps_identity(graph, frames):
    frame = frames["notes-replies-first"]
    problems = []
    for node in frame["nodes"]:
        refocused = vr.respond(graph, frame, "focus", node["id"])["frame"]
        if node["id"] not in {n["id"] for n in refocused["nodes"]}:
            problems.append(f"refocus on {node['id']} lost it")
    return problems


CHECKS = {
    f.__name__: f
    for f in (
        frames_match_expected,
        frame_invariants,
        unreadable_never_shown,
        outside_targets_refused,
        ring_and_angle_order,
        keyboard_order_is_floor_order,
        every_intent_from_keyboard,
        plain_click_opens,
        focus_is_not_plain_click,
        look_intents_keep_frame,
        claims_shown_as_claims,
        edited_opens_head,
        floor_matches_golden,
        floor_sanitises_labels,
        no_tty_has_no_escapes,
        fallback_triggers,
        teardown_restores,
        graphics_chunking,
        nodes_win_over_edges,
        every_node_reachable,
        budget_cap_and_more,
        refocus_keeps_identity,
    )
}


def check():
    graph, frames = load()
    items = json.loads((HERE / "conformance.json").read_text())["items"]
    failures = []
    ids = [item["id"] for item in items]
    if len(ids) != len(set(ids)):
        failures.append("duplicate item ids")
    used = set()
    manual = 0
    for item in items:
        surfaces = set(item["surfaces"])
        if not surfaces or surfaces - SURFACES:
            failures.append(f"{item['id']}: bad surfaces {sorted(surfaces)}")
        if "check" in item:
            func = CHECKS.get(item["check"])
            if func is None:
                failures.append(f"{item['id']}: no check named {item['check']}")
                continue
            used.add(item["check"])
            for problem in func(graph, frames):
                failures.append(f"{item['id']}: {problem}")
        elif item.get("procedure"):
            manual += 1
        else:
            failures.append(f"{item['id']}: neither a check nor a procedure")
    for name in sorted(set(CHECKS) - used):
        failures.append(f"check {name} is not in the checklist")
    for failure in failures:
        print(failure)
    print(
        f"{len(items)} items ({len(items) - manual} executable, {manual} manual), "
        f"{len(failures)} failures"
    )
    return 1 if failures else 0


def write_floor():
    _, frames = load()
    floors = {name: floor_lines(frame) for name, frame in sorted(frames.items())}
    (HERE / "floor_expected.json").write_text(
        json.dumps(floors, indent=2, ensure_ascii=False) + "\n"
    )
    return 0


def main(argv):
    if argv[1:] == ["check"]:
        return check()
    if argv[1:] == ["write-floor"]:
        return write_floor()
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
