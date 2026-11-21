#!/usr/bin/env python3
"""Strictly validate shards and canonical Fleet v0.3 statement inventory.

Run from repository root:
  python3 -I doc/2026-11-13T01-34-11Z_design_fleet_ca_credentials_secrets/inventory/validate_inventory.py

Exit 1: malformed data, duplicate IDs/keys, source/hash/range errors or unmapped
nonblank source lines. Exit 2: structural validation passed, but atomicity or
statement-specific oracle gaps remain. No section-level coverage inference.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import re
import sys
from collections import Counter, defaultdict

BASE = pathlib.Path(__file__).absolute().parent.parent
SOURCES = BASE / "source-snapshots" / "intake-34faffbff2" / "sources"
SOURCE_ROOT = BASE / "source-snapshots" / "intake-34faffbff2"
INVENTORY = BASE / "inventory"
CANONICAL = INVENTORY / "statements.jsonl"
SOURCE_MANIFEST = INVENTORY / "source_snapshot_manifest.json"

REQUIRED = {
    "requirement_id", "source_id", "source_file_and_section", "source_lines",
    "statement", "normative_history_admin_class", "canonical_requirement_ids",
    "primary_starter", "source_precedence_resolution", "user_outcome",
    "severity_or_security_class", "verified_current_behavior",
    "proposed_architecture_or_API", "primary_issue_number", "dependent_issue_numbers",
    "provider_capabilities_and_permissions", "acceptance_test_or_benchmark",
    "migration_or_rollback", "evidence_status", "current_status",
}
CANONICAL_REQUIRED = REQUIRED | {"source_path", "source_excerpt", "source_sha256", "source_commit", "source_authority", "record_class", "granularity_status", "oracle_status"}
GENERIC_ORACLE = re.compile(
    r"independent acceptance oracle must|acceptance/evidence must be defined|"
    r"acceptance follows|not specified by source|confirm implementation only from pinned|"
    r"test must assert outcome|acceptance criteria.*to be defined",
    re.IGNORECASE,
)
RESOLVED_GRANULARITY = {
    "atomic-record", "register-atomic-outcome", "register-atomic-requirement",
    "aggregate-register-witness", "aggregate-section-witness", "aggregate-source-witness",
    "reviewed-cohesive-parameterized-oracle", "reviewed-explicit-research",
    "reviewed-unsupported", "reviewed-approval-gated",
}


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON key {key!r}")
        result[key] = value
    return result


def line_ranges(value):
    if not isinstance(value, list):
        return []
    if len(value) == 2 and all(type(n) is int for n in value):
        value = [value]
    result = []
    for item in value:
        if isinstance(item, list) and len(item) == 2 and all(type(n) is int for n in item) and 1 <= item[0] <= item[1]:
            result.append((item[0], item[1]))
    return result


def resolve_source(record):
    ref = str(record.get("source_file_and_section", "")).strip()
    for source in sorted(SOURCES.iterdir()):
        if source.is_file() and (ref.startswith(source.name) or ref.startswith(f"sources/{source.name}") or f"/sources/{source.name}" in ref):
            return source
    if ref.startswith("intake note.md") or ref.startswith("note.md"):
        note = SOURCE_ROOT / "note.md"
        return note if note.is_file() else None
    return None


def read_jsonl(path, errors):
    rows = []
    for line_no, text in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        try:
            row = json.loads(text, object_pairs_hook=unique_object)
        except Exception as exc:
            errors.append(f"{path.name}:{line_no}: invalid JSON: {exc}")
            continue
        if not isinstance(row, dict):
            errors.append(f"{path.name}:{line_no}: record must be an object")
            continue
        rows.append((line_no, row))
    return rows


def main(argv=None):
    parser = argparse.ArgumentParser(
        prog="validate_inventory.py",
        description="Validate the frozen Fleet v0.3 source-statement inventory (no options).",
        allow_abbrev=False,
    )
    parser.parse_args(argv)
    errors = []
    if not SOURCE_MANIFEST.is_file():
        print("pinned source snapshot manifest missing", file=sys.stderr)
        return 1
    source_manifest = json.loads(SOURCE_MANIFEST.read_text(encoding="utf-8"))
    expected_source_hashes = {row["source_file"]: row["sha256"] for row in source_manifest["source_files"]}
    expected_source_hashes.update({row["source_file"]: row["sha256"] for row in source_manifest.get("auxiliary_files", [])})
    shard_rows = []
    seen = {}
    excluded_assignment_manifests = 0
    for shard in sorted(INVENTORY.glob("*.jsonl")):
        if shard == CANONICAL:
            continue
        if shard.name in {"spec00-13_lines045-299.jsonl", "spec14-24_atomicity.jsonl"}:
            draft_rows = read_jsonl(shard, errors)
            if not draft_rows or not (INVENTORY / "refinements" / shard.name).exists():
                errors.append(f"{shard.name}: excluded draft or reviewed replacement missing")
            excluded_assignment_manifests += 1
            continue
        for line_no, row in read_jsonl(shard, errors):
            if row.get("record_type") == "assignment_manifest":
                excluded_assignment_manifests += 1
                continue
            missing = sorted(REQUIRED - row.keys())
            if missing:
                errors.append(f"{shard.name}:{line_no}: missing §8/integration fields {','.join(missing)}")
            sid = row.get("source_id")
            if not isinstance(sid, str) or not sid:
                errors.append(f"{shard.name}:{line_no}: source_id must be nonempty string")
            elif sid in seen:
                errors.append(f"duplicate source_id {sid}: {seen[sid]} and {shard.name}:{line_no}")
            else:
                seen[sid] = f"{shard.name}:{line_no}"
            if row.get("requirement_id") != sid:
                errors.append(f"{shard.name}:{line_no}: requirement_id/source_id mismatch")
            spans = line_ranges(row.get("source_lines"))
            if not spans:
                errors.append(f"{shard.name}:{line_no}: source_lines must contain valid inclusive ranges")
            if not isinstance(row.get("canonical_requirement_ids"), list):
                errors.append(f"{shard.name}:{line_no}: canonical_requirement_ids must be an explicit list")
            cls = str(row.get("normative_history_admin_class", "")).lower()
            contextual = any(t in cls for t in ("history", "context", "admin", "formatting", "out-of-scope", "provenance", "metadata", "structural", "non-normative"))
            if not contextual and not row.get("canonical_requirement_ids"):
                errors.append(f"{shard.name}:{line_no}: normative row requires canonical requirement ID(s)")
            if "primary_starter" not in row:
                errors.append(f"{shard.name}:{line_no}: missing explicit primary_starter field")
            shard_rows.append((shard.name, line_no, row))

    if not CANONICAL.exists():
        errors.append("statements.jsonl is missing; run build_canonical_inventory.py")
        canonical_rows = []
    else:
        canonical_rows = read_jsonl(CANONICAL, errors)
    canonical_ids = set()
    source_contents = {}
    coverage = defaultdict(set)
    oracle_gaps = []
    atomicity_gaps = []
    classes = Counter()
    member_ids = set()
    member_classes = Counter()
    member_oracle_gaps = []
    member_integrity_errors = []
    for line_no, row in canonical_rows:
        missing = sorted(CANONICAL_REQUIRED - row.keys())
        if missing:
            errors.append(f"statements.jsonl:{line_no}: missing canonical fields {','.join(missing)}")
            continue
        sid = row["source_id"]
        if sid in canonical_ids:
            errors.append(f"statements.jsonl:{line_no}: duplicate canonical source_id {sid}")
        canonical_ids.add(sid)
        if row["requirement_id"] != sid:
            errors.append(f"statements.jsonl:{line_no}: requirement_id/source_id mismatch")
        if row["record_class"] == "normative-or-proposed" and (not row["canonical_requirement_ids"] or not str(row.get("primary_starter") or "").startswith("#")):
            errors.append(f"statements.jsonl:{line_no}: normative record lacks canonical requirement IDs or normalized primary starter")
        source = resolve_source(row)
        if source is None:
            errors.append(f"statements.jsonl:{line_no}: source reference does not resolve: {row['source_file_and_section']}")
            continue
        if row["source_path"] != source.name:
            errors.append(f"statements.jsonl:{line_no}: source_path is {row['source_path']!r}, expected {source.name!r}")
        if row["source_commit"] != source_manifest["source_commit"]:
            errors.append(f"statements.jsonl:{line_no}: source_commit does not match frozen source manifest")
        if row["source_authority"] != "source-snapshots/intake-34faffbff2":
            errors.append(f"statements.jsonl:{line_no}: source_authority is not the pinned original snapshot")
        if source not in source_contents:
            raw = source.read_bytes()
            digest = hashlib.sha256(raw).hexdigest()
            expected_digest = expected_source_hashes.get(source.name)
            if digest != expected_digest:
                errors.append(f"pinned snapshot hash mismatch for {source.name}: expected {expected_digest}, got {digest}")
            source_contents[source] = (digest, raw.decode("utf-8").splitlines())
        digest, lines = source_contents[source]
        if row["source_sha256"] != digest:
            errors.append(f"statements.jsonl:{line_no}: source hash mismatch for {source.name}")
        spans = line_ranges(row["source_lines"])
        if not spans:
            errors.append(f"statements.jsonl:{line_no}: invalid/empty source_lines")
            continue
        excerpts = []
        for start, end in spans:
            if end > len(lines):
                errors.append(f"statements.jsonl:{line_no}: range {start}-{end} exceeds {source.name} ({len(lines)} lines)")
                continue
            excerpts.append("\n".join(lines[start - 1:end]))
            if source.is_relative_to(SOURCES):
                coverage[source.name].update(range(start, end + 1))
        expected_excerpt = "\n\n--- source span ---\n\n".join(excerpts)
        if row["source_excerpt"] != expected_excerpt:
            errors.append(f"statements.jsonl:{line_no}: exact source excerpt mismatch for {sid}")
        classes[row["record_class"]] += 1
        if row["record_class"] == "normative-or-proposed":
            oracle = str(row.get("specific_acceptance_oracle") or row.get("acceptance_test_or_benchmark") or "")
            if row["oracle_status"] != "source-specific-oracle-present" or not oracle or GENERIC_ORACLE.search(oracle):
                oracle_gaps.append(sid)
            if row["granularity_status"] not in RESOLVED_GRANULARITY:
                atomicity_gaps.append(sid)
        parent_spans = spans
        for member_key in ("atomic_members", "cohesive_members"):
            members = row.get(member_key, [])
            if not isinstance(members, list):
                errors.append(f"statements.jsonl:{line_no}: {member_key} must be a list")
                continue
            for member in members:
                if not isinstance(member, dict):
                    errors.append(f"statements.jsonl:{line_no}: {member_key} record must be an object")
                    continue
                mid = member.get("member_id")
                if not isinstance(mid, str) or not mid:
                    errors.append(f"statements.jsonl:{line_no}: {member_key} member_id missing")
                    continue
                if mid in member_ids or mid in canonical_ids:
                    errors.append(f"statements.jsonl:{line_no}: duplicate member/source ID {mid}")
                member_ids.add(mid)
                if member.get("parent_source_id") != sid:
                    errors.append(f"statements.jsonl:{line_no}: member {mid} has wrong parent_source_id")
                if member.get("source_path") != source.name or member.get("source_sha256") != digest:
                    errors.append(f"statements.jsonl:{line_no}: member {mid} source path/hash does not match parent snapshot")
                if member.get("source_commit") != source_manifest["source_commit"]:
                    errors.append(f"statements.jsonl:{line_no}: member {mid} source commit does not match snapshot")
                member_spans = line_ranges(member.get("source_lines"))
                if not member_spans:
                    errors.append(f"statements.jsonl:{line_no}: member {mid} has invalid/empty source_lines")
                    continue
                if any(not any(pa <= a and b <= pb for pa, pb in parent_spans) for a, b in member_spans):
                    errors.append(f"statements.jsonl:{line_no}: member {mid} source spans escape parent source spans")
                member_excerpt = "\n\n--- source span ---\n\n".join("\n".join(lines[a - 1:b]) for a, b in member_spans if b <= len(lines))
                if member.get("source_excerpt") != member_excerpt:
                    errors.append(f"statements.jsonl:{line_no}: member {mid} source excerpt mismatch")
                if not isinstance(member.get("canonical_requirement_ids"), list) or not member.get("canonical_requirement_ids"):
                    errors.append(f"statements.jsonl:{line_no}: member {mid} lacks canonical grouping IDs")
                if not str(member.get("primary_starter") or "").startswith("#"):
                    errors.append(f"statements.jsonl:{line_no}: member {mid} lacks primary starter")
                member_classes[member.get("record_class", "unknown")] += 1
                if member.get("record_class") == "normative-or-proposed":
                    oracle = str(member.get("specific_acceptance_oracle") or "")
                    if member.get("oracle_status") != "source-specific-oracle-present" or not oracle or GENERIC_ORACLE.search(oracle):
                        member_oracle_gaps.append(mid)
                    if member.get("granularity_status") not in {"atomic-record", "reviewed-cohesive-parameterized-oracle"}:
                        member_integrity_errors.append(mid)

    source_gaps = 0
    for source in sorted(p for p in SOURCES.iterdir() if p.is_file()):
        lines = source.read_text(encoding="utf-8").splitlines()
        mapped = coverage[source.name]
        missing = [n for n, text in enumerate(lines, 1) if text.strip() and n not in mapped]
        source_gaps += len(missing)
        print(f"{source.name}: nonblank={sum(bool(x.strip()) for x in lines)} mapped={sum(bool(x.strip()) and n in mapped for n,x in enumerate(lines,1))} gaps={len(missing)}")
        if missing:
            print("  unmapped lines:", ",".join(map(str, missing)))

    print(f"shard_records={len(shard_rows)} excluded_assignment_manifests={excluded_assignment_manifests} canonical_records={len(canonical_rows)} unique_source_ids={len(seen)}")
    print(f"record_classes={dict(classes)}")
    print(f"member_outcomes={len(member_ids)} member_classes={dict(member_classes)} member_oracle_gaps={len(member_oracle_gaps)} member_atomicity_gaps={len(member_integrity_errors)}")
    print(f"normative_oracle_gaps={len(oracle_gaps)} normative_atomicity_gaps={len(atomicity_gaps)} unmapped_nonblank_source_lines={source_gaps}")
    if oracle_gaps:
        print("oracle_gap_ids=" + ",".join(oracle_gaps))
    if atomicity_gaps:
        print("atomicity_gap_ids=" + ",".join(atomicity_gaps))
    if member_oracle_gaps:
        print("member_oracle_gap_ids=" + ",".join(member_oracle_gaps))
    if member_integrity_errors:
        print("member_atomicity_gap_ids=" + ",".join(member_integrity_errors))
    if errors:
        print(f"structural_errors={len(errors)}", file=sys.stderr)
        for error in errors:
            print(error, file=sys.stderr)
        return 1
    if source_gaps or oracle_gaps or atomicity_gaps or member_oracle_gaps or member_integrity_errors:
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
