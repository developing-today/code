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
MAP_TERM = re.compile(r"S(\d+)\.(\d+)(?:–S?(\d+)\.(\d+))?")
BOLD = re.compile(r"\*\*(.*?)\*\*")
HOME = re.compile(r"#\d+")
ORACLE_COUNTS = {"S0": 17, "S1": 120, "S2": 19, "S3": 38, "S4": 93, "S5": 64, "S6": 17, "S7": 16}
ORACLE_TOTAL = 384


class InventoryError(ValueError):
    pass


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
    source_path, output_path = Path(values["--source"]), Path(values["--json"])
    if source_path.resolve() == output_path.resolve():
        raise ValueError("--source and --json must not name the same path")
    if source_path.exists() and output_path.exists():
        try:
            same_file = source_path.samefile(output_path)
        except OSError as exc:
            raise ValueError(f"cannot verify source/output path identity: {exc}") from exc
        if same_file:
            raise ValueError("--source and --json must not refer to the same file")
    return source_path, output_path


def parse_map_expression(expression: str, valid_ids: set[str]) -> list[str]:
    """Parse every comma-delimited map token; never accept a matching substring."""
    if not expression.strip():
        raise InventoryError("empty statement range expression")
    expanded: list[str] = []
    for raw_term in expression.split(","):
        term = raw_term.strip()
        match = MAP_TERM.fullmatch(term)
        if not match:
            raise InventoryError(f"malformed map range token: {term!r}")
        section, first, end_section, end_number = match.groups()
        start_id = f"S{section}.{int(first)}"
        finish_id = f"S{end_section}.{int(end_number)}" if end_section is not None else start_id
        if start_id not in valid_ids:
            raise InventoryError(f"unknown map endpoint: {start_id}")
        if finish_id not in valid_ids:
            raise InventoryError(f"unknown map endpoint: {finish_id}")
        if end_section is not None and end_section != section:
            raise InventoryError(f"cross-section map range is not allowed: {term!r}")
        start_number = int(first)
        finish_number = int(end_number) if end_number is not None else start_number
        if finish_number < start_number:
            raise InventoryError(f"reversed map range: {term!r}")
        expanded.extend(f"S{section}.{number}" for number in range(start_number, finish_number + 1))
    return expanded


def parse(source: str) -> tuple[dict[str, int], list[dict], dict[str, list[str]], list[tuple[str, list[str]]], list[tuple[str, str]]]:
    lines = source.splitlines()
    counts: dict[str, int] = {}
    statements: list[dict] = []
    in_inventory = False
    in_map = False
    found_map_section = False
    found_map_header = False
    map_rows: list[tuple[str, list[str]]] = []
    assignments: list[tuple[str, str]] = []

    for line_no, line in enumerate(lines, 1):
        if line.strip() == "## 2. Statement inventory":
            in_inventory = True
            continue
        if line.strip() == "## 3. Statement → issue map":
            in_inventory = False
            in_map = True
            found_map_section = True
            continue
        if in_map and line.startswith("## "):
            # Section 3 ends at the next top-level section; later tables are
            # not part of the issue map and cannot satisfy its coverage.
            in_map = False
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
            # Definitions start at line start or after the inventory's middle-dot delimiter.
            for segment in re.split(r"\s+·\s+", line):
                match = STATEMENT.match(segment)
                if not match and "** " in segment:
                    # Subsection labels and their first definition share a line.
                    match = STATEMENT.match(segment.rsplit("** ", 1)[-1])
                if match:
                    ident = f"{match.group(1)}.{match.group(2)}"
                    statements.append({"id": ident, "text": match.group(3).strip(), "source_line": line_no, "issue_homes": []})
        elif in_map and line.startswith("|") and not re.match(r"^\|\s*[-:| ]+\s*\|?$", line):
            cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
            if len(cells) < 2:
                continue
            if cells[0] == "Statements":
                found_map_header = True
                continue
            expression = cells[0]
            bold_tokens = BOLD.findall(cells[1])
            if not bold_tokens:
                raise InventoryError(f"map row has no bold issue home: {expression!r}")
            homes: list[str] = []
            for token in bold_tokens:
                token = token.strip()
                if not HOME.fullmatch(token):
                    raise InventoryError(f"malformed issue home in map row {expression!r}: {token!r}")
                homes.append(token)
            # This labeled row assigns sequencing work, not statement IDs.
            if expression == "Sequencing of all of the above":
                continue
            map_rows.append((expression, homes))

    if not found_map_section:
        raise InventoryError("section 3 statement map not found")
    if not found_map_header or not map_rows:
        raise InventoryError("section 3 contains no statement map rows")

    by_id = {item["id"]: item for item in statements}
    if len(by_id) != len(statements):
        seen: set[str] = set()
        repeated: set[str] = set()
        for item in statements:
            if item["id"] in seen:
                repeated.add(item["id"])
            seen.add(item["id"])
        raise InventoryError("duplicate statement definitions: " + ", ".join(sorted(repeated)))
    valid_ids = set(by_id)
    for expression, homes in map_rows:
        for ident in parse_map_expression(expression, valid_ids):
            for home in homes:
                by_id[ident]["issue_homes"].append(home)
                assignments.append((ident, home))
    for item in statements:
        item["issue_homes"] = list(dict.fromkeys(item["issue_homes"]))
    return counts, statements, by_id, map_rows, assignments


def main(argv: list[str]) -> int:
    try:
        source_path, output_path = arguments(argv)
    except ValueError as exc:
        print(f"{USAGE}\nerror: {exc}", file=sys.stderr)
        return 2

    try:
        source_text = source_path.read_text(encoding="utf-8")
    except OSError as exc:
        print(f"source error: {exc}", file=sys.stderr)
        return 1
    try:
        counts, statements, by_id, map_rows, assignments = parse(source_text)
    except InventoryError as exc:
        print(f"inventory error: {exc}", file=sys.stderr)
        return 1

    actual_by_section: dict[str, list[int]] = {}
    for item in statements:
        section, number = item["id"].split(".")
        actual_by_section.setdefault(section, []).append(int(number))
    errors: list[str] = []
    if counts != ORACLE_COUNTS:
        errors.append(f"declared section counts differ from fixed oracle: expected {ORACLE_COUNTS}, found {counts}")
    for section, expected_count in ORACLE_COUNTS.items():
        actual = sorted(actual_by_section.get(section, []))
        if actual != list(range(1, expected_count + 1)):
            errors.append(f"{section}: expected IDs 1..{expected_count}, found {len(actual)} definitions")
    for section in actual_by_section.keys() - ORACLE_COUNTS.keys():
        errors.append(f"{section}: definitions found without a fixed-oracle section")
    total_match = re.search(r"\*\*Inventory total: (\d+) statements\*\*", source_text)
    declared_total = int(total_match.group(1)) if total_match else None
    if declared_total != ORACLE_TOTAL or len(statements) != ORACLE_TOTAL:
        errors.append(f"total: fixed oracle {ORACLE_TOTAL}, declared {declared_total}, parsed {len(statements)} definitions")
    unmapped = [item["id"] for item in statements if not item["issue_homes"]]
    if unmapped:
        errors.append("unmapped statements: " + ", ".join(unmapped))
    if errors:
        print("ERRORS:\n- " + "\n- ".join(errors), file=sys.stderr)
        return 1

    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(json.dumps(statements, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    seen: set[str] = set()
    duplicate_assignments: dict[str, list[str]] = {}
    all_assignments: dict[str, list[str]] = {}
    for ident, home in assignments:
        all_assignments.setdefault(ident, []).append(home)
    duplicate_assignments = {ident: homes for ident, homes in all_assignments.items() if len(homes) > 1}
    for item in statements:
        seen.add(item["id"])
    print(f"fixed section counts: {json.dumps(ORACLE_COUNTS, sort_keys=True)}")
    print(f"parsed statements: {len(statements)} (declared total: {declared_total})")
    print(f"unique IDs: {len(seen)}; duplicate definitions: []")
    print("unmapped statements: 0")
    print(f"map rows with multiple terms: {sum(',' in expression for expression, _ in map_rows)}")
    print(f"multiply assigned statements: {json.dumps(duplicate_assignments, sort_keys=True)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
