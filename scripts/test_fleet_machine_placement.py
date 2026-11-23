#!/usr/bin/env python3
"""Tests for fleet-machine-placement.py (#2179; framework v1.1 §8).

Oracle: the machine onboarding standard
(doc/2026-11-04T05-15-02Z_design_machine-onboarding-standard-v1 —
§1.4 capabilities are declarations a scheduler places from, §1.5
silence reads as left, §2 a machine is not active before steps 5–6,
§4 draining and unplanned departure stop placement) and the
task-placement framework's v1.1 machine dimension
(doc/2026-11-03T15-10-56Z_design_task-placement-framework-v1 §8,
rules M1–M4): tier 1 = active + healthy, tier 2 = active + degraded
behind every tier-1 machine, everything else ineligible with a reason.
Run: python3 -m unittest scripts.test_fleet_machine_placement
(or execute this file directly). Requires PyYAML.
"""

import importlib.util
import json
import pathlib
import tempfile
import unittest
from datetime import datetime, timedelta, timezone

import yaml

HERE = pathlib.Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location(
    "fleet_machine_placement", HERE / "fleet-machine-placement.py"
)
placement = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(placement)

REGISTRY = HERE.parent / "doc" / "fleet-machine-registry.yaml"
NOW = datetime(2026, 11, 22, 19, 30, 0, tzinfo=timezone.utc)
FRESH = "2026-11-22T19:20:00Z"  # 10 minutes before NOW: inside 2 cadences
STALE = "2026-11-22T18:59:59Z"  # 30 minutes + 1 second before NOW: stale


def health(status, last_seen=FRESH, failed=()):
    checks = {}
    for name in placement.watchdog.GATING_CHECKS:
        checks[name] = {"state": "failed" if name in failed else "ok"}
    checks["sessions"] = {
        "state": "ok",
        "listed": 50,
        "active_within_cadence": 17,
        "max_sessions": 12,
    }
    return {"source": "watchdog", "last_seen": last_seen, "status": status, "checks": checks}


def record(state="active", health_block=None, **cap_overrides):
    capabilities = {
        "queue_worker": True,
        "opencode_sessions": True,
        "builds": True,
        "gpu": False,
    }
    capabilities.update(cap_overrides)
    return {
        "id": "amd-6",
        "state": state,
        "capabilities": capabilities,
        "capacity": {
            "max_sessions": 12,
            "cpu_cores": 16,
            "ram_gb": 123,
            "worktree_disk_gb": 3372,
        },
        "health": health_block if health_block is not None else health("healthy"),
    }


class StateGateTests(unittest.TestCase):
    """M1: only `active` may receive work, however healthy the cache looks."""

    def test_active_is_the_only_placeable_state(self):
        for state in ("declared", "joining", "draining", "retired"):
            view = placement.machine_view(record(state=state), NOW)
            self.assertFalse(view["eligible"], state)
            self.assertIsNone(view["tier"], state)
            self.assertIn(state, view["reason"] if state != "declared" else "declared")

    def test_state_gate_beats_health(self):
        # A draining machine with a fresh healthy cache still takes nothing.
        view = placement.machine_view(record(state="draining"), NOW)
        self.assertFalse(view["eligible"])
        self.assertIsNone(view["view"])  # health is not even consulted


class HealthGateTests(unittest.TestCase):
    """M2/M3: the earned health view decides the tier for active records."""

    def test_healthy_is_tier_1(self):
        view = placement.machine_view(record(), NOW)
        self.assertTrue(view["eligible"])
        self.assertEqual(view["tier"], 1)
        self.assertEqual(view["view"], "healthy")

    def test_degraded_is_tier_2_and_names_the_failed_checks(self):
        rec = record(health_block=health("degraded", failed=("disk_headroom",)))
        view = placement.machine_view(rec, NOW)
        self.assertTrue(view["eligible"])
        self.assertEqual(view["tier"], 2)
        self.assertIn("disk_headroom", view["reason"])

    def test_unreachable_is_ineligible(self):
        view = placement.machine_view(record(health_block=health("unreachable")), NOW)
        self.assertFalse(view["eligible"])
        self.assertEqual(view["view"], "unreachable")

    def test_unknown_is_ineligible(self):
        view = placement.machine_view(record(health_block=health("unknown")), NOW)
        self.assertFalse(view["eligible"])
        self.assertEqual(view["view"], "unknown")

    def test_stale_health_reads_as_left(self):
        rec = record(health_block=health("healthy", last_seen=STALE))
        view = placement.machine_view(rec, NOW)
        self.assertFalse(view["eligible"])
        self.assertEqual(view["view"], "left")

    def test_never_seen_reads_as_left(self):
        rec = record(health_block=health("unknown", last_seen=None))
        view = placement.machine_view(rec, NOW)
        self.assertFalse(view["eligible"])
        self.assertEqual(view["view"], "left")


class CapabilityTests(unittest.TestCase):
    """M4: requirements filter on declared capabilities (§1.4)."""

    def test_met_requirement_keeps_eligibility(self):
        view = placement.machine_view(record(), NOW, require=("queue_worker",))
        self.assertTrue(view["eligible"])

    def test_unmet_requirement_excludes_with_reason(self):
        view = placement.machine_view(record(), NOW, require=("gpu",))
        self.assertFalse(view["eligible"])
        self.assertIn("gpu", view["reason"])

    def test_requirement_beats_state_and_health(self):
        rec = record(state="draining")
        view = placement.machine_view(rec, NOW, require=("gpu",))
        self.assertIn("gpu", view["reason"])


class OrderingTests(unittest.TestCase):
    def test_tier_1_before_tier_2_then_ineligible(self):
        degraded = record(health_block=health("degraded", failed=("load_headroom",)))
        degraded["id"] = "aaa-degraded"
        healthy = record()
        healthy["id"] = "zzz-healthy"
        gone = record(health_block=health("healthy", last_seen=STALE))
        gone["id"] = "mmm-left"
        ordered = placement.order_views(
            [placement.machine_view(r, NOW) for r in (degraded, healthy, gone)]
        )
        self.assertEqual(
            [v["id"] for v in ordered], ["zzz-healthy", "aaa-degraded", "mmm-left"]
        )


class FactsTests(unittest.TestCase):
    def test_sessions_facts_ride_beside_the_declared_cap(self):
        view = placement.machine_view(record(), NOW)
        self.assertEqual(view["sessions"]["listed"], 50)
        self.assertEqual(view["sessions"]["active_within_cadence"], 17)
        self.assertEqual(view["sessions"]["max_sessions"], 12)


class MainTests(unittest.TestCase):
    def _registry_file(self, doc):
        fh = tempfile.NamedTemporaryFile(
            "w", suffix=".yaml", delete=False, encoding="utf-8"
        )
        yaml.safe_dump(doc, fh)
        fh.close()
        self.addCleanup(pathlib.Path(fh.name).unlink)
        return fh.name

    def _full_record(self, **overrides):
        rec = record(**overrides)
        rec.update(
            {
                "hostname": "amd",
                "roles": ["execution"],
                "links": {"talos_inventory": None, "evidence_issues": []},
                "access": {
                    "ssh": "user@amd-6",
                    "opencode_server": "http://127.0.0.1:13420",
                },
                "secrets": ["github-gh-auth"],
            }
        )
        # The validator requires the earned checks record for any
        # non-unknown status; keep the fixture minimal but valid.
        rec["health"] = {
            "source": "watchdog",
            "last_seen": None,
            "status": "unknown",
        }
        return rec

    def test_valid_registry_runs(self):
        path = self._registry_file({"machines": [self._full_record()]})
        self.assertEqual(placement.main([path, "--now", "2026-11-22T19:30:00Z"]), 0)

    def test_json_output_shape(self):
        path = self._registry_file({"machines": [self._full_record()]})
        import io
        from contextlib import redirect_stdout

        buf = io.StringIO()
        with redirect_stdout(buf):
            rc = placement.main([path, "--now", "2026-11-22T19:30:00Z", "--json"])
        self.assertEqual(rc, 0)
        out = json.loads(buf.getvalue())
        self.assertEqual(out["machines"][0]["id"], "amd-6")
        # Never seen (last_seen null) reads as left, hence not placeable.
        self.assertEqual(out["machines"][0]["view"], "left")
        self.assertEqual(out["placeable"], [])

    def test_invalid_registry_is_refused(self):
        path = self._registry_file({"machines": [{"id": "x"}]})
        self.assertEqual(placement.main([path]), 1)

    def test_missing_registry_is_refused(self):
        self.assertEqual(placement.main(["/nonexistent/registry.yaml"]), 1)

    def test_unknown_machine_is_refused(self):
        path = self._registry_file({"machines": [self._full_record()]})
        self.assertEqual(placement.main([path, "--machine", "nope"]), 1)

    def test_unknown_capability_is_a_usage_error(self):
        path = self._registry_file({"machines": [self._full_record()]})
        with self.assertRaises(SystemExit) as ctx:
            placement.main([path, "--require", "quantum"])
        self.assertEqual(ctx.exception.code, 2)

    def test_undeclared_argument_is_a_usage_error(self):
        with self.assertRaises(SystemExit) as ctx:
            placement.main(["a.yaml", "b.yaml"])
        self.assertEqual(ctx.exception.code, 2)


class CommittedRegistryTests(unittest.TestCase):
    """The committed registry always yields the view its own cache earns.

    Self-adjusting: whatever the watchdog last wrote, reading at
    last_seen + 1 minute must reproduce the recorded status (or `left`
    when the machine has never been seen), and last_seen + 31 minutes
    must read as left. Oracle: §1.5's staleness rule.
    """

    def test_committed_registry_view_matches_its_cache(self):
        doc = yaml.safe_load(REGISTRY.read_text(encoding="utf-8"))
        self.assertEqual(placement.validator.validate(doc), [])
        rec = next(r for r in doc["machines"] if r["id"] == "amd-6")
        self.assertEqual(rec["state"], "active")
        health_block = rec["health"]
        last_seen = placement.watchdog.parse_iso(health_block["last_seen"])
        if last_seen is None:
            view = placement.machine_view(rec, NOW)
            self.assertEqual(view["view"], "left")
            return
        fresh_view = placement.machine_view(rec, last_seen + timedelta(minutes=1))
        self.assertEqual(fresh_view["view"], health_block["status"])
        stale_view = placement.machine_view(rec, last_seen + timedelta(minutes=31))
        self.assertEqual(stale_view["view"], "left")
        self.assertFalse(stale_view["eligible"])


if __name__ == "__main__":
    unittest.main()
