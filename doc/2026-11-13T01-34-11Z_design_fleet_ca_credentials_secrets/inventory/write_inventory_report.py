#!/usr/bin/env python3
"""Write a reproducible human/JSON audit summary for the canonical inventory."""
from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import sys
from collections import Counter

BASE = pathlib.Path(__file__).absolute().parent.parent
INV = BASE / "inventory"
SOURCES = BASE / "source-snapshots" / "intake-34faffbff2" / "sources"
SOURCE_MANIFEST = INV / "source_snapshot_manifest.json"
CANONICAL = INV / "statements.jsonl"
RESOLVED_GRANULARITY = {
    "atomic-record", "register-atomic-outcome", "register-atomic-requirement",
    "aggregate-register-witness", "aggregate-section-witness", "aggregate-source-witness",
    "reviewed-cohesive-parameterized-oracle", "reviewed-explicit-research",
    "reviewed-unsupported", "reviewed-approval-gated",
}


def main(argv=None):
    parser = argparse.ArgumentParser(
        prog="write_inventory_report.py",
        description="Write a report from the canonical inventory and pinned snapshot (no options).",
        allow_abbrev=False,
    )
    parser.parse_args(argv)
    rows = [json.loads(line) for line in CANONICAL.read_text(encoding="utf-8").splitlines()]
    class_counts = Counter(row["record_class"] for row in rows)
    oracle_counts = Counter(row["oracle_status"] for row in rows)
    granularity_counts = Counter(row["granularity_status"] for row in rows)
    members = [member for row in rows for member in (row.get("atomic_members", []) + row.get("cohesive_members", []))]
    member_class_counts = Counter(member.get("record_class", "unknown") for member in members)
    member_oracle_counts = Counter(member.get("oracle_status", "unknown") for member in members)
    canonical_groups = {group for row in rows for group in row.get("canonical_requirement_ids", [])}
    canonical_groups.update(group for member in members for group in member.get("canonical_requirement_ids", []))
    oracle_gaps = [
        {"source_id": row["source_id"], "source_path": row["source_path"],
         "source_lines": row["source_lines"], "source_file_and_section": row["source_file_and_section"],
         "statement": row["statement"], "canonical_requirement_ids": row["canonical_requirement_ids"],
         "primary_starter": row["primary_starter"], "oracle_origin": row.get("oracle_origin"),
         "oracle": row.get("specific_acceptance_oracle")}
        for row in rows
        if row["record_class"] == "normative-or-proposed" and row["oracle_status"] != "source-specific-oracle-present"
    ]
    gaps = [
        {"source_id": row["source_id"], "source_path": row["source_path"],
         "source_lines": row["source_lines"], "source_file_and_section": row["source_file_and_section"],
         "statement": row["statement"], "canonical_requirement_ids": row["canonical_requirement_ids"],
         "primary_starter": row["primary_starter"], "reason": row["granularity_status"]}
        for row in rows
        if row["record_class"] == "normative-or-proposed"
        and row["granularity_status"] not in RESOLVED_GRANULARITY
    ]
    source_stats = []
    for source in sorted(path for path in SOURCES.iterdir() if path.is_file()):
        lines = source.read_text(encoding="utf-8").splitlines()
        mapped = set()
        for row in rows:
            if row["source_path"] != source.name:
                continue
            for start, end in row["source_lines"]:
                mapped.update(range(start, end + 1))
        nonblank = {n for n, text in enumerate(lines, 1) if text.strip()}
        source_stats.append({"source": source.name, "lines": len(lines), "nonblank_lines": len(nonblank),
                             "mapped_nonblank_lines": len(nonblank & mapped), "unmapped_nonblank_lines": sorted(nonblank - mapped),
                             "sha256": hashlib.sha256(source.read_bytes()).hexdigest()})
    report = {
        "canonical_records": len(rows),
        "source_ids_unique": len({row["source_id"] for row in rows}) == len(rows),
        "canonical_group_ids_unique": len(canonical_groups),
        "member_outcomes": len(members),
        "member_ids_unique": len({member["member_id"] for member in members}) == len(members),
        "member_record_class_counts": dict(member_class_counts),
        "member_oracle_status_counts": dict(member_oracle_counts),
        "source_commit": json.loads(SOURCE_MANIFEST.read_text(encoding="utf-8"))["source_commit"],
        "source_root": json.loads(SOURCE_MANIFEST.read_text(encoding="utf-8"))["source_root"],
        "record_class_counts": dict(class_counts),
        "oracle_status_counts": dict(oracle_counts),
        "oracle_gap_count": len(oracle_gaps),
        "oracle_gaps": oracle_gaps,
        "granularity_status_counts": dict(granularity_counts),
        "atomicity_gap_count": len(gaps),
        "atomicity_gaps": gaps,
        "source_files": source_stats,
        "register_anchor_audit": "Run audit_register_anchors.py; expected 158 register rows / zero anchor errors.",
        "proposal_mapping": "Not performed; parent will launch a separate proposal mapper after canonical freeze.",
    }
    (INV / "inventory_audit.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    lines = ["# Canonical inventory audit", "",
             f"Canonical records: **{len(rows)}**; stable source IDs unique: **{report['source_ids_unique']}**.",
             f"Canonical grouping IDs: **{len(canonical_groups)}**; nested independently testable outcomes: **{len(members)}** (unique member IDs: {report['member_ids_unique']}).",
             f"Class counts: {dict(class_counts)}.",
             f"Nested member classes/oracles: {dict(member_class_counts)} / {dict(member_oracle_counts)}.",
             f"Specific-oracle status: {dict(oracle_counts)}.",
             f"Normative/proposed rows needing oracle refinement: **{len(oracle_gaps)}** (exact IDs in inventory_audit.json).",
             f"Granularity status: {dict(granularity_counts)}.",
             f"Normative/proposed source statements still needing atomicity review: **{len(gaps)}**.",
             "Nonblank-line coverage is computed from normalized inclusive line ranges; each canonical record also carries exact excerpt and whole-source SHA-256.",
             "Register ID-to-line audit: `python3 -I .../audit_register_anchors.py` (expected 158 rows and zero anchor errors).",
             "Granular proposal/issue mapping intentionally not performed.", "", "## Atomicity gaps", ""]
    grouped = {}
    for gap in gaps:
        key = gap["source_path"]
        grouped.setdefault(key, []).append(gap)
    for source, entries in sorted(grouped.items()):
        lines.append(f"### `{source}` ({len(entries)} records)")
        for gap in entries:
            spans = ",".join(f"{a}-{b}" for a, b in gap["source_lines"])
            lines.append(f"- `{gap['source_id']}` lines {spans} — {gap['statement']} (group: `{','.join(gap['canonical_requirement_ids'])}`; starter {gap['primary_starter']})")
        lines.append("")
    (INV / "inventory_audit.md").write_text("\n".join(lines).rstrip() + "\n", encoding="utf-8")
    print(f"records={len(rows)} classes={dict(class_counts)} oracle={dict(oracle_counts)} atomicity_gaps={len(gaps)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
