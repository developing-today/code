#!/usr/bin/env python3
"""Strict inventory checker: python3 -I check_inventory.py --source PATH --json PATH"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path


USAGE = "usage: check_inventory.py --source PATH --json PATH"
SECTION_HEADER = re.compile(r"^### §(\d+) — .*?\((\d+)(?:;|\))")
STATEMENT = re.compile(r"^\s*(S\d+)\.(\d+)\s+((?:\(implied\)\s*)?.*?)\s*$")
MAP_RANGE = re.compile(r"S(\d+)\.(\d+)(?:–S?\1\.(\d+))?")
HOME = re.compile(r"\*\*(#\d+)\*\*")


def arguments(argv: list[str]) -> tuple[Path, Path]:
    """Parse the complete command line before any source read or file write."""
    values: dict[str, str] = {}
    allowed = {"--source", "--json"}
    i = 0
    while i < len(argv):
        arg = argv[i]
        if "=" in arg:
            key, value = arg.split("=", 1)
        else:
            key = arg
            if i + 1 >= len(argv) or argv[i + 1].startswith("--"):
                raise ValueError(f"missing value for {key}")
            value = argv[i + 1]
            i += 1
        if key not in allowed:
            raise ValueError(f"unrecognized argument: {key}")
        if not value:
            raise ValueError(f"empty value for {key}")
        if key in values:
            raise ValueError(f"duplicate option: {key}")
        values[key] = value
        i += 1
    missing = allowed - values.keys()
    if missing:
        raise ValueError("missing required option(s): " + ", ".join(sorted(missing)))
    return Path(values["--source"]), Path(values["--json"])


def parse(source: str) -> tuple[dict[str, int], list[dict], dict[str, list[str]], list[tuple[str, list[str]]], list[tuple[str, str]]]:
    lines = source.splitlines()
    counts: dict[str, int] = {}
    statements: list[dict] = []
    in_inventory = False
    in_map = False
    map_rows: list[tuple[str, list[str]]] = []
    assignments: list[tuple[str, str]] = []

    for line_no, line in enumerate(lines, 1):
        if line.strip() == "## 2. Statement inventory":
            in_inventory = True
            continue
        if line.strip() == "## 3. Statement → issue map":
            in_inventory = False
            in_map = True
            continue
        if in_inventory:
            header = SECTION_HEADER.match(line)
            if header:
                section, count = header.groups()
                counts["S" + section] = int(count)
                continue
            if line.startswith("## "):
                in_inventory = False
                continue
            # Definitions are segments beginning at line start or after the
            # inventory's middle-dot delimiter; references in prose are not.
            for segment in re.split(r"\s+·\s+", line):
                match = STATEMENT.match(segment)
                if not match and "** " in segment:
                    # Subsection labels and the first definition share a line.
                    match = STATEMENT.match(segment.rsplit("** ", 1)[-1])
                if not match:
                    continue
                ident = f"{match.group(1)}.{match.group(2)}"
                text = match.group(3).strip()
                statements.append({"id": ident, "text": text, "source_line": line_no, "issue_homes": []})
        elif in_map and line.startswith("|") and not re.match(r"^\|\s*[-:| ]+\s*\|?$", line):
            cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
            if len(cells) < 2 or cells[0] == "Statements":
                continue
            homes = HOME.findall(cells[1])
            if homes:
                map_rows.append((cells[0], homes))

    by_id = {item["id"]: item for item in statements}
    for expression, homes in map_rows:
        for section, first, last in MAP_RANGE.findall(expression):
            start = int(first)
            end = int(last) if last else start
            for number in range(start, end + 1):
                ident = f"S{section}.{number}"
                if ident in by_id:
                    for home in homes:
                        by_id[ident]["issue_homes"].append(home)
                        assignments.append((ident, home))
    # Make duplicate homes explicit while retaining stable map order.
    for item in statements:
        item["issue_homes"] = list(dict.fromkeys(item["issue_homes"]))
    return counts, statements, by_id, map_rows, assignments


def main(argv: list[str]) -> int:
    try:
        source_path, output_path = arguments(argv)
    except ValueError as exc:
        print(f"{USAGE}\nerror: {exc}", file=sys.stderr)
        return 2

    source_text = source_path.read_text(encoding="utf-8")
    counts, statements, by_id, map_rows, assignments = parse(source_text)
    actual_by_section: dict[str, list[int]] = {}
    duplicate_ids: list[str] = []
    seen: set[str] = set()
    for item in statements:
        if item["id"] in seen:
            duplicate_ids.append(item["id"])
        seen.add(item["id"])
        section, number = item["id"].split(".")
        actual_by_section.setdefault(section, []).append(int(number))

    errors: list[str] = []
    for section, expected_count in counts.items():
        actual = sorted(actual_by_section.get(section, []))
        expected = list(range(1, expected_count + 1))
        if actual != expected:
            errors.append(f"{section}: expected IDs 1..{expected_count}, found {len(actual)} definitions")
    for section in actual_by_section.keys() - counts.keys():
        errors.append(f"{section}: definitions found without a declared section count")
    declared_total_match = re.search(r"\*\*Inventory total: (\d+) statements\*\*", source_text)
    declared_total = int(declared_total_match.group(1)) if declared_total_match else None
    if declared_total is None:
        errors.append("inventory total declaration not found")
    elif len(statements) != declared_total:
        errors.append(f"total: declared {declared_total}, parsed {len(statements)} definitions")
    unmapped = [item["id"] for item in statements if not item["issue_homes"]]
    if unmapped:
        errors.append("unmapped statements: " + ", ".join(unmapped))

    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(json.dumps(statements, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"declared sections: {json.dumps(counts, sort_keys=True)}")
    print(f"parsed statements: {len(statements)} (declared total: {declared_total})")
    print(f"unique IDs: {len(seen)}; duplicate definitions: {duplicate_ids}")
    print(f"unmapped statements: {len(unmapped)}" + (f" ({', '.join(unmapped)})" if unmapped else ""))
    overlaps = [(expr, homes) for expr, homes in map_rows if len(MAP_RANGE.findall(expr)) > 1]
    assignment_ids: dict[str, list[str]] = {}
    for ident, home in assignments:
        assignment_ids.setdefault(ident, []).append(home)
    duplicate_assignments = {ident: homes for ident, homes in assignment_ids.items() if len(homes) > 1}
    print(f"map rows with multiple ranges: {len(overlaps)}")
    print(f"multiply assigned statements: {json.dumps(duplicate_assignments, sort_keys=True)}")
    if errors:
        print("ERRORS:\n- " + "\n- ".join(errors), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
