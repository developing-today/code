#!/usr/bin/env python3
"""Tests for fleet-machine-watchdog.py (#2179 §1.5 health reporting).

Oracle: the machine onboarding standard
(doc/2026-11-04T05-15-02Z_design_machine-onboarding-standard-v1, §1.5
and §3): health is earned by checks that actually ran; a check that
could not run yields unknown with a reason, never a guessed healthy;
silence (stale last_seen) reads as left the pool.
Run: python3 -m unittest scripts.test_fleet_machine_watchdog
(or execute this file directly). Requires PyYAML.
"""

import importlib.util
import pathlib
import unittest
from datetime import datetime, timedelta, timezone

import yaml

HERE = pathlib.Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location(
    "fleet_machine_watchdog", HERE / "fleet-machine-watchdog.py"
)
watchdog = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(watchdog)

REGISTRY = HERE.parent / "doc" / "fleet-machine-registry.yaml"
NOW = datetime(2026, 11, 22, 19, 30, 0, tzinfo=timezone.utc)
PRIOR = {"source": "watchdog", "last_seen": "2026-11-22T19:00:00Z", "status": "healthy"}


def record():
    return {
        "id": "amd-6",
        "capacity": {
            "max_sessions": 12,
            "cpu_cores": 16,
            "ram_gb": 123,
            "worktree_disk_gb": 3372,
        },
    }


def facts(**overrides):
    base = {
        "ssh": True,
        "opencode_server_http": 200,
        "load": [3.0, 4.0, 6.0],
        "disk": {"total_gb": 3372, "free_gb": 1400, "free_pct": 41.5},
        "sessions_listed": 50,
        "sessions_active": 4,
    }
    base.update(overrides)
    return base


QUEUE_OK = (
    {
        "task_id": 21084,
        "queue": "repo-health-code-pilot",
        "worker": "repo-worker-code",
        "completed_at": "2026-11-22T19:12:00Z",
    },
    None,
)


def build(f, terr=None, queue=QUEUE_OK, prior=None):
    return watchdog.build_health(
        record(),
        f,
        terr,
        queue[0],
        queue[1],
        NOW,
        prior if prior is not None else PRIOR,
    )


class BuildHealthTests(unittest.TestCase):
    def test_all_checks_ok_is_healthy_and_seen(self):
        health = build(facts())
        self.assertEqual(health["status"], "healthy")
        self.assertEqual(health["last_seen"], "2026-11-22T19:30:00Z")
        for name in watchdog.ALL_CHECKS:
            self.assertEqual(health["checks"][name]["state"], "ok", name)

    def test_low_disk_headroom_is_degraded(self):
        health = build(facts(disk={"total_gb": 3372, "free_gb": 183, "free_pct": 5.4}))
        self.assertEqual(health["checks"]["disk_headroom"]["state"], "failed")
        self.assertEqual(health["status"], "degraded")
        self.assertEqual(health["last_seen"], "2026-11-22T19:30:00Z")

    def test_saturated_load_is_degraded(self):
        health = build(facts(load=[20.0, 19.0, 17.0]))
        self.assertEqual(health["checks"]["load_headroom"]["state"], "failed")
        self.assertEqual(health["status"], "degraded")

    def test_session_counts_are_recorded_not_gated_on_the_cap(self):
        # Updated-within-a-cadence is an activity rate, not concurrency;
        # the declared cap is a placement limit (§1.3). The counts are
        # recorded as facts and the check earns ok by the API answering.
        health = build(facts(sessions_active=13))
        self.assertEqual(health["checks"]["sessions"]["state"], "ok")
        self.assertEqual(health["checks"]["sessions"]["active_within_cadence"], 13)
        self.assertEqual(health["checks"]["sessions"]["max_sessions"], 12)
        self.assertEqual(health["status"], "healthy")

    def test_server_not_responding_is_unreachable_and_last_seen_stands(self):
        health = build(facts(opencode_server_http=None))
        self.assertEqual(health["checks"]["reachable"]["state"], "failed")
        self.assertEqual(health["status"], "unreachable")
        # The last sighting is not advanced by a run that could not see it.
        self.assertEqual(health["last_seen"], PRIOR["last_seen"])

    def test_transport_failure_is_unknown_with_reason(self):
        health = build(None, terr="fleet shell transport failed: timed out")
        self.assertEqual(health["status"], "unknown")
        self.assertEqual(health["last_seen"], PRIOR["last_seen"])
        for name in watchdog.ALL_CHECKS:
            self.assertEqual(health["checks"][name]["state"], "unknown", name)
            self.assertIn("timed out", health["checks"][name]["reason"])

    def test_unrunnable_gating_check_blocks_healthy(self):
        health = build(
            facts(
                sessions_listed=None,
                sessions_active=None,
                sessions_error="attach API HTTP 503",
            )
        )
        self.assertEqual(health["checks"]["sessions"]["state"], "unknown")
        self.assertEqual(health["status"], "unknown")
        # The machine WAS seen; only its health is unearned.
        self.assertEqual(health["last_seen"], "2026-11-22T19:30:00Z")

    def test_queue_check_is_informational_not_a_gate(self):
        health = build(facts(), queue=(None, "queue store settings unavailable"))
        self.assertEqual(health["checks"]["last_queue_job"]["state"], "unknown")
        self.assertEqual(health["status"], "healthy")


class PlacementViewTests(unittest.TestCase):
    def test_fresh_record_reads_as_its_status(self):
        health = {"last_seen": "2026-11-22T19:20:00Z", "status": "degraded"}
        self.assertEqual(watchdog.placement_view(health, NOW), "degraded")

    def test_stale_record_reads_as_left(self):
        # 60m01s old: just past two cadences at the 30-minute cadence.
        health = {"last_seen": "2026-11-22T18:29:59Z", "status": "healthy"}
        self.assertEqual(watchdog.placement_view(health, NOW), "left")

    def test_never_seen_reads_as_left(self):
        health = {"last_seen": None, "status": "unknown"}
        self.assertEqual(watchdog.placement_view(health, NOW), "left")


class ReplaceHealthBlockTests(unittest.TestCase):
    def test_only_the_health_block_changes(self):
        text = REGISTRY.read_text(encoding="utf-8")
        health = build(facts())
        new_text = watchdog.replace_health_block(text, "amd-6", health)
        old_lines = text.splitlines()
        new_lines = new_text.splitlines()
        # Everything before the health block is byte-identical.
        hstart = next(i for i, l in enumerate(old_lines) if l.startswith("    health:"))
        self.assertEqual(new_lines[:hstart], old_lines[:hstart])
        # The result parses, validates, and carries the earned record.
        doc = yaml.safe_load(new_text)
        self.assertEqual(watchdog.validator.validate(doc), [])
        written = doc["machines"][0]["health"]
        self.assertEqual(written["status"], "healthy")
        self.assertEqual(written["checks"]["sessions"]["listed"], 50)
        # Declared fields untouched.
        self.assertEqual(doc["machines"][0]["capacity"]["cpu_cores"], 16)
        self.assertEqual(doc["machines"][0]["state"], "active")

    def test_unknown_machine_raises(self):
        text = REGISTRY.read_text(encoding="utf-8")
        with self.assertRaises(ValueError):
            watchdog.replace_health_block(text, "no-such-machine", build(facts()))


if __name__ == "__main__":
    unittest.main()
