#!/usr/bin/env python3
"""Tests for validate-fleet-machine-registry.py (#2179 §3 shape).

Oracle: the registry shape defined in
doc/2026-11-04T05-15-02Z_design_machine-onboarding-standard-v1 (§3).
Run: python3 -m unittest scripts.test_validate_fleet_machine_registry
(or execute this file directly). Requires PyYAML.
"""

import copy
import importlib.util
import pathlib
import unittest

import yaml

HERE = pathlib.Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location(
    "validate_fleet_machine_registry", HERE / "validate-fleet-machine-registry.py"
)
validator = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(validator)

REGISTRY = HERE.parent / "doc" / "fleet-machine-registry.yaml"


def valid_record():
    return {
        "id": "amd-6",
        "hostname": "amd",
        "state": "active",
        "roles": ["execution"],
        "links": {"talos_inventory": None, "evidence_issues": []},
        "access": {"ssh": "user@amd-6", "opencode_server": "http://127.0.0.1:13420"},
        "capabilities": {
            "queue_worker": True,
            "opencode_sessions": True,
            "builds": True,
            "gpu": False,
        },
        "capacity": {
            "max_sessions": 12,
            "cpu_cores": 16,
            "ram_gb": 123,
            "worktree_disk_gb": 3372,
        },
        "secrets": ["github-gh-auth"],
        "health": {"source": "watchdog", "last_seen": None, "status": "unknown"},
    }


def doc_with(rec):
    return {"machines": [rec]}


class RegistryShapeTests(unittest.TestCase):
    def test_shipped_registry_is_valid(self):
        # Oracle: the §3 shape against the real artifact this PR lands.
        with open(REGISTRY, encoding="utf-8") as fh:
            self.assertEqual(validator.validate(yaml.safe_load(fh)), [])

    def test_minimal_valid_record_passes(self):
        self.assertEqual(validator.validate(doc_with(valid_record())), [])

    def test_missing_field_fails(self):
        rec = valid_record()
        del rec["capacity"]
        errors = validator.validate(doc_with(rec))
        self.assertTrue(any("capacity" in e for e in errors), errors)

    def test_unknown_state_fails(self):
        rec = valid_record()
        rec["state"] = "online"
        self.assertTrue(validator.validate(doc_with(rec)))

    def test_duplicate_id_fails(self):
        doc = {"machines": [valid_record(), copy.deepcopy(valid_record())]}
        errors = validator.validate(doc)
        self.assertTrue(any("duplicate" in e for e in errors), errors)

    def test_both_combined_with_other_roles_fails(self):
        rec = valid_record()
        rec["roles"] = ["both", "execution"]
        self.assertTrue(validator.validate(doc_with(rec)))

    def test_active_with_zero_capacity_fails(self):
        rec = valid_record()
        rec["capacity"]["cpu_cores"] = 0
        self.assertTrue(validator.validate(doc_with(rec)))

    def test_declared_with_zero_capacity_passes(self):
        # A machine that has not joined yet declares its record before its
        # capacity is measured; positivity binds only `active` records.
        rec = valid_record()
        rec["state"] = "declared"
        rec["capacity"] = {
            "max_sessions": 0,
            "cpu_cores": 0,
            "ram_gb": 0,
            "worktree_disk_gb": 0,
        }
        self.assertEqual(validator.validate(doc_with(rec)), [])

    def test_secret_value_shaped_entry_fails(self):
        rec = valid_record()
        rec["secrets"] = ["ghp_0123456789abcdef0123456789abcdef"]
        self.assertTrue(validator.validate(doc_with(rec)))

    def test_unknown_health_status_fails(self):
        rec = valid_record()
        rec["health"]["status"] = "fine"
        self.assertTrue(validator.validate(doc_with(rec)))


def earned_health(status="healthy"):
    """A health block as the watchdog writes it: status + checks record."""
    return {
        "source": "watchdog",
        "last_seen": "2026-11-22T19:00:00Z",
        "status": status,
        "checks": {
            "reported_at": "2026-11-22T19:00:00Z",
            "reachable": {"state": "ok", "ssh": True, "opencode_server_http": 200},
            "load_headroom": {"state": "ok", "load15": 6.0, "cpu_cores": 16},
            "disk_headroom": {"state": "ok", "free_pct": 42.0},
            "sessions": {"state": "ok", "listed": 50, "active_within_cadence": 3},
            "last_queue_job": {"state": "ok", "task_id": 21084},
        },
    }


class HealthEarningTests(unittest.TestCase):
    """Oracle: §1.5 — health is a watchdog-written cache; a hand-written
    pass is fabricated and must fail validation."""

    def test_watchdog_written_health_passes(self):
        rec = valid_record()
        rec["health"] = earned_health()
        self.assertEqual(validator.validate(doc_with(rec)), [])

    def test_bare_handwritten_healthy_fails(self):
        rec = valid_record()
        rec["health"] = {"source": "watchdog", "last_seen": None, "status": "healthy"}
        errors = validator.validate(doc_with(rec))
        self.assertTrue(any("fabricated pass" in e for e in errors), errors)

    def test_status_with_last_seen_but_no_checks_fails(self):
        rec = valid_record()
        rec["health"] = {
            "source": "watchdog",
            "last_seen": "2026-11-22T19:00:00Z",
            "status": "healthy",
        }
        self.assertTrue(validator.validate(doc_with(rec)))

    def test_last_seen_without_checks_fails_even_when_unknown(self):
        rec = valid_record()
        rec["health"] = {
            "source": "watchdog",
            "last_seen": "2026-11-22T19:00:00Z",
            "status": "unknown",
        }
        self.assertTrue(validator.validate(doc_with(rec)))

    def test_partial_checks_record_fails(self):
        rec = valid_record()
        health = earned_health()
        del health["checks"]["sessions"]
        rec["health"] = health
        self.assertTrue(validator.validate(doc_with(rec)))

    def test_unparseable_last_seen_fails(self):
        rec = valid_record()
        health = earned_health()
        health["last_seen"] = "yesterday-ish"
        rec["health"] = health
        self.assertTrue(validator.validate(doc_with(rec)))

    def test_check_with_bad_state_fails(self):
        rec = valid_record()
        health = earned_health()
        health["checks"]["reachable"] = {"state": "probably"}
        rec["health"] = health
        self.assertTrue(validator.validate(doc_with(rec)))

    def test_unreachable_with_checks_record_passes(self):
        # An earned negative is valid health data, not a shape error.
        rec = valid_record()
        health = earned_health(status="unreachable")
        health["last_seen"] = "2026-11-22T18:00:00Z"  # last sighting stands
        health["checks"]["reachable"] = {
            "state": "failed",
            "ssh": True,
            "opencode_server_http": None,
        }
        rec["health"] = health
        self.assertEqual(validator.validate(doc_with(rec)), [])


def with_offers():
    rec = valid_record()
    rec["transports"] = [
        {
            "name": "commander-door",
            "kind": "commander-door",
            "endpoint": {"url": "https://commander.example/mcp"},
            "credentials": ["desktop-commander.machine-bearer"],
        },
        {
            "name": "ssh",
            "kind": "ssh",
            "endpoint": {"target": "user@amd-6", "port": 22},
            "credentials": ["ssh.operator-key"],
        },
    ]
    rec["enrollment"] = {
        "opencode-lanes": ["commander-door", "ssh"],
        "external-mcp-clients": ["commander-door"],
    }
    return rec


class TransportOfferTests(unittest.TestCase):
    """#14714 — transport offers + enrollment (pooling design §1.2)."""

    def errors(self, rec):
        return validator.validate(doc_with(rec))

    def test_record_with_offers_passes(self):
        self.assertEqual(self.errors(with_offers()), [])

    def test_offers_are_optional(self):
        # Records that predate #14714 stay valid: no consumer change.
        self.assertNotIn("transports", valid_record())
        self.assertEqual(self.errors(valid_record()), [])

    def test_shipped_amd_record_carries_its_current_legs(self):
        with open(REGISTRY, encoding="utf-8") as fh:
            doc = yaml.safe_load(fh)
        amd = next(m for m in doc["machines"] if m["id"] == "amd-6")
        kinds = {o["kind"] for o in amd["transports"]}
        self.assertEqual(kinds, {"commander-door", "laptop-agent", "tsnet", "ssh"})
        names = {o["name"] for o in amd["transports"]}
        for offers in amd["enrollment"].values():
            self.assertTrue(set(offers) <= names)

    def test_round_trip_through_yaml_is_lossless_and_valid(self):
        rec = with_offers()
        text = yaml.safe_dump(doc_with(rec), sort_keys=False)
        back = yaml.safe_load(text)
        self.assertEqual(back, doc_with(rec))
        self.assertEqual(validator.validate(back), [])
        with open(REGISTRY, encoding="utf-8") as fh:
            shipped = yaml.safe_load(fh)
        again = yaml.safe_load(yaml.safe_dump(shipped, sort_keys=False))
        self.assertEqual(again, shipped)
        self.assertEqual(validator.validate(again), [])

    def test_unknown_kind_fails(self):
        rec = with_offers()
        rec["transports"][0]["kind"] = "carrier-pigeon"
        errors = self.errors(rec)
        self.assertTrue(any("kind 'carrier-pigeon'" in e for e in errors), errors)

    def test_credential_value_instead_of_name_fails(self):
        for value in (
            "ghp_16C7e42F292c6912E7710c838347Ae178B4a",
            "Bearer abc",
            "client_secret=s3cr3t",
            "a3f9c2e1b4d7a3f9c2e1b4d7a3f9c2e1",
            "token.9f8e7d6c5b4a39f8e7d6c5b4",
        ):
            rec = with_offers()
            rec["transports"][0]["credentials"] = [value]
            errors = self.errors(rec)
            self.assertTrue(any("credential name" in e for e in errors), (value, errors))

    def test_credential_mapping_fails(self):
        rec = with_offers()
        rec["transports"][0]["credentials"] = [
            {"client_id": "id.access", "client_secret": "s"}
        ]
        self.assertTrue(any("credential name" in e for e in self.errors(rec)))

    def test_secret_key_beside_the_offer_fails(self):
        rec = with_offers()
        rec["transports"][0]["token"] = "abc"
        errors = self.errors(rec)
        self.assertTrue(any("unexpected keys ['token']" in e for e in errors), errors)

    def test_secret_in_endpoint_fails(self):
        rec = with_offers()
        rec["transports"][0]["endpoint"]["bearer"] = "abc"
        self.assertTrue(self.errors(rec))
        rec = with_offers()
        rec["transports"][0]["endpoint"]["url"] = "https://u:p@commander.example/mcp"
        self.assertTrue(self.errors(rec))
        rec = with_offers()
        rec["transports"][0]["endpoint"]["url"] = "https://commander.example/mcp?key=x"
        self.assertTrue(self.errors(rec))

    def test_offer_without_credential_fails_except_iroh(self):
        rec = with_offers()
        rec["transports"][1]["credentials"] = []
        self.assertTrue(self.errors(rec))
        rec = with_offers()
        rec["transports"].append(
            {
                "name": "iroh",
                "kind": "iroh",
                "endpoint": {"node_id": "ae58ff8833241ac82d6ff7611046ed67"},
                "credentials": [],
            }
        )
        self.assertEqual(self.errors(rec), [])

    def test_kind_specific_endpoint_is_required(self):
        rec = with_offers()
        rec["transports"][1]["endpoint"] = {"port": 22}
        errors = self.errors(rec)
        self.assertTrue(any("must name ['target']" in e for e in errors), errors)

    def test_duplicate_offer_name_fails(self):
        rec = with_offers()
        rec["transports"][1]["name"] = "commander-door"
        self.assertTrue(any("duplicate offer" in e for e in self.errors(rec)))

    def test_enrollment_must_reference_offers_of_the_record(self):
        rec = with_offers()
        rec["enrollment"]["opencode-lanes"].append("tsnet")
        errors = self.errors(rec)
        self.assertTrue(any("'tsnet' is not an offer" in e for e in errors), errors)

    def test_enrollment_without_transports_fails(self):
        rec = with_offers()
        del rec["transports"]
        self.assertTrue(any("enrollment without" in e for e in self.errors(rec)))


if __name__ == "__main__":
    unittest.main()
