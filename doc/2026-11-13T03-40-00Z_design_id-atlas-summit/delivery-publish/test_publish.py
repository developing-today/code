#!/usr/bin/env python3
"""Offline regression tests; no GitHub, queue broker, or real CLI calls."""
import importlib.util
import hashlib
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("publisher", HERE / "publish.py")
publisher = importlib.util.module_from_spec(spec)
spec.loader.exec_module(publisher)


class PublisherTests(unittest.TestCase):
    class FakeResponse:
        def __init__(self, body): self.body = body
        def read(self): return self.body
        def __enter__(self): return self
        def __exit__(self, *exc): return None

    def test_unresolved_enqueue_intent_refuses_blind_replay(self):
        for phase in ("queue-intent", "queue-unverified"):
            with self.assertRaises(publisher.Refusal):
                publisher.raise_if_unresolved_enqueue({"phase": phase}, "fixture", "dedupe:key")
        # A recorded queue_id or a pre-intent phase is not unresolved.
        publisher.raise_if_unresolved_enqueue({"phase": "queue-intent", "queue_id": 42}, "fixture", "dedupe:key")
        publisher.raise_if_unresolved_enqueue({"phase": "issue-created"}, "fixture", "dedupe:key")
        publisher.raise_if_unresolved_enqueue({}, "fixture", "dedupe:key")

    def test_self_block_qualifies_every_native_issue_reference(self):
        packet = publisher.queue_packet({"id":"fixture", "title":"Fixture"}, 1, "a" * 64, "2026-11-13T00:00:00Z", [7,8])
        self.assertIn("developing-today/code#7,developing-today/code#8", packet["payload"]["problem"])
        self.assertNotIn("developing-today/code#7,8", packet["payload"]["problem"])

    def test_digest_is_stable_and_matches_contract(self):
        obj = [{"z": "é", "a": 1}]
        self.assertEqual(publisher.canonical(obj), b'[{"a":1,"z":"\xc3\xa9"}]')

    def test_native_cli_blocker_connection_is_normalized_and_complete(self):
        args = type("Args", (), {"gh": "gh"})()
        with patch.object(publisher, "run", return_value=json.dumps({
                "blockedBy": {"nodes": [{"number": 9110}], "totalCount": 1}})):
            self.assertEqual(publisher.gh_issue(args, 1)["blockedBy"], [{"number": 9110}])
        with patch.object(publisher, "run", return_value=json.dumps({
                "blockedBy": {"nodes": [], "totalCount": 1}})):
            with self.assertRaises(publisher.Refusal):
                publisher.gh_issue(args, 1)

    def test_normalized_draft_is_exactly_bound_to_row_and_digest(self):
        row = {"id":"S-1","goal":"Preserve identity.","version":"V2","design_source":"design.md",
          "design_section":"URI — ownership","design_section_unused":"", "tier":"recommended","trigger":"",
          "statements":["S0.1"],"enables":"Conformance.","parent":9110,"owned":[],"hard":[],"soft":[],
          "test_file":"tests/new.rs","command":"cargo test -p example","oracle":{"kind":"invariant","expected":"One stable identity.","verifier":"Fixed reference comparison."},
          "research_evidence":[],"rollback":"Revert this slice.","marker":"<!-- id-atlas-delivery: #9106:S-1 -->"}
        body = publisher.expected_draft(row, "a" * 64)
        self.assertIn("uri--ownership", body)
        self.assertIn("Manifest SHA-256 approval binding: `" + "a" * 64 + "`.", body)
        self.assertEqual(body.count(row["marker"]), 1)

    def test_report_and_drafts_bind_to_repo_relative_manifest_digest(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td) / "repo"
            package = root / "pkg"
            drafts = package / "drafts"
            drafts.mkdir(parents=True)
            old_root, old_package = publisher.ROOT, publisher.PACKAGE
            publisher.ROOT, publisher.PACKAGE = root, package
            try:
                raw = {"slices":[{"id":"S-1"}]}
                (package / "one.json").write_text(json.dumps(raw))
                digest = hashlib.sha256(publisher.canonical([["pkg/one.json", raw]])).hexdigest()
                entry = {"id":"S-1","source":"pkg/one.json","goal":"Preserve identity.","version":"V2","design_source":"design.md",
                  "design_section":"Stable API","tier":"recommended","trigger":"","statements":["S0.1"],
                  "enables":"Conformance.","parent":9110,"owned":[],"hard":[],"soft":[],"test_file":"tests/new.rs",
                  "command":"cargo test -p example","oracle":{"kind":"invariant","expected":"One identity.","verifier":"Fixed oracle."},
                  "research_evidence":[],"rollback":"Revert this slice.","marker":"<!-- id-atlas-delivery: #9106:S-1 -->"}
                (drafts / "S-1.md").write_text(publisher.expected_draft(entry, digest))
                report = {"manifest_sha256":digest,"valid":True,"errors":[],"entries":[entry],
                          "orchestration":{"id":"meta-9112","parent":9112,"marker":"<!-- id-atlas-delivery: #9106:meta-9112 -->"}}
                report_path = package / "report.json"
                report_path.write_text(json.dumps(report))
                report_digest = hashlib.sha256(report_path.read_bytes()).hexdigest()
                args = type("Args", (), {"manifest":["one.json"],"approved_digest":digest,"approved_report_digest":report_digest,"report":str(report_path),"draft_dir":str(drafts)})()
                got, got_digest, got_report_digest, _, _ = publisher.read_inputs(args)
                self.assertEqual(got_digest, digest)
                self.assertEqual(got_report_digest, report_digest)
                self.assertEqual(got[0]["_body"], publisher.expected_draft(entry, digest))
                # Same IDs/order and unchanged raw-manifest approval must not
                # authorize altered normalized report fields.
                report["entries"][0]["goal"] = "tampered normalized goal"
                report_path.write_text(json.dumps(report))
                with self.assertRaisesRegex(publisher.Refusal, "report digest drift"):
                    publisher.read_inputs(args)
            finally:
                publisher.ROOT, publisher.PACKAGE = old_root, old_package

    def test_hard_dependency_order_and_cycle(self):
        rows = [
            {"id": "B", "hard": [{"type": "slice", "id": "A", "strength": "hard"}]},
            {"id": "A", "hard": []},
        ]
        _, graph, order = publisher.validate_dag(rows)
        self.assertEqual(order, ["A", "B"])
        self.assertEqual(graph["B"], ["A"])
        rows[0]["hard"] = [{"type": "slice", "id": "A"}]
        rows[1]["hard"] = [{"type": "slice", "id": "B"}]
        with self.assertRaisesRegex(publisher.Refusal, "cycle"):
            publisher.validate_dag(rows)

    def test_unknown_or_malformed_hard_dependency_fails_closed(self):
        with self.assertRaisesRegex(publisher.Refusal, "unknown"):
            publisher.validate_dag([{"id": "A", "hard": ["missing"]}])
        with self.assertRaisesRegex(publisher.Refusal, "malformed"):
            publisher.validate_dag([{"id": "A", "hard": [{"type": "mystery", "id": 4}]}])

    def test_enqueue_row_requires_supported_json_result(self):
        row = publisher.enqueue_row('enqueued new task 42\n{"id":42,"queue":"repo-health-code-pilot","status":"queued"}\n')
        self.assertEqual(row["id"], 42)
        with self.assertRaisesRegex(publisher.Refusal, "no JSON row"):
            publisher.enqueue_row("enqueued new task 42\n")

    def test_broker_readback_exact_task_queue_key_and_live_status(self):
        key = "repo-health:v1:triage:developing-today/code:issue:9"
        args = type("Args", (), {"_broker_url":"http://broker","_broker_token":"fake-token"})()
        response = self.FakeResponse(json.dumps({"ok":True,"rows":[{"id":42,"queue":publisher.QUEUE,"status":"queued","dedupe_key":key}]}).encode())
        with patch.object(publisher.urllib.request, "urlopen", return_value=response) as open_url:
            row = publisher.queue_readback(args, 42, key)
        self.assertEqual(row["id"], 42)
        request = open_url.call_args.args[0]
        self.assertEqual(json.loads(request.data), {"queue":publisher.QUEUE,"task":42,"limit":1,"offset":0})
        self.assertEqual(request.get_header("X-agent-token"), "fake-token")
        for bad in [
            {"ok":True,"rows":[{"id":43,"queue":publisher.QUEUE,"status":"queued","dedupe_key":key}]},
            {"ok":True,"rows":[{"id":42,"queue":"shadow","status":"queued","dedupe_key":key}]},
            {"ok":True,"rows":[{"id":42,"queue":publisher.QUEUE,"status":"done","dedupe_key":key}]},
            {"ok":True,"rows":[{"id":42,"queue":publisher.QUEUE,"status":"queued","dedupe_key":"wrong"}]},
        ]:
            fake = self.FakeResponse(json.dumps(bad).encode())
            with patch.object(publisher.urllib.request, "urlopen", return_value=fake):
                with self.assertRaisesRegex(publisher.Refusal, "readback"):
                    publisher.queue_readback(args, 42, key)

    def test_enqueue_row_from_offline_fake_cli_output(self):
        with tempfile.TemporaryDirectory() as td:
            fake = Path(td) / "fleet-fake"
            fake.write_text("#!/usr/bin/env python3\nprint('enqueued new task 15')\nprint('{\\\"id\\\":15,\\\"queue\\\":\\\"repo-health-code-pilot\\\",\\\"status\\\":\\\"queued\\\"}')\n")
            fake.chmod(0o755)
            out = publisher.run([str(fake), "queue", "enqueue"])
            self.assertEqual(publisher.enqueue_row(out)["id"], 15)

    def test_queue_packet_is_preflight_only_and_never_authorizes_rollout(self):
        entry = {"id":"S-1","parent":9110,"title":"Slice","trigger":"benchmark gate passed"}
        packet = publisher.queue_packet(entry, 9, "a"*64, "2026-11-12T00:00:00Z", [7,8])
        payload = packet["payload"]
        self.assertEqual(payload["kind"], "triage")
        self.assertIn("do not implement", payload["problem"])
        self.assertIn("self-block", payload["problem"])
        self.assertIn("benchmark gate passed", payload["problem"])
        self.assertIn("not production rollout authorization", payload["problem"])
        self.assertNotIn("Implement approved", payload["problem"])

    def test_dry_run_has_no_lock_or_journal_writes_or_external_calls(self):
        args = type("Args", (), {"journal": str(Path(tempfile.gettempdir()) / "missing-id-atlas-test" / "j.json"), "execute": False, "design_sha": "a" * 40})()
        entry = {"id":"A","parent":9110,"title":"Slice A","_body":"draft"}
        orch = {"parent":9112,"title":"Plan"}
        with patch.object(publisher, "run", side_effect=AssertionError("external command in dry-run")):
            rows = publisher.publish(args, [entry], "b" * 64, "c" * 64, orch)
        self.assertEqual([r["id"] for r in rows], ["A", "orchestration"])
        self.assertFalse(Path(args.journal).exists())

    def test_paths_reject_traversal(self):
        with self.assertRaisesRegex(publisher.Refusal, "unsafe"):
            publisher.inside("../../outside", publisher.PACKAGE)

    def test_fakegit_old_merge_with_different_local_manifest_stops_before_gh(self):
        with tempfile.TemporaryDirectory() as td:
            temp = Path(td)
            fake_git = temp / "git-fake"
            old_tree = {"slices":[{"id":"old-committed"}]}
            fake_git.write_text("#!/usr/bin/env python3\nimport json,sys\n"
                f"if sys.argv[1:] == ['rev-parse','--show-toplevel']: print({str(publisher.ROOT)!r})\n"
                f"elif sys.argv[1] == 'show': print({json.dumps(old_tree)!r})\n"
                "else: sys.exit(9)\n")
            fake_git.chmod(0o755)
            local = temp / "local-manifest.json"
            local.write_text(json.dumps({"slices":[{"id":"unmerged-local"}]}))
            argv = ["--approved-digest","a"*64,"--approved-report-digest","b"*64,"--design-sha","c"*40,
                    "--journal",str(temp / "journal.json"),"--execute","--go","9106","--git",str(fake_git)]
            with patch.object(publisher, "read_inputs", return_value=([],"a"*64,"b"*64,{},["fixture.json"])), \
                 patch.object(publisher, "approved_tree_inputs", return_value=[("doc/fixture.json",local,True)]), \
                 patch.object(publisher, "verify_main") as gh_read, patch.object(publisher, "publish") as gh_write:
                result = publisher.main(argv)
            self.assertEqual(result, 2)
            gh_read.assert_not_called()
            gh_write.assert_not_called()


if __name__ == "__main__":
    unittest.main()
