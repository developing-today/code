#!/usr/bin/env python3
"""Oracle-named tests for the proposal contract and no-write default."""

import contextlib
import importlib.util
import io
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


SCRIPT = Path(__file__).with_name("publish_issues.py")
SPEC = importlib.util.spec_from_file_location("publish_issues", SCRIPT)
publisher = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(publisher)


def proposal(pid="sec-model-001", **overrides):
    value = {
        "id": pid,
        "title": "Define the canonical credential record",
        "parent": 9158,
        "kind": "design",
        "source_ranges": ["spec §8.1", "addendum §3"],
        "requirement_ids": ["REQ-SEC-014"],
        "stage": "design",
        "lifecycle": "Ready",
        "urgency": "normal",
        "depends_on": [],
        "body": "## Outcome\nA self-contained credential record contract.\n",
    }
    value.update(overrides)
    return value


class PublishIssuesTests(unittest.TestCase):
    def test_common_array_maps_to_existing_priority_scale_and_stable_marker(self):
        items = publisher.validate_manifest([proposal()])
        item = items[0]
        self.assertEqual(item["priority"], 40)
        self.assertEqual(item["priority_label"], "priority/normal")
        self.assertEqual(item["queue_kind"], "triage")
        self.assertEqual(publisher.marker(item), "<!-- fleet-ca-v03-proposal:sec-model-001 -->")

    def test_dependencies_are_topologically_ordered(self):
        blocker = proposal("sec-broker-prereq")
        blocked = proposal("sec-fnox-client", depends_on=["sec-broker-prereq"])
        result = publisher.validate_manifest([blocked, blocker])
        self.assertEqual([p["id"] for p in result], ["sec-broker-prereq", "sec-fnox-client"])

    def test_unknown_or_cyclic_dependency_rejected_before_writes(self):
        with self.assertRaisesRegex(publisher.PublishError, "unknown proposal IDs"):
            publisher.validate_manifest([proposal(depends_on=["missing-id"])])
        a = proposal("sec-proposal-a", depends_on=["sec-proposal-b"])
        b = proposal("sec-proposal-b", depends_on=["sec-proposal-a"])
        with self.assertRaisesRegex(publisher.PublishError, "cycle"):
            publisher.validate_manifest([a, b])

    def test_invalid_lifecycle_and_starter_rejected(self):
        with self.assertRaisesRegex(publisher.PublishError, "lifecycle"):
            publisher.validate_manifest([proposal(lifecycle="In Progress")])
        with self.assertRaisesRegex(publisher.PublishError, "domain starters"):
            publisher.validate_manifest([proposal(parent=9155)])
        with self.assertRaisesRegex(publisher.PublishError, "domain starters"):
            publisher.validate_manifest([proposal(parent=9161)])

    def test_strict_cli_requires_receipt_only_for_explicit_publish(self):
        dry = publisher.parse_args(["--manifest", "plan.json", "--queue", "repo-health-code-pilot"])
        self.assertFalse(dry.publish)
        with self.assertRaises(SystemExit):
            publisher.parse_args(["--manifest", "plan.json", "--queue", "q", "--publis"])
        with self.assertRaises(SystemExit):
            publisher.parse_args(["--manifest", "plan.json", "--queue", "q", "--publish"])

    def test_default_dry_run_prints_plan_without_subprocess_or_receipt(self):
        with tempfile.TemporaryDirectory() as tmp:
            manifest = Path(tmp) / "proposals.json"
            manifest.write_text(json.dumps([proposal()]), encoding="utf-8")
            out = io.StringIO()
            with patch.object(publisher.subprocess, "run", side_effect=AssertionError("network/write")):
                with contextlib.redirect_stdout(out):
                    result = publisher.main(["--manifest", str(manifest), "--queue", "repo-health-code-pilot"])
            plan = json.loads(out.getvalue())
            self.assertEqual(result, 0)
            self.assertEqual(plan["mode"], "dry-run")
            self.assertEqual(plan["proposals"][0]["parent"], 9158)
        self.assertIn("repo-health:v1:triage:developing-today/code:issue:<resolved-number>", plan["proposals"][0]["queue_dedupe_key"])

    def test_all_validation_precedes_first_subprocess(self):
        with tempfile.TemporaryDirectory() as tmp:
            manifest = Path(tmp) / "bad.json"
            manifest.write_text(json.dumps([proposal(urgency="urgent")]), encoding="utf-8")
            with patch.object(publisher.subprocess, "run", side_effect=AssertionError("subprocess called")):
                result = publisher.main(["--manifest", str(manifest), "--queue", "repo-health-code-pilot",
                                         "--publish", "--receipt", str(Path(tmp) / "receipt.json")])
            self.assertEqual(result, 2)
            self.assertFalse((Path(tmp) / "receipt.json").exists())

    def test_queue_packet_uses_only_supported_repo_task_kind(self):
        item = publisher.validate_manifest([proposal()])[0]
        packet = publisher.queue_packet(item, "repo-health-code-pilot", {
            "number": 12345, "title": item["title"], "url": "https://github.com/developing-today/code/issues/12345"
        })
        self.assertEqual(packet["payload"]["schema"], "repo-task/v1")
        self.assertEqual(packet["payload"]["kind"], "triage")
        self.assertEqual(packet["priority"], 40)
        self.assertEqual(packet["payload"]["timebox_minutes"], 20)
        self.assertEqual(packet["payload"]["max_attempts"], 6)
        self.assertEqual(packet["dedupe_key"], "repo-health:v1:triage:developing-today/code:issue:12345")


if __name__ == "__main__":
    unittest.main()
