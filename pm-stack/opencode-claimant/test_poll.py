#!/usr/bin/env python3
"""Tests for poll.py's active-only cap counting (#3026 gate).

Run:  python3 -m unittest pm-stack/opencode-claimant/test_poll.py

Fixtures mirror the live server's session shape (GET /api/session,
verified 2026-10-31): a finished root session carries time.idle and an
outcome; a running one carries neither. Non-dict / child entries must
never count.
"""
import importlib.util
import unittest
from pathlib import Path

_spec = importlib.util.spec_from_file_location(
    "poll", Path(__file__).resolve().parent / "poll.py")
poll = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(poll)

NOW = 1_793_500_000_000
MIN = 60 * 1000


def root(**over):
    s = {"id": "ses_x", "time": {"created": NOW - 5 * MIN,
                                 "updated": NOW - 1 * MIN}}
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
        sessions = [root(), root(), finished,
                    root(parentID="ses_p"),
                    {"id": "ses_old",
                     "time": {"created": NOW - 90 * MIN,
                              "updated": NOW - 90 * MIN}}]
        self.assertEqual(poll.count_active_roots(sessions, NOW), 2)
        # The same list with ses_old in the server's active set (a
        # resumed zombie-looking root) counts it too.
        self.assertEqual(
            poll.count_active_roots(sessions, NOW, {"ses_old"}), 3)


class TestSummarizeCap(unittest.TestCase):
    def test_unreadable_session_list_is_unknown(self):
        self.assertEqual(poll.summarize_cap(None, set(), NOW),
                         (None, None, None))

    def test_unreadable_active_set_is_unknown_not_zero(self):
        # Fail closed: markers alone must never become a launch
        # decision when /api/session/active cannot be read.
        live, active, room = poll.summarize_cap([root()], None, NOW)
        self.assertEqual(live, 1)
        self.assertIsNone(active)
        self.assertIsNone(room)

    def test_known_counts_give_room(self):
        live, active, room = poll.summarize_cap(
            [root(), root(outcome="failed")], set(), NOW)
        self.assertEqual((live, active, room), (2, 1, poll.CAP - 1))


if __name__ == "__main__":
    unittest.main()
