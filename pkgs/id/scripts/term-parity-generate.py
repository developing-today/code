#!/usr/bin/env python3
"""Generate the Roc parity expects for the primary-mode painter (#14936).

Reads the shared frame fixtures (test-data/paint-primary-fixtures.json)
and appends two generated `expect`s per case to a copy of the real
examples/roc-world/Term.roc: the fixture's prev/next frames — built by
folding the case's put-text writes over Screen.new, in the same shape
Term.roc's own expects use — must satisfy

    Term.paint_primary(prev, next) == <pinned transition bytes>
    Term.paint_primary_full(next)  == <pinned full-repaint bytes>

The same fixtures pin the host-side Rust painter
(ScreenSurface::paint_primary in crates/id-core), so the Roc contract
and the Rust implementation cannot drift: a byte change on either side
fails one of the two oracles. This is a generator, not a second
painter: every verdict comes from running the compiled Roc module
(see scripts/term-parity-test.sh).

Usage: term-parity-generate.py FIXTURES_JSON TERM_ROC OUTPUT_ROC
"""

import json
import sys

MIN_CASES = 10


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


def frame_expr(width: int, height: int, writes: list) -> str:
    expr = f"Screen.new({width}, {height})"
    for write in writes:
        bold = "Bool.True" if write.get("bold", False) else "Bool.False"
        style = f"{{ fg: {write.get('fg', 0)}, bg: {write.get('bg', 0)}, bold: {bold} }}"
        expr = (
            f"Screen.put_text({expr}, {write['x']}, {write['y']}, "
            f"{roc_str(write['text'])}, {style})"
        )
    return expr


def main() -> None:
    if len(sys.argv) != 4:
        raise SystemExit("usage: term-parity-generate.py FIXTURES_JSON TERM_ROC OUTPUT_ROC")
    fixtures_path, term_path, out_path = sys.argv[1:4]
    with open(fixtures_path, encoding="utf-8") as fh:
        corpus = json.load(fh)
    cases = corpus["cases"]
    if len(cases) < MIN_CASES:
        raise SystemExit(
            f"term-parity-generate: corpus shrank to {len(cases)} cases "
            f"(< {MIN_CASES}); refusing to run a weakened oracle"
        )
    lines = []
    for case in cases:
        prev = frame_expr(case["width"], case["height"], case["prev"])
        next_frame = frame_expr(
            case.get("next_width", case["width"]),
            case.get("next_height", case["height"]),
            case["next"],
        )
        lines.append(
            f"expect Term.paint_primary({prev}, {next_frame}) == {roc_str(case['paint'])}"
        )
        lines.append(
            f"expect Term.paint_primary_full({next_frame}) == {roc_str(case['full'])}"
        )
    with open(term_path, encoding="utf-8") as fh:
        source = fh.read()
    with open(out_path, "w", encoding="utf-8") as fh:
        fh.write(source)
        fh.write("\n# --- generated parity expects: paint-primary fixtures (#14936) ---\n")
        fh.write("\n".join(lines))
        fh.write("\n")
    print(f"term-parity-generate: {len(cases)} cases -> {out_path}")


if __name__ == "__main__":
    main()
