#!/usr/bin/env python3
"""Check ID research chapter navigation and linked files, optionally in a built book."""

import argparse
from pathlib import Path
import re
from urllib.parse import unquote, urlsplit


BOOK = Path(__file__).resolve().parents[1] / "book"
SUMMARY_ENTRY = re.compile(r"^\s*-\s+\[([^\]]+)\]\(([^)]+)\)\s*$")
LINK = re.compile(r"!?\[([^\]\n]+)\]\(([^)\s]+)\)")
DEFINITION = re.compile(r"^\[[^\]]+\]:\s+(\S+)")
FENCE = re.compile(r"^\s*(`{3,}|~{3,})")


def prose_lines(text):
    """Skip fenced code examples when checking headings, navigation and links."""
    fence = ""
    for number, line in enumerate(text.splitlines(), 1):
        marker = FENCE.match(line)
        if marker:
            if not fence:
                fence = marker[1]
            elif marker[1].startswith(fence):
                fence = ""
            continue
        if not fence:
            yield number, line


def check(rendered):
    errors = []
    titles = {}
    for line in (BOOK / "SUMMARY.md").read_text().splitlines():
        entry = SUMMARY_ENTRY.match(line)
        if not entry:
            continue
        title, name = entry.groups()
        chapter = (BOOK / name).resolve()
        if not chapter.is_relative_to(BOOK) or not chapter.is_file():
            errors.append(f"SUMMARY.md: missing or out-of-tree chapter {name}")
            continue
        if chapter in titles:
            errors.append(f"SUMMARY.md: duplicate chapter {name}")
        titles[chapter] = title

    research = BOOK / "research"
    sources = sorted(research.glob("*.md"))
    if not sources:
        errors.append("Missing research chapters")
    for path in sources:
        if path not in titles:
            errors.append(f"Chapter missing from SUMMARY.md: {path.name}")

    linked = set()
    for path, title in titles.items():
        lines = list(prose_lines(path.read_text()))
        heads = [line for _, line in lines if line.startswith("# ")]
        if heads != ["# " + title] or not lines or lines[0][1] != "# " + title:
            errors.append(f"Chapter title disagrees with SUMMARY.md: {path.relative_to(BOOK)}")
        if not path.is_relative_to(research):
            continue
        for number, line in lines:
            matches = [(m[1], m[2]) for m in LINK.finditer(line)]
            definition = DEFINITION.match(line)
            if definition:
                matches.append((None, definition[1]))
            for label, raw_url in matches:
                url = urlsplit(raw_url.strip("<>"))
                if url.scheme or url.netloc or not url.path:
                    continue
                dest = (path.parent / unquote(url.path)).resolve()
                if not dest.is_relative_to(BOOK) or not dest.is_file():
                    errors.append(f"{path.name}:{number}: missing or out-of-tree {raw_url}")
                    continue
                linked.add(dest)
                if dest.suffix == ".md":
                    if dest not in titles:
                        errors.append(f"{path.name}:{number}: unlisted Markdown target {raw_url}")
                    elif label is not None and label != titles[dest]:
                        errors.append(f"{path.name}:{number}: link title disagrees with {raw_url}")

    if rendered:
        for target in sorted(set(titles) | linked):
            rel = target.relative_to(BOOK)
            expected = rel.with_suffix(".html") if rel.suffix == ".md" else rel
            if not (rendered / expected).is_file():
                errors.append(f"Rendered book is missing {expected}")

    if errors:
        raise SystemExit("\n".join(errors))
    print(f"{len(titles)} listed chapters, {len(sources)} research chapters, "
          f"{len(linked)} distinct research link targets verified")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--rendered", type=Path, metavar="DIR",
                        help="also check the static files installed by the ID docs build")
    check(parser.parse_args().rendered)
