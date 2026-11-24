#!/usr/bin/env python3
"""Hermetic tests for freshness_probe.py (#8584 five-copy probe).

Oracle: the status doc the probe writes must round-trip through the
real freshness_readout.judge; a copy is stamped only when its checker
proves it, failures carry the previous stamp forward, and the alert
issue is created/updated/closed exactly once per state change.
"""

import io
import json
import os
import sys
import tempfile
import unittest
from datetime import datetime, timezone
from unittest import mock

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "bin"))
import freshness_probe
import freshness_readout

NOW = datetime(2026, 11, 24, 12, 0, tzinfo=timezone.utc)
NOW_ISO = "2026-11-24T12:00:00Z"
EXPORTED = "2026-11-24T08:40:00Z"


def ok_outcomes():
    return {
        "turso_export": freshness_probe.outcome(True, "ok", freshness_probe.freshness_readout.parse_time(EXPORTED, "t")),
        "d1_copy": freshness_probe.outcome(True, "ok", freshness_probe.freshness_readout.parse_time(EXPORTED, "t")),
        "cloudflare_rehost": freshness_probe.outcome(False, "drift"),
        "plain_mirror": freshness_probe.outcome(True, "ok", NOW),
        "r2_bundles": freshness_probe.outcome(True, "ok", NOW),
    }


class JudgeD1MetaTests(unittest.TestCase):
    def test_meta_stamps_export_time(self):
        result = freshness_probe.judge_d1_meta({"exported_at": EXPORTED}, NOW)
        self.assertTrue(result["ok"])
        self.assertEqual(freshness_probe.iso(result["verified_at"]), EXPORTED)

    def test_missing_meta_unverified(self):
        self.assertFalse(freshness_probe.judge_d1_meta(None, NOW)["ok"])
        self.assertFalse(freshness_probe.judge_d1_meta({}, NOW)["ok"])
        self.assertFalse(
            freshness_probe.judge_d1_meta({"exported_at": "junk"}, NOW)["ok"]
        )

    def test_future_meta_rejected(self):
        result = freshness_probe.judge_d1_meta(
            {"exported_at": "2026-11-24T13:00:00Z"}, NOW
        )
        self.assertFalse(result["ok"])


class HeadPairTests(unittest.TestCase):
    def test_all_match(self):
        result = freshness_probe.judge_head_pairs(
            [("a", "abc", "abc"), ("b", "def", "def")]
        )
        self.assertTrue(result["ok"])

    def test_drift_and_missing_fail_loudly(self):
        result = freshness_probe.judge_head_pairs(
            [("a", "abc", "xyz"), ("b", "def", None)]
        )
        self.assertFalse(result["ok"])
        self.assertIn("a", result["detail"])
        self.assertIn("b", result["detail"])


class BundleHeadTests(unittest.TestCase):
    def test_parse_and_newest(self):
        head_a = "a" * 40
        head_b = "b" * 40
        entries = [
            {
                "Name": head_a + "-" + "0" * 64 + ".bundle",
                "ModTime": "2026-11-20T01:00:00Z",
                "IsDir": False,
            },
            {
                "Name": head_b + "-" + "1" * 64 + ".bundle",
                "ModTime": "2026-11-22T01:00:00Z",
                "IsDir": False,
            },
            {"Name": "subdir", "ModTime": "2026-11-23T01:00:00Z", "IsDir": True},
            {
                "Name": "not-a-bundle.txt",
                "ModTime": "2026-11-23T02:00:00Z",
                "IsDir": False,
            },
        ]
        self.assertEqual(freshness_probe.newest_bundle_head(entries), head_b)
        self.assertIsNone(freshness_probe.newest_bundle_head([]))
        self.assertIsNone(freshness_probe.parse_bundle_head("short-digest.bundle"))
        self.assertIsNone(
            freshness_probe.parse_bundle_head("z" * 40 + "-" + "0" * 64 + ".bundle")
        )


def _pkt(payload):
    return b"%04x" % (len(payload) + 4) + payload


def _advertisement(head):
    return (
        _pkt(b"# service=git-upload-pack\n")
        + b"0000"
        + _pkt(head.encode() + b" HEAD\0ofs-delta symref=HEAD:refs/heads/main\n")
        + _pkt(head.encode() + b" refs/heads/main\n")
        + b"0000"
    )


class RehostParseTests(unittest.TestCase):
    def test_advertised_head_parsed(self):
        head = "a" * 40
        self.assertEqual(
            freshness_probe.parse_advertised_head(_advertisement(head)), head
        )

    def test_non_advertisements_parse_to_none(self):
        self.assertIsNone(freshness_probe.parse_advertised_head(b""))
        self.assertIsNone(freshness_probe.parse_advertised_head(b"<html>404</html>"))
        self.assertIsNone(
            freshness_probe.parse_advertised_head(_advertisement("z" * 40))
        )
        self.assertIsNone(
            freshness_probe.parse_advertised_head(
                _pkt(b"# service=git-receive-pack\n") + b"0000"
            )
        )

    def test_fetch_pairs_compares_advertised_to_mirror(self):
        head = "b" * 40
        with (
            mock.patch.object(
                freshness_probe, "_git_head", return_value=head
            ),
            mock.patch.object(
                freshness_probe,
                "_fetch_advertisement",
                return_value=_advertisement(head),
            ) as fetch,
        ):
            pairs = freshness_probe.fetch_rehost_pairs(
                "/root", "https://rehost.example"
            )
        self.assertEqual(len(pairs), len(freshness_probe.FLEET_REPOS))
        self.assertTrue(all(p[1] == p[2] == head for p in pairs))
        self.assertIn("info/refs?service=git-upload-pack", fetch.call_args[0][0])


class BuildStatusTests(unittest.TestCase):
    def test_verified_copies_stamp_and_rehost_never_does(self):
        doc = freshness_probe.build_status({}, ok_outcomes(), NOW)
        self.assertEqual(doc["copies"]["turso_export"]["last_success"], EXPORTED)
        self.assertEqual(doc["copies"]["d1_copy"]["last_success"], EXPORTED)
        self.assertEqual(doc["copies"]["plain_mirror"]["last_success"], NOW_ISO)
        self.assertIsNone(doc["copies"]["cloudflare_rehost"]["last_success"])
        results = freshness_readout.judge(doc, NOW)
        verdicts = {r["copy"]: r["verdict"] for r in results}
        self.assertEqual(verdicts["turso_export"], "PASS")
        self.assertEqual(verdicts["cloudflare_rehost"], "STALE")

    def test_failure_carries_previous_stamp_forward(self):
        previous = {
            "copies": {
                "plain_mirror": {
                    "last_success": "2026-11-23T12:00:00Z",
                    "target_hours": 26,
                }
            }
        }
        outcomes = ok_outcomes()
        outcomes["plain_mirror"] = freshness_probe.outcome(False, "drift")
        doc = freshness_probe.build_status(previous, outcomes, NOW)
        self.assertEqual(
            doc["copies"]["plain_mirror"]["last_success"], "2026-11-23T12:00:00Z"
        )
        self.assertFalse(doc["copies"]["plain_mirror"]["verified"])


class AlertSyncTests(unittest.TestCase):
    def _runner(self, existing):
        calls = []

        def runner(argv):
            calls.append(argv)
            if argv[0] == "list":
                return json.dumps(existing)
            return ""

        return runner, calls

    def _results(self, stale_names):
        return [
            {
                "copy": name,
                "verdict": "STALE" if name in stale_names else "PASS",
                "detail": "behind",
                "lag_hours": None,
                "target_hours": 26,
            }
            for name in freshness_readout.CANONICAL_COPIES
        ]

    def test_creates_when_none_open(self):
        runner, calls = self._runner([])
        action = freshness_probe.sync_alert(
            self._results({"r2_bundles"}), NOW, runner=runner
        )
        self.assertEqual(action, "opened alert issue")
        creates = [c for c in calls if c[0] == "create"]
        self.assertEqual(len(creates), 1)
        self.assertIn(freshness_probe.ALERT_MARKER, creates[0][-1])

    def test_updates_existing_instead_of_duplicating(self):
        existing = [
            {
                "number": 4242,
                "title": "Backup freshness: x behind target (#8584)",
                "body": f"<!-- {freshness_probe.ALERT_MARKER} -->",
            }
        ]
        runner, calls = self._runner(existing)
        action = freshness_probe.sync_alert(
            self._results({"r2_bundles"}), NOW, runner=runner
        )
        self.assertEqual(action, "updated alert issue #4242")
        self.assertFalse(any(c[0] == "create" for c in calls))
        self.assertTrue(any(c[0] == "comment" for c in calls))

    def test_closes_on_recovery_and_noops_when_clean(self):
        existing = [
            {
                "number": 4242,
                "title": "Backup freshness: x behind target (#8584)",
                "body": f"<!-- {freshness_probe.ALERT_MARKER} -->",
            }
        ]
        runner, calls = self._runner(existing)
        action = freshness_probe.sync_alert(self._results(set()), NOW, runner=runner)
        self.assertEqual(action, "closed alert issue #4242 on recovery")
        self.assertTrue(any(c[0] == "close" for c in calls))

        runner2, calls2 = self._runner([])
        action2 = freshness_probe.sync_alert(
            self._results(set()), NOW, runner=runner2
        )
        self.assertEqual(action2, "no alert issue needed")
        self.assertEqual([c[0] for c in calls2], ["list"])


class MainFlowTests(unittest.TestCase):
    def test_rejects_unaccounted_arguments(self):
        with mock.patch("sys.stderr", new_callable=io.StringIO):
            with self.assertRaises(SystemExit) as ctx:
                freshness_probe.main(["--status-json", "x", "--bogus"])
            self.assertEqual(ctx.exception.code, 2)

    def test_main_writes_doc_and_reports_rehost_stale(self):
        with tempfile.TemporaryDirectory() as tmp:
            status = os.path.join(tmp, "status.json")
            meta = {"exported_at": EXPORTED}
            pairs = [("developing-today/code", "abc", "abc")]
            drifted = [("developing-today/code", "abc", "xyz")]
            with (
                mock.patch.object(
                    freshness_probe, "fetch_d1_meta", return_value=meta
                ),
                mock.patch.object(
                    freshness_probe, "fetch_mirror_pairs", return_value=pairs
                ),
                mock.patch.object(
                    freshness_probe, "fetch_r2_pairs", return_value=pairs
                ),
                mock.patch.object(
                    freshness_probe, "fetch_rehost_pairs", return_value=drifted
                ),
                mock.patch("sys.stdout", new_callable=io.StringIO) as out,
            ):
                rc = freshness_probe.main(
                    ["--status-json", status, "--root", tmp, "--now", NOW_ISO]
                )
            # The rehost advertises a different HEAD than the mirror:
            # honest exit 1, and the copy is not stamped.
            self.assertEqual(rc, 1)
            self.assertIn("cloudflare_rehost: STALE", out.getvalue())
            with open(status, encoding="utf-8") as fh:
                doc = json.load(fh)
            self.assertEqual(doc["copies"]["d1_copy"]["last_success"], EXPORTED)
            self.assertIsNone(doc["copies"]["cloudflare_rehost"]["last_success"])

    def test_main_stamps_rehost_when_advertisement_matches(self):
        with tempfile.TemporaryDirectory() as tmp:
            status = os.path.join(tmp, "status.json")
            meta = {"exported_at": EXPORTED}
            pairs = [("developing-today/code", "abc", "abc")]
            with (
                mock.patch.object(
                    freshness_probe, "fetch_d1_meta", return_value=meta
                ),
                mock.patch.object(
                    freshness_probe, "fetch_mirror_pairs", return_value=pairs
                ),
                mock.patch.object(
                    freshness_probe, "fetch_r2_pairs", return_value=pairs
                ),
                mock.patch.object(
                    freshness_probe, "fetch_rehost_pairs", return_value=pairs
                ),
                mock.patch("sys.stdout", new_callable=io.StringIO) as out,
            ):
                rc = freshness_probe.main(
                    ["--status-json", status, "--root", tmp, "--now", NOW_ISO]
                )
            # Every copy verified: exit 0 and the rehost is stamped.
            self.assertEqual(rc, 0)
            self.assertIn("cloudflare_rehost: PASS", out.getvalue())
            with open(status, encoding="utf-8") as fh:
                doc = json.load(fh)
            self.assertEqual(
                doc["copies"]["cloudflare_rehost"]["last_success"], NOW_ISO
            )

    def test_fetcher_failure_keeps_probe_alive_and_loud(self):
        with tempfile.TemporaryDirectory() as tmp:
            status = os.path.join(tmp, "status.json")
            with open(status, "w", encoding="utf-8") as fh:
                json.dump(
                    {
                        "copies": {
                            "d1_copy": {
                                "last_success": EXPORTED,
                                "target_hours": 26,
                            },
                            "turso_export": {
                                "last_success": EXPORTED,
                                "target_hours": 26,
                            },
                        }
                    },
                    fh,
                )
            boom = RuntimeError("network down")
            with (
                mock.patch.object(freshness_probe, "fetch_d1_meta", side_effect=boom),
                mock.patch.object(
                    freshness_probe, "fetch_mirror_pairs", side_effect=boom
                ),
                mock.patch.object(
                    freshness_probe, "fetch_r2_pairs", side_effect=boom
                ),
                mock.patch.object(
                    freshness_probe, "fetch_rehost_pairs", side_effect=boom
                ),
                mock.patch("sys.stderr", new_callable=io.StringIO) as err,
            ):
                rc = freshness_probe.main(
                    ["--status-json", status, "--root", tmp, "--now", NOW_ISO]
                )
            self.assertEqual(rc, 1)
            self.assertIn("network down", err.getvalue())
            with open(status, encoding="utf-8") as fh:
                doc = json.load(fh)
            # Carried forward, not refreshed and not dropped.
            self.assertEqual(doc["copies"]["d1_copy"]["last_success"], EXPORTED)


if __name__ == "__main__":
    unittest.main()
