#!/usr/bin/env python3
"""Offline tests for claimant ordering and fail-closed behavior."""

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("claimant", ROOT / "claimant.py")
claimant = importlib.util.module_from_spec(spec)
spec.loader.exec_module(claimant)
poll_spec = importlib.util.spec_from_file_location("claimant_poll_test", ROOT / "poll.py")
poll_module = importlib.util.module_from_spec(poll_spec)
poll_spec.loader.exec_module(poll_module)


class TestLaunch(unittest.TestCase):
    def candidate(self):
        return {
            "id": 73001,
            "queue": "repo-health-code-pilot",
            "priority": 1,
            "worker_id": "repo-worker-code",
            "lane": "opencode",
            "stage": "handed_off",
            "session_id": None,
            "issue_number": 3026,
            "packet": {
                "payload": {
                    "repo": "developing-today/code",
                    "issue": {"number": 3026},
                    "route": {"providerID": "opencode-route", "modelID": "summit"},
                    "password": "must-not-enter-prompt",
                }
            },
        }

    def test_session_is_bound_before_checkpoint_and_prompt(self):
        candidate = self.candidate()
        calls = []

        def api(method, path, body=None, timeout=15):
            calls.append(("api", method, path, body))
            if method == "POST" and path == "/api/session":
                self.assertEqual(body["location"]["directory"], str(Path.home() / "git/code"))
                self.assertEqual(body["model"], {"id": "summit", "providerID": "opencode-route"})
                return {"id": "ses_test", "location": body["location"]}
            return None

        def cli(args, timeout=30):
            calls.append(("cli", args))
            return "{}"

        with patch.object(claimant, "_verify_candidate_is_current", return_value=True), \
             patch.object(claimant, "_github_issue_labels", return_value=set()), \
             patch.object(claimant, "_load_runbook", return_value="runbook body"), \
             patch.object(claimant, "_api_request", side_effect=api), \
             patch.object(claimant, "_run_cli", side_effect=cli), \
             patch.object(claimant, "_post_launch_note", side_effect=lambda *args: calls.append(("note",))):
            prepared = claimant.prepare_candidate(candidate)
            result = claimant.launch_one(prepared)

        self.assertEqual(result["status"], "prompted")
        bind_at = next(i for i, call in enumerate(calls) if call[0] == "cli" and call[1][0] == "bind")
        checkpoint_at = next(i for i, call in enumerate(calls) if call[0] == "cli" and call[1][0] == "checkpoint")
        prompt_at = next(i for i, call in enumerate(calls) if call[0] == "api" and call[1] == "POST" and call[2].endswith("/prompt"))
        note_at = next(i for i, call in enumerate(calls) if call[0] == "note")
        self.assertLess(bind_at, checkpoint_at)
        self.assertLess(checkpoint_at, prompt_at)
        self.assertLess(prompt_at, note_at)
        prompt = calls[prompt_at][3]["text"]
        self.assertIn("73001", prompt)
        self.assertIn("repo-worker-code", prompt)
        self.assertIn('"session_id":"ses_test"', prompt)
        self.assertNotIn("must-not-enter-prompt", prompt)

    def test_create_failure_checkpoints_without_binding(self):
        candidate = self.candidate()
        calls = []

        def cli(args, timeout=30):
            calls.append(args)
            return "{}"

        with patch.object(claimant, "_verify_candidate_is_current", return_value=True), \
             patch.object(claimant, "_api_request", side_effect=RuntimeError("server unavailable")), \
             patch.object(claimant, "_run_cli", side_effect=cli):
            result = claimant.launch_one({"candidate": candidate, "packet": candidate["packet"], "payload": candidate["packet"]["payload"], "repo": "developing-today/code", "directory": str(Path.home() / "git/code"), "issue_number": 3026, "route": claimant._route(candidate["packet"], candidate["packet"]["payload"]), "runbook": "runbook"})

        self.assertEqual(result["reason"], "session_create_failed")
        self.assertEqual(len(calls), 1)
        self.assertEqual(calls[0][0], "checkpoint")
        self.assertIn("--stage", calls[0])
        self.assertIn("launch_failed", calls[0])

    def test_held_issue_label_prevents_preflight(self):
        candidate = self.candidate()
        with patch.object(claimant, "_github_issue_labels", return_value={"do-not-merge"}):
            with self.assertRaisesRegex(claimant.ClaimantError, "held issue labels"):
                claimant.prepare_candidate(candidate)


class TestSupervision(unittest.TestCase):
    def test_zero_message_delete_precedes_atomic_recovery(self):
        session = {"id": "ses_old", "time": {"created": 1, "updated": 1}}
        row = {
            "id": 73001,
            "queue": "repo-health-code-pilot",
            "worker_id": "repo-worker-code",
            "lane_id": "code-a",
            "lease_until": "2020-01-01T00:00:00Z",
            "lane": "opencode",
            "stage": "launched",
            "session_id": "ses_old",
            "attempts": 0,
            "max_attempts": 3,
        }
        data = {"bound": [row], "stuck_bound": [row]}
        actions = []
        session_reads = {"count": 0}

        def fetch_sessions():
            session_reads["count"] += 1
            return [session] if session_reads["count"] <= 3 else []

        poll = type("Poll", (), {
            "AUTH_FILE": "/unread",
            "parse_lease_ms": staticmethod(lambda _: 0),
            "is_active_root": staticmethod(lambda *_: False),
            "is_stuck_bound": staticmethod(lambda *_: True),
            "fetch_sessions": staticmethod(fetch_sessions),
            "fetch_active_ids": staticmethod(lambda: set()),
        })

        def api(method, path, body=None, timeout=15):
            actions.append(("api", method))
            return None

        def cli(args, timeout=30):
            actions.append(("cli", args))
            return "{}"

        with patch.object(claimant, "_POLL", poll), \
             patch.object(claimant, "_current_bound_row", return_value=row), \
             patch.object(claimant, "_read_messages", side_effect=lambda _: (actions.append(("messages",)) or [])), \
             patch.object(claimant, "_api_request", side_effect=api), \
             patch.object(claimant, "_run_cli", side_effect=cli):
            report, skips = claimant.supervision_step(data, "live")

        self.assertEqual(skips, [])
        self.assertEqual(report[0]["status"], "reclaimed")
        message_at = next(i for i, item in enumerate(actions) if item[0] == "messages")
        delete_at = next(i for i, item in enumerate(actions) if item[:2] == ("api", "DELETE"))
        recovery_at = next(i for i, item in enumerate(actions) if item[0] == "cli")
        self.assertLess(message_at, delete_at)
        self.assertLess(delete_at, recovery_at)
        self.assertIn(claimant.LAUNCHED_BY_RECOVERY_ZERO_MESSAGE, actions[recovery_at][1])





if __name__ == "__main__":
    unittest.main()
