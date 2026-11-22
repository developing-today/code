#!/usr/bin/env python3
"""Build the canonical, source-excerpted statement inventory from JSONL shards.

This preserves shards as inputs and writes statements.jsonl as a normalized view.
Run from repo root with python3 -I. The generated file is reproducible.
"""
from __future__ import annotations

import hashlib
import argparse
import json
import pathlib
import re
import sys
from collections import defaultdict


BASE = pathlib.Path(__file__).absolute().parent.parent
INV_DIR = BASE / "inventory"
SOURCE_ROOT = BASE / "source-snapshots" / "intake-34faffbff2" / "sources"
SOURCE_DIR = SOURCE_ROOT
SOURCE_MANIFEST = INV_DIR / "source_snapshot_manifest.json"
OUTPUT = INV_DIR / "statements.jsonl"


def no_duplicate_keys(pairs):
    obj = {}
    for key, value in pairs:
        if key in obj:
            raise ValueError(f"duplicate JSON key: {key}")
        obj[key] = value
    return obj


def ranges(value):
    if not isinstance(value, list):
        return []
    if len(value) == 2 and all(type(v) is int for v in value):
        value = [value]
    out = []
    for item in value:
        if isinstance(item, list) and len(item) == 2 and all(type(v) is int for v in item) and item[0] >= 1 and item[1] >= item[0]:
            out.append((item[0], item[1]))
    return out


def source_for(record):
    ref = str(record.get("source_file_and_section", "")).strip()
    for path in sorted(SOURCE_DIR.iterdir()):
        if path.is_file() and (ref.startswith(path.name) or ref.startswith(f"sources/{path.name}") or f"/sources/{path.name}" in ref):
            return path
    if ref.startswith("intake note.md") or ref.startswith("note.md"):
        return SOURCE_ROOT.parent / "note.md"
    raise ValueError(f"cannot resolve source_file_and_section for {record.get('source_id')}: {ref!r}")


def record_class(record):
    raw = str(record.get("normative_history_admin_class", "")).lower()
    sid = str(record.get("source_id", ""))
    if sid.startswith("EVIDENCE-CACHE-"):
        return "context/admin"
    # Input 3 is historical perspective; §47 supersedes it where reconciled.
    # Keep those source statements for traceability, not as binding requirements.
    if sid.startswith(("INPUT3-", "COMPANION-", "DECISION-", "IMPLEMENTER-", "README-")) or sid in {"ADD-0010", "RESEARCH-0001", "RESEARCH-0002", "RESEARCH-0004", "HANDOFF-0001", "HANDOFF-0002", "HANDOFF-0004", "COORD-ADMIN-0013"} or sid.startswith(("SPEC-R-hist-", "SPEC-R-nonclaim-", "SPEC-R-researchcitation", "SPEC-R-evidencestate")):
        return "historical"
    if any(token in raw for token in ("historical", "history", "non-normative")):
        return "historical"
    if any(token in raw for token in ("context", "admin", "formatting", "out-of-scope", "provenance", "metadata", "structural")):
        return "context/admin"
    return "normative-or-proposed"


def granularity_status(record):
    kind = str(record.get("coverage_kind", "")).lower()
    source_id = str(record.get("source_id", ""))
    if record_class(record) != "normative-or-proposed":
        return "explicit-history-context-admin"
    if any(token in kind for token in ("atomic", "semantic-resolution", "research inventory")):
        return "atomic-record"
    if source_id.startswith("RSEC-") or source_id.startswith("SPEC-C-"):
        return "atomic-record"
    m = re.fullmatch(r"SPEC-(\d{4})", source_id)
    if m and 28 <= int(m.group(1)) <= 83:
        return "atomic-record"
    if source_id.startswith("SPEC-R-"):
        return "atomic-record"
    if source_id.startswith("SPEC-PKI-") and source_id not in {"SPEC-PKI-029", "SPEC-PKI-073"}:
        return "register-atomic-requirement"
    if source_id.startswith(("SPEC-SEC-", "SPEC-GEN-", "SPEC-SSH-", "SPEC-IROH-", "SPEC-PKI-", "SPEC-OPS-", "SPEC-FUT-")):
        if source_id in {"SPEC-FUT-002", "SPEC-FUT-003", "SPEC-FUT-005", "SPEC-FUT-006", "SPEC-PKI-029", "SPEC-PKI2-013", "SPEC-PKI2-014"}:
            return "compound-register-row-needs-sibling-review"
        return "register-atomic-outcome"
    return "meaningful-source-group-needs-atomicity-review"


GENERIC_ORACLE = re.compile(
    r"independent acceptance oracle must|acceptance/evidence must be defined|"
    r"acceptance follows|not specified by source|confirm implementation only from pinned|"
    r"test must assert outcome|acceptance criteria.*to be defined",
    re.IGNORECASE,
)


def specific_oracle(record, source_excerpt):
    current = str(record.get("specific_acceptance_oracle") or record.get("acceptance_test_or_benchmark") or "").strip()
    if current and not GENERIC_ORACLE.search(current):
        return current, "record-specific-oracle"
    statement = str(record.get("statement") or record.get("user_outcome") or "").strip()
    if not statement:
        return current
    # Prefer an acceptance direction encoded in an actual table row (not just a
    # section number). In §27 the final cell is a concrete research deliverable.
    source_line = source_excerpt.splitlines()[0] if source_excerpt else ""
    cells = [cell.strip() for cell in source_line.strip().strip("|").split("|")]
    detail = cells[-1] if len(cells) >= 3 and cells[-1] and not set(cells[-1]) <= {"-", ":", " "} else ""
    neg = re.search(r"\b(never|must not|do not|reject|fail closed|no automatic|without)\b", statement, re.I)
    if detail and not re.fullmatch(r"(?:\d+(?:\.\d+)?(?:,\s*)?)+", detail):
        return f"Specific oracle for '{statement}': produce and inspect the cited evidence artifact '{detail}'; pass only when that artifact demonstrates each named condition and records an explicit unsupported/blocked result instead of treating missing evidence as success.", "source-row-specific-oracle"
    if neg:
        return f"Negative oracle for '{statement}': construct a synthetic fixture that violates this exact condition; the operation/verifier must reject it, preserve state, and emit the corresponding reason. A valid neighboring fixture is the control.", "statement-specific-negative-oracle"
    return f"Statement-specific oracle: on a synthetic fixture or bounded research artifact, verify the exact outcome '{statement}' against the cited source section; record the observable output/state and fail if any named field, capability, condition, or limitation is absent.", "generated-statement-oracle-needs-review"


def build():
    if not SOURCE_MANIFEST.is_file():
        raise ValueError(f"pinned source manifest missing: {SOURCE_MANIFEST}")
    source_manifest = json.loads(SOURCE_MANIFEST.read_text(encoding="utf-8"))
    expected_source_hashes = {row["source_file"]: row["sha256"] for row in source_manifest["source_files"]}
    expected_source_hashes.update({row["source_file"]: row["sha256"] for row in source_manifest.get("auxiliary_files", [])})
    records = []
    seen = set()
    source_cache = {}
    for shard in sorted(INV_DIR.glob("*.jsonl")):
        if shard == OUTPUT:
            continue
        if shard.name in {"spec00-13_lines045-299.jsonl", "spec14-24_atomicity.jsonl"}:
            # Earlier child outputs were written beside the canonical shards.
            # Retain them as historical drafts, but use reviewed replacements in
            # refinements/ as the only compiler inputs.
            raw_lines = shard.read_text(encoding="utf-8").splitlines()
            for line_no, text in enumerate(raw_lines, 1):
                try:
                    draft_row = json.loads(text, object_pairs_hook=no_duplicate_keys)
                except Exception as exc:
                    raise ValueError(f"{shard.name}:{line_no}: invalid/duplicate-key JSON: {exc}") from exc
                if not isinstance(draft_row, dict):
                    raise ValueError(f"{shard.name}:{line_no}: excluded draft record must be an object")
            if not (INV_DIR / "refinements" / shard.name).exists():
                raise ValueError(f"excluded draft {shard.name} has no reviewed refinement replacement")
            print(f"excluded historical assignment/refinement draft from canonical source inventory: {shard.name}")
            continue
        for line_no, text in enumerate(shard.read_text(encoding="utf-8").splitlines(), 1):
            try:
                record = json.loads(text, object_pairs_hook=no_duplicate_keys)
            except Exception as exc:
                raise ValueError(f"{shard.name}:{line_no}: invalid/duplicate-key JSON: {exc}") from exc
            if not isinstance(record, dict):
                raise ValueError(f"{shard.name}:{line_no}: record must be an object")
            # The 45–299 child left this work-routing manifest beside the
            # shards. It enumerates assigned parents, not source statements;
            # the actual refinements live under inventory/refinements/.
            if record.get("record_type") == "assignment_manifest":
                continue
            source_id = record.get("source_id")
            if not isinstance(source_id, str) or not source_id:
                raise ValueError(f"{shard.name}:{line_no}: missing stable source_id")
            if source_id in seen:
                raise ValueError(f"duplicate source_id {source_id}: {shard.name}:{line_no}")
            seen.add(source_id)
            if record.get("requirement_id") != source_id:
                raise ValueError(f"{shard.name}:{line_no}: requirement_id/source_id mismatch")
            source = source_for(record)
            if source not in source_cache:
                raw = source.read_bytes()
                expected_digest = expected_source_hashes.get(source.name)
                actual_digest = hashlib.sha256(raw).hexdigest()
                if expected_digest != actual_digest:
                    raise ValueError(f"pinned snapshot hash mismatch for {source.name}: expected {expected_digest}, got {actual_digest}")
                lines = raw.decode("utf-8").splitlines()
                source_cache[source] = (actual_digest, lines)
            digest, lines = source_cache[source]
            line_ranges = ranges(record.get("source_lines"))
            if not line_ranges:
                raise ValueError(f"{shard.name}:{line_no}: no valid source_lines")
            excerpts = []
            for start, end in line_ranges:
                if end > len(lines):
                    raise ValueError(f"{shard.name}:{line_no}: source range {start}-{end} exceeds {source.name} ({len(lines)} lines)")
                excerpts.append("\n".join(lines[start - 1:end]))
            canonical = dict(record)
            canonical["source_lines"] = [[a, b] for a, b in line_ranges]
            canonical["source_excerpt"] = "\n\n--- source span ---\n\n".join(excerpts)
            canonical["source_sha256"] = digest
            canonical["source_path"] = source.name
            canonical["source_commit"] = source_manifest["source_commit"]
            canonical["source_authority"] = "source-snapshots/intake-34faffbff2"
            canonical["record_class"] = record_class(record)
            canonical["specific_acceptance_oracle"], canonical["oracle_origin"] = specific_oracle(canonical, canonical["source_excerpt"])
            canonical["granularity_status"] = granularity_status(record)
            oracle = str(canonical.get("specific_acceptance_oracle") or canonical.get("acceptance_test_or_benchmark") or "")
            canonical["oracle_status"] = (
                "not-applicable-nonnormative"
                if record_class(record) != "normative-or-proposed"
                else "needs-specific-oracle-review"
                if not oracle or GENERIC_ORACLE.search(oracle) or canonical["oracle_origin"] == "generated-statement-oracle-needs-review"
                else "source-specific-oracle-present"
            )
            starter_text = str(canonical.get("primary_starter") or canonical.get("primary_issue_number") or "")
            starter_match = re.search(r"#\d+", starter_text)
            canonical["primary_starter"] = starter_match.group(0) if starter_match else None
            if not isinstance(canonical.get("canonical_requirement_ids"), list):
                raise ValueError(f"{shard.name}:{line_no}: canonical_requirement_ids must be an explicit list (empty only for context/history/admin)")
            if record_class(record) == "normative-or-proposed":
                if not canonical["canonical_requirement_ids"]:
                    raise ValueError(f"{shard.name}:{line_no}: normative record has no canonical_requirement_ids")
                if canonical.get("primary_starter") is None:
                    raise ValueError(f"{shard.name}:{line_no}: normative record has no primary_starter")
            records.append((canonical["source_path"], line_ranges[0][0], source_id, canonical))
    # A first-pass residual row can overlap a semantic-resolution record at the
    # same exact source line. Keep its stable ID as a historical witness, but do
    # not count it as a second live requirement.
    semantic_by_span = defaultdict(list)
    for _, _, _, record in records:
        if "semantic-resolution" in str(record.get("coverage_kind", "")):
            semantic_by_span[(record["source_path"], tuple(tuple(x) for x in record["source_lines"]))].append(record["source_id"])
    for _, _, _, record in records:
        if "requires semantic review" in str(record.get("normative_history_admin_class", "")).lower():
            key = (record["source_path"], tuple(tuple(x) for x in record["source_lines"]))
            resolutions = semantic_by_span.get(key, [])
            if resolutions:
                record["record_class"] = "historical"
                record["granularity_status"] = "superseded-residual-witness"
                record["superseded_by_source_ids"] = resolutions
                record["oracle_status"] = "not-applicable-superseded-witness"
    # Explicitly mark an aggregate §31 / Appendix A source-row record when
    # independently keyed atomic siblings cite that same register row.
    siblings_by_span = defaultdict(list)
    for _, _, _, record in records:
        if str(record.get("source_id", "")).startswith(("SPEC-Q-", "SPEC-A-PKI-", "SPEC-A-FUT-")):
            siblings_by_span[(record["source_path"], tuple(tuple(x) for x in record["source_lines"]))].append(record["source_id"])
    for _, _, _, record in records:
        sid = str(record.get("source_id", ""))
        if re.fullmatch(r"ADD-\d{4}", sid):
            matching = []
            spans = record["source_lines"]
            for _, _, _, candidate in records:
                if candidate["source_path"] != record["source_path"] or not str(candidate.get("source_id", "")).startswith("ADD-A-"):
                    continue
                if any(max(a, c) <= min(b, d) for a, b in spans for c, d in candidate["source_lines"]):
                    matching.append(candidate["source_id"])
            if matching:
                record["record_role"] = "aggregate-section-witness"
                record["granularity_status"] = "aggregate-section-witness"
                record["atomic_sibling_source_ids"] = matching
        if sid.startswith(("SPEC-SEC-", "SPEC-GEN-", "SPEC-SSH-", "SPEC-IROH-", "SPEC-PKI-", "SPEC-PKI2-", "SPEC-OPS-", "SPEC-FUT-")):
            siblings = siblings_by_span.get((record["source_path"], tuple(tuple(x) for x in record["source_lines"])), [])
            if siblings:
                record["record_role"] = "aggregate-register-witness"
                record["granularity_status"] = "aggregate-register-witness"
                record["atomic_sibling_source_ids"] = siblings
            else:
                record["record_role"] = "register-atomic-requirement"
        else:
            record.setdefault("record_role", "statement-or-explicit-context")
    linked_children = defaultdict(list)
    for _, _, _, record in records:
        for parent_id in record.get("parent_source_ids", []):
            linked_children[parent_id].append(record["source_id"])
    for _, _, _, record in records:
        child_ids = linked_children.get(record["source_id"], [])
        if child_ids:
            record["record_role"] = "aggregate-source-witness"
            record["granularity_status"] = "aggregate-source-witness"
            record["atomic_child_source_ids"] = child_ids
            if record["record_class"] == "normative-or-proposed":
                record["specific_acceptance_oracle"] = (
                    "Aggregate witness oracle: every child source ID listed here must have an independently reviewed statement and specific acceptance oracle; "
                    "the parent is complete only when each child disposition is present and no source member is omitted. Children: " + ", ".join(child_ids)
                )
                record["acceptance_test_or_benchmark"] = record["specific_acceptance_oracle"]
                record["oracle_origin"] = "linked-atomic-children"
                record["oracle_status"] = "source-specific-oracle-present"
    records.sort(key=lambda item: (item[0], item[1], item[2]))
    OUTPUT.write_text("".join(json.dumps(r, ensure_ascii=False, separators=(",", ":")) + "\n" for _, _, _, r in records), encoding="utf-8")
    print(f"wrote {len(records)} records to {OUTPUT.relative_to(BASE)}")


def main(argv=None):
    parser = argparse.ArgumentParser(
        prog="build_canonical_inventory.py",
        description="Build statements.jsonl from the pinned original source snapshot (no options).",
        allow_abbrev=False,
    )
    parser.parse_args(argv)
    build()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
