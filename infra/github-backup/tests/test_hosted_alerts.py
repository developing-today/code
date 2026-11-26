#!/usr/bin/env python3
"""Hermetic tests for hosted_alerts.py (#15001 alerting slice).

Oracle: the planner is a pure function of (hosted_freshness report,
existing open issues). Exactly one open alert may exist per unhealthy
(leg, repo): a first unhealthy run creates it, a later run with new
evidence updates that same issue (never a second create), re-planning
the same run is a noop, and a run that verifies the pair fresh
resolves (closes) the open alert. stale / missing / partial / failed
stay distinct in titles and bodies, every alert body carries the
report's run identity as evidence, and the inventory-global d1_export
leg alerts under repo "-". No network, no gh: the executor is the seam.

Run: python3 infra/github-backup/tests/test_hosted_alerts.py
"""

import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "bin"))
import hosted_alerts as ha

REPO = "developing-today/code"


def report(run_id="run-1", mirror_status="fresh", d1_status="fresh"):
    return {
        "probe": "hosted-freshness:v1",
        "version": 1,
        "run_id": run_id,
        "generated_at": "2026-11-25T12:00:00Z",
        "inventory": [REPO],
        "legs": {
            "source": {
                "status": "fresh",
                "repos": {REPO: {"status": "fresh", "detail": "source head observed"}},
            },
            "plain_mirror": {
                "slo_hours": 26,
                "status": mirror_status,
                "repos": {
                    REPO: {
                        "status": mirror_status,
                        "detail": f"mirror is {mirror_status}",
                        "copy_head": "b" * 40,
                    }
                },
            },
            "d1_export": {
                "slo_hours": 26,
                "status": d1_status,
                "detail": f"d1 is {d1_status}",
                "repos": {},
                "exported_at": "2026-11-24T10:00:00Z",
            },
        },
        "verdict": "fresh" if mirror_status == d1_status == "fresh" else "attention",
    }


def existing_from(action, number=77):
    return [
        {
            "number": number,
            "title": action["title"],
            "body": action["body"],
            "state": "open",
        }
    ]


class CreateTests(unittest.TestCase):
    def test_stale_mirror_creates_one_alert_with_evidence(self):
        actions = ha.plan_actions(report(mirror_status="stale"), [])
        self.assertEqual(len(actions), 1)
        action = actions[0]
        self.assertEqual(action["action"], "create")
        self.assertEqual((action["leg"], action["repo"]), ("plain_mirror", REPO))
        self.assertIn("stale", action["title"])
        self.assertIn("run_id: run-1", action["body"])
        self.assertIn(ha.marker("plain_mirror", REPO), action["body"])

    def test_all_fresh_creates_nothing(self):
        self.assertEqual(ha.plan_actions(report(), []), [])

    def test_statuses_stay_distinct(self):
        for status in ("missing", "partial", "failed"):
            actions = ha.plan_actions(report(mirror_status=status), [])
            self.assertEqual(len(actions), 1, status)
            self.assertIn(status, actions[0]["title"])
            self.assertIn(f"status: {status}", actions[0]["body"])

    def test_d1_global_leg_alerts_under_dash_repo(self):
        actions = ha.plan_actions(report(d1_status="stale"), [])
        self.assertEqual(len(actions), 1)
        self.assertEqual(
            (actions[0]["leg"], actions[0]["repo"]), ("d1_export", ha.GLOBAL_REPO)
        )
        self.assertIn("exported_at: 2026-11-24T10:00:00Z", actions[0]["body"])


class DedupTests(unittest.TestCase):
    def test_second_run_updates_same_issue_never_creates(self):
        first = ha.plan_actions(report(run_id="run-1", mirror_status="stale"), [])
        existing = existing_from(first[0])
        second = ha.plan_actions(
            report(run_id="run-2", mirror_status="missing"), existing
        )
        self.assertEqual(len(second), 1)
        self.assertEqual(second[0]["action"], "update")
        self.assertEqual(second[0]["issue"], 77)
        self.assertIn("missing", second[0]["title"])

    def test_same_run_replan_is_noop(self):
        first = ha.plan_actions(report(run_id="run-1", mirror_status="stale"), [])
        existing = existing_from(first[0])
        again = ha.plan_actions(report(run_id="run-1", mirror_status="stale"), existing)
        self.assertEqual([a["action"] for a in again], ["noop"])

    def test_unmarked_issues_are_ignored(self):
        issues = [
            {"number": 5, "title": "unrelated", "body": "no marker", "state": "open"}
        ]
        actions = ha.plan_actions(report(mirror_status="failed"), issues)
        self.assertEqual([a["action"] for a in actions], ["create"])

    def test_closed_alert_does_not_block_a_new_create(self):
        first = ha.plan_actions(report(mirror_status="stale"), [])
        closed = existing_from(first[0])
        closed[0]["state"] = "closed"
        actions = ha.plan_actions(report(run_id="run-2", mirror_status="stale"), closed)
        self.assertEqual([a["action"] for a in actions], ["create"])


class ResolveTests(unittest.TestCase):
    def test_fresh_reverification_resolves_open_alert(self):
        first = ha.plan_actions(report(run_id="run-1", mirror_status="stale"), [])
        existing = existing_from(first[0])
        actions = ha.plan_actions(report(run_id="run-2"), existing)
        self.assertEqual(len(actions), 1)
        self.assertEqual(actions[0]["action"], "resolve")
        self.assertEqual(actions[0]["issue"], 77)
        self.assertIn("run-2", actions[0]["comment"])


class ApplyTests(unittest.TestCase):
    def test_apply_dispatches_each_action_to_executor(self):
        seen = []
        actions = ha.plan_actions(report(mirror_status="stale", d1_status="failed"), [])
        results = ha.apply_actions(actions, lambda a: seen.append(a) or 100 + len(seen))
        self.assertEqual(len(seen), 2)
        self.assertEqual([r["issue"] for r in results], [101, 102])


if __name__ == "__main__":
    unittest.main()
