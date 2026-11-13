#!/usr/bin/env python3
"""Oracle-named tests for the proposal contract and no-write default."""

import contextlib
import importlib.util
import io
import json
import os
import tempfile
import unittest
import subprocess
from pathlib import Path
from datetime import datetime, timezone
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
        "urgency": 450,
        "depends_on": [],
        "body": (
            "## Outcome\nA self-contained credential record contract.\n\n"
            "## Current evidence\nCurrent repo evidence.\n\n"
            "## Technical boundaries\nOwned interface and exclusions.\n\n"
            "## Acceptance\nMeasurable done check.\n\n"
            "## Tests\nPositive and negative tests.\n\n"
            "## Failure modes\nKnown failure response.\n\n"
            "## Safety\nSynthetic only.\n\n"
            "## Rollback\nRevert the synthetic slice.\n\n"
            "## Estimate\nM.\n\n"
            "## Open decisions\nNone beyond the issue text.\n"
        ),
    }
    value.update(overrides)
    return value


def gh_issue(item, number=12345):
    return {
        "number": number,
        "title": item["title"],
        "body": publisher.issue_body(item),
        "url": f"https://github.com/developing-today/code/issues/{number}",
        "labels": [{"name": name} for name in item["labels"]],
        "state": "OPEN",
    }


def empty_receipt_state():
    return publisher.empty_receipt_state()


def table_text(rows):
    columns = list(publisher.QUEUE_COLUMNS)
    widths = [len(column) for column in columns]
    for row in rows:
        values = [f"task {row['id']}", str(row.get("priority", 20)), row["status"],
                  row.get("age", "1m"), row["dedupe_key"], row.get("title", "Task")]
        widths = [max(width, len(value)) for width, value in zip(widths, values)]

    def render(values):
        return "  ".join(value + " " * (width - len(value)) for value, width in zip(values, widths)).rstrip()

    if not rows:
        return "(no tasks)"
    lines = [render(columns), render(["-" * len(column) for column in columns])]
    for row in rows:
        lines.append(render([f"task {row['id']}", str(row.get("priority", 20)), row["status"],
                             row.get("age", "1m"), row["dedupe_key"], row.get("title", "Task")]))
    return "\n".join(lines)


def scans_for_live_row(row):
    scans = {}
    for status in sorted(publisher.LIVE_TASK_STATUSES):
        rows = [row] if row["status"] == status else []
        scans[status] = {"count": len(rows), "ids": [x["id"] for x in rows],
                         "limit": 500, "truncated": False}
        if rows:
            break
    return scans


class PublishIssuesTests(unittest.TestCase):
    def setUp(self):
        self.issue_history_patcher = patch.object(
            publisher, "require_fresh_issue_history",
            return_value={"checked": True, "closed": False, "reopened": False,
                          "closed_then_reopened": False, "event_count": 0},
        )
        self.issue_history_patcher.start()
        self.addCleanup(self.issue_history_patcher.stop)

    def test_common_array_maps_to_existing_priority_scale_and_stable_marker(self):
        items = publisher.validate_manifest([proposal()])
        item = items[0]
        self.assertEqual(item["urgency"], 450)
        self.assertEqual(item["urgency_band"], "high")
        self.assertEqual(item["priority"], 20)
        self.assertEqual(item["priority_label"], "priority/high")
        self.assertEqual(item["queue_kind"], "triage")
        self.assertEqual(publisher.marker(item), "<!-- fleet-ca-v03-proposal:sec-model-001 -->")

    def test_intrinsic_score_bands_and_queue_projection_are_separate(self):
        expected = [(50, "emergency", "priority/critical", 10),
                    (150, "supercritical", "priority/critical", 10),
                    (300, "critical", "priority/critical", 10),
                    (450, "high", "priority/high", 20),
                    (1000, "standard", "priority/normal", 40),
                    (1400, "low", "priority/low", 70),
                    (1800, "someday", "priority/low", 70),
                    (2500, "cold", "priority/low", 70)]
        for score, band, label, queue_priority in expected:
            item = publisher.validate_manifest([proposal(urgency=score)])[0]
            self.assertEqual((item["urgency_band"], item["priority_label"], item["priority"]),
                             (band, label, queue_priority))
            self.assertEqual(item["urgency"], score)

    def test_dependencies_are_topologically_ordered(self):
        blocker = proposal("ARCH-06", parent="#9157")
        blocked = proposal("SEC-09", depends_on=["ARCH-06"])
        result = publisher.validate_manifest([blocked, blocker])
        self.assertEqual([p["id"] for p in result], ["ARCH-06", "SEC-09"])

    def test_unknown_or_cyclic_dependency_rejected_before_writes(self):
        with self.assertRaisesRegex(publisher.PublishError, "unknown proposal IDs"):
            publisher.validate_manifest([proposal(depends_on=["missing-id"])])
        a = proposal("sec-proposal-a", depends_on=["sec-proposal-b"])
        b = proposal("sec-proposal-b", depends_on=["sec-proposal-a"])
        with self.assertRaisesRegex(publisher.PublishError, "cycle"):
            publisher.validate_manifest([a, b])

    def test_duplicate_proposal_ids_rejected(self):
        with self.assertRaisesRegex(publisher.PublishError, "duplicate proposal id"):
            publisher.validate_manifest([proposal(), proposal()])
        with self.assertRaisesRegex(publisher.PublishError, "coordinator issue-kind"):
            publisher.validate_manifest([proposal(kind="root implementation")])
        with self.assertRaisesRegex(publisher.PublishError, "schema mismatch"):
            publisher.validate_manifest([proposal(size="M")])

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
        with self.assertRaises(SystemExit):
            publisher.parse_args(["--manifest", "plan.json", "--queue", "q", "--fleet", "/usr/bin/gh"])
        with self.assertRaises(SystemExit):
            publisher.parse_args(["--manifest", "plan.json", "--queue", "code-shadow"])
        for unsafe_path in ("bad\x00path.json", "bad\ud800path.json"):
            with self.subTest(unsafe_path="non-text path"):
                with self.assertRaises(SystemExit):
                    publisher.parse_args(["--manifest", unsafe_path, "--queue", publisher.QUEUE])

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

    def test_recursive_utf8_and_nul_validation_rejects_whole_batch_before_any_effect(self):
        malformed = [
            proposal("SEC-BAD-TITLE-SURROGATE", title="title-\ud800"),
            proposal("SEC-BAD-TITLE-NUL", title="title-\x00"),
            proposal("SEC-BAD-BODY-NUL", body=proposal()["body"] + "\x00"),
            proposal("SEC-BAD-SOURCE-SURROGATE", source_ranges=["source-\udfff"]),
            {**proposal("SEC-BAD-KEY"), "\ud800": "not printable in diagnostics"},
        ]
        for bad in malformed:
            with self.subTest(id=bad.get("id")):
                with tempfile.TemporaryDirectory() as tmp:
                    manifest = Path(tmp) / "manifest.json"
                    manifest.write_text(json.dumps([proposal(), bad], ensure_ascii=True), encoding="utf-8")
                    stderr = io.StringIO()
                    with patch.object(publisher.subprocess, "run", side_effect=AssertionError("subprocess called")):
                        with patch.object(publisher, "json_output", side_effect=AssertionError("network called")):
                            with patch.object(publisher, "write_temp_json", side_effect=AssertionError("temp write")):
                                with contextlib.redirect_stderr(stderr):
                                    result = publisher.main(["--manifest", str(manifest), "--queue", publisher.QUEUE,
                                                             "--publish", "--receipt", str(Path(tmp) / "receipt.jsonl")])
                    self.assertEqual(result, 2)
                    self.assertEqual({p.name for p in Path(tmp).iterdir()}, {"manifest.json"})
                    self.assertNotIn("\ud800", stderr.getvalue())

    def test_complete_rendered_body_limit_checked_before_any_publish_side_effect(self):
        base = proposal()["body"]
        oversized = proposal(body=base + ("x" * (publisher.MAX_ISSUE_BODY_CHARS - len(base))))
        with self.assertRaisesRegex(publisher.PublishError, "rendered_body exceeds"):
            publisher.validate_manifest([oversized])
        with tempfile.TemporaryDirectory() as tmp:
            manifest = Path(tmp) / "oversized.json"
            manifest.write_text(json.dumps([oversized]), encoding="utf-8")
            with patch.object(publisher.subprocess, "run", side_effect=AssertionError("subprocess called")):
                result = publisher.main(["--manifest", str(manifest), "--queue", "repo-health-code-pilot",
                                         "--publish", "--receipt", str(Path(tmp) / "receipt.jsonl")])
            self.assertEqual(result, 2)

    def test_coordinator_content_contract_required_before_side_effects(self):
        with self.assertRaisesRegex(publisher.PublishError, "lacks coordinator §4.2 sections"):
            publisher.validate_manifest([proposal(body="## Outcome\nOnly one section.\n")])

    def test_receipt_cannot_overwrite_manifest(self):
        with tempfile.TemporaryDirectory() as tmp:
            manifest = Path(tmp) / "proposals.json"
            original = json.dumps([proposal()])
            manifest.write_text(original, encoding="utf-8")
            with patch.object(publisher.subprocess, "run", side_effect=AssertionError("subprocess called")):
                result = publisher.main(["--manifest", str(manifest), "--queue", "repo-health-code-pilot",
                                         "--publish", "--receipt", str(manifest)])
            self.assertEqual(result, 2)
            self.assertEqual(manifest.read_text(encoding="utf-8"), original)

    def test_receipt_path_policy_rejects_tracked_symlink_and_hardlink_targets(self):
        item = publisher.validate_manifest([proposal()])
        digest = publisher.build_plan([proposal()], item, "publish")["proposal_digest"]
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            pubdir = root / "doc" / "design" / "evidence" / "publication"
            pubdir.parent.mkdir(parents=True)
            target = pubdir / f"{digest}.jsonl"
            with patch.object(publisher, "run", return_value=subprocess.CompletedProcess([], 1, "", "untracked")):
                self.assertEqual(publisher.validate_receipt_target(target, digest, repo_root=root,
                                                                   publication_dir=pubdir), target)

            pubdir.mkdir()
            victim = root / "other-source.jsonl"
            victim.write_text("keep", encoding="utf-8")
            target.symlink_to(victim)
            with patch.object(publisher, "run", side_effect=AssertionError("Git check after symlink")):
                with self.assertRaisesRegex(publisher.PublishError, "symlink"):
                    publisher.validate_receipt_target(target, digest, repo_root=root,
                                                      publication_dir=pubdir)
            target.unlink()
            os.link(victim, target)
            with self.assertRaisesRegex(publisher.PublishError, "single-link"):
                publisher.validate_receipt_target(target, digest, repo_root=root,
                                                  publication_dir=pubdir)
            target.unlink()
            with patch.object(publisher, "run", return_value=subprocess.CompletedProcess([], 0, str(target), "")):
                with self.assertRaisesRegex(publisher.PublishError, "tracked"):
                    publisher.validate_receipt_target(target, digest, repo_root=root,
                                                      publication_dir=pubdir)

    def test_publication_directory_symlink_rejected(self):
        item = publisher.validate_manifest([proposal()])
        digest = publisher.build_plan([proposal()], item, "publish")["proposal_digest"]
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            design = root / "doc" / "design"
            design.mkdir(parents=True)
            evidence = design / "evidence"
            evidence.mkdir()
            target_dir = root / "outside"
            target_dir.mkdir()
            pubdir = evidence / "publication"
            pubdir.symlink_to(target_dir, target_is_directory=True)
            with patch.object(publisher, "run", side_effect=AssertionError("Git check after symlink")):
                with self.assertRaisesRegex(publisher.PublishError, "symlink"):
                    publisher.validate_receipt_target(pubdir / f"{digest}.jsonl", digest,
                                                      repo_root=root, publication_dir=pubdir)

    def test_receipt_duplicate_keys_wrong_repo_and_inconsistent_event_order_rejected(self):
        raw = [proposal()]
        item = publisher.validate_manifest(raw)[0]
        plan = publisher.build_plan(raw, [item], "publish")
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "receipt.jsonl"
            publisher.append_receipt(path, {"event": "plan", "repo": publisher.REPO,
                                            "queue": plan["queue"], "proposal_digest": plan["proposal_digest"],
                                            "proposals": plan["proposals"], "started_at": "2026-11-12T00:00:00Z"})
            with path.open("a", encoding="utf-8") as stream:
                stream.write('{"event":"failure","event":"failure","error":"x","at":"t"}\n')
            with self.assertRaisesRegex(publisher.PublishError, "duplicate JSON key"):
                publisher.read_receipt(path, plan, [item])

            path.unlink()
            publisher.append_receipt(path, {"event": "plan", "repo": "wrong/repo",
                                            "queue": plan["queue"], "proposal_digest": plan["proposal_digest"],
                                            "proposals": plan["proposals"], "started_at": "2026-11-12T00:00:00Z"})
            with self.assertRaisesRegex(publisher.PublishError, "does not match"):
                publisher.read_receipt(path, plan, [item])

            path.unlink()
            publisher.append_receipt(path, {"event": "plan", "repo": publisher.REPO,
                                            "queue": plan["queue"], "proposal_digest": plan["proposal_digest"],
                                            "proposals": plan["proposals"], "started_at": "2026-11-12T00:00:00Z"})
            publisher.append_receipt(path, {"event": "issue_verified", "proposal_id": item["id"],
                                            "issue": {}, "at": "2026-11-12T00:00:01Z"})
            with self.assertRaisesRegex(publisher.PublishError, "out-of-order"):
                publisher.read_receipt(path, plan, [item])

    def test_forged_receipt_issue_id_is_not_authority_and_live_mismatch_stops(self):
        raw = [proposal()]
        item = publisher.validate_manifest(raw)[0]
        plan = publisher.build_plan(raw, [item], "publish")
        key = publisher.marker(item)
        issue = gh_issue(item)
        actual_parent_url = f"https://api.github.com/repos/{publisher.REPO}/issues/{item['parent']}"
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "receipt.jsonl"
            publisher.append_receipt(path, {"event": "plan", "repo": publisher.REPO,
                                            "queue": plan["queue"], "proposal_digest": plan["proposal_digest"],
                                            "proposals": plan["proposals"], "started_at": "2026-11-12T00:00:00Z"})
            publisher.append_receipt(path, {"event": "create_intent", "proposal_id": item["id"],
                                            "marker": key, "body_sha256": item["rendered_body_sha256"],
                                            "parent": item["parent"], "title": item["title"], "at": "t"})
            publisher.append_receipt(path, {"event": "create_result", "proposal_id": item["id"],
                                            "number": 12345, "url": issue["url"],
                                            "body_sha256": item["rendered_body_sha256"],
                                            "source": "create_response", "at": "t"})
            forged_issue = {"number": 12345, "url": issue["url"], "database_id": 999999,
                            "marker": key, "body_sha256": item["rendered_body_sha256"],
                            "parent": item["parent"], "parent_issue_url": actual_parent_url,
                            "blocked_by": [], "labels": sorted(item["labels"]),
                            "lifecycle_history": {"checked": True, "closed": False, "reopened": False,
                                                  "closed_then_reopened": False, "event_count": 0}}
            publisher.append_receipt(path, {"event": "issue_verified", "proposal_id": item["id"],
                                            "issue": forged_issue, "at": "t"})
            state = publisher.read_receipt(path, plan, [item])
            self.assertNotIn("published", state)
            def api(fleet, method, endpoint, *fields):
                if endpoint == f"repos/{publisher.REPO}/issues/12345":
                    return {"id": 7001, "created_at": "2026-11-12T18:00:00Z",
                            "parent_issue_url": actual_parent_url}
                if endpoint.endswith("/sub_issues?per_page=100"):
                    return [{"number": 12345}]
                if endpoint.endswith("/dependencies/blocked_by"):
                    return []
                raise AssertionError((method, endpoint, fields))
            with patch.object(publisher, "issue_by_marker", return_value=issue):
                with patch.object(publisher, "require_epic_open"):
                    with patch.object(publisher, "api_json", side_effect=api):
                        with patch.object(publisher, "checked", side_effect=AssertionError("must not enqueue")):
                            with self.assertRaisesRegex(publisher.PublishError, "differs from its receipt"):
                                publisher.publish_one(item, "fleet", "repo-queue", publisher.QUEUE,
                                                      path, state, {})

    def test_duplicate_json_object_keys_rejected_before_subprocess(self):
        with tempfile.TemporaryDirectory() as tmp:
            manifest = Path(tmp) / "duplicate.json"
            manifest.write_text('[{"id":"sec-model-001","id":"sec-model-002"}]', encoding="utf-8")
            with patch.object(publisher.subprocess, "run", side_effect=AssertionError("subprocess called")):
                result = publisher.main(["--manifest", str(manifest), "--queue", "repo-health-code-pilot"])
            self.assertEqual(result, 2)

    def test_queue_packet_uses_only_supported_repo_task_kind(self):
        item = publisher.validate_manifest([proposal()])[0]
        packet = publisher.queue_packet(item, "repo-health-code-pilot", {
            "number": 12345, "title": item["title"], "url": "https://github.com/developing-today/code/issues/12345",
            "created_at": "2026-11-12T18:00:00Z",
        })
        self.assertEqual(packet["payload"]["schema"], "repo-task/v1")
        self.assertEqual(packet["payload"]["kind"], "triage")
        self.assertEqual(packet["priority"], 20)
        self.assertEqual(packet["payload"]["timebox_minutes"], 20)
        self.assertEqual(packet["payload"]["max_attempts"], 6)
        self.assertEqual(packet["dedupe_key"], "repo-health:v1:triage:developing-today/code:issue:12345")
        self.assertEqual(packet, publisher.queue_packet(item, "repo-health-code-pilot", {
            "number": 12345, "title": item["title"], "url": "https://github.com/developing-today/code/issues/12345",
            "created_at": "2026-11-12T18:00:00Z",
        }))

    def test_explicit_rate_limit_refusal_retries_and_unknown_create_failure_does_not(self):
        first = subprocess.CompletedProcess([], 1, "", "HTTP 429: API rate limit exceeded; Retry-After: 3")
        second = subprocess.CompletedProcess([], 0, "ok", "")
        with patch.object(publisher, "require_epic_open") as gate:
            with patch.object(publisher, "run", side_effect=[first, second]) as run:
                with patch.object(publisher.time, "sleep") as sleep:
                    self.assertEqual(publisher.checked(["fleet", "gh", "issue", "create"]), "ok")
        self.assertEqual(run.call_count, 2)
        self.assertEqual(gate.call_count, 2)
        sleep.assert_called_once_with(3)

        ambiguous = subprocess.CompletedProcess([], 1, "", "connection reset after request")
        with patch.object(publisher, "require_epic_open"):
            with patch.object(publisher, "run", return_value=ambiguous) as run:
                with self.assertRaisesRegex(publisher.PublishError, "command returned"):
                    publisher.checked(["fleet", "gh", "issue", "create"])
        run.assert_called_once()

    def test_fleet_gh_binary_override_is_not_forwarded(self):
        result = subprocess.CompletedProcess([], 0, "ok", "")
        with patch.dict(os.environ, {"FLEET_GH_BIN": "/usr/bin/gh"}):
            with patch.object(publisher.subprocess, "run", return_value=result) as run:
                publisher.run(["fleet", "gh", "issue", "list"])
        call = run.call_args.kwargs
        self.assertNotIn("FLEET_GH_BIN", call["env"])
        self.assertEqual(run.call_args.args[0][0], "fleet")

    def test_status_scan_fallback_survives_ignored_task_filter_probe(self):
        baseline = [
            {"id": 1111, "status": "done", "dedupe_key": "repo-health:v1:fix:developing-today/code:issue:948"},
            {"id": 1254, "status": "done", "dedupe_key": "repo-health:v1:triage:developing-today/code:issue:241"},
        ]
        by_status = {
            "blocked": [{"id": 15117, "status": "blocked", "dedupe_key": "repo-health:v1:fix:developing-today/code:issue:8900"}],
            "claimed": [{"id": 9341, "status": "claimed", "dedupe_key": "repo-health:v1:fix:developing-today/code:issue:788"}],
            "parked": [{"id": 2955, "status": "parked", "dedupe_key": "repo-health:v1:fix:developing-today/code:issue:171"}],
            "queued": [{"id": 15360, "status": "queued", "dedupe_key": "repo-health:v1:fix:developing-today/code:issue:9188"}],
        }
        replies = [table_text(baseline), table_text([])]
        replies += [table_text(by_status[s]) for s in sorted(publisher.LIVE_TASK_STATUSES)]
        replies += [table_text(baseline)]
        with patch.object(publisher, "checked", side_effect=replies) as checked:
            result = publisher.verify_queue_list_filters("repo-queue", "repo-health-code-pilot")
        self.assertEqual(result["task_filter_probe"], "nonmatching-rows-returned")
        self.assertEqual(result["queue_filter_probe"], "empty")
        self.assertEqual(set(result["status_scan_ids"]), publisher.LIVE_TASK_STATUSES)
        self.assertEqual(checked.call_count, 7)
        self.assertTrue(any("--task" in call.args[0] for call in checked.call_args_list))

    def test_wrong_status_filter_response_fails_preflight(self):
        done_rows = [{"id": 1111, "status": "done", "dedupe_key": "repo-health:v1:fix:developing-today/code:issue:948"}]
        with patch.object(publisher, "checked", side_effect=[table_text(done_rows), table_text([]), table_text(done_rows)]):
            with self.assertRaisesRegex(publisher.PublishError, "status blocked filter"):
                publisher.verify_queue_list_filters("repo-queue", "repo-health-code-pilot")

    def test_task_moved_from_queued_to_claimed_is_found_by_exact_live_row(self):
        key = "repo-health:v1:triage:developing-today/code:issue:321"
        target = {"id": 77, "priority": 20, "status": "claimed", "age": "1m",
                  "dedupe_key": key, "title": "Triage"}
        with patch.object(publisher, "checked", side_effect=[table_text([]), table_text([target])]) as checked:
            row, scans = publisher.find_queue_task("repo-queue", publisher.QUEUE, 77, key)
        self.assertEqual(row, target)
        self.assertEqual(scans["blocked"]["count"], 0)
        self.assertEqual(checked.call_count, 2)

    def test_queue_id_and_dedupe_key_must_match_the_same_row(self):
        key = "repo-health:v1:triage:developing-today/code:issue:321"
        unrelated = {"id": 78, "priority": 20, "status": "claimed", "age": "1m",
                     "dedupe_key": f"prefix-{key}-suffix", "title": "Other ID"}
        replies = [table_text([]), table_text([unrelated]), table_text([]), table_text([])]
        with patch.object(publisher, "checked", side_effect=replies):
            row, _ = publisher.find_queue_task("repo-queue", publisher.QUEUE, 77, key)
        self.assertIsNone(row)

        exact_key_wrong_id = {**unrelated, "dedupe_key": key}
        with patch.object(publisher, "checked", side_effect=[table_text([]), table_text([exact_key_wrong_id])]):
            with self.assertRaisesRegex(publisher.PublishError, "different task ID"):
                publisher.find_queue_task("repo-queue", publisher.QUEUE, 77, key)

        wrong_key = {**unrelated, "id": 77, "dedupe_key": "repo-health:v1:fix:developing-today/code:issue:9"}
        with patch.object(publisher, "checked", side_effect=[table_text([]), table_text([wrong_key])]):
            with self.assertRaisesRegex(publisher.PublishError, "same-row dedupe/status"):
                publisher.find_queue_task("repo-queue", publisher.QUEUE, 77, key)

    def test_truncated_bounded_scan_is_indeterminate_not_absence(self):
        rows = [{"id": n, "priority": 40, "status": "queued", "age": "1m",
                 "dedupe_key": f"repo-health:v1:triage:developing-today/code:issue:{n}",
                 "title": f"Task {n}"} for n in range(1, 501)]
        with patch.object(publisher, "checked", side_effect=[table_text([]), table_text([]),
                                                                table_text([]), table_text(rows)]):
            found, scans = publisher.find_queue_task("repo-queue", publisher.QUEUE, 999, "unmatched")
        self.assertIsNone(found)
        self.assertEqual(scans["queued"]["count"], 500)
        self.assertTrue(scans["queued"]["truncated"])

    def test_matching_row_at_scan_cap_is_not_accepted_but_other_complete_status_can_prove(self):
        key = "repo-health:v1:triage:developing-today/code:issue:999"
        capped = [{"id": n, "priority": 40, "status": "queued", "age": "1m",
                   "dedupe_key": key if n == 250 else f"other:{n}", "title": f"Task {n}"}
                  for n in range(1, 501)]
        empty_replies = [table_text([]), table_text([]), table_text([])]
        with patch.object(publisher, "checked", side_effect=empty_replies + [table_text(capped)]):
            with self.assertRaisesRegex(publisher.QueueReadbackError, "capped/truncated"):
                publisher.find_queue_task("repo-queue", publisher.QUEUE, 250, key)

        blocked_capped = [{"id": n, "priority": 40, "status": "blocked", "age": "1m",
                           "dedupe_key": f"other:{n}", "title": f"Task {n}"}
                          for n in range(1001, 1501)]
        claimed = {"id": 250, "priority": 20, "status": "claimed", "age": "1m",
                   "dedupe_key": key, "title": "Task 250"}
        with patch.object(publisher, "checked", side_effect=[table_text(blocked_capped), table_text([claimed])]):
            found, scans = publisher.find_queue_task("repo-queue", publisher.QUEUE, 250, key)
        self.assertEqual(found, claimed)
        self.assertTrue(scans["blocked"]["truncated"])
        self.assertEqual(scans["claimed"]["count"], 1)

    def test_main_stops_before_issue_write_when_status_filters_are_untrusted(self):
        item = proposal()
        done_rows = [{"id": 1111, "priority": 10, "status": "done", "age": "1d",
                      "dedupe_key": "repo-health:v1:fix:developing-today/code:issue:948", "title": "Done"}]
        with tempfile.TemporaryDirectory() as tmp:
            manifest = Path(tmp) / "proposals.json"
            receipt = Path(tmp) / "receipt.jsonl"
            manifest.write_text(json.dumps([item]), encoding="utf-8")
            labels = [{"name": label} for label in publisher.validate_manifest([item])[0]["labels"]]
            with patch.object(publisher, "validate_receipt_target", return_value=receipt):
                with patch.object(publisher, "json_output", return_value=labels):
                    with patch.object(publisher, "checked", side_effect=["packet valid", table_text(done_rows),
                                                                           table_text([]), table_text(done_rows)]) as checked:
                        result = publisher.main(["--manifest", str(manifest), "--queue", publisher.QUEUE,
                                                 "--publish", "--receipt", str(receipt)])
            self.assertEqual(result, 3)
            self.assertEqual(checked.call_count, 4)
            self.assertFalse(receipt.exists())

    def test_duplicate_marker_search_fails_closed(self):
        item = publisher.validate_manifest([proposal()])[0]
        duplicate = gh_issue(item)
        with patch.object(publisher, "json_output", return_value=[duplicate, duplicate]):
            with self.assertRaisesRegex(publisher.PublishError, "multiple issues"):
                publisher.issue_by_marker("fleet", publisher.marker(item))

    def test_issue_history_scan_is_paginated_and_detects_close_then_reopen(self):
        first_page = [{"event": "commented", "created_at": "2026-11-01T00:00:00Z"}
                      for _ in range(99)] + [{"event": "closed", "created_at": "2026-11-02T00:00:00Z"}]
        second_page = [{"event": "reopened", "created_at": "2026-11-03T00:00:00Z"}]
        with patch.object(publisher, "api_json", side_effect=[first_page, second_page]) as api:
            history = publisher.issue_close_reopen_history("fleet", 12345)
        self.assertEqual(history["event_count"], 101)
        self.assertTrue(history["closed"])
        self.assertTrue(history["reopened"])
        self.assertTrue(history["closed_then_reopened"])
        self.assertIn("page=2", api.call_args_list[1].args[2])

    def test_currently_open_reused_child_with_close_reopen_history_is_conflict(self):
        item = publisher.validate_manifest([proposal()])[0]
        issue = gh_issue(item)
        with tempfile.TemporaryDirectory() as tmp:
            with patch.object(publisher, "issue_by_marker", return_value=issue):
                with patch.object(publisher, "require_epic_open"):
                    with patch.object(publisher, "require_fresh_issue_history", side_effect=publisher.PublishError(
                            "issue #12345 was previously closed then reopened; refusing to reuse its marker")):
                        with patch.object(publisher, "checked", side_effect=AssertionError("must not mutate")):
                            with self.assertRaisesRegex(publisher.PublishError, "previously closed then reopened"):
                                publisher.publish_one(item, publisher.FLEET, publisher.REPO_QUEUE,
                                                      publisher.QUEUE, Path(tmp) / "receipt.jsonl",
                                                      empty_receipt_state(), {})

    def test_marker_body_mismatch_is_conflict_without_write(self):
        item = publisher.validate_manifest([proposal()])[0]
        edited = gh_issue(item)
        edited["body"] += "\nuser edit"
        with tempfile.TemporaryDirectory() as tmp:
            with patch.object(publisher, "issue_by_marker", return_value=edited):
                with patch.object(publisher, "require_epic_open"):
                    with patch.object(publisher, "checked", side_effect=AssertionError("must not write")):
                        with self.assertRaisesRegex(publisher.PublishError, "body/title differs"):
                            publisher.publish_one(item, "fleet", "repo-queue", "pilot",
                                                  Path(tmp) / "receipt.jsonl", empty_receipt_state(), {})

    def test_ambiguous_create_recovers_by_marker_without_second_create(self):
        item = publisher.validate_manifest([proposal()])[0]
        issue = gh_issue(item)
        def api(fleet, method, endpoint, *fields):
            if endpoint == f"repos/{publisher.REPO}/issues/12345":
                return {"id": 7001, "created_at": "2026-11-12T18:00:00Z",
                        "parent_issue_url": f"https://api.github.com/repos/{publisher.REPO}/issues/9158"}
            if endpoint.endswith("/sub_issues?per_page=100"):
                return [{"number": 12345}]
            if endpoint.endswith("/dependencies/blocked_by"):
                return []
            raise AssertionError((method, endpoint, fields))

        key = "repo-health:v1:triage:developing-today/code:issue:12345"
        command_results = [publisher.PublishError("connection reset after issue create"), "packet valid",
                           f"enqueued new task 55\n{json.dumps({'id': 55, 'dedupe_key': key, 'queue': 'pilot', 'priority': 20})}"]
        with tempfile.TemporaryDirectory() as tmp:
            checked = Path(tmp) / "receipt.jsonl"
            state = empty_receipt_state()
            live_row = {"id": 55, "priority": 20, "status": "queued", "age": "1m",
                        "dedupe_key": key, "title": "Triage"}
            with patch.object(publisher, "issue_by_marker", side_effect=[None, issue]):
                with patch.object(publisher, "require_epic_open"):
                    with patch.object(publisher, "api_json", side_effect=api):
                        with patch.object(publisher, "checked", side_effect=command_results) as command:
                            with patch.object(publisher, "find_queue_task", return_value=(live_row, scans_for_live_row(live_row))):
                                with self.assertRaisesRegex(publisher.PublishError, "connection reset"):
                                    publisher.publish_one(item, "fleet", "repo-queue", "pilot", checked, state, {})
                                publisher.publish_one(item, "fleet", "repo-queue", "pilot", checked, state, {})
            self.assertEqual(command.call_count, 3)
            self.assertEqual(command.call_args_list[0].args[0][:5], [publisher.FLEET, "gh", "api", "--method", "POST"])
            self.assertNotIn("--parent", command.call_args_list[0].args[0])
            self.assertNotIn("--blocked-by", command.call_args_list[0].args[0])
            self.assertEqual(command.call_args_list[1].args[0][:2], ["repo-queue", "validate"])
            records = [json.loads(line) for line in checked.read_text().splitlines()]
            self.assertEqual([record["event"] for record in records],
                             ["create_intent", "create_result", "issue_verified",
                              "queue_intent", "queue_ack", "task_verified"])

    def test_ambiguous_create_plus_zero_search_stays_indeterminate_without_second_post(self):
        raw = [proposal()]
        item = publisher.validate_manifest(raw)[0]
        plan = publisher.build_plan(raw, [item], "publish")
        key = publisher.marker(item)
        with tempfile.TemporaryDirectory() as tmp:
            receipt = Path(tmp) / "receipt.jsonl"
            publisher.append_receipt(receipt, {
                "event": "plan", "repo": publisher.REPO, "queue": publisher.QUEUE,
                "proposal_digest": plan["proposal_digest"], "proposals": plan["proposals"],
                "started_at": "2026-11-12T00:00:00Z",
            })
            state = publisher.read_receipt(receipt, plan, [item])
            with patch.object(publisher, "issue_by_marker", side_effect=[None, None]) as search:
                with patch.object(publisher, "require_epic_open"):
                    with patch.object(publisher, "api_json", return_value=[]):
                        with patch.object(publisher, "checked", side_effect=publisher.PublishError(
                                "connection reset after create committed")) as command:
                            with self.assertRaisesRegex(publisher.PublishError, "connection reset"):
                                publisher.publish_one(item, "fleet", "repo-queue", publisher.QUEUE,
                                                      receipt, state, {})
                            state = publisher.read_receipt(receipt, plan, [item])
                            self.assertEqual(state["create_intents"][item["id"]]["marker"], key)
                            with self.assertRaisesRegex(publisher.PublishError, "unresolved create_intent"):
                                publisher.publish_one(item, "fleet", "repo-queue", publisher.QUEUE,
                                                      receipt, state, {})
            self.assertEqual(search.call_count, 2)
            command.assert_called_once()  # no second issue-create after zero marker search

    def test_parent_link_response_loss_resumes_by_live_edge_without_second_create(self):
        raw = [proposal()]
        item = publisher.validate_manifest(raw)[0]
        plan = publisher.build_plan(raw, [item], "publish")
        issue = gh_issue(item)
        parent_url = f"https://api.github.com/repos/{publisher.REPO}/issues/{item['parent']}"
        stateful = {"parent_linked": False, "child_metadata_reads": 0}
        api_events = []

        def api(fleet, method, endpoint, *fields):
            api_events.append((method, endpoint, fields))
            if endpoint == f"repos/{publisher.REPO}/issues/12345":
                stateful["child_metadata_reads"] += 1
                return {"id": 7001, "created_at": "2026-11-12T18:00:00Z",
                        "parent_issue_url": parent_url if stateful["parent_linked"] else None}
            if endpoint.endswith("/sub_issues?per_page=100"):
                return [{"number": 12345}] if stateful["parent_linked"] else []
            if endpoint.endswith("/sub_issues") and method == "POST":
                if not stateful["parent_linked"]:
                    stateful["parent_linked"] = True  # remote edge committed; response is lost
                    raise publisher.PublishError("parent-link response lost after commit")
                return {}
            if endpoint.endswith("/dependencies/blocked_by"):
                return []
            raise AssertionError((method, endpoint, fields))

        key = "repo-health:v1:triage:developing-today/code:issue:12345"
        create_payloads = []
        create_calls = []

        def checked(argv):
            if argv[:3] == [publisher.FLEET, "gh", "api"] and argv[3:5] == ["--method", "POST"]:
                create_calls.append(argv)
                self.assertEqual(argv[5], f"repos/{publisher.REPO}/issues")
                self.assertNotIn("--parent", argv)
                self.assertNotIn("--blocked-by", argv)
                with Path(argv[-1]).open(encoding="utf-8") as stream:
                    create_payloads.append(json.load(stream))
                return json.dumps({"number": 12345, "html_url": issue["url"]})
            if argv[:3] == [publisher.FLEET, "gh", "issue"] and argv[3] == "view":
                return json.dumps(issue)
            if argv[:2] == [publisher.REPO_QUEUE, "validate"]:
                return "packet valid"
            if argv[:2] == [publisher.REPO_QUEUE, "enqueue"]:
                return f"enqueued task 55\n{json.dumps({'id': 55, 'dedupe_key': key, 'queue': publisher.QUEUE, 'priority': 20})}"
            raise AssertionError(argv)

        live_row = {"id": 55, "priority": 20, "status": "queued", "age": "1m",
                    "dedupe_key": key, "title": "Triage"}
        with tempfile.TemporaryDirectory() as tmp:
            receipt = Path(tmp) / "receipt.jsonl"
            publisher.append_receipt(receipt, {"event": "plan", "repo": publisher.REPO,
                                               "queue": plan["queue"], "proposal_digest": plan["proposal_digest"],
                                               "proposals": plan["proposals"], "started_at": "t"})
            state = publisher.read_receipt(receipt, plan, [item])
            marker_issues = [None, issue]
            with patch.object(publisher, "issue_by_marker", side_effect=marker_issues):
                with patch.object(publisher, "require_epic_open"):
                    with patch.object(publisher, "api_json", side_effect=api):
                        with patch.object(publisher, "checked", side_effect=checked):
                            with patch.object(publisher, "find_queue_task", return_value=(live_row, scans_for_live_row(live_row))):
                                with self.assertRaisesRegex(publisher.PublishError, "response lost after commit"):
                                    publisher.publish_one(item, publisher.FLEET, publisher.REPO_QUEUE,
                                                          publisher.QUEUE, receipt, state, {})
                                state = publisher.read_receipt(receipt, plan, [item])
                                publisher.publish_one(item, publisher.FLEET, publisher.REPO_QUEUE,
                                                      publisher.QUEUE, receipt, state, {})
            self.assertEqual(len(create_calls), 1)
            self.assertEqual(create_payloads, [{"title": item["title"], "body": item["rendered_body"],
                                                "labels": item["labels"]}])
            parent_posts = [event for event in api_events if event[0] == "POST" and event[1].endswith("/sub_issues")]
            self.assertEqual(len(parent_posts), 1)
            records = [json.loads(line) for line in receipt.read_text().splitlines()]
            self.assertEqual([record["event"] for record in records],
                             ["plan", "create_intent", "create_result", "issue_verified",
                              "queue_intent", "queue_ack", "task_verified"])

    def test_epic_closed_prevents_batch_receipt_and_issue_mutations(self):
        item = proposal()
        with tempfile.TemporaryDirectory() as tmp:
            manifest = Path(tmp) / "proposals.json"
            receipt = Path(tmp) / "receipt.jsonl"
            manifest.write_text(json.dumps([item]), encoding="utf-8")
            label_rows = [{"name": name} for name in publisher.validate_manifest([item])[0]["labels"]]
            with patch.object(publisher, "validate_receipt_target", return_value=receipt):
                with patch.object(publisher, "json_output", side_effect=[label_rows, {"number": 9155, "state": "CLOSED"}]):
                    with patch.object(publisher, "checked", return_value="validated") as checked:
                        with patch.object(publisher, "verify_queue_list_filters", return_value={"task_filter_probe": "nonmatching-rows-returned"}):
                            result = publisher.main(["--manifest", str(manifest), "--queue", "repo-health-code-pilot",
                                                     "--publish", "--receipt", str(receipt)])
            self.assertEqual(result, 3)
            self.assertEqual(checked.call_count, 1)  # queue schema validation only
            self.assertFalse(receipt.exists())

    def test_create_rechecks_epic_and_stops_if_closed(self):
        item = publisher.validate_manifest([proposal()])[0]
        with tempfile.TemporaryDirectory() as tmp:
            with patch.object(publisher, "issue_by_marker", return_value=None):
                with patch.object(publisher, "require_epic_open", side_effect=publisher.PublishError("closed")):
                    with patch.object(publisher, "checked", side_effect=AssertionError("issue create attempted")):
                        with self.assertRaisesRegex(publisher.PublishError, "closed"):
                            publisher.publish_one(item, "fleet", "repo-queue", "pilot",
                                                  Path(tmp) / "receipt.jsonl", empty_receipt_state(), {})

    def test_open_program_epic_is_not_subject_to_child_marker_reuse_history_gate(self):
        with patch.object(publisher, "json_output", return_value={"number": 9155, "state": "OPEN"}):
            with patch.object(publisher, "issue_close_reopen_history", side_effect=AssertionError("epic history gate")):
                publisher.require_epic_open(publisher.FLEET)

    def test_epic_closing_after_create_blocks_followup_queue_mutation(self):
        item = publisher.validate_manifest([proposal()])[0]
        issue = gh_issue(item)
        api_events = []

        def api(fleet, method, endpoint, *fields):
            api_events.append((method, endpoint, fields))
            if endpoint == f"repos/{publisher.REPO}/issues/12345":
                return {"id": 7001, "created_at": "2026-11-12T18:00:00Z",
                        "parent_issue_url": f"https://api.github.com/repos/{publisher.REPO}/issues/{item['parent']}"}
            if endpoint.endswith("/sub_issues?per_page=100"):
                return [{"number": 12345}]
            if endpoint.endswith("/dependencies/blocked_by"):
                return []
            raise AssertionError((method, endpoint, fields))

        epic_states = [{"number": 9155, "state": "OPEN"},
                       {"number": 9155, "state": "OPEN"},
                       {"number": 9155, "state": "OPEN"}, issue,
                       {"number": 9155, "state": "CLOSED"}]
        url = "https://github.com/developing-today/code/issues/12345"
        captured_payloads = []
        def fake_run(argv, **kwargs):
            if argv[:3] == [publisher.FLEET, "gh", "api"]:
                with Path(argv[-1]).open(encoding="utf-8") as stream:
                    captured_payloads.append(json.load(stream))
                return subprocess.CompletedProcess(argv, 0, json.dumps({"number": 12345, "html_url": url}), "")
            return subprocess.CompletedProcess(argv, 0, "packet valid", "")
        with tempfile.TemporaryDirectory() as tmp:
            receipt = Path(tmp) / "receipt.jsonl"
            with patch.object(publisher, "issue_by_marker", return_value=None):
                with patch.object(publisher, "json_output", side_effect=epic_states):
                    with patch.object(publisher, "api_json", side_effect=api):
                        with patch.object(publisher, "run", side_effect=fake_run) as run:
                            with self.assertRaisesRegex(publisher.PublishError, "#9155 is not OPEN"):
                                publisher.publish_one(item, publisher.FLEET, publisher.REPO_QUEUE,
                                                      publisher.QUEUE, receipt, empty_receipt_state(), {})
            self.assertEqual(run.call_count, 2)  # create + read-only packet validation; no enqueue POST
            create_argv = run.call_args_list[0].args[0]
            self.assertEqual(create_argv[:5], [publisher.FLEET, "gh", "api", "--method", "POST"])
            self.assertEqual(create_argv[5], f"repos/{publisher.REPO}/issues")
            self.assertNotIn("--parent", create_argv)
            self.assertNotIn("--blocked-by", create_argv)
            self.assertEqual(set(captured_payloads[0]), {"title", "body", "labels"})
            self.assertEqual(run.call_args_list[1].args[0][:2], [publisher.REPO_QUEUE, "validate"])
            self.assertTrue(all("enqueue" not in call.args[0] for call in run.call_args_list))
            events = [json.loads(line)["event"] for line in receipt.read_text().splitlines()]
            self.assertEqual(events, ["create_intent", "create_result", "issue_verified", "queue_intent"])

    def test_full_parent_stops_before_create_without_rehoming(self):
        item = publisher.validate_manifest([proposal()])[0]
        with tempfile.TemporaryDirectory() as tmp:
            with patch.object(publisher, "issue_by_marker", return_value=None):
                with patch.object(publisher, "require_epic_open"):
                    with patch.object(publisher, "api_json", return_value=[{"number": n} for n in range(1, 101)]):
                        with patch.object(publisher, "checked", side_effect=AssertionError("issue create attempted")):
                            with self.assertRaisesRegex(publisher.PublishError, "100 direct sub-issue cap"):
                                publisher.publish_one(item, "fleet", "repo-queue", "pilot",
                                                      Path(tmp) / "receipt.jsonl", empty_receipt_state(), {})

    def test_native_parent_uses_database_id_and_live_queue_readback(self):
        item = publisher.validate_manifest([proposal()])[0]
        issue = gh_issue(item)
        with tempfile.TemporaryDirectory() as tmp:
            events = []

            def api(fleet, method, endpoint, *fields):
                events.append((method, endpoint, fields))
                if endpoint == f"repos/{publisher.REPO}/issues/12345":
                    parent_url = (f"https://api.github.com/repos/{publisher.REPO}/issues/9158"
                                  if any(e[0] == "POST" and e[1].endswith("/sub_issues") for e in events)
                                  else None)
                    return {"id": 7001, "created_at": "2026-11-12T18:00:00Z",
                            "parent_issue_url": parent_url}
                if method == "POST" and endpoint.endswith("/sub_issues"):
                    return {}
                if endpoint.endswith("/sub_issues?per_page=100"):
                    return [] if not any(e[0] == "POST" and e[1].endswith("/sub_issues") for e in events) else [{"number": 12345}]
                if endpoint.endswith("/dependencies/blocked_by"):
                    return []
                raise AssertionError((method, endpoint, fields))

            key = "repo-health:v1:triage:developing-today/code:issue:12345"
            checked = ["packet valid", f"enqueued new task 55\n{json.dumps({'id': 55, 'dedupe_key': key, 'queue': 'pilot', 'priority': 20})}"]
            live_row = {"id": 55, "priority": 20, "status": "queued", "age": "1m",
                        "dedupe_key": key, "title": "Triage"}
            with patch.object(publisher, "issue_by_marker", return_value=issue):
                with patch.object(publisher, "require_epic_open"):
                    with patch.object(publisher, "api_json", side_effect=api):
                        with patch.object(publisher, "checked", side_effect=checked):
                            receipt = Path(tmp) / "receipt.jsonl"
                            published = {}
                            with patch.object(publisher, "find_queue_task", return_value=(live_row, scans_for_live_row(live_row))):
                                state = empty_receipt_state()
                                publisher.publish_one(item, "fleet", "repo-queue", "pilot", receipt, state, published)
            post = [e for e in events if e[0] == "POST"]
            self.assertEqual(post, [("POST", f"repos/{publisher.REPO}/issues/9158/sub_issues", ("sub_issue_id=7001",))])
            self.assertEqual(published[item["id"]]["number"], 12345)
            records = [json.loads(line) for line in receipt.read_text().splitlines()]
            self.assertEqual([record["event"] for record in records],
                             ["create_result", "issue_verified", "queue_intent", "queue_ack", "task_verified"])

    def test_missing_live_queue_row_is_recorded_unverified_without_enqueue_retry(self):
        raw = [proposal()]
        item = publisher.validate_manifest(raw)[0]
        plan = publisher.build_plan(raw, [item], "publish")
        issue = gh_issue(item)
        with tempfile.TemporaryDirectory() as tmp:
            def api(fleet, method, endpoint, *fields):
                if endpoint == f"repos/{publisher.REPO}/issues/12345":
                    return {"id": 7001, "created_at": "2026-11-12T18:00:00Z",
                            "parent_issue_url": f"https://api.github.com/repos/{publisher.REPO}/issues/9158"}
                if endpoint.endswith("/sub_issues?per_page=100"):
                    return [{"number": 12345}]
                if endpoint.endswith("/dependencies/blocked_by"):
                    return []
                raise AssertionError((method, endpoint))

            key = "repo-health:v1:triage:developing-today/code:issue:12345"
            command_results = ["valid", f"enqueued task 55\n{json.dumps({'id': 55, 'dedupe_key': key, 'queue': publisher.QUEUE, 'priority': 20})}", "valid"]
            scans = {s: {"count": 0, "ids": [], "limit": 500, "truncated": False}
                     for s in publisher.LIVE_TASK_STATUSES}
            receipt = Path(tmp) / "receipt.jsonl"
            publisher.append_receipt(receipt, {"event": "plan", "repo": publisher.REPO,
                                               "queue": plan["queue"], "proposal_digest": plan["proposal_digest"],
                                               "proposals": plan["proposals"], "started_at": "2026-11-12T00:00:00Z"})
            state = publisher.read_receipt(receipt, plan, [item])
            with patch.object(publisher, "issue_by_marker", return_value=issue):
                with patch.object(publisher, "require_epic_open"):
                    with patch.object(publisher, "api_json", side_effect=api):
                        with patch.object(publisher, "checked", side_effect=command_results) as command:
                            with patch.object(publisher, "find_queue_task", side_effect=[(None, scans), (None, scans)]) as find:
                                with self.assertRaisesRegex(publisher.PublishError, "supervisor reconciliation"):
                                    publisher.publish_one(item, "fleet", "repo-queue", publisher.QUEUE, receipt,
                                                          state, {})
                                find.assert_called_once()
                                state = publisher.read_receipt(receipt, plan, [item])
                                with self.assertRaisesRegex(publisher.PublishError, "supervisor reconciliation"):
                                    publisher.publish_one(item, "fleet", "repo-queue", publisher.QUEUE, receipt,
                                                          state, {})
                                self.assertEqual(command.call_count, 3)
                                self.assertEqual(command.call_args_list[2].args[0][:2], ["repo-queue", "validate"])
            records = [json.loads(line) for line in receipt.read_text().splitlines()]
            self.assertEqual([record["event"] for record in records],
                             ["plan", "create_result", "issue_verified", "queue_intent", "queue_ack",
                              "queue_unverified", "queue_unverified"])
            recovered_state = publisher.read_receipt(receipt, plan, [item])
            self.assertEqual(len(recovered_state["queue_unverified"]), 2)

    def test_queue_intent_without_ack_resumes_read_only_and_never_reenqueues(self):
        raw = [proposal()]
        item = publisher.validate_manifest(raw)[0]
        plan = publisher.build_plan(raw, [item], "publish")
        issue = gh_issue(item)
        key = "repo-health:v1:triage:developing-today/code:issue:12345"
        issue_record = {"number": 12345, "url": issue["url"], "database_id": 7001,
                        "marker": publisher.marker(item), "body_sha256": item["rendered_body_sha256"],
                        "parent": item["parent"],
                        "parent_issue_url": f"https://api.github.com/repos/{publisher.REPO}/issues/{item['parent']}",
                        "blocked_by": [], "labels": sorted(item["labels"]),
                        "lifecycle_history": {"checked": True, "closed": False, "reopened": False,
                                              "closed_then_reopened": False, "event_count": 0}}
        with tempfile.TemporaryDirectory() as tmp:
            receipt = Path(tmp) / "receipt.jsonl"
            publisher.append_receipt(receipt, {"event": "plan", "repo": publisher.REPO,
                                                "queue": plan["queue"], "proposal_digest": plan["proposal_digest"],
                                                "proposals": plan["proposals"], "started_at": "t"})
            publisher.append_receipt(receipt, {"event": "create_result", "proposal_id": item["id"],
                                                "number": 12345, "url": issue["url"],
                                                "body_sha256": item["rendered_body_sha256"],
                                                "source": "marker_search", "at": "t"})
            publisher.append_receipt(receipt, {"event": "issue_verified", "proposal_id": item["id"],
                                                "issue": issue_record, "at": "t"})
            publisher.append_receipt(receipt, {"event": "queue_intent", "proposal_id": item["id"],
                                                "queue": publisher.QUEUE, "dedupe_key": key,
                                                "issue_number": 12345, "priority": 20, "at": "t"})
            state = publisher.read_receipt(receipt, plan, [item])
            def api(fleet, method, endpoint, *fields):
                if endpoint == f"repos/{publisher.REPO}/issues/12345":
                    return {"id": 7001, "created_at": "2026-11-12T18:00:00Z",
                            "parent_issue_url": issue_record["parent_issue_url"]}
                if endpoint.endswith("/sub_issues?per_page=100"):
                    return [{"number": 12345}]
                if endpoint.endswith("/dependencies/blocked_by"):
                    return []
                raise AssertionError((method, endpoint, fields))
            live_row = {"id": 55, "priority": 20, "status": "claimed", "age": "1m",
                        "dedupe_key": key, "title": "Triage"}
            with patch.object(publisher, "issue_by_marker", return_value=issue):
                with patch.object(publisher, "require_epic_open"):
                    with patch.object(publisher, "api_json", side_effect=api):
                        with patch.object(publisher, "checked", return_value="packet valid") as checked:
                            with patch.object(publisher, "find_queue_task", return_value=(live_row, scans_for_live_row(live_row))):
                                publisher.publish_one(item, "fleet", "repo-queue", publisher.QUEUE,
                                                      receipt, state, {})
            checked.assert_called_once()
            self.assertEqual(checked.call_args.args[0][:2], ["repo-queue", "validate"])
            final = publisher.read_receipt(receipt, plan, [item])
            self.assertIn(item["id"], final["task_verified"])

    def test_wrong_queue_ack_is_recorded_unverified_without_scan_or_retry(self):
        item = publisher.validate_manifest([proposal()])[0]
        issue = gh_issue(item)
        key = "repo-health:v1:triage:developing-today/code:issue:12345"

        def api(fleet, method, endpoint, *fields):
            if endpoint == f"repos/{publisher.REPO}/issues/12345":
                return {"id": 7001, "created_at": "2026-11-12T18:00:00Z",
                        "parent_issue_url": f"https://api.github.com/repos/{publisher.REPO}/issues/9158"}
            if endpoint.endswith("/sub_issues?per_page=100"):
                return [{"number": 12345}]
            if endpoint.endswith("/dependencies/blocked_by"):
                return []
            raise AssertionError((method, endpoint, fields))

        with tempfile.TemporaryDirectory() as tmp:
            receipt = Path(tmp) / "receipt.jsonl"
            checked = ["packet valid", f"enqueued\n{json.dumps({'id': 55, 'dedupe_key': key, 'queue': 'other-stream', 'priority': 20})}"]
            with patch.object(publisher, "issue_by_marker", return_value=issue):
                with patch.object(publisher, "require_epic_open"):
                    with patch.object(publisher, "api_json", side_effect=api):
                        with patch.object(publisher, "checked", side_effect=checked) as command:
                            with patch.object(publisher, "find_queue_task") as find:
                                with self.assertRaisesRegex(publisher.PublishError, "ACK did not identify"):
                                    publisher.publish_one(item, "fleet", "repo-queue", publisher.QUEUE,
                                                          receipt, empty_receipt_state(), {})
                                find.assert_not_called()
                                self.assertEqual(command.call_count, 2)
            events = [json.loads(line)["event"] for line in receipt.read_text().splitlines()]
            self.assertEqual(events, ["create_result", "issue_verified", "queue_intent", "queue_ack", "queue_unverified"])

    def test_native_blocker_uses_blocker_database_id_and_reads_back_issue_number(self):
        blocker, target = publisher.validate_manifest([
            proposal("sec-blocker"), proposal("sec-dependent", depends_on=["sec-blocker"])
        ])
        issue = gh_issue(target, number=12346)
        events = []

        def api(fleet, method, endpoint, *fields):
            events.append((method, endpoint, fields))
            if endpoint == f"repos/{publisher.REPO}/issues/12346":
                return {"id": 7002, "created_at": "2026-11-12T18:01:00Z",
                        "parent_issue_url": f"https://api.github.com/repos/{publisher.REPO}/issues/9158"}
            if endpoint == f"repos/{publisher.REPO}/issues/12345":
                return {"id": 7001, "number": 12345}
            if endpoint.endswith("/sub_issues?per_page=100"):
                return [{"number": 12346}]
            if endpoint.endswith("/dependencies/blocked_by"):
                if method == "POST":
                    return {}
                if any(e[0] == "POST" and e[1].endswith("/dependencies/blocked_by") for e in events):
                    return [{"id": 7001, "number": 12345}]
                return []
            raise AssertionError((method, endpoint, fields))

        key = "repo-health:v1:triage:developing-today/code:issue:12346"
        checked = ["valid", f"enqueued task 56\n{json.dumps({'id': 56, 'dedupe_key': key, 'queue': 'pilot', 'priority': 20})}"]
        live_row = {"id": 56, "priority": 20, "status": "queued", "age": "1m",
                    "dedupe_key": key, "title": "Triage"}
        with tempfile.TemporaryDirectory() as tmp:
            with patch.object(publisher, "issue_by_marker", return_value=issue):
                with patch.object(publisher, "require_epic_open"):
                    with patch.object(publisher, "api_json", side_effect=api):
                        with patch.object(publisher, "checked", side_effect=checked):
                            with patch.object(publisher, "find_queue_task", return_value=(live_row, scans_for_live_row(live_row))):
                                publisher.publish_one(target, "fleet", "repo-queue", "pilot",
                                                      Path(tmp) / "receipt.jsonl", empty_receipt_state(),
                                                      {"sec-blocker": {"number": 12345, "database_id": 7001}})
        dependency_post = [e for e in events if e[0] == "POST" and e[1].endswith("/dependencies/blocked_by")]
        self.assertEqual(dependency_post, [("POST", f"repos/{publisher.REPO}/issues/12346/dependencies/blocked_by",
                                           ("issue_id=7001",))])

    def test_wrong_existing_parent_stops_without_reparenting(self):
        item = publisher.validate_manifest([proposal()])[0]
        issue = gh_issue(item)
        wrong_parent = {"id": 7001, "created_at": "2026-11-12T18:00:00Z",
                        "parent_issue_url": f"https://api.github.com/repos/{publisher.REPO}/issues/9163"}
        with tempfile.TemporaryDirectory() as tmp:
            with patch.object(publisher, "issue_by_marker", return_value=issue):
                with patch.object(publisher, "require_epic_open"):
                    with patch.object(publisher, "api_json", return_value=wrong_parent):
                        with patch.object(publisher, "checked", side_effect=AssertionError("must not mutate")):
                            with self.assertRaisesRegex(publisher.PublishError, "another native parent"):
                                publisher.publish_one(item, "fleet", "repo-queue", publisher.QUEUE,
                                                      Path(tmp) / "receipt.jsonl", empty_receipt_state(), {})

    def test_unexpected_existing_native_blocker_stops_without_removing_it(self):
        item = publisher.validate_manifest([proposal()])[0]
        issue = gh_issue(item)

        def api(fleet, method, endpoint, *fields):
            if endpoint == f"repos/{publisher.REPO}/issues/12345":
                return {"id": 7001, "created_at": "2026-11-12T18:00:00Z",
                        "parent_issue_url": f"https://api.github.com/repos/{publisher.REPO}/issues/9158"}
            if endpoint.endswith("/sub_issues?per_page=100"):
                return [{"number": 12345}]
            if endpoint.endswith("/dependencies/blocked_by"):
                return [{"number": 999, "id": 7999}]
            raise AssertionError((method, endpoint, fields))

        with tempfile.TemporaryDirectory() as tmp:
            with patch.object(publisher, "issue_by_marker", return_value=issue):
                with patch.object(publisher, "require_epic_open"):
                    with patch.object(publisher, "api_json", side_effect=api):
                        with patch.object(publisher, "checked", side_effect=AssertionError("must not enqueue")):
                            with self.assertRaisesRegex(publisher.PublishError, "unexpected native blockers"):
                                publisher.publish_one(item, "fleet", "repo-queue", publisher.QUEUE,
                                                      Path(tmp) / "receipt.jsonl", empty_receipt_state(), {})

    def test_native_dependency_rows_require_positive_ids_canonical_urls_and_uniqueness(self):
        row = {"number": 123, "id": 7001,
               "url": "https://api.github.com/repos/developing-today/code/issues/123",
               "html_url": "https://github.com/developing-today/code/issues/123",
               "repository_url": "https://api.github.com/repos/developing-today/code"}
        self.assertEqual(publisher.validate_dependency_rows([row])[123]["id"], 7001)
        invalid = [
            {"number": 123, "id": 0},
            {"number": "123", "id": 7001},
            {"number": 123, "id": 7001, "url": "https://api.github.com/repos/other/repo/issues/123"},
            {"number": 123, "id": 7001, "repository_url": "https://api.github.com/repos/other/repo"},
        ]
        for candidate in invalid:
            with self.subTest(candidate=candidate):
                with self.assertRaises(publisher.PublishError):
                    publisher.validate_dependency_rows([candidate])
        with self.assertRaisesRegex(publisher.PublishError, "duplicate"):
            publisher.validate_dependency_rows([row, {**row, "id": 7002}])


if __name__ == "__main__":
    unittest.main()
