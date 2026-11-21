#!/usr/bin/env python3
"""Strictly normalize authored Fleet CA proposals into publisher input."""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import re
import sys
from pathlib import Path
from typing import Any


HERE = Path(__file__).resolve().parent.parent
POLICY_PATH = HERE / "traceability" / "normalization-policy.json"
BINDING_PATH = HERE / "traceability" / "binding.py"
_BINDING_SPEC = importlib.util.spec_from_file_location("fleet_ca_traceability_binding", BINDING_PATH)
if not _BINDING_SPEC or not _BINDING_SPEC.loader:
    raise RuntimeError("cannot load shared traceability binding renderer")
traceability_binding = importlib.util.module_from_spec(_BINDING_SPEC)
_BINDING_SPEC.loader.exec_module(traceability_binding)
REQUIRED = {"id", "title", "parent", "kind", "source_ranges", "requirement_ids", "stage",
            "lifecycle", "urgency", "depends_on", "body"}
OPTIONAL = {"size", "owned_files_or_interfaces", "lane", "security_classification",
            "urgency_band", "priority_band", "priority", "queue_priority_projection"}
PUBLISHER_KINDS = {"design", "research", "implementation", "test", "migration", "operations",
                   "security review", "documentation"}
PUBLISHER_PARENTS = {9156, 9157, 9158, 9159, 9160, 9162, 9163, 9164, 9165, 9166, 9167}
PUBLISHER_MARKER_PREFIX = "fleet-ca-v03-proposal:"
SOURCE_PROVENANCE = {
    "repository": "developing-today/code",
    "source_commit": "34faffbff26615e313488f6d04705a444342e41e",
    "snapshot_commit": "40af815bf0",
    "source_root": "doc/2026-11-13T01-34-11Z_design_fleet_ca_credentials_secrets/source-snapshots/intake-34faffbff2/sources",
    "source_integrity": "doc/2026-11-13T01-34-11Z_design_fleet_ca_credentials_secrets/source-integrity.md",
    "auxiliary_source": "doc/2026-11-13T01-34-11Z_design_fleet_ca_credentials_secrets/source-snapshots/intake-34faffbff2/note.md",
}
PUBLICATION_GUARD = (
    "Design input only; keep this issue open until its own acceptance evidence passes. "
    "Initial queue work is triage, not production authorization."
)


class ManifestError(Exception):
    pass


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(allow_abbrev=False)
    parser.add_argument("--proposal", action="append", required=True,
                        help="proposal JSON file; may be repeated")
    parser.add_argument("--traceability-matrix",
                        help="verified draft/final traceability.jsonl used to bind canonical/source/member IDs and oracles")
    parser.add_argument("--output-dir")
    parser.add_argument("--write", action="store_true", help="write normalized manifest; default is dry-run")
    parser.add_argument("--overwrite", action="store_true", help="allow replacing proposals.normalized.json")
    args = parser.parse_args(argv)
    if args.overwrite and not args.write:
        parser.error("--overwrite requires --write")
    if args.write and not args.output_dir:
        parser.error("--output-dir is required with --write")
    if not args.write and args.output_dir:
        parser.error("--output-dir is only valid with --write")
    return args


def load_policy() -> dict[str, Any]:
    try:
        policy = json.loads(POLICY_PATH.read_text(encoding="utf-8"), object_pairs_hook=_unique_object)
    except (OSError, json.JSONDecodeError, _DuplicateKey) as exc:
        raise ManifestError(f"cannot load normalization policy: {exc}") from exc
    if not isinstance(policy, dict) or policy.get("version") != 1:
        raise ManifestError("unsupported normalization policy")
    if set(policy.get("output_fields", [])) != REQUIRED:
        raise ManifestError("policy output schema does not match publisher's exact 11 fields")
    if policy.get("source_provenance") != SOURCE_PROVENANCE:
        raise ManifestError("policy source provenance must remain pinned to the original intake snapshot")
    if policy.get("publication_guard") != PUBLICATION_GUARD:
        raise ManifestError("policy publication guard must retain the design-only lifecycle notice")
    overrides = policy.get("proposal_kind_overrides")
    if not isinstance(overrides, dict):
        raise ManifestError("policy proposal_kind_overrides must be an exact-ID mapping table")
    for pid, override in overrides.items():
        if (not _nonempty(pid) or not isinstance(override, dict) or
                set(override) != {"authored_kind", "publisher_kind", "reason"} or
                override["publisher_kind"] not in PUBLISHER_KINDS or
                not _nonempty(override["authored_kind"]) or not _nonempty(override["reason"])):
            raise ManifestError(f"invalid explicit kind override for {pid!r}")
    return policy


def _read(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=_unique_object)
    except (OSError, json.JSONDecodeError, _DuplicateKey) as exc:
        raise ManifestError(f"{path}: cannot read valid JSON: {exc}") from exc


class _DuplicateKey(ValueError):
    pass


def _unique_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result = {}
    for key, value in pairs:
        if key in result:
            raise _DuplicateKey(f"duplicate JSON key {key!r}")
        result[key] = value
    return result


def _format_ranges(value: Any) -> str:
    if isinstance(value, list) and len(value) == 2 and all(type(n) is int for n in value):
        value = [value]
    if not isinstance(value, list):
        raise ManifestError("traceability row source_lines must be a range list")
    formatted = []
    for span in value:
        if (not isinstance(span, list) or len(span) != 2 or
                any(type(n) is not int for n in span) or span[0] < 1 or span[1] < span[0]):
            raise ManifestError(f"traceability row has malformed source span {span!r}")
        formatted.append(str(span[0]) if span[0] == span[1] else f"{span[0]}-{span[1]}")
    return ", ".join(formatted)


def bind_traceability(manifest: list[dict[str, Any]], matrix_path: str,
                      policy: dict[str, Any]) -> list[dict[str, Any]]:
    path = Path(matrix_path)
    try:
        raw = path.read_bytes()
        text = raw.decode("utf-8")
    except (OSError, UnicodeDecodeError) as exc:
        raise ManifestError(f"cannot read traceability matrix {path}: {exc}") from exc
    matrix_sha = hashlib.sha256(raw).hexdigest()
    proposals = {proposal["id"]: proposal for proposal in manifest}
    by_proposal: dict[str, list[dict[str, Any]]] = {pid: [] for pid in proposals}
    seen = set()
    for line_no, line in enumerate(text.splitlines(), 1):
        if not line.strip():
            continue
        try:
            row = json.loads(line, object_pairs_hook=_unique_object)
        except (json.JSONDecodeError, _DuplicateKey) as exc:
            raise ManifestError(f"{path}:{line_no}: invalid/duplicate-key traceability JSON: {exc}") from exc
        if not isinstance(row, dict):
            raise ManifestError(f"{path}:{line_no}: traceability row must be an object")
        sid, pid = row.get("source_id"), row.get("proposal_id")
        if not _nonempty(sid) or sid in seen:
            raise ManifestError(f"{path}:{line_no}: duplicate/missing source or member ID")
        seen.add(sid)
        if row.get("source_hash_verified") is not True or row.get("source_commit") != SOURCE_PROVENANCE["source_commit"]:
            raise ManifestError(f"{path}:{line_no}: row lacks verified pinned-source provenance")
        if row.get("issue_publication_status") != "not-published":
            raise ManifestError(f"{path}:{line_no}: cannot alter or bind a matrix after issue publication has started")
        if row.get("row_type") not in {"source-statement", "atomic-member", "cohesive-member"}:
            raise ManifestError(f"{path}:{line_no}: invalid row_type")
        if not pid:
            continue
        if not _nonempty(pid) or pid not in proposals:
            raise ManifestError(f"{path}:{line_no}: mapping references unknown exact proposal ID {pid!r}")
        for field in ("canonical_requirement_id", "source_statement", "source_file_and_section",
                      "source_path", "design_section", "test_oracle", "disposition", "disposition_reason"):
            if not _nonempty(row.get(field)):
                raise ManifestError(f"{path}:{line_no}: mapped row lacks {field}")
        if row.get("oracle_quality") != "reviewed-specific-oracle":
            raise ManifestError(f"{path}:{line_no}: mapped row's oracle is not marked as a reviewed specific oracle")
        if row.get("disposition") == "external-issue":
            raise ManifestError(f"{path}:{line_no}: external issue references do not bind a local proposal")
        by_proposal[pid].append(row)
    orphaned = sorted(pid for pid, rows in by_proposal.items() if not rows)
    if orphaned:
        raise ManifestError(f"local proposals lack reciprocal source/member bindings: {orphaned}")

    for pid, proposal in proposals.items():
        bound = sorted(by_proposal[pid], key=lambda row: (row["row_type"], row["source_id"]))
        canonical_ids = sorted({row["canonical_requirement_id"] for row in bound})
        binding_ids = sorted({row["source_id"] for row in bound})
        original_requirement_ids = list(proposal["requirement_ids"])
        proposal["requirement_ids"] = list(dict.fromkeys(original_requirement_ids + canonical_ids + binding_ids))
        detail = [
            "## Source-linked requirements and planned acceptance (not implementation evidence)",
            "",
            f"Traceability matrix SHA-256: `{matrix_sha}`. These clauses identify this issue's planned owner scope; they do not say the code exists or that the acceptance oracle has passed.",
            "",
        ]
        for row in bound:
            ident = row["member_id"] or row["source_id"]
            member_label = f"{row['row_type']} {ident}"
            if row["parent_source_id"]:
                member_label += f" (parent {row['parent_source_id']})"
            detail += [f"### {member_label}",
                       f"- Canonical requirement: `{row['canonical_requirement_id']}`",
                       f"- Source: `{row['source_path']}` lines {_format_ranges(row['source_lines'])}; `{row['source_file_and_section']}`",
                       f"- Source clause: {row['source_statement']}",
                       f"- Design target: {row['design_section']}",
                       f"- Planned acceptance oracle (not run): {row['test_oracle']}",
                       f"- Traceability disposition: {row['disposition']}; {row['disposition_reason']}", ""]
        proposal["body"] = proposal["body"].rstrip() + "\n\n" + "\n".join(detail)
        # Preserve authored IDs separately for review; enriched publisher IDs also contain source/canonical aliases.
        proposal["body"] = proposal["body"].replace(
            '"source_provenance":',
            '"traceability_binding":' + json.dumps({"matrix_sha256": matrix_sha,
                "canonical_requirement_ids": canonical_ids, "source_and_member_ids": binding_ids},
                sort_keys=True, ensure_ascii=False, separators=(",", ":")) + ',"source_provenance":', 1)
        proposal["body"] = proposal["body"].replace(
            '"original_kind":',
            '"original_requirement_ids":' + json.dumps(original_requirement_ids, ensure_ascii=False, separators=(",", ":")) + ',"original_kind":', 1)
        # Match publish_issues.issue_body's rendered footer before issue writes.
        urgency_band = _priority(proposal["urgency"], policy)["urgency_band"].lower()
        suffix = (f"\n\nKind: {proposal['kind']}\nStage: {proposal['stage']}\n"
                  f"Lifecycle: {proposal['lifecycle']}\nIntrinsic urgency: {proposal['urgency']} "
                  f"({urgency_band}; lower is sooner)\n"
                  f"Requirement IDs: {', '.join(proposal['requirement_ids'])}\n"
                  f"Source ranges: {', '.join(proposal['source_ranges'])}\n"
                  f"<!-- fleet-ca-v03-proposal:{pid} -->")
        rendered = proposal["body"].rstrip() + suffix
        if len(rendered) > 65_536 or len(rendered.encode("utf-8")) > 65_536:
            raise ManifestError(f"{pid}: traceability-enriched issue body exceeds 65,536 chars/bytes; split the deliverable coherently, never truncate")
    return manifest


def _traceability_rows(path: Path) -> tuple[list[dict[str, Any]], str]:
    try:
        raw = path.read_bytes()
        text = raw.decode("utf-8")
    except (OSError, UnicodeDecodeError) as exc:
        raise ManifestError(f"{path}: cannot read UTF-8 traceability matrix: {exc}") from exc
    rows = []
    seen = set()
    for index, line in enumerate(text.splitlines(), 1):
        if not line.strip():
            continue
        try:
            row = json.loads(line)
        except json.JSONDecodeError as exc:
            raise ManifestError(f"{path}:{index}: invalid traceability JSONL: {exc}") from exc
        if not isinstance(row, dict):
            raise ManifestError(f"{path}:{index}: traceability row must be an object")
        sid, pid = row.get("source_id"), row.get("proposal_id")
        if not _nonempty(sid) or sid in seen:
            raise ManifestError(f"{path}:{index}: source/member ID must be unique and exact")
        seen.add(sid)
        if row.get("source_hash_verified") is not True or row.get("source_commit") != SOURCE_PROVENANCE["source_commit"]:
            raise ManifestError(f"{path}:{index}: source row lacks verified pinned provenance")
        if row.get("row_type") not in {"source-statement", "atomic-member", "cohesive-member"}:
            raise ManifestError(f"{path}:{index}: unsupported traceability row_type")
        if pid and not _nonempty(pid):
            raise ManifestError(f"{path}:{index}: proposal_id is not an exact nonempty ID")
        if pid:
            for field in ("canonical_requirement_id", "source_statement", "source_file_and_section",
                          "design_section", "test_oracle", "disposition", "disposition_reason"):
                if not _nonempty(row.get(field)):
                    raise ManifestError(f"{path}:{index}: mapped row lacks {field}")
        rows.append(row)
    return rows, hashlib.sha256(raw).hexdigest()


def _nonempty(value: Any) -> bool:
    return isinstance(value, str) and bool(value.strip())


def _priority(urgency: int, policy: dict[str, Any]) -> dict[str, Any]:
    for band in policy["priority_bands"]:
        if urgency >= band["min"] and (band["max"] is None or urgency <= band["max"]):
            return band
    raise ManifestError(f"urgency score outside supported range: {urgency}")


def _parent(value: Any, where: str) -> int:
    if type(value) is int:
        parent = value
    elif isinstance(value, str) and re.fullmatch(r"#?[0-9]+", value):
        parent = int(value.removeprefix("#"))
    else:
        parent = -1
    if parent not in PUBLISHER_PARENTS:
        raise ManifestError(f"{where}.parent must be an existing domain starter (not #9161)")
    return parent


def _proposal_id(value: Any, where: str) -> str:
    if not _nonempty(value):
        raise ManifestError(f"{where}.id must be a non-empty string")
    # Match publish_issues.validate_manifest exactly; IDs are identity-bearing.
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,100}", value):
        raise ManifestError(f"{where}.id cannot be represented as a publisher ID: {value!r}")
    return value


def normalize_one(source: Any, where: str, policy: dict[str, Any]) -> dict[str, Any]:
    if not isinstance(source, dict):
        raise ManifestError(f"{where} must be an object")
    unknown = set(source) - REQUIRED - OPTIONAL
    missing = REQUIRED - set(source)
    if unknown or missing:
        raise ManifestError(f"{where} schema mismatch; missing={sorted(missing)}, unknown={sorted(unknown)}")
    for key in ("id", "title", "kind", "stage", "lifecycle", "body"):
        if not _nonempty(source[key]):
            raise ManifestError(f"{where}.{key} must be a non-empty string")
    if len(source["title"]) > 256 or "\n" in source["title"] or "\r" in source["title"]:
        raise ManifestError(f"{where}.title must be one line of at most 256 characters")
    if PUBLISHER_MARKER_PREFIX in source["body"]:
        raise ManifestError(f"{where}.body contains reserved publisher marker text")
    if source["lifecycle"] not in {"Ready", "Backlog", "Blocked", "Parked"}:
        raise ManifestError(f"{where}.lifecycle is not a publisher lifecycle")
    if type(source["urgency"]) is not int or source["urgency"] < 0:
        raise ManifestError(f"{where}.urgency must be a non-negative integer")
    priority = _priority(source["urgency"], policy)
    for field, expected in (("urgency_band", priority["urgency_band"]),
                            ("priority_band", priority["priority_band"]),
                            ("queue_priority_projection", priority["queue_priority_projection"])):
        if field in source and source[field] != expected:
            raise ManifestError(f"{where}.{field}={source[field]!r} disagrees with urgency {source['urgency']} (expected {expected!r})")
    if "priority" in source and source["priority"] not in {
        priority["priority_band"], priority["queue_priority_projection"]
    }:
        raise ManifestError(f"{where}.priority disagrees with urgency {source['urgency']}")
    for key in ("source_ranges", "requirement_ids", "depends_on"):
        value = source[key]
        if not isinstance(value, list) or any(not _nonempty(item) for item in value):
            raise ManifestError(f"{where}.{key} must be a list of non-empty strings")
    if not source["source_ranges"] or not source["requirement_ids"]:
        raise ManifestError(f"{where}: source_ranges and requirement_ids must not be empty")
    owned = source.get("owned_files_or_interfaces")
    if owned is not None and (not isinstance(owned, list) or any(not _nonempty(x) for x in owned)):
        raise ManifestError(f"{where}.owned_files_or_interfaces must be a list of non-empty strings")
    for key in ("size", "lane", "security_classification"):
        if key in source and not _nonempty(source[key]):
            raise ManifestError(f"{where}.{key} must be a non-empty string")
    override = policy["proposal_kind_overrides"].get(source["id"])
    if override:
        if source["kind"] != override["authored_kind"]:
            raise ManifestError(f"{where}.kind differs from the exact authored kind expected by its ID override")
        kind_rule = {"publisher_kind": override["publisher_kind"], "reason": override["reason"]}
    else:
        kind_rule = policy["kinds"].get(source["kind"])
    if not isinstance(kind_rule, dict) or set(kind_rule) != {"publisher_kind", "reason"}:
        raise ManifestError(f"{where}.kind has no explicit reviewed normalization: {source['kind']!r}")
    if kind_rule["publisher_kind"] not in PUBLISHER_KINDS or not _nonempty(kind_rule["reason"]):
        raise ManifestError(f"policy has invalid kind mapping for {source['kind']!r}")
    metadata = {key: source[key] for key in sorted(OPTIONAL) if key in source}
    metadata["original_kind"] = source["kind"]
    metadata["original_id"] = source["id"]
    metadata["original_parent"] = source["parent"]
    metadata["original_depends_on"] = source["depends_on"]
    metadata["kind_normalization"] = {"publisher_kind": kind_rule["publisher_kind"],
                                      "reason": kind_rule["reason"]}
    metadata["source_provenance"] = policy["source_provenance"]
    pin = policy["source_provenance"]
    commit = pin["source_commit"]
    snapshot_commit = pin["snapshot_commit"]
    root_url = f"https://github.com/{pin['repository']}/tree/{snapshot_commit}/{pin['source_root']}"
    integrity_url = f"https://github.com/{pin['repository']}/blob/{snapshot_commit}/{pin['source_integrity']}"
    note_url = f"https://github.com/{pin['repository']}/blob/{snapshot_commit}/{pin['auxiliary_source']}"
    citation = ("## Design-only lifecycle and authorization\n"
                f"> {policy['publication_guard']}\n\n"
                "## Immutable source authority\n"
                f"- Intake commit: `{commit}`\n"
                f"- Snapshot documentation commit: `{snapshot_commit}`\n"
                f"- Original source bytes: [`{pin['source_root']}`]({root_url})\n"
                f"- Integrity and formatting-incident provenance: [`source-integrity.md`]({integrity_url})\n"
                f"- Auxiliary evidence note: [`note.md`]({note_url})\n\n")
    body = ("<!-- Authored proposal metadata preserved by prepare_manifest.py -->\n"
            + citation +
            "## Authored proposal metadata (administrative; not dropped)\n"
            "```json\n" + json.dumps(metadata, sort_keys=True, ensure_ascii=False, separators=(",", ":")) +
            "\n```\n\n" + source["body"])
    if PUBLISHER_MARKER_PREFIX in body:
        raise ManifestError(f"{where}.body contains reserved publisher marker text")
    return {
        "id": _proposal_id(source["id"], where), "title": source["title"], "parent": _parent(source["parent"], where),
        "kind": kind_rule["publisher_kind"], "source_ranges": source["source_ranges"],
        "requirement_ids": source["requirement_ids"], "stage": source["stage"],
        "lifecycle": source["lifecycle"], "urgency": source["urgency"],
        "depends_on": [_proposal_id(dep, f"{where}.depends_on") for dep in source["depends_on"]], "body": body,
    }


def normalize(paths: list[str], policy: dict[str, Any]) -> list[dict[str, Any]]:
    result = []
    ids: set[str] = set()
    for raw_path in paths:
        document = _read(Path(raw_path))
        if not isinstance(document, list):
            raise ManifestError(f"{raw_path}: expected proposal array")
        for index, source in enumerate(document):
            proposal = normalize_one(source, f"{raw_path}[{index}]", policy)
            pid = proposal["id"]
            if pid in ids:
                raise ManifestError(f"duplicate proposal id: {pid}")
            ids.add(pid)
            result.append(proposal)
    by_id = {item["id"]: item for item in result}
    for proposal in result:
        deps = proposal["depends_on"]
        if len(set(deps)) != len(deps) or proposal["id"] in deps:
            raise ManifestError(f"{proposal['id']}: duplicate or self dependency")
        missing = set(deps) - by_id.keys()
        if missing:
            raise ManifestError(f"{proposal['id']}: unknown proposal dependencies {sorted(missing)}")
    pending, completed = set(by_id), set()
    while pending:
        ready = {pid for pid in pending if set(by_id[pid]["depends_on"]).issubset(completed)}
        if not ready:
            raise ManifestError(f"proposal dependency cycle: {sorted(pending)}")
        completed.update(ready)
        pending.difference_update(ready)
    return result


def bind_traceability(manifest: list[dict[str, Any]], matrix_path: str,
                      policy: dict[str, Any]) -> list[dict[str, Any]]:
    """Enrich each future issue with exact source/member bindings and planned oracles."""
    path = Path(matrix_path)
    try:
        raw = path.read_bytes()
        text = raw.decode("utf-8")
    except (OSError, UnicodeDecodeError) as exc:
        raise ManifestError(f"cannot read UTF-8 traceability matrix {path}: {exc}") from exc
    matrix_digest = hashlib.sha256(raw).hexdigest()
    by_proposal: dict[str, list[tuple[dict[str, Any], str]]] = {proposal["id"]: [] for proposal in manifest}
    seen_ids = set()
    for line_number, line in enumerate(text.splitlines(), 1):
        if not line.strip():
            continue
        try:
            row = json.loads(line, object_pairs_hook=_unique_object)
        except (json.JSONDecodeError, _DuplicateKey) as exc:
            raise ManifestError(f"{path}:{line_number}: invalid/duplicate-key traceability JSON: {exc}") from exc
        if not isinstance(row, dict):
            raise ManifestError(f"{path}:{line_number}: traceability row must be an object")
        sid, pid = row.get("source_id"), row.get("proposal_id")
        if not _nonempty(sid) or sid in seen_ids:
            raise ManifestError(f"{path}:{line_number}: source/member ID must be unique and exact")
        seen_ids.add(sid)
        if (row.get("source_hash_verified") is not True or
                row.get("source_commit") != SOURCE_PROVENANCE["source_commit"] or
                row.get("source_snapshot_commit") != SOURCE_PROVENANCE["snapshot_commit"]):
            raise ManifestError(f"{path}:{line_number}: row is not verified against the pinned original snapshot")
        if row.get("issue_publication_status") not in {"not-published", "external-reference-verified"}:
            raise ManifestError(f"{path}:{line_number}: cannot mutate/rebind a matrix after issue publication has started")
        if row.get("row_type") not in {"source-statement", "atomic-member", "cohesive-member"}:
            raise ManifestError(f"{path}:{line_number}: unsupported traceability row_type")
        if row.get("issue_publication_status") == "external-reference-verified" and pid:
            raise ManifestError(f"{path}:{line_number}: external references cannot be local proposal bindings")
        supports = row.get("supporting_proposal_ids", [])
        if (not isinstance(supports, list) or any(not _nonempty(support) for support in supports)
                or len(set(supports)) != len(supports) or (pid and pid in supports)):
            raise ManifestError(f"{path}:{line_number}: supporting proposal IDs must be exact and distinct")
        targets = []
        if pid:
            targets.append((pid, "primary"))
        targets.extend((support, "supporting") for support in supports)
        if not targets:
            continue
        unknown_targets = [target for target, _ in targets if target not in by_proposal]
        if unknown_targets:
            raise ManifestError(f"{path}:{line_number}: unknown exact local proposal IDs {unknown_targets}")
        if row.get("oracle_quality") != "reviewed-specific-oracle":
            raise ManifestError(f"{path}:{line_number}: mapped oracle is not reviewed and outcome-specific")
        for field in ("canonical_requirement_id", "source_statement", "source_file_and_section",
                      "source_path", "design_section", "test_oracle",
                      "disposition", "disposition_reason"):
            if field == "canonical_requirement_id" and not _nonempty(row.get(field)):
                raise ManifestError(f"{path}:{line_number}: proposal binding has no canonical requirement ID")
            if field != "canonical_requirement_id" and not _nonempty(row.get(field)):
                raise ManifestError(f"{path}:{line_number}: mapped row lacks {field}")
        try:
            _format_ranges(row.get("source_lines"))
        except ManifestError as exc:
            raise ManifestError(f"{path}:{line_number}: {exc}") from exc
        if row["disposition"] == "external-issue":
            raise ManifestError(f"{path}:{line_number}: external issue references cannot bind a local proposal")
        enrichments = row.get("acceptance_enrichment", [])
        if not isinstance(enrichments, list) or any(not _nonempty(item) for item in enrichments):
            raise ManifestError(f"{path}:{line_number}: acceptance_enrichment must be a list of nonempty clauses")
        for target, role in targets:
            by_proposal[target].append((row, role))
    orphaned = sorted(pid for pid, rows in by_proposal.items() if not rows)
    if orphaned:
        raise ManifestError(f"local proposals lack reciprocal source/member bindings: {orphaned}")

    for pid, proposal in sorted(((p["id"], p) for p in manifest), key=lambda item: item[0]):
        binding_pairs = sorted(by_proposal[pid], key=lambda item: (item[0]["row_type"], item[0]["source_id"], item[1]))
        bindings = [row for row, _ in binding_pairs]
        canonical_ids = sorted({row["canonical_requirement_id"] for row in bindings})
        binding_ids = sorted({row["source_id"] for row in bindings})
        source_ranges = sorted({row["source_file_and_section"] for row in bindings})
        authored_requirement_ids = list(proposal["requirement_ids"])
        proposal["requirement_ids"] = list(dict.fromkeys(authored_requirement_ids + canonical_ids + binding_ids))
        proposal["source_ranges"] = list(dict.fromkeys(proposal["source_ranges"] + source_ranges))

        appendix = [
            "## Source-linked requirements and planned acceptance (not implementation evidence)", "",
            f"This issue's exact canonical/source/member bindings are from traceability matrix SHA-256 `{matrix_digest}`. These are future acceptance conditions; they do not assert implementation or passing tests.", "",
        ]
        for row, role, source_ids, parent_ids in traceability_binding.group_rows(binding_pairs):
            appendix += [traceability_binding.render_binding_block(row, role, SOURCE_PROVENANCE,
                                                                      source_ids, parent_ids), ""]
        proposal["body"] = proposal["body"].rstrip() + "\n\n" + "\n".join(appendix)
        binding_meta = {"matrix_sha256": matrix_digest,
                        "canonical_requirement_ids": canonical_ids,
                        "source_and_member_ids": binding_ids,
                        "original_requirement_ids": authored_requirement_ids}
        proposal["body"] = proposal["body"].replace(
            '"source_provenance":',
            '"traceability_binding":' + json.dumps(binding_meta, sort_keys=True, ensure_ascii=False,
                                                     separators=(",", ":")) + ',"source_provenance":', 1)
        if '"traceability_binding":' not in proposal["body"]:
            raise ManifestError(f"{pid}: cannot locate administrative metadata header to record traceability binding")

        urgency_band = _priority(proposal["urgency"], policy)["urgency_band"].lower()
        rendered_suffix = (f"\n\nKind: {proposal['kind']}\nStage: {proposal['stage']}\n"
                          f"Lifecycle: {proposal['lifecycle']}\nIntrinsic urgency: {proposal['urgency']} "
                          f"({urgency_band}; lower is sooner)\n"
                          f"Requirement IDs: {', '.join(proposal['requirement_ids'])}\n"
                          f"Source ranges: {', '.join(proposal['source_ranges'])}\n"
                          f"<!-- fleet-ca-v03-proposal:{pid} -->")
        rendered = proposal["body"].rstrip() + rendered_suffix
        if len(rendered) > 65_536 or len(rendered.encode("utf-8")) > 65_536:
            raise ManifestError(f"{pid}: traceability-enriched rendered issue body exceeds 65,536 chars/bytes; split the deliverable coherently, never truncate")
    return manifest


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    try:
        policy = load_policy()
        manifest = normalize(args.proposal, policy)
        if args.traceability_matrix:
            manifest = bind_traceability(manifest, args.traceability_matrix, policy)
        rendered = json.dumps(manifest, sort_keys=True, ensure_ascii=False, indent=2) + "\n"
        if args.write:
            directory = Path(args.output_dir)
            if not directory.is_dir():
                raise ManifestError("--output-dir must name an existing directory")
            output = directory / "proposals.normalized.json"
            if output.exists() and not args.overwrite:
                raise ManifestError(f"refusing to overwrite {output}; pass --overwrite explicitly")
            output.write_text(rendered, encoding="utf-8")
            print(json.dumps({"normalized": len(manifest), "output": str(output)}, indent=2))
        else:
            print(rendered, end="")
        return 0
    except (OSError, ManifestError) as exc:
        print(f"prepare_manifest: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
