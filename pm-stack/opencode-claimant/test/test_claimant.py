#!/usr/bin/env python3
"""Offline tests for claimant ordering and fail-closed behavior.

Ported to the live claimant API for #7670 (task 14501). The Slice-1
suite at this path patched a claimant API that no longer exists
(prepare_candidate / launch_one / _POLL / _current_bound_row /
_poll-shaped supervision_step(data, mode)) and errored 4/4 while no
workflow ran it. This port pins the surviving behaviors against the
live surface - launch_step(mode, cap_room) and supervision_step(mode):

- bind precedes prompt: a session is created, then bound to the task,
  then prompted (the Slice-1 checkpoint-between-bind-and-prompt pin is
  superseded: the live flow has no checkpoint step; prompt delivery is
  verified by _send_prompt's reconcile-before-resend instead);
- a credential-shaped packet value never enters the prompt text (the
  live claimant redacts credential-shaped keys when it dumps the
  packet into the prompt);
- a held issue label (held / user/owned / do-not-merge /
  do-not-auto-edit) prevents launch, and an unreadable label set
  fails closed the same way - this gate was genuinely missing in the
  live claimant and is implemented in launch_step for this issue;
- session-create failure binds nothing and is a recorded skip;
- supervision: a confirmed zero-message read precedes the session
  DELETE, which precedes the atomic recovery bind carrying
  LAUNCHED_BY_RECOVERY_ZERO_MESSAGE.

The Slice-1 per-packet route/model pin is superseded: the live
claimant launches every session on the fixed workhorse route.
"""

import copy
import importlib.util
import json
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("claimant", ROOT / "claimant.py")
claimant = importlib.util.module_from_spec(spec)
spec.loader.exec_module(claimant)


class Result:
    """Minimal subprocess.run stand-in."""

    def __init__(self, returncode=0, stdout="", stderr=""):
        self.returncode = returncode
        self.stdout = stdout
        self.stderr = stderr


def make_fake_run(events, *, create_rc=0, messages="[]"):
    """subprocess.run stub dispatching on argv shape, recording an
    ordered event log: ("create",) / ("bind", argv) / ("prompt", argv)
    / ("messages",) / ("delete",) / ("session_get",).

    Stub-shape audit (#7772, #7770 Phase 4): every curl reply mirrors
    the writer it stands in for — the claimant's one transport
    (`_server_request`) invokes curl with `-w "\\n%{http_code}"`, so a
    real reply is the body plus a trailing status line, and curl -s
    exits 0 even on a 401/404. This stub therefore appends the status
    line to every curl body and answers rc 0 for HTTP errors; only a
    transport-level create failure uses create_rc. (The pre-audit stub
    returned bare bodies with no status line, which the verified
    transport correctly read as UNKNOWN — the reader's-assumption
    shape #7760 classed as a defect.)"""

    def reply(body, status):
        return Result(0, body + "\n" + str(status))

    def fake_run(cmd, **kwargs):
        argv = [str(part) for part in cmd]
        joined = " ".join(argv)
        if "repo-queue" in argv[0] and "bind" in argv:
            events.append(("bind", argv))
            return Result(0, "{}")
        if argv[0] == "curl" and "-X" in argv and "DELETE" in argv:
            events.append(("delete",))
            return reply("", 200)
        if argv[0] == "curl" and joined.rstrip().endswith("/message"):
            events.append(("messages",))
            return reply(messages, 200)
        if argv[0] == "curl" and joined.rstrip().endswith("/prompt"):
            events.append(("prompt", argv))
            return reply("", 200)
        if argv[0] == "curl" and "-X" in argv and "POST" in argv:
            events.append(("create",))
            if create_rc != 0:
                return Result(create_rc, "", "server unavailable")
            return reply(
                json.dumps(
                    {
                        "id": "ses_new1",
                        "location": {"directory": str(claimant.PRIMARY_CLONE)},
                    }
                ),
                200,
            )
        if argv[0] == "curl" and "/api/session/" in joined:
            events.append(("session_get",))
            # The v2 server's shape for a missing session: a 404 with
            # an error body (the "gone" verdict's only lawful source).
            return reply(json.dumps({"error": "session not found"}), 404)
        return Result(0, "")

    return fake_run


CANDIDATE = {
    "id": 73001,
    "queue": "repo-health-code-pilot",
    "status": "claimed",
    "priority": 1,
    "worker_id": "repo-worker-code",
    "lane_id": "code-a",
    "lease_until": "2999-01-01T00:00:00Z",
    "lane": "opencode",
    "stage": "handed_off",
    "session_id": None,
    "issue": 3026,
    "issue_number": 3026,
    "packet": {
        "repo": "developing-today/code",
        "issue": {"number": 3026},
        "route": {"providerID": "opencode-route", "modelID": "summit"},
        "password": "must-not-enter-prompt",
        "nested": {"api_token": "must-not-enter-prompt-either"},
    },
    "attempts": 0,
    "max_attempts": 3,
    "title": "Queue task 73001",
    "dedupe_key": "test",
}


def run_launch(events, *, labels=set(), create_rc=0):
    with (
        patch.object(
            claimant,
            "run_poll",
            side_effect=lambda: {"candidates": [copy.deepcopy(CANDIDATE)]},
        ),
        patch.object(claimant, "_github_issue_labels", return_value=set(labels)),
        patch.object(
            claimant.subprocess, "run", make_fake_run(events, create_rc=create_rc)
        ),
    ):
        return claimant.launch_step("live", 3)


class TestLaunch(unittest.TestCase):
    def test_session_is_bound_before_prompt_and_password_stays_out(self):
        events = []
        launched, skips = run_launch(events)
        self.assertEqual(skips, [])
        self.assertEqual(len(launched), 1)
        self.assertEqual(launched[0]["session_id"], "ses_new1")
        kinds = [e[0] for e in events]
        self.assertLess(kinds.index("create"), kinds.index("bind"))
        self.assertLess(kinds.index("bind"), kinds.index("prompt"))
        prompt_argv = next(e[1] for e in events if e[0] == "prompt")
        body = json.loads(next(a for a in prompt_argv if a.startswith("{")))
        prompt = body["text"]
        self.assertIn("73001", prompt)
        self.assertIn("repo-worker-code", prompt)
        self.assertIn("ses_new1", prompt)
        self.assertNotIn("must-not-enter-prompt", prompt)
        self.assertIn("[redacted]", prompt)

    def test_create_failure_binds_nothing(self):
        events = []
        launched, skips = run_launch(events, create_rc=1)
        self.assertEqual(launched, [])
        self.assertEqual(len(skips), 1)
        self.assertEqual(skips[0]["task_id"], 73001)
        self.assertTrue(skips[0]["reason"].startswith("launch_error"))
        self.assertNotIn("bind", [e[0] for e in events])

    def test_held_issue_label_prevents_launch(self):
        events = []
        launched, skips = run_launch(events, labels={"do-not-merge"})
        self.assertEqual(launched, [])
        self.assertEqual(len(skips), 1)
        self.assertTrue(skips[0]["reason"].startswith("held_issue_labels"))
        self.assertIn("do-not-merge", skips[0]["reason"])
        self.assertNotIn("create", [e[0] for e in events])
        self.assertNotIn("bind", [e[0] for e in events])

    def test_unreadable_labels_fail_closed(self):
        events = []
        with (
            patch.object(
                claimant,
                "run_poll",
                side_effect=lambda: {"candidates": [copy.deepcopy(CANDIDATE)]},
            ),
            patch.object(
                claimant, "_github_issue_labels", side_effect=RuntimeError("gh down")
            ),
            patch.object(claimant.subprocess, "run", make_fake_run(events)),
        ):
            launched, skips = claimant.launch_step("live", 3)
        self.assertEqual(launched, [])
        self.assertEqual(len(skips), 1)
        self.assertTrue(skips[0]["reason"].startswith("held_label_check_failed"))
        self.assertNotIn("create", [e[0] for e in events])


BOUND_ROW = {
    "id": 4242,
    "attempts": 0,
    "max_attempts": 5,
    "lease_until": "2020-01-01T00:00:00Z",
    "progress": {"lane": "opencode", "session_id": "ses_gone"},
}


class TestSupervision(unittest.TestCase):
    def test_zero_message_delete_precedes_atomic_recovery(self):
        events = []
        poll_mod = claimant._load_poll_module()

        def fake_fetch(queue):
            return [dict(BOUND_ROW)] if queue == claimant.QUEUES[0] else []

        with (
            patch.object(poll_mod, "fetch_claimed_rows", side_effect=fake_fetch),
            patch.object(claimant.subprocess, "run", make_fake_run(events)),
        ):
            recovered = claimant.supervision_step("live")

        self.assertEqual(
            recovered,
            [
                {
                    "task_id": 4242,
                    "session_id": "ses_gone",
                    "reason": "zero_message",
                }
            ],
        )
        kinds = [e[0] for e in events]
        self.assertLess(kinds.index("messages"), kinds.index("delete"))
        self.assertLess(kinds.index("delete"), kinds.index("bind"))
        bind_argv = next(e[1] for e in events if e[0] == "bind")
        self.assertIn(claimant.LAUNCHED_BY_RECOVERY_ZERO_MESSAGE, bind_argv)


class ReadModeLoudDefaultTests(unittest.TestCase):
    """#10141: read_mode() keeps shadow as the fail-safe default for a
    missing/unreadable/invalid mode file, but logs a loud
    CLAIMANT-MODE-WARNING line on every such read — the silence was
    the bug, not the default. Oracle: captured stderr."""

    def _read_with_file(self, tmp: Path, content):
        mode_file = tmp / "mode"
        if content is not None:
            mode_file.write_text(content)
        import io
        from contextlib import redirect_stderr

        buf = io.StringIO()
        with patch.object(claimant, "MODE_FILE", mode_file), redirect_stderr(buf):
            result = claimant.read_mode()
        return result, buf.getvalue()

    def test_missing_file_defaults_shadow_and_warns_loudly(self):
        import tempfile

        with tempfile.TemporaryDirectory() as td:
            result, err = self._read_with_file(Path(td), None)
        self.assertEqual(result, "shadow")
        self.assertIn("CLAIMANT-MODE-WARNING", err)
        self.assertIn("MISSING", err)

    def test_invalid_content_defaults_shadow_and_warns_loudly(self):
        import tempfile

        with tempfile.TemporaryDirectory() as td:
            result, err = self._read_with_file(Path(td), "banana\n")
        self.assertEqual(result, "shadow")
        self.assertIn("CLAIMANT-MODE-WARNING", err)

    def test_valid_modes_are_returned_quietly(self):
        import tempfile

        with tempfile.TemporaryDirectory() as td:
            for content, want in (
                ("live\n", "live"),
                ("shadow", "shadow"),
                (" LIVE ", "live"),
            ):
                result, err = self._read_with_file(Path(td), content)
                self.assertEqual(result, want)
                self.assertNotIn("CLAIMANT-MODE-WARNING", err)

    def test_every_read_records_where_the_mode_came_from(self):
        # #10332: a flip must be attributable from the fires log alone.
        import tempfile

        with tempfile.TemporaryDirectory() as td:
            tmp = Path(td)
            for content, state in (
                ("live\n", "ok"),
                ("banana", "invalid"),
                (None, "missing"),
            ):
                (tmp / "mode").unlink(missing_ok=True)
                self._read_with_file(tmp, content)
                source = claimant._mode_source
                self.assertEqual(source["path"], str(tmp / "mode"))
                self.assertEqual(source["state"], state)
                self.assertIsNotNone(source["dir_mtime"])
                if content is None:
                    self.assertIsNone(source["file_mtime"])
                else:
                    self.assertIsNotNone(source["file_mtime"])

    def test_fire_log_entries_carry_the_mode_source(self):
        import tempfile

        with tempfile.TemporaryDirectory() as td:
            tmp = Path(td)
            self._read_with_file(tmp, None)
            fires = tmp / "fires.jsonl"
            with (
                patch.object(claimant, "STATE_DIR", tmp),
                patch.object(claimant, "FIRES_FILE", fires),
            ):
                claimant.log_fire("shadow", [], [], None, [{"reason": "no_candidates"}])
            entry = json.loads(fires.read_text().splitlines()[-1])
        self.assertEqual(entry["mode"], "shadow")
        self.assertEqual(entry["mode_source"]["state"], "missing")
        self.assertEqual(entry["mode_source"]["path"], str(tmp / "mode"))


def make_hand_back_run(events, *, claim_fail_ids=frozenset(), park_rc=0, note_rc=0):
    """subprocess.run stub for the saturated-fire hand-back (#1187):
    repo-queue claim --task / park, and gh issue comment. Records an
    ordered event log: ("claim", argv) / ("park", argv) / ("note", argv).
    A claim answers the claimed row ({"id": <task>}); a refused claim
    answers rc 1, mirroring the CLI's no-claimable-task path."""

    def fake_run(cmd, **kwargs):
        argv = [str(part) for part in cmd]
        if "repo-queue" in argv[0] and len(argv) > 1 and argv[1] == "claim":
            task_id = int(argv[argv.index("--task") + 1])
            events.append(("claim", argv))
            if task_id in claim_fail_ids:
                return Result(1, "", "no claimable task")
            return Result(0, json.dumps({"id": task_id}))
        if "repo-queue" in argv[0] and len(argv) > 1 and argv[1] == "park":
            events.append(("park", argv))
            if park_rc != 0:
                return Result(park_rc, "", "park refused")
            return Result(0, "{}")
        if argv[0] == "gh" and argv[1:3] == ["issue", "comment"]:
            events.append(("note", argv))
            return Result(note_rc, "", "")
        return Result(0, "")

    return fake_run


def second_candidate():
    c = copy.deepcopy(CANDIDATE)
    c["id"] = 73002
    c["issue"] = 3027
    c["issue_number"] = 3027
    c["packet"] = copy.deepcopy(CANDIDATE["packet"])
    c["packet"]["issue"] = {"number": 3027}
    c["title"] = "Queue task 73002"
    return c


def run_hand_back(events, candidates, cap_room=0, mode="live", **stub_kwargs):
    with patch.object(
        claimant.subprocess, "run", make_hand_back_run(events, **stub_kwargs)
    ):
        return claimant.hand_back_next_candidate(mode, candidates, cap_room)


class TestHandBack(unittest.TestCase):
    """#1187 criterion 2: a saturated claimant fire hands back the
    next candidate (parked, executor-saturated, issue noted) within
    one round, instead of logging cap_saturated and exiting."""

    def test_live_fire_hands_back_first_candidate(self):
        events = []
        cands = [copy.deepcopy(CANDIDATE), second_candidate()]
        skips = run_hand_back(events, cands)
        kinds = [e[0] for e in events]
        self.assertEqual(kinds, ["claim", "park", "note"])
        claim_argv = next(e[1] for e in events if e[0] == "claim")
        self.assertIn("73001", claim_argv)
        self.assertIn("--task", claim_argv)
        self.assertIn(claimant.CLAIMANT_WORKER, claim_argv)
        self.assertIn("openchamber", claim_argv)
        park_argv = next(e[1] for e in events if e[0] == "park")
        self.assertIn("73001", park_argv)
        reason = park_argv[park_argv.index("--reason") + 1]
        self.assertTrue(reason.startswith("executor-saturated"))
        self.assertIn("36", reason)
        self.assertEqual(
            park_argv[park_argv.index("--delay-seconds") + 1],
            str(claimant.HAND_BACK_DELAY_SECONDS),
        )
        self.assertEqual(
            json.loads(park_argv[park_argv.index("--ready-condition") + 1]),
            {"kind": "capacity"},
        )
        note_argv = next(e[1] for e in events if e[0] == "note")
        self.assertEqual(note_argv[1:5], ["issue", "comment", "3026", "--repo"])
        self.assertIn("developing-today/code", note_argv)
        body = note_argv[note_argv.index("--body") + 1]
        self.assertIn("executor-saturated", body)
        self.assertIn("73001", body)
        self.assertIn("36/36", body)
        self.assertEqual(len(skips), 2)
        self.assertEqual(skips[0]["task_id"], 73001)
        self.assertEqual(skips[0]["reason"], "handed_back_executor_saturated")
        self.assertTrue(skips[0]["note_posted"])
        self.assertEqual(skips[1], {"task_id": 73002, "reason": "cap_saturated"})

    def test_claim_failure_tries_next_candidate(self):
        events = []
        cands = [copy.deepcopy(CANDIDATE), second_candidate()]
        skips = run_hand_back(events, cands, claim_fail_ids={73001})
        kinds = [e[0] for e in events]
        self.assertEqual(kinds, ["claim", "claim", "park", "note"])
        self.assertEqual(skips[0]["task_id"], 73001)
        self.assertEqual(skips[0]["reason"], "hand_back_claim_failed")
        self.assertEqual(skips[1]["task_id"], 73002)
        self.assertEqual(skips[1]["reason"], "handed_back_executor_saturated")

    def test_shadow_fire_mutates_nothing(self):
        events = []
        cands = [copy.deepcopy(CANDIDATE), second_candidate()]
        skips = run_hand_back(events, cands, mode="shadow")
        self.assertEqual(events, [])
        self.assertEqual(skips[0]["reason"], "shadow_would_hand_back")
        self.assertEqual(skips[0]["task_id"], 73001)
        self.assertEqual(skips[1], {"task_id": 73002, "reason": "cap_saturated"})

    def test_park_failure_stops_and_names_the_stranded_claim(self):
        events = []
        cands = [copy.deepcopy(CANDIDATE), second_candidate()]
        skips = run_hand_back(events, cands, park_rc=1)
        self.assertEqual([e[0] for e in events], ["claim", "park"])
        self.assertEqual(skips[0]["reason"], "hand_back_park_failed")
        self.assertTrue(skips[0]["claim_held"])
        self.assertEqual(skips[1], {"task_id": 73002, "reason": "cap_saturated"})

    def test_note_failure_still_hands_back(self):
        events = []
        cands = [copy.deepcopy(CANDIDATE)]
        skips = run_hand_back(events, cands, note_rc=1)
        self.assertEqual([e[0] for e in events], ["claim", "park", "note"])
        self.assertEqual(skips[0]["reason"], "handed_back_executor_saturated")
        self.assertFalse(skips[0]["note_posted"])

    def test_no_candidates_logs_plain_saturation(self):
        events = []
        skips = run_hand_back(events, [])
        self.assertEqual(events, [])
        self.assertEqual(skips, [{"reason": "cap_saturated"}])

    def test_recovered_candidate_is_not_handed_back(self):
        events = []
        cands = [copy.deepcopy(CANDIDATE), second_candidate()]
        with patch.object(
            claimant.subprocess, "run", make_hand_back_run(events)
        ):
            skips = claimant.hand_back_next_candidate(
                "live", cands, 0, exclude_task_ids=[73001]
            )
        self.assertEqual(
            skips[0],
            {"task_id": 73001, "reason": "recovered_this_fire_next_fire_only"},
        )
        self.assertEqual(skips[1]["reason"], "handed_back_executor_saturated")
        self.assertNotIn("73001", [str(e[1]) for e in events if e[0] == "claim"])

    def test_mid_work_candidate_is_never_taken(self):
        events = []
        mid_work = copy.deepcopy(CANDIDATE)
        mid_work["stage"] = "claimed"
        skips = run_hand_back(events, [mid_work, second_candidate()])
        self.assertEqual([e[0] for e in events], ["claim", "park", "note"])
        claim_argv = next(e[1] for e in events if e[0] == "claim")
        self.assertIn("73002", claim_argv)
        self.assertEqual(len(skips), 1)
        self.assertEqual(skips[0]["reason"], "handed_back_executor_saturated")


if __name__ == "__main__":
    unittest.main()
