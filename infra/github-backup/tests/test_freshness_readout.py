#!/usr/bin/env python3
"""Hermetic tests for freshness_readout.py (#8584 five-copy readout)."""

import io
import json
import os
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "bin"))
import freshness_readout


NOW = "2026-11-24T08:00:00Z"


class FreshnessReadoutTests(unittest.TestCase):
    def _status_file(self, doc):
        fh = tempfile.NamedTemporaryFile(
            mode="w", suffix=".json", delete=False, encoding="utf-8"
        )
        json.dump(doc, fh)
        fh.close()
        self.addCleanup(os.unlink, fh.name)
        return fh.name

    def _fresh_doc(self):
        return {
            "generated_at": NOW,
            "copies": {
                name: {"last_success": "2026-11-24T07:00:00Z", "target_hours": 26}
                for name in freshness_readout.CANONICAL_COPIES
            },
        }

    def test_rejects_unaccounted_arguments_before_reading(self):
        with (
            mock.patch("builtins.open") as fake_open,
            mock.patch("sys.stderr", new_callable=io.StringIO),
        ):
            with self.assertRaises(SystemExit) as ctx:
                freshness_readout.main(["--status-json", "x", "--unexpected"])
            self.assertEqual(ctx.exception.code, 2)
            fake_open.assert_not_called()

    def test_all_fresh_passes(self):
        path = self._status_file(self._fresh_doc())
        with mock.patch("sys.stdout", new_callable=io.StringIO) as out:
            self.assertEqual(
                freshness_readout.main(["--status-json", path, "--now", NOW]),
                0,
            )
        self.assertIn("all five copies fresh", out.getvalue())

    def test_stale_copy_fails_and_names_it(self):
        doc = self._fresh_doc()
        doc["copies"]["r2_bundles"]["last_success"] = "2026-11-20T07:00:00Z"
        path = self._status_file(doc)
        with (
            mock.patch("sys.stdout", new_callable=io.StringIO) as out,
            mock.patch("sys.stderr", new_callable=io.StringIO) as err,
        ):
            self.assertEqual(
                freshness_readout.main(["--status-json", path, "--now", NOW]),
                1,
            )
        self.assertIn("r2_bundles: STALE", out.getvalue())
        self.assertIn("r2_bundles", err.getvalue())

    def test_missing_canonical_copy_is_stale(self):
        doc = self._fresh_doc()
        del doc["copies"]["cloudflare_rehost"]
        path = self._status_file(doc)
        with mock.patch("sys.stdout", new_callable=io.StringIO) as out:
            self.assertEqual(
                freshness_readout.main(["--status-json", path, "--now", NOW]),
                1,
            )
        self.assertIn("cloudflare_rehost: STALE", out.getvalue())

    def test_never_verified_is_stale(self):
        doc = self._fresh_doc()
        doc["copies"]["d1_copy"]["last_success"] = None
        path = self._status_file(doc)
        with mock.patch("sys.stdout", new_callable=io.StringIO) as out:
            self.assertEqual(
                freshness_readout.main(["--status-json", path, "--now", NOW]),
                1,
            )
        self.assertIn("d1_copy: STALE", out.getvalue())

    def test_bad_timestamp_is_invalid_failure(self):
        doc = self._fresh_doc()
        doc["copies"]["plain_mirror"]["last_success"] = "not-a-time"
        path = self._status_file(doc)
        with mock.patch("sys.stdout", new_callable=io.StringIO) as out:
            self.assertEqual(
                freshness_readout.main(["--status-json", path, "--now", NOW]),
                1,
            )
        self.assertIn("plain_mirror: INVALID", out.getvalue())

    def test_unreadable_status_file_fails(self):
        with mock.patch("sys.stderr", new_callable=io.StringIO) as err:
            self.assertEqual(
                freshness_readout.main(
                    ["--status-json", "/nonexistent-8584.json", "--now", NOW]
                ),
                1,
            )
        self.assertIn("cannot read status file", err.getvalue())

    def test_json_format_emits_machine_readable_results(self):
        path = self._status_file(self._fresh_doc())
        with mock.patch("sys.stdout", new_callable=io.StringIO) as out:
            self.assertEqual(
                freshness_readout.main(
                    ["--status-json", path, "--now", NOW, "--format", "json"]
                ),
                0,
            )
        payload = json.loads(out.getvalue())
        self.assertEqual(
            sorted(r["copy"] for r in payload["results"]),
            sorted(freshness_readout.CANONICAL_COPIES),
        )
        self.assertTrue(all(r["verdict"] == "PASS" for r in payload["results"]))


if __name__ == "__main__":
    unittest.main()
