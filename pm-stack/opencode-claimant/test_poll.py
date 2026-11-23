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
import urllib.error
import urllib.parse
from pathlib import Path
from unittest.mock import mock_open, patch

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


class TestClaimantCapAccounting(unittest.TestCase):
    """#5016, ruling `session-cap-5016`: the concurrency cap is the
    claimant's LAUNCH budget. Scheduled fleet sessions (lane runners,
    the claimant's own fire, sweeps, the conductor) are infrastructure
    and never consume a slot; claimant work roots do; a server-active
    root silent past ACTIVE_SET_STALE_MS is a zombie and releases its
    slot (#5016's 18h session).

    Oracle: the census table on #5016 — 11 scheduled lane runners, the
    claimant's own fire, 1 genuinely running bound work session, 1
    unbound ~18h zombie — plus the invariant that the count equals the
    number of live claimant work roots regardless of how many scheduled
    roots sit beside them."""

    def lane_runner(self, i=0, **over):
        s = {
            "id": f"ses_lane_{i}",
            "title": f"Queue lane (scheduled) - code 2026-11-03 1{i}:00",
            "metadata": {"openchamber": {"kind": "scheduled"}},
            "time": {"created": NOW - 90 * MIN, "updated": NOW - 90 * MIN},
        }
        s.update(over)
        return s

    def claimant_fire(self, **over):
        s = {
            "id": "ses_fire",
            "title": "Opencode lane claimant/executor - #3026 2026-11-03 10:10",
            "metadata": {"openchamber": {"kind": "scheduled"}},
            "time": {"created": NOW - 10 * MIN, "updated": NOW - 1 * MIN},
        }
        s.update(over)
        return s

    def work_root(self, sid="ses_work", **over):
        s = {
            "id": sid,
            "title": f"Queue task 8067 - 5016 claimant cap",
            "time": {"created": NOW - 30 * MIN, "updated": NOW - 1 * MIN},
        }
        s.update(over)
        return s

    def test_lane_runner_never_consumes_a_claimant_slot(self):
        live, active, room = poll.summarize_cap(
            [self.lane_runner(0), self.lane_runner(1), self.work_root()],
            {"ses_lane_0", "ses_lane_1", "ses_work"},
            NOW,
        )
        self.assertEqual((live, active, room), (3, 1, poll.CAP - 1))

    def test_claimant_fire_session_does_not_count_itself(self):
        _live, active, _room = poll.summarize_cap(
            [self.claimant_fire()], {"ses_fire"}, NOW
        )
        self.assertEqual(active, 0)

    def test_claimant_work_roots_do_count(self):
        roots = [self.work_root(f"ses_w{i}") for i in range(12)]
        _live, active, room = poll.summarize_cap(roots, {r["id"] for r in roots}, NOW)
        self.assertEqual(active, 12)
        self.assertEqual(room, max(0, poll.CAP - 12))

    def test_scheduled_roots_never_change_claimant_headroom(self):
        # Invariant sweep: lane-runner population must not move the
        # claimant count at any size (0..40), the exact shape that
        # starved the fleet on 2026-11-03.
        for n in range(41):
            sessions = [self.lane_runner(i) for i in range(n)] + [self.work_root()]
            active_ids = {s["id"] for s in sessions}
            _live, active, room = poll.summarize_cap(sessions, active_ids, NOW)
            self.assertEqual(active, 1, f"n={n}")
            self.assertEqual(room, poll.CAP - 1, f"n={n}")

    def test_issue_5016_census_mix(self):
        # 11 lane runners + the claimant fire + 1 running bound work
        # session + the 18h unbound zombie (a lane runner, still in the
        # server's active set). Claimant count must be 1, not 14.
        sessions = [self.lane_runner(i) for i in range(11)]
        sessions.append(self.claimant_fire())
        sessions.append(self.work_root())
        zombie = self.lane_runner(
            99,
            title="Queue lane (scheduled) - opencode 2026-11-02 16:00",
            time={"created": NOW - 19 * 60 * MIN, "updated": NOW - 18 * 60 * MIN},
        )
        sessions.append(zombie)
        active_ids = {s["id"] for s in sessions}
        live, active, room = poll.summarize_cap(sessions, active_ids, NOW)
        self.assertEqual(live, len(sessions))
        self.assertEqual(active, 1)
        self.assertEqual(room, poll.CAP - 1)

    def test_active_set_zombie_releases_its_slot_after_the_bound(self):
        zombie = self.work_root(
            "ses_zombie",
            time={
                "created": NOW - 19 * 60 * MIN,
                "updated": NOW - poll.ACTIVE_SET_STALE_MS,
            },
        )
        self.assertFalse(poll.is_active_root(zombie, NOW, {"ses_zombie"}))
        _live, active, _room = poll.summarize_cap([zombie], {"ses_zombie"}, NOW)
        self.assertEqual(active, 0)

    def test_boundary_just_inside_the_staleness_bound_still_counts(self):
        s = self.work_root(
            "ses_zombie",
            time={
                "created": NOW - 19 * 60 * MIN,
                "updated": NOW - poll.ACTIVE_SET_STALE_MS + 1,
            },
        )
        self.assertTrue(poll.is_active_root(s, NOW, {"ses_zombie"}))

    def test_zombie_does_not_need_a_terminal_marker(self):
        # #5016's zombie has no outcome and no time.idle; silence past
        # the bound is the whole evidence.
        s = self.work_root(
            "ses_zombie",
            time={"created": NOW - 19 * 60 * MIN, "updated": NOW - 18 * 60 * MIN},
        )
        self.assertIsNone(s.get("outcome"))
        self.assertIsNone((s.get("time") or {}).get("idle"))
        self.assertFalse(poll.is_active_root(s, NOW, {"ses_zombie"}))

    def test_scheduled_marker_survives_missing_metadata(self):
        # The title is the fallback when a legacy row carries no
        # metadata (rows created before OpenChamber stamped `kind`).
        legacy = self.lane_runner(0, metadata=None)
        self.assertTrue(poll.is_scheduled_session(legacy))
        self.assertFalse(poll.is_scheduled_session(self.work_root()))
        self.assertFalse(poll.is_scheduled_session({"id": "x", "metadata": "soon"}))
        self.assertFalse(poll.is_scheduled_session(None))

    def test_claimant_root_count_delegates_to_the_exclusion(self):
        sessions = [self.lane_runner(i) for i in range(3)] + [
            self.work_root("ses_w0"),
            self.work_root("ses_w1"),
        ]
        self.assertEqual(
            poll.count_active_claimant_roots(
                sessions, NOW, {"ses_lane_0", "ses_w0", "ses_w1"}
            ),
            2,
        )

    def test_unreadable_active_set_stays_unknown_with_scheduled_roots(self):
        _live, active, room = poll.summarize_cap(
            [self.lane_runner(0), self.work_root()], None, NOW
        )
        self.assertIsNone(active)
        self.assertIsNone(room)


class TestWrongShapeTime(unittest.TestCase):
    """#6868: a wrong-shaped `time` / timestamp must never raise out of
    the classifiers, and must fail closed: never counted as a
    known-inactive root (cap goes unknown), never stuck-bound."""

    BAD_TIMES = [
        "soon",
        123,
        True,
        ["x"],
        {"updated": "soon"},
        {"created": "soon"},
        {"updated": ["x"]},
        {"created": {"at": 1}},
        {"updated": True},
    ]

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
            [root(), {"id": "ses_bad", "time": {"updated": "soon"}}], set(), NOW
        )
        self.assertEqual(live, 2)
        self.assertIsNone(active)
        self.assertIsNone(room)

    def test_terminal_marker_with_malformed_timing_is_inactive(self):
        # #8853 (oracle: the acceptance example on issue #8853): a
        # finished root's malformed timestamp must not make the cap
        # unknown — the terminal marker is checked before timing.
        s = {"id": "ses_done", "time": {"idle": 123, "updated": "bad"}}
        self.assertIs(poll._active_root_state(s, NOW, set()), False)
        self.assertEqual(poll.count_active_roots([s], NOW, set()), 0)
        live, active, room = poll.summarize_cap([root(), s], set(), NOW)
        self.assertEqual((live, active, room), (2, 1, poll.CAP - 1))

    def test_outcome_with_malformed_timing_is_inactive(self):
        s = {"id": "ses_done", "time": {"updated": "bad"}, "outcome": "failed"}
        self.assertIs(poll._active_root_state(s, NOW, set()), False)

    def test_active_set_still_overrides_terminal_marker(self):
        # A resumed session in the server's active set counts active
        # even with a stored terminal marker and malformed timing.
        s = {"id": "ses_done", "time": {"idle": 123, "updated": "bad"}}
        self.assertIs(poll._active_root_state(s, NOW, {"ses_done"}), True)

    def test_is_stuck_bound_never_raises_and_never_stuck(self):
        row = TestIsStuckBound().row()
        for bad in self.BAD_TIMES:
            s = {"id": "ses_x", "time": bad}
            self.assertFalse(poll.is_stuck_bound(row, s, set(), NOW), repr(bad))

    def test_stuck_entry_session_updated_none_on_malformed(self):
        row = TestIsStuckBound().row()
        for bad in self.BAD_TIMES:
            entry = poll._stuck_entry(row, "q", {"id": "ses_x", "time": bad})
            self.assertIsNone(entry["session_updated"], repr(bad))

    def test_stuck_entry_keeps_numeric_updated(self):
        row = TestIsStuckBound().row()
        s = {
            "id": "ses_x",
            "time": {"created": NOW - 90 * MIN, "updated": NOW - 31 * MIN},
        }
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
            {
                "queue": "repo-health-code-pilot",
                "status": "claimed",
                "rich": True,
                "limit": poll.LIST_LIMIT,
            },
        )

    def test_flattened_row_gets_progress_synthesized(self):
        rows, _ = self.fetch(
            {
                "rows": [
                    {
                        "id": 7,
                        "status": "claimed",
                        "lane": "opencode",
                        "stage": "handed_off",
                        "session_id": None,
                    }
                ]
            }
        )
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["progress"]["lane"], "opencode")
        self.assertEqual(rows[0]["progress"]["stage"], "handed_off")

    def test_stage_lifted_from_last_checkpoint(self):
        # #7706: the checkpoint verb records the yield under
        # progress.last_checkpoint.stage; the classifiers read
        # progress.stage. The lifted row must classify as a candidate.
        rows, _ = self.fetch(
            {
                "rows": [
                    {
                        "id": 9,
                        "status": "claimed",
                        "progress": {
                            "lane": "opencode",
                            "last_checkpoint": {"stage": "handed_off"},
                        },
                    }
                ]
            }
        )
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["progress"]["stage"], "handed_off")
        self.assertTrue(poll.is_candidate(rows[0]))

    def test_packet_lifted_from_payload(self):
        packet = {"kind": "fix", "repo": "developing-today/code", "issue": 1234}
        rows, _ = self.fetch(
            {
                "rows": [
                    {
                        "id": 11,
                        "status": "claimed",
                        "payload": packet,
                        "progress": {"lane": "opencode", "stage": "handed_off"},
                    }
                ]
            }
        )
        self.assertEqual(rows[0]["packet"], packet)
        entry = poll._candidate_entry(rows[0], "repo-health-code-pilot")
        self.assertEqual(entry["packet"], packet)

    def test_packet_lifted_from_payload_string(self):
        rows, _ = self.fetch(
            {
                "rows": [
                    {
                        "id": 12,
                        "status": "claimed",
                        "payload": '{"kind": "triage"}',
                        "progress": {"lane": "opencode", "stage": "handed_off"},
                    }
                ]
            }
        )
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
        rows, _ = self.fetch(
            {
                "rows": [
                    None,
                    {"id": 1, "status": "queued"},
                    {"id": 2, "status": "claimed", "progress": {"lane": "opencode"}},
                ]
            }
        )
        self.assertEqual([r["id"] for r in rows], [2])


class TestFetchSessions(unittest.TestCase):
    """Fetch-layer completeness: session list pages use cursor.next
    and never request more than the server's 200-row cap.
    _server_get is stubbed; no server is touched."""

    def stub_server(self, total, fail_on_call=None):
        calls = []

        def fake_get(path, **_kwargs):
            calls.append(path)
            if fail_on_call is not None and len(calls) == fail_on_call:
                return None
            query = urllib.parse.parse_qs(path.partition("?")[2])
            limit = int(query["limit"][0])
            start = int(query.get("cursor", ["0"])[0])
            page = [{"id": f"ses_{i}"} for i in range(start, min(start + limit, total))]
            next_cursor = str(start + limit) if start + limit < total else None
            return {"data": page, "cursor": {"next": next_cursor}}

        return fake_get, calls

    def run_fetch(self, total, **kw):
        fake_get, calls = self.stub_server(total, **kw)
        original = poll._server_get
        poll._server_get = fake_get
        try:
            return poll.fetch_sessions(), calls
        finally:
            poll._server_get = original

    def test_short_single_page_is_complete(self):
        sessions, calls = self.run_fetch(3)
        self.assertEqual(len(sessions), 3)
        self.assertEqual(len(calls), 1)

    def test_more_than_200_sessions_follow_cursor_to_completion(self):
        sessions, calls = self.run_fetch(450)
        self.assertEqual(len(sessions), 450)
        self.assertEqual([s["id"] for s in sessions], [f"ses_{i}" for i in range(450)])
        self.assertEqual(len(calls), 3)
        for path in calls:
            query = urllib.parse.parse_qs(path.partition("?")[2])
            self.assertLessEqual(int(query["limit"][0]), 200)
        self.assertEqual(calls[0], "/api/session?limit=200")
        self.assertIn("cursor=200", calls[1])

    def test_failed_read_fails_closed(self):
        sessions, _ = self.run_fetch(450, fail_on_call=1)
        self.assertIsNone(sessions)
        sessions, _ = self.run_fetch(450, fail_on_call=2)
        self.assertIsNone(sessions)

    def test_http_error_is_not_swallowed_for_session_list(self):
        error = urllib.error.HTTPError("url", 400, "Bad Request", {}, None)
        with patch("builtins.open", mock_open(read_data='{"password":"pw"}')):
            with patch.object(poll.urllib.request, "urlopen", side_effect=error):
                with self.assertRaises(urllib.error.HTTPError) as raised:
                    poll._server_get("/api/session?limit=1000", raise_http_errors=True)
        self.assertEqual(raised.exception.code, 400)


class TestStaleBinding(unittest.TestCase):
    """#5508: progress.session_id clear + lane-ledger binding present
    must never be offered as a launch candidate."""

    def row(self, **over):
        r = {
            "id": 4242,
            "progress": {"lane": "opencode", "stage": "handed_off"},
            "lane_id": "code-a",
        }
        r.update(over)
        return r

    def binding(self, **over):
        b = {
            "session_id": "ses_bound",
            "assigned_lane": "opencode",
            "executed_lane": "opencode",
            "updated_at": "2026-11-04T13:00:59Z",
        }
        b.update(over)
        return b

    def test_progress_cleared_ledger_bound_is_stale(self):
        self.assertTrue(poll.is_stale_binding(self.row(), self.binding()))

    def test_progress_cleared_ledger_unbound_is_not_stale(self):
        self.assertFalse(
            poll.is_stale_binding(self.row(), self.binding(session_id=None))
        )

    def test_no_ledger_row_is_not_stale(self):
        self.assertFalse(poll.is_stale_binding(self.row(), None))

    def test_progress_bound_row_is_never_stale(self):
        row = self.row(progress={"lane": "opencode", "session_id": "ses_live"})
        self.assertFalse(poll.is_stale_binding(row, self.binding()))

    def test_direct_lane_row_is_never_stale(self):
        row = self.row(progress={"lane": "direct"})
        self.assertFalse(poll.is_stale_binding(row, self.binding()))

    def test_filter_keeps_unbound_and_no_row_candidates(self):
        bound = self.row(id=1)
        unbound = self.row(id=2)
        no_row = self.row(id=3)
        bindings = {1: self.binding(), 2: self.binding(session_id=None)}
        launchable, stale = poll.apply_ledger_filter([bound, unbound, no_row], bindings)
        self.assertEqual(launchable, [unbound, no_row])
        self.assertEqual(stale, [bound])

    def test_filter_with_empty_ledger_keeps_candidates(self):
        row = self.row()
        launchable, stale = poll.apply_ledger_filter([row], {})
        self.assertEqual(launchable, [row])
        self.assertEqual(stale, [])

    def test_unreadable_ledger_fails_closed(self):
        row = self.row()
        launchable, stale = poll.apply_ledger_filter([row], None)
        self.assertEqual(launchable, [])
        self.assertEqual(stale, [row])

    def test_stale_entry_carries_ledger_evidence(self):
        entry = poll._stale_binding_entry(
            self.row(), "repo-health-code-pilot", self.binding(), True
        )
        self.assertEqual(entry["ledger_session_id"], "ses_bound")
        self.assertEqual(entry["ledger_updated_at"], "2026-11-04T13:00:59Z")
        self.assertTrue(entry["ledger_readable"])
        self.assertIsNone(entry["session_id"])

    def test_stale_entry_marks_unreadable_ledger(self):
        entry = poll._stale_binding_entry(
            self.row(), "repo-health-code-pilot", None, False
        )
        self.assertIsNone(entry["ledger_session_id"])
        self.assertFalse(entry["ledger_readable"])

    def test_fetch_ledger_bindings_parses_turso_output(self):
        stdout = (
            "HTTP 200\n"
            "[0] ok affected=0 rowid=None rows=["
            "[{'type': 'integer', 'value': '4924'}, "
            "{'type': 'text', 'value': 'ses_x'}, "
            "{'type': 'text', 'value': 'opencode'}, "
            "{'type': 'null', 'value': None}, "
            "{'type': 'text', 'value': '2026-11-04T13:00:59Z'}]]\n"
        )
        fake = unittest.mock.Mock(returncode=0, stdout=stdout)
        with patch.object(poll.subprocess, "run", return_value=fake):
            bindings = poll.fetch_ledger_bindings([4924])
        self.assertEqual(bindings[4924]["session_id"], "ses_x")
        self.assertEqual(bindings[4924]["assigned_lane"], "opencode")
        self.assertIsNone(bindings[4924]["executed_lane"])

    def test_fetch_ledger_bindings_fails_closed_on_error(self):
        fake = unittest.mock.Mock(returncode=1, stdout="")
        with patch.object(poll.subprocess, "run", return_value=fake):
            self.assertIsNone(poll.fetch_ledger_bindings([4924]))
        with patch.object(poll.subprocess, "run", side_effect=OSError("no helper")):
            self.assertIsNone(poll.fetch_ledger_bindings([4924]))

    def test_fetch_ledger_bindings_no_ids_reads_nothing(self):
        with patch.object(poll.subprocess, "run") as run:
            self.assertEqual(poll.fetch_ledger_bindings([]), {})
            run.assert_not_called()


class TestFetchRowsReadErrors(unittest.TestCase):
    """#8042: a failed broker read must be distinguishable from an
    empty queue — _fetch_rows still returns [] (fail-closed for
    launches) but records the failure for queue_read_errors."""

    def setUp(self):
        poll._READ_ERRORS.clear()

    def test_broker_failure_records_read_error(self):
        with patch.object(
            poll, "_broker_post", side_effect=RuntimeError("broker down: HTTP 503")
        ):
            rows = poll._fetch_rows("repo-health-code-pilot", "claimed")
        self.assertEqual(rows, [])
        errors = poll.queue_read_errors("repo-health-code-pilot")
        self.assertIn("claimed", errors)
        self.assertIn("broker down", errors["claimed"])
        self.assertEqual(poll.queue_read_errors("other-queue"), {})

    def test_successful_read_records_no_error(self):
        with patch.object(poll, "_broker_post", return_value={"rows": []}):
            rows = poll._fetch_rows("repo-health-code-pilot", "handed_off")
        self.assertEqual(rows, [])
        self.assertEqual(poll.queue_read_errors("repo-health-code-pilot"), {})

    def test_unexpected_shape_records_read_error(self):
        with patch.object(poll, "_broker_post", return_value=42):
            rows = poll._fetch_rows("repo-health-code-pilot", "claimed")
        self.assertEqual(rows, [])
        self.assertIn("claimed", poll.queue_read_errors("repo-health-code-pilot"))



class TestPollCompleteness(unittest.TestCase):
    def test_shorter_than_total_missing_rows_and_capped_are_unknown(self):
        for response in (
            {"total": 2, "rows": [{"id": 1}]},
            {"total": 0},
            [{"id": i} for i in range(poll.LIST_LIMIT)],
        ):
            with self.subTest(size=len(response)), patch.object(poll, "_broker_post", return_value=response):
                poll._READ_ERRORS.clear()
                self.assertEqual(poll.fetch_claimed_rows("fixture"), [])
                self.assertIn("claimed", poll.queue_read_errors("fixture"))
        poll._READ_ERRORS.clear()

    def test_child_timeout_uses_remaining_budget(self):
        with patch.object(poll, "_POLL_DEADLINE", 44.0), patch.object(
            poll.time, "monotonic", side_effect=[41.0, 45.0]
        ):
            self.assertEqual(poll._poll_timeout(30), 3.0)
            with self.assertRaises(TimeoutError):
                poll._poll_timeout(30)

    def test_repeated_census_cursor_is_incomplete(self):
        page = {"data": [], "cursor": {"next": "same"}}
        with patch.object(poll, "_server_get", return_value=page) as request:
            self.assertIsNone(poll.fetch_sessions())
        self.assertEqual(request.call_count, 2)

if __name__ == "__main__":
    unittest.main()
