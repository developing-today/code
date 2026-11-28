#!/usr/bin/env python3
"""Tests for fleet-dns-cold-start.py (#14717).

Oracle: transport & pooling design §1.2 — DNS under our own zones
publishes the stable public doors only; DNS carries offers, never
secrets, and is never a liveness source.
Run: python3 -m unittest scripts.test_fleet_dns_cold_start
"""

import copy
import importlib.util
import pathlib
import unittest

import yaml

HERE = pathlib.Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location(
    "fleet_dns_cold_start", HERE / "fleet-dns-cold-start.py"
)
dns = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(dns)

REGISTRY = HERE.parent / "doc" / "fleet-machine-registry.yaml"
ZONE_FILE = HERE.parent / "doc" / "fleet-dns-cold-start.zone"


def shipped_doc():
    with open(REGISTRY, encoding="utf-8") as fh:
        return yaml.safe_load(fh)


def record(**overrides):
    rec = {
        "id": "amd-6",
        "state": "active",
        "secrets": ["github-gh-auth"],
        "health": {
            "source": "watchdog",
            "status": "healthy",
            "last_seen": "2026-11-25T00:00:00Z",
        },
        "transports": [
            {
                "name": "commander-door",
                "kind": "commander-door",
                "endpoint": {"url": "https://commander.security.cab/mcp"},
                "credentials": ["desktop-commander.machine-bearer"],
            },
            {
                "name": "laptop-agent-tailnet",
                "kind": "tsnet",
                "endpoint": {"address": "100.99.113.103", "port": 4799},
                "credentials": ["laptop-agent-token"],
            },
            {
                "name": "ssh",
                "kind": "ssh",
                "endpoint": {"target": "user@amd-6", "port": 22},
                "credentials": ["ssh.operator-key"],
            },
        ],
    }
    rec.update(overrides)
    return rec


class PublicationTests(unittest.TestCase):
    def test_shipped_registry_publishes_only_its_public_doors(self):
        records = dns.publish_records(shipped_doc())
        values = [v for _, v in records]
        self.assertTrue(
            any("url=https://commander.security.cab/mcp" in v for v in values), values
        )
        self.assertTrue(
            any("url=https://sandbox-link.security.cab" in v for v in values), values
        )
        self.assertFalse(
            any("100.99" in v or "user@amd-6" in v for v in values), values
        )

    def test_shipped_zone_file_matches_regeneration(self):
        expected = dns.format_zone(dns.publish_records(shipped_doc()))
        with open(ZONE_FILE, encoding="utf-8") as fh:
            self.assertEqual(fh.read(), expected)

    def test_only_public_https_doors_are_published(self):
        records = dns.publish_records({"machines": [record()]})
        self.assertEqual(len(records), 1)
        name, value = records[0]
        self.assertEqual(name, "_doors.amd-6.fleet.security.cab")
        self.assertIn("kind=commander-door", value)

    def test_no_credential_or_secret_name_is_ever_published(self):
        zone = dns.format_zone(dns.publish_records(shipped_doc()))
        for rec in shipped_doc()["machines"]:
            for name in rec.get("secrets") or []:
                self.assertNotIn(name, zone)
            for offer in rec.get("transports") or []:
                for cred in offer.get("credentials") or []:
                    self.assertNotIn(cred, zone)

    def test_no_liveness_is_published(self):
        doc = {"machines": [record()]}
        before = dns.publish_records(doc)
        doc["machines"][0]["health"] = {
            "source": "watchdog",
            "status": "unreachable",
            "last_seen": "2030-01-01T00:00:00Z",
        }
        self.assertEqual(dns.publish_records(doc), before)
        self.assertNotIn("last_seen", dns.format_zone(before))
        self.assertNotIn("healthy", dns.format_zone(before))

    def test_non_active_and_non_public_offers_are_skipped(self):
        self.assertEqual(
            dns.publish_records({"machines": [record(state="retired")]}), []
        )
        http = record()
        http["transports"][0]["endpoint"]["url"] = "http://commander.security.cab/mcp"
        self.assertEqual(dns.publish_records({"machines": [http]}), [])
        tailnet_named = record()
        tailnet_named["transports"][0]["endpoint"]["url"] = (
            "https://amd-6.example.ts.net/mcp"
        )
        self.assertEqual(dns.publish_records({"machines": [tailnet_named]}), [])

    def test_publication_is_deterministic(self):
        doc = shipped_doc()
        self.assertEqual(
            dns.publish_records(doc), dns.publish_records(copy.deepcopy(doc))
        )


class ResolverTests(unittest.TestCase):
    def test_round_trip_publish_then_resolve(self):
        records = dns.publish_records({"machines": [record()]})
        lines = dns.format_zone(records).splitlines()
        candidates = dns.resolve_candidates(lines)
        self.assertEqual(len(candidates), 1)
        cand = candidates[0]
        self.assertEqual(cand["url"], "https://commander.security.cab/mcp")
        self.assertEqual(cand["source"], "dns")
        # DNS is never a liveness source: a candidate is an offer to try.
        self.assertIs(cand["liveness"], False)

    def test_resolver_rejects_credential_material(self):
        bad = (
            "_doors.amd-6.fleet.security.cab. 300 IN TXT "
            '"v=fleet-door1; machine=amd-6; offer=x; kind=commander-door; '
            'url=https://commander.security.cab/mcp; token=abc123"'
        )
        with self.assertRaises(ValueError):
            dns.resolve_candidates([bad])

    def test_resolver_rejects_liveness_claims_and_non_public_urls(self):
        for extra in ("last_seen=2026-11-25T00:00:00Z", "status=healthy"):
            bad = (
                f"v=fleet-door1; machine=amd-6; offer=x; kind=commander-door; "
                f"url=https://commander.security.cab/mcp; {extra}"
            )
            with self.assertRaises(ValueError, msg=extra):
                dns.parse_record_value(bad)
        with self.assertRaises(ValueError):
            dns.parse_record_value(
                "v=fleet-door1; machine=amd-6; offer=x; kind=tsnet; "
                "url=https://100.99.113.103:4799"
            )


if __name__ == "__main__":
    unittest.main()
