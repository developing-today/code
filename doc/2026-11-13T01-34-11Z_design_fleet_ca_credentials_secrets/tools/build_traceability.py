#!/usr/bin/env python3
"""Build source-proven, receipt-backed traceability rows without changing inputs."""
from __future__ import annotations

import argparse
import csv
from datetime import datetime
import hashlib
import importlib.util
import json
import re
import sys
from pathlib import Path
from typing import Any


COORDINATOR_COLUMNS = (
    "requirement_id", "source_file_and_section", "source_precedence_resolution",
    "user_outcome", "severity_or_security_class", "verified_current_behavior",
    "proposed_architecture_or_API", "primary_issue_number", "dependent_issue_numbers",
    "provider_capabilities_and_permissions", "acceptance_test_or_benchmark",
    "migration_or_rollback", "evidence_status", "current_status",
)
TRACE_COLUMNS = (
    "source_excerpt", "source_excerpt_sha256", "source_sha256", "source_lines",
    "source_hash_verified", "source_commit", "source_snapshot_commit", "source_snapshot_path",
    "source_path", "row_type", "member_id", "parent_source_id", "source_id",
    "source_statement", "source_specific_oracle", "expert_test_oracle",
    "canonical_requirement_id", "proposal_id",
    "design_section", "test_oracle", "disposition", "disposition_reason",
    "mapping_resolution_reason", "coverage_status",
    "oracle_quality", "coverage_ruling_id", "supporting_proposal_ids", "supporting_issue_numbers",
    "acceptance_enrichment",
    "domain_starter", "implementation_status", "issue_publication_status",
    "numeric_issue_receipt", "external_issue_number", "external_issue_url",
    "external_issue_title", "external_issue_state", "external_issue_verified_at",
    "external_issue_verified_from", "external_issue_evidence",
)
OUTPUT_COLUMNS = COORDINATOR_COLUMNS + TRACE_COLUMNS
MAP_FIELDS = {"source_id", "canonical_requirement_id", "proposal_id", "design_section",
              "test_oracle", "disposition", "reason"}
RESOLUTION_FIELDS = {"source_id", "selected_from", "selected_row_sha256", "reason"}
MEMBER_MAP_FIELDS = {"member_id", "parent_source_id", "canonical_requirement_id", "proposal_id",
                     "design_section", "test_oracle", "disposition", "reason"}
DISPOSITIONS = {"proposal", "mapped", "partial", "unresolved-cross-domain", "covered-with-limits",
                "research", "future", "approval-blocked", "unsupported", "existing",
                "superseded", "administrative", "historical", "external-issue", "unmapped"}
MAPPING_DISPOSITIONS = DISPOSITIONS - {"unmapped"}
PROPOSAL_REQUIRED = {"id", "title", "parent", "kind", "source_ranges", "requirement_ids",
                     "stage", "lifecycle", "urgency", "depends_on", "body"}
ISSUE_ID = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]{0,100}")
MARKER_PREFIX = "fleet-ca-v03-proposal:"
REPO = "developing-today/code"
LIVE_QUEUE_STATES = {"queued", "claimed", "parked", "blocked"}
EXTERNAL_ISSUE_NUMBERS = {9077, 6767, 2056}
EXTERNAL_ISSUE_FIELDS = {"number", "url", "title", "state", "verified_at", "verified_from", "evidence"}
GENERIC_ORACLE_MARKERS = (
    "independent issue states goal, dependencies",
    "implement measurable negative/positive tests from the stated constraint",
    "independent test oracle for this outcome",
    "independent-test-oracle-for-this-outcome",
    "tbd", "todo",
)
SOURCE_COMMIT = "34faffbff26615e313488f6d04705a444342e41e"
SOURCE_SNAPSHOT_COMMIT = "40af815bf0"
CANONICAL_INVENTORY_SHA256 = "e8158639602683cfb948e14fad4a774faa8169f6070f7c3f44e6e4062da86d58"
CANONICAL_SOURCE_COUNT = 1496
CANONICAL_MEMBER_COUNT = 1825
CANONICAL_GROUP_COUNT = 1345
CANONICAL_CLASS_COUNTS = {"normative-or-proposed": 935, "historical": 540, "context/admin": 21}
DEFAULT_SOURCE_ROOT = (Path(__file__).resolve().parent.parent /
                       "source-snapshots/intake-34faffbff2/sources")
_BINDING_SPEC = importlib.util.spec_from_file_location(
    "fleet_ca_traceability_binding", Path(__file__).resolve().parent.parent / "traceability" / "binding.py")
if not _BINDING_SPEC or not _BINDING_SPEC.loader:
    raise RuntimeError("cannot load shared traceability binding renderer")
traceability_binding = importlib.util.module_from_spec(_BINDING_SPEC)
_BINDING_SPEC.loader.exec_module(traceability_binding)


class TraceabilityError(Exception):
    pass


def _oracle_quality(source: dict[str, Any], mapping: dict[str, Any]) -> str:
    disposition = mapping["disposition"]
    oracle = mapping["test_oracle"].strip().lower()
    if disposition == "external-issue":
        return "external-ownership-not-implementation"
    if disposition in {"administrative", "historical", "superseded", "unmapped"} or oracle.startswith("not applicable:"):
        return "not-applicable"
    if disposition == "unresolved-cross-domain" or mapping["reason"].upper().startswith("UNRESOLVED/CROSS-DOMAIN:"):
        return "unresolved-no-acceptance-oracle"
    if any(marker in oracle for marker in GENERIC_ORACLE_MARKERS):
        expert = str(mapping.get("expert_test_oracle", "")).strip().lower()
        if not expert or any(marker in expert for marker in GENERIC_ORACLE_MARKERS):
            return "needs-review"
    raw = str(source.get("acceptance_test_or_benchmark", "")).strip().lower()
    if (oracle == raw and "needs-review" in str(source.get("oracle_origin", "")).lower()):
        return "needs-review"
    return "reviewed-specific-oracle"


def _verify_issue_body_binding(proposal: dict[str, Any], source_id: str, canonical_id: str,
                               source_statement: str, design_section: str,
                                test_oracle: str, reason: str, *, role: str = "primary",
                               acceptance_enrichment: list[str] | None = None,
                               row_type: str = "source-statement",
                               parent_source_id: str = "", source_specific_oracle: str = "",
                               expert_test_oracle: str = "") -> None:
    required_ids = {source_id, canonical_id}
    if not required_ids.issubset(set(proposal.get("requirement_ids", []))):
        raise TraceabilityError(f"{source_id}: enriched issue requirement_ids omit source/canonical identity")
    row = {"source_id": source_id, "member_id": source_id if row_type != "source-statement" else "",
            "parent_source_id": parent_source_id, "row_type": row_type,
            "canonical_requirement_id": canonical_id, "source_statement": source_statement,
            "design_section": design_section, "test_oracle": test_oracle,
            "source_specific_oracle": source_specific_oracle,
            "expert_test_oracle": expert_test_oracle,
           "disposition_reason": reason, "acceptance_enrichment": acceptance_enrichment or []}
    for value in traceability_binding.binding_tokens(row, role, [source_id], [parent_source_id] if parent_source_id else []):
        if not value or value not in proposal["body"]:
            raise TraceabilityError(f"{source_id}: enriched issue body lacks shared binding token {value!r}")


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(allow_abbrev=False)
    parser.add_argument("--inventory", action="append", required=True)
    parser.add_argument("--proposals", action="append", required=True)
    parser.add_argument("--mapping", action="append", required=True,
                        help="reviewed mapping file; may be repeated for domain shards")
    parser.add_argument("--member-mapping", action="append", default=[],
                        help="reviewed per-member mapping file; may be repeated")
    parser.add_argument("--mapping-resolution", action="append", default=[],
                        help="explicit selection for duplicate source IDs across mapping shards")
    parser.add_argument("--coverage-ruling", action="append", default=[],
                        help="expert-reviewed owner/gap ruling file; may be repeated")
    parser.add_argument("--receipts", required=True)
    parser.add_argument("--published-manifest",
                        help="exact normalized manifest used for issue publication (required in final mode)")
    parser.add_argument("--source-root", default=str(DEFAULT_SOURCE_ROOT),
                        help="pinned original source snapshot directory (never formatted sources/)")
    parser.add_argument("--output-dir")
    parser.add_argument("--overwrite", action="store_true",
                        help="replace only the three generated traceability output files")
    parser.add_argument("--final", action="store_true")
    args = parser.parse_args(argv)
    if args.overwrite and not args.output_dir:
        parser.error("--overwrite requires --output-dir")
    if args.final and not args.published_manifest:
        parser.error("--published-manifest is required with --final")
    return args


class _DuplicateKey(ValueError):
    pass


def _object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result = {}
    for key, value in pairs:
        if key in result:
            raise _DuplicateKey(f"duplicate JSON key {key!r}")
        result[key] = value
    return result


def _json_loads(text: str, where: str) -> Any:
    try:
        return json.loads(text, object_pairs_hook=_object)
    except (json.JSONDecodeError, _DuplicateKey) as exc:
        raise TraceabilityError(f"{where}: invalid or duplicate-key JSON: {exc}") from exc


def _read_jsonl(path: Path) -> list[dict[str, Any]]:
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except OSError as exc:
        raise TraceabilityError(f"cannot read {path}: {exc}") from exc
    result = []
    for number, line in enumerate(lines, 1):
        if not line.strip():
            continue
        row = _json_loads(line, f"{path}:{number}")
        if not isinstance(row, dict):
            raise TraceabilityError(f"{path}:{number}: expected JSON object")
        result.append(row)
    return result


def _json(path: Path) -> Any:
    try:
        text = path.read_text(encoding="utf-8")
    except OSError as exc:
        raise TraceabilityError(f"cannot read {path}: {exc}") from exc
    return _json_loads(text, str(path))


def _ranges(value: Any, where: str) -> list[tuple[int, int]]:
    if isinstance(value, list) and len(value) == 2 and all(type(n) is int for n in value):
        value = [value]
    if not isinstance(value, list) or not value:
        raise TraceabilityError(f"{where}: source_lines must contain inclusive ranges")
    ranges = []
    previous_end = 0
    for pair in value:
        if (not isinstance(pair, list) or len(pair) != 2 or
                any(type(n) is not int for n in pair) or pair[0] < 1 or pair[1] < pair[0]):
            raise TraceabilityError(f"{where}: malformed inclusive source range {pair!r}")
        if pair[0] <= previous_end:
            raise TraceabilityError(f"{where}: source ranges overlap or are out of order")
        ranges.append((pair[0], pair[1]))
        previous_end = pair[1]
    return ranges


def _text(value: Any) -> bool:
    return isinstance(value, str) and bool(value.strip())


def _range_label(value: Any) -> str:
    ranges = _ranges(value, "source reference")
    return ",".join(str(start) if start == end else f"{start}-{end}" for start, end in ranges)


def _source_proof(row: dict[str, Any], source_root: Path,
                  provenance_hashes: dict[str, str]) -> tuple[str, str, str]:
    sid = row.get("source_id", row.get("requirement_id"))
    where = f"{sid or '<missing source_id>'}"
    path_value = row.get("source_path")
    expected_file_hash = row.get("source_sha256")
    reference = row.get("source_file_and_section")
    if not _text(path_value) or not _text(reference):
        raise TraceabilityError(f"{where}: source_path and source_file_and_section are required")
    if row.get("source_commit") != SOURCE_COMMIT:
        raise TraceabilityError(f"{where}: source_commit does not match the pinned intake commit")
    rel = Path(path_value)
    if rel.is_absolute() or ".." in rel.parts:
        raise TraceabilityError(f"{where}: unsafe source_path {path_value!r}")
    if path_value == "note.md":
        # The pinned auxiliary evidence note is the sole documented exception
        # to the eleven-file source directory; arbitrary parent paths stay banned.
        if source_root.resolve() != DEFAULT_SOURCE_ROOT.resolve():
            raise TraceabilityError(f"{where}: note.md is allowed only under the pinned snapshot root")
        candidate = (source_root.parent / "note.md").resolve()
        allowed_root = source_root.parent.resolve()
    else:
        candidate = (source_root / rel).resolve()
        allowed_root = source_root.resolve()
    if not candidate.is_file() or not candidate.is_relative_to(allowed_root):
        raise TraceabilityError(f"{where}: source_path does not resolve inside pinned source root: {path_value}")
    if Path(path_value).name not in reference:
        raise TraceabilityError(f"{where}: source_path does not agree with source_file_and_section")
    if not isinstance(expected_file_hash, str) or not re.fullmatch(r"[0-9a-f]{64}", expected_file_hash):
        raise TraceabilityError(f"{where}: source_sha256 must be the recorded lowercase SHA-256 of the source file")
    canonical_hash = provenance_hashes.get(path_value)
    if canonical_hash is None:
        raise TraceabilityError(f"{where}: source_path is absent from pinned source provenance")
    if expected_file_hash != canonical_hash:
        raise TraceabilityError(f"{where}: inventory source_sha256 differs from pinned provenance")
    try:
        raw = candidate.read_bytes()
        content = raw.decode("utf-8")
    except (OSError, UnicodeDecodeError) as exc:
        raise TraceabilityError(f"{where}: cannot read source bytes as UTF-8: {exc}") from exc
    actual_file_hash = hashlib.sha256(raw).hexdigest()
    if actual_file_hash != expected_file_hash:
        raise TraceabilityError(f"{where}: current source file hash differs from source_sha256")
    ranges = _ranges(row.get("source_lines"), where)
    lines = content.splitlines()
    try:
        spans = ["\n".join(lines[number - 1] for number in range(start, end + 1))
                 for start, end in ranges]
    except IndexError as exc:
        raise TraceabilityError(f"{where}: source range exceeds current source file") from exc
    excerpt = "\n\n--- source span ---\n\n".join(spans)
    given_excerpt = row.get("source_excerpt")
    if not isinstance(given_excerpt, str) or given_excerpt != excerpt:
        raise TraceabilityError(f"{where}: source_excerpt does not exactly match source_path/source_lines")
    digest = hashlib.sha256(excerpt.encode("utf-8")).hexdigest()
    given_excerpt_hash = row.get("source_excerpt_sha256")
    if given_excerpt_hash is not None and given_excerpt_hash != digest:
        raise TraceabilityError(f"{where}: source_excerpt_sha256 does not match exact source bytes")
    return excerpt, digest, actual_file_hash


def _source_provenance(package_root: Path, source_root: Path) -> dict[str, str]:
    provenance_path = package_root / "source-snapshots" / "provenance.json"
    provenance = _json(provenance_path)
    if not isinstance(provenance, dict) or provenance.get("source_commit") != SOURCE_COMMIT:
        raise TraceabilityError("source provenance does not identify the pinned intake commit")
    records = provenance.get("files")
    if not isinstance(records, list):
        raise TraceabilityError("source provenance files must be a list")
    expected = {}
    for record in records:
        if not isinstance(record, dict):
            raise TraceabilityError("source provenance contains a non-object file record")
        filename, digest = record.get("source_file"), record.get("original_sha256")
        recorded_path = record.get("snapshot_path")
        if (not isinstance(filename, str) or not filename or
                not isinstance(digest, str) or not re.fullmatch(r"[0-9a-f]{64}", digest) or
                not isinstance(recorded_path, str) or record.get("source_commit") != SOURCE_COMMIT):
            raise TraceabilityError("source provenance record lacks exact file, hash, or snapshot path")
        recorded = Path(recorded_path)
        repo_relative = (Path(__file__).resolve().parents[3] / recorded).resolve()
        package_relative = (package_root / recorded).resolve()
        expected_path = (source_root / filename).resolve()
        if expected_path not in {repo_relative, package_relative}:
            raise TraceabilityError(f"source provenance path for {filename} differs from --source-root")
        if filename in expected:
            raise TraceabilityError(f"duplicate source provenance record for {filename}")
        expected[filename] = digest
    auxiliary = provenance.get("auxiliary_intake_note")
    if auxiliary is not None:
        if (not isinstance(auxiliary, dict) or auxiliary.get("source_file") != "note.md" or
                auxiliary.get("source_commit") != SOURCE_COMMIT or
                not isinstance(auxiliary.get("original_sha256"), str) or
                not re.fullmatch(r"[0-9a-f]{64}", auxiliary["original_sha256"])):
            raise TraceabilityError("auxiliary intake note provenance is malformed")
        auxiliary_path = (Path(__file__).resolve().parents[3] / auxiliary.get("snapshot_path", "")).resolve()
        if (source_root.resolve() != DEFAULT_SOURCE_ROOT.resolve() or
                auxiliary_path != (source_root.parent / "note.md").resolve()):
            raise TraceabilityError("auxiliary note is only permitted at the provenance-whitelisted snapshot path")
        expected["note.md"] = auxiliary["original_sha256"]
    elif source_root.resolve() == DEFAULT_SOURCE_ROOT.resolve():
        raise TraceabilityError("pinned source provenance lacks its auxiliary note record")
    return expected


def _mapping_file(path: str) -> list[dict[str, Any]]:
    doc = _json(Path(path))
    if isinstance(doc, dict) and set(doc) == {"rows"}:
        doc = doc["rows"]
    elif isinstance(doc, dict) and set(doc) == {"version", "rows"} and doc.get("version") == 1:
        doc = doc["rows"]
    if not isinstance(doc, list):
        raise TraceabilityError(f"{path}: mapping must be a JSON array or version-1 rows object")
    return doc


def _validate_external_issue(external: Any, reason: str, where: str) -> None:
    if not isinstance(external, dict) or set(external) != EXTERNAL_ISSUE_FIELDS:
        raise TraceabilityError(f"{where}.external_issue fields must be exactly {sorted(EXTERNAL_ISSUE_FIELDS)}")
    number = external["number"]
    if type(number) is not int or number not in EXTERNAL_ISSUE_NUMBERS:
        raise TraceabilityError(f"{where}: external issue number is not on the intake whitelist")
    if external["url"] != f"https://github.com/{REPO}/issues/{number}":
        raise TraceabilityError(f"{where}: external issue URL must match its whitelisted number")
    if (not _text(external["title"]) or not isinstance(external["state"], str) or
            external["state"] not in {"OPEN", "CLOSED"}):
        raise TraceabilityError(f"{where}: external issue title/state must be verified")
    if not _text(external["verified_from"]) or not _text(external["evidence"]):
        raise TraceabilityError(f"{where}: external issue needs verified_from and exact-scope evidence")
    try:
        verified_at = datetime.fromisoformat(external["verified_at"].replace("Z", "+00:00"))
    except (AttributeError, TypeError, ValueError) as exc:
        raise TraceabilityError(f"{where}: external issue verified_at must be an ISO timestamp") from exc
    if verified_at.tzinfo is None:
        raise TraceabilityError(f"{where}: external issue verified_at must include a timezone")
    if not reason.startswith("EXTERNAL-ISSUE:"):
        raise TraceabilityError(f"{where}: external issue reason must begin EXTERNAL-ISSUE:")


def _validate_existing_census(row: dict[str, Any], where: str) -> None:
    prefix = "census:"
    evidence_path = row["design_section"][len(prefix):] if row["design_section"].startswith(prefix) else ""
    rel = Path(evidence_path)
    if not evidence_path or rel.is_absolute() or ".." in rel.parts:
        raise TraceabilityError(f"{where}: existing-code disposition requires census:<repo-relative-path>")
    repo_root = Path(__file__).resolve().parents[3]
    candidate = (repo_root / rel).resolve()
    if not candidate.is_file() or not candidate.is_relative_to(repo_root.resolve()):
        raise TraceabilityError(f"{where}: existing-code census path is not a repository file")
    if not row["reason"].startswith("EXISTING-CODE:") or evidence_path not in row["reason"]:
        raise TraceabilityError(f"{where}: existing-code reason must cite its exact census path")


def _load_mapping_resolutions(paths: list[str]) -> dict[str, dict[str, Any]]:
    result = {}
    for path in paths:
        doc = _json(Path(path))
        if not isinstance(doc, dict) or set(doc) != {"version", "resolutions"} or doc["version"] != 1 or not isinstance(doc["resolutions"], list):
            raise TraceabilityError(f"{path}: resolutions must be version 1 with a resolutions array")
        for index, row in enumerate(doc["resolutions"]):
            where = f"{path}.resolutions[{index}]"
            if not isinstance(row, dict) or set(row) != RESOLUTION_FIELDS:
                raise TraceabilityError(f"{where}: fields must be exactly {sorted(RESOLUTION_FIELDS)}")
            sid = row["source_id"]
            if not _text(sid) or sid in result:
                raise TraceabilityError(f"{where}: source_id must be nonempty and unique")
            if not _text(row["selected_from"]) or not _text(row["reason"]):
                raise TraceabilityError(f"{where}: selected_from and reason must be nonempty")
            if not isinstance(row["selected_row_sha256"], str) or not re.fullmatch(r"[0-9a-f]{64}", row["selected_row_sha256"]):
                raise TraceabilityError(f"{where}: selected_row_sha256 must be lowercase SHA-256")
            result[sid] = row
    return result


def _mapping_row_digest(row: dict[str, Any]) -> str:
    return hashlib.sha256(json.dumps(row, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()).hexdigest()


def _validate_mapping(paths: list[str], *, final: bool,
                      resolution_paths: list[str] | None = None
                      ) -> tuple[dict[str, dict[str, Any]], dict[str, list[dict[str, Any]]], dict[str, dict[str, Any]]]:
    mapping: dict[str, dict[str, Any]] = {}
    conflicts: dict[str, list[dict[str, Any]]] = {}
    candidate_files: dict[str, list[str]] = {}
    resolutions = _load_mapping_resolutions(resolution_paths or [])
    resolved: dict[str, dict[str, Any]] = {}
    for path in paths:
        for index, row in enumerate(_mapping_file(path)):
            where = f"{path}[{index}]"
            if not isinstance(row, dict):
                raise TraceabilityError(f"{where}: mapping row must be an object")
            expected_fields = MAP_FIELDS | ({"external_issue"} if row.get("disposition") == "external-issue" else set())
            if set(row) != expected_fields:
                raise TraceabilityError(f"{where}: fields must be exactly {sorted(expected_fields)}")
            sid = row["source_id"]
            if not _text(sid):
                raise TraceabilityError(f"{where}: source_id must be non-empty")
            if not isinstance(row["disposition"], str) or row["disposition"] not in MAPPING_DISPOSITIONS:
                raise TraceabilityError(f"{where}: invalid disposition {row['disposition']!r}")
            required_text = MAP_FIELDS - {"proposal_id", "canonical_requirement_id"}
            if any(not _text(row[field]) for field in required_text):
                raise TraceabilityError(f"{where}: mapping fields except proposal_id and context canonical ID must be non-empty")
            if not isinstance(row["canonical_requirement_id"], str):
                raise TraceabilityError(f"{where}.canonical_requirement_id must be an exact string")
            if (not row["canonical_requirement_id"] and
                    row["disposition"] not in {"administrative", "historical", "superseded"}):
                raise TraceabilityError(f"{where}: empty canonical ID is allowed only for explicit admin/history/superseded context")
            if row["disposition"] in {"proposal", "mapped", "partial", "covered-with-limits"} and not _text(row["proposal_id"]):
                raise TraceabilityError(f"{where}: implementation-related disposition requires proposal_id")
            if not isinstance(row["proposal_id"], str):
                raise TraceabilityError(f"{where}: proposal_id must be an exact string (empty only for no proposal)")
            if row["disposition"] != "proposal" and row["proposal_id"] and not _text(row["proposal_id"]):
                raise TraceabilityError(f"{where}: proposal_id must be empty or a non-empty string")
            if row["disposition"] == "external-issue":
                if row["proposal_id"]:
                    raise TraceabilityError(f"{where}: external issue must not be represented as a proposal ID")
                _validate_external_issue(row["external_issue"], row["reason"], where)
            if row["disposition"] == "existing":
                _validate_existing_census(row, where)
            candidate_files.setdefault(sid, []).append(Path(path).name)
            if sid in conflicts:
                conflicts[sid].append(row)
            elif sid in mapping:
                conflicts[sid] = [mapping.pop(sid), row]
            else:
                mapping[sid] = row
    stale_resolutions = set(resolutions) - set(conflicts)
    if stale_resolutions:
        raise TraceabilityError(f"mapping resolutions do not correspond to duplicate source IDs: {sorted(stale_resolutions)}")
    for sid, resolution in resolutions.items():
        candidates = [(filename, row) for filename, row in zip(candidate_files[sid], conflicts[sid])
                      if filename == resolution["selected_from"] and
                      _mapping_row_digest(row) == resolution["selected_row_sha256"]]
        if len(candidates) != 1:
            raise TraceabilityError(f"{sid}: resolution does not select one exact row from {resolution['selected_from']}")
        mapping[sid] = candidates[0][1]
        del conflicts[sid]
        resolved[sid] = resolution
    if final and conflicts:
        raise TraceabilityError(f"final matrix has unresolved duplicate mappings: {sorted(conflicts)}")
    return mapping, conflicts, resolved


def _validate_member_mappings(paths: list[str]) -> dict[str, dict[str, Any]]:
    mapping = {}
    for path in paths:
        for index, row in enumerate(_mapping_file(path)):
            where = f"{path}[{index}]"
            if not isinstance(row, dict):
                raise TraceabilityError(f"{where}: member mapping must be an object")
            expected = MEMBER_MAP_FIELDS | ({"external_issue"} if row.get("disposition") == "external-issue" else set())
            if set(row) != expected:
                raise TraceabilityError(f"{where}: member mapping fields must be exactly {sorted(expected)}")
            member_id = row["member_id"]
            if not _text(member_id) or member_id in mapping:
                raise TraceabilityError(f"{where}: member_id must be nonempty and unique across member shards")
            if not _text(row["parent_source_id"]):
                raise TraceabilityError(f"{where}: parent_source_id must be nonempty")
            if not isinstance(row["disposition"], str) or row["disposition"] not in MAPPING_DISPOSITIONS:
                raise TraceabilityError(f"{where}: invalid member disposition {row['disposition']!r}")
            if any(not _text(row[field]) for field in MEMBER_MAP_FIELDS - {"proposal_id"}):
                raise TraceabilityError(f"{where}: all member mapping fields except proposal_id must be nonempty")
            if row["disposition"] in {"proposal", "mapped", "partial", "covered-with-limits"} and not _text(row["proposal_id"]):
                raise TraceabilityError(f"{where}: implementation-related member disposition requires proposal_id")
            if not isinstance(row["proposal_id"], str):
                raise TraceabilityError(f"{where}: member proposal_id must be an exact string")
            if row["disposition"] == "external-issue":
                if row["proposal_id"]:
                    raise TraceabilityError(f"{where}: external member issue cannot be represented as a proposal ID")
                _validate_external_issue(row["external_issue"], row["reason"], where)
            if row["disposition"] == "existing":
                _validate_existing_census(row, where)
            mapping[member_id] = row
    return mapping


def _apply_coverage_rulings(paths: list[str], source_by_id: dict[str, dict[str, Any]],
                            member_by_id: dict[str, tuple[str, dict[str, Any]]],
                            mapping: dict[str, dict[str, Any]],
                            member_mapping: dict[str, dict[str, Any]],
                            proposals: dict[str, dict[str, Any]]) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    audit = []
    applied = set()

    def get_record(identifier: str) -> tuple[dict[str, Any], dict[str, Any], bool]:
        if identifier in source_by_id:
            if identifier not in mapping:
                raise TraceabilityError(f"coverage ruling references source ID without a mapping row: {identifier}")
            return source_by_id[identifier], mapping[identifier], False
        if identifier in member_by_id:
            parent, member = member_by_id[identifier]
            if identifier not in member_mapping:
                raise TraceabilityError(f"coverage ruling references member ID without a mapping row: {identifier}")
            if member_mapping[identifier]["parent_source_id"] != parent:
                raise TraceabilityError(f"coverage ruling member parent mismatch for {identifier}")
            return member, member_mapping[identifier], True
        raise TraceabilityError(f"coverage ruling references unknown source/member ID: {identifier}")

    for path in paths:
        doc = _json(Path(path))
        if not isinstance(doc, list):
            raise TraceabilityError(f"{path}: expert coverage rulings must be a JSON array")
        for index, ruling in enumerate(doc):
            where = f"{path}[{index}]"
            if not isinstance(ruling, dict):
                raise TraceabilityError(f"{where}: ruling must be an object")
            if "source_or_member_id" in ruling:
                rid = ruling["source_or_member_id"]
                if not _text(rid) or rid in applied:
                    raise TraceabilityError(f"{where}: source/member ruling ID must be nonempty and unique")
                source, existing, is_member = get_record(rid)
                primary = ruling.get("primary_proposal_id")
                supports = ruling.get("supporting_proposal_ids")
                design = ruling.get("design_section")
                owner_oracle = ruling.get("test_oracle", ruling.get("planned_test_oracle"))
                rationale = ruling.get("coverage_rationale")
                enrich = ruling.get("acceptance_enrichment", [])
                if not _text(primary) or primary not in proposals:
                    raise TraceabilityError(f"{where}: primary_proposal_id must resolve to an exact local proposal")
                if (not isinstance(supports, list) or any(not _text(item) or item not in proposals for item in supports)
                        or primary in supports or len(set(supports)) != len(supports)):
                    raise TraceabilityError(f"{where}: supporting_proposal_ids must be distinct known local proposal IDs")
                if not _text(design) or not _text(owner_oracle) or not _text(rationale):
                    raise TraceabilityError(f"{where}: design_section, test oracle, and coverage rationale are required")
                if not isinstance(enrich, list) or any(not _text(item) for item in enrich):
                    raise TraceabilityError(f"{where}: acceptance_enrichment must be a list of explicit clauses")
                source_oracle = source.get("specific_acceptance_oracle")
                oracle_parts = []
                if _text(source_oracle):
                    oracle_parts.append("Source-specific acceptance oracle (preserved verbatim):\n" + source_oracle)
                if owner_oracle != source_oracle:
                    oracle_parts.append("Expert planned coverage oracle:\n" + owner_oracle)
                combined_oracle = "\n\n".join(oracle_parts) if oracle_parts else owner_oracle
                rationale_upper = rationale.upper()
                if rationale_upper.startswith("PLANNED/PARTIAL:"):
                    disposition = "partial"
                    reason = ("PARTIAL: acceptance-enrichment-needed: " if enrich else "PARTIAL: ") + rationale
                elif rationale_upper.startswith(("RESEARCH-CONTRACT", "RESEARCH/PLANNED:")):
                    disposition = "research"
                    reason = "RESEARCH-OWNER: " + rationale
                elif rationale_upper.startswith(("FUTURE/NONBLOCKING:", "FUTURE/PLANNED:")):
                    disposition = "future"
                    reason = "FUTURE-OWNER: " + rationale
                elif rationale_upper.startswith("PLANNED/COVERED WITH LIMITS:"):
                    disposition = "covered-with-limits"
                    reason = "PARTIAL: " + rationale
                elif rationale_upper.startswith("PLANNED:"):
                    disposition = "proposal"
                    reason = "COVERED: " + rationale
                else:
                    # Presence in this expert-authored ruling file is itself an
                    # explicit planned-owner decision, not a lexical mapping.
                    disposition = "proposal"
                    reason = "COVERED: Expert owner ruling (planned, not implemented): " + rationale
                canonical = existing["canonical_requirement_id"]
                canonical_ids = source.get("canonical_requirement_ids", [])
                if not canonical and len(canonical_ids) == 1:
                    canonical = canonical_ids[0]
                if canonical not in canonical_ids:
                    raise TraceabilityError(f"{where}: ruling has no exact canonical requirement binding")
                updated = dict(existing)
                updated.update({"canonical_requirement_id": canonical, "proposal_id": primary,
                                "design_section": design, "test_oracle": combined_oracle,
                                "expert_test_oracle": owner_oracle,
                                "disposition": disposition, "reason": reason,
                                "supporting_proposal_ids": supports,
                                "acceptance_enrichment": enrich, "coverage_ruling_id": Path(path).name})
                if is_member:
                    member_mapping[rid] = updated
                else:
                    mapping[rid] = updated
                applied.add(rid)
                audit.append({"id": rid, "ruling_file": Path(path).name, "disposition": disposition,
                              "primary_proposal_id": primary, "supporting_proposal_ids": supports,
                              "acceptance_enrichment": enrich})
            elif "gap_id" in ruling:
                gap_id = ruling.get("gap_id")
                members = ruling.get("source_members")
                why = ruling.get("why_no_owner")
                resolved_by = ruling.get("resolved_by")
                if not _text(gap_id) or not _text(why) or not isinstance(members, list) or not members:
                    why = ruling.get("why_no_existing_packet_owns_it", why)
                if not _text(gap_id) or not _text(why) or not isinstance(members, list) or not members:
                    raise TraceabilityError(f"{where}: gap ruling requires id, exact why-no-owner rationale and source_members")
                if resolved_by and resolved_by not in proposals:
                    raise TraceabilityError(f"{where}: resolved_by references unknown exact new proposal {resolved_by}")
                for member_ref in members:
                    if not isinstance(member_ref, dict) or not _text(member_ref.get("source_or_member_id")):
                        raise TraceabilityError(f"{where}: malformed source_members entry")
                    rid = member_ref["source_or_member_id"]
                    if rid in applied:
                        raise TraceabilityError(f"{where}: duplicate expert ruling for {rid}")
                    source, existing, is_member = get_record(rid)
                    expected_parent = member_ref.get("parent_source_id")
                    if is_member and expected_parent and source.get("parent_source_id") != expected_parent:
                        raise TraceabilityError(f"{where}: gap parent source mismatch for {rid}")
                    canonical_ids = source.get("canonical_requirement_ids", [])
                    canonical = existing["canonical_requirement_id"]
                    if canonical and canonical not in canonical_ids:
                        raise TraceabilityError(f"{where}: existing canonical ID is not valid for {rid}")
                    source_oracle = source.get("specific_acceptance_oracle")
                    oracle = source_oracle if _text(source_oracle) else existing["test_oracle"]
                    source_ref = member_ref.get("source_ref", existing["design_section"])
                    clause = member_ref.get("clause", source.get("statement", ""))
                    if resolved_by:
                        design_file = {"TRACE-03": "census.json", "ARCH-11": "architecture.json",
                                       "ARCH-12": "architecture.json"}.get(resolved_by)
                        if not design_file:
                            raise TraceabilityError(f"{where}: no exact proposal file binding for resolved_by {resolved_by}")
                        disposition = "proposal"
                        design_section = f"proposals/{design_file}#{resolved_by} §Outcome"
                        reason = (f"COVERED: Parent expert ruling assigns {gap_id} to planned owner {resolved_by}; "
                                  f"this is planned design/implementation coverage, not implemented behavior. Exact clause: {clause}")
                        enrichments = [f"Acceptance enrichment from {gap_id}: {clause}"]
                        proposal_id = resolved_by
                    else:
                        disposition = "unresolved-cross-domain"
                        design_section = f"unresolved: {gap_id} — {source_ref}"
                        reason = f"UNRESOLVED/CROSS-DOMAIN: {gap_id}: {why} Exact clause: {clause}"
                        enrichments = []
                        proposal_id = ""
                    updated = dict(existing)
                    updated.update({"canonical_requirement_id": canonical, "proposal_id": proposal_id,
                                    "design_section": design_section, "test_oracle": oracle,
                                    "disposition": disposition, "reason": reason,
                                    "supporting_proposal_ids": [], "acceptance_enrichment": enrichments,
                                    "coverage_ruling_id": gap_id})
                    if is_member:
                        member_mapping[rid] = updated
                    else:
                        mapping[rid] = updated
                    applied.add(rid)
                    audit.append({"id": rid, "ruling_file": Path(path).name,
                                  "disposition": disposition, "gap_id": gap_id,
                                  "resolved_by": resolved_by or "", "acceptance_enrichment": enrichments})
            else:
                raise TraceabilityError(f"{where}: unrecognized expert coverage ruling shape")
    return audit, sorted(applied)


def _proposals(paths: list[str], *, final: bool) -> dict[str, dict[str, Any]]:
    proposals = {}
    for path in paths:
        doc = _json(Path(path))
        if not isinstance(doc, list):
            raise TraceabilityError(f"{path}: proposals must be a JSON array")
        for index, proposal in enumerate(doc):
            where = f"{path}[{index}]"
            if not isinstance(proposal, dict):
                raise TraceabilityError(f"{where}: proposal must be an object")
            if final and set(proposal) != PROPOSAL_REQUIRED:
                raise TraceabilityError(f"{where}: final published proposal must have exactly the publisher's 11 fields")
            if not PROPOSAL_REQUIRED <= proposal.keys():
                raise TraceabilityError(f"{where}: missing shared proposal schema fields")
            pid = proposal["id"]
            if not _text(pid) or not ISSUE_ID.fullmatch(pid) or pid in proposals:
                raise TraceabilityError(f"{where}: duplicate or invalid proposal id {pid!r}")
            if not isinstance(proposal["depends_on"], list) or any(not _text(dep) for dep in proposal["depends_on"]):
                raise TraceabilityError(f"{where}.depends_on must be a list of exact proposal IDs")
            proposals[pid] = proposal
    for pid, proposal in proposals.items():
        deps = proposal["depends_on"]
        if len(set(deps)) != len(deps) or pid in deps:
            raise TraceabilityError(f"{pid}: duplicate or self dependency")
        missing = set(deps) - proposals.keys()
        if missing:
            raise TraceabilityError(f"{pid}: unknown proposal dependencies {sorted(missing)}")
    pending, completed = set(proposals), set()
    while pending:
        ready = {pid for pid in pending if set(proposals[pid]["depends_on"]).issubset(completed)}
        if not ready:
            raise TraceabilityError("proposal depends_on contains a cycle")
        completed.update(ready)
        pending.difference_update(ready)
    return proposals


def _queue_state(readback: Any, task_id: int) -> str | None:
    if not isinstance(readback, str):
        return None
    for line in readback.splitlines():
        columns = line.split()
        if len(columns) >= 4 and columns[0] == "task" and columns[1] == str(task_id):
            return columns[3]
    return None


def _verified_publication(receipt_path: str, manifest_path: str,
                          proposals: dict[str, dict[str, Any]]) -> dict[str, dict[str, Any]]:
    manifest = _json(Path(manifest_path))
    if not isinstance(manifest, list) or manifest != [proposals[pid] for pid in proposals]:
        raise TraceabilityError("published manifest must exactly match the final proposal input arrays and order")
    publisher_path = Path(__file__).with_name("publish_issues.py")
    spec = importlib.util.spec_from_file_location("fleet_ca_publish_issues_for_receipt_validation", publisher_path)
    if not spec or not spec.loader:
        raise TraceabilityError("cannot load the shared issue publisher receipt contract")
    publisher = importlib.util.module_from_spec(spec)
    try:
        spec.loader.exec_module(publisher)
        publisher_proposals = publisher.validate_manifest(manifest)
        plan = publisher.build_plan(manifest, publisher_proposals, "publish")
        state = publisher.read_receipt(Path(receipt_path), plan, publisher_proposals)
    except Exception as exc:
        raise TraceabilityError(f"publisher receipt validation failed: {exc}") from exc
    if not state.get("records"):
        raise TraceabilityError("final mode requires a nonempty publisher receipt")
    if any(record.get("event") in {"failure", "queue_unverified"} for record in state["records"]):
        raise TraceabilityError("publisher receipt contains failure or queue_unverified evidence")
    proposal_ids = set(proposals)
    if set(state.get("issue_verified", {})) != proposal_ids:
        missing = sorted(proposal_ids - set(state.get("issue_verified", {})))
        raise TraceabilityError(f"publisher receipt lacks issue_verified evidence for {missing}")
    if set(state.get("task_verified", {})) != proposal_ids:
        missing = sorted(proposal_ids - set(state.get("task_verified", {})))
        raise TraceabilityError(f"publisher receipt lacks task_verified evidence for {missing}")
    if state.get("queue_unverified"):
        raise TraceabilityError("publisher receipt contains unresolved queue read-back failures")
    verified = {}
    for pid in proposals:
        issue_event = state["issue_verified"][pid]
        task_event = state["task_verified"][pid]
        issue = issue_event["issue"]
        task = task_event["task"]
        row = task_event["readback_scope"]["matched_row"]
        verified[pid] = {
            "number": issue["number"], "database_id": issue["database_id"],
            "blocked_by": [edge["number"] for edge in issue["blocked_by"]],
            "publication_status": "published", "task_id": task["id"],
            "task_status": task["status"], "queue_row": row,
        }
    return verified


def _validate_canonical_freeze(inventory_paths: list[str],
                               records: list[dict[str, Any]],
                               members: list[tuple[str, str, dict[str, Any]]]) -> None:
    if len(inventory_paths) != 1:
        raise TraceabilityError("final mode requires the sole canonical statements.jsonl input, not caller-selected subsets")
    package_root = Path(inventory_paths[0]).resolve().parent.parent
    expected_path = (package_root / "inventory/statements.jsonl").resolve()
    input_path = Path(inventory_paths[0]).resolve()
    if input_path != expected_path:
        raise TraceabilityError("final mode requires inventory/statements.jsonl from the canonical frozen package")
    digest = hashlib.sha256(input_path.read_bytes()).hexdigest()
    if digest != CANONICAL_INVENTORY_SHA256:
        raise TraceabilityError("final inventory SHA-256 differs from CANONICAL_FREEZE; regenerate mappings against the new explicit freeze")
    source_ids = {row.get("source_id", row.get("requirement_id")) for row in records}
    member_ids = {member.get("member_id") for _, _, member in members}
    if len(records) != CANONICAL_SOURCE_COUNT or len(source_ids) != CANONICAL_SOURCE_COUNT:
        raise TraceabilityError(f"final canonical inventory must contain {CANONICAL_SOURCE_COUNT} unique source IDs")
    if len(members) != CANONICAL_MEMBER_COUNT or len(member_ids) != CANONICAL_MEMBER_COUNT:
        raise TraceabilityError(f"final canonical inventory must contain {CANONICAL_MEMBER_COUNT} unique nested member IDs")
    classes: dict[str, int] = {}
    for row in records:
        key = row.get("record_class")
        classes[key] = classes.get(key, 0) + 1
    if classes != CANONICAL_CLASS_COUNTS:
        raise TraceabilityError(f"final canonical record_class counts differ from CANONICAL_FREEZE: {classes}")
    canonical_groups = {canonical for row in records + [member for _, _, member in members]
                        for canonical in row.get("canonical_requirement_ids", [])}
    if len(canonical_groups) != CANONICAL_GROUP_COUNT:
        raise TraceabilityError(f"final canonical grouping ID count must be {CANONICAL_GROUP_COUNT}, got {len(canonical_groups)}")


def build(inventory_paths: list[str], proposal_paths: list[str], mapping_paths: list[str],
          receipts_path: str, final: bool, published_manifest: str | None = None,
          source_root: str | Path | None = None, member_mapping_paths: list[str] | None = None,
          mapping_resolution_paths: list[str] | None = None, coverage_ruling_paths: list[str] | None = None
          ) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    mapping, mapping_conflicts, resolved_mapping_conflicts = _validate_mapping(
        mapping_paths, final=final, resolution_paths=mapping_resolution_paths)
    member_mapping = _validate_member_mappings(member_mapping_paths or [])
    proposals = _proposals(proposal_paths, final=final)
    inventory_records = []
    for raw_path in inventory_paths:
        inventory_records.extend(_read_jsonl(Path(raw_path)))
    package_root = Path(inventory_paths[0]).resolve().parent.parent
    resolved_source_root = Path(source_root or DEFAULT_SOURCE_ROOT).resolve()
    if not resolved_source_root.is_dir():
        raise TraceabilityError(f"pinned --source-root is not a directory: {resolved_source_root}")
    provenance_hashes = _source_provenance(package_root, resolved_source_root)
    try:
        snapshot_path = str(resolved_source_root.relative_to(package_root))
    except ValueError:
        snapshot_path = str(resolved_source_root)
    verified = (_verified_publication(receipts_path, published_manifest, proposals)
                if final and published_manifest else {})
    nested_members = []
    member_ids = set()
    top_source_ids = set()
    for item in inventory_records:
        sid = item.get("source_id", item.get("requirement_id"))
        top_source_ids.add(sid)
        for field, row_type in (("atomic_members", "atomic-member"), ("cohesive_members", "cohesive-member")):
            values = item.get(field, [])
            if values is None:
                values = []
            if not isinstance(values, list):
                raise TraceabilityError(f"{sid}: {field} must be a list")
            for member in values:
                if not isinstance(member, dict):
                    raise TraceabilityError(f"{sid}: {field} contains a non-object member")
                mid = member.get("member_id")
                if not _text(mid) or mid in member_ids:
                    raise TraceabilityError(f"duplicate or missing nested member ID: {mid}")
                if member.get("parent_source_id") != sid:
                    raise TraceabilityError(f"{mid}: parent_source_id does not match enclosing source statement")
                member_ids.add(mid)
                nested_members.append((row_type, sid, member))
    if member_ids & top_source_ids:
        raise TraceabilityError(f"nested member IDs collide with top-level source IDs: {sorted(member_ids & top_source_ids)}")
    source_by_id = {item.get("source_id", item.get("requirement_id")): item for item in inventory_records}
    member_by_id = {member["member_id"]: (parent_id, member) for _, parent_id, member in nested_members}
    ruling_audit = []
    ruling_ids = []
    if coverage_ruling_paths:
        ruling_audit, ruling_ids = _apply_coverage_rulings(
            coverage_ruling_paths, source_by_id, member_by_id, mapping, member_mapping, proposals)
    if final:
        _validate_canonical_freeze(inventory_paths, inventory_records, nested_members)
    extra_member_ids = set(member_mapping) - member_ids
    if extra_member_ids:
        raise TraceabilityError(f"member mapping contains IDs absent from canonical nested members: {sorted(extra_member_ids)}")
    missing_member_ids = member_ids - set(member_mapping)
    if final and missing_member_ids:
        raise TraceabilityError(f"final matrix has unmapped nested member IDs: {sorted(missing_member_ids)}")
    rows = []
    seen: set[str] = set()
    domain_starters: set[str] = set()
    partial_ids = []
    conflicting_ids = []
    unmapped_ids = []
    disposition_sources: dict[str, set[str]] = {key: set() for key in sorted(DISPOSITIONS)}
    unresolved_outcomes = []
    oracle_review_ids = []
    oracle_counts: dict[str, set[str]] = {}
    acceptance_enriched_ids = []
    canonical_dispositions: dict[str, set[str]] = {}
    member_unmapped_ids = []
    member_partial_ids = []
    member_unresolved_ids = []
    member_oracle_review_ids = []
    member_disposition_sources: dict[str, set[str]] = {key: set() for key in sorted(DISPOSITIONS)}
    for item in inventory_records:
            sid = item.get("source_id", item.get("requirement_id"))
            if not _text(sid) or sid in seen:
                raise TraceabilityError(f"duplicate or missing source_id: {sid}")
            if item.get("requirement_id") != sid:
                raise TraceabilityError(f"{sid}: requirement_id must preserve the exact source_id")
            seen.add(sid)
            excerpt, excerpt_hash, source_hash = _source_proof(item, resolved_source_root, provenance_hashes)
            mapped = mapping.get(sid)
            if sid in mapping_conflicts:
                conflicting_ids.append(sid)
                choices = mapping_conflicts[sid]
                canonicals = {choice["canonical_requirement_id"] for choice in choices}
                mapped = {"canonical_requirement_id": next(iter(canonicals)) if len(canonicals) == 1 else "",
                          "proposal_id": "", "design_section": "",
                          "test_oracle": "", "disposition": "unresolved-cross-domain",
                          "reason": "UNRESOLVED/CROSS-DOMAIN: conflicting reviewed shard mappings select different outcomes/proposals " +
                                    ", ".join(sorted({str(choice["proposal_id"]) for choice in choices}))}
            if not mapped:
                unmapped_ids.append(sid)
                if final:
                    raise TraceabilityError(f"final matrix has unmapped source ID: {sid}")
                mapped = {"canonical_requirement_id": "", "proposal_id": "", "design_section": "",
                          "test_oracle": "", "disposition": "unmapped",
                          "reason": "UNMAPPED: awaiting explicit reviewed mapping"}
            canonical_ids = item.get("canonical_requirement_ids")
            if not isinstance(canonical_ids, list) or any(not _text(value) for value in canonical_ids):
                raise TraceabilityError(f"{sid}: canonical_requirement_ids must be a list of exact string IDs (may be empty for context)")
            canonical = mapped["canonical_requirement_id"]
            disposition = mapped["disposition"]
            if canonical and canonical not in canonical_ids:
                raise TraceabilityError(f"{sid}: mapped canonical ID {canonical!r} is not in source canonical_requirement_ids")
            if not canonical_ids and canonical:
                raise TraceabilityError(f"{sid}: source has no canonical IDs; refusing an invented canonical mapping")
            if not canonical_ids and disposition not in {"administrative", "historical", "superseded", "unmapped"}:
                raise TraceabilityError(f"{sid}: source without canonical IDs must have explicit context disposition")
            disposition_sources[disposition].add(sid)
            for canonical_id in canonical_ids:
                canonical_dispositions.setdefault(canonical_id, set()).add(disposition)
            pid = mapped["proposal_id"]
            supporting_pids = mapped.get("supporting_proposal_ids", [])
            if (not isinstance(supporting_pids, list) or any(not _text(spid) for spid in supporting_pids)
                    or len(set(supporting_pids)) != len(supporting_pids) or (pid and pid in supporting_pids)):
                raise TraceabilityError(f"{sid}: supporting_proposal_ids must be exact and distinct from primary")
            if pid and pid not in proposals:
                raise TraceabilityError(f"{sid}: mapping references unknown exact proposal ID {pid!r}")
            unknown_support = set(supporting_pids) - proposals.keys()
            if unknown_support:
                raise TraceabilityError(f"{sid}: unknown supporting proposal IDs {sorted(unknown_support)}")
            enrichment_clauses = mapped.get("acceptance_enrichment", [])
            if not isinstance(enrichment_clauses, list) or any(not _text(clause) for clause in enrichment_clauses):
                raise TraceabilityError(f"{sid}: acceptance_enrichment must contain nonempty clauses")
            if final and pid:
                _verify_issue_body_binding(proposals[pid], sid, canonical,
                                           str(item.get("statement", "")), mapped["design_section"],
                                           mapped["test_oracle"], mapped["reason"], role="primary",
                                           acceptance_enrichment=enrichment_clauses,
                                           source_specific_oracle=str(item.get("specific_acceptance_oracle", "")),
                                           expert_test_oracle=str(mapped.get("expert_test_oracle", "")))
            if final:
                for support_id in supporting_pids:
                    _verify_issue_body_binding(proposals[support_id], sid, canonical,
                                               str(item.get("statement", "")), mapped["design_section"],
                                               mapped["test_oracle"], mapped["reason"], role="supporting",
                                               acceptance_enrichment=enrichment_clauses,
                                               source_specific_oracle=str(item.get("specific_acceptance_oracle", "")),
                                               expert_test_oracle=str(mapped.get("expert_test_oracle", "")))
            if final and disposition in {"research", "future"} and not pid:
                raise TraceabilityError(f"{sid}: final {disposition} disposition needs an exact proposal or external issue reference")
            reason = mapped["reason"]
            if disposition in {"partial", "covered-with-limits"} or reason.upper().startswith("PARTIAL:"):
                coverage_status = "partial"
            elif disposition == "unresolved-cross-domain" or reason.upper().startswith("UNRESOLVED/CROSS-DOMAIN:"):
                coverage_status = "unresolved"
            elif reason.upper().startswith("COVERED:"):
                coverage_status = "covered"
            else:
                coverage_status = "reviewed-disposition"
            if coverage_status == "partial":
                if final:
                    if (pid or supporting_pids) and (reason.startswith("PARTIAL: acceptance-enrichment-needed:") or enrichment_clauses):
                        coverage_status = "acceptance-enriched"
                        acceptance_enriched_ids.append(sid)
                    else:
                        partial_ids.append(sid)
                        raise TraceabilityError(f"final matrix contains partially covered source outcome: {sid}")
                else:
                    partial_ids.append(sid)
            if coverage_status == "unresolved":
                unresolved_outcomes.append(sid)
                if final:
                    raise TraceabilityError(f"final matrix contains unresolved cross-domain source outcome: {sid}")
            oracle_quality = _oracle_quality(item, mapped)
            oracle_counts.setdefault(oracle_quality, set()).add(sid)
            if oracle_quality == "needs-review":
                oracle_review_ids.append(sid)
                if final:
                    raise TraceabilityError(f"{sid}: generic or inherited acceptance text is not a concrete reviewed oracle")
            if (final and oracle_quality == "not-applicable" and
                    disposition not in {"administrative", "historical", "superseded", "unsupported", "unmapped"}):
                raise TraceabilityError(f"{sid}: active outcome has no concrete acceptance oracle")
            domain_starter = item.get("primary_starter", item.get("domain_starter", ""))
            if _text(domain_starter):
                domain_starters.add(domain_starter)
            published = verified.get(pid) if pid else None
            external_issue = mapped.get("external_issue") if disposition == "external-issue" else None
            primary_issue = (published["number"] if published else
                             external_issue["number"] if external_issue else "")
            dependent_issues = [verified[dep]["number"] for dep in proposals[pid]["depends_on"]] if published else ""
            source_status = item.get("implementation_status", "not-recorded")
            if not _text(source_status):
                source_status = "not-recorded"
            row = {column: item.get(column, "") for column in COORDINATOR_COLUMNS}
            row_snapshot_path = (snapshot_path.rsplit("/sources", 1)[0] + "/note.md"
                                 if item["source_path"] == "note.md"
                                 else snapshot_path.rstrip("/") + "/" + item["source_path"])
            row.update({"source_excerpt": excerpt, "source_excerpt_sha256": excerpt_hash,
                        "source_sha256": source_hash, "source_lines": item["source_lines"],
                        "source_hash_verified": True, "source_commit": SOURCE_COMMIT,
                        "source_snapshot_commit": SOURCE_SNAPSHOT_COMMIT,
                        "source_snapshot_path": row_snapshot_path, "source_path": item["source_path"],
                        "source_id": sid, "source_statement": item.get("statement", ""),
                        "source_specific_oracle": item.get("specific_acceptance_oracle", ""),
                        "expert_test_oracle": mapped.get("expert_test_oracle", ""),
                        "canonical_requirement_id": canonical, "proposal_id": pid,
                        "design_section": mapped["design_section"], "test_oracle": mapped["test_oracle"],
                        "disposition": disposition, "disposition_reason": reason,
                        "mapping_resolution_reason": resolved_mapping_conflicts.get(sid, {}).get("reason", ""),
                         "coverage_status": coverage_status, "oracle_quality": oracle_quality,
                        "domain_starter": domain_starter,
                         "coverage_ruling_id": mapped.get("coverage_ruling_id", ""),
                         "supporting_proposal_ids": supporting_pids,
                         "supporting_issue_numbers": [verified[spid]["number"] for spid in supporting_pids if spid in verified],
                         "acceptance_enrichment": enrichment_clauses,
                        "primary_issue_number": primary_issue,
                        "dependent_issue_numbers": dependent_issues,
                        "implementation_status": source_status,
                        "issue_publication_status": (published["publication_status"] if published else
                                                      "external-reference-verified" if external_issue else "not-published"),
                        "numeric_issue_receipt": published["number"] if published else "",
                        "external_issue_number": external_issue["number"] if external_issue else "",
                        "external_issue_url": external_issue["url"] if external_issue else "",
                        "external_issue_title": external_issue["title"] if external_issue else "",
                        "external_issue_state": external_issue["state"] if external_issue else "",
                        "external_issue_verified_at": external_issue["verified_at"] if external_issue else "",
                        "external_issue_verified_from": external_issue["verified_from"] if external_issue else "",
                        "external_issue_evidence": external_issue["evidence"] if external_issue else ""})
            row["row_type"] = "source-statement"
            row["member_id"] = ""
            row["parent_source_id"] = ""
            rows.append(row)

    for row_type, parent_id, member in nested_members:
        member_id = member["member_id"]
        if member.get("source_commit") != SOURCE_COMMIT:
            raise TraceabilityError(f"{member_id}: nested member source_commit is not the pinned intake commit")
        proof_row = dict(member)
        proof_row["source_id"] = member_id
        proof_row["requirement_id"] = member_id
        if not _text(proof_row.get("source_file_and_section")):
            proof_row["source_file_and_section"] = (
                f"sources/{member['source_path']} lines {_range_label(member.get('source_lines'))}")
        excerpt, excerpt_hash, source_hash = _source_proof(proof_row, resolved_source_root, provenance_hashes)
        mapped = member_mapping.get(member_id)
        if not mapped:
            member_unmapped_ids.append(member_id)
            if final:
                raise TraceabilityError(f"final matrix has unmapped nested member ID: {member_id}")
            mapped = {"member_id": member_id, "parent_source_id": parent_id,
                      "canonical_requirement_id": "", "proposal_id": "", "design_section": "",
                      "test_oracle": "", "disposition": "unmapped",
                      "reason": "UNMAPPED: awaiting explicit per-member proposal/oracle mapping"}
        elif mapped["parent_source_id"] != parent_id:
            raise TraceabilityError(f"{member_id}: mapping parent_source_id differs from nested inventory parent")
        canonical_ids = member.get("canonical_requirement_ids")
        if not isinstance(canonical_ids, list) or not canonical_ids or any(not _text(cid) for cid in canonical_ids):
            raise TraceabilityError(f"{member_id}: canonical_requirement_ids must be exact nonempty IDs")
        canonical = mapped["canonical_requirement_id"]
        if canonical and canonical not in canonical_ids:
            raise TraceabilityError(f"{member_id}: mapped canonical ID {canonical!r} is not in member canonical_requirement_ids")
        disposition = mapped["disposition"]
        member_disposition_sources[disposition].add(member_id)
        for canonical_id in canonical_ids:
            canonical_dispositions.setdefault(canonical_id, set()).add(disposition)
        pid = mapped["proposal_id"]
        supporting_pids = mapped.get("supporting_proposal_ids", [])
        if (not isinstance(supporting_pids, list) or any(not _text(spid) for spid in supporting_pids)
                or len(set(supporting_pids)) != len(supporting_pids) or (pid and pid in supporting_pids)):
            raise TraceabilityError(f"{member_id}: supporting_proposal_ids must be exact and distinct from primary")
        if pid and pid not in proposals:
            raise TraceabilityError(f"{member_id}: mapping references unknown exact proposal ID {pid!r}")
        unknown_support = set(supporting_pids) - proposals.keys()
        if unknown_support:
            raise TraceabilityError(f"{member_id}: unknown supporting proposal IDs {sorted(unknown_support)}")
        enrichment_clauses = mapped.get("acceptance_enrichment", [])
        if not isinstance(enrichment_clauses, list) or any(not _text(clause) for clause in enrichment_clauses):
            raise TraceabilityError(f"{member_id}: acceptance_enrichment must contain nonempty clauses")
        if final and pid:
            _verify_issue_body_binding(proposals[pid], member_id, canonical,
                                       str(member.get("statement", "")), mapped["design_section"],
                                       mapped["test_oracle"], mapped["reason"], role="primary",
                                       acceptance_enrichment=enrichment_clauses,
                                       row_type=row_type, parent_source_id=parent_id,
                                       source_specific_oracle=str(member.get("specific_acceptance_oracle", "")),
                                       expert_test_oracle=str(mapped.get("expert_test_oracle", "")))
        if final:
            for support_id in supporting_pids:
                _verify_issue_body_binding(proposals[support_id], member_id, canonical,
                                           str(member.get("statement", "")), mapped["design_section"],
                                           mapped["test_oracle"], mapped["reason"], role="supporting",
                                           acceptance_enrichment=enrichment_clauses,
                                           row_type=row_type, parent_source_id=parent_id,
                                           source_specific_oracle=str(member.get("specific_acceptance_oracle", "")),
                                           expert_test_oracle=str(mapped.get("expert_test_oracle", "")))
        if final and disposition in {"research", "future"} and not pid:
            raise TraceabilityError(f"{member_id}: final {disposition} member needs an exact proposal or external issue reference")
        if final and disposition not in {"administrative", "historical", "superseded", "unsupported", "external-issue", "unresolved-cross-domain", "unmapped"}:
            source_oracle = member.get("specific_acceptance_oracle")
            if not _text(source_oracle):
                raise TraceabilityError(f"{member_id}: nested outcome lacks its source-specific acceptance oracle")
            if mapped["test_oracle"] != source_oracle and not mapped["reason"].startswith("ORACLE-OVERRIDE:"):
                raise TraceabilityError(f"{member_id}: member oracle must match its own source oracle or explain ORACLE-OVERRIDE:")
        reason = mapped["reason"]
        if disposition in {"partial", "covered-with-limits"} or reason.upper().startswith("PARTIAL:"):
            coverage_status = "partial"
            if final:
                if (pid or supporting_pids) and (reason.startswith("PARTIAL: acceptance-enrichment-needed:") or enrichment_clauses):
                    coverage_status = "acceptance-enriched"
                    acceptance_enriched_ids.append(member_id)
                else:
                    member_partial_ids.append(member_id)
                    raise TraceabilityError(f"final matrix has partially covered nested member: {member_id}")
            else:
                member_partial_ids.append(member_id)
        elif disposition == "unresolved-cross-domain" or reason.upper().startswith("UNRESOLVED/CROSS-DOMAIN:"):
            coverage_status = "unresolved"
            member_unresolved_ids.append(member_id)
            if final:
                raise TraceabilityError(f"final matrix has unresolved nested member: {member_id}")
        elif reason.upper().startswith("COVERED:"):
            coverage_status = "covered"
        else:
            coverage_status = "reviewed-disposition"
        oracle_quality = _oracle_quality(member, mapped)
        oracle_counts.setdefault(oracle_quality, set()).add(member_id)
        if oracle_quality == "needs-review":
            member_oracle_review_ids.append(member_id)
            if final:
                raise TraceabilityError(f"{member_id}: generic/inherited oracle is not a concrete reviewed member oracle")
        if (final and oracle_quality == "not-applicable" and
                disposition not in {"administrative", "historical", "superseded", "unsupported", "unmapped"}):
            raise TraceabilityError(f"{member_id}: active nested outcome has no concrete acceptance oracle")
        external_issue = mapped.get("external_issue") if disposition == "external-issue" else None
        published = verified.get(pid) if pid else None
        member_starter = member.get("primary_starter", "")
        if _text(member_starter):
            domain_starters.add(member_starter)
        primary_issue = published["number"] if published else external_issue["number"] if external_issue else ""
        dependent_issues = [verified[dep]["number"] for dep in proposals[pid]["depends_on"]] if published else ""
        row = {column: member.get(column, "") for column in COORDINATOR_COLUMNS}
        row["requirement_id"] = member_id
        row["source_file_and_section"] = proof_row["source_file_and_section"]
        row["user_outcome"] = member.get("user_outcome", member.get("statement", ""))
        row["acceptance_test_or_benchmark"] = member.get("specific_acceptance_oracle", "")
        row_snapshot_path = (snapshot_path.rsplit("/sources", 1)[0] + "/note.md"
                             if member["source_path"] == "note.md"
                             else snapshot_path.rstrip("/") + "/" + member["source_path"])
        row.update({"source_excerpt": excerpt, "source_excerpt_sha256": excerpt_hash,
                    "source_sha256": source_hash, "source_lines": member["source_lines"],
                    "source_hash_verified": True, "source_commit": SOURCE_COMMIT,
                    "source_snapshot_commit": SOURCE_SNAPSHOT_COMMIT,
                    "source_snapshot_path": row_snapshot_path, "source_path": member["source_path"],
                    "row_type": row_type, "member_id": member_id, "parent_source_id": parent_id,
                    "source_id": member_id, "source_statement": member.get("statement", ""),
                    "source_specific_oracle": member.get("specific_acceptance_oracle", ""),
                    "expert_test_oracle": mapped.get("expert_test_oracle", ""),
                    "canonical_requirement_id": canonical,
                    "proposal_id": pid, "design_section": mapped["design_section"],
                    "test_oracle": mapped["test_oracle"], "disposition": disposition,
                    "disposition_reason": reason, "mapping_resolution_reason": "",
                    "coverage_status": coverage_status,
                    "oracle_quality": oracle_quality, "domain_starter": member.get("primary_starter", ""),
                    "coverage_ruling_id": mapped.get("coverage_ruling_id", ""),
                    "supporting_proposal_ids": supporting_pids,
                    "supporting_issue_numbers": [verified[spid]["number"] for spid in supporting_pids if spid in verified],
                    "acceptance_enrichment": enrichment_clauses,
                    "primary_issue_number": primary_issue, "dependent_issue_numbers": dependent_issues,
                    "implementation_status": member.get("implementation_status", "not-recorded"),
                    "issue_publication_status": (published["publication_status"] if published else
                                                  "external-reference-verified" if external_issue else "not-published"),
                    "numeric_issue_receipt": published["number"] if published else "",
                    "external_issue_number": external_issue["number"] if external_issue else "",
                    "external_issue_url": external_issue["url"] if external_issue else "",
                    "external_issue_title": external_issue["title"] if external_issue else "",
                    "external_issue_state": external_issue["state"] if external_issue else "",
                    "external_issue_verified_at": external_issue["verified_at"] if external_issue else "",
                    "external_issue_verified_from": external_issue["verified_from"] if external_issue else "",
                    "external_issue_evidence": external_issue["evidence"] if external_issue else ""})
        rows.append(row)
    if final and member_unmapped_ids:
        raise TraceabilityError(f"final matrix has unmapped nested member IDs: {sorted(member_unmapped_ids)}")
    extra_member_map_ids = set(member_mapping) - member_ids
    if extra_member_map_ids:
        raise TraceabilityError(f"member mappings reference IDs absent from inventory: {sorted(extra_member_map_ids)}")
    extra_map_ids = (set(mapping) | set(mapping_conflicts)) - seen
    if extra_map_ids:
        raise TraceabilityError(f"mapping contains source IDs absent from inventory: {sorted(extra_map_ids)}")
    if final and unmapped_ids:
        raise TraceabilityError(f"final matrix has unmapped source IDs: {sorted(unmapped_ids)}")
    rows.sort(key=lambda row: (str(row["source_path"]), str(row["source_id"]), str(row["row_type"])))
    context_dispositions = {"administrative", "historical", "superseded"}
    all_canonical = set(canonical_dispositions)
    active_canonical = {cid for cid, statuses in canonical_dispositions.items()
                        if statuses - context_dispositions - {"unmapped"}}
    context_canonical = {cid for cid, statuses in canonical_dispositions.items()
                         if statuses and statuses <= context_dispositions}
    unclassified_canonical = all_canonical - active_canonical - context_canonical
    disposition_counts = {key: {"source_statements": len(disposition_sources[key])}
                          for key in sorted(DISPOSITIONS)}
    summary = {
        "source_statements": len(inventory_records), "nested_member_outcomes": len(nested_members),
        "matrix_rows": len(rows), "unique_canonical_requirements": len(active_canonical),
        "active_unique_canonical_requirements": len(active_canonical),
        "context_unique_canonical_groups": len(context_canonical),
        "unclassified_unique_canonical_groups": len(unclassified_canonical),
        "all_unique_canonical_requirement_groups": len(all_canonical),
        "primary_domain_starters": len(domain_starters),
        "published_primary_issues": len({row["primary_issue_number"] for row in rows
                                          if row["issue_publication_status"] == "published"}),
        "reused_existing_issue_references": len({row["external_issue_number"] for row in rows
                                                 if row["external_issue_number"]}),
        "verified_existing_capability_outcomes": len({(row["row_type"], row["source_id"]) for row in rows
                                                      if row["disposition"] == "existing"}),
        "mapped_source_ids": len(seen) - len(unmapped_ids) - len(conflicting_ids), "unmapped_source_ids": len(unmapped_ids),
        "mapped_member_ids": len(member_ids) - len(member_unmapped_ids),
        "unmapped_member_ids": len(member_unmapped_ids), "unmapped_member_id_list": sorted(member_unmapped_ids),
        "member_partial_outcomes": len(member_partial_ids), "member_partial_ids": sorted(member_partial_ids),
        "member_unresolved_outcomes": len(member_unresolved_ids), "member_unresolved_ids": sorted(member_unresolved_ids),
        "member_oracle_needs_review": len(member_oracle_review_ids),
        "member_oracle_needs_review_ids": sorted(member_oracle_review_ids),
        "acceptance_enriched_outcomes": len(acceptance_enriched_ids),
        "acceptance_enriched_ids": sorted(acceptance_enriched_ids),
        "mapping_conflict_source_ids": len(conflicting_ids), "mapping_conflicts": sorted(conflicting_ids),
        "resolved_mapping_conflict_source_ids": len(resolved_mapping_conflicts),
        "resolved_mapping_conflicts": sorted(resolved_mapping_conflicts),
        "unmapped_ids": sorted(unmapped_ids), "partial_source_outcomes": len(partial_ids),
        "partial_source_ids": sorted(partial_ids), "unresolved_source_outcomes": len(unresolved_outcomes),
        "unresolved_source_ids": sorted(unresolved_outcomes), "dispositions": disposition_counts,
        "oracle_quality_counts": {key: len(value) for key, value in sorted(oracle_counts.items())},
        "oracle_needs_review_source_ids": sorted(oracle_review_ids),
        "concrete_outcome_oracles": len(oracle_counts.get("reviewed-specific-oracle", set())),
        "coverage_complete": not unmapped_ids and not conflicting_ids and not partial_ids and
                            not unresolved_outcomes and not oracle_review_ids and not member_unmapped_ids and
                            not member_partial_ids and not member_unresolved_ids and not member_oracle_review_ids,
        "publication_verified": bool(final), "source_hashes_verified": len(rows),
        "source_statement_hashes_verified": len(inventory_records),
        "member_source_hashes_verified": len(nested_members),
        "member_disposition_counts": {key: len(value) for key, value in sorted(member_disposition_sources.items())},
        "coverage_ruling_count": len(ruling_audit),
        "coverage_ruling_ids": sorted(ruling_ids),
        "source_commit": SOURCE_COMMIT, "source_snapshot_commit": SOURCE_SNAPSHOT_COMMIT,
        "source_snapshot_path": snapshot_path,
    }
    return rows, summary


def _write(rows: list[dict[str, Any]], summary: dict[str, Any], directory: Path,
           *, overwrite: bool = False) -> None:
    if not directory.is_dir():
        raise TraceabilityError("--output-dir must name an existing directory")
    paths = [directory / "traceability.jsonl", directory / "traceability.csv",
             directory / "traceability-summary.md", directory / "acceptance-enrichment.md"]
    if not overwrite and any(path.exists() for path in paths):
        raise TraceabilityError("refusing to overwrite traceability outputs")
    mode = "w" if overwrite else "x"
    with paths[0].open(mode, encoding="utf-8", newline="\n") as output:
        for row in rows:
            output.write(json.dumps(row, sort_keys=True, ensure_ascii=False, separators=(",", ":")) + "\n")
    with paths[1].open(mode, encoding="utf-8", newline="") as output:
        writer = csv.DictWriter(output, fieldnames=OUTPUT_COLUMNS, extrasaction="ignore", lineterminator="\n")
        writer.writeheader()
        writer.writerows(rows)
    summary_lines = ["# Traceability coverage", "",
                     f"- Authoritative source commit: `{summary['source_commit']}`",
                     f"- Snapshot documentation commit: `{summary['source_snapshot_commit']}`",
                     f"- Verified source snapshot: `{summary['source_snapshot_path']}`",
                     f"- Source statements: {summary['source_statements']}",
                     f"- Active unique canonical requirements: {summary['active_unique_canonical_requirements']}",
                     f"- Context-only canonical groups (admin/history/superseded): {summary['context_unique_canonical_groups']}",
                     f"- Unclassified canonical groups: {summary['unclassified_unique_canonical_groups']}",
                     f"- All source canonical groups: {summary['all_unique_canonical_requirement_groups']}",
                     f"- Nested member outcomes (not source IDs): {summary['nested_member_outcomes']}",
                     f"- Member IDs mapped: {summary['mapped_member_ids']}",
                     f"- Member IDs unmapped: {summary['unmapped_member_ids']}",
                     f"- Partially covered member outcomes: {summary['member_partial_outcomes']}",
                     f"- Unresolved member outcomes: {summary['member_unresolved_outcomes']}",
                     f"- Primary domain starters: {summary['primary_domain_starters']}",
                     f"- Published primary issues with confirmed issue+queue receipts: {summary['published_primary_issues']}",
                     f"- Reused existing external issue references: {summary['reused_existing_issue_references']}",
                     f"- Rows citing verified existing-code capabilities: {summary['verified_existing_capability_outcomes']}",
                     f"- Source IDs unmapped: {summary['unmapped_source_ids']}",
                     f"- Source IDs with conflicting mapping shards: {summary['mapping_conflict_source_ids']}",
                     f"- Explicitly resolved mapping collisions: {summary['resolved_mapping_conflict_source_ids']}",
                      f"- Applied expert owner/gap rulings: {summary['coverage_ruling_count']}",
                      f"- Partially covered source outcomes: {summary['partial_source_outcomes']}",
                      f"- Acceptance-enriched planned owner outcomes: {summary['acceptance_enriched_outcomes']}",
                     f"- Concrete reviewed outcome oracles: {summary['concrete_outcome_oracles']}",
                     f"- Generic/unreviewed oracle mappings: {len(summary['oracle_needs_review_source_ids'])}",
                     f"- Member oracle mappings needing review: {summary['member_oracle_needs_review']}",
                     f"- Coverage complete: {str(summary['coverage_complete']).lower()}", "",
                     "## Disposition counts (source statements and unique canonical requirements)", ""]
    for key, count in summary["dispositions"].items():
        summary_lines.append(f"- {key}: {count['source_statements']} source statements")
    summary_lines += ["", "## Nested member disposition counts", ""]
    for key, count in summary["member_disposition_counts"].items():
        summary_lines.append(f"- {key}: {count} members")
    summary_lines += ["", "## Unmapped source IDs", "", *(f"- {sid}" for sid in summary["unmapped_ids"]),
                      "", "## Partial source outcomes", "", *(f"- {sid}" for sid in summary["partial_source_ids"]),
                      "", "## Conflicting mapping shards", "", *(f"- {sid}" for sid in summary["mapping_conflicts"]),
                      "", "## Explicitly resolved mapping collisions", "",
                      *(f"- {sid}" for sid in summary["resolved_mapping_conflicts"]),
                      "", "## Unresolved cross-domain outcomes", "", *(f"- {sid}" for sid in summary["unresolved_source_ids"]),
                      "", "## Generic/unreviewed oracle mappings", "",
                      *(f"- {sid}" for sid in summary["oracle_needs_review_source_ids"]),
                      "", "## Unmapped nested member IDs", "", *(f"- {mid}" for mid in summary["unmapped_member_id_list"]),
                      "", "## Partially covered nested members", "", *(f"- {mid}" for mid in summary["member_partial_ids"]),
                      "", "## Unresolved nested members", "", *(f"- {mid}" for mid in summary["member_unresolved_ids"]),
                      "", "## Nested member oracles needing review", "",
                      *(f"- {mid}" for mid in summary["member_oracle_needs_review_ids"]),
                      "", "## Acceptance-enriched planned owner outcomes", "",
                      *(f"- {ident}" for ident in summary["acceptance_enriched_ids"])]
    with paths[2].open(mode, encoding="utf-8", newline="\n") as output:
        output.write("\n".join(summary_lines) + "\n")
    enrichment: dict[str, list[tuple[dict[str, Any], str]]] = {}
    unowned_partial = []
    for row in rows:
        if row["coverage_status"] != "partial" and not row.get("acceptance_enrichment"):
            continue
        if row["proposal_id"]:
            enrichment.setdefault(row["proposal_id"], []).append((row, "primary"))
        for support in row.get("supporting_proposal_ids", []):
            enrichment.setdefault(support, []).append((row, "supporting"))
        if not row["proposal_id"] and not row.get("supporting_proposal_ids"):
            unowned_partial.append(row)
    enrichment_lines = ["# Acceptance enrichment by future implementation owner", "",
                        "These reviewed rows and expert ruling clauses are planned issue-body inputs, not evidence of implemented behavior. Enrich only the named owner's body; supporting issues are labeled separately. The source oracle is a planned acceptance condition, not a passing test result.", "",
                        f"- Owner-linked partial/ruling rows: {sum(map(len, enrichment.values()))}",
                        f"- Partial rows without a local proposal owner: {len(unowned_partial)}", ""]
    for pid, owner_rows in sorted(enrichment.items()):
        enrichment_lines += [f"## {pid} ({len(owner_rows)} rows)", ""]
        for row, role in sorted(owner_rows, key=lambda item: (item[0]["row_type"], item[0]["source_id"], item[1])):
            ident = row["member_id"] or row["source_id"]
            parent = f"; parent {row['parent_source_id']}" if row["parent_source_id"] else ""
            enrichment_lines += [f"### {ident}{parent} — {role} owner",
                                 f"- Canonical requirement: `{row['canonical_requirement_id']}`",
                                 f"- Disposition: {row['disposition']}; section: {row['design_section']}",
                                 f"- Reason: {row['disposition_reason']}",
                                 f"- Oracle / acceptance case: {row['test_oracle']}", ""]
            if row.get("acceptance_enrichment"):
                enrichment_lines += ["- Expert ruling acceptance enrichment:",
                                     *(f"  - {clause}" for clause in row["acceptance_enrichment"]), ""]
    if unowned_partial:
        enrichment_lines += ["## Partial rows without a local issue owner", ""]
        for row in sorted(unowned_partial, key=lambda item: item["source_id"]):
            enrichment_lines += [f"- `{row['source_id']}` ({row['canonical_requirement_id']}): {row['disposition_reason']} Oracle: {row['test_oracle']}"]
        enrichment_lines.append("")
    with paths[3].open(mode, encoding="utf-8", newline="\n") as output:
        output.write("\n".join(enrichment_lines) + "\n")


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    try:
        rows, summary = build(args.inventory, args.proposals, args.mapping, args.receipts,
                              args.final, args.published_manifest, args.source_root,
                              args.member_mapping, args.mapping_resolution, args.coverage_ruling)
        if args.output_dir:
            _write(rows, summary, Path(args.output_dir), overwrite=args.overwrite)
        print(json.dumps(summary, sort_keys=True, indent=2))
        return 0
    except (OSError, TraceabilityError) as exc:
        print(f"build_traceability: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
