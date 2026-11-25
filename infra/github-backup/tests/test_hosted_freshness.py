#!/usr/bin/env python3
"""Hermetic tests for hosted_freshness.py (#15001 hosted probe).

Oracle: with fake fetchers, every leg of every repo is classified
exactly as the module docstring defines — fresh only on proof (head
equality for git legs, in-SLO timestamped proof for r2_bundles and
d1_export), stale for a behind or over-SLO copy, missing for no copy,
failed for a check that could not run — and a leg's rollup across the
inventory distinguishes partial from stale. No network, no gh, no
rclone: the fetchers are the seam.

Run: python3 infra/github-backup/tests/test_hosted_freshness.py
"""

import os
import sys
import unittest
from datetime import datetime, timedelta, timezone

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "bin"))
import hosted_freshness as hf

NOW = datetime(2026, 11, 25, 12, 0, tzinfo=timezone.utc)
REPOS = ("developing-today/code", "developing-today/fleet")
HEAD_A = "a" * 40
HEAD_B = "b" * 40


def fetchers(**overrides):
    base = {
        "source_head": lambda repo: HEAD_A,
        "mirror_head": lambda repo: HEAD_A,
        "bundle": lambda repo: (HEAD_A, NOW - timedelta(hours=2)),
        "rehost_head": lambda repo: HEAD_A,
        "d1_meta": lambda: {"exported_at": "2026-11-25T10:00:00Z"},
    }
    base.update(overrides)
    return base


def leg_status(report, leg, repo=None):
    entry = report["legs"][leg]
    if repo is None:
        return entry["status"]
    return entry["repos"][repo]["status"]


class AllFreshTests(unittest.TestCase):
    def test_all_legs_fresh_verdict_fresh(self):
        report = hf.build_report(REPOS, fetchers(), NOW, "run-1")
        self.assertEqual(report["verdict"], "fresh")
        for leg in ("source", "plain_mirror", "r2_bundles", "cloudflare_rehost", "d1_export"):
            self.assertEqual(leg_status(report, leg), hf.FRESH, leg)
        self.assertEqual(report["probe"], "hosted-freshness:v1")
        self.assertEqual(report["run_id"], "run-1")
        for leg in ("plain_mirror", "r2_bundles", "cloudflare_rehost", "d1_export"):
            self.assertIn("slo_hours", report["legs"][leg])


class GitLegTests(unittest.TestCase):
    def test_mirror_behind_source_is_stale(self):
        report = hf.build_report(
            REPOS, fetchers(mirror_head=lambda repo: HEAD_B), NOW, "r"
        )
        self.assertEqual(leg_status(report, "plain_mirror", REPOS[0]), hf.STALE)
        self.assertEqual(leg_status(report, "plain_mirror"), hf.STALE)
        self.assertEqual(report["verdict"], "attention")

    def test_mirror_absent_is_missing(self):
        report = hf.build_report(
            REPOS, fetchers(mirror_head=lambda repo: None), NOW, "r"
        )
        self.assertEqual(leg_status(report, "plain_mirror", REPOS[0]), hf.MISSING)
        self.assertEqual(leg_status(report, "plain_mirror"), hf.MISSING)

    def test_mirror_check_error_is_failed_not_fresh(self):
        def boom(repo):
            raise RuntimeError("gh api 500")

        report = hf.build_report(REPOS, fetchers(mirror_head=boom), NOW, "r")
        self.assertEqual(leg_status(report, "plain_mirror", REPOS[0]), hf.FAILED)
        self.assertEqual(leg_status(report, "plain_mirror"), hf.FAILED)

    def test_one_repo_fresh_one_stale_rolls_up_partial(self):
        def mirror(repo):
            return HEAD_A if repo == REPOS[0] else HEAD_B

        report = hf.build_report(REPOS, fetchers(mirror_head=mirror), NOW, "r")
        self.assertEqual(leg_status(report, "plain_mirror"), hf.PARTIAL)

    def test_rehost_unpublished_is_missing(self):
        report = hf.build_report(
            REPOS, fetchers(rehost_head=lambda repo: None), NOW, "r"
        )
        self.assertEqual(leg_status(report, "cloudflare_rehost"), hf.MISSING)


class BundleLegTests(unittest.TestCase):
    def test_bundle_naming_source_head_within_slo_is_fresh(self):
        report = hf.build_report(REPOS, fetchers(), NOW, "r")
        entry = report["legs"]["r2_bundles"]["repos"][REPOS[0]]
        self.assertEqual(entry["status"], hf.FRESH)
        self.assertEqual(entry["copy_head"], HEAD_A)
        self.assertIn("bundle_at", entry)

    def test_bundle_naming_old_head_is_stale(self):
        old = (HEAD_B, NOW - timedelta(hours=1))
        report = hf.build_report(REPOS, fetchers(bundle=lambda repo: old), NOW, "r")
        self.assertEqual(leg_status(report, "r2_bundles", REPOS[0]), hf.STALE)

    def test_bundle_beyond_slo_is_stale_even_at_source_head(self):
        aged = (HEAD_A, NOW - timedelta(hours=30))
        report = hf.build_report(REPOS, fetchers(bundle=lambda repo: aged), NOW, "r")
        self.assertEqual(leg_status(report, "r2_bundles", REPOS[0]), hf.STALE)

    def test_no_bundle_is_missing(self):
        report = hf.build_report(
            REPOS, fetchers(bundle=lambda repo: None), NOW, "r"
        )
        self.assertEqual(leg_status(report, "r2_bundles"), hf.MISSING)

    def test_newest_bundle_picks_latest_valid_object(self):
        entries = [
            {"Name": f"{HEAD_B}-{'c' * 64}.bundle", "ModTime": "2026-11-25T08:00:00Z", "IsDir": False},
            {"Name": f"{HEAD_A}-{'d' * 64}.bundle", "ModTime": "2026-11-25T10:00:00Z", "IsDir": False},
            {"Name": "not-a-bundle.txt", "ModTime": "2026-11-25T11:00:00Z", "IsDir": False},
            {"Name": f"{HEAD_B}-{'e' * 64}.bundle", "ModTime": "2026-11-25T11:30:00Z", "IsDir": True},
        ]
        head, mod = hf.newest_bundle(entries)
        self.assertEqual(head, HEAD_A)
        self.assertEqual(mod, datetime(2026, 11, 25, 10, 0, tzinfo=timezone.utc))


class D1LegTests(unittest.TestCase):
    def test_recent_export_is_fresh(self):
        report = hf.build_report(REPOS, fetchers(), NOW, "r")
        self.assertEqual(leg_status(report, "d1_export"), hf.FRESH)
        self.assertEqual(report["legs"]["d1_export"]["exported_at"], "2026-11-25T10:00:00Z")

    def test_old_export_is_stale(self):
        meta = lambda: {"exported_at": "2026-11-23T10:00:00Z"}
        report = hf.build_report(REPOS, fetchers(d1_meta=meta), NOW, "r")
        self.assertEqual(leg_status(report, "d1_export"), hf.STALE)

    def test_no_meta_is_missing(self):
        report = hf.build_report(REPOS, fetchers(d1_meta=lambda: None), NOW, "r")
        self.assertEqual(leg_status(report, "d1_export"), hf.MISSING)

    def test_unusable_meta_is_failed(self):
        meta = lambda: {"exported_at": "not-a-time"}
        report = hf.build_report(REPOS, fetchers(d1_meta=meta), NOW, "r")
        self.assertEqual(leg_status(report, "d1_export"), hf.FAILED)

    def test_future_meta_is_failed(self):
        meta = lambda: {"exported_at": "2026-11-25T13:00:00Z"}
        report = hf.build_report(REPOS, fetchers(d1_meta=meta), NOW, "r")
        self.assertEqual(leg_status(report, "d1_export"), hf.FAILED)

    def test_meta_fetch_error_is_failed(self):
        def boom():
            raise RuntimeError("D1 database not found")

        report = hf.build_report(REPOS, fetchers(d1_meta=boom), NOW, "r")
        self.assertEqual(leg_status(report, "d1_export"), hf.FAILED)


class SourceFailureTests(unittest.TestCase):
    def test_source_failure_fails_copy_legs_but_not_d1(self):
        def boom(repo):
            raise RuntimeError("gh api unreachable")

        report = hf.build_report(REPOS, fetchers(source_head=boom), NOW, "r")
        for leg in ("plain_mirror", "r2_bundles", "cloudflare_rehost"):
            self.assertEqual(leg_status(report, leg, REPOS[0]), hf.FAILED, leg)
        self.assertEqual(leg_status(report, "source"), hf.FAILED)
        self.assertEqual(leg_status(report, "d1_export"), hf.FRESH)
        self.assertEqual(report["verdict"], "attention")


class RollupTests(unittest.TestCase):
    def test_rollup_matrix(self):
        self.assertEqual(hf.rollup([hf.FRESH, hf.FRESH]), hf.FRESH)
        self.assertEqual(hf.rollup([hf.MISSING, hf.MISSING]), hf.MISSING)
        self.assertEqual(hf.rollup([hf.FAILED, hf.FAILED]), hf.FAILED)
        self.assertEqual(hf.rollup([hf.FRESH, hf.STALE]), hf.PARTIAL)
        self.assertEqual(hf.rollup([hf.FRESH, hf.MISSING]), hf.PARTIAL)
        self.assertEqual(hf.rollup([hf.STALE, hf.MISSING]), hf.STALE)
        self.assertEqual(hf.rollup([hf.STALE, hf.FAILED]), hf.STALE)
        self.assertEqual(hf.rollup([]), hf.FAILED)


class CliTests(unittest.TestCase):
    def test_unknown_argument_is_usage_error(self):
        with self.assertRaises(SystemExit) as ctx:
            hf.parse_args(["--definitely-not-a-flag"])
        self.assertEqual(ctx.exception.code, 2)


if __name__ == "__main__":
    unittest.main(verbosity=2)
