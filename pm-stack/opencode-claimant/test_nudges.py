#!/usr/bin/env python3
"""Tests for nudges.py's stall nudges + escalation (#1187).

Run:  python3 -m unittest pm-stack/opencode-claimant/test_nudges.py

The oracle is #1338's nudge rule text, carried into #1187 as its
next slice:

- nudge at most twice per stall episode (message injection
  through the verified transport), then escalate to the fleet as
  a stuck-holder case;
- only CONFIRMED stalls actuate (babysitter.py's verdict +
  confirmed flag); a suspect is data, not a trigger;
- unbound sessions are observed, never driven (babysitter.py's
  rule): a confirmed unbound stall is recorded, nothing is sent.

Plus the claimant's delivery discipline:

- a nudge counts only when the sender confirms delivery; a
  failed send consumes no budget and retries next round;
- escalation is the stuck-holder note on the bound issue: a
  failed post does not mark the episode escalated (it retries),
  and an unaddressable binding (no repo/issue on the row) is
  recorded unaddressed once - a repo is never guessed;
- episodes close when the session stops reading confirmed-
  stalled: the state entry drops, and a later stall starts a
  fresh episode with a fresh budget;
- shadow fires plan and mutate nothing; an absent or errored
  round actuates nothing and touches no state.

All effects are injected fakes and state lives in a temp dir, so
the suite is hermetic: no server, no queue, no network.

Negative control: this suite fails against origin/main, which
has no nudges.py and no nudge step in the claimant fire (the
actuation surface does not exist there).
"""

import importlib.util
import json
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

_HERE = Path(__file__).resolve().parent


def _load(name, filename):
    spec = importlib.util.spec_from_file_location(name, _HERE / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


nudges = _load("nudges", "nudges.py")
claimant = _load("claimant_for_nudges", "claimant.py")


def _fake_git_run(argv, **kwargs):
    """Fake git at the subprocess boundary for fire-log tests.

    log_fire() records read_code_source()'s provenance read in every
    entry; that read spawns git, which the hermetic suite forbids.
    Fire-log tests whose subject is another entry field fake the read
    here (the TestCodeSourceFireLog shape, #13254) so the entry still
    carries a real-shaped code_source and no spawn is attempted.
    """
    stdout = (
        "main\n"
        if "--abbrev-ref" in argv
        else "0123456789abcdef0123456789abcdef01234567\n"
    )
    return subprocess.CompletedProcess(argv, 0, stdout=stdout)


def record(round_no=3, verdicts=None):
    return {"round": round_no, "verdicts": verdicts or {}, "changes": []}


def stalled(task_id=101, confirmed=True):
    entry = {
        "verdict": "stalled",
        "task_id": task_id,
        "pool": "opencode-route/workhorse",
    }
    if confirmed:
        entry["confirmed"] = True
    return entry


def error_stopped(task_id=101, binding=None):
    return {
        "verdict": "error-stopped",
        "outcome": "error",
        "retryable": True,
        "task_id": task_id,
        "binding": binding
        or {
            "task_id": task_id,
            "queue": "repo-health-code-pilot",
            "issue_number": 4242,
            "repo": "developing-today/code",
        },
    }


class FakeEffects:
    """The injected effect boundary: records every call, answers
    from the configured result maps."""

    def __init__(self, send_ok=True, note_ok=True):
        self.send_ok = send_ok
        self.note_ok = note_ok
        self.sent = []  # (session_id, text)
        self.notes = []  # kwargs dicts

    def send_prompt(self, session_id, text):
        self.sent.append((session_id, text))
        return self.send_ok

    def post_note(self, **kw):
        self.notes.append(kw)
        return self.note_ok


BINDINGS = {
    "ses_stuck": {
        "task_id": 101,
        "queue": "repo-health-code-pilot",
        "issue_number": 4242,
        "repo": "developing-today/code",
    }
}


class TestPlan(unittest.TestCase):
    def test_first_confirmed_stall_plans_first_nudge(self):
        actions = nudges.plan_actions(record(verdicts={"ses_stuck": stalled()}), {})
        self.assertEqual(
            actions,
            [
                {
                    "session_id": "ses_stuck",
                    "kind": "nudge",
                    "task_id": 101,
                    "nudges_sent": 0,
                }
            ],
        )

    def test_suspect_stall_plans_nothing(self):
        actions = nudges.plan_actions(
            record(verdicts={"ses_stuck": stalled(confirmed=False)}), {}
        )
        self.assertEqual(actions, [])

    def test_error_stopped_bound_session_plans_one_retry_with_binding(self):
        actions = nudges.plan_actions(
            record(verdicts={"ses_failed": error_stopped()}), {}
        )
        self.assertEqual(actions[0]["kind"], "retry")
        self.assertEqual(actions[0]["task_id"], 101)
        self.assertEqual(actions[0]["binding"]["repo"], "developing-today/code")

    def test_error_retry_is_not_replanned_after_success(self):
        prior = {"ses_failed": {"error_retried": True}}
        actions = nudges.plan_actions(
            record(verdicts={"ses_failed": error_stopped()}), prior
        )
        self.assertEqual(actions, [])

    def test_unbound_confirmed_stall_is_observed_only(self):
        actions = nudges.plan_actions(
            record(verdicts={"ses_free": stalled(task_id=None)}), {}
        )
        self.assertEqual(
            actions,
            [{"session_id": "ses_free", "kind": "observed", "task_id": None}],
        )

    def test_spent_budget_plans_escalation(self):
        prior = {"ses_stuck": {"nudges_sent": 2, "escalated": False}}
        actions = nudges.plan_actions(record(verdicts={"ses_stuck": stalled()}), prior)
        self.assertEqual(len(actions), 1)
        self.assertEqual(actions[0]["kind"], "escalate")

    def test_escalated_episode_plans_nothing(self):
        prior = {"ses_stuck": {"nudges_sent": 2, "escalated": True}}
        actions = nudges.plan_actions(record(verdicts={"ses_stuck": stalled()}), prior)
        self.assertEqual(actions, [])

    def test_error_round_plans_nothing(self):
        self.assertEqual(nudges.plan_actions({"error": "sessions_unreadable"}, {}), [])
        self.assertEqual(nudges.plan_actions(None, {}), [])


class TestNudgeStep(unittest.TestCase):
    def _step(self, fx, rec, state_dir, mode="live", bindings=None, recover_error=None):
        return nudges.nudge_step(
            mode,
            rec,
            send_prompt=fx.send_prompt,
            post_note=fx.post_note,
            recover_error=recover_error,
            bindings=BINDINGS if bindings is None else bindings,
            state_dir=state_dir,
        )

    def test_live_error_retry_calls_recovery_and_logs_binding(self):
        fx = FakeEffects()
        calls = []

        def recover_error(session_id, task_id, binding):
            calls.append((session_id, task_id, binding))
            return True

        with tempfile.TemporaryDirectory() as tmp:
            out = self._step(
                fx,
                record(verdicts={"ses_failed": error_stopped()}),
                Path(tmp),
                recover_error=recover_error,
            )
        self.assertEqual(calls[0][0:2], ("ses_failed", 101))
        self.assertEqual(calls[0][2]["repo"], "developing-today/code")
        self.assertEqual(out["actions"][0]["kind"], "retry")
        self.assertTrue(out["actions"][0]["applied"])
        self.assertEqual(out["actions"][0]["detail"], "requeued_retry")

    def _state(self, state_dir):
        return json.loads((state_dir / nudges.STATE_FILE_NAME).read_text())

    def test_live_first_round_sends_one_nudge_and_persists(self):
        fx = FakeEffects()
        with tempfile.TemporaryDirectory() as tmp:
            state_dir = Path(tmp)
            out = self._step(fx, record(verdicts={"ses_stuck": stalled()}), state_dir)
            self.assertEqual(len(fx.sent), 1)
            self.assertEqual(fx.sent[0][0], "ses_stuck")
            self.assertEqual(fx.sent[0][1], nudges.NUDGE_TEXT)
            self.assertEqual(fx.notes, [])
            self.assertEqual(out["actions"][0]["detail"], "delivered")
            state = self._state(state_dir)
            self.assertEqual(state["sessions"]["ses_stuck"]["nudges_sent"], 1)
            self.assertEqual(state["sessions"]["ses_stuck"]["escalated"], False)

    def test_episode_walks_nudge_nudge_escalate_then_stops(self):
        fx = FakeEffects()
        with tempfile.TemporaryDirectory() as tmp:
            state_dir = Path(tmp)
            rec = record(verdicts={"ses_stuck": stalled()})
            self._step(fx, rec, state_dir)
            self._step(fx, rec, state_dir)
            out3 = self._step(fx, rec, state_dir)
            self.assertEqual(len(fx.sent), 2)
            self.assertEqual(len(fx.notes), 1)
            note = fx.notes[0]
            self.assertEqual(note["task_id"], 101)
            self.assertEqual(note["session_id"], "ses_stuck")
            self.assertEqual(note["repo"], "developing-today/code")
            self.assertEqual(note["issue_number"], 4242)
            self.assertEqual(note["nudges_sent"], 2)
            self.assertEqual(out3["actions"][0]["detail"], "noted")
            out4 = self._step(fx, rec, state_dir)
            self.assertEqual(out4["actions"], [])
            self.assertEqual(len(fx.sent), 2)
            self.assertEqual(len(fx.notes), 1)

    def test_failed_send_consumes_no_budget(self):
        fx = FakeEffects(send_ok=False)
        with tempfile.TemporaryDirectory() as tmp:
            state_dir = Path(tmp)
            out = self._step(fx, record(verdicts={"ses_stuck": stalled()}), state_dir)
            self.assertEqual(out["actions"][0]["applied"], False)
            self.assertEqual(out["actions"][0]["detail"], "send_failed")
            self.assertEqual(
                self._state(state_dir)["sessions"]["ses_stuck"]["nudges_sent"], 0
            )

    def test_failed_note_retries_escalation_next_round(self):
        fx = FakeEffects(note_ok=False)
        with tempfile.TemporaryDirectory() as tmp:
            state_dir = Path(tmp)
            (state_dir / nudges.STATE_FILE_NAME).write_text(
                json.dumps(
                    {
                        "sessions": {
                            "ses_stuck": {
                                "task_id": 101,
                                "nudges_sent": 2,
                                "escalated": False,
                                "first_round": 1,
                                "last_round": 2,
                            }
                        }
                    }
                )
            )
            out = self._step(fx, record(verdicts={"ses_stuck": stalled()}), state_dir)
            self.assertEqual(out["actions"][0]["detail"], "note_failed")
            self.assertEqual(
                self._state(state_dir)["sessions"]["ses_stuck"]["escalated"], False
            )
            fx.note_ok = True
            out = self._step(fx, record(verdicts={"ses_stuck": stalled()}), state_dir)
            self.assertEqual(out["actions"][0]["detail"], "noted")
            self.assertEqual(len(fx.notes), 2)

    def test_unaddressed_escalation_recorded_once(self):
        fx = FakeEffects()
        with tempfile.TemporaryDirectory() as tmp:
            state_dir = Path(tmp)
            (state_dir / nudges.STATE_FILE_NAME).write_text(
                json.dumps(
                    {
                        "sessions": {
                            "ses_stuck": {
                                "task_id": 101,
                                "nudges_sent": 2,
                                "escalated": False,
                                "first_round": 1,
                                "last_round": 2,
                            }
                        }
                    }
                )
            )
            bindings = {
                "ses_stuck": {"task_id": 101, "issue_number": None, "repo": None}
            }
            out = self._step(
                fx,
                record(verdicts={"ses_stuck": stalled()}),
                state_dir,
                bindings=bindings,
            )
            self.assertEqual(out["actions"][0]["detail"], "no_issue_address")
            self.assertEqual(fx.notes, [])
            out = self._step(
                fx,
                record(verdicts={"ses_stuck": stalled()}),
                state_dir,
                bindings=bindings,
            )
            self.assertEqual(out["actions"], [])

    def test_episode_close_drops_state_and_resets_budget(self):
        fx = FakeEffects()
        with tempfile.TemporaryDirectory() as tmp:
            state_dir = Path(tmp)
            self._step(fx, record(verdicts={"ses_stuck": stalled()}), state_dir)
            # A round where the session reads progressing closes
            # the episode: the entry drops out of the state.
            self._step(
                fx,
                record(
                    round_no=4,
                    verdicts={"ses_stuck": {"verdict": "progressing", "task_id": 101}},
                ),
                state_dir,
            )
            self.assertEqual(self._state(state_dir)["sessions"], {})
            # A later stall is a new episode with a fresh budget:
            # the plan starts from zero again, and the delivered
            # nudge is recorded as the new episode's first.
            planned = nudges.plan_actions(
                record(round_no=5, verdicts={"ses_stuck": stalled()}),
                self._state(state_dir)["sessions"],
            )
            self.assertEqual(planned[0]["nudges_sent"], 0)
            out = self._step(
                fx, record(round_no=5, verdicts={"ses_stuck": stalled()}), state_dir
            )
            self.assertEqual(out["actions"][0]["kind"], "nudge")
            self.assertEqual(out["actions"][0]["detail"], "delivered")
            self.assertEqual(out["actions"][0]["nudges_sent"], 1)

    def test_shadow_plans_and_mutates_nothing(self):
        fx = FakeEffects()
        with tempfile.TemporaryDirectory() as tmp:
            state_dir = Path(tmp)
            out = self._step(
                fx, record(verdicts={"ses_stuck": stalled()}), state_dir, mode="shadow"
            )
            self.assertEqual(len(out["actions"]), 1)
            self.assertEqual(out["actions"][0]["applied"], False)
            self.assertEqual(fx.sent, [])
            self.assertEqual(fx.notes, [])
            self.assertFalse((state_dir / nudges.STATE_FILE_NAME).exists())

    def test_error_round_skips_and_touches_no_state(self):
        fx = FakeEffects()
        with tempfile.TemporaryDirectory() as tmp:
            state_dir = Path(tmp)
            out = self._step(fx, {"error": "sessions_unreadable"}, state_dir)
            self.assertEqual(out["skipped"], "no_round")
            self.assertEqual(fx.sent, [])
            self.assertFalse((state_dir / nudges.STATE_FILE_NAME).exists())


class TestFireWiring(unittest.TestCase):
    """The claimant fire runs the nudge step on the round it just
    computed and the fire log carries the record under its
    `nudges` key (the babysitter / hand_back precedent). Oracle:
    the fires.jsonl entry."""

    def test_fire_log_carries_nudges_record(self):
        sentinel = {"round": 7, "actions": []}

        class FakeBabysitter:
            @staticmethod
            def babysitter_step(mode):
                return {"round": 7, "verdicts": {}, "changes": [], "mode": mode}

        class FakeNudges:
            @staticmethod
            def nudge_step(mode, record, **kw):
                return dict(sentinel, mode=mode)

        with tempfile.TemporaryDirectory() as tmp:
            state_dir = Path(tmp)
            fires = state_dir / "fires.jsonl"
            with (
                patch.object(claimant, "read_mode", return_value="shadow"),
                patch.object(claimant, "run_poll", return_value={"candidates": []}),
                patch.object(
                    claimant, "run_queue_health", return_value="health-test-line"
                ),
                patch.object(
                    claimant, "_load_babysitter_module", return_value=FakeBabysitter
                ),
                patch.object(claimant, "_load_nudges_module", return_value=FakeNudges),
                patch.object(claimant, "STATE_DIR", state_dir),
                patch.object(claimant, "FIRES_FILE", fires),
                patch.object(claimant.subprocess, "run", _fake_git_run),
            ):
                rc = claimant.main()
            self.assertEqual(rc, 0)
            entries = [
                json.loads(line) for line in fires.read_text().splitlines() if line
            ]
        self.assertEqual(len(entries), 1)
        self.assertEqual(entries[0]["nudges"], dict(sentinel, mode="shadow"))


class TestInterventionAccounting(unittest.TestCase):
    """#1656 AC4: keeper state separates queue-session
    interventions from the unbound (observed-only) class, and a
    burst of queue interventions past the per-round cap is
    overflow - reported in the round record and deferred, never
    dropped."""

    def _step(self, fx, rec, state_dir, mode="live", bindings=None):
        return nudges.nudge_step(
            mode,
            rec,
            send_prompt=fx.send_prompt,
            post_note=fx.post_note,
            bindings=BINDINGS if bindings is None else bindings,
            state_dir=state_dir,
        )

    def _state(self, state_dir):
        return json.loads((state_dir / nudges.STATE_FILE_NAME).read_text())

    def test_counts_separate_queue_from_other_and_persist(self):
        fx = FakeEffects()
        with tempfile.TemporaryDirectory() as tmp:
            state_dir = Path(tmp)
            rec = record(
                verdicts={
                    "ses_stuck": stalled(task_id=101),
                    "ses_free": stalled(task_id=None),
                }
            )
            out = self._step(fx, rec, state_dir)
            self.assertEqual(
                out["interventions"]["queue"],
                {"retries": 0, "nudges": 1, "escalations": 0},
            )
            self.assertEqual(out["interventions"]["other"], {"observed": 1})
            self.assertEqual(
                out["interventions"]["cumulative"],
                {
                    "queue": {"retries": 0, "nudges": 1, "escalations": 0},
                    "other": {"observed": 1},
                },
            )
            state = self._state(state_dir)
            self.assertEqual(state["counts"], out["interventions"]["cumulative"])

    def test_failed_send_is_not_counted_as_an_intervention(self):
        fx = FakeEffects(send_ok=False)
        with tempfile.TemporaryDirectory() as tmp:
            state_dir = Path(tmp)
            out = self._step(fx, record(verdicts={"ses_stuck": stalled()}), state_dir)
            self.assertEqual(out["interventions"]["queue"]["nudges"], 0)
            self.assertEqual(self._state(state_dir)["counts"]["queue"]["nudges"], 0)

    def test_counts_accumulate_across_rounds_and_episode_close(self):
        fx = FakeEffects()
        with tempfile.TemporaryDirectory() as tmp:
            state_dir = Path(tmp)
            rec = record(verdicts={"ses_stuck": stalled()})
            self._step(fx, rec, state_dir)
            self._step(fx, rec, state_dir)
            out3 = self._step(fx, rec, state_dir)  # escalates
            self.assertEqual(
                out3["interventions"]["cumulative"]["queue"],
                {"retries": 0, "nudges": 2, "escalations": 1},
            )
            # Episode closes (session no longer stalled); the
            # counts stay - they are run accounting, not episode
            # state.
            self._step(fx, record(verdicts={}), state_dir)
            state = self._state(state_dir)
            self.assertEqual(state["sessions"], {})
            self.assertEqual(
                state["counts"]["queue"],
                {"retries": 0, "nudges": 2, "escalations": 1},
            )

    def test_queue_burst_overflows_and_defers_not_drops(self):
        fx = FakeEffects()
        cap = nudges.MAX_QUEUE_INTERVENTIONS_PER_ROUND
        sids = [f"ses_{i:02d}" for i in range(cap + 1)]
        verdicts = {sid: stalled(task_id=100 + i) for i, sid in enumerate(sids)}
        bindings = {
            sid: {
                "task_id": 100 + i,
                "queue": "repo-health-code-pilot",
                "issue_number": 4242,
                "repo": "developing-today/code",
            }
            for i, sid in enumerate(sids)
        }
        with tempfile.TemporaryDirectory() as tmp:
            state_dir = Path(tmp)
            out = self._step(
                fx,
                record(round_no=3, verdicts=verdicts),
                state_dir,
                bindings=bindings,
            )
            self.assertEqual(len(fx.sent), cap)
            self.assertEqual(len(out["overflow"]), 1)
            overflowed = out["overflow"][0]["session_id"]
            self.assertEqual(out["overflow"][0]["detail"], "over_queue_cap")
            self.assertEqual(out["overflow"][0]["applied"], False)
            self.assertEqual(out["interventions"]["queue"]["nudges"], cap)
            # The deferred episode exists with its budget
            # untouched, so the intervention re-plans next round.
            state = self._state(state_dir)
            self.assertEqual(state["sessions"][overflowed]["nudges_sent"], 0)
            out2 = self._step(
                fx,
                record(round_no=4, verdicts=verdicts),
                state_dir,
                bindings=bindings,
            )
            # Least-served first: the session deferred in round 3
            # is served in round 4, ahead of the sessions that
            # consumed budget ahead of it.
            self.assertIn(overflowed, [sid for sid, _ in fx.sent[cap:]])
            self.assertNotIn(overflowed, [a["session_id"] for a in out2["overflow"]])

    def test_shadow_reports_overflow_and_counts_but_writes_nothing(self):
        fx = FakeEffects()
        cap = nudges.MAX_QUEUE_INTERVENTIONS_PER_ROUND
        verdicts = {f"ses_{i:02d}": stalled(task_id=100 + i) for i in range(cap + 1)}
        with tempfile.TemporaryDirectory() as tmp:
            state_dir = Path(tmp)
            out = self._step(fx, record(verdicts=verdicts), state_dir, mode="shadow")
            self.assertEqual(len(out["overflow"]), 1)
            self.assertEqual(out["interventions"]["queue"]["nudges"], cap)
            self.assertEqual(fx.sent, [])
            self.assertFalse((state_dir / nudges.STATE_FILE_NAME).exists())


if __name__ == "__main__":
    unittest.main()
