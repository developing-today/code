#!/usr/bin/env python3
"""Validate inventory JSONL schema, stable IDs, and source-line coverage.

Run from the repository root:
  python3 -I doc/2026-11-13T01-34-11Z_design_fleet_ca_credentials_secrets/inventory/validate_inventory.py

Coverage is reported, never inferred from section names: a source line counts only
when a record names that exact source file and includes it in `source_lines`.
Unmapped nonblank lines are failures; blank source lines are reported separately.
"""
from __future__ import annotations

import json
import pathlib
import sys
from collections import defaultdict


ROOT = pathlib.Path(__file__).resolve().parent.parent
SOURCES = ROOT / "sources"
INVENTORY = ROOT / "inventory"
REQUIRED = {
    "requirement_id",
    "source_id",
    "source_file_and_section",
    "source_lines",
    "statement",
    "normative_history_admin_class",
    "canonical_requirement_ids",
    "primary_starter",
    "source_precedence_resolution",
    "user_outcome",
    "severity_or_security_class",
    "verified_current_behavior",
    "proposed_architecture_or_API",
    "primary_issue_number",
    "dependent_issue_numbers",
    "provider_capabilities_and_permissions",
    "acceptance_test_or_benchmark",
    "migration_or_rollback",
    "evidence_status",
    "current_status",
}


def ranges(value: object) -> list[tuple[int, int]]:
    """Read normalized inclusive [start,end] range(s)."""
    if not isinstance(value, list):
        return []
    raw = value
    if len(raw) == 2 and all(isinstance(n, int) for n in raw):
        raw = [raw]
    out = []
    for item in raw:
        if (
            isinstance(item, list)
            and len(item) == 2
            and all(isinstance(n, int) for n in item)
            and 1 <= item[0] <= item[1]
        ):
            out.append((item[0], item[1]))
    return out


def main() -> int:
    errors: list[str] = []
    rows: list[tuple[pathlib.Path, int, dict]] = []
    ids: dict[str, list[str]] = defaultdict(list)
    for path in sorted(INVENTORY.glob("*.jsonl")):
        for line_no, text in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            try:
                row = json.loads(text)
            except json.JSONDecodeError as exc:
                errors.append(f"{path.name}:{line_no}: invalid JSON: {exc}")
                continue
            if not isinstance(row, dict):
                errors.append(f"{path.name}:{line_no}: record is not an object")
                continue
            missing = sorted(REQUIRED - row.keys())
            if missing:
                errors.append(f"{path.name}:{line_no}: missing fields {','.join(missing)}")
            if row.get("source_id") != row.get("requirement_id"):
                errors.append(f"{path.name}:{line_no}: source_id != requirement_id")
            if not ranges(row.get("source_lines")):
                errors.append(f"{path.name}:{line_no}: source_lines has no valid inclusive ranges")
            ids[str(row.get("source_id"))].append(f"{path.name}:{line_no}")
            rows.append((path, line_no, row))

    for source_id, locations in sorted(ids.items()):
        if len(locations) > 1:
            errors.append(f"duplicate source_id {source_id}: {', '.join(locations)}")

    coverage: dict[str, set[int]] = defaultdict(set)
    for _, _, row in rows:
        source_ref = str(row.get("source_file_and_section", "")).strip()
        for source in SOURCES.iterdir():
            if source.is_file() and (
                source_ref.startswith(source.name)
                or source_ref.startswith(f"sources/{source.name}")
                or f"/sources/{source.name}" in source_ref
            ):
                for start, end in ranges(row.get("source_lines")):
                    coverage[source.name].update(range(start, end + 1))

    print(f"records={len(rows)} source_files={sum(p.is_file() for p in SOURCES.iterdir())}")
    nonblank_gaps = 0
    blank_unmapped = 0
    for source in sorted(p for p in SOURCES.iterdir() if p.is_file()):
        text_lines = source.read_text(encoding="utf-8").splitlines()
        mapped = coverage[source.name]
        gaps = [i for i, text in enumerate(text_lines, 1) if text.strip() and i not in mapped]
        blank = [i for i, text in enumerate(text_lines, 1) if not text.strip() and i not in mapped]
        nonblank_gaps += len(gaps)
        blank_unmapped += len(blank)
        print(f"{source.name}: lines={len(text_lines)} mapped_nonblank={sum(1 for i,t in enumerate(text_lines,1) if t.strip() and i in mapped)} nonblank_gaps={len(gaps)} blank_unmapped={len(blank)}")
        if gaps:
            print("  gaps:", ",".join(map(str, gaps)))
    print(f"unmapped_nonblank_lines={nonblank_gaps} unmapped_blank_lines={blank_unmapped}")
    if errors:
        print(f"schema_or_id_errors={len(errors)}", file=sys.stderr)
        for error in errors:
            print(error, file=sys.stderr)
    return 1 if errors or nonblank_gaps else 0


if __name__ == "__main__":
    raise SystemExit(main())
