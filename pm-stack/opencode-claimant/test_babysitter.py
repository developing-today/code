#!/usr/bin/env python3
"""Tests for babysitter.py's verdict rounds (#1338).

Run:  python3 -m unittest pm-stack/opencode-claimant/test_babysitter.py

The oracle is #1338's own rule text, with #1192's detector:

- every root session at the location gets a verdict: progressing /
  stalled / done-unreported / blocked / walled (402-class) / idle;
  unbound sessions are observed, never driven (this module drives
  nothing - it only classifies);
- stall rule: suspect when >= 20 min since the session's last
  activity AND no live CI on its bound PR; confirm on a second
  suspect round (same activity stamp) or on expiry-equivalent
  silence (poll.py's ACTIVE_SET_STALE_MS zombie bound - silence so
  long the session is indistinguishable from expired);
- a session on a walled pool is flagged the round it happens
  (pool_walled on the verdict entry), and a non-progressing session
  whose silence the wall explains reads walled, not stalled;
- the round record is the data out: full verdicts plus a
  changed-only delta (the report is the delta; the log carries
  both), one entry per session naming its pool.

Session snapshots are poll.py's verified shape (root = no
parentID/parent_id, activity = time.updated falling back to
time.created, finished = time.idle or a top-level outcome, pool =
model.providerID / model.id per the aggregator's session mapper).
All inputs are injected, so the suite is hermetic: no server, no
queue, no files outside a temp state dir.

Negative control: this suite fails against origin/main, which has
no babysitter.py and no babysitter round in the claimant fire
(the verdict surface does not exist there).
"""

import importlib.util
import json
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


babysitter = _load("babysitter", "babysitter.py")
claimant = _load("claimant_for_babysitter", "claimant.py")

NOW = 1_700_000_000_000  # fixed round clock (ms)
MIN = 60 * 1000


def session(
    sid,
    *,
    updated=None,
    created=None,
    idle=None,
    outcome=None,
    parent=None,
    provider="opencode-route",
    model="workhorse",
):
    s = {"id": sid}
    if parent:
        s["parentID"] = parent
    t = {}
    if updated is not None:
        t["updated"] = updated
    if created is not None:
        t["created"] = created
    if idle is not None:
        t["idle"] = idle
    if t:
        s["time"] = t
    if outcome:
        s["outcome"] = outcome
    if provider:
        s["model"] = {"providerID": provider, "id": model}
    return s


def binding(task_id=101, **kw):
    b = {
        "task_id": task_id,
        "queue": "repo-health-code-pilot",
        "issue_number": 4242,
        "ci_live": None,
        "pr_merged": None,
        "blocked_reason": None,
    }
    b.update(kw)
    return b


def round_of(sessions, **kw):
    kw.setdefault("bindings", {})
    kw.setdefault("active_ids", set())
    kw.setdefault("pools", {})
    kw.setdefault("prior_state", {})
    kw.setdefault("now_ms", NOW)
    return babysitter.run_round(sessions, **kw)


class TestVerdicts(unittest.TestCase):
    def test_active_session_progressing_with_pool_attribution(self):
        s = session("ses_a", updated=NOW - 2 * MIN)
        record, _ = round_of([s], active_ids={"ses_a"})
        v = record["verdicts"]["ses_a"]
        self.assertEqual(v["verdict"], "progressing")
        self.assertEqual(v["pool"], "opencode-route/workhorse")
        self.assertFalse(v["pool_walled"])

    def test_quiet_session_under_threshold_is_idle(self):
        s = session("ses_a", updated=NOW - 5 * MIN)
        record, _ = round_of([s])
        self.assertEqual(record["verdicts"]["ses_a"]["verdict"], "idle")

    def test_unbound_silent_session_is_stall_suspect_then_confirmed(self):
        # Criterion 2's mechanism: suspect inside one round of the
        # threshold, confirmed on the second suspect round.
        s = session("ses_a", updated=NOW - 25 * MIN)
        r1, state1 = round_of([s])
        v1 = r1["verdicts"]["ses_a"]
        self.assertEqual(v1["verdict"], "stalled")
        self.assertFalse(v1["confirmed"])
        r2, _ = round_of([s], prior_state=state1, now_ms=NOW + 20 * MIN)
        v2 = r2["verdicts"]["ses_a"]
        self.assertEqual(v2["verdict"], "stalled")
        self.assertTrue(v2["confirmed"])

    def test_activity_between_rounds_resets_confirmation(self):
        s = session("ses_a", updated=NOW - 25 * MIN)
        _, state1 = round_of([s])
        moved = session("ses_a", updated=NOW + 10 * MIN)
        r2, _ = round_of([moved], prior_state=state1, now_ms=NOW + 20 * MIN)
        self.assertEqual(r2["verdicts"]["ses_a"]["verdict"], "idle")

    def test_expiry_equivalent_silence_confirms_in_one_round(self):
        s = session("ses_a", updated=NOW - 3 * 60 * MIN)
        record, _ = round_of([s])
        v = record["verdicts"]["ses_a"]
        self.assertEqual(v["verdict"], "stalled")
        self.assertTrue(v["confirmed"])

    def test_live_ci_on_bound_pr_vetoes_stall(self):
        s = session("ses_a", updated=NOW - 25 * MIN)
        record, _ = round_of([s], bindings={"ses_a": binding(ci_live=True)})
        self.assertEqual(record["verdicts"]["ses_a"]["verdict"], "progressing")

    def test_unknown_ci_never_proves_a_stall(self):
        s = session("ses_a", updated=NOW - 25 * MIN)
        record, _ = round_of([s], bindings={"ses_a": binding(ci_live=None)})
        v = record["verdicts"]["ses_a"]
        self.assertEqual(v["verdict"], "idle")
        self.assertIn("ci", v["reason"])

    def test_no_live_ci_bound_session_stalls(self):
        s = session("ses_a", updated=NOW - 25 * MIN)
        record, _ = round_of([s], bindings={"ses_a": binding(ci_live=False)})
        v = record["verdicts"]["ses_a"]
        self.assertEqual(v["verdict"], "stalled")
        self.assertEqual(v["task_id"], 101)

    def test_bound_progressing_session_carries_full_binding(self):
        # #1656 AC1: the round output identifies queue-spawned
        # sessions with their task/issue binding - not just the
        # task id. Oracle: build_bindings' binding shape, carried
        # through public_binding.
        s = session("ses_a", updated=NOW - 1 * MIN)
        bound = binding(repo="developing-today/code")
        record, _ = round_of(
            [s], active_ids={"ses_a"}, bindings={"ses_a": bound}
        )
        v = record["verdicts"]["ses_a"]
        self.assertEqual(v["verdict"], "progressing")
        self.assertEqual(
            v["binding"],
            {
                "task_id": 101,
                "queue": "repo-health-code-pilot",
                "issue_number": 4242,
                "repo": "developing-today/code",
            },
        )

    def test_error_stopped_bound_session_is_retryable_with_binding(self):
        s = session("ses_a", idle=NOW, outcome="error")
        bound = binding(repo="developing-today/code")
        record, _ = round_of([s], bindings={"ses_a": bound})
        v = record["verdicts"]["ses_a"]
        self.assertEqual(v["verdict"], "error-stopped")
        self.assertEqual(v["outcome"], "error")
        self.assertTrue(v["retryable"])
        self.assertEqual(v["binding"]["task_id"], 101)
        self.assertEqual(v["binding"]["repo"], "developing-today/code")

    def test_blocked_reason_explains_silence(self):
        s = session("ses_a", updated=NOW - 25 * MIN)
        record, _ = round_of(
            [s],
            bindings={"ses_a": binding(blocked_reason="waiting on #999")},
        )
        self.assertEqual(record["verdicts"]["ses_a"]["verdict"], "blocked")

    def test_walled_pool_explains_silence(self):
        s = session("ses_a", updated=NOW - 25 * MIN, provider="opencode-go")
        pools = {"opencode-go": {"walled": True, "percent": 100.0, "state": "ok"}}
        record, _ = round_of([s], pools=pools)
        v = record["verdicts"]["ses_a"]
        self.assertEqual(v["verdict"], "walled")
        self.assertTrue(v["pool_walled"])
        self.assertTrue(record["pools"]["opencode-go"]["walled"])

    def test_progressing_session_on_walled_pool_is_flagged_not_walled(self):
        s = session("ses_a", updated=NOW - 1 * MIN, provider="opencode-go")
        pools = {"opencode-go": {"walled": True, "percent": 100.0, "state": "ok"}}
        record, _ = round_of([s], active_ids={"ses_a"}, pools=pools)
        v = record["verdicts"]["ses_a"]
        self.assertEqual(v["verdict"], "progressing")
        self.assertTrue(v["pool_walled"])

    def test_finished_bound_session_is_done_unreported(self):
        s = session("ses_a", updated=NOW - 30 * MIN, idle=NOW - 30 * MIN)
        record, _ = round_of([s], bindings={"ses_a": binding(pr_merged=True)})
        v = record["verdicts"]["ses_a"]
        self.assertEqual(v["verdict"], "done-unreported")
        self.assertTrue(v["harvest_ready"])

    def test_finished_unbound_session_is_idle_not_done_unreported(self):
        s = session("ses_a", updated=NOW - 30 * MIN, outcome="completed")
        record, _ = round_of([s])
        self.assertEqual(record["verdicts"]["ses_a"]["verdict"], "idle")

    def test_resumed_session_with_stale_markers_progresses(self):
        # poll.py's rule: active-set membership overrides stored
        # terminal markers (a resumed session keeps its old idle).
        s = session("ses_a", updated=NOW - 1 * MIN, idle=NOW - 60 * MIN)
        record, _ = round_of([s], active_ids={"ses_a"})
        self.assertEqual(record["verdicts"]["ses_a"]["verdict"], "progressing")

    def test_unreadable_timing_is_named_not_verdicted(self):
        s = {
            "id": "ses_a",
            "time": {"updated": "soon"},
            "model": {"providerID": "opencode-route", "id": "workhorse"},
        }
        record, _ = round_of([s])
        self.assertNotIn("ses_a", record["verdicts"])
        self.assertEqual([u["session_id"] for u in record["unclassified"]], ["ses_a"])

    def test_child_sessions_are_counted_not_verdicted(self):
        root = session("ses_root", updated=NOW - 1 * MIN)
        child = session("ses_child", updated=NOW - 1 * MIN, parent="ses_root")
        record, _ = round_of([root, child], active_ids={"ses_root"})
        self.assertIn("ses_root", record["verdicts"])
        self.assertNotIn("ses_child", record["verdicts"])
        self.assertEqual(record["child_sessions"], 1)

    def test_timeless_session_fails_closed_to_confirmed_stall(self):
        # No readable activity stamp and not in the active set: the
        # fleet's zombie rule - indistinguishable from expired.
        s = {
            "id": "ses_a",
            "model": {"providerID": "opencode-route", "id": "workhorse"},
        }
        record, _ = round_of([s])
        v = record["verdicts"]["ses_a"]
        self.assertEqual(v["verdict"], "stalled")
        self.assertTrue(v["confirmed"])


class TestRoundDelta(unittest.TestCase):
    def test_changes_are_delta_only(self):
        a = session("ses_a", updated=NOW - 1 * MIN)
        b = session("ses_b", updated=NOW - 1 * MIN)
        _, state1 = round_of([a, b], active_ids={"ses_a", "ses_b"})
        # Round 2: a unchanged, b went quiet past the threshold,
        # c is new, and the round no longer sees a fourth session.
        b2 = session("ses_b", updated=NOW - 1 * MIN)
        c = session("ses_c", updated=NOW - 1 * MIN)
        gone_state = {"round": state1["round"], "sessions": dict(state1["sessions"])}
        gone_state["sessions"]["ses_gone"] = {
            "verdict": "idle",
            "confirmed": False,
            "updated_ms": NOW - 1 * MIN,
            "pool": None,
        }
        r2, _ = round_of(
            [a, b2, c],
            active_ids={"ses_a", "ses_c"},
            prior_state=gone_state,
            now_ms=NOW + 25 * MIN,
        )
        changed = {ch["session_id"]: ch for ch in r2["changes"]}
        self.assertNotIn("ses_a", changed)
        self.assertIn("ses_b", changed)  # progressing -> stalled
        self.assertIn("ses_c", changed)  # new
        self.assertIn("ses_gone", changed)  # gone
        self.assertEqual(changed["ses_gone"]["change"], "gone")


class TestBabysitterStep(unittest.TestCase):
    def _inputs(self):
        s = session("ses_a", updated=NOW - 25 * MIN)
        return dict(
            sessions=[s],
            active_ids=set(),
            bindings={},
            now_ms=NOW,
        )

    def test_live_step_persists_state_and_appends_round_log(self):
        with tempfile.TemporaryDirectory() as tmp:
            state_dir = Path(tmp)
            rec1 = babysitter.babysitter_step(
                "live", state_dir=state_dir, **self._inputs()
            )
            self.assertFalse(rec1["verdicts"]["ses_a"]["confirmed"])
            log = state_dir / "babysitter-rounds.jsonl"
            self.assertTrue(log.exists())
            lines = log.read_text().splitlines()
            self.assertEqual(len(lines), 1)
            self.assertEqual(json.loads(lines[0])["round"], rec1["round"])
            # Second live step reads the persisted state back and
            # confirms the stall - persistence is the mechanism.
            inputs = self._inputs()
            inputs["now_ms"] = NOW + 20 * MIN
            rec2 = babysitter.babysitter_step("live", state_dir=state_dir, **inputs)
            self.assertTrue(rec2["verdicts"]["ses_a"]["confirmed"])
            self.assertEqual(len(log.read_text().splitlines()), 2)

    def test_shadow_step_mutates_nothing(self):
        with tempfile.TemporaryDirectory() as tmp:
            state_dir = Path(tmp)
            rec = babysitter.babysitter_step(
                "shadow", state_dir=state_dir, **self._inputs()
            )
            self.assertEqual(rec["verdicts"]["ses_a"]["verdict"], "stalled")
            self.assertFalse((state_dir / "babysitter-rounds.jsonl").exists())
            self.assertFalse((state_dir / "babysitter-state.json").exists())

    def test_unreadable_session_list_records_error_and_writes_nothing(self):
        with tempfile.TemporaryDirectory() as tmp:
            state_dir = Path(tmp)
            rec = babysitter.babysitter_step(
                "live",
                sessions=None,
                active_ids=None,
                bindings={},
                pools={},
                state_dir=state_dir,
                now_ms=NOW,
            )
            self.assertEqual(rec["error"], "sessions_unreadable")
            self.assertFalse((state_dir / "babysitter-rounds.jsonl").exists())
            self.assertFalse((state_dir / "babysitter-state.json").exists())

    def test_pool_state_file_read_and_unread(self):
        with tempfile.TemporaryDirectory() as tmp:
            state_dir = Path(tmp)
            rec = babysitter.babysitter_step(
                "shadow", state_dir=state_dir, **self._inputs()
            )
            self.assertEqual(rec["pool_state"], "unread")
            (state_dir / "pool-state.json").write_text(
                json.dumps(
                    {
                        "pools": {
                            "opencode-route": {
                                "walled": True,
                                "percent": 100.0,
                                "state": "ok",
                            }
                        }
                    }
                )
            )
            s = session("ses_a", updated=NOW - 25 * MIN)
            rec2 = babysitter.babysitter_step(
                "shadow",
                sessions=[s],
                active_ids=set(),
                bindings={},
                pools=None,
                state_dir=state_dir,
                now_ms=NOW,
            )
            self.assertEqual(rec2["pool_state"], "read")
            self.assertEqual(rec2["verdicts"]["ses_a"]["verdict"], "walled")


class TestFireWiring(unittest.TestCase):
    """The claimant fire runs one round per fire and the fire log
    carries the record under its `babysitter` key (the hand_back /
    provider_wall precedent). Oracle: the fires.jsonl entry."""

    def test_fire_log_carries_babysitter_record(self):
        sentinel = {"round": 7, "verdicts": {}, "changes": []}

        class FakeBabysitter:
            @staticmethod
            def babysitter_step(mode):
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
                patch.object(claimant, "STATE_DIR", state_dir),
                patch.object(claimant, "FIRES_FILE", fires),
            ):
                rc = claimant.main()
            self.assertEqual(rc, 0)
            entries = [
                json.loads(line) for line in fires.read_text().splitlines() if line
            ]
        self.assertEqual(len(entries), 1)
        self.assertEqual(entries[0]["babysitter"], dict(sentinel, mode="shadow"))


if __name__ == "__main__":
    unittest.main()
