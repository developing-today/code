#!/usr/bin/env python3
"""Focused offline regressions for delivery schema/gates; no live calls/writes."""

import importlib.util
import io
import json
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

HERE = Path(__file__).resolve().parent


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, HERE / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


prep = load("prepare_delivery", "prepare_delivery.py")
research_report = load("validate_research_report", "validate_research_report.py")


def row(
    *,
    sid="fixture-slice",
    statements=None,
    file="pkgs/id/tests/new_slice.rs",
    oracle=None,
):
    return {
        "id": sid,
        "parent": 9110,
        "title": "Preserve exact boundary behavior",
        "kind": "implementation",
        "version": "V2",
        "tier": "crucial",
        "statements": statements or ["S0.1"],
        "goal": "Encode/decode an explicit Option<Bytes> slot without collapsing null and empty bytes.",
        "enables": "Consumers can distinguish absent, null, and empty payload values.",
        "owned_files": [{"path": file, "status": "new"}],
        "requires": [],
        "after": [],
        "test": {
            "file": file,
            "command": "cargo test --manifest-path pkgs/id/Cargo.toml --test new_slice",
            "oracle": oracle
            or {
                "kind": "invariant",
                "expected": "Null remains distinct from an empty byte sequence.",
                "verifier": "The named test compares both outputs to fixed reference values.",
            },
        },
        "rollback": "Revert the new test and implementation change in this slice only.",
        "design_source": "crosscut/versioned-design.md",
        "design_section": "Decision summary",
        "owner": "fixture-owner",
    }


def matrix_report(scope="crdt-automerge"):
    report = {
        "schema_version": 1,
        "report_kind": "research-matrix",
        "research_scope": scope,
        "checked_at": "2026-11-12T00:00:00Z",
        "question": "Which option best fits the declared criteria?",
        "source_ledger": [
            {
                "id": "S1",
                "reference": "https://example.test/primary-a",
                "published_at": "2025-01-01T00:00:00Z",
                "accessed_at": "2026-11-12T00:00:00Z",
            },
            {
                "id": "S2",
                "reference": "https://example.test/primary-b",
                "published_at": "2025-02-01T00:00:00Z",
                "accessed_at": "2026-11-12T00:00:00Z",
            },
        ],
        "options": [{"id": "A", "label": "Option A"}, {"id": "B", "label": "Option B"}],
        "criteria": [
            {"id": "merge", "question": "What merge semantics are documented?"},
            {"id": "interop", "question": "What interoperability evidence exists?"},
        ],
        "assessments": [],
        "experiments": [],
        "decision": {
            "outcome": "inconclusive",
            "reason": "Evidence does not establish a winner.",
        },
        "limitations": [
            "No performance measurements were run.",
            "This report assesses traceability, not truth.",
        ],
    }
    for option in ("A", "B"):
        for criterion in ("merge", "interop"):
            report["assessments"].append(
                {
                    "option_id": option,
                    "criterion_id": criterion,
                    "status": "unknown",
                    "reason": "Not established from the reviewed source set.",
                    "evidence_refs": ["S1", "S2"],
                    "empirical_claim": False,
                }
            )
    return report


class DeliveryPreparationTests(unittest.TestCase):
    def validate_fixture(self, rows):
        with tempfile.TemporaryDirectory(dir=HERE) as temp:
            path = Path(temp) / "fixture.json"
            path.write_text(json.dumps({"slices": rows}))
            return prep.validate([path])

    def test_option_bytes_is_not_a_placeholder_and_structured_oracle_needs_no_keywords(
        self,
    ):
        errors = []
        normalized = prep.normalize_row(
            row(
                oracle={
                    "kind": "reference",
                    "expected": "blue blork",
                    "verifier": "ref checker",
                }
            ),
            "fixture",
            errors,
        )
        self.assertIsNotNone(normalized)
        self.assertFalse(any("placeholder" in error for error in errors))
        self.assertEqual(normalized["test"]["oracle"]["expected"], "blue blork")

    def test_free_text_oracle_requires_explicit_fresh_judge_review(self):
        candidate = row()
        candidate["test"]["oracle"] = "Expected result is documented here."
        errors = []
        prep.normalize_row(candidate, "fixture", errors)
        self.assertTrue(any("fresh_judge_review" in error for error in errors))
        candidate["fresh_judge_review"] = {
            "reviewer": "independent-judge",
            "reviewed_at": "2026-11-12T00:00:00Z",
            "decision": "accepted",
            "reference": "review/receipt.md#oracle",
        }
        errors = []
        normalized = prep.normalize_row(candidate, "fixture", errors)
        self.assertFalse(any("fresh_judge_review" in error for error in errors))
        self.assertEqual(normalized["test"]["oracle"]["kind"], "reviewed-text")

    def test_dot_paths_normalize_before_test_and_collision_comparison(self):
        candidate = row(file="./pkgs/id/tests/dot_test.rs")
        errors = []
        normalized = prep.normalize_row(candidate, "fixture", errors)
        self.assertEqual(normalized["owned"][0]["path"], "pkgs/id/tests/dot_test.rs")
        self.assertEqual(normalized["test"]["file"], "pkgs/id/tests/dot_test.rs")
        self.assertFalse(any("test.file must be owned" in error for error in errors))

    def test_hard_file_collision_requires_pairwise_transitive_order(self):
        a, b, c = ({"id": x} for x in "abc")
        files = {
            "shared.rs": [(a, {"owner": "a"}), (b, {"owner": "b"}), (c, {"owner": "c"})]
        }
        errors = []
        prep.check_file_collisions(files, {"a": {"b"}, "b": set(), "c": set()}, errors)
        self.assertEqual(
            sum("lack pairwise transitive hard ordering" in x for x in errors), 2
        )
        errors = []
        prep.check_file_collisions(
            files, {"a": {"b", "c"}, "b": {"c"}, "c": set()}, errors
        )
        self.assertFalse(errors)

    def test_crucial_tier_rule_checks_transitive_hard_dependencies(self):
        rows = [
            {"id": "a", "tier": "crucial"},
            {"id": "b", "tier": "crucial"},
            {"id": "c", "tier": "recommended"},
        ]
        errors = []
        prep.check_crucial_tier_reachability(
            rows, {"a": {"b", "c"}, "b": {"c"}, "c": set()}, errors
        )
        self.assertTrue(
            any(
                "a: crucial slice transitively depends on lower tier c" in x
                for x in errors
            )
        )

    def test_hard_null_dependency_is_a_validation_error_not_a_type_error(self):
        errors = []
        hard, soft = prep.dependency_edges(
            {"dependencies": {"hard": None, "soft": []}}, "x", errors
        )
        self.assertEqual(hard, [])
        self.assertEqual(soft, [])
        self.assertTrue(any("dependencies.hard must be an array" in x for x in errors))

    def test_requires_null_and_unknown_exact_ids_are_rejected(self):
        errors = []
        prep.dependency_edges({"requires": None}, "x", errors)
        self.assertTrue(any("requires must be an array" in x for x in errors))
        manifest = row()
        manifest["requires"] = ["missing-slice-id"]
        _, report = self.validate_fixture([manifest])
        self.assertTrue(
            any(
                "unknown exact slice dependency 'missing-slice-id'" in x
                for x in report["errors"]
            )
        )

    def test_exact_dependency_refs_and_soft_edge_separation(self):
        errors = []
        hard, soft = prep.dependency_edges(
            {
                "requires": [{"ref": "A-1", "description": "contract"}],
                "after": [{"ref": "B-2", "description": "sequence only"}],
            },
            "x",
            errors,
        )
        self.assertEqual([x["strength"] for x in hard], ["hard"])
        self.assertEqual([x["strength"] for x in soft], ["soft"])
        errors = []
        prep.dependency_edges({"requires": ["A-1 prose glued to ID"]}, "x", errors)
        self.assertTrue(any("exact slice ID" in x for x in errors))

    def test_filename_sanitization_collisions_are_detected(self):
        self.assertEqual(
            prep.duplicate_draft_stems([{"id": "A/B"}, {"id": "A-B"}]),
            [("A/B", "A-B", "A-B")],
        )

    def test_draft_preserves_tier_and_contingent_trigger(self):
        candidate = row()
        candidate["tier"] = "contingent"
        candidate["trigger"] = "When the owning deployment needs this adapter."
        errors = []
        normalized = prep.normalize_row(candidate, "fixture", errors)
        body = prep.draft(normalized, "0" * 64)
        self.assertIn("Tier: **contingent**", body)
        self.assertIn(
            "Start trigger: When the owning deployment needs this adapter.", body
        )

    def test_output_source_alias_is_refused(self):
        with self.assertRaises(ValueError):
            prep.safe_output_path(prep.MAP, [prep.MAP])

    def test_live_snapshot_is_a_protected_input(self):
        inputs = prep.protected_inputs([], [])
        self.assertIn(prep.LIVE_SNAPSHOT, inputs)
        with self.assertRaises(ValueError):
            prep.safe_output_path(prep.LIVE_SNAPSHOT, inputs)

    def test_existing_hardlink_to_input_is_refused_before_report_write(self):
        with tempfile.TemporaryDirectory(dir=HERE) as temp:
            source = Path(temp) / "authored.json"
            report_alias = Path(temp) / "report.json"
            source.write_text('{"keep":"authored"}\n')
            report_alias.hardlink_to(source)
            with self.assertRaises(ValueError):
                prep.safe_output_path(report_alias, [source])
            self.assertEqual(source.read_text(), '{"keep":"authored"}\n')
            self.assertTrue(prep.output_paths_overlap(source, report_alias))

    def test_github_anchor_preserves_one_hyphen_per_whitespace(self):
        self.assertEqual(prep.github_anchor("9113 — Foo"), "9113--foo")
        self.assertEqual(prep.github_anchor("Some  Section"), "some--section")
        self.assertEqual(prep.github_anchor("Some\tSection"), "somesection")

    def test_atx_heading_parser_preserves_literal_issue_hashes_and_exact_text(self):
        headings = prep.atx_headings(
            "## 4. #9125 — Graph\n### Wire/data model\n### #9133 — Storage\n"
        )
        self.assertEqual(
            [x["rendered"] for x in headings],
            ["4. #9125 — Graph", "Wire/data model", "#9133 — Storage"],
        )
        self.assertEqual(headings[0]["raw"], "## 4. #9125 — Graph")

    def test_heading_accepts_only_exact_raw_or_rendered_heading(self):
        candidate = row()
        candidate["design_section"] = "## Decision summary"
        errors = []
        normalized = prep.normalize_row(candidate, "fixture", errors)
        self.assertEqual(normalized["design_section"], "Decision summary")
        self.assertFalse(any("design_section is not an exact" in x for x in errors))
        candidate["design_section"] = "Decision"
        errors = []
        prep.normalize_row(candidate, "fixture", errors)
        self.assertTrue(any("design_section is not an exact" in x for x in errors))

    def test_additional_well_formed_research_scope_is_allowed(self):
        candidate = row()
        candidate["kind"] = "research"
        candidate["research_scope"] = "reputation-abuse-evaluation"
        candidate["trigger"] = "When the risk review starts."
        candidate["research_evidence"] = ["part-b/research.md#evidence"]
        candidate["test"]["command"] = (
            f"python3 -I doc/2026-11-13T03-40-00Z_design_id-atlas-summit/delivery/validate_research_report.py --input {candidate['test']['file']}"
        )
        candidate["test"]["oracle"]["kind"] = "research-matrix"
        errors = []
        normalized = prep.normalize_row(candidate, "fixture", errors)
        self.assertEqual(normalized["research_scope"], "reputation-abuse-evaluation")
        self.assertFalse(any("unknown research_scope" in x for x in errors))

    def test_research_can_create_an_exact_owned_golden_validator(self):
        script = "doc/2026-11-13T03-40-00Z_design_id-atlas-summit/crosscut/future_profile_validator.py"
        candidate = row(file=script)
        candidate["kind"] = "research"
        candidate["research_evidence"] = ["pkgs/id/crates/id-sync/src/artifact.rs"]
        candidate["test"]["command"] = f"python3 -I {script}"
        candidate["test"]["oracle"] = {
            "kind": "golden",
            "expected": "Exact legacy preimages match the independent known-answer vectors.",
            "verifier": "The owned validator recomputes bytes and compares the fixed expected digests.",
        }
        errors = []
        prep.normalize_row(candidate, "fixture", errors)
        self.assertFalse(errors)

    def test_unowned_missing_research_validator_is_not_executable_evidence(self):
        missing = "doc/unowned_missing_validator.py"
        self.assertFalse(
            prep.research_command_has_artifact_validator(
                f"python3 -I {missing} --input future-result.json",
                "future-result.json",
                "crdt-automerge",
                [],
            )
        )

    def test_research_traceability_lists_cannot_be_empty(self):
        report = matrix_report()
        for item in report["assessments"]:
            item["status"] = "supported"
            item["evidence_refs"] = []
        self.assertTrue(
            any(
                "nonempty evidence_refs" in e
                for e in research_report.validate_report(report)
            )
        )
        report = matrix_report()
        report["limitations"] = []
        self.assertTrue(
            any("limitations" in e for e in research_report.validate_report(report))
        )
        self.assertFalse(research_report.strings([]))

    def test_strict_cli_rejects_unknown_and_abbreviated_options_before_writes(self):
        destination = HERE / "must-not-exist-from-cli-test"
        for argv in (["--out", str(destination), "--typo"], ["--app"]):
            buffer = io.StringIO()
            with (
                redirect_stdout(buffer),
                redirect_stderr(buffer),
                self.assertRaises(SystemExit) as raised,
            ):
                prep.main(list(argv))
            self.assertEqual(raised.exception.code, 2)
        self.assertFalse(destination.exists())

    def test_report_envelope_contains_normalized_rows_and_meta(self):
        with tempfile.TemporaryDirectory(dir=HERE) as temp:
            source = Path(temp) / "manifest.json"
            report = Path(temp) / "report.json"
            source.write_text(json.dumps({"slices": [row()]}))
            buffer = io.StringIO()
            with redirect_stdout(buffer), redirect_stderr(buffer):
                result = prep.main(["--manifest", str(source), "--report", str(report)])
            self.assertEqual(
                result, 1
            )  # Missing other 37 homes is correctly a gate failure.
            body = json.loads(report.read_text())
            self.assertIn("manifest_sha256", body)
            self.assertFalse(body["valid"])
            self.assertEqual(body["entries"][0]["id"], "fixture-slice")
            self.assertIn("orchestration", body)

    def test_research_matrix_accepts_complete_crdt_comparison(self):
        self.assertEqual(research_report.validate_report(matrix_report()), [])

    def test_research_matrix_accepts_local_benchmark_plan_without_claiming_measurements(
        self,
    ):
        report = matrix_report("local-benchmark-plan")
        report["experiments"] = [
            {
                "id": "cloud-run-1",
                "status": "unmeasured",
                "method": "equal-recall fixture plan",
                "checked_at": "2026-11-12T00:00:00Z",
                "measurements": [],
            }
        ]
        self.assertEqual(research_report.validate_report(report), [])

    def test_research_matrix_rejects_missing_criterion_and_unknown_source(self):
        report = matrix_report()
        report["assessments"].pop()
        report["assessments"][0]["evidence_refs"] = ["absent"]
        errors = research_report.validate_report(report)
        self.assertTrue(any("matrix incomplete" in error for error in errors))
        self.assertTrue(any("absent from source_ledger" in error for error in errors))

    def test_research_matrix_rejects_empirical_claim_without_experiment(self):
        report = matrix_report()
        report["assessments"][0]["empirical_claim"] = True
        report["assessments"][0]["experiment_ref"] = "missing-run"
        self.assertTrue(
            any(
                "measured experiment_ref" in error
                for error in research_report.validate_report(report)
            )
        )

    def test_original30_accepts_bounded_unrecovered_or_exact_recovery_only(self):
        report = {
            "schema_version": 1,
            "research_scope": "original-30-provenance-recovery",
            "status": "unrecovered",
            "checked_at": "2026-11-12T00:00:00Z",
            "bounded_scan": {
                "complete": True,
                "scope": "bounded primary-source and repository search",
                "methods": ["issue provenance review"],
            },
            "search_log": [
                {
                    "source": "candidate-provenance.md",
                    "result": "No exact original list recovered",
                    "checked_at": "2026-11-12T00:00:00Z",
                }
            ],
            "original_items": [],
            "reason": "Unrecovered after the documented bounded scan.",
        }
        self.assertEqual(research_report.validate_report(report), [])
        report["original_items"] = [
            {"id": "invented", "title": "not source-backed", "source_ref": "none"}
        ]
        self.assertTrue(
            any(
                "must not fabricate" in error
                for error in research_report.validate_report(report)
            )
        )
        report["status"] = []
        self.assertIsInstance(
            research_report.validate_report(report), list
        )  # Malformed type never raises.

        report["status"] = "recovered"
        report["original_items"] = [
            {
                "id": f"I{i}",
                "title": f"Original item {i}",
                "source_ref": "source-record",
            }
            for i in range(30)
        ]
        self.assertEqual(research_report.validate_report(report), [])

    def test_research_validator_cli_requires_input_before_io(self):
        output = io.StringIO()
        with (
            redirect_stdout(output),
            redirect_stderr(output),
            self.assertRaises(SystemExit) as raised,
        ):
            research_report.main(
                ["--input", str(HERE / "must-not-be-read.json"), "--unknown"]
            )
        self.assertEqual(raised.exception.code, 2)

    def test_original_30_row_is_issue_ready_research_shape(self):
        doc = json.loads((HERE / "research-slices.json").read_text())
        errors = []
        normalized = prep.normalize_row(
            doc["research_slices"][0], "delivery/research-slices.json", errors
        )
        self.assertEqual(errors, [])
        self.assertEqual(normalized["parent"], 9112)
        self.assertEqual(
            normalized["research_scope"], "original-30-provenance-recovery"
        )
        self.assertEqual(normalized["test"]["oracle"]["kind"], "research-matrix")


if __name__ == "__main__":
    unittest.main()
