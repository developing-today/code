#!/usr/bin/env python3
"""Offline safety tests for publish_one; all external adapters are fakes/spies."""

import importlib.util
import tempfile
import unittest
from pathlib import Path
import subprocess
from unittest.mock import patch

SCRIPT = Path(__file__).with_name("publish_one.py")
spec = importlib.util.spec_from_file_location("publish_one", SCRIPT)
pmo = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pmo)


class PublisherSafetyTests(unittest.TestCase):
    def test_unknown_args_fail_before_effects(self):
        with self.assertRaises(SystemExit):
            pmo.parse_args(["--apply", "CEN-01", "--surprise"])

    def test_lowercase_manifest_key_is_not_rewritten(self):
        args = pmo.parse_args(["--apply", "PRI-05a"])
        self.assertEqual(args.apply, "PRI-05a")

    def test_invalid_key_stops_before_any_external_adapter(self):
        with (
            patch.object(pmo, "specs_and_aliases", return_value=([], {})),
            patch.object(
                pmo,
                "gh",
                side_effect=AssertionError("invalid key must fail before GitHub"),
            ),
        ):
            with self.assertRaisesRegex(ValueError, "not a canonical"):
                pmo.apply_one("NO-SUCH-KEY")

    def test_cycle_and_whole_parent_are_rejected(self):
        base = lambda key, blockers: {
            "key": key,
            "hard_blockers": blockers,
            "owned_files": [],
        }
        issues, stats = pmo.graph_and_collision_problems(
            [base("A", ["B"]), base("B", ["A"])]
        )
        self.assertTrue(any("cycles" in x for x in issues))
        issues, _ = pmo.graph_and_collision_problems([base("A", ["#9198"])])
        self.assertTrue(any("whole domain" in x for x in issues))

    def test_prepare_refuses_to_discard_prior_intent(self):
        with (
            patch.object(
                pmo,
                "read_index",
                return_value={"operations": {"CEN-01": {"status": "create_intent"}}},
            ),
            patch.object(
                pmo,
                "source_candidates",
                side_effect=AssertionError("must stop before regeneration"),
            ),
        ):
            with self.assertRaisesRegex(ValueError, "refuses to overwrite"):
                pmo.prepare()

    def test_numbered_complete_retry_gets_exact_issue_not_search_or_create(self):
        canonical = {"CEN-01": {"key": "CEN-01", "bodypath": "bodies/CEN-01.md"}}
        index = {"operations": {"CEN-01": {"issue_number": 7, "status": "complete"}}}
        with (
            patch.object(pmo, "specs_and_aliases", return_value=([], canonical)),
            patch.object(
                pmo, "check", return_value={"apply_ready": True, "problems": []}
            ),
            patch.object(pmo, "read_index", return_value=index),
            patch.object(
                pmo, "reconcile_one", return_value={"status": "readback"}
            ) as reconcile,
            patch.object(
                pmo,
                "gh",
                side_effect=AssertionError(
                    "must use durable number before any search/create"
                ),
            ),
        ):
            result = pmo.apply_one("CEN-01")
        reconcile.assert_called_once()
        self.assertEqual(result["status"], "readback")

    def test_lost_create_response_never_blindly_creates_again(self):
        spec_row = {"key": "CEN-01", "bodypath": "bodies/CEN-01.md"}
        index = {
            "operations": {
                "CEN-01": {
                    "key": "CEN-01",
                    "marker": "<!-- marker -->",
                    "status": "create_intent",
                }
            }
        }
        with (
            patch.object(pmo, "marker_search", return_value=([], True, "")),
            patch.object(pmo, "save_index") as save,
            patch.object(
                pmo,
                "gh",
                side_effect=AssertionError("reconciliation must not call create"),
            ),
        ):
            result = pmo.reconcile_one(
                "CEN-01", index, spec_row, create_if_absent=False
            )
        self.assertEqual(result["status"], "uncertain")
        self.assertNotIn("issue_number", index["operations"]["CEN-01"])
        save.assert_called()

    def test_adjacent_shared_writer_chain_is_acyclic_and_records_reason(self):
        rows = [
            {
                "key": "A",
                "kind": "contract",
                "hard_blockers": [],
                "owned_files": ["shared.rs"],
                "hard_blocker_reasons": [],
                "bodypath": "bodies/A.md",
            },
            {
                "key": "B",
                "kind": "implementation",
                "hard_blockers": [],
                "owned_files": ["shared.rs"],
                "hard_blocker_reasons": [],
                "bodypath": "bodies/B.md",
            },
            {
                "key": "C",
                "kind": "implementation",
                "hard_blockers": [],
                "owned_files": ["shared.rs"],
                "hard_blocker_reasons": [],
                "bodypath": "bodies/C.md",
            },
        ]
        bodies = {"bodies/A.md": "", "bodies/B.md": "", "bodies/C.md": ""}
        result = pmo.add_shared_file_sequences(rows, bodies)
        by = {r["key"]: r for r in rows}
        self.assertEqual(by["B"]["hard_blockers"], ["A"])
        self.assertEqual(by["C"]["hard_blockers"], ["B"])
        self.assertEqual(result["final_topological_order"], ["A", "B", "C"])
        self.assertTrue(
            all(
                "shared-file:shared.rs" in x
                for x in [
                    by["B"]["hard_blocker_reasons"][0]["reason"],
                    by["C"]["hard_blocker_reasons"][0]["reason"],
                ]
            )
        )

    def test_parent_link_failure_does_not_skip_same_pass_queue_coverage(self):
        with tempfile.TemporaryDirectory() as td:
            body = Path(td) / "body.md"
            body.write_text("<!-- pmo-9196:CEN-01 -->\nbody")
            spec_row = {
                "key": "CEN-01",
                "title": "title",
                "bodypath": str(body),
                "labels": [],
                "parent": 9198,
                "hard_blockers": [],
                "source_refs": [],
                "design_refs": [],
                "goal": "goal",
                "acceptance": [],
            }
            issue = {
                "number": 7,
                "id": 70,
                "html_url": "https://example/7",
                "title": "title",
                "state": "open",
                "body": body.read_text(),
                "labels": [],
            }
            index = {
                "operations": {
                    "CEN-01": {
                        "key": "CEN-01",
                        "marker": "<!-- pmo-9196:CEN-01 -->",
                        "issue_number": 7,
                        "database_id": 70,
                        "body_sha256": __import__("hashlib")
                        .sha256(body.read_bytes())
                        .hexdigest(),
                    }
                }
            }
            gh_results = [
                subprocess.CompletedProcess([], 1, "", "404 Not Found"),
                subprocess.CompletedProcess(
                    [], 1, "", "simulated parent write failure"
                ),
            ]
            calls = []

            def run(args, **kwargs):
                calls.append(args)
                return subprocess.CompletedProcess(
                    args,
                    0,
                    "validated"
                    if args[0] == "repo-queue" and args[1] == "validate"
                    else '{"id":"task-1","status":"queued"}',
                    "",
                )

            with (
                patch.object(pmo, "api_get", return_value=issue),
                patch.object(pmo, "gh", side_effect=gh_results),
                patch.object(
                    pmo,
                    "make_packet",
                    return_value={"dedupe_key": "d", "title": "triage"},
                ),
                patch.object(pmo.subprocess, "run", side_effect=run),
                patch.object(
                    pmo,
                    "parse_enqueue_response",
                    return_value={"id": "task-1", "status": "queued"},
                ),
                patch.object(
                    pmo,
                    "coverage_readback",
                    return_value={
                        "covered_issues": [
                            {"number": 7, "task": "task-1", "status": "claimed"}
                        ]
                    },
                ),
                patch.object(pmo, "save_index"),
            ):
                result = pmo.finish_issue(
                    "CEN-01", index, spec_row, index["operations"]["CEN-01"]
                )
            self.assertTrue(result["publication_verified"])
            self.assertEqual(result["parent_status"], "uncertain")
            self.assertEqual(result["task_id"], "task-1")
            self.assertEqual([x[1] for x in calls], ["validate", "enqueue"])

    def test_reconcile_readonly_never_posts_or_mutates_index(self):
        row = {
            "key": "CEN-01",
            "marker": "<!-- marker -->",
            "issue_number": 7,
            "task_id": "task-1",
        }
        canonical = {
            "CEN-01": {
                "key": "CEN-01",
                "title": "t",
                "parent": 9198,
                "bodypath": "bodies/CEN-01.md",
                "labels": [],
            }
        }
        issue = {
            "number": 7,
            "id": 70,
            "html_url": "https://example/7",
            "title": "t",
            "state": "open",
            "body": "<!-- marker -->\nbody",
            "labels": [],
        }
        with tempfile.TemporaryDirectory() as td:
            body = Path(td) / "body.md"
            body.write_text("<!-- marker -->\nbody")
            canonical["CEN-01"]["bodypath"] = str(body)
            idx = {"operations": {"CEN-01": row}}
            with (
                patch.object(pmo, "specs_and_aliases", return_value=([], canonical)),
                patch.object(pmo, "read_index", return_value=idx),
                patch.object(pmo, "api_get", side_effect=[issue, {"number": 9198}]),
                patch.object(
                    pmo,
                    "coverage_readback",
                    return_value={
                        "covered_issues": [
                            {"number": 7, "status": "claimed", "task": "task-1"}
                        ]
                    },
                ),
                patch.object(
                    pmo,
                    "save_index",
                    side_effect=AssertionError(
                        "read-only reconcile must not write index"
                    ),
                ),
                patch.object(
                    pmo,
                    "gh",
                    side_effect=AssertionError(
                        "read-only reconcile must not invoke write adapter"
                    ),
                ),
            ):
                result = pmo.reconcile_readonly("CEN-01")
        self.assertEqual(result["status"], "readback_complete")
        self.assertTrue(result["read_only"])

    def test_changed_remote_body_is_conflict_not_overwritten(self):
        with tempfile.TemporaryDirectory() as td:
            body = Path(td) / "body.md"
            body.write_text("expected")
            spec_row = {"bodypath": str(body), "labels": []}
            ok, reason, digest = pmo.issue_readback_matches(
                {"body": "human edit"}, spec_row, {}
            )
        self.assertFalse(ok)
        self.assertIn("human edit", reason)
        self.assertEqual(len(digest), 64)

    def test_external_blocker_edge_uses_blocked_issue_endpoint_and_blocker_database_id(
        self,
    ):
        spec_row = {"key": "CEN-01", "title": "title", "hard_blockers": ["#9028"]}
        rows = []
        index = {
            "operations": {
                "CEN-01": {
                    "status": "complete_edges_pending",
                    "issue_number": 70,
                    "database_id": 700,
                    "parent_link_status": "verified",
                    "native_blocker_edges": {},
                }
            }
        }
        actual = {"title": "title", "body": "<!-- pmo-9196:CEN-01 -->", "state": "open"}
        api_calls = []

        def api(path, jq=None):
            api_calls.append(path)
            if path.endswith("/issues/70"):
                return actual
            if path.endswith("/issues/9028"):
                return {"id": 902800, "html_url": "https://example/9028"}
            if path.endswith("/issues/70/dependencies/blocked_by"):
                if any("POST" in call for call in write_calls):
                    return [{"id": 902800}]
                return []
            raise AssertionError(path)

        write_calls = []

        def gh(args, timeout=120):
            write_calls.append(args)
            return subprocess.CompletedProcess(args, 0, "{}", "")

        with (
            patch.object(
                pmo, "specs_and_aliases", return_value=(rows, {"CEN-01": spec_row})
            ),
            patch.object(
                pmo, "check", return_value={"apply_ready": True, "problems": []}
            ),
            patch.object(pmo, "read_index", return_value=index),
            patch.object(pmo, "api_get", side_effect=api),
            patch.object(pmo, "gh", side_effect=gh),
            patch.object(pmo, "save_index"),
        ):
            result = pmo.apply_edges_one("CEN-01")
        self.assertEqual(result["status"], "complete")
        self.assertIn(
            "repos/developing-today/code/issues/70/dependencies/blocked_by",
            write_calls[0],
        )
        self.assertIn("issue_id=902800", write_calls[0])
        self.assertNotIn(
            "repos/developing-today/code/issues/9028/dependencies/blocked_by",
            write_calls[0],
        )


class PacketSizeTests(unittest.TestCase):
    def test_every_canonical_packet_fits_queue_ceiling(self):
        import json as _json

        _, canonical = pmo.specs_and_aliases()
        self.assertGreaterEqual(len(canonical), 91)
        for key, spec in sorted(canonical.items()):
            packet = pmo.make_packet(
                spec, 999999, "https://example.invalid/issues/999999"
            )
            raw = _json.dumps(packet).encode()
            self.assertLessEqual(len(raw), 15000, f"{key}: {len(raw)} bytes")
            self.assertEqual(packet["payload"]["requirement_ids"], spec["requirements"])

    def test_all_bodies_fit_github_limit(self):
        _, canonical = pmo.specs_and_aliases()
        for key, spec in sorted(canonical.items()):
            body = (pmo.ISSUES / spec["bodypath"]).read_text()
            self.assertLessEqual(
                len(body),
                65536,
                f"{key}: {len(body)} chars exceeds GitHub issue-body limit",
            )


if __name__ == "__main__":
    unittest.main()
