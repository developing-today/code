#!/usr/bin/env python3
"""Generate the checked-in html.unescape oracle corpus for the Worker decoder."""

from __future__ import annotations

import html
import html.entities
import json
from pathlib import Path


HERE = Path(__file__).resolve().parent
OUTPUT = HERE / "html5-entity-corpus.json"


def main() -> None:
    cases: list[dict[str, str]] = []
    seen: set[str] = set()

    def add(source: str) -> None:
        if source not in seen:
            seen.add(source)
            cases.append({"input": source, "expected": html.unescape(source)})

    # Every HTML5 table key includes both semicolon and legacy semicolonless
    # spellings. Exercise each exact key, then ambiguous suffixes for the
    # semicolonless forms so longest-prefix behavior stays oracle-pinned.
    for key in sorted(html.entities.html5):
        add(f"&{key}")
        if not key.endswith(";"):
            add(f"&{key}x")
            add(f"&{key}=")

    for source in (
        "&amp B",
        "&notanentity;",
        "&ampx",
        "&copy=",
        "&not=",
        "&#;",
        "&#x;",
        "&#xZZ;",
        "&#65x",
        "&#65=x",
    ):
        add(source)

    # Numeric normalization boundaries: C1 remapping, NUL/surrogate and
    # out-of-range replacement, and the valid scalar boundaries around them.
    for codepoint in (
        0,
        1,
        9,
        0x7F,
        0x80,
        0x81,
        0x82,
        0x8F,
        0x9F,
        0xA0,
        0xD7FF,
        0xD800,
        0xDFFF,
        0xE000,
        0xFDD0,
        0xFDEF,
        0xFFFD,
        0xFFFE,
        0xFFFF,
        0x10000,
        0x1FFFE,
        0x1FFFF,
        0x10FFFE,
        0x10FFFF,
        0x110000,
    ):
        for digits in (str(codepoint), f"x{codepoint:X}"):
            add(f"&#{digits};")
            add(f"&#{digits}")

    add("&#999999999999999999999999999999999999999999999;")
    add("&#xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF;")

    OUTPUT.write_text(
        json.dumps(
            {
                "oracle": "Python html.unescape",
                "entity_key_count": len(html.entities.html5),
                "cases": cases,
            },
            ensure_ascii=False,
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )
    print(
        f"Wrote {len(cases)} cases for {len(html.entities.html5)} HTML5 keys to {OUTPUT}"
    )


if __name__ == "__main__":
    main()
