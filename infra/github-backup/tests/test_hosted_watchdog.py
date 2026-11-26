#!/usr/bin/env python3
"""Hermetic tests for hosted_watchdog.py (#15001 missed-run slice).

Oracle: the watchdog judges only the probe's R2 heartbeat record —
never the probe job's status. A heartbeat within the 26h cadence SLO
is ok; an older heartbeat, or no heartbeat at all, is the distinct
probe-missed signal; a heartbeat read that cannot run (or a record
that is unusable or future-dated) is failed and is never read as a
miss. Exactly one open probe-missed alert may exist: a first miss
creates it under the (probe, -) marker key, a continuing miss with
unchanged evidence is a noop, changed last-run evidence updates it,
and a fresh heartbeat resolves it. No network, no rclone, no gh: the
heartbeat fetcher and the alert executor are the seams.

Run: python3 infra/github-backup/tests/test_hosted_watchdog.py
"""

import os
import sys
import unittest
from datetime import datetime, timezone

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "bin"))
import hosted_alerts as ha
import hosted_watchdog as hw

NOW = datetime(2026, 11, 26, 12, 0, 0, tzinfo=timezone.utc)


def heartbeat(run_id="run-9", generated_at="2026-11-26T05:17:00Z"):
    return {
        "probe": "hosted-freshness:v1",
        "version": 1,
        "run_id": run_id,
        "generated_at": generated_at,
        "verdict": "attention",
        "legs": {},
    }


def existing_from(action, number=55):
    return [{
        "number": number,
        "title": action["title"],
        "body": action["body"],
        "state": "open",
    }]


class JudgeTests(unittest.TestCase):
    def test_fresh_heartbeat_is_ok(self):
        verdict = hw.judge_heartbeat(heartbeat(), NOW)
        self.assertEqual(verdict["status"], hw.OK)
        self.assertEqual(verdict["last_run_id"], "run-9")
        self.assertLess(verdict["age_hours"], hw.PROBE_SLO_HOURS)

    def test_stale_heartbeat_is_missed(self):
        verdict = hw.judge_heartbeat(
            heartbeat(generated_at="2026-11-24T05:17:00Z"), NOW)
        self.assertEqual(verdict["status"], hw.MISSED)
        self.assertGreater(verdict["age_hours"], hw.PROBE_SLO_HOURS)
        self.assertIn("cadence SLO", verdict["detail"])

    def test_missing_heartbeat_is_missed_not_failed(self):
        verdict = hw.judge_heartbeat(None, NOW)
        self.assertEqual(verdict["status"], hw.MISSED)
        self.assertIsNone(verdict["last_run_id"])
        self.assertIn("no probe heartbeat", verdict["detail"])

    def test_future_heartbeat_is_failed_not_missed(self):
        verdict = hw.judge_heartbeat(
            heartbeat(generated_at="2026-11-27T05:17:00Z"), NOW)
        self.assertEqual(verdict["status"], hw.FAILED)

    def test_foreign_record_is_failed(self):
        record = heartbeat()
        record["probe"] = "something-else:v9"
        self.assertEqual(hw.judge_heartbeat(record, NOW)["status"], hw.FAILED)

    def test_unusable_timestamp_is_failed(self):
        verdict = hw.judge_heartbeat(heartbeat(generated_at="not-a-time"), NOW)
        self.assertEqual(verdict["status"], hw.FAILED)


class PlanTests(unittest.TestCase):
    def missed(self, run_id="run-9"):
        return hw.judge_heartbeat(
            heartbeat(run_id=run_id, generated_at="2026-11-24T05:17:00Z"), NOW)

    def test_first_miss_creates_one_marked_alert(self):
        actions = hw.plan_watchdog_actions(self.missed(), [], "wd-1", hw.hf.iso(NOW))
        self.assertEqual(len(actions), 1)
        action = actions[0]
        self.assertEqual(action["action"], "create")
        self.assertEqual((action["leg"], action["repo"]), ("probe", ha.GLOBAL_REPO))
        self.assertIn("probe is missed", action["title"])
        self.assertIn(ha.marker("probe", ha.GLOBAL_REPO), action["body"])
        self.assertIn("run_id: run-9", action["body"])

    def test_continuing_miss_same_evidence_is_noop(self):
        first = hw.plan_watchdog_actions(self.missed(), [], "wd-1", hw.hf.iso(NOW))
        existing = existing_from(first[0])
        again = hw.plan_watchdog_actions(self.missed(), existing, "wd-2", hw.hf.iso(NOW))
        self.assertEqual([a["action"] for a in again], ["noop"])

    def test_never_ran_miss_replans_as_noop(self):
        judgement = hw.judge_heartbeat(None, NOW)
        first = hw.plan_watchdog_actions(judgement, [], "wd-1", hw.hf.iso(NOW))
        self.assertEqual([a["action"] for a in first], ["create"])
        existing = existing_from(first[0])
        again = hw.plan_watchdog_actions(judgement, existing, "wd-2", hw.hf.iso(NOW))
        self.assertEqual([a["action"] for a in again], ["noop"])

    def test_miss_with_new_last_run_updates_same_alert(self):
        first = hw.plan_watchdog_actions(
            self.missed(run_id="run-9"), [], "wd-1", hw.hf.iso(NOW))
        existing = existing_from(first[0])
        actions = hw.plan_watchdog_actions(
            self.missed(run_id="run-10"), existing, "wd-2", hw.hf.iso(NOW))
        self.assertEqual([a["action"] for a in actions], ["update"])
        self.assertEqual(actions[0]["issue"], 55)

    def test_fresh_heartbeat_resolves_open_alert(self):
        first = hw.plan_watchdog_actions(self.missed(), [], "wd-1", hw.hf.iso(NOW))
        existing = existing_from(first[0])
        ok = hw.judge_heartbeat(heartbeat(run_id="run-11"), NOW)
        actions = hw.plan_watchdog_actions(ok, existing, "wd-2", hw.hf.iso(NOW))
        self.assertEqual([a["action"] for a in actions], ["resolve"])
        self.assertEqual(actions[0]["issue"], 55)
        self.assertIn("run-11", actions[0]["comment"])

    def test_ok_without_alert_plans_nothing(self):
        ok = hw.judge_heartbeat(heartbeat(), NOW)
        self.assertEqual(
            hw.plan_watchdog_actions(ok, [], "wd-1", hw.hf.iso(NOW)), [])

    def test_failed_judgement_plans_nothing(self):
        failed = hw.judge_heartbeat(
            heartbeat(generated_at="2026-11-27T05:17:00Z"), NOW)
        self.assertEqual(
            hw.plan_watchdog_actions(failed, [], "wd-1", hw.hf.iso(NOW)), [])


class RunTests(unittest.TestCase):
    def test_read_failure_is_failed_and_alerts_nothing(self):
        def boom():
            raise RuntimeError("rclone cat failed: connection reset")
        readout, exit_code = hw.run(NOW, "wd-1", boom, [], False,
                                    "developing-today/code")
        self.assertEqual(readout["status"], hw.FAILED)
        self.assertEqual(readout["actions"], [])
        self.assertEqual(exit_code, 1)
        self.assertIn("could not run", readout["detail"])

    def test_ok_run_exits_zero(self):
        readout, exit_code = hw.run(
            NOW, "wd-1", lambda: heartbeat(), [], False, "developing-today/code")
        self.assertEqual(readout["status"], hw.OK)
        self.assertEqual(exit_code, 0)

    def test_missed_run_exits_one_and_plans_create(self):
        readout, exit_code = hw.run(
            NOW, "wd-1", lambda: None, [], False, "developing-today/code")
        self.assertEqual(readout["status"], hw.MISSED)
        self.assertEqual([a["action"] for a in readout["actions"]], ["create"])
        self.assertEqual(exit_code, 1)


if __name__ == "__main__":
    unittest.main()
