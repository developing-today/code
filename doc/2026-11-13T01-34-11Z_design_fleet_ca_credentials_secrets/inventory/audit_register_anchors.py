#!/usr/bin/env python3
"""Audit §31 and Appendix A register ID -> exact source line anchors."""
from __future__ import annotations

import argparse
import json
import hashlib
import pathlib
import re
import sys


BASE = pathlib.Path(__file__).absolute().parent.parent
SOURCE = BASE / "source-snapshots" / "intake-34faffbff2" / "sources" / "fleet_architecture_spec_v0_3.md"
INV = BASE / "inventory"


def strict_pairs(pairs):
    result = {}
    for k, v in pairs:
        if k in result:
            raise ValueError(f"duplicate key {k}")
        result[k] = v
    return result


def line_number_by_id(lines, start, end, id_pattern):
    found = {}
    for n in range(start, end + 1):
        m = id_pattern.search(lines[n - 1])
        if m:
            found[m.group(1)] = n
    return found


def q_id_to_register_id(source_id):
    value = source_id.removeprefix("SPEC-Q-")
    m = re.match(r"(sec|gen|ssh|iroh|ops|fut)(\d{3})", value, re.I)
    if m:
        return f"{m.group(1).upper()}-{m.group(2)}"
    m = re.match(r"pki(\d{3})", value, re.I)
    if m:
        n = int(m.group(1))
        return f"PKI2-{n - 200:03}" if n >= 200 else f"PKI-{n:03}"
    return None


def row_line_by_id(lines, start, end, id_pattern):
    return line_number_by_id(lines, start, end, id_pattern)


def main(argv=None):
    parser = argparse.ArgumentParser(
        prog="audit_register_anchors.py",
        description="Audit §31 and Appendix A ID anchors in the pinned snapshot (no options).",
        allow_abbrev=False,
    )
    parser.parse_args(argv)
    lines = SOURCE.read_text(encoding="utf-8").splitlines()
    manifest = json.loads((INV / "source_snapshot_manifest.json").read_text(encoding="utf-8"))
    source_meta = next((x for x in manifest["source_files"] if x["source_file"] == SOURCE.name), None)
    actual_hash = hashlib.sha256(SOURCE.read_bytes()).hexdigest()
    if not source_meta or source_meta["sha256"] != actual_hash:
        raise ValueError(f"pinned snapshot hash mismatch for {SOURCE.name}: expected {source_meta and source_meta['sha256']}, got {actual_hash}")
    sec = row_line_by_id(lines, 1988, 2067, re.compile(r"^\|\s*([A-Z0-9]+-\d{3})\s*\|"))
    app = row_line_by_id(lines, 2709, 2786, re.compile(r"^\|\s*(PKI-\d{3})\s*\|"))
    all_expected = {**sec, **app}
    errors = []
    checked = 0
    # Primary one-record-per-register-row records.
    primary = INV / "SPEC_v03_sections25-49_appA_sec31.jsonl"
    for no, text in enumerate(primary.read_text(encoding="utf-8").splitlines(), 1):
        r = json.loads(text, object_pairs_hook=strict_pairs)
        key = r.get("canonical_requirement_key")
        if key not in all_expected:
            continue
        checked += 1
        expected = all_expected[key]
        actual = r.get("source_lines", [])
        if actual != [expected, expected] and actual != [[expected, expected]]:
            errors.append(f"{primary.name}:{no} {key}: expected source line {expected}, got {actual}")
        if r.get("user_outcome") != lines[expected - 1].split("|", 4)[3].strip():
            errors.append(f"{primary.name}:{no} {key}: outcome does not match source register row {expected}")
    # Additional decomposed entries must point to the source row named by their ID.
    extra_file = INV / "SPEC_v03_register_atomic_audit.jsonl"
    for no, text in enumerate(extra_file.read_text(encoding="utf-8").splitlines(), 1):
        r = json.loads(text, object_pairs_hook=strict_pairs)
        if str(r.get("source_id", "")).startswith("SPEC-A-"):
            continue
        if r.get("source_id") == "SPEC-Q-v3-table-header":
            if r.get("source_lines") != [[2653, 2653]] and r.get("source_lines") != [2653, 2653]:
                errors.append(f"{extra_file.name}:{no}: V3 gate table heading must map to line 2653")
            continue
        key = q_id_to_register_id(str(r.get("source_id", "")))
        if key is None:
            errors.append(f"{extra_file.name}:{no}: cannot derive register ID from {r.get('source_id')}")
            continue
        expected = all_expected.get(key)
        if expected is None:
            errors.append(f"{extra_file.name}:{no}: source register ID {key} absent")
            continue
        checked += 1
        actual = r.get("source_lines", [])
        if actual != [expected, expected] and actual != [[expected, expected]]:
            errors.append(f"{extra_file.name}:{no} {key}: expected source line {expected}, got {actual}")
    print(f"register_rows={len(all_expected)} checked_records={checked} anchor_errors={len(errors)}")
    for error in errors:
        print(error)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
