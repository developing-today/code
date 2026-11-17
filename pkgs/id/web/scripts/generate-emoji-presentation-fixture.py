#!/usr/bin/env python3
"""Regenerate the EMOJI_PRESENTATION_SEQ_RANGES parity fixture.

Usage: generate-emoji-presentation-fixture.py <path-to-unicode-width-tables.rs> [output.json]

Decodes the EMOJI_PRESENTATION_LEAVES bit table from unicode-width 0.2.2's
src/tables.rs exactly the way the crate's starts_emoji_presentation_seq
lookup does (top bits select the leaf row, bits 3..9 index the byte, the
low 3 bits select the bit), sweeps every code point, and writes the
resulting membership ranges as the fixture consumed by
src/world-surface.test.ts (#10819). The committed fixture was generated
from the unicode-width 0.2.2 registry source; re-run against a newer
crate only together with a deliberate table update in world-surface.ts.
"""
import json
import re
import sys

TOP_MAP = {0x0: 0, 0x8: 1, 0x9: 2, 0xA: 3, 0xC: 4, 0x7C: 5, 0x7D: 6}


def main() -> None:
    tables_rs = sys.argv[1]
    out_path = sys.argv[2] if len(sys.argv) > 2 else "src/test-fixtures/emoji-presentation-seq-ranges.json"
    src = open(tables_rs).read()
    i = src.index("static EMOJI_PRESENTATION_LEAVES")
    block=src[i:src.index("]);",i)]
    nums = [int(x, 16) for x in re.findall(r"0x[0-9A-Fa-f]+", block)]
    assert len(nums) == 7 * 128, f"expected 896 leaf bytes, parsed {len(nums)}"
    leaves = [nums[k * 128:(k + 1) * 128] for k in range(7)]

    def member(cp: int) -> bool:
        leaf = TOP_MAP.get(cp >> 10)
        if leaf is None:
            return False
        return ((leaves[leaf][(cp >> 3) & 0x7F] >> (cp & 7)) & 1) == 1

    ranges = []
    start = None
    for cp in range(0x110000):
        cur = False if 0xD800 <= cp <= 0xDFFF else member(cp)
        if cur and start is None:
            start = cp
        if not cur and start is not None:
            ranges.append([start, cp - 1])
            start = None
    if start is not None:
        ranges.append([start, 0x10FFFF])

    fixture = {
        "description": "Code point ranges that can start an emoji presentation sequence (be followed by U+FE0F to form a width-2 cluster), decoded from unicode-width 0.2.2's EMOJI_PRESENTATION_LEAVES bit table. Pins the hand transcription in world-surface.ts (EMOJI_PRESENTATION_SEQ_RANGES) against the crate (#10819).",
        "source": "unicode-width 0.2.2, src/tables.rs, starts_emoji_presentation_seq / EMOJI_PRESENTATION_LEAVES",
        "generated_by": "scripts/generate-emoji-presentation-fixture.py",
        "range_count": len(ranges),
        "code_point_count": sum(h - l + 1 for l, h in ranges),
        "ranges": ranges,
    }
    with open(out_path, "w") as fh:
        json.dump(fixture, fh, indent=2)
        fh.write("\n")
    print(f"wrote {out_path}: {fixture['range_count']} ranges, {fixture['code_point_count']} code points")


if __name__ == "__main__":
    main()
