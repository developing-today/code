from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

_SCRIPT = Path(__file__).with_name("prepare_manifest.py")
_SPEC = importlib.util.spec_from_file_location("prepare_manifest", _SCRIPT)
assert _SPEC and _SPEC.loader
prepare_manifest = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(prepare_manifest)
ManifestError = prepare_manifest.ManifestError


def authored(**changes):
    value = {
        "id": "TRACE-01", "title": "Trace test", "parent": "#9156",
        "kind": "design+implementation+test", "source_ranges": ["spec.md:1-2"],
        "requirement_ids": ["REQ-1", "SRC-1"], "stage": "V2", "lifecycle": "Backlog",
        "urgency": 600, "depends_on": [], "body": "Design details",
        "size": "M", "owned_files_or_interfaces": ["src/lib.rs"], "lane": "security",
        "security_classification": "high", "urgency_band": "High",
        "priority_band": "High", "priority": "High", "queue_priority_projection": 20,
    }
    value.update(changes)
    return value


class PrepareManifestTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.policy = prepare_manifest.load_policy()

    def test_exact_publisher_schema_and_all_metadata_preserved(self):
        result = prepare_manifest.normalize_one(authored(), "fixture", self.policy)
        self.assertEqual(set(result), prepare_manifest.REQUIRED)
        self.assertEqual(result["id"], "TRACE-01")
        self.assertEqual(result["parent"], 9156)
        self.assertEqual(result["kind"], "implementation")
        self.assertIn('"original_kind":"design+implementation+test"', result["body"])
        self.assertIn('"owned_files_or_interfaces":["src/lib.rs"]', result["body"])
        self.assertIn('"priority_band":"High"', result["body"])
        self.assertIn("34faffbff26615e313488f6d04705a444342e41e", result["body"])
        self.assertIn("40af815bf0", result["body"])
        self.assertIn("source-snapshots/intake-34faffbff2/sources", result["body"])
        self.assertIn("source-integrity.md", result["body"])
        self.assertIn("keep this issue open until its own acceptance evidence passes", result["body"])
        self.assertIn("not production authorization", result["body"])
        self.assertIn("mixed deliverable includes code and tests", result["body"])

    def test_unknown_extra_and_missing_canonical_fields_fail(self):
        with self.assertRaisesRegex(ManifestError, "unknown=.*surprise"):
            prepare_manifest.normalize_one(authored(surprise="do not drop"), "fixture", self.policy)
        item = authored()
        del item["requirement_ids"]
        with self.assertRaisesRegex(ManifestError, "missing=.*requirement_ids"):
            prepare_manifest.normalize_one(item, "fixture", self.policy)

    def test_duplicate_authored_json_keys_are_rejected_before_normalization(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "duplicate.json"
            path.write_text('[{"id":"ARCH-01","id":"ARCH-02"}]', encoding="utf-8")
            with self.assertRaisesRegex(ManifestError, "duplicate JSON key"):
                prepare_manifest._read(path)

    def test_duplicate_policy_keys_are_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "policy.json"
            path.write_text('{"version":1,"version":1}', encoding="utf-8")
            with patch.object(prepare_manifest, "POLICY_PATH", path):
                with self.assertRaisesRegex(ManifestError, "duplicate JSON key"):
                    prepare_manifest.load_policy()

    def test_kind_requires_exact_explicit_mapping(self):
        with self.assertRaisesRegex(ManifestError, "no explicit reviewed normalization"):
            prepare_manifest.normalize_one(authored(kind="design+new-magic-test"), "fixture", self.policy)

    def test_per_proposal_kind_override_uses_owned_code_deliverable(self):
        result = prepare_manifest.normalize_one(
            authored(id="ARCH-01", kind="design+implementation+test"), "fixture", self.policy)
        self.assertEqual(result["kind"], "implementation")
        self.assertEqual(result["title"], "Trace test")
        self.assertIn("design+implementation+test", result["body"])
        self.assertIn("deliverable is code", result["body"])
        with self.assertRaisesRegex(ManifestError, "differs from the exact authored kind"):
            prepare_manifest.normalize_one(authored(id="ARCH-01", kind="design"), "fixture", self.policy)

    def test_priority_band_and_projection_must_match_urgency(self):
        for change in ({"urgency_band": "Standard"}, {"priority_band": "Standard"},
                       {"queue_priority_projection": 40}, {"priority": 40}):
            with self.subTest(change=change), self.assertRaisesRegex(ManifestError, "disagrees with urgency"):
                prepare_manifest.normalize_one(authored(**change), "fixture", self.policy)

    def test_publisher_parent_and_title_contract_is_enforced(self):
        with self.assertRaisesRegex(ManifestError, "existing domain starter"):
            prepare_manifest.normalize_one(authored(parent="#9999"), "fixture", self.policy)
        with self.assertRaisesRegex(ManifestError, "one line of at most 256"):
            prepare_manifest.normalize_one(authored(title="x" * 257), "fixture", self.policy)

    def test_canonical_priority_boundaries(self):
        cases = [(399, 10), (400, 20), (799, 20), (800, 40), (1199, 40),
                 (1200, 70), (1599, 70), (1600, 70), (1999, 70), (2000, 70)]
        for score, expected in cases:
            with self.subTest(score=score):
                self.assertEqual(prepare_manifest._priority(score, self.policy)["queue_priority_projection"], expected)

    def test_global_dag_cycle_fails_but_case_distinct_ids_remain_exact(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "proposals.json"
            one, two = authored(), authored(id="TRACE-02")
            one["depends_on"], two["depends_on"] = ["TRACE-02"], ["TRACE-01"]
            path.write_text(json.dumps([one, two]), encoding="utf-8")
            with self.assertRaisesRegex(ManifestError, "dependency cycle"):
                prepare_manifest.normalize([str(path)], self.policy)
            one, two = authored(), authored(id="trace-02")
            one["depends_on"] = ["trace-02"]
            path.write_text(json.dumps([one, two]), encoding="utf-8")
            normalized = prepare_manifest.normalize([str(path)], self.policy)
            self.assertEqual(normalized[0]["id"], "TRACE-01")
            self.assertEqual(normalized[0]["depends_on"], ["trace-02"])
            self.assertEqual(normalized[1]["id"], "trace-02")
            one["depends_on"] = ["Trace-02"]
            path.write_text(json.dumps([one, two]), encoding="utf-8")
            with self.assertRaisesRegex(ManifestError, "unknown proposal dependencies"):
                prepare_manifest.normalize([str(path)], self.policy)

    def test_argument_contract_and_write_gate(self):
        with self.assertRaises(SystemExit):
            prepare_manifest.parse_args(["--propos", "x"])
        with self.assertRaises(SystemExit):
            prepare_manifest.parse_args(["--proposal", "x", "--output-dir", "."])
        with self.assertRaises(SystemExit):
            prepare_manifest.parse_args(["--proposal", "x", "--write"])
        with self.assertRaises(SystemExit):
            prepare_manifest.parse_args(["--proposal", "x", "--overwrite"])


if __name__ == "__main__":
    unittest.main()
