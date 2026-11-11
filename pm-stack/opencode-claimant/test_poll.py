#!/usr/bin/env python3
"""Tests for poll.py's active-only cap counting (#3026 gate).

Run:  python3 -m unittest pm-stack/opencode-claimant/test_poll.py

Fixtures mirror the live server's session shape (GET /api/session,
verified 2026-10-31): a finished root session carries time.idle and an
outcome; a running one carries neither. Non-dict / child entries must
never count.
"""

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

_spec = importlib.util.spec_from_file_location(
    "poll", Path(__file__).resolve().parent / "poll.py"
)
poll = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(poll)

NOW = 1_793_500_000_000
MIN = 60 * 1000


def root(**over):
    s = {"id": "ses_x", "time": {"created": NOW - 5 * MIN, "updated": NOW - 1 * MIN}}
    s.update(over)
    return s


class TestIsActiveRoot(unittest.TestCase):
    def test_fresh_markerless_root_is_active(self):
        self.assertTrue(poll.is_active_root(root(), NOW))

    def test_child_session_never_active(self):
        self.assertFalse(poll.is_active_root(root(parentID="ses_p"), NOW))
        self.assertFalse(poll.is_active_root(root(parent_id="ses_p"), NOW))

    def test_idle_marker_is_finished(self):
        s = root()
        s["time"]["idle"] = NOW
        self.assertFalse(poll.is_active_root(s, NOW))

    def test_outcome_is_finished(self):
        self.assertFalse(poll.is_active_root(root(outcome="failed"), NOW))
        self.assertFalse(poll.is_active_root(root(outcome="completed"), NOW))

    def test_stale_markerless_root_is_a_zombie(self):
        s = root()
        s["time"]["updated"] = NOW - 31 * MIN
        self.assertFalse(poll.is_active_root(s, NOW))

    def test_window_boundary_is_excluded(self):
        s = root()
        s["time"]["updated"] = NOW - poll.ACTIVE_WINDOW_MS
        self.assertFalse(poll.is_active_root(s, NOW))
        s["time"]["updated"] = NOW - poll.ACTIVE_WINDOW_MS + 1
        self.assertTrue(poll.is_active_root(s, NOW))

    def test_created_is_the_fallback_clock(self):
        s = {"id": "ses_x", "time": {"created": NOW - 1 * MIN}}
        self.assertTrue(poll.is_active_root(s, NOW))

    def test_non_dict_and_timeless_entries_never_active(self):
        self.assertFalse(poll.is_active_root("nope", NOW))
        self.assertFalse(poll.is_active_root({"id": "ses_x"}, NOW))

    def test_resumed_session_counts_via_the_active_set(self):
        # A resumed root keeps its stored outcome/idle from the earlier
        # run; the server's active set overrides those markers.
        s = root(outcome="failed")
        s["time"]["idle"] = NOW
        self.assertFalse(poll.is_active_root(s, NOW, set()))
        self.assertFalse(poll.is_active_root(s, NOW, None))
        self.assertTrue(poll.is_active_root(s, NOW, {"ses_x"}))

    def test_active_set_membership_overrides_staleness(self):
        s = root()
        s["time"]["updated"] = NOW - 90 * MIN
        self.assertTrue(poll.is_active_root(s, NOW, {"ses_x"}))

    def test_count_active_roots_over_a_mixed_list(self):
        finished = root(outcome="failed")
        finished["time"]["idle"] = NOW
        sessions = [
            root(),
            root(),
            finished,
            root(parentID="ses_p"),
            {
                "id": "ses_old",
                "time": {"created": NOW - 90 * MIN, "updated": NOW - 90 * MIN},
            },
        ]
        self.assertEqual(poll.count_active_roots(sessions, NOW), 2)
        # The same list with ses_old in the server's active set (a
        # resumed zombie-looking root) counts it too.
        self.assertEqual(poll.count_active_roots(sessions, NOW, {"ses_old"}), 3)


class TestSummarizeCap(unittest.TestCase):
    def test_unreadable_session_list_is_unknown(self):
        self.assertEqual(poll.summarize_cap(None, set(), NOW), (None, None, None))

    def test_unreadable_active_set_is_unknown_not_zero(self):
        # Fail closed: markers alone must never become a launch
        # decision when /api/session/active cannot be read.
        live, active, room = poll.summarize_cap([root()], None, NOW)
        self.assertEqual(live, 1)
        self.assertIsNone(active)
        self.assertIsNone(room)

    def test_known_counts_give_room(self):
        live, active, room = poll.summarize_cap(
            [root(), root(outcome="failed")], set(), NOW
        )
        self.assertEqual((live, active, room), (2, 1, poll.CAP - 1))



class TestWrongShapeTime(unittest.TestCase):
    """#6868: a wrong-shaped `time` / timestamp must never raise out of
    the classifiers, and must fail closed: never counted as a
    known-inactive root (cap goes unknown), never stuck-bound."""

    BAD_TIMES = ["soon", 123, True, ["x"], {"updated": "soon"},
                 {"created": "soon"}, {"updated": ["x"]},
                 {"created": {"at": 1}}, {"updated": True}]

    def test_is_active_root_never_raises_and_never_active(self):
        for bad in self.BAD_TIMES:
            s = {"id": "ses_x", "time": bad}
            self.assertFalse(poll.is_active_root(s, NOW), repr(bad))
            self.assertFalse(poll.is_active_root(s, NOW, set()), repr(bad))

    def test_active_set_membership_still_authoritative(self):
        s = {"id": "ses_x", "time": "soon"}
        self.assertTrue(poll.is_active_root(s, NOW, {"ses_x"}))

    def test_count_active_roots_unknown_on_malformed_entry(self):
        sessions = [root(), {"id": "ses_bad", "time": "soon"}]
        self.assertIsNone(poll.count_active_roots(sessions, NOW, set()))

    def test_summarize_cap_unknown_not_manufactured_room(self):
        live, active, room = poll.summarize_cap(
            [root(), {"id": "ses_bad", "time": {"updated": "soon"}}],
            set(), NOW)
        self.assertEqual(live, 2)
        self.assertIsNone(active)
        self.assertIsNone(room)

    def test_is_stuck_bound_never_raises_and_never_stuck(self):
        row = TestIsStuckBound().row()
        for bad in self.BAD_TIMES:
            s = {"id": "ses_x", "time": bad}
            self.assertFalse(
                poll.is_stuck_bound(row, s, set(), NOW), repr(bad))

    def test_stuck_entry_session_updated_none_on_malformed(self):
        row = TestIsStuckBound().row()
        for bad in self.BAD_TIMES:
            entry = poll._stuck_entry(row, "q", {"id": "ses_x", "time": bad})
            self.assertIsNone(entry["session_updated"], repr(bad))

    def test_stuck_entry_keeps_numeric_updated(self):
        row = TestIsStuckBound().row()
        s = {"id": "ses_x",
             "time": {"created": NOW - 90 * MIN, "updated": NOW - 31 * MIN}}
        entry = poll._stuck_entry(row, "q", s)
        self.assertEqual(entry["session_updated"], NOW - 31 * MIN)

    def test_get_stuck_bound_over_mixed_list(self):
        row = TestIsStuckBound().row()
        good_dead = TestIsStuckBound().dead_session()
        bad = {"id": "ses_bad", "time": "soon"}
        by_id = {"ses_x": good_dead, "ses_bad": bad}
        stuck = poll.get_stuck_bound([row], by_id, set(), NOW)
        self.assertEqual(stuck, [row])


class TestIsCandidate(unittest.TestCase):
    def row(self, **over):
        r = {
            "progress": {"lane": "opencode", "stage": "handed_off"},
            "lane_id": "code-a",
        }
        r.update(over)
        return r

    def test_shift_lane_opencode_unbound_is_candidate(self):
        self.assertTrue(poll.is_candidate(self.row()))

    def test_legacy_claim_without_lane_id_is_candidate(self):
        self.assertTrue(poll.is_candidate(self.row(lane_id=None)))

    def test_openchamber_lane_claim_is_never_candidate(self):
        self.assertFalse(
            poll.is_candidate(self.row(lane_id="8ac61616-3815-4e7b-b33d-840a3829154a"))
        )

    def test_direct_lane_is_not_candidate(self):
        self.assertFalse(poll.is_candidate(self.row(progress={"lane": "direct"})))

    def test_bound_session_is_not_candidate(self):
        self.assertFalse(
            poll.is_candidate(
                self.row(progress={"lane": "opencode", "session_id": "ses_1"})
            )
        )

    def test_launch_failed_reclaim_is_candidate_again(self):
        # The #4758 recovery rule checkpoints stage "launch_failed"
        # with the binding cleared; that recorded yield must make the
        # task a launch candidate again or the reclaim strands it.
        self.assertTrue(
            poll.is_candidate(
                self.row(progress={"lane": "opencode", "stage": "launch_failed"})
            )
        )

    def test_assigned_but_not_handed_off_is_not_candidate(self):
        self.assertFalse(
            poll.is_candidate(
                self.row(progress={"lane": "opencode", "stage": "verified"})
            )
        )
        self.assertFalse(poll.is_candidate(self.row(progress={"lane": "opencode"})))

    def test_unassigned_task_is_not_candidate(self):
        self.assertFalse(poll.is_candidate(self.row(progress={})))


class TestIsHandoffWaiting(unittest.TestCase):
    """The handoff sink: a handed-off task resting in the sink
    (status 'handed_off', #8234) must surface as a candidate, or it
    is never collected. Sink membership is the STATUS — no stage
    is read here (#8192)."""

    def row(self, **over):
        r = {
            "status": "handed_off",
            "progress": {
                "lane": "opencode",
                "last_checkpoint": {"stage": "handed_off"},
                "session_id": None,
            },
            "lane_id": "code-j",
        }
        r.update(over)
        return poll.normalize_row(r)

    def test_sink_handoff_is_candidate(self):
        self.assertTrue(poll.is_handoff_waiting(self.row()))
        cands = poll.get_handed_off_candidates([self.row()])
        self.assertEqual(len(cands), 1)

    def test_sink_launch_failed_yield_is_candidate(self):
        r = self.row(progress={"lane": "opencode", "stage": "launch_failed"})
        self.assertTrue(poll.is_handoff_waiting(r))

    def test_stage_source_disagreement_is_still_candidate(self):
        # #8192 regression: the top-level stage and the last
        # checkpoint's stage DISAGREE (top-level advanced past the
        # yield). The broker used to read one source and this poll
        # the other; since #8234 neither reads a stage for a sink
        # row — the status is the membership — so the row is a
        # candidate either way.
        r = self.row(
            progress={
                "lane": "opencode",
                "stage": "working",
                "last_checkpoint": {"stage": "handed_off"},
                "session_id": None,
            }
        )
        self.assertTrue(poll.is_handoff_waiting(r))

    def test_genuine_wait_park_is_not_candidate(self):
        # A 'parked' row is a genuine wait by definition since
        # #8234 — even one whose progress is handoff-shaped (a
        # recorded handoff that later took a real triage wait,
        # task 10995's shape). The status decides, not the shape.
        r = self.row(status="parked")
        self.assertFalse(poll.is_handoff_waiting(r))

    def test_sink_bound_row_is_not_candidate(self):
        r = self.row(
            progress={
                "lane": "opencode",
                "stage": "handed_off",
                "session_id": "ses_1",
            }
        )
        self.assertFalse(poll.is_handoff_waiting(r))

    def test_claimed_row_is_not_a_sink_candidate(self):
        r = self.row(status="claimed")
        self.assertFalse(poll.is_handoff_waiting(r))

    def test_sink_openchamber_lane_is_never_candidate(self):
        r = self.row(lane_id="8d133595-c60c-4dcb-8e82-59b7d0ac9b5a")
        self.assertFalse(poll.is_handoff_waiting(r))


class TestParseLeaseMs(unittest.TestCase):
    def test_zulu_lease_parses(self):
        self.assertEqual(
            poll.parse_lease_ms("2026-11-03T04:43:41Z"),
            poll.parse_lease_ms("2026-11-03T04:43:41+00:00"),
        )
        self.assertGreater(poll.parse_lease_ms("2026-11-03T04:43:41Z"), 0)

    def test_offset_lease_parses(self):
        self.assertIsInstance(
            poll.parse_lease_ms("2026-11-03T04:43:41.3139+00:00"), int
        )

    def test_unparseable_lease_is_none(self):
        self.assertIsNone(poll.parse_lease_ms(None))
        self.assertIsNone(poll.parse_lease_ms(""))
        self.assertIsNone(poll.parse_lease_ms("not-a-date"))
        self.assertIsNone(poll.parse_lease_ms(12345))


EXPIRED = "2025-01-01T00:00:00Z"  # long before NOW
FUTURE = "2027-01-01T00:00:00Z"  # long after NOW


class TestIsStuckBound(unittest.TestCase):
    def row(self, **over):
        r = {
            "progress": {
                "lane": "opencode",
                "session_id": "ses_x",
                "stage": "handed_off",
            },
            "lease_until": EXPIRED,
            "lane_id": "code-a",
        }
        r.update(over)
        return r

    def dead_session(self, **over):
        s = {
            "id": "ses_x",
            "time": {"created": NOW - 90 * MIN, "updated": NOW - 31 * MIN},
        }
        s.update(over)
        return s

    def test_dead_bound_session_with_expired_lease_is_stuck(self):
        self.assertTrue(
            poll.is_stuck_bound(self.row(), self.dead_session(), set(), NOW)
        )

    def test_exactly_at_window_boundary_is_stuck(self):
        s = self.dead_session()
        s["time"]["updated"] = NOW - poll.ACTIVE_WINDOW_MS
        self.assertTrue(poll.is_stuck_bound(self.row(), s, set(), NOW))

    def test_recent_activity_vetoes(self):
        s = self.dead_session()
        s["time"]["updated"] = NOW - 1 * MIN
        self.assertFalse(poll.is_stuck_bound(self.row(), s, set(), NOW))

    def test_active_set_membership_vetoes(self):
        self.assertFalse(
            poll.is_stuck_bound(self.row(), self.dead_session(), {"ses_x"}, NOW)
        )

    def test_unknown_active_set_fails_closed(self):
        self.assertFalse(
            poll.is_stuck_bound(self.row(), self.dead_session(), None, NOW)
        )

    def test_terminal_markers_veto(self):
        s = self.dead_session(outcome="failed")
        self.assertFalse(poll.is_stuck_bound(self.row(), s, set(), NOW))
        s = self.dead_session()
        s["time"]["idle"] = NOW - 40 * MIN
        self.assertFalse(poll.is_stuck_bound(self.row(), s, set(), NOW))

    def test_unexpired_lease_vetoes(self):
        self.assertFalse(
            poll.is_stuck_bound(
                self.row(lease_until=FUTURE), self.dead_session(), set(), NOW
            )
        )

    def test_unparseable_lease_vetoes(self):
        self.assertFalse(
            poll.is_stuck_bound(
                self.row(lease_until="garbage"), self.dead_session(), set(), NOW
            )
        )
        self.assertFalse(
            poll.is_stuck_bound(
                self.row(lease_until=None), self.dead_session(), set(), NOW
            )
        )

    def test_missing_session_is_the_gone_case_not_stuck(self):
        self.assertFalse(poll.is_stuck_bound(self.row(), None, set(), NOW))

    def test_child_session_vetoes(self):
        self.assertFalse(
            poll.is_stuck_bound(
                self.row(), self.dead_session(parentID="ses_p"), set(), NOW
            )
        )

    def test_unbound_or_direct_lane_is_not_stuck(self):
        self.assertFalse(
            poll.is_stuck_bound(
                self.row(progress={"lane": "opencode"}), self.dead_session(), set(), NOW
            )
        )
        self.assertFalse(
            poll.is_stuck_bound(
                self.row(progress={"lane": "direct", "session_id": "ses_x"}),
                self.dead_session(),
                set(),
                NOW,
            )
        )


class TestFetchClaimedRows(unittest.TestCase):
    """Claimed rows come from the queue-broker's rich /list (#7325:
    the repo-queue binary's list verb prints a human table, and
    json-parsing it read every queue as empty). _broker_post is
    stubbed; no broker is touched. A queue that cannot be read
    contributes no rows: fetch_claimed_rows returns [] on any broker
    or payload failure."""

    def fetch(self, payload, raises=None):
        captured = {}

        def fake_post(path, body):
            captured["path"] = path
            captured["body"] = body
            if raises is not None:
                raise raises
            return payload

        with patch.object(poll, "_broker_post", side_effect=fake_post):
            rows = poll.fetch_claimed_rows("repo-health-code-pilot")
        return rows, captured

    def test_broker_list_reads_claimed_rows_and_normalizes(self):
        raw = {
            "id": 42,
            "queue": "repo-health-code-pilot",
            "status": "claimed",
            "worker_id": "repo-worker",
            "progress": {"lane": "opencode", "stage": "handed_off"},
        }
        rows, captured = self.fetch({"ok": True, "rows": [raw]})
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["id"], 42)
        self.assertEqual(rows[0]["progress"]["stage"], "handed_off")
        self.assertEqual(captured["path"], "/list")
        self.assertEqual(
            captured["body"],
            {"queue": "repo-health-code-pilot", "status": "claimed",
             "rich": True, "limit": poll.LIST_LIMIT},
        )

    def test_flattened_row_gets_progress_synthesized(self):
        rows, _ = self.fetch({"rows": [{
            "id": 7,
            "status": "claimed",
            "lane": "opencode",
            "stage": "handed_off",
            "session_id": None,
        }]})
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["progress"]["lane"], "opencode")
        self.assertEqual(rows[0]["progress"]["stage"], "handed_off")

    def test_stage_lifted_from_last_checkpoint(self):
        # #7706: the checkpoint verb records the yield under
        # progress.last_checkpoint.stage; the classifiers read
        # progress.stage. The lifted row must classify as a candidate.
        rows, _ = self.fetch({"rows": [{
            "id": 9,
            "status": "claimed",
            "progress": {"lane": "opencode",
                         "last_checkpoint": {"stage": "handed_off"}},
        }]})
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["progress"]["stage"], "handed_off")
        self.assertTrue(poll.is_candidate(rows[0]))

    def test_packet_lifted_from_payload(self):
        packet = {"kind": "fix", "repo": "developing-today/code",
                  "issue": 1234}
        rows, _ = self.fetch({"rows": [{
            "id": 11, "status": "claimed", "payload": packet,
            "progress": {"lane": "opencode", "stage": "handed_off"},
        }]})
        self.assertEqual(rows[0]["packet"], packet)
        entry = poll._candidate_entry(rows[0], "repo-health-code-pilot")
        self.assertEqual(entry["packet"], packet)

    def test_packet_lifted_from_payload_string(self):
        rows, _ = self.fetch({"rows": [{
            "id": 12, "status": "claimed",
            "payload": "{\"kind\": \"triage\"}",
            "progress": {"lane": "opencode", "stage": "handed_off"},
        }]})
        self.assertEqual(rows[0]["packet"], {"kind": "triage"})

    def test_empty_list_is_a_valid_empty_queue(self):
        rows, _ = self.fetch({"ok": True, "rows": []})
        self.assertEqual(rows, [])

    def test_broker_failure_returns_no_rows(self):
        rows, _ = self.fetch(None, raises=RuntimeError("broker down"))
        self.assertEqual(rows, [])

    def test_malformed_payload_returns_no_rows(self):
        rows, _ = self.fetch({"ok": True, "rows": "not a list"})
        self.assertEqual(rows, [])
        rows, _ = self.fetch({"ok": True})
        self.assertEqual(rows, [])

    def test_non_dict_and_unclaimed_rows_are_skipped(self):
        rows, _ = self.fetch({"rows": [
            None,
            {"id": 1, "status": "queued"},
            {"id": 2, "status": "claimed",
             "progress": {"lane": "opencode"}},
        ]})
        self.assertEqual([r["id"] for r in rows], [2])


class TestFetchSessions(unittest.TestCase):
    """Fetch-layer completeness (#4757): /api/session truncates at 50
    by default and honours limit but not offset, so fetch_sessions
    escalates the limit until a read comes back below it — and fails
    closed (None) whenever completeness cannot be proven.
    _server_get is stubbed; no server is touched."""

    def stub_server(self, total, fail_on_call=None):
        calls = []

        def fake_get(path):
            calls.append(path)
            if fail_on_call is not None and len(calls) == fail_on_call:
                return None
            limit = int(path.rsplit("limit=", 1)[1])
            return [{"id": f"ses_{i}"} for i in range(min(limit, total))]

        return fake_get, calls

    def run_fetch(self, total, **kw):
        fake_get, calls = self.stub_server(total, **kw)
        original = poll._server_get
        poll._server_get = fake_get
        try:
            return poll.fetch_sessions(), calls
        finally:
            poll._server_get = original

    def test_short_single_read_is_complete(self):
        sessions, calls = self.run_fetch(3)
        self.assertEqual(len(sessions), 3)
        self.assertEqual(len(calls), 1)

    def test_escalating_limit_finds_the_whole_list(self):
        sessions, calls = self.run_fetch(2500)
        self.assertEqual(len(sessions), 2500)
        self.assertEqual(len(calls), 2)

    def test_count_exactly_at_a_limit_escalates(self):
        sessions, calls = self.run_fetch(1000)
        self.assertEqual(len(sessions), 1000)
        self.assertEqual(len(calls), 2)

    def test_failed_read_fails_closed(self):
        sessions, _ = self.run_fetch(2500, fail_on_call=1)
        self.assertIsNone(sessions)
        sessions, _ = self.run_fetch(2500, fail_on_call=2)
        self.assertIsNone(sessions)

    def test_count_at_every_limit_fails_closed(self):
        sessions, calls = self.run_fetch(10**9)
        self.assertIsNone(sessions)
        self.assertEqual(len(calls), len(poll.SESSION_FETCH_LIMITS))


if __name__ == "__main__":
    unittest.main()
