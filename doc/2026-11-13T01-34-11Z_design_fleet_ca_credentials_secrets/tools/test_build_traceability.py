from __future__ import annotations

import hashlib
import importlib.util
import json
from collections import Counter
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

_BUILDER_PATH = Path(__file__).with_name("build_traceability.py")
_SPEC = importlib.util.spec_from_file_location("build_traceability", _BUILDER_PATH)
assert _SPEC and _SPEC.loader
build_traceability = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(build_traceability)
TraceabilityError = build_traceability.TraceabilityError
build = build_traceability.build
parse_args = build_traceability.parse_args
_PREP_PATH = Path(__file__).with_name("prepare_manifest.py")
_PREP_SPEC = importlib.util.spec_from_file_location("prepare_manifest_for_binding_tests", _PREP_PATH)
assert _PREP_SPEC and _PREP_SPEC.loader
prepare_manifest = importlib.util.module_from_spec(_PREP_SPEC)
_PREP_SPEC.loader.exec_module(prepare_manifest)
_PUBLISHER_PATH = Path(__file__).with_name("publish_issues.py")
_PUBLISHER_SPEC = importlib.util.spec_from_file_location("publisher_for_traceability_tests", _PUBLISHER_PATH)
assert _PUBLISHER_SPEC and _PUBLISHER_SPEC.loader
publisher = importlib.util.module_from_spec(_PUBLISHER_SPEC)
_PUBLISHER_SPEC.loader.exec_module(publisher)


def proposal(pid: str = "pki-root-abc", dependencies: list[str] | None = None) -> dict:
    return {"id": pid, "title": f"Issue {pid}", "parent": 9156, "kind": "design",
            "source_ranges": ["spec.md:1-2"], "requirement_ids": ["proposal-req"],
            "stage": "design", "lifecycle": "Ready", "urgency": 400,
            "depends_on": dependencies or [], "body": (
                "## Outcome\nDesign outcome.\n\n## Current evidence\nCurrent repository evidence.\n\n"
                "## Technical boundaries\nScope boundaries.\n\n## Acceptance\nAcceptance outcome.\n\n"
                "## Tests\nTest cases.\n\n## Failure modes\nFailure behavior.\n\n## Safety\nSafety gates.\n\n"
                "## Rollback\nRollback plan.\n\n## Estimate\nSize M.\n\n## Open decisions\nNone deferred.")}


def mapping_row(sid: str = "S-1", disposition: str = "proposal", pid: str = "pki-root-abc",
                canonical: str = "root-key", reason: str = "COVERED: synthetic restore invariant") -> dict:
    return {"source_id": sid, "canonical_requirement_id": canonical,
            "proposal_id": pid, "design_section": "designs/pki-roots.md#root",
            "test_oracle": "Run the synthetic restore fixture; compare exact root fingerprint and ensure no plaintext artifact.",
            "disposition": disposition, "reason": reason}


def receipt_records(manifest: list[dict], queue_state: str = "queued") -> list[dict]:
    validated = publisher.validate_manifest(manifest)
    plan = publisher.build_plan(manifest, validated, "publish")
    records = [{"event": "plan", "repo": publisher.REPO, "queue": plan["queue"],
                "proposal_digest": plan["proposal_digest"], "proposals": plan["proposals"],
                "started_at": "2026-11-12T00:00:00Z"}]
    issue_numbers = {item["id"]: 9400 + index for index, item in enumerate(validated)}
    database_ids = {item["id"]: 3000 + index for index, item in enumerate(validated)}
    for index, item in enumerate(validated):
        pid = item["id"]
        number = issue_numbers[pid]
        dedupe = f"repo-health:v1:triage:{publisher.REPO}:issue:{number}"
        task_id = 8000 + index
        issue_url = f"https://github.com/{publisher.REPO}/issues/{number}"
        issue = {"number": number, "url": issue_url, "database_id": database_ids[pid],
                 "marker": publisher.marker(item), "body_sha256": item["rendered_body_sha256"],
                 "parent": item["parent"],
                 "parent_issue_url": f"https://api.github.com/repos/{publisher.REPO}/issues/{item['parent']}",
                 "blocked_by": [{"number": issue_numbers[dep], "database_id": database_ids[dep]}
                                for dep in item["depends_on"]],
                 "labels": sorted(item["labels"]),
                 "lifecycle_history": {"checked": True, "closed": False, "reopened": False,
                                       "closed_then_reopened": False, "event_count": 0}}
        records.append({"event": "create_intent", "proposal_id": pid,
                        "marker": publisher.marker(item), "body_sha256": item["rendered_body_sha256"],
                        "parent": item["parent"], "title": item["title"], "at": "2026-11-12T00:00:01Z"})
        records.append({"event": "create_result", "proposal_id": pid, "number": number,
                        "url": issue_url, "body_sha256": item["rendered_body_sha256"],
                        "source": "create_response", "at": "2026-11-12T00:00:02Z"})
        records.append({"event": "issue_verified", "proposal_id": pid, "issue": issue,
                        "at": "2026-11-12T00:00:03Z"})
        records.append({"event": "queue_intent", "proposal_id": pid, "queue": plan["queue"],
                        "dedupe_key": dedupe, "issue_number": number,
                        "priority": item["priority"], "at": "2026-11-12T00:00:04Z"})
        ack = {"task_id": task_id, "queue": plan["queue"], "received_queue": plan["queue"],
               "expected_dedupe_key": dedupe, "received_dedupe_key": dedupe,
               "expected_priority": item["priority"], "received_priority": item["priority"],
               "output_excerpt": "accepted"}
        records.append({"event": "queue_ack", "proposal_id": pid, "ack": ack,
                        "at": "2026-11-12T00:00:05Z"})
        matched_row = {"id": task_id, "priority": item["priority"], "status": queue_state,
                       "age": "1m", "dedupe_key": dedupe,
                       "title": f"Triage {publisher.REPO}#{number}: {item['title']}"[:180]}
        statuses = sorted(publisher.LIVE_TASK_STATUSES)
        scope = {"queue": plan["queue"], "status_filters": statuses,
                 "completed_status_filters": statuses, "limit_per_status": 500,
                 "matched_row": matched_row}
        scans = {status: {"count": int(status == queue_state),
                          "ids": [task_id] if status == queue_state else [],
                          "limit": 500, "truncated": False} for status in statuses}
        task = {"id": task_id, "status": queue_state, "queue": plan["queue"],
                "dedupe_key": dedupe, "issue_number": number, "priority": item["priority"]}
        records.append({"event": "task_verified", "proposal_id": pid, "task": task,
                        "readback_scope": scope, "scans": scans,
                        "at": "2026-11-12T00:00:06Z"})
    return records


class Fixture:
    def __init__(self, temp: str, *, source_lines: list[list[int]] | None = None):
        self.root = Path(temp)
        (self.root / "inventory").mkdir()
        self.source_root = self.root / "source-snapshots/intake-34faffbff2/sources"
        self.source_root.mkdir(parents=True)
        self.source = self.source_root / "spec.md"
        self.source.write_text("MUST preserve\nNever expose\n", encoding="utf-8")
        raw = self.source.read_bytes()
        self.source_hash = hashlib.sha256(raw).hexdigest()
        self.provenance = self.root / "source-snapshots/provenance.json"
        self.provenance.write_text(json.dumps({
            "source_commit": "34faffbff26615e313488f6d04705a444342e41e",
            "files": [{"source_file": "spec.md", "source_commit": "34faffbff26615e313488f6d04705a444342e41e",
                       "original_sha256": self.source_hash,
                       "snapshot_path": "source-snapshots/intake-34faffbff2/sources/spec.md"}],
        }), encoding="utf-8")
        self.inv = self.root / "inventory/statements.jsonl"
        self.prop = self.root / "proposals.json"
        self.map = self.root / "mapping.json"
        self.member_map = self.root / "member-mapping.json"
        self.receipts = self.root / "receipts.jsonl"
        self.manifest = self.root / "published-manifest.json"
        self.write_inventory(source_lines=source_lines)
        self.write_proposals([proposal()])
        self.write_mapping([mapping_row()])
        self.receipts.write_text("", encoding="utf-8")

    def inventory_row(self, sid="S-1", source_lines=None, current_status="design"):
        source_lines = source_lines or [[1, 2]]
        spans = ["\n".join(["MUST preserve", "Never expose"][n - 1]
                            for n in range(start, end + 1)) for start, end in source_lines]
        excerpt = "\n\n--- source span ---\n\n".join(spans)
        return {
            "requirement_id": sid, "source_id": sid,
            "source_file_and_section": "sources/spec.md §1", "source_path": "spec.md",
            "source_lines": source_lines, "source_sha256": self.source_hash,
            "source_commit": "34faffbff26615e313488f6d04705a444342e41e",
            "source_excerpt": excerpt, "source_excerpt_sha256": hashlib.sha256(excerpt.encode()).hexdigest(),
            "canonical_requirement_ids": ["root-key"], "primary_starter": "#9156",
            "statement": "MUST preserve the canonical identity across an alias rename.",
            "source_precedence_resolution": "controlling", "user_outcome": "safe behavior",
            "severity_or_security_class": "security", "verified_current_behavior": "No implementation evidence",
            "record_class": "normative-or-proposed",
            "proposed_architecture_or_API": "explicit design", "primary_issue_number": "#9156",
            "dependent_issue_numbers": ["#9157"], "provider_capabilities_and_permissions": "scoped",
            "acceptance_test_or_benchmark": "oracle", "migration_or_rollback": "none",
            "evidence_status": "source", "current_status": current_status,
            "normative_history_admin_class": "normative requirement",
        }

    def write_inventory(self, rows=None, source_lines=None):
        rows = rows if rows is not None else [self.inventory_row(source_lines=source_lines)]
        self.inv.write_text("".join(json.dumps(row) + "\n" for row in rows), encoding="utf-8")

    def write_proposals(self, items):
        self.proposals = items
        self.prop.write_text(json.dumps(items), encoding="utf-8")

    def write_mapping(self, rows):
        self.map.write_text(json.dumps({"rows": rows}), encoding="utf-8")

    def write_receipts(self, records):
        self.receipts.write_text("".join(json.dumps(row) + "\n" for row in records), encoding="utf-8")

    def build(self, final=False, member_mapping_paths=None):
        member_paths = member_mapping_paths or ([str(self.member_map)] if self.member_map.exists() else [])
        if not final:
            return build([str(self.inv)], [str(self.prop)], [str(self.map)], str(self.receipts),
                         False, None, self.source_root, member_paths)
        inventory_rows = [json.loads(line) for line in self.inv.read_text().splitlines() if line.strip()]
        members = [member for row in inventory_rows
                   for key in ("atomic_members", "cohesive_members")
                   for member in row.get(key, [])]
        groups = {canonical for row in inventory_rows + members
                  for canonical in row.get("canonical_requirement_ids", [])}
        classes = dict(Counter(row["record_class"] for row in inventory_rows))
        with (patch.object(build_traceability, "CANONICAL_INVENTORY_SHA256", hashlib.sha256(self.inv.read_bytes()).hexdigest()),
              patch.object(build_traceability, "CANONICAL_SOURCE_COUNT", len(inventory_rows)),
              patch.object(build_traceability, "CANONICAL_MEMBER_COUNT", len(members)),
              patch.object(build_traceability, "CANONICAL_GROUP_COUNT", len(groups)),
              patch.object(build_traceability, "CANONICAL_CLASS_COUNTS", classes)):
            return build([str(self.inv)], [str(self.prop)], [str(self.map)], str(self.receipts),
                         True, str(self.manifest), self.source_root, member_paths)

    def prepare_published_manifest(self, proposals=None):
        source = proposals or self.proposals
        self.write_proposals(source)
        rows, _ = self.build()
        matrix = self.root / "traceability.jsonl"
        matrix.write_text("".join(json.dumps(row) + "\n" for row in rows), encoding="utf-8")
        policy = prepare_manifest.load_policy()
        normalized = [prepare_manifest.normalize_one(item, f"fixture[{index}]", policy)
                      for index, item in enumerate(source)]
        bound = prepare_manifest.bind_traceability(normalized, str(matrix), policy)
        self.write_proposals(bound)
        self.manifest.write_text(json.dumps(bound), encoding="utf-8")
        self.write_receipts(receipt_records(bound))
        return bound

    def with_atomic_member(self):
        row = self.inventory_row()
        row["atomic_members"] = [{
            "member_id": "S-1-atomic-001", "parent_source_id": "S-1",
            "source_path": "spec.md",
            "source_lines": [[1, 1]], "source_sha256": self.source_hash,
            "source_excerpt": "MUST preserve", "source_commit": "34faffbff26615e313488f6d04705a444342e41e",
            "statement": "A root has a canonical identity independent of alias rename.",
            "canonical_requirement_ids": ["root-key"], "primary_starter": "#9156",
            "specific_acceptance_oracle": "Decider: compare the persisted identity. Input: one synthetic root entry with fingerprint A. Member clause: root identity is canonical. Expected invariant: the fingerprint survives a rename. Reference: the source line and typed identity contract.",
            "oracle_origin": "reviewed-gap-refinement", "oracle_status": "source-specific-oracle-present",
            "normative_history_admin_class": "requirement", "record_class": "normative-or-proposed",
        }]
        self.write_inventory([row])
        member_mapping = {"member_id": "S-1-atomic-001", "parent_source_id": "S-1",
                          "canonical_requirement_id": "root-key", "proposal_id": "pki-root-abc",
                          "design_section": "designs/pki-roots.md#root", "test_oracle": row["atomic_members"][0]["specific_acceptance_oracle"],
                          "disposition": "proposal", "reason": "COVERED: the member's own identity invariant is implemented by this proposal."}
        self.member_map.write_text(json.dumps([member_mapping]), encoding="utf-8")
        return row, member_mapping


class BuildTraceabilityTests(unittest.TestCase):
    def test_draft_rows_preserve_lineage_and_separate_source_from_publication_status(self):
        with tempfile.TemporaryDirectory() as temp:
            fixture = Fixture(temp)
            rows, summary = fixture.build()
            row = rows[0]
            self.assertEqual(row["source_excerpt"], "MUST preserve\nNever expose")
            self.assertTrue(row["source_hash_verified"])
            self.assertEqual(row["source_commit"], "34faffbff26615e313488f6d04705a444342e41e")
            self.assertEqual(row["source_snapshot_commit"], "40af815bf0")
            self.assertIn("source-snapshots/intake-34faffbff2/sources", row["source_snapshot_path"])
            self.assertEqual(row["implementation_status"], "not-recorded")
            self.assertEqual(row["issue_publication_status"], "not-published")
            self.assertEqual(row["primary_issue_number"], "")
            self.assertEqual(row["dependent_issue_numbers"], "")
            self.assertEqual(row["domain_starter"], "#9156")
            self.assertEqual(summary["unique_canonical_requirements"], 1)
            self.assertTrue(summary["coverage_complete"])

    def test_source_excerpt_hash_self_consistency_does_not_replace_file_hash_and_exact_range_proof(self):
        with tempfile.TemporaryDirectory() as temp:
            fixture = Fixture(temp)
            row = fixture.inventory_row()
            row["source_excerpt"] = "forged but self-hashed"
            row["source_excerpt_sha256"] = hashlib.sha256(row["source_excerpt"].encode()).hexdigest()
            fixture.write_inventory([row])
            with self.assertRaisesRegex(TraceabilityError, "source_excerpt does not exactly match"):
                fixture.build()

    def test_multispan_excerpt_uses_inventory_source_span_separator(self):
        with tempfile.TemporaryDirectory() as temp:
            fixture = Fixture(temp)
            row = fixture.inventory_row(source_lines=[[1, 1], [2, 2]])
            fixture.write_inventory([row])
            rows, _ = fixture.build()
            self.assertEqual(rows[0]["source_excerpt"], "MUST preserve\n\n--- source span ---\n\nNever expose")

    def test_current_source_file_hash_must_match_inventory_for_every_disposition(self):
        with tempfile.TemporaryDirectory() as temp:
            fixture = Fixture(temp)
            fixture.source.write_text("Changed source\nNever expose\n", encoding="utf-8")
            row = fixture.inventory_row()
            row["normative_history_admin_class"] = "administrative"
            fixture.write_inventory([row])
            with self.assertRaisesRegex(TraceabilityError, "current source file hash differs"):
                fixture.build()

    def test_formatted_sources_view_is_never_used_as_fallback(self):
        with tempfile.TemporaryDirectory() as temp:
            fixture = Fixture(temp)
            formatted = fixture.root / "sources"
            formatted.mkdir()
            (formatted / "spec.md").write_text("1. MUST preserve\n2. Never expose\n", encoding="utf-8")
            with self.assertRaisesRegex(TraceabilityError, "source provenance path"):
                build([str(fixture.inv)], [str(fixture.prop)], [str(fixture.map)],
                      str(fixture.receipts), False, source_root=formatted)

    def test_malformed_ranges_fail_even_for_administrative_source(self):
        with tempfile.TemporaryDirectory() as temp:
            fixture = Fixture(temp)
            row = fixture.inventory_row(source_lines=[[2, 1]])
            row["normative_history_admin_class"] = "administrative"
            fixture.write_inventory([row])
            with self.assertRaisesRegex(TraceabilityError, "malformed inclusive source range"):
                fixture.build()

    def test_every_canonical_mapping_must_be_declared_on_source(self):
        with tempfile.TemporaryDirectory() as temp:
            fixture = Fixture(temp)
            fixture.write_mapping([mapping_row(canonical="invented-key")])
            with self.assertRaisesRegex(TraceabilityError, "not in source canonical_requirement_ids"):
                fixture.build()

    def test_missing_canonical_ids_are_allowed_only_with_explicit_context_disposition(self):
        with tempfile.TemporaryDirectory() as temp:
            fixture = Fixture(temp)
            row = fixture.inventory_row(current_status="historical")
            row["canonical_requirement_ids"] = []
            row["record_class"] = "historical"
            fixture.write_inventory([row])
            fixture.write_mapping([mapping_row(disposition="historical", pid="", canonical="",
                                               reason="HISTORICAL: source span is a superseded witness, not a requirement.")])
            _, summary = fixture.build()
            self.assertEqual(summary["active_unique_canonical_requirements"], 0)

    def test_conflicting_mapping_shards_are_unresolved_not_double_counted(self):
        with tempfile.TemporaryDirectory() as temp:
            fixture = Fixture(temp)
            second = Path(temp) / "second-map.json"
            second.write_text(json.dumps([mapping_row(pid="pki-root-other", reason="PARTIAL: second owner")]), encoding="utf-8")
            fixture.write_proposals([proposal(), proposal("pki-root-other")])
            rows, summary = build([str(fixture.inv)], [str(fixture.prop)],
                                  [str(fixture.map), str(second)], str(fixture.receipts), False,
                                  source_root=fixture.source_root)
            self.assertEqual(len(rows), 1)
            self.assertEqual(rows[0]["disposition"], "unresolved-cross-domain")
            self.assertEqual(summary["source_statements"], 1)
            self.assertEqual(summary["mapping_conflicts"], ["S-1"])
            with self.assertRaisesRegex(TraceabilityError, "unresolved duplicate mappings"):
                build_traceability._validate_mapping([str(fixture.map), str(second)], final=True)

    def test_final_duplicate_resolution_selects_exact_reviewed_shard_row_hash(self):
        with tempfile.TemporaryDirectory() as temp:
            fixture = Fixture(temp)
            second = Path(temp) / "second-map.json"
            second.write_text(json.dumps([mapping_row(pid="pki-root-other", reason="PARTIAL: competing owner")]), encoding="utf-8")
            selected = mapping_row(reason="COVERED: fully inspected exact proposal scope")
            fixture.write_mapping([selected])
            resolution_path = Path(temp) / "resolutions.json"
            digest = hashlib.sha256(json.dumps(selected, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()).hexdigest()
            resolution_path.write_text(json.dumps({"version": 1, "resolutions": [{
                "source_id": "S-1", "selected_from": "mapping.json", "selected_row_sha256": digest,
                "reason": "Full source and proposal-body review shows pki-root-abc owns the exact outcome; competing row is retained but not selected.",
            }]}), encoding="utf-8")
            mapping, conflicts, resolved = build_traceability._validate_mapping(
                [str(fixture.map), str(second)], final=True, resolution_paths=[str(resolution_path)])
            self.assertEqual(mapping["S-1"], selected)
            self.assertEqual(conflicts, {})
            self.assertEqual(resolved["S-1"]["selected_from"], "mapping.json")

    def test_unmapped_any_source_id_fails_final_not_only_normative(self):
        with tempfile.TemporaryDirectory() as temp:
            fixture = Fixture(temp)
            row = fixture.inventory_row()
            row["normative_history_admin_class"] = "historical"
            fixture.write_inventory([row])
            fixture.write_mapping([])
            fixture.manifest.write_text(json.dumps(fixture.proposals), encoding="utf-8")
            fixture.write_receipts(receipt_records(fixture.proposals))
            with self.assertRaisesRegex(TraceabilityError, "unmapped source ID"):
                fixture.build(final=True)

    def test_final_mode_rejects_fully_mapped_one_row_noncanonical_subset(self):
        with tempfile.TemporaryDirectory() as temp:
            fixture = Fixture(temp)
            fixture.manifest.write_text(json.dumps(fixture.proposals), encoding="utf-8")
            fixture.write_receipts(receipt_records(fixture.proposals))
            with self.assertRaisesRegex(TraceabilityError, "SHA-256 differs from CANONICAL_FREEZE"):
                build([str(fixture.inv)], [str(fixture.prop)], [str(fixture.map)],
                      str(fixture.receipts), True, str(fixture.manifest), fixture.source_root)

    def test_confirmed_issue_and_queue_receipts_override_starters_but_not_implementation_status(self):
        with tempfile.TemporaryDirectory() as temp:
            fixture = Fixture(temp)
            fixture.prepare_published_manifest()
            rows, summary = fixture.build(final=True)
            row = rows[0]
            self.assertEqual(row["primary_issue_number"], 9400)
            self.assertEqual(row["numeric_issue_receipt"], 9400)
            self.assertEqual(row["issue_publication_status"], "published")
            self.assertEqual(row["implementation_status"], "not-recorded")
            self.assertEqual(row["dependent_issue_numbers"], [])
            self.assertTrue(summary["coverage_complete"])

    def test_nonproposal_with_proposal_id_still_requires_published_numeric_receipt(self):
        with tempfile.TemporaryDirectory() as temp:
            fixture = Fixture(temp)
            fixture.map.write_text(json.dumps({"rows": [mapping_row(disposition="future", reason="FUTURE: later phase")]}), encoding="utf-8")
            fixture.prepare_published_manifest()
            fixture.receipts.unlink()
            with self.assertRaisesRegex(TraceabilityError, "nonempty publisher receipt"):
                fixture.build(final=True)

    def test_nonproposal_with_proposal_id_is_receipt_backed_when_published(self):
        with tempfile.TemporaryDirectory() as temp:
            fixture = Fixture(temp)
            fixture.write_mapping([mapping_row(disposition="future", reason="FUTURE: later phase")])
            fixture.prepare_published_manifest()
            rows, _ = fixture.build(final=True)
            self.assertEqual(rows[0]["issue_publication_status"], "published")
            self.assertEqual(rows[0]["numeric_issue_receipt"], 9400)

    def test_primary_and_dependent_issue_numbers_come_from_verified_numeric_edges(self):
        with tempfile.TemporaryDirectory() as temp:
            fixture = Fixture(temp)
            manifest = [proposal("Root-01"), proposal("Cert-02", ["Root-01"])]
            rows = [fixture.inventory_row("S-1"), fixture.inventory_row("S-2")]
            fixture.write_inventory(rows)
            fixture.write_proposals(manifest)
            fixture.write_mapping([mapping_row(sid="S-1", pid="Root-01"),
                                   mapping_row(sid="S-2", pid="Cert-02")])
            fixture.prepare_published_manifest(manifest)
            rows, _ = fixture.build(final=True)
            child = next(row for row in rows if row["source_id"] == "S-2")
            self.assertEqual(child["primary_issue_number"], 9401)
            self.assertEqual(child["dependent_issue_numbers"], [9400])

    def test_whitelisted_external_issue_is_not_mislabeled_as_published_or_implemented(self):
        with tempfile.TemporaryDirectory() as temp:
            fixture = Fixture(temp)
            ext = {"number": 9077, "url": "https://github.com/developing-today/code/issues/9077",
                   "title": "SSO/OIDC integration outcome", "state": "OPEN",
                   "verified_at": "2026-11-12T18:00:00-06:00", "verified_from": "parent fresh issue read",
                   "evidence": "Current issue scope explicitly owns the SSO/OIDC login outcome; this is not code verification."}
            row = mapping_row(disposition="external-issue", pid="",
                              reason="EXTERNAL-ISSUE: exact user outcome is owned by #9077, verified at the URL in external_issue.")
            row["design_section"] = "external issue #9077 (not a design section)"
            row["test_oracle"] = "Issue ownership only; implementation still needs source-code and acceptance-test evidence."
            row["external_issue"] = ext
            fixture.write_mapping([row])
            fixture.manifest.write_text(json.dumps(fixture.proposals), encoding="utf-8")
            fixture.write_receipts(receipt_records(fixture.proposals))
            rows, summary = fixture.build(final=True)
            self.assertEqual(rows[0]["primary_issue_number"], 9077)
            self.assertEqual(rows[0]["external_issue_number"], 9077)
            self.assertEqual(rows[0]["numeric_issue_receipt"], "")
            self.assertEqual(rows[0]["issue_publication_status"], "external-reference-verified")
            self.assertEqual(rows[0]["implementation_status"], "not-recorded")
            self.assertEqual(summary["reused_existing_issue_references"], 1)
            self.assertEqual(summary["verified_existing_capability_outcomes"], 0)
            self.assertEqual(summary["published_primary_issues"], 0)

    def test_external_issue_mapping_is_whitelisted_and_structured(self):
        with tempfile.TemporaryDirectory() as temp:
            fixture = Fixture(temp)
            row = mapping_row(disposition="external-issue", pid="", reason="EXTERNAL-ISSUE: exact scope")
            row["external_issue"] = {"number": 9999}
            fixture.write_mapping([row])
            with self.assertRaisesRegex(TraceabilityError, "fields must be exactly"):
                fixture.build()

    def test_external_issue_number_must_be_whitelisted(self):
        with tempfile.TemporaryDirectory() as temp:
            fixture = Fixture(temp)
            row = mapping_row(disposition="external-issue", pid="", reason="EXTERNAL-ISSUE: exact scope")
            row["external_issue"] = {"number": 9999, "url": "https://github.com/developing-today/code/issues/9999",
                                      "title": "External", "state": "OPEN",
                                      "verified_at": "2026-11-12T18:00:00Z", "verified_from": "parent",
                                      "evidence": "Exact owned scope evidence."}
            fixture.write_mapping([row])
            with self.assertRaisesRegex(TraceabilityError, "not on the intake whitelist"):
                fixture.build()

    def test_existing_capability_requires_actual_census_path(self):
        with tempfile.TemporaryDirectory() as temp:
            fixture = Fixture(temp)
            row = mapping_row(disposition="existing", pid="", reason="EXISTING-CODE: verified")
            fixture.write_mapping([row])
            with self.assertRaisesRegex(TraceabilityError, "requires census:"):
                fixture.build()

    def test_future_or_research_without_specific_issue_fails_final(self):
        with tempfile.TemporaryDirectory() as temp:
            fixture = Fixture(temp)
            fixture.write_mapping([mapping_row(disposition="future", pid="", reason="FUTURE: unowned scope")])
            fixture.manifest.write_text(json.dumps(fixture.proposals), encoding="utf-8")
            fixture.write_receipts(receipt_records(fixture.proposals))
            with self.assertRaisesRegex(TraceabilityError, "needs an exact proposal or external issue"):
                fixture.build(final=True)

    def test_generic_raw_oracle_is_reported_and_cannot_pass_final(self):
        with tempfile.TemporaryDirectory() as temp:
            fixture = Fixture(temp)
            row = mapping_row()
            row["test_oracle"] = "Independent issue states goal, dependencies, security/public implications, tests, docs, complexity, and acceptance."
            fixture.write_mapping([row])
            rows, summary = fixture.build()
            self.assertEqual(rows[0]["oracle_quality"], "needs-review")
            self.assertEqual(summary["concrete_outcome_oracles"], 0)
            matrix = fixture.root / "generic-oracle.jsonl"
            matrix.write_text("".join(json.dumps(item) + "\n" for item in rows), encoding="utf-8")
            normalized = prepare_manifest.normalize_one(fixture.proposals[0], "fixture", prepare_manifest.load_policy())
            with self.assertRaisesRegex(prepare_manifest.ManifestError, "not reviewed and outcome-specific"):
                prepare_manifest.bind_traceability([normalized], str(matrix), prepare_manifest.load_policy())

    def test_nested_members_are_separate_rows_and_never_inherit_parent_mapping(self):
        with tempfile.TemporaryDirectory() as temp:
            fixture = Fixture(temp)
            fixture.with_atomic_member()
            fixture.member_map.unlink()
            rows, summary = fixture.build()
            self.assertEqual(summary["source_statements"], 1)
            self.assertEqual(summary["nested_member_outcomes"], 1)
            self.assertEqual(summary["unmapped_member_ids"], 1)
            self.assertEqual([row["row_type"] for row in rows], ["source-statement", "atomic-member"])
            member_row = next(row for row in rows if row["row_type"] == "atomic-member")
            self.assertEqual(member_row["proposal_id"], "")
            self.assertEqual(member_row["parent_source_id"], "S-1")

    def test_every_member_has_explicit_parent_proposal_and_own_oracle_in_final(self):
        with tempfile.TemporaryDirectory() as temp:
            fixture = Fixture(temp)
            fixture.with_atomic_member()
            fixture.prepare_published_manifest()
            rows, summary = fixture.build(final=True)
            member_row = next(row for row in rows if row["row_type"] == "atomic-member")
            self.assertEqual(member_row["source_id"], "S-1-atomic-001")
            self.assertEqual(member_row["proposal_id"], "pki-root-abc")
            self.assertEqual(member_row["test_oracle"], "Decider: compare the persisted identity. Input: one synthetic root entry with fingerprint A. Member clause: root identity is canonical. Expected invariant: the fingerprint survives a rename. Reference: the source line and typed identity contract.")
            self.assertEqual(member_row["primary_issue_number"], 9400)
            self.assertEqual(summary["member_source_hashes_verified"], 1)

    def test_receipt_failure_event_is_not_publication_evidence(self):
        with tempfile.TemporaryDirectory() as temp:
            fixture = Fixture(temp)
            fixture.prepare_published_manifest()
            fixture.write_receipts(receipt_records(fixture.proposals) + [
                {"event": "failure", "error": "failed", "at": "2026-11-12T01:00:00Z"}])
            with self.assertRaisesRegex(TraceabilityError, "failure"):
                fixture.build(final=True)

    def test_wrong_plan_body_digest_and_unconfirmed_queue_each_fail(self):
        with tempfile.TemporaryDirectory() as temp:
            fixture = Fixture(temp)
            fixture.prepare_published_manifest()
            events = receipt_records(fixture.proposals)
            events[0]["proposal_digest"] = "0" * 64
            fixture.write_receipts(events)
            with self.assertRaisesRegex(TraceabilityError, "publisher receipt validation failed"):
                fixture.build(final=True)

    def test_actual_publisher_same_id_wrong_row_key_fails_readback_binding(self):
        with tempfile.TemporaryDirectory() as temp:
            fixture = Fixture(temp)
            fixture.prepare_published_manifest()
            events = receipt_records(fixture.proposals)
            task = next(row for row in events if row.get("event") == "task_verified")
            task["readback_scope"]["matched_row"]["dedupe_key"] = "repo-health:v1:triage:other/row"
            fixture.write_receipts(events)
            with self.assertRaisesRegex(TraceabilityError, "publisher receipt validation failed.*exact same ID/key/status observation"):
                fixture.build(final=True)
            events = receipt_records(fixture.proposals, queue_state="done")
            fixture.write_receipts(events)
            with self.assertRaisesRegex(TraceabilityError, "publisher receipt validation failed"):
                fixture.build(final=True)

    def test_final_requires_all_mapping_rows_to_be_covered_not_partial(self):
        with tempfile.TemporaryDirectory() as temp:
            fixture = Fixture(temp)
            fixture.map.write_text(json.dumps({"rows": [mapping_row(reason="PARTIAL: misses one clause")]}), encoding="utf-8")
            fixture.prepare_published_manifest()
            with self.assertRaisesRegex(TraceabilityError, "partially covered"):
                fixture.build(final=True)

    def test_duplicate_json_keys_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "bad.jsonl"
            path.write_text('{"source_id":"A","source_id":"B"}\n', encoding="utf-8")
            with self.assertRaisesRegex(TraceabilityError, "duplicate-key"):
                build_traceability._read_jsonl(path)

    def test_final_body_verification_requires_enrichment_clause_and_role(self):
        clause = "Reject an alias collision without replacing the persisted identity"
        body = ("S-1 root-key\n- Ownership role in this issue: primary\nstmt\n"
                "design-x\noracle-y\nreason-z\n" + clause)
        proposal = {"requirement_ids": ["S-1", "root-key"], "body": body}
        build_traceability._verify_issue_body_binding(
            proposal, "S-1", "root-key", "stmt", "design-x", "oracle-y",
            "reason-z", role="primary", acceptance_enrichment=[clause])
        stale = {"requirement_ids": ["S-1", "root-key"], "body": body.replace(clause, "")}
        with self.assertRaisesRegex(TraceabilityError, "lacks shared binding token"):
            build_traceability._verify_issue_body_binding(
                stale, "S-1", "root-key", "stmt", "design-x", "oracle-y",
                "reason-z", role="primary", acceptance_enrichment=[clause])
        with self.assertRaisesRegex(TraceabilityError, "Ownership role in this issue: supporting"):
            build_traceability._verify_issue_body_binding(
                proposal, "S-1", "root-key", "stmt", "design-x", "oracle-y",
                "reason-z", role="supporting", acceptance_enrichment=[clause])

    def test_coverage_ruling_reassigns_owner_and_binds_supporting_role(self):
        with tempfile.TemporaryDirectory() as temp:
            fixture = Fixture(temp)
            fixture.write_proposals([proposal(), proposal("new-owner")])
            fixture.write_mapping([mapping_row(
                disposition="unresolved-cross-domain",
                reason="UNRESOLVED/CROSS-DOMAIN: pki-root-abc is a policy boundary, not the evaluation owner")])
            unruled, _ = fixture.build()
            self.assertEqual(unruled[0]["oracle_quality"], "unresolved-no-acceptance-oracle")
            clause = "Model application-level immutable generation IDs, not native version IDs."
            ruling = {
                "source_or_member_id": "S-1", "primary_proposal_id": "new-owner",
                "supporting_proposal_ids": ["pki-root-abc"], "design_section": "proposals.json#new-owner",
                "test_oracle": "Restore two synthetic generations by manifest and reject a replayed manifest.",
                "coverage_rationale": "RESEARCH/PLANNED: new-owner evaluates this; pki-root-abc is policy only.",
                "acceptance_enrichment": [clause],
            }
            path = Path(temp) / "coverage-rulings.json"

            def ruled(doc):
                path.write_text(json.dumps([doc]), encoding="utf-8")
                return build([str(fixture.inv)], [str(fixture.prop)], [str(fixture.map)],
                             str(fixture.receipts), False, None, fixture.source_root, [], None, [str(path)])

            rows, _ = ruled(ruling)
            self.assertEqual(len(rows), 1)
            row = rows[0]
            self.assertEqual(row["proposal_id"], "new-owner")
            self.assertEqual(row["supporting_proposal_ids"], ["pki-root-abc"])
            self.assertEqual(row["disposition"], "research")
            self.assertEqual(row["oracle_quality"], "reviewed-specific-oracle")
            self.assertEqual(row["acceptance_enrichment"], [clause])
            self.assertEqual(row["issue_publication_status"], unruled[0]["issue_publication_status"])
            self.assertEqual(row["implementation_status"], unruled[0]["implementation_status"])
            matrix = Path(temp) / "traceability.jsonl"
            matrix.write_text("".join(json.dumps(item) + "\n" for item in rows), encoding="utf-8")
            policy = prepare_manifest.load_policy()
            normalized = [prepare_manifest.normalize_one(item, f"fixture[{index}]", policy)
                          for index, item in enumerate(fixture.proposals)]
            bound = {item["id"]: item for item in prepare_manifest.bind_traceability(normalized, str(matrix), policy)}
            self.assertIn("- Ownership role in this issue: primary", bound["new-owner"]["body"])
            self.assertNotIn("- Ownership role in this issue: supporting", bound["new-owner"]["body"])
            self.assertIn("- Ownership role in this issue: supporting", bound["pki-root-abc"]["body"])
            self.assertNotIn("- Ownership role in this issue: primary", bound["pki-root-abc"]["body"])
            for body in (bound["new-owner"]["body"], bound["pki-root-abc"]["body"]):
                self.assertIn(clause, body)
                self.assertIn("Restore two synthetic generations by manifest", body)
            with self.assertRaisesRegex(TraceabilityError, "supporting_proposal_ids must be distinct"):
                ruled({**ruling, "supporting_proposal_ids": ["new-owner"]})
            with self.assertRaisesRegex(TraceabilityError, "primary_proposal_id must resolve"):
                ruled({**ruling, "primary_proposal_id": "no-such-proposal"})

    def test_proposal_dependency_cycles_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            p1, p2 = proposal("Proposal-1", ["Proposal-2"]), proposal("Proposal-2", ["Proposal-1"])
            path = Path(temp) / "proposals.json"
            path.write_text(json.dumps([p1, p2]), encoding="utf-8")
            with self.assertRaisesRegex(TraceabilityError, "contains a cycle"):
                build_traceability._proposals([str(path)], final=False)

    def test_strict_arguments_require_manifest_final_and_reject_unknown(self):
        with self.assertRaises(SystemExit):
            parse_args(["--invent", "x", "--proposals", "p", "--mapping", "m", "--receipts", "r"])
        with self.assertRaises(SystemExit):
            parse_args(["--inventory", "x", "--proposals", "p", "--mapping", "m", "--receipts", "r", "--oops"])
        with self.assertRaises(SystemExit):
            parse_args(["--inventory", "x", "--proposals", "p", "--mapping", "m", "--receipts", "r", "--final"])

    def test_output_requires_explicit_existing_dir_and_overwrite_gate(self):
        with tempfile.TemporaryDirectory() as temp:
            fixture = Fixture(temp)
            rows, summary = fixture.build()
            output = Path(temp) / "reports"
            output.mkdir()
            build_traceability._write(rows, summary, output)
            self.assertTrue((output / "traceability.jsonl").is_file())
            self.assertTrue((output / "traceability.csv").is_file())
            self.assertTrue((output / "traceability-summary.md").is_file())
            self.assertTrue((output / "acceptance-enrichment.md").is_file())
            with self.assertRaisesRegex(TraceabilityError, "refusing to overwrite"):
                build_traceability._write(rows, summary, output)
            build_traceability._write(rows, summary, output, overwrite=True)

    def test_traceability_binding_enriches_requirement_ids_and_body_for_standalone_issue(self):
        with tempfile.TemporaryDirectory() as temp:
            fixture = Fixture(temp)
            rows, _ = fixture.build()
            matrix = Path(temp) / "traceability.jsonl"
            matrix.write_text("".join(json.dumps(row) + "\n" for row in rows), encoding="utf-8")
            policy = prepare_manifest.load_policy()
            authored_proposal = {
                "id": "pki-root-abc", "title": "Root seam", "parent": "#9156",
                "kind": "design", "source_ranges": ["spec.md:1-2"],
                "requirement_ids": ["AUTHORED-REQ"], "stage": "V2", "lifecycle": "Backlog",
                "urgency": 400, "depends_on": [], "body": "## Outcome\nDesign only",
            }
            normalized = prepare_manifest.normalize_one(authored_proposal, "fixture", policy)
            bound = prepare_manifest.bind_traceability([normalized], str(matrix), policy)[0]
            self.assertEqual(bound["requirement_ids"], ["AUTHORED-REQ", "root-key", "S-1"])
            self.assertIn('"source_and_member_ids"', bound["body"])
            self.assertIn("S-1", bound["body"])
            self.assertIn("MUST preserve the canonical identity across an alias rename.", bound["body"])
            self.assertIn("Planned acceptance oracle (not yet run)", bound["body"])

    def test_traceability_binding_fails_orphan_proposal_and_body_overflow(self):
        with tempfile.TemporaryDirectory() as temp:
            fixture = Fixture(temp)
            rows, _ = fixture.build()
            matrix = Path(temp) / "traceability.jsonl"
            matrix.write_text("".join(json.dumps(row) + "\n" for row in rows), encoding="utf-8")
            policy = prepare_manifest.load_policy()
            p1 = prepare_manifest.normalize_one({
                "id": "pki-root-abc", "title": "Root", "parent": 9156, "kind": "design",
                "source_ranges": ["s:1"], "requirement_ids": ["A"], "stage": "V2",
                "lifecycle": "Ready", "urgency": 400, "depends_on": [], "body": "Root body"}, "p1", policy)
            p2 = {**p1, "id": "orphan-issue", "title": "Orphan"}
            with self.assertRaisesRegex(prepare_manifest.ManifestError, "lack reciprocal source/member bindings"):
                prepare_manifest.bind_traceability([p1, p2], str(matrix), policy)
            oversized = dict(rows[0])
            oversized["source_statement"] = "X" * 70_000
            matrix.write_text(json.dumps(oversized) + "\n", encoding="utf-8")
            with self.assertRaisesRegex(prepare_manifest.ManifestError, "exceeds 65,536"):
                prepare_manifest.bind_traceability([p1], str(matrix), policy)


if __name__ == "__main__":
    unittest.main()
