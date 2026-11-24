#!/usr/bin/env python3
"""Generate the Roc parity expects for kitty-disambiguate-v1 (#787 Slice D).

Reads the shared corpus (doc/kitty-disambiguate-v1-fixtures.json) and
appends one generated `expect` per case to a copy of the real
examples/roc-world/Key.roc: the fixture's terminal bytes must decode
under the actual Roc `Key.parse_events` to the corpus's expected rich
events, and — for cases carrying a `browser` field — the browser
encoder's bytes for the same keystroke must decode to the same events.
This is a generator, not a second decoder: every verdict comes from
running the compiled Roc module (see scripts/key-parity-test.sh).

Usage: key-parity-generate.py FIXTURES_JSON KEY_ROC OUTPUT_ROC

Generated expects compare through a `parity_events` projection that
zeroes the `source` field of each decoded event: `source` records
which byte form produced the event (a terminal's alternate form and
the browser's canonical form for one keystroke differ there by
design), while parity is equality of key + modifiers + kind — plus
the full payload of `EventUnknown`, which is semantic.
"""

import json
import sys

MIN_CASES = 40
MIN_BROWSER_CASES = 20


def roc_str(value: str) -> str:
    out = []
    for ch in value:
        code = ord(ch)
        if ch == "\\":
            out.append("\\\\")
        elif ch == '"':
            out.append('\\"')
        elif ch == "\n":
            out.append("\\n")
        elif ch == "\r":
            out.append("\\r")
        elif ch == "\t":
            out.append("\\t")
        elif code < 0x20 or code == 0x7F:
            out.append(f"\\u({code:x})")
        else:
            out.append(ch)
    return '"' + "".join(out) + '"'


def event_expr(event: dict) -> str:
    if "unknown" in event:
        return f"EventUnknown({roc_str(event['unknown'])})"
    press = event["press"]
    if press["kind"] != "Press":
        raise SystemExit(f"unsupported event kind in corpus: {press['kind']!r}")
    modifiers = "[" + ", ".join(press["modifiers"]) + "]"
    key = press["key"]
    if "named" in key:
        return f"Key.event_named(Named{key['named']}, {modifiers}, \"\")"
    return f"Key.event_scalar({roc_str(key['scalar'])}, {modifiers}, \"\")"


def expected_expr(events: list) -> str:
    return "[" + ", ".join(event_expr(e) for e in events) + "]"


PROJECTION = """
# Parity projection (#787 Slice D): decode with the real parse_events,
# then zero each event's `source` (the byte form that produced it).
# Parity compares key + modifiers + kind, and EventUnknown payloads.
parity_events : Str -> List(KeyEvent)
parity_events = |text|
    List.map(
        Key.parse_events(text),
        |ev|
            match ev {
                KeyEvent(r) => KeyEvent({ key: r.key, modifiers: r.modifiers, kind: r.kind, source: "" })
                EventUnknown(s) => EventUnknown(s)
            },
    )
"""


def main() -> None:
    if len(sys.argv) != 4:
        raise SystemExit("usage: key-parity-generate.py FIXTURES_JSON KEY_ROC OUTPUT_ROC")
    fixtures_path, key_path, out_path = sys.argv[1:4]
    with open(fixtures_path, encoding="utf-8") as fh:
        corpus = json.load(fh)
    cases = corpus["cases"]
    if len(cases) < MIN_CASES:
        raise SystemExit(
            f"key-parity-generate: corpus shrank to {len(cases)} cases "
            f"(< {MIN_CASES}); refusing to run a weakened oracle"
        )
    lines = []
    browser_cases = 0
    for case in cases:
        lines.append(
            f"expect parity_events({roc_str(case['input'])}) "
            f"== {expected_expr(case['expected'])}"
        )
        browser = case.get("browser")
        if browser:
            browser_cases += 1
            lines.append(
                f"expect parity_events({roc_str(browser['bytes'])}) "
                f"== {expected_expr(case['expected'])}"
            )
    if browser_cases < MIN_BROWSER_CASES:
        raise SystemExit(
            f"key-parity-generate: only {browser_cases} browser cases "
            f"(< {MIN_BROWSER_CASES}); refusing to run a weakened oracle"
        )
    with open(key_path, encoding="utf-8") as fh:
        source = fh.read()
    with open(out_path, "w", encoding="utf-8") as fh:
        fh.write(source)
        fh.write("\n# --- generated parity expects: kitty-disambiguate-v1 corpus (#787 Slice D) ---")
        fh.write(PROJECTION)
        fh.write("\n".join(lines))
        fh.write("\n")
    print(
        f"key-parity-generate: {len(cases)} terminal cases, "
        f"{browser_cases} browser cases -> {out_path}"
    )


if __name__ == "__main__":
    main()
