#!/usr/bin/env python3
"""Contract tests for poll.py candidate, stale-binding and session-cap
classification.

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

    def test_progress_cleared_session_is_not_a_candidate(self):
        self.assertFalse(
            poll.is_candidate(
                self.row(
                    progress={
                        "lane": "opencode",
                        "stage": "handed_off",
                        "session_id": None,
                        "session_cleared": {
                            "via": "expire_leases",
                            "cleared_at": "2026-11-05T20:01:19Z",
                            "session_id": "ses_old",
                        },
                    }
                )
            )
        )


class TestStaleBinding(unittest.TestCase):
    def row(self, **over):
        r = {
            "id": 4924,
            "queue": "repo-health-code-pilot",
            "priority": 30,
            "worker_id": "repo-worker-code",
            "lane_id": "code-a",
            "lease_until": FUTURE,
            "progress": {
                "lane": "opencode",
                "stage": "handed_off",
                "session_id": None,
                "issue_number": 5508,
                "session_cleared": {
                    "via": "expire_leases",
                    "cleared_at": "2026-11-05T20:01:19Z",
                    "session_id": "ses_old",
                },
            },
        }
        r.update(over)
        return r

    def test_progress_cleared_task_is_classified_for_reconciliation(self):
        row = self.row()
        self.assertTrue(poll.is_stale_binding(row))
        self.assertEqual(poll.get_stale_bindings([row]), [row])
        self.assertEqual(poll.get_candidates([row]), [])

    def test_current_binding_is_not_stale(self):
        self.assertFalse(
            poll.is_stale_binding(
                self.row(
                    progress={
                        "lane": "opencode",
                        "session_id": "ses_current",
                        "session_cleared": {"session_id": "ses_old"},
                    }
                )
            )
        )

    def test_other_execution_lane_is_not_stale(self):
        self.assertFalse(
            poll.is_stale_binding(
                self.row(
                    progress={
                        "lane": "direct",
                        "session_id": None,
                        "session_cleared": {"session_id": "ses_old"},
                    }
                )
            )
        )

    def test_stale_binding_entry_exposes_cleared_session_and_task(self):
        entry = poll._stale_binding_entry(self.row(), "repo-health-code-pilot")
        self.assertEqual(entry["id"], 4924)
        self.assertEqual(entry["issue_number"], 5508)
        self.assertIsNone(entry["session_id"])
        self.assertEqual(entry["cleared_session_id"], "ses_old")
        self.assertEqual(entry["cleared_via"], "expire_leases")



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
    """Claimed rows come from the repo-queue binary (#5419 replaced
    the broker-HTTP read this suite originally pinned when it was
    written against the pre-#5419 poll.py). subprocess.run is
    stubbed; no queue is touched. A queue that cannot be read
    contributes no rows: fetch_claimed_rows returns [] on any CLI
    or payload failure."""

    class Result:
        def __init__(self, returncode=0, stdout="", stderr=""):
            self.returncode = returncode
            self.stdout = stdout
            self.stderr = stderr

    def fetch(self, payload, returncode=0):
        captured = {}

        def fake_run(cmd, **kwargs):
            captured["cmd"] = cmd
            captured["kwargs"] = kwargs
            stdout = payload if isinstance(payload, str) else json.dumps(payload)
            return self.Result(returncode, stdout)

        with patch.object(poll.subprocess, "run", side_effect=fake_run):
            rows = poll.fetch_claimed_rows("repo-health-code-pilot")
        return rows, captured

    def test_cli_list_reads_claimed_rows_and_normalizes(self):
        raw = {
            "id": 42,
            "queue": "repo-health-code-pilot",
            "status": "claimed",
            "worker_id": "repo-worker",
            "progress": {"lane": "opencode", "stage": "handed_off"},
        }
        rows, captured = self.fetch({"rows": [raw]})
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["id"], 42)
        self.assertEqual(rows[0]["progress"]["stage"], "handed_off")
        cmd = [str(part) for part in captured["cmd"]]
        self.assertEqual(
            cmd[1:],
            ["list", "--queue", "repo-health-code-pilot",
             "--status", "claimed"],
        )

    def test_flattened_row_gets_progress_synthesized(self):
        rows, _ = self.fetch([{
            "id": 7,
            "status": "claimed",
            "lane": "opencode",
            "stage": "handed_off",
            "session_id": None,
        }])
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["progress"]["lane"], "opencode")
        self.assertEqual(rows[0]["progress"]["stage"], "handed_off")

    def test_empty_list_is_a_valid_empty_queue(self):
        rows, _ = self.fetch({"rows": []})
        self.assertEqual(rows, [])

    def test_cli_failure_returns_no_rows(self):
        rows, _ = self.fetch("", returncode=1)
        self.assertEqual(rows, [])

    def test_malformed_payload_returns_no_rows(self):
        rows, _ = self.fetch("this is not json")
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
