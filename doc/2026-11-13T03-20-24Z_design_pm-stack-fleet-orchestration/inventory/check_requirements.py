#!/usr/bin/env python3
"""Read-only validation for curated requirements and source dispositions.

Usage: python3 -I check_requirements.py --check | --self-test
Semantic completeness is not machine-provable; this checks IDs, trace links,
source bytes, disposition coverage, and declared modality consistency only.
"""
import argparse
import hashlib
import json
import re
import sys
from collections import Counter
from pathlib import Path

SOURCE_PREFIX = {
    "01_handoff_pm_stack_fleet_orchestration.md": "H",
    "02_charter_v1_1.md": "C",
    "03_full_orchestration_proposal.md": "P",
    "04_recovered_original_combined_design.md": "O",
    "05_supplemental_raw_note.md": "S",
}
DOMAINS = {"#9198", "#9199", "#9200", "#9201", "#9203", "#9204", "#9206", "#9207", "#9208", "#9209"}
DISPOSITION_MODALITY = {
    "requirement_link": "requirement",
    "candidate_link": "candidate",
    "history": "history",
    "example": "example",
}
NONSEMANTIC = {"metadata", "heading", "formatting", "continuation"}
MODALITIES = {"requirement", "candidate", "history", "example"}


def load_jsonl(path):
    rows = []
    for number, line in enumerate(path.read_text().splitlines(), 1):
        if not line.strip():
            continue
        try:
            rows.append(json.loads(line))
        except json.JSONDecodeError as exc:
            raise ValueError(f"{path.name}:{number}: invalid JSON: {exc}") from exc
    return rows


def core_errors(seed, requirements, dispositions):
    """Validate ID/citation/disposition relationships without filesystem I/O."""
    errors = []
    seed_by_id = {}
    for row in seed:
        oid = row.get("id")
        if not oid or oid in seed_by_id:
            errors.append(f"missing/duplicate source occurrence ID: {oid!r}")
        else:
            seed_by_id[oid] = row
    req_by_id = {}
    for row in requirements:
        rid = row.get("id")
        if not rid or rid in req_by_id:
            errors.append(f"missing/duplicate requirement ID: {rid!r}")
        else:
            req_by_id[rid] = row
    for row in requirements:
        rid = row.get("id", "<missing>")
        if row.get("modality") not in MODALITIES:
            errors.append(f"{rid}: invalid modality")
        refs = row.get("source_occurrence_refs")
        if not isinstance(refs, list) or not refs:
            errors.append(f"{rid}: missing source occurrence citations")
            continue
        seen = set()
        for ref in refs:
            oid = ref.get("occurrence_id")
            source = seed_by_id.get(oid)
            if source is None:
                errors.append(f"{rid}: unknown occurrence {oid!r}")
                continue
            if oid in seen:
                errors.append(f"{rid}: repeated occurrence reference {oid}")
            seen.add(oid)
            if ref.get("source_section") != source.get("locator", {}).get("section"):
                errors.append(f"{rid}/{oid}: source-section hint mismatch")
            if ref.get("quote") != source.get("quote"):
                errors.append(f"{rid}/{oid}: quote differs from source ledger")
            for key in ("line", "char_start", "char_end_exclusive", "byte_start", "byte_end_exclusive"):
                if ref.get(key) != source.get("locator", {}).get(key):
                    errors.append(f"{rid}/{oid}: {key} differs from source ledger")
    disposition_ids = [row.get("occurrence_id") for row in dispositions]
    duplicates = sorted(oid for oid, n in Counter(disposition_ids).items() if n != 1)
    if duplicates:
        errors.append(f"disposition IDs are not unique: {duplicates[:8]}")
    missing = sorted(set(seed_by_id) - set(disposition_ids))
    extra = sorted(set(disposition_ids) - set(seed_by_id))
    if missing:
        errors.append(f"missing dispositions ({len(missing)}): {missing[:8]}")
    if extra:
        errors.append(f"unknown disposition occurrences ({len(extra)}): {extra[:8]}")
    for row in dispositions:
        oid = row.get("occurrence_id", "<missing>")
        source = seed_by_id.get(oid)
        if source is None:
            continue
        if row.get("source_file") != source.get("source_file"):
            errors.append(f"{oid}: disposition source_file mismatch")
        for key in ("line", "char_start", "char_end_exclusive", "byte_start", "byte_end_exclusive"):
            if row.get("locator", {}).get(key) != source.get("locator", {}).get(key):
                errors.append(f"{oid}: disposition locator {key} mismatch")
        category = row.get("disposition")
        refs = row.get("semantic_refs")
        if not isinstance(refs, list):
            errors.append(f"{oid}: semantic_refs must be a list")
            continue
        if category in NONSEMANTIC and refs:
            errors.append(f"{oid}: {category} disposition must not carry semantic refs")
        if category in DISPOSITION_MODALITY:
            if not refs:
                errors.append(f"{oid}: {category} disposition has no semantic reference")
                continue
            found_required_modality = False
            for ref in refs:
                rid = ref.get("requirement_id")
                requirement = req_by_id.get(rid)
                if requirement is None:
                    errors.append(f"{oid}: unknown semantic requirement {rid!r}")
                    continue
                if oid not in {x.get("occurrence_id") for x in requirement.get("source_occurrence_refs", [])}:
                    errors.append(f"{oid}: {rid} does not cite this source occurrence")
                if requirement.get("modality") == DISPOSITION_MODALITY[category]:
                    found_required_modality = True
            if not found_required_modality:
                errors.append(f"{oid}: {category} has no linked row with matching modality")
        elif category == "unresolved_substantive":
            if refs:
                errors.append(f"{oid}: unresolved_substantive must not claim a semantic link")
            if not row.get("reason"):
                errors.append(f"{oid}: unresolved substantive gap lacks a reason")
        elif category not in NONSEMANTIC:
            errors.append(f"{oid}: unknown disposition category {category!r}")
        if not row.get("reason"):
            errors.append(f"{oid}: missing disposition reason")
    return errors


def run_self_test():
    seed = [{"id": "H-001", "source_file": "01_handoff_pm_stack_fleet_orchestration.md",
             "quote": "ok", "statement": "ok", "locator": {"section": "test", "line": 1,
             "char_start": 0, "char_end_exclusive": 2, "byte_start": 0, "byte_end_exclusive": 2}}]
    req = [{"id": "H-R001", "source_file": seed[0]["source_file"], "modality": "requirement",
            "source_occurrence_refs": [{"occurrence_id": "H-001", "source_section": "test", "quote": "ok",
              "line": 1, "char_start": 0, "char_end_exclusive": 2, "byte_start": 0, "byte_end_exclusive": 2}]}]
    disp = [{"occurrence_id": "H-001", "source_file": seed[0]["source_file"], "locator": {k: seed[0]["locator"][k]
             for k in ("line", "char_start", "char_end_exclusive", "byte_start", "byte_end_exclusive")},
             "disposition": "requirement_link", "semantic_refs": [{"requirement_id": "H-R001"}], "reason": "fixture"}]
    if core_errors(seed, req, disp):
        raise AssertionError("valid fixture rejected")
    bad = json.loads(json.dumps(req)); bad[0]["source_occurrence_refs"][0]["quote"] = "wrong"
    if not core_errors(seed, bad, disp):
        raise AssertionError("wrong quote fixture passed")
    if not core_errors(seed, req, []):
        raise AssertionError("deleted disposition fixture passed")
    bad = json.loads(json.dumps(disp)); bad[0]["semantic_refs"][0]["requirement_id"] = "C-R999"
    if not core_errors(seed, req, bad):
        raise AssertionError("missing requirement reference fixture passed")
    bad = json.loads(json.dumps(disp)); bad[0]["disposition"] = "candidate_link"
    if not core_errors(seed, req, bad):
        raise AssertionError("modality mismatch fixture passed")
    print("OK: self-tests reject wrong source quote, deleted disposition, missing requirement reference, and modality mismatch")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--check", action="store_true")
    group.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        run_self_test()
        return 0
    root = Path(__file__).resolve().parents[1]
    inventory = root / "inventory"
    sources = root / "sources"
    errors = []
    try:
        seed = load_jsonl(inventory / "statements.jsonl")
        requirements = load_jsonl(inventory / "requirements.jsonl")
        dispositions = load_jsonl(inventory / "dispositions.jsonl")
        coverage = json.loads((inventory / "source-coverage.json").read_text())
        conflict_text = (inventory / "conflicts.md").read_text()
        markdown = (inventory / "requirements.md").read_text()
    except Exception as exc:
        print(f"cannot load validation inputs: {exc}", file=sys.stderr)
        return 1
    errors.extend(core_errors(seed, requirements, dispositions))
    req_by_id = {r.get("id"): r for r in requirements}
    valid_conflicts = set(re.findall(r"^\|\s*(CF-\d+)\s+—", conflict_text, re.M))
    represented_sources = {r.get("source_file") for r in seed}
    if represented_sources != set(SOURCE_PREFIX):
        errors.append("seed ledger does not contain exactly the five declared source files")
    target_by_domain = {}
    sequence = Counter()
    for row in requirements:
        rid = row.get("id", "")
        m = re.fullmatch(r"([HCPOS])-R(\d{3})", rid)
        if not m:
            errors.append(f"invalid requirement ID shape: {rid!r}")
        else:
            prefix, number = m.group(1), int(m.group(2))
            sequence[prefix] += 1
            if row.get("source_file") not in SOURCE_PREFIX or SOURCE_PREFIX[row["source_file"]] != prefix:
                errors.append(f"{rid}: prefix/source_file mismatch")
            if f"## {rid} — {row.get('modality')}" not in markdown:
                errors.append(f"{rid}: missing/mismatched Markdown heading")
        if not row.get("outcome", "").strip():
            errors.append(f"{rid}: empty outcome")
        if not row.get("domains") or not row.get("design_targets"):
            errors.append(f"{rid}: missing domain/target")
        for domain in row.get("domains", []):
            if domain not in DOMAINS:
                errors.append(f"{rid}: unknown starter domain {domain}")
        target_starters = [t.get("starter") for t in row.get("design_targets", [])]
        if set(target_starters) != set(row.get("domains", [])) or len(target_starters) != len(set(target_starters)):
            errors.append(f"{rid}: design target starters do not match declared planned domains")
        for target in row.get("design_targets", []):
            if not target.get("filename") or not target.get("section"):
                errors.append(f"{rid}: target must name a planned filename and section")
            starter = target.get("starter")
            pair = (target.get("filename"), target.get("section"))
            if starter in target_by_domain and target_by_domain[starter] != pair:
                errors.append(f"{rid}: planned target for {starter} differs from its other rows")
            target_by_domain[starter] = pair
        kind = row.get("node_kind", "atomic")
        if kind not in ("atomic", "group", "alias"):
            errors.append(f"{rid}: invalid node_kind {kind!r}")
        children = row.get("child_requirement_ids", [])
        if kind == "group" and not children:
            errors.append(f"{rid}: group node has no child_requirement_ids")
        if kind in ("atomic", "alias") and children:
            errors.append(f"{rid}: {kind} node unexpectedly has children")
        if kind == "alias" and row.get("duplicate_of") is None:
            errors.append(f"{rid}: alias node lacks duplicate_of")
        if rid in children:
            errors.append(f"{rid}: group cannot be its own child")
        if any(child not in req_by_id for child in children):
            errors.append(f"{rid}: unknown child requirement ID")
        if row.get("duplicate_of") == rid:
            errors.append(f"{rid}: cannot duplicate itself")
        if row.get("duplicate_of") is not None and row.get("duplicate_of") not in req_by_id:
            errors.append(f"{rid}: unknown duplicate_of {row.get('duplicate_of')}")
        if row.get("duplicate_of") is not None and row.get("duplicate_of") == rid:
            errors.append(f"{rid}: self-referential duplicate_of")
        for conflict in row.get("conflict_refs", []):
            if conflict not in valid_conflicts:
                errors.append(f"{rid}: unresolved conflict reference {conflict}")
    for prefix, count in sequence.items():
        expected = {f"{prefix}-R{i:03d}" for i in range(1, count + 1)}
        actual = {r.get("id") for r in requirements if r.get("id", "").startswith(prefix + "-R")}
        if expected != actual:
            errors.append(f"{prefix}: requirement sequence has gaps")
    for filename, prefix in SOURCE_PREFIX.items():
        source_path = sources / filename
        try:
            raw = source_path.read_bytes()
        except OSError as exc:
            errors.append(f"missing source {filename}: {exc}")
            continue
        expected = coverage.get("source_coverage", {}).get(filename, {}).get("sha256_repo_bytes")
        if hashlib.sha256(raw).hexdigest() != expected:
            errors.append(f"source hash differs from saved manifest: {filename}")
    for row in seed:
        section = row.get("locator", {}).get("section", "")
        if len(section) > 160:
            errors.append(f"{row.get('id')}: section hint exceeds 160 characters")
        if row.get("id", "").startswith("O-") and "provisional" not in section:
            errors.append(f"{row.get('id')}: flattened locator lacks provisional marker")
    for md in inventory.glob("*.md"):
        for number, line in enumerate(md.read_text().splitlines(), 1):
            if line.rstrip(" \t") != line:
                errors.append(f"{md.name}:{number}: trailing whitespace")
    if errors:
        print("\n".join(errors[:100]), file=sys.stderr)
        print(f"FAIL: {len(errors)} validation errors", file=sys.stderr)
        return 1
    by_modality = Counter(r["modality"] for r in requirements)
    by_disposition = Counter(d["disposition"] for d in dispositions)
    provisional = sum(d.get("review_status") == "provisional" for d in dispositions)
    flattened = sum(r["id"].startswith("O-") for r in seed)
    provisional_section_refs = sum(
        "provisional" in ref.get("source_section", "")
        for row in requirements for ref in row.get("source_occurrence_refs", [])
    )
    unresolved = by_disposition.get("unresolved_substantive", 0)
    aliases = sum(row.get("node_kind") == "alias" for row in requirements)
    groups = sum(row.get("node_kind") == "group" for row in requirements)
    atomic = sum(row.get("node_kind", "atomic") == "atomic" for row in requirements)
    print(f"OK: {len(requirements)} requirement records ({atomic} atomic, {groups} groups, {aliases} aliases); {len(dispositions)} dispositions cover every seed occurrence exactly once")
    print(f"Modalities: {dict(by_modality)}; dispositions: {dict(by_disposition)}")
    print(f"Review flags: {provisional} provisional occurrence classifications; {flattened} flattened-source occurrences; {provisional_section_refs} provisional section citations; {unresolved} explicit substantive gaps")
    print("This validates trace integrity and declared modality consistency, not semantic completeness or implementation/issue coverage.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
