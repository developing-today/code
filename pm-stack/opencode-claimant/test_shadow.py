#!/usr/bin/env python3
"""Safety tests for the default shadow fire."""

import sys
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import claimant


class TestShadowFire(unittest.TestCase):
    def test_shadow_reports_routes_without_entering_live_paths(self):
        candidates = [
            {
                "id": "held-task",
                "queue": "repo-health-code-pilot",
                "issue": "#12",
                "packet": {"issue": {"labels": ["do-not-merge"]}},
            },
            {
                "id": "work-task",
                "queue": "repo-health-nix-pilot",
                "issue": "#34",
                "packet": {
                    "route": {
                        "providerID": "opencode-route",
                        "modelID": "specialist",
                        "agent": "build",
                    }
                },
            },
        ]
        poll_data = {
            "candidates": candidates,
            "stuck_bound": [{"id": "bound-task", "session_id": "ses_stale"}],
            "live_root_sessions": 35,
            "active_root_sessions": 34,
            "cap": 36,
            "cap_room": 2,
        }
        logged = {}

        def capture_log(mode, seen, launched, cap_room, skips, report=None):
            logged.update({
                "mode": mode,
                "candidates_seen": seen,
                "sessions_launched": launched,
                "cap_room": cap_room,
                "skips": skips,
                **(report or {}),
            })

        with (
            patch.object(claimant, "read_mode", return_value="shadow"),
            patch.object(claimant, "run_poll", return_value=poll_data),
            patch.object(claimant, "log_fire", side_effect=capture_log),
            patch.object(claimant, "supervision_step", side_effect=AssertionError("live supervision")),
            patch.object(claimant, "gate_step", side_effect=AssertionError("live gate")),
            patch.object(claimant, "cap_step", side_effect=AssertionError("live cap")),
            patch.object(claimant, "launch_step", side_effect=AssertionError("live launch")),
        ):
            self.assertEqual(claimant.main(), 0)

        self.assertEqual(logged["mode"], "shadow")
        self.assertEqual(logged["candidates_seen"], candidates)
        self.assertEqual(logged["sessions_launched"], [])
        self.assertEqual(logged["cap_room"], 2)
        self.assertEqual(logged["cap_count"], 34)
        self.assertEqual(logged["supervision_stalls"], poll_data["stuck_bound"])
        self.assertEqual(
            logged["would_launch"],
            [{
                "task_id": "work-task",
                "queue": "repo-health-nix-pilot",
                "issue": "#34",
                "route": {
                    "providerID": "opencode-route",
                    "modelID": "specialist",
                    "agent": "build",
                },
            }],
        )
        self.assertEqual(logged["skips"][0]["task_id"], "held-task")
        self.assertEqual(logged["skips"][0]["reason"], "held_issue_label")

    def test_shadow_launch_helper_is_a_noop(self):
        with patch.object(claimant, "run_poll", side_effect=AssertionError("polled")):
            self.assertEqual(claimant.launch_step("shadow", 1), [])


if __name__ == "__main__":
    unittest.main()
