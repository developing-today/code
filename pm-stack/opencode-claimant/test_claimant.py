#!/usr/bin/env python3
"""Tests for claimant.py's launch and supervision steps (#6865 / task 11370).

Run:  python3 -m unittest pm-stack/opencode-claimant/test_claimant.py

Oracle: the documented claimant contract — a valid handed_off candidate
is launched (session created via the session API, task bound via the
repo-queue CLI) and reported in ; every candidate that cannot
launch is reported in  with its reason, and main() records those
skips in the fires.jsonl entry. A bound session that is gone from the
server with zero messages is a supervision recovery candidate.
All transport (subprocess: curl + repo-queue CLI) and poll discovery are
stubbed; nothing touches a server or the queue.
"""

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

_spec = importlib.util.spec_from_file_location(
    "claimant", Path(__file__).resolve().parent / "claimant.py"
)
claimant = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(claimant)


class Result:
    def __init__(self, returncode=0, stdout="", stderr=""):
        self.returncode = returncode
        self.stdout = stdout
        self.stderr = stderr


def candidate(task_id=123, **over):
    c = {
        "id": task_id,
        "lane": "opencode",
        "session_id": None,
        "stage": "handed_off",
        "worker_id": "repo-worker-code",
        "issue": "#9999",
        "packet": {},
    }
    c.update(over)
    return c


class TestLaunchStep(unittest.TestCase):
    def run_launch(self, candidates, cap_room=1, create_stdout=None):
        calls = []

        def fake_run(argv, **kwargs):
            calls.append(argv)
            if "bind" in argv:
                return Result(0)
            if any("api/session" in str(a) for a in argv):
                body = create_stdout if create_stdout is not None else {
                    "id": "ses_test1",
                    "location": {"directory": str(claimant.PRIMARY_CLONE)},
                }
                return Result(0, json.dumps(body))
            return Result(0)

        with patch.object(claimant, "run_poll",
                          return_value={"candidates": candidates}), \
             patch.object(claimant.subprocess, "run", side_effect=fake_run):
            launched, skips = claimant.launch_step("live", cap_room)
        return launched, skips, calls

    def test_valid_handed_off_candidate_launches_and_binds(self):
        launched, skips, calls = self.run_launch([candidate()])
        self.assertEqual(skips, [])
        self.assertEqual(len(launched), 1)
        self.assertEqual(launched[0]["task_id"], 123)
        self.assertEqual(launched[0]["session_id"], "ses_test1")
        bind_calls = [a for a in calls if "bind" in a]
        self.assertEqual(len(bind_calls), 1)
        self.assertIn("123", bind_calls[0])
        self.assertIn("ses_test1", bind_calls[0])

    def test_wrong_location_session_is_skipped_not_launched(self):
        launched, skips, _ = self.run_launch(
            [candidate()],
            create_stdout={"id": "ses_bad",
                           "location": {"directory": "/somewhere/else"}},
        )
        self.assertEqual(launched, [])
        self.assertEqual([s["reason"] for s in skips], ["session_wrong_location"])

    def test_cap_saturation_is_recorded_as_a_skip(self):
        launched, skips, _ = self.run_launch(
            [candidate(1), candidate(2)], cap_room=1)
        self.assertEqual(len(launched), 1)
        self.assertEqual([s["reason"] for s in skips], ["cap_saturated"])
        self.assertEqual(skips[0]["task_id"], 2)

    def test_no_cap_room_launches_and_skips_nothing(self):
        launched, skips, _ = self.run_launch([candidate()], cap_room=0)
        self.assertEqual((launched, skips), ([], []))

    def test_main_records_launch_skips_in_the_fire_entry(self):
        with tempfile.TemporaryDirectory() as d:
            state = Path(d)
            with patch.object(claimant, "STATE_DIR", state), \
                 patch.object(claimant, "FIRES_FILE", state / "fires.jsonl"), \
                 patch.object(claimant, "read_mode", return_value="live"), \
                 patch.object(claimant, "supervision_step", return_value=[]), \
                 patch.object(claimant, "gate_step", return_value=True), \
                 patch.object(claimant, "cap_step", return_value=1), \
                 patch.object(claimant, "launch_step",
                              return_value=([], [{"task_id": 9, "reason": "bind_failed"}])):
                rc = claimant.main()
            self.assertEqual(rc, 0)
            entry = json.loads((state / "fires.jsonl").read_text().splitlines()[-1])
            self.assertEqual(entry["skips"], [{"task_id": 9, "reason": "bind_failed"}])


class TestSupervisionStep(unittest.TestCase):
    TASK = {
        "id": 555,
        "status": "claimed",
        "attempts": 1,
        "max_attempts": 5,
        "progress": {"lane": "opencode", "session_id": "ses_gone"},
    }

    def run_supervision(self, mode="live"):
        calls = []

        def fake_run(argv, **kwargs):
            calls.append(argv)
            argv_s = [str(a) for a in argv]
            if "list" in argv_s:
                queue = argv_s[argv_s.index("--queue") + 1]
                rows = [self.TASK] if queue == "repo-health-code-pilot" else []
                return Result(0, json.dumps({"rows": rows}))
            if "DELETE" in argv_s:
                return Result(0)
            if any("/message" in a for a in argv_s):
                return Result(0, "[]")
            if any("api/session" in a for a in argv_s):
                return Result(0, "\n404")  # session positively gone: HTTP 404 (curl -w shape)
            if "bind" in argv_s:
                return Result(0)
            return Result(0)

        with patch.object(claimant.subprocess, "run", side_effect=fake_run):
            recovered = claimant.supervision_step(mode)
        return recovered, calls

    def test_gone_zero_message_session_is_recovered(self):
        recovered, calls = self.run_supervision()
        self.assertEqual(
            recovered,
            [{"task_id": 555, "session_id": "ses_gone", "reason": "zero_message"}],
        )
        bind_calls = [a for a in calls if "bind" in [str(x) for x in a]]
        self.assertEqual(len(bind_calls), 1)
        flat = [str(x) for x in bind_calls[0]]
        self.assertIn("555", flat)
        self.assertIn(claimant.LAUNCHED_BY_RECOVERY_ZERO_MESSAGE, flat)

    def test_shadow_mode_recovers_nothing(self):
        recovered, calls = self.run_supervision(mode="shadow")
        self.assertEqual(recovered, [])
        self.assertEqual(calls, [])


class TestCodexFindings(unittest.TestCase):
    """Regression tests for the six Codex findings on PR #6990."""

    def run_launch(self, candidates, cap_room=1, create_stdout=None,
                   prompt_rc=0, mode="live"):
        calls = []

        def fake_run(argv, **kwargs):
            calls.append(argv)
            argv_s = [str(a) for a in argv]
            if "bind" in argv_s:
                return Result(0)
            if any(a.endswith("/prompt") for a in argv_s):
                return Result(prompt_rc, "", "prompt boom" if prompt_rc else "")
            if any("api/session" in a for a in argv_s):
                body = create_stdout if create_stdout is not None else {
                    "id": "ses_test1",
                    "location": {"directory": str(claimant.PRIMARY_CLONE)},
                }
                return Result(0, json.dumps(body))
            return Result(0)

        with patch.object(claimant, "run_poll",
                          return_value={"candidates": candidates}), \
             patch.object(claimant.subprocess, "run", side_effect=fake_run):
            launched, skips = claimant.launch_step(mode, cap_room)
        return launched, skips, calls

    def test_shadow_mode_launch_is_read_only(self):
        launched, skips, calls = self.run_launch([candidate()], mode="shadow")
        self.assertEqual(launched, [])
        self.assertEqual([s["reason"] for s in skips],
                         ["shadow_mode_simulated"])
        self.assertEqual(calls, [])

    def test_enveloped_create_response_is_unwrapped(self):
        launched, skips, _ = self.run_launch(
            [candidate()],
            create_stdout={"data": {
                "id": "ses_env",
                "location": {"directory": str(claimant.PRIMARY_CLONE)}}})
        self.assertEqual(skips, [])
        self.assertEqual(len(launched), 1)
        self.assertEqual(launched[0]["session_id"], "ses_env")

    def test_create_without_id_is_a_skip_not_a_phantom_delete(self):
        launched, skips, calls = self.run_launch(
            [candidate()], create_stdout={"data": {"location": {}}})
        self.assertEqual(launched, [])
        self.assertEqual([s["reason"] for s in skips], ["session_create_no_id"])
        self.assertFalse(any("None" in str(a) for a in calls))

    # --- Codex round-2 on PR #6990 (2026-11-08T01:31Z) ---
    def test_prompt_failure_after_bind_keeps_session_and_binding(self):
        launched, skips, calls = self.run_launch([candidate()], prompt_rc=1)
        self.assertEqual(launched, [])
        self.assertEqual(len(skips), 1)
        self.assertEqual(skips[0]["reason"], "prompt_delivery_failed_bound")
        self.assertEqual(skips[0]["session_id"], "ses_test1")
        flat = [[str(x) for x in a] for a in calls]
        # the bind DID happen (it precedes delivery now) ...
        self.assertTrue(any("bind" in a for a in flat))
        # ... and the bound session was NOT deleted on prompt failure
        self.assertFalse(any("DELETE" in a for a in flat))

    def test_prompt_http_error_counts_as_delivery_failure(self):
        # curl --fail-with-body exits 22 on an HTTP error status; that
        # must be treated as a failed delivery, never as success.
        launched, skips, calls = self.run_launch([candidate()], prompt_rc=22)
        self.assertEqual(launched, [])
        self.assertEqual([s["reason"] for s in skips],
                         ["prompt_delivery_failed_bound"])
        flat = [[str(x) for x in a] for a in calls]
        prompt_calls = [a for a in flat if any(x.endswith("/prompt") for x in a)]
        self.assertTrue(prompt_calls)
        for a in prompt_calls:
            self.assertIn("--fail-with-body", a)

    def test_bind_is_sent_before_prompt(self):
        _, _, calls = self.run_launch([candidate()])
        flat = [[str(x) for x in a] for a in calls]
        bind_idx = next(i for i, a in enumerate(flat) if "bind" in a)
        prompt_idx = next(i for i, a in enumerate(flat)
                          if any(x.endswith("/prompt") for x in a))
        self.assertLess(bind_idx, prompt_idx)

    def test_prompt_resend_to_same_session_recovers(self):
        calls = []
        prompt_attempts = {"n": 0}

        def fake_run(argv, **kwargs):
            calls.append(argv)
            argv_s = [str(a) for a in argv]
            if "bind" in argv_s:
                return Result(0)
            if any(a.endswith("/prompt") for a in argv_s):
                prompt_attempts["n"] += 1
                if prompt_attempts["n"] == 1:
                    return Result(1, "", "transient")
                return Result(0)
            if any("/message" in a for a in argv_s):
                return Result(0, "[]")
            if any("api/session" in a for a in argv_s):
                return Result(0, json.dumps({
                    "id": "ses_test1",
                    "location": {"directory": str(claimant.PRIMARY_CLONE)}}))
            return Result(0)

        with patch.object(claimant, "run_poll",
                          return_value={"candidates": [candidate()]}), \
             patch.object(claimant.subprocess, "run", side_effect=fake_run):
            launched, skips = claimant.launch_step("live", 1)
        self.assertEqual(skips, [])
        self.assertEqual(len(launched), 1)
        self.assertEqual(prompt_attempts["n"], 2)

    def test_session_calls_use_configured_opencode_url(self):
        with patch.object(claimant, "OPENCODE_URL", "http://example.test:9999"):
            launched, skips, calls = self.run_launch([candidate()])
        self.assertEqual(len(launched), 1)
        flat = [[str(x) for x in a] for a in calls]
        session_urls = [x for a in flat for x in a if "api/session" in x]
        self.assertTrue(session_urls)
        for u in session_urls:
            self.assertTrue(u.startswith("http://example.test:9999"), u)
        self.assertFalse(any("127.0.0.1:13420" in x for a in flat for x in a))

    def test_session_calls_carry_auth_header_when_credential_exists(self):
        with tempfile.TemporaryDirectory() as d:
            auth = Path(d) / "auth.json"
            auth.write_text(json.dumps({"password": "sekrit"}))
            with patch.object(claimant, "AUTH_FILE", str(auth)):
                _, _, calls = self.run_launch([candidate()])
        flat = [[str(x) for x in a] for a in calls]
        create = next(a for a in flat if any("api/session" in x for x in a))
        self.assertIn("Authorization: Basic b3BlbmNvZGU6c2Vrcml0", create)

    def test_main_records_discovered_candidates_in_fire_entry(self):
        seen = [{"id": 42, "lane": "opencode"}]
        with tempfile.TemporaryDirectory() as d:
            state = Path(d)
            with patch.object(claimant, "STATE_DIR", state), \
                 patch.object(claimant, "FIRES_FILE", state / "fires.jsonl"), \
                 patch.object(claimant, "read_mode", return_value="live"), \
                 patch.object(claimant, "supervision_step", return_value=[]), \
                 patch.object(claimant, "gate_step", return_value=True), \
                 patch.object(claimant, "cap_step", return_value=1), \
                 patch.object(claimant, "run_poll",
                              return_value={"candidates": seen}), \
                 patch.object(claimant, "launch_step", return_value=([], [])):
                rc = claimant.main()
            self.assertEqual(rc, 0)
            entry = json.loads((state / "fires.jsonl").read_text().splitlines()[-1])
            self.assertEqual(entry["candidates_seen"], seen)


class TestSupervisionStuckBound(unittest.TestCase):
    def test_stale_nonterminal_session_reaches_recovery(self):
        task = {
            "id": 777, "status": "claimed", "attempts": 1, "max_attempts": 5,
            "lease_until": "2000-01-01T00:00:00Z",
            "progress": {"lane": "opencode", "session_id": "ses_stuck"},
        }
        session = {"id": "ses_stuck", "time": {"created": 0, "updated": 0}}

        class FakePoll:
            @staticmethod
            def fetch_sessions():
                return [session]

            @staticmethod
            def fetch_active_ids():
                return set()

            @staticmethod
            def is_stuck_bound(row, sess, active_ids, now_ms):
                return True

        calls = []

        def fake_run(argv, **kwargs):
            calls.append(argv)
            argv_s = [str(a) for a in argv]
            if "list" in argv_s:
                queue = argv_s[argv_s.index("--queue") + 1]
                rows = [task] if queue == "repo-health-code-pilot" else []
                return Result(0, json.dumps({"rows": rows}))
            if "DELETE" in argv_s:
                return Result(0)
            if any("/message" in a for a in argv_s):
                return Result(0, "[]")
            if any("api/session" in a for a in argv_s):
                return Result(0, json.dumps(session) + "\n200")
            if "bind" in argv_s:
                return Result(0)
            return Result(0)

        with patch.object(claimant, "_load_poll_module", return_value=FakePoll), \
             patch.object(claimant.subprocess, "run", side_effect=fake_run):
            recovered = claimant.supervision_step("live")
        self.assertEqual(
            recovered,
            [{"task_id": 777, "session_id": "ses_stuck",
              "reason": "zero_message"}])


class TestCodexRound3(unittest.TestCase):
    """Regression tests for the four Codex round-3 findings on PR #6990
    (2026-11-08T02:26Z): verified-empty resend guard, the supported v2
    message endpoint, cap reservation for retained sessions, and
    packet-route selection at session creation."""

    def run_launch(self, candidates, cap_room=1, prompt_rcs=(0,),
                   messages_result=None):
        """messages_result: Result for the /message read, or None to
        answer an enveloped empty array (verified empty)."""
        calls = []
        state = {"prompt_n": 0}

        def fake_run(argv, **kwargs):
            calls.append(argv)
            argv_s = [str(a) for a in argv]
            if "bind" in argv_s:
                return Result(0)
            if any(a.endswith("/prompt") for a in argv_s):
                rc = prompt_rcs[min(state["prompt_n"], len(prompt_rcs) - 1)]
                state["prompt_n"] += 1
                return Result(rc, "", "boom" if rc else "")
            if any("/message" in a for a in argv_s):
                if messages_result is not None:
                    return messages_result
                return Result(0, json.dumps({"data": []}))
            if any("api/session" in a for a in argv_s):
                return Result(0, json.dumps({
                    "id": "ses_test1",
                    "location": {"directory": str(claimant.PRIMARY_CLONE)}}))
            return Result(0)

        with patch.object(claimant, "run_poll",
                          return_value={"candidates": candidates}), \
             patch.object(claimant.subprocess, "run", side_effect=fake_run):
            launched, skips = claimant.launch_step("live", cap_room)
        return launched, skips, calls, state

    def test_prompt_not_resent_when_messages_read_inconclusive(self):
        # P1: the first prompt POST fails and the messages read ALSO
        # fails — delivery state is unknown, so no resend may happen.
        launched, skips, calls, state = self.run_launch(
            [candidate()], prompt_rcs=(1,),
            messages_result=Result(1, "", "timeout"))
        self.assertEqual(launched, [])
        self.assertEqual(state["prompt_n"], 1)
        self.assertEqual([s["reason"] for s in skips],
                         ["prompt_delivery_failed_bound"])

    def test_prompt_resent_only_after_verified_empty(self):
        launched, skips, calls, state = self.run_launch(
            [candidate()], prompt_rcs=(1, 0))
        self.assertEqual(skips, [])
        self.assertEqual(len(launched), 1)
        self.assertEqual(state["prompt_n"], 2)

    def test_message_reads_use_singular_v2_endpoint(self):
        _, _, calls, _ = self.run_launch(
            [candidate()], prompt_rcs=(1, 0))
        flat = [str(a) for a in calls]
        msg_urls = [x for x in flat if "/message" in x]
        self.assertTrue(msg_urls)
        for u in msg_urls:
            self.assertIn("/message?limit=", u)
        self.assertFalse(any("/messages" in x for x in flat))

    def test_retained_session_consumes_cap_room(self):
        # P2: candidate 123's session is retained after delivery
        # failure, so candidate 124 must see a saturated cap.
        launched, skips, calls, state = self.run_launch(
            [candidate(123), candidate(124)], cap_room=1,
            prompt_rcs=(1,), messages_result=Result(1, "", "timeout"))
        self.assertEqual(launched, [])
        self.assertEqual(
            [(s["task_id"], s["reason"]) for s in skips],
            [(123, "prompt_delivery_failed_bound"),
             (124, "cap_saturated")])

    def test_packet_route_wins_at_session_creation(self):
        launched, skips, calls, _ = self.run_launch(
            [candidate(packet={"route": "specialist"})])
        self.assertEqual(len(launched), 1)
        create = next(a for a in calls
                      if any("api/session" in str(x) for x in a)
                      and not any("/prompt" in str(x) for x in a)
                      and not any("/message" in str(x) for x in a))
        body = json.loads(create[create.index("-d") + 1])
        self.assertEqual(body["model"],
                         {"id": "specialist", "providerID": "opencode-route"})

    def test_default_route_is_workhorse(self):
        launched, skips, calls, _ = self.run_launch([candidate()])
        self.assertEqual(len(launched), 1)
        create = next(a for a in calls
                      if any("api/session" in str(x) for x in a)
                      and not any("/prompt" in str(x) for x in a)
                      and not any("/message" in str(x) for x in a))
        body = json.loads(create[create.index("-d") + 1])
        self.assertEqual(body["model"]["id"], "workhorse")

    def test_route_model_normalization(self):
        self.assertEqual(claimant._packet_route_model({}), "workhorse")
        self.assertEqual(
            claimant._packet_route_model({"route": "opencode-route/summit"}),
            "summit")
        self.assertEqual(
            claimant._packet_route_model({"route": "specialist#high"}),
            "specialist")
        self.assertEqual(
            claimant._packet_route_model({"model": {"id": "scout"}}), "scout")



class TestCodexRound4(unittest.TestCase):
    """Regression tests for the two Codex round-4 findings on PR #6990
    (2026-11-08T02:53Z): bind-timeout reconciliation in launch_step, and
    the exhausted-attempts terminal path in supervision_step
    (prompt.md step 1e)."""

    # ---- launch_step: bind timeout ----

    def run_launch_bind_timeout(self, candidates, reconcile_poll, cap_room=1):
        calls = []
        def fake_run(argv, **kwargs):
            calls.append(argv)
            argv_s = [str(a) for a in argv]
            if "bind" in argv_s:
                raise claimant.subprocess.TimeoutExpired(argv, 10)
            if any(a.endswith("/prompt") for a in argv_s):
                return Result(0)
            if "DELETE" in argv_s:
                return Result(0)
            if any("api/session" in a for a in argv_s):
                return Result(0, json.dumps({
                    "id": "ses_test1",
                    "location": {"directory": str(claimant.PRIMARY_CLONE)}}))
            return Result(0)
        base = {"candidates": candidates}
        with patch.object(claimant, "run_poll",
                          side_effect=[base, base, reconcile_poll]),              patch.object(claimant.subprocess, "run", side_effect=fake_run):
            launched, skips = claimant.launch_step("live", cap_room)
        return launched, skips, calls

    def test_bind_timeout_committed_binding_proceeds_to_prompt(self):
        launched, skips, calls = self.run_launch_bind_timeout(
            [candidate()], {"candidates": [candidate(session_id="ses_test1")]})
        self.assertEqual(skips, [])
        self.assertEqual(len(launched), 1)
        self.assertEqual(launched[0]["session_id"], "ses_test1")
        self.assertTrue(any(any(a.endswith("/prompt") for a in [str(x) for x in c]) for c in calls))

    def test_bind_timeout_unbound_cleans_up_session(self):
        launched, skips, calls = self.run_launch_bind_timeout(
            [candidate()], {"candidates": [candidate()]})
        self.assertEqual(launched, [])
        self.assertEqual([(s["task_id"], s["reason"], s.get("session_id")) for s in skips],
                         [(123, "bind_timeout_unbound", "ses_test1")])
        self.assertTrue(any("DELETE" in [str(x) for x in c] for c in calls))
        self.assertFalse(any(any(a.endswith("/prompt") for a in [str(x) for x in c]) for c in calls))

    def test_bind_timeout_unreconciled_retains_session_and_cap(self):
        launched, skips, calls = self.run_launch_bind_timeout(
            [candidate(123), candidate(124)], RuntimeError("poll down"), cap_room=1)
        self.assertEqual(launched, [])
        self.assertEqual([(s["task_id"], s["reason"]) for s in skips],
                         [(123, "bind_timeout_unreconciled"), (124, "cap_saturated")])
        self.assertEqual(skips[0].get("session_id"), "ses_test1")
        self.assertFalse(any("DELETE" in [str(x) for x in c] for c in calls))

    # ---- supervision_step: attempts exhausted ----

    def run_supervision_exhausted(self, task, session_rc, session_body="", fake_poll=None, delete_rc=0):
        calls = []
        def fake_run(argv, **kwargs):
            calls.append(argv)
            argv_s = [str(a) for a in argv]
            if "list" in argv_s:
                queue = argv_s[argv_s.index("--queue") + 1]
                rows = [task] if queue == "repo-health-code-pilot" else []
                return Result(0, json.dumps({"rows": rows}))
            if "DELETE" in argv_s:
                return Result(delete_rc)
            if any("/message" in a for a in argv_s):
                return Result(0, "[]")
            if any("api/session" in a for a in argv_s):
                if session_rc == 0:
                    return Result(0, session_body + "\n200")
                return Result(0, "\n404")  # positively verified absence
            return Result(0)
        patches = [patch.object(claimant.subprocess, "run", side_effect=fake_run)]
        if fake_poll is not None:
            patches.append(patch.object(claimant, "_load_poll_module", return_value=fake_poll))
        for pt in patches:
            pt.start()
        try:
            recovered = claimant.supervision_step("live")
        finally:
            for pt in patches:
                pt.stop()
        return recovered, calls

    @staticmethod
    def fail_calls(calls):
        return [[str(x) for x in a] for a in calls if "fail" in [str(x) for x in a]]

    def test_gone_session_attempts_exhausted_fails_terminally(self):
        task = {"id": 555, "status": "claimed", "attempts": 5, "max_attempts": 5,
                "worker_id": "repo-worker-code",
                "progress": {"lane": "opencode", "session_id": "ses_gone"}}
        recovered, calls = self.run_supervision_exhausted(task, session_rc=1)
        self.assertEqual(recovered, [])
        fails = self.fail_calls(calls)
        self.assertEqual(len(fails), 1)
        flat = fails[0]
        self.assertIn("555", flat)
        self.assertIn("repo-worker-code", flat)
        self.assertIn("deterministic", flat)
        execution = json.loads(flat[flat.index("--execution") + 1])
        self.assertEqual(execution["session_id"], "ses_gone")
        self.assertEqual(execution["fallback_reason"], "launch_failed_attempts_exhausted")
        self.assertFalse(any("bind" in [str(x) for x in a] for a in calls))

    def test_stuck_zero_message_attempts_exhausted_deletes_then_fails(self):
        task = {"id": 777, "status": "claimed", "attempts": 5, "max_attempts": 5,
                "worker_id": "repo-worker-code",
                "lease_until": "2000-01-01T00:00:00Z",
                "progress": {"lane": "opencode", "session_id": "ses_stuck"}}
        session = {"id": "ses_stuck", "time": {"created": 0, "updated": 0}}
        class FakePoll:
            @staticmethod
            def fetch_sessions():
                return [session]
            @staticmethod
            def fetch_active_ids():
                return set()
            @staticmethod
            def is_stuck_bound(row, sess, active_ids, now_ms):
                return True
        recovered, calls = self.run_supervision_exhausted(
            task, session_rc=0, session_body=json.dumps(session), fake_poll=FakePoll)
        self.assertEqual(recovered, [])
        kinds = []
        for a in calls:
            argv_s = [str(x) for x in a]
            if "DELETE" in argv_s:
                kinds.append("delete")
            if "fail" in argv_s:
                kinds.append("fail")
        self.assertEqual(kinds, ["delete", "fail"])

    def test_exhausted_delete_failure_leaves_task_bound(self):
        task = {"id": 777, "status": "claimed", "attempts": 5, "max_attempts": 5,
                "worker_id": "repo-worker-code",
                "lease_until": "2000-01-01T00:00:00Z",
                "progress": {"lane": "opencode", "session_id": "ses_stuck"}}
        session = {"id": "ses_stuck", "time": {"created": 0, "updated": 0}}
        class FakePoll:
            @staticmethod
            def fetch_sessions():
                return [session]
            @staticmethod
            def fetch_active_ids():
                return set()
            @staticmethod
            def is_stuck_bound(row, sess, active_ids, now_ms):
                return True
        recovered, calls = self.run_supervision_exhausted(
            task, session_rc=0, session_body=json.dumps(session), fake_poll=FakePoll, delete_rc=1)
        self.assertEqual(recovered, [])
        self.assertEqual(self.fail_calls(calls), [])




class TestCodexRound5(unittest.TestCase):
    """Regression tests for the three Codex round-5 findings on PR #6990
    (2026-11-08T03:27Z, review of f7277c7817): session absence must be
    positively verified before a terminal fail, the per-ID fallback must
    fail closed when the session list is unavailable (and must not pass
    the v2 envelope to the stuck classifier), and launch metadata must be
    refreshed from the re-verified task. Oracle: prompt.md step 1a/1e and
    the backend claimed_by() guard, as cited in the findings."""

    # ---- supervision: absence verification ----

    def run_supervision(self, task, session_result, fake_poll=None):
        calls = []

        def fake_run(argv, **kwargs):
            calls.append(argv)
            argv_s = [str(a) for a in argv]
            if "list" in argv_s:
                queue = argv_s[argv_s.index("--queue") + 1]
                rows = [task] if queue == "repo-health-code-pilot" else []
                return Result(0, json.dumps({"rows": rows}))
            if "DELETE" in argv_s:
                return Result(0)
            if any("/message" in a for a in argv_s):
                return Result(0, "[]")
            if any("api/session" in a for a in argv_s):
                return session_result
            return Result(0)

        patches = [patch.object(claimant.subprocess, "run", side_effect=fake_run)]
        if fake_poll is not None:
            patches.append(patch.object(claimant, "_load_poll_module",
                                        return_value=fake_poll))
        for pt in patches:
            pt.start()
        try:
            recovered = claimant.supervision_step("live")
        finally:
            for pt in patches:
                pt.stop()
        return recovered, calls

    @staticmethod
    def fail_calls(calls):
        return [[str(x) for x in a] for a in calls if "fail" in [str(x) for x in a]]

    EXHAUSTED = {"id": 555, "status": "claimed", "attempts": 5,
                 "max_attempts": 5, "worker_id": "repo-worker-code",
                 "progress": {"lane": "opencode", "session_id": "ses_gone"}}

    def test_transport_failure_is_unknown_never_terminally_fails(self):
        # P1: the session GET times out (curl rc 28, http_code 000) while
        # the task is on its final attempt. Absence is NOT established:
        # no terminal fail, no delete, binding untouched.
        recovered, calls = self.run_supervision(
            dict(self.EXHAUSTED), Result(28, "\n000"))
        self.assertEqual(recovered, [])
        self.assertEqual(self.fail_calls(calls), [])
        self.assertFalse(any("DELETE" in [str(x) for x in a] for a in calls))

    def test_connection_failure_is_unknown_never_terminally_fails(self):
        recovered, calls = self.run_supervision(
            dict(self.EXHAUSTED), Result(7, "\n000"))
        self.assertEqual(recovered, [])
        self.assertEqual(self.fail_calls(calls), [])

    def test_verified_404_attempts_exhausted_fails_terminally(self):
        # A positively verified absence (HTTP 404) keeps the round-4
        # terminal path: exactly one deterministic fail as the owner.
        recovered, calls = self.run_supervision(
            dict(self.EXHAUSTED), Result(0, "\n404"))
        self.assertEqual(recovered, [])
        fails = self.fail_calls(calls)
        self.assertEqual(len(fails), 1)
        self.assertIn("repo-worker-code", fails[0])
        self.assertIn("deterministic", fails[0])

    def test_http_500_is_unknown_never_terminally_fails(self):
        recovered, calls = self.run_supervision(
            dict(self.EXHAUSTED), Result(0, "server error\n500"))
        self.assertEqual(recovered, [])
        self.assertEqual(self.fail_calls(calls), [])

    # ---- supervision: stuck fallback shape ----

    STUCK_TASK = {"id": 777, "status": "claimed", "attempts": 1,
                  "max_attempts": 5, "lease_until": "2000-01-01T00:00:00Z",
                  "progress": {"lane": "opencode", "session_id": "ses_stuck"}}
    SESSION = {"id": "ses_stuck", "time": {"created": 0, "updated": 0}}

    def test_fallback_fails_closed_when_session_list_unavailable(self):
        # P2: fetch_sessions() failed (None). Even though the per-ID GET
        # succeeds, the stuck classifier must not run on a fallback
        # envelope - supervision leaves the binding untouched.
        seen = {}

        class FakePoll:
            @staticmethod
            def fetch_sessions():
                return None

            @staticmethod
            def fetch_active_ids():
                return set()

            @staticmethod
            def is_stuck_bound(row, sess, active_ids, now_ms):
                seen["sess"] = sess
                return True

        enveloped = Result(0, json.dumps({"data": self.SESSION}) + "\n200")
        recovered, calls = self.run_supervision(
            dict(self.STUCK_TASK), enveloped, fake_poll=FakePoll)
        self.assertEqual(recovered, [])
        self.assertNotIn("sess", seen)
        self.assertFalse(any("DELETE" in [str(x) for x in a] for a in calls))

    def test_fallback_unwraps_envelope_when_list_read_succeeded(self):
        # The list read succeeded (session simply not in it): the
        # fallback passes the UNWRAPPED session - its own time fields -
        # to the classifier, and a stuck verdict reaches recovery.
        seen = {}

        class FakePoll:
            @staticmethod
            def fetch_sessions():
                return []

            @staticmethod
            def fetch_active_ids():
                return set()

            @staticmethod
            def is_stuck_bound(row, sess, active_ids, now_ms):
                seen["sess"] = sess
                return True

        enveloped = Result(0, json.dumps({"data": self.SESSION}) + "\n200")
        recovered, calls = self.run_supervision(
            dict(self.STUCK_TASK), enveloped, fake_poll=FakePoll)
        self.assertEqual(
            recovered,
            [{"task_id": 777, "session_id": "ses_stuck",
              "reason": "zero_message"}])
        self.assertIn("sess", seen)
        self.assertEqual(seen["sess"].get("time"), self.SESSION["time"])
        self.assertNotIn("data", seen["sess"])

    # ---- launch: refresh claim owner after re-verification ----

    def test_launch_refreshes_worker_id_from_verified_task(self):
        # P2: discovery saw worker "old-worker"; by re-verification the
        # task was reclaimed by "new-worker". The launched record and
        # the delivered prompt must carry the NEW identity.
        calls = []

        def fake_run(argv, **kwargs):
            calls.append(argv)
            argv_s = [str(a) for a in argv]
            if "bind" in argv_s:
                return Result(0)
            if any("api/session" in a for a in argv_s):
                return Result(0, json.dumps({
                    "id": "ses_test1",
                    "location": {"directory": str(claimant.PRIMARY_CLONE)}}))
            return Result(0)

        polls = [{"candidates": [candidate(worker_id="old-worker")]},
                 {"candidates": [candidate(worker_id="new-worker")]}]
        with patch.object(claimant, "run_poll", side_effect=polls), \
             patch.object(claimant.subprocess, "run", side_effect=fake_run):
            launched, skips = claimant.launch_step("live", 1)
        self.assertEqual(skips, [])
        self.assertEqual(len(launched), 1)
        self.assertEqual(launched[0]["worker_id"], "new-worker")
        flat = [str(x) for a in calls for x in a]
        prompt_bodies = [x for x in flat if "--worker" in x]
        self.assertTrue(prompt_bodies)
        self.assertTrue(any("new-worker" in x for x in prompt_bodies))
        self.assertFalse(any("old-worker" in x for x in prompt_bodies))


if __name__ == "__main__":
    unittest.main()
