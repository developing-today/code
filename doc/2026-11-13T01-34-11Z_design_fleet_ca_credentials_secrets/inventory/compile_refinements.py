#!/usr/bin/env python3
"""Compile reviewed add-only refinements into the sole canonical inventory.

The source shards and refinement files remain immutable inputs. This compiler is
the only writer of statements.jsonl after refinement review.
"""
from __future__ import annotations

import hashlib
import argparse
import json
import importlib.util
import pathlib
import re
import sys
from collections import Counter, defaultdict

GENERIC_ORACLE_PATTERN = re.compile(
    r"acceptance reviewer|test oracle for this outcome|independently exercise this exact source member|"
    r"exact cited source lines|exact source lines, checked against|test the outcome|chosen for this member|"
    r"independent acceptance oracle|fixture/evidence must independently",
    re.IGNORECASE,
)

BASE = pathlib.Path(__file__).absolute().parent.parent
INV = BASE / "inventory"
SOURCES = BASE / "source-snapshots" / "intake-34faffbff2" / "sources"
CANONICAL = INV / "statements.jsonl"
REFINEMENTS = INV / "refinements"
SOURCE_MANIFEST = INV / "source_snapshot_manifest.json"
ALLOWED_GRANULARITY = {
    "atomic-record", "register-atomic-outcome", "register-atomic-requirement",
    "aggregate-register-witness", "aggregate-section-witness", "aggregate-source-witness",
    "reviewed-cohesive-parameterized-oracle", "reviewed-explicit-research",
    "reviewed-unsupported", "reviewed-approval-gated", "explicit-history-context-admin",
}

def rebuild_from_pinned_snapshot():
    spec = importlib.util.spec_from_file_location("build_canonical_inventory", INV / "build_canonical_inventory.py")
    module = importlib.util.module_from_spec(spec)
    if not spec or not spec.loader:
        raise RuntimeError("cannot load pinned-snapshot canonical builder")
    spec.loader.exec_module(module)
    module.build()


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON key {key!r}")
        result[key] = value
    return result


def ranges(value):
    if not isinstance(value, list):
        return []
    if len(value) == 2 and all(type(v) is int for v in value):
        value = [value]
    out = []
    for span in value:
        if isinstance(span, list) and len(span) == 2 and all(type(v) is int for v in span) and span[0] >= 1 and span[1] >= span[0]:
            out.append([span[0], span[1]])
    return out


def format_oracle(oracle):
    if isinstance(oracle, str):
        text = oracle.strip()
        if len(text) < 40 or re.search(r"independent acceptance oracle must|test the outcome|to be defined", text, re.I):
            raise ValueError(f"oracle is generic/too short: {text!r}")
        return text
    if not isinstance(oracle, dict):
        raise ValueError("oracle must be a specific string or {decider,input_fixture_or_evidence,expected_invariant,reference_oracle}")
    required = ("decider", "input_fixture_or_evidence", "expected_invariant", "reference_oracle")
    missing = [key for key in required if not str(oracle.get(key, "")).strip()]
    if missing:
        raise ValueError(f"oracle missing fields {missing}")
    if len({str(oracle[k]).strip().lower() for k in required}) == 1:
        raise ValueError("oracle fields are not independently specified")
    joined = " ".join(str(oracle[k]) for k in required)
    if GENERIC_ORACLE_PATTERN.search(joined):
        raise ValueError(f"oracle uses generic template language: {joined[:240]}")
    if len(str(oracle["input_fixture_or_evidence"]).strip()) < 40 or len(str(oracle["reference_oracle"]).strip()) < 30:
        raise ValueError("oracle input/reference lacks a concrete fixture or independent decision procedure")
    return (f"Decider: {oracle['decider']} Input/evidence: {oracle['input_fixture_or_evidence']} "
            f"Expected invariant: {oracle['expected_invariant']} Reference oracle: {oracle['reference_oracle']}")


def source_data(record):
    name = record.get("source_path")
    if name and "/" in name:
        name = pathlib.Path(name).name
    if not name:
        ref = str(record.get("source_file_and_section", ""))
        matches = [p for p in SOURCES.iterdir() if p.is_file() and (ref.startswith(p.name) or f"/sources/{p.name}" in ref or ref.startswith(f"sources/{p.name}"))]
        if len(matches) != 1:
            raise ValueError(f"could not resolve source for {record.get('source_id')}: {ref}")
        path = matches[0]
    else:
        path = SOURCES / name
    if not path.is_file():
        raise ValueError(f"refinement source is not in the pinned snapshot root: {path}")
    raw = path.read_bytes()
    digest = hashlib.sha256(raw).hexdigest()
    manifest = json.loads(SOURCE_MANIFEST.read_text(encoding="utf-8"))
    expected = {row["source_file"]: row["sha256"] for row in manifest["source_files"]}.get(path.name)
    if expected != digest:
        raise ValueError(f"pinned source snapshot hash mismatch for {path.name}: expected {expected}, got {digest}")
    return path, raw, raw.decode("utf-8").splitlines(), digest


def excerpt_for(path, lines, spans):
    pieces = []
    for start, end in spans:
        if end > len(lines):
            raise ValueError(f"range {start}-{end} exceeds {path.name} line count {len(lines)}")
        pieces.append("\n".join(lines[start - 1:end]))
    return "\n\n--- source span ---\n\n".join(pieces)


def excerpt_matches(stated, path, lines, spans):
    pieces = []
    line_pieces = []
    for start, end in spans:
        if end > len(lines):
            return False
        piece_lines = lines[start - 1:end]
        pieces.append("\n".join(piece_lines))
        line_pieces.extend(piece_lines)
    choices = {
        "\n\n--- source span ---\n\n".join(pieces),
        "\n\n".join(pieces),
        "\n".join(pieces),
    }
    # Refinement children may cite an exact clause substring of an anchored
    # source line (for example one command token or one semicolon clause).
    return stated in choices or any(stated and stated in line for line in line_pieces)


def row_class(value):
    value = str(value or "").lower()
    if any(x in value for x in ("historical", "history", "non-normative")):
        return "historical"
    if any(x in value for x in ("context", "admin", "formatting", "out-of-scope", "provenance", "metadata", "structural")):
        return "context/admin"
    return "normative-or-proposed"


def load_refinements():
    rows = []
    for path in sorted(REFINEMENTS.glob("*.jsonl")):
        if path.name.endswith("_draft.jsonl"):
            # Preserve the moved first-pass child artifact for audit, but only
            # consume reviewed active refinements in the canonical compiler.
            continue
        for line_no, text in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            try:
                row = json.loads(text, object_pairs_hook=unique_object)
            except Exception as exc:
                raise ValueError(f"{path.name}:{line_no}: invalid JSON: {exc}") from exc
            row["_refinement_file"] = path.name
            row.setdefault("gap_kind", "oracle" if "oracle" in path.name else "atomicity")
            rows.append(row)
    return rows


def main(argv=None):
    parser = argparse.ArgumentParser(
        prog="compile_refinements.py",
        description="Compile reviewed refinements into statements.jsonl from the pinned source snapshot (no options).",
        allow_abbrev=False,
    )
    parser.parse_args(argv)
    # Always derive from immutable source shards first; reruns cannot stack or
    # silently duplicate prior compiled children in statements.jsonl.
    rebuild_from_pinned_snapshot()
    current = [json.loads(line, object_pairs_hook=unique_object) for line in CANONICAL.read_text(encoding="utf-8").splitlines()]
    by_id = {row["source_id"]: row for row in current}
    if len(by_id) != len(current):
        raise ValueError("canonical source IDs are not unique before refinement compilation")
    # Parent source IDs from the immutable pre-refinement canonical snapshot.
    oracle_gaps = {row["source_id"] for row in current if row.get("oracle_status") == "needs-specific-oracle-review"}
    atomic_gaps = {row["source_id"] for row in current if row.get("record_class") == "normative-or-proposed" and row.get("granularity_status") not in ALLOWED_GRANULARITY}
    refinements = load_refinements()
    by_gap: dict[tuple[str, str], list[dict]] = defaultdict(list)
    for refinement in refinements:
        key = (refinement.get("gap_kind"), refinement.get("parent_source_id"))
        by_gap[key].append(refinement)
    selected = []
    for key, candidates in by_gap.items():
        # Early first-half pass recorded unresolved placeholders for all parents;
        # prefer a later source-specific split/cohesive disposition when present.
        concrete = [r for r in candidates if r.get("disposition") not in {"unsupported", "specific-oracle still unresolved", "unresolved"} or r.get("children") or r.get("oracle") or r.get("acceptance_oracle")]
        if len(concrete) > 1:
            raise ValueError(f"multiple concrete refinements for {key}: {[x.get('_refinement_file') for x in concrete]}")
        if concrete:
            selected.append(concrete[0])
        else:
            selected.append(candidates[-1])
    seen: dict[tuple[str, str], str] = {}
    new_members = []
    resolutions = {"oracle": set(), "atomicity": set()}
    for refinement in selected:
        kind = refinement.get("gap_kind")
        if kind not in {"oracle", "atomicity"}:
            # Infer only from the dedicated file name; absence of an unambiguous
            # gap class is a hard error rather than an implicit disposition.
            name = refinement.get("_refinement_file", "")
            kind = "oracle" if "oracle" in name else "atomicity" if "atomicity" in name else None
        if kind not in resolutions:
            raise ValueError(f"refinement {refinement.get('refinement_id')} lacks gap_kind")
        parent_id = refinement.get("parent_source_id")
        if parent_id not in by_id:
            raise ValueError(f"refinement parent source ID not found: {parent_id}")
        key = (kind, parent_id)
        if key in seen:
            raise ValueError(f"duplicate {kind} refinement for {parent_id}: {seen[key]} and {refinement.get('_refinement_file')}")
        seen[key] = str(refinement.get("_refinement_file"))
        resolutions[kind].add(parent_id)
        parent = by_id[parent_id]
        disposition = refinement.get("disposition")
        parent_spans = ranges(parent.get("source_lines"))
        refined_spans = ranges(refinement.get("source_lines") or refinement.get("source_ranges"))
        if refined_spans:
            source_path, _, source_lines, _ = source_data({**parent, **refinement})
            exact_refinement_excerpt = excerpt_for(source_path, source_lines, refined_spans)
            stated_refinement_excerpt = refinement.get("exact_source_excerpt") or refinement.get("source_excerpt")
            if stated_refinement_excerpt and not excerpt_matches(stated_refinement_excerpt, source_path, source_lines, refined_spans):
                raise ValueError(f"refinement {refinement.get('refinement_id')} excerpt mismatch for {parent_id}")
            if kind == "oracle" and refined_spans != parent_spans:
                raise ValueError(f"oracle refinement for {parent_id} must match exact parent source spans {parent_spans}, got {refined_spans}")
        raw_oracle = refinement.get("oracle") or refinement.get("acceptance_oracle")
        oracle_text = format_oracle(raw_oracle) if raw_oracle else None
        if kind == "atomicity" and disposition == "reviewed-cohesive" and not oracle_text:
            member_oracles = []
            for member in refinement.get("members", []):
                if not isinstance(member, dict) or not member.get("oracle"):
                    member_oracles = []
                    break
                member_oracles.append(format_oracle(member["oracle"]))
            if member_oracles:
                oracle_text = "Parameterized cohesive oracle; each member is decided independently and all must pass:\n" + "\n".join(f"- {oracle}" for oracle in member_oracles)
        if oracle_text:
            parent["specific_acceptance_oracle"] = oracle_text
            parent["acceptance_test_or_benchmark"] = oracle_text
            parent["oracle_origin"] = "reviewed-gap-refinement"
            parent["oracle_status"] = "source-specific-oracle-present"
        if kind == "oracle":
            if disposition != "oracle-refined" and disposition not in {"explicit-research", "unsupported", "approval-gated", "specific-oracle still unresolved"}:
                raise ValueError(f"unexpected oracle disposition {disposition!r} for {parent_id}")
            if disposition == "specific-oracle still unresolved":
                parent["oracle_status"] = "needs-specific-oracle-review"
                parent["oracle_gap_reason"] = refinement.get("rationale", "no rationale")
            parent["oracle_refinement_id"] = refinement.get("refinement_id")
        else:
            if disposition == "split":
                children = refinement.get("children")
                if not isinstance(children, list) or not children:
                    raise ValueError(f"split refinement for {parent_id} must have children")
                parent["record_role"] = "aggregate-source-witness"
                parent["granularity_status"] = "aggregate-source-witness"
                parent["atomic_refinement_id"] = refinement.get("refinement_id")
                parent["atomic_child_source_ids"] = []
                for child in children:
                    child_id = child.get("source_id") or child.get("refinement_child_id") or child.get("child_id")
                    if not child_id or child_id in by_id or any(x["member_id"] == child_id for x in new_members):
                        raise ValueError(f"invalid/duplicate refinement child ID {child_id!r} under {parent_id}")
                    child_spans = ranges(child.get("source_lines") or child.get("source_ranges"))
                    if not child_spans:
                        raise ValueError(f"child {child_id} has no valid source_lines")
                    if any(not any(parent_start <= child_start and child_end <= parent_end for parent_start, parent_end in parent_spans) for child_start, child_end in child_spans):
                        raise ValueError(f"child {child_id} source range is outside parent {parent_id}: {child_spans} not within {parent_spans}")
                    source_path, raw, lines, sha = source_data({**parent, **child})
                    child_excerpt = excerpt_for(source_path, lines, child_spans)
                    stated_excerpt = child.get("source_excerpt") or child.get("exact_source_excerpt")
                    if stated_excerpt and not excerpt_matches(stated_excerpt, source_path, lines, child_spans):
                        raise ValueError(f"child {child_id} source excerpt mismatch")
                    statement = str(child.get("statement") or child.get("independent_outcome") or "").strip()
                    if not statement:
                        raise ValueError(f"child {child_id} lacks statement")
                    oracle = format_oracle(child.get("oracle"))
                    cls = child.get("normative_history_admin_class") or child.get("class") or refinement.get("normative_history_admin_class") or parent["normative_history_admin_class"]
                    group_id = child.get("grouping_id") or child.get("group")
                    canonical_groups = child.get("canonical_requirement_ids") or ([group_id] if group_id else None) or refinement.get("canonical_requirement_ids") or parent["canonical_requirement_ids"]
                    member = {
                        "member_id": child_id, "parent_source_id": parent_id,
                        "source_file_and_section": child.get("source_file_and_section", parent["source_file_and_section"]),
                        "source_path": source_path.name, "source_commit": parent.get("source_commit"),
                        "source_authority": parent.get("source_authority"),
                        "source_lines": child_spans, "source_excerpt": child_excerpt,
                        "source_sha256": sha, "statement": statement,
                        "user_outcome": child.get("user_outcome", statement),
                        "normative_history_admin_class": cls, "record_class": row_class(cls),
                        "canonical_requirement_ids": canonical_groups,
                        "primary_starter": child.get("primary_starter") or child.get("starter") or parent["primary_starter"],
                        "severity_or_security_class": child.get("severity_or_security_class", parent.get("severity_or_security_class")),
                        "provider_capabilities_and_permissions": child.get("provider_capabilities_and_permissions", parent.get("provider_capabilities_and_permissions")),
                        "specific_acceptance_oracle": oracle, "oracle_origin": "reviewed-gap-refinement",
                        "oracle_status": "source-specific-oracle-present", "granularity_status": "atomic-record",
                        "disposition": "atomic-member",
                        "parent_source_ids": [parent_id], "atomic_refinement_id": refinement.get("refinement_id"),
                    }
                    new_members.append(member)
                    parent.setdefault("atomic_members", []).append(member)
                    parent["atomic_child_source_ids"].append(child_id)
                parent["superseded_by_source_ids"] = list(parent["atomic_child_source_ids"])
                aggregate_oracle = (
                    "Aggregate source-witness oracle: the parent is covered only when each listed child record has a specific oracle and its evidence satisfies that child's expected invariant; "
                    "the traceability validator also checks that child source ranges remain within the parent source ranges and that no member is omitted. Child IDs: "
                    + ", ".join(parent["atomic_child_source_ids"])
                    + "."
                )
                parent["specific_acceptance_oracle"] = aggregate_oracle
                parent["acceptance_test_or_benchmark"] = aggregate_oracle
                parent["oracle_origin"] = "parameterized-child-oracle-aggregation"
                parent["oracle_status"] = "source-specific-oracle-present"
            elif disposition == "reviewed-cohesive":
                members = refinement.get("members")
                rationale = str(refinement.get("rationale", "")).strip()
                if not rationale:
                    rationale = str(refinement.get("cohesion_rationale", "")).strip()
                if not isinstance(members, list) or not members or not rationale:
                    raise ValueError(f"reviewed-cohesive parent {parent_id} needs enumerated members and rationale")
                member_texts = []
                for index, member in enumerate(members, 1):
                    if not isinstance(member, dict):
                        raise ValueError(f"cohesive member for {parent_id} must be a structured record")
                    member_text = str(member.get("statement") or member.get("independent_outcome") or "").strip()
                    member_spans = ranges(member.get("source_lines") or member.get("source_ranges"))
                    if not member_text or not member_spans:
                        raise ValueError(f"cohesive member for {parent_id} needs statement and exact source spans")
                    if any(not any(parent_start <= child_start and child_end <= parent_end for parent_start, parent_end in parent_spans) for child_start, child_end in member_spans):
                        raise ValueError(f"cohesive member span lies outside parent {parent_id}")
                    member_excerpt = excerpt_for(source_path, source_lines, member_spans)
                    stated_member_excerpt = member.get("exact_source_excerpt") or member.get("source_excerpt")
                    if stated_member_excerpt and not excerpt_matches(stated_member_excerpt, source_path, source_lines, member_spans):
                        raise ValueError(f"cohesive member excerpt mismatch for {parent_id}")
                    member_oracle = member.get("oracle")
                    if not member_oracle and len(members) == 1:
                        member_oracle = raw_oracle
                    if not member_oracle:
                        raise ValueError(f"cohesive member for {parent_id} lacks a member-specific oracle (or a single-member parent oracle)")
                    member_oracle_text = format_oracle(member_oracle)
                    member_id = member.get("member_id") or member.get("child_id") or f"{parent_id}-member-{index:03d}"
                    member_record = {
                        "member_id": member_id, "parent_source_id": parent_id,
                        "statement": member_text, "source_lines": member_spans,
                        "source_excerpt": member_excerpt, "source_path": source_path.name,
                        "source_sha256": parent.get("source_sha256"),
                        "source_commit": parent.get("source_commit"), "source_authority": parent.get("source_authority"),
                        "normative_history_admin_class": member.get("normative_history_admin_class") or member.get("class") or parent["normative_history_admin_class"],
                        "record_class": row_class(member.get("normative_history_admin_class") or member.get("class") or parent["normative_history_admin_class"]),
                        "canonical_requirement_ids": member.get("canonical_requirement_ids") or member.get("canonical_grouping_ids") or parent["canonical_requirement_ids"],
                        "primary_starter": member.get("primary_starter") or parent["primary_starter"],
                        "specific_acceptance_oracle": member_oracle_text, "oracle_origin": "reviewed-gap-refinement",
                        "oracle_status": "source-specific-oracle-present", "granularity_status":"reviewed-cohesive-parameterized-oracle",
                        "disposition": "cohesive-member",
                        "parent_source_ids": [parent_id], "atomic_refinement_id": refinement.get("refinement_id"),
                    }
                    member_texts.append(member_record)
                if not oracle_text:
                    raise ValueError(f"cohesive parent {parent_id} needs a specific oracle")
                parent["cohesive_members"] = member_texts
                parent["cohesive_rationale"] = rationale
                parent["granularity_status"] = "reviewed-cohesive-parameterized-oracle"
                parent["specific_acceptance_oracle"] = oracle_text
                parent["acceptance_test_or_benchmark"] = oracle_text
                parent["oracle_origin"] = "reviewed-gap-refinement"
                parent["oracle_status"] = "source-specific-oracle-present"
                parent["atomic_refinement_id"] = refinement.get("refinement_id")
            elif disposition in {"explicit-research", "unsupported", "approval-gated"}:
                rationale = str(refinement.get("rationale", "")).strip()
                if not rationale or not oracle_text:
                    raise ValueError(f"{disposition} {parent_id} requires rationale and a specific disposition oracle")
                parent["granularity_status"] = f"reviewed-{disposition}"
                parent["disposition_rationale"] = rationale
                parent["specific_acceptance_oracle"] = oracle_text
                parent["acceptance_test_or_benchmark"] = oracle_text
                parent["oracle_origin"] = "reviewed-gap-refinement"
                parent["oracle_status"] = "source-specific-oracle-present"
                parent["atomic_refinement_id"] = refinement.get("refinement_id")
            else:
                raise ValueError(f"atomicity refinement for {parent_id} has unsupported disposition {disposition!r}")

    missing_oracle = oracle_gaps - resolutions["oracle"]
    missing_atomic = atomic_gaps - resolutions["atomicity"]
    extra_oracle = resolutions["oracle"] - oracle_gaps
    extra_atomic = resolutions["atomicity"] - atomic_gaps
    if missing_oracle or missing_atomic or extra_oracle or extra_atomic:
        raise ValueError(f"gap inventory mismatch: missing oracle={sorted(missing_oracle)}, missing atomicity={sorted(missing_atomic)}, extra oracle={sorted(extra_oracle)}, extra atomicity={sorted(extra_atomic)}")
    # Members remain nested under their stable parent source ID. They are
    # independently testable logical outcomes, not invented source IDs.
    ids = [row["source_id"] for row in current]
    if len(ids) != len(set(ids)):
        dupes = [x for x,c in Counter(ids).items() if c>1]
        raise ValueError(f"compiled canonical source IDs duplicate: {dupes}")
    member_ids = [m["member_id"] for row in current for m in row.get("atomic_members", []) + row.get("cohesive_members", [])]
    if len(member_ids) != len(set(member_ids)):
        dupes = [x for x,c in Counter(member_ids).items() if c>1]
        raise ValueError(f"compiled member IDs duplicate: {dupes}")
    current.sort(key=lambda row: (row.get("source_path", ""), row.get("source_lines", [[0, 0]])[0][0], row["source_id"]))
    CANONICAL.write_text("".join(json.dumps(row, ensure_ascii=False, separators=(",", ":")) + "\n" for row in current), encoding="utf-8")
    print(f"compiled source IDs={len(current)} member outcomes={len(member_ids)} oracle parents={len(resolutions['oracle'])} atomicity parents={len(resolutions['atomicity'])}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
