"""Check the Atlas DOC-01 inventories. This command accepts no arguments."""

import argparse
from collections import Counter
import json
from pathlib import Path
import re
import sys


def inventory(names, source):
    counts = Counter(names)
    if not counts:
        raise ValueError(f"{source}: empty inventory")
    duplicates = sorted(name for name, count in counts.items() if count > 1)
    if duplicates:
        raise ValueError(f"{source}: duplicate entries {duplicates}")
    return set(counts)


def check_modules(rust, chapter):
    modules = []
    for line in rust.splitlines():
        if re.match(r"^\s*pub\s+mod\b", line):
            match = re.fullmatch(r"\s*pub\s+mod\s+(\w+)\s*;\s*(?://.*)?", line)
            if not match:
                raise ValueError(f"atlas-core/lib.rs: unsupported module declaration: {line}")
            modules.append(match[1])
    registered = inventory(modules, "atlas-core/lib.rs")

    headings = re.findall(r"(?m)^## The (\d+) public modules, grouped by job$", chapter)
    summaries = re.findall(r"(?m)^`lib\.rs` declares (\d+) public modules\.", chapter)
    if len(headings) != 1 or len(summaries) != 1:
        raise ValueError("atlas-core.md: expected one public-module heading and count sentence")
    for count in (*headings, *summaries):
        if int(count) != len(registered):
            raise ValueError(
                f"atlas-core.md: stale public-module count {count}; expected {len(registered)}"
            )

    documented = []
    in_module_table = False
    for line in chapter.splitlines():
        if line.startswith("|"):
            cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
            if len(cells) >= 2 and cells[0] == "Module" and cells[1] == "Job":
                in_module_table = True
                continue
            if not in_module_table:
                continue
            if cells and re.fullmatch(r"[-: ]+", cells[0]):
                continue
            match = re.fullmatch(r"\|\s*`(\w+)`\s*\|.*\|", line)
            if not match:
                raise ValueError(f"atlas-core.md: invalid module row: {line}")
            documented.append(match[1])
        else:
            in_module_table = False

    listed = inventory(documented, "atlas-core.md")
    if registered != listed:
        raise ValueError(
            f"atlas-core.md: missing {sorted(registered - listed)}, "
            f"stale {sorted(listed - registered)}"
        )
    return len(registered)


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON key {key}")
        result[key] = value
    return result


def check_defaults(contract, chapter):
    sections = [
        section
        for section in re.split(r"(?m)^## ", chapter)[1:]
        if "`query-parameters-v1.json`" in section.partition("\n")[0]
    ]
    if len(sections) != 1:
        raise ValueError("atlas-proto.md: expected one query-parameters-v1.json section")
    examples = re.findall(r"(?ms)^```json[ \t]*\n(.*?)^```[ \t]*$", sections[0])
    if len(examples) != 1:
        raise ValueError("atlas-proto.md: expected one fenced JSON defaults example")
    expected, actual = [
        json.loads(text, object_pairs_hook=unique_object) for text in (contract, examples[0])
    ]
    if not all(isinstance(obj, dict) and obj for obj in (expected, actual)):
        raise ValueError("query defaults must be nonempty JSON objects")

    # Canonical JSON distinguishes booleans from numbers (False == 0 in Python).
    different = sorted(
        key
        for key in expected.keys() | actual.keys()
        if key not in expected
        or key not in actual
        or json.dumps(expected[key], sort_keys=True) != json.dumps(actual[key], sort_keys=True)
    )
    if different:
        raise ValueError(f"atlas-proto.md: differing query default keys {different}")
    return len(expected)


def main():
    argparse.ArgumentParser(description=__doc__).parse_args()
    root = Path(__file__).resolve().parents[1]

    def read(path):
        return (root / path).read_text(encoding="utf-8")

    try:
        modules = check_modules(
            read("crates/atlas-core/src/lib.rs"), read("book/library/atlas-core.md")
        )
        defaults = check_defaults(
            read("crates/atlas-proto/query-parameters-v1.json"),
            read("book/library/atlas-proto.md"),
        )
    except (OSError, ValueError, json.JSONDecodeError) as error:
        sys.exit(f"Atlas DOC-01: {error}")
    print(f"Atlas DOC-01: {modules} public modules and {defaults} query defaults match the book")


if __name__ == "__main__":
    main()
