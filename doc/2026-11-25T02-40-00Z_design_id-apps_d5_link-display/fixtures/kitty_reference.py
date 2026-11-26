#!/usr/bin/env python3
"""D5 slice 3 reference: how the Kitty terminal renderer chooses, draws and reads.

Stdlib only. Builds on view_reference.py (slice 1) and web_reference.py (slice 2).

  python3 -I kitty_reference.py check   # exit 0 when every check passes

It checks the renderer obligations of slice-3-kitty.md: the surface chosen for
each probe profile and the named fallback trigger, SGR mouse reports turned
into pixels and hits on the same projection as the web, mouse event sequences
turned into slice 1 intents (hover, click, double click, no-motion terminals),
graphics commands chunked within 4096 base64 characters with the control keys
on the first chunk only, labels placed in cells without overlap, and every
mode the renderer enables restored on teardown.
"""

import base64
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import view_reference as vr
import web_reference as wr

ESC = "\x1b"
CHUNK = 4096  # base64 characters per graphics command chunk
DOUBLE_CLICK_MS = 400  # default; the terminal's own interval wins when known
IMAGE_ID = 0x1D50  # the graph picture; the overlay uses IMAGE_ID + 1

# ---------------------------------------------------------------------------
# Surface choice (§2, §3): positive evidence only, one reason per fallback.

GRAPH_NEEDS = (
    ("tty", "no_tty"),
    ("kitty_graphics", "no_kitty_graphics"),
    ("png_accepted", "query_refused"),
    ("synchronized_output", "no_synchronized_output"),
    ("cell_pixels", "cell_size_unknown"),
)


def choose_surface(profile):
    """Return (surface, reason). Surfaces: picture, floor, lines."""
    if not profile.get("tty"):
        return "lines", "no_tty"
    if profile.get("conflicted"):
        return "floor", "probe_conflicted"
    for key, reason in GRAPH_NEEDS:
        if not profile.get(key):
            return "floor", reason
    return "picture", None


def pointer_mode(profile):
    """Which mouse reporting the renderer turns on, from DECRQM evidence."""
    if not profile.get("mouse_sgr"):
        return "keyboard"
    if profile.get("mouse_any_event"):
        return "hover"
    return "click"


PROFILES = {
    "kitty": (
        {
            "tty": True,
            "kitty_graphics": True,
            "png_accepted": True,
            "synchronized_output": True,
            "cell_pixels": (10, 20),
            "mouse_sgr": True,
            "mouse_any_event": True,
            "mouse_pixels": True,
        },
        ("picture", None, "hover"),
    ),
    "ghostty-no-pixels": (
        {
            "tty": True,
            "kitty_graphics": True,
            "png_accepted": True,
            "synchronized_output": True,
            "cell_pixels": (9, 18),
            "mouse_sgr": True,
            "mouse_any_event": True,
        },
        ("picture", None, "hover"),
    ),
    "graphics-no-motion": (
        {
            "tty": True,
            "kitty_graphics": True,
            "png_accepted": True,
            "synchronized_output": True,
            "cell_pixels": (10, 20),
            "mouse_sgr": True,
        },
        ("picture", None, "click"),
    ),
    "graphics-no-mouse": (
        {
            "tty": True,
            "kitty_graphics": True,
            "png_accepted": True,
            "synchronized_output": True,
            "cell_pixels": (10, 20),
        },
        ("picture", None, "keyboard"),
    ),
    "tmux-without-passthrough": (
        {"tty": True, "synchronized_output": True, "mouse_sgr": True},
        ("floor", "no_kitty_graphics", "click"),
    ),
    "query-refused": (
        {
            "tty": True,
            "kitty_graphics": True,
            "synchronized_output": True,
            "cell_pixels": (10, 20),
        },
        ("floor", "query_refused", "keyboard"),
    ),
    "no-sync": (
        {
            "tty": True,
            "kitty_graphics": True,
            "png_accepted": True,
            "cell_pixels": (10, 20),
        },
        ("floor", "no_synchronized_output", "keyboard"),
    ),
    "cell-size-unknown": (
        {
            "tty": True,
            "kitty_graphics": True,
            "png_accepted": True,
            "synchronized_output": True,
        },
        ("floor", "cell_size_unknown", "keyboard"),
    ),
    "conflicted": (
        {"tty": True, "kitty_graphics": True, "conflicted": True},
        ("floor", "probe_conflicted", "keyboard"),
    ),
    "pipe": ({}, ("lines", "no_tty", "keyboard")),
}

# ---------------------------------------------------------------------------
# Mouse reports (§5): SGR encoding, cells or pixels, into pane pixels.


def parse_sgr(data):
    """Parse SGR (1006) mouse reports: ESC [ < b ; x ; y M|m."""
    events = []
    text = data
    while True:
        start = text.find(ESC + "[<")
        if start < 0:
            return events
        end = start + 3
        while end < len(text) and text[end] not in "Mm":
            end += 1
        if end >= len(text):
            return events
        b, x, y = (int(part) for part in text[start + 3 : end].split(";"))
        kind = "motion" if b & 32 else ("press" if text[end] == "M" else "release")
        if b & 64:
            kind = "wheel"
        events.append(
            {
                "kind": kind,
                "button": b & 3,
                "shift": bool(b & 4),
                "alt": bool(b & 8),
                "ctrl": bool(b & 16),
                "x": x,
                "y": y,
            }
        )
        text = text[end + 1 :]


def to_pixels(event, profile):
    """Pane-relative pixel for a report. Cell reports land on the cell centre."""
    if profile.get("mouse_pixels"):
        return event["x"] - 1.0, event["y"] - 1.0
    cw, ch = profile["cell_pixels"]
    return (event["x"] - 0.5) * cw, (event["y"] - 0.5) * ch


def cell_of(point, profile):
    cw, ch = profile["cell_pixels"]
    return int(point[0] // cw) + 1, int(point[1] // ch) + 1


def hit_radius(profile, zoom=1.0):
    """A cell report is a cell centre, so a node must be hittable from it."""
    cw, ch = profile["cell_pixels"]
    if profile.get("mouse_pixels"):
        return wr.NODE_RADIUS * max(zoom, 1.0)
    return max(wr.NODE_RADIUS * max(zoom, 1.0), (cw**2 + ch**2) ** 0.5 / 2)


def hit(frame, points, x, y, profile, zoom=1.0):
    radius = hit_radius(profile, zoom)
    best = None
    for node in frame["nodes"]:
        px, py = points[node["id"]]
        d = ((x - px) ** 2 + (y - py) ** 2) ** 0.5
        if d <= radius and (best is None or d < best[0]):
            best = (d, "node", node["id"])
    if best:
        return best[1:]
    slop = wr.EDGE_SLOP if profile.get("mouse_pixels") else radius / 2
    for edge in frame["edges"]:
        (ax, ay), (bx, by) = points[edge["source"]], points[edge["target"]]
        length2 = (bx - ax) ** 2 + (by - ay) ** 2
        if length2 == 0:
            continue
        t = max(0.0, min(1.0, ((x - ax) * (bx - ax) + (y - ay) * (by - ay)) / length2))
        d = ((x - ax - t * (bx - ax)) ** 2 + (y - ay - t * (by - ay)) ** 2) ** 0.5
        if d <= slop and (best is None or d < best[0]):
            best = (d, "edge", edge["id"])
    return best[1:] if best else None


# ---------------------------------------------------------------------------
# Gestures (§5): timed mouse events into slice 1 intents.


def gestures(events, targets, mode, double_click_ms=DOUBLE_CLICK_MS):
    """events: (ms, event); targets: the hit for each event. Returns intents.

    hover mode: motion identifies, a click opens, a double click focuses.
    click mode (no motion reports): a click selects and identifies, a click on
    the selected target opens, a double click focuses; the card in cells also
    offers Open, Focus and Inspect, like touch in slice 2.
    """
    out = []
    selected = None
    last_hover = None
    last_click = None
    pending_open = None
    for (ms, event), target in zip(events, targets, strict=True):
        if pending_open and ms - pending_open[0] > double_click_ms:
            out.append(pending_open[1])
            pending_open = None
        if event["kind"] == "motion" and mode == "hover":
            if target != last_hover and target is not None:
                kind = "identify" if target[0] == "node" else "identify-link"
                out.append((kind, target[1]))
            last_hover = target
            continue
        if event["kind"] != "release" or event["button"] != 0 or target is None:
            continue
        if (
            last_click
            and last_click[1] == target
            and ms - last_click[0] <= double_click_ms
        ):
            pending_open = None
            if target[0] == "node":
                out.append(("focus", target[1]))
            last_click = None
            continue
        last_click = (ms, target)
        if mode == "click" and selected != target:
            selected = target
            kind = "identify" if target[0] == "node" else "identify-link"
            out.append((kind, target[1]))
            continue
        intent = "open" if target[0] == "node" else "inspect-link"
        pending_open = (ms, (intent, target[1]))
    if pending_open:
        out.append(pending_open[1])
    return out


KEYS = {
    "Up": "identify",
    "Down": "identify",
    "Left": "identify",
    "Right": "identify",
    "Tab": "identify-link",
    "l": "identify-link",
    "Enter": "open",
    "i": "inspect-link",
    "f": "focus",
}

# ---------------------------------------------------------------------------
# Drawing (§4): graphics commands, labels in cells, teardown.


def graphics_commands(payload, image_id, placement_id, cols, rows, z):
    """Transmit-and-place one PNG, chunked; control keys on the first chunk."""
    data = base64.b64encode(payload).decode()
    chunks = [data[i : i + CHUNK] for i in range(0, len(data), CHUNK)] or [""]
    out = []
    for index, chunk in enumerate(chunks):
        more = 1 if index < len(chunks) - 1 else 0
        if index == 0:
            keys = (
                f"a=T,f=100,t=d,i={image_id},p={placement_id},"
                f"c={cols},r={rows},z={z},C=1,q=2,m={more}"
            )
        else:
            keys = f"m={more}"
        out.append(f"{ESC}_G{keys};{chunk}{ESC}\\")
    return out


def parse_command(command):
    body = command[len(ESC) + 2 : -2]
    keys, _, payload = body.partition(";")
    return dict(part.split("=", 1) for part in keys.split(",")), payload


def place_labels(frame, points, profile, cols, rows):
    """Label each node in the cells right of its dot, cut at the next label.

    Returns {node id: (col, row, text)}; nodes whose cell is taken get none.
    The focus, then bright nodes, then dimmed ones claim cells first.
    """
    taken = {
        cell_of(points[node["id"]], profile): node["id"] for node in frame["nodes"]
    }
    labels = {}
    order = sorted(frame["nodes"], key=lambda n: (n["hop"], n["dim"], n["id"]))
    for node in order:
        col, row = cell_of(points[node["id"]], profile)
        start = col + 1
        if not (1 <= row <= rows) or start > cols:
            continue
        width = 0
        while start + width <= cols and (start + width, row) not in taken:
            width += 1
            if width >= 24:
                break
        if width < 4:
            continue
        label = node["label"]
        text = label if len(label) < width else label[: width - 2] + "…"
        for c in range(start, min(start + len(text) + 1, cols + 1)):
            taken.setdefault((c, row), node["id"])
        labels[node["id"]] = (start, row, text)
    return labels


SETUP = (
    f"{ESC}[?1049h",  # alternate screen
    f"{ESC}[?25l",  # hide cursor
    f"{ESC}[?1006h",  # SGR mouse encoding
)
MODE_PAIRS = {
    f"{ESC}[?1049h": f"{ESC}[?1049l",
    f"{ESC}[?25l": f"{ESC}[?25h",
    f"{ESC}[?1006h": f"{ESC}[?1006l",
    f"{ESC}[?1000h": f"{ESC}[?1000l",
    f"{ESC}[?1003h": f"{ESC}[?1003l",
    f"{ESC}[?1016h": f"{ESC}[?1016l",
    f"{ESC}[?2026h": f"{ESC}[?2026l",
}


def session_modes(profile):
    modes = list(SETUP)
    pointer = pointer_mode(profile)
    if pointer == "keyboard":
        modes.remove(f"{ESC}[?1006h")
    else:
        modes.append(f"{ESC}[?1000h")
        if pointer == "hover":
            modes.append(f"{ESC}[?1003h")
        if profile.get("mouse_pixels"):
            modes.append(f"{ESC}[?1016h")
    return modes


def teardown(modes, image_ids):
    out = [f"{ESC}_Ga=d,d=I,i={image_id},q=2;{ESC}\\" for image_id in image_ids]
    out += [MODE_PAIRS[mode] for mode in reversed(modes)]
    return out


# ---------------------------------------------------------------------------


def check():
    data = json.loads((Path(vr.HERE) / "graph.json").read_text())
    graph = vr.Graph(data)
    frames = vr.all_frames(graph)
    failures = []

    # Surface choice and the named fallback triggers.
    for name, (profile, expected) in PROFILES.items():
        surface, reason = choose_surface(profile)
        got = (surface, reason, pointer_mode(profile))
        if got != expected:
            failures.append(f"profile {name}: got {got}, expected {expected}")
    reasons = {reason for _, reason in GRAPH_NEEDS} | {"probe_conflicted"}
    seen = {choose_surface(p)[1] for p, _ in PROFILES.values()} - {None}
    if reasons - seen:
        failures.append(
            f"fallback triggers without a profile: {sorted(reasons - seen)}"
        )

    # SGR parsing.
    report = f"{ESC}[<0;12;5M{ESC}[<0;12;5m{ESC}[<35;13;5M{ESC}[<64;1;1M{ESC}[<16;2;3M"
    kinds = [
        (e["kind"], e["button"], e["x"], e["y"], e["ctrl"]) for e in parse_sgr(report)
    ]
    if kinds != [
        ("press", 0, 12, 5, False),
        ("release", 0, 12, 5, False),
        ("motion", 3, 13, 5, False),
        ("wheel", 0, 1, 1, False),
        ("press", 0, 2, 3, True),
    ]:
        failures.append(f"SGR parse: {kinds}")

    # Hits from cell and pixel reports, on the slice 2 projection.
    for profile_name in ("kitty", "ghostty-no-pixels"):
        profile = PROFILES[profile_name][0]
        cw, ch = profile["cell_pixels"]
        cols, rows = 120, 40
        width, height = cols * cw, rows * ch
        for name, frame in frames.items():
            points = wr.project(frame, width, height)
            for node in frame["nodes"]:
                px, py = points[node["id"]]
                if profile.get("mouse_pixels"):
                    event = {"x": int(px) + 1, "y": int(py) + 1}
                else:
                    col, row = cell_of((px, py), profile)
                    event = {"x": col, "y": row}
                x, y = to_pixels(event, profile)
                got = hit(frame, points, x, y, profile)
                same_cell = [
                    n["id"]
                    for n in frame["nodes"]
                    if cell_of(points[n["id"]], profile) == cell_of((px, py), profile)
                ]
                if got is None or got[0] != "node" or got[1] not in same_cell:
                    failures.append(
                        f"{profile_name} {name}: report at {node['id']} hit {got}"
                    )

    # Gestures: hover mode, click mode, double click, keyboard reach.
    n, m = ("node", "a"), ("node", "b")
    e = ("edge", "l")
    move = {"kind": "motion", "button": 3, "x": 0, "y": 0}
    press = {"kind": "press", "button": 0, "x": 0, "y": 0}
    release = {"kind": "release", "button": 0, "x": 0, "y": 0}
    scripts = {
        "hover-identify-open": (
            "hover",
            [(0, move), (10, move), (20, move), (500, press), (520, release)],
            [n, n, e, n, n],
            [("identify", "a"), ("identify-link", "l"), ("open", "a")],
        ),
        "hover-double-click-focus": (
            "hover",
            [(0, press), (30, release), (200, press), (230, release)],
            [n, n, n, n],
            [("focus", "a")],
        ),
        "hover-slow-clicks-open-twice": (
            "hover",
            [(0, press), (30, release), (900, press), (930, release)],
            [m, m, m, m],
            [("open", "b"), ("open", "b")],
        ),
        "click-select-then-open": (
            "click",
            [(0, press), (20, release), (900, press), (920, release)],
            [n, n, n, n],
            [("identify", "a"), ("open", "a")],
        ),
        "click-edge-then-inspect": (
            "click",
            [(0, press), (20, release), (900, press), (920, release)],
            [e, e, e, e],
            [("identify-link", "l"), ("inspect-link", "l")],
        ),
        "click-double-click-focus": (
            "click",
            [(0, press), (20, release), (150, press), (170, release)],
            [m, m, m, m],
            [("identify", "b"), ("focus", "b")],
        ),
    }
    for name, (mode, events, targets, expected) in scripts.items():
        got = gestures(events, targets, mode)
        if got != expected:
            failures.append(f"gesture {name}: got {got}, expected {expected}")
    if set(KEYS.values()) != wr.INTENTS:
        failures.append("the keyboard does not reach every intent")

    # Graphics commands: chunking and keys.
    for size in (0, 1, 3071, 3072, 3073, 10_000, 100_000):
        payload = bytes(range(256)) * (size // 256) + bytes(size % 256)
        commands = graphics_commands(payload, IMAGE_ID, 1, 120, 40, -1)
        parsed = [parse_command(c) for c in commands]
        joined = "".join(p for _, p in parsed)
        if base64.b64decode(joined) != payload:
            failures.append(f"chunks of {size} bytes do not round-trip")
        for index, (keys, chunk) in enumerate(parsed):
            last = index == len(parsed) - 1
            if len(chunk) > CHUNK or keys.get("m") != ("0" if last else "1"):
                failures.append(f"chunk {index} of {size} bytes is malformed")
            if index > 0 and set(keys) != {"m"}:
                failures.append(f"chunk {index} of {size} bytes repeats control keys")
        first = parsed[0][0]
        if first.get("t") != "d" or first.get("q") != "2" or first.get("C") != "1":
            failures.append("first chunk must be direct, quiet and keep the cursor")

    # Labels in cells: no overlap, never over another node's dot.
    profile = PROFILES["kitty"][0]
    cw, ch = profile["cell_pixels"]
    for name, frame in frames.items():
        points = wr.project(frame, 120 * cw, 40 * ch)
        labels = place_labels(frame, points, profile, 120, 40)
        used = {}
        dots = {cell_of(points[i], profile) for i in points}
        for node_id, (col, row, text) in labels.items():
            for c in range(col, col + len(text)):
                if (c, row) in used:
                    failures.append(f"{name}: labels overlap at {(c, row)}")
                if (c, row) in dots:
                    failures.append(f"{name}: label of {node_id} covers a dot")
                used[(c, row)] = node_id
        if frame["focus"] not in labels:
            failures.append(f"{name}: the focus has no label")

    # Teardown restores every mode it set and deletes every image.
    for name, (profile, _) in PROFILES.items():
        modes = session_modes(profile)
        out = teardown(modes, (IMAGE_ID, IMAGE_ID + 1))
        for mode in modes:
            if MODE_PAIRS[mode] not in out:
                failures.append(f"profile {name}: {mode!r} is not restored")
        if f"i={IMAGE_ID}," not in "".join(out):
            failures.append(f"profile {name}: the picture is not deleted")

    for failure in failures:
        print(failure)
    print(
        f"{len(PROFILES)} profiles, {len(frames)} frames, "
        f"{len(scripts)} gesture scripts, {len(failures)} failures"
    )
    return 1 if failures else 0


def main(argv):
    if argv[1:] == ["check"]:
        return check()
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
