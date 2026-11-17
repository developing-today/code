#!/usr/bin/env python3
"""Tests for the run-silence heartbeat (code#10917).

Everything runs offline against fixtures: `--fixture` replaces the
GitHub reads, so no live GitHub access and no issue writes are needed.
The acceptance demonstrations from code#10917 are here by name:

- a stale newest-run timestamp fires exactly one tracking issue;
- a fresh timestamp stays quiet;
- a repo that already has a tracking issue is updated, not duplicated.
"""

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
SCRIPT = HERE / "check_silence.py"
sys.path.insert(0, str(HERE))

import check_silence  # noqa: E402

NOW = "2026-11-17T12:00:00Z"
REPO = "developing-today-forks/openchamber"


def run_fixture(fixture: dict, repos: list[str] | None = None) -> dict:
    with tempfile.NamedTemporaryFile(
        "w", suffix=".json", delete=False
    ) as fh:
        json.dump(fixture, fh)
        path = fh.name
    cmd = [sys.executable, str(SCRIPT), "--fixture", path, "--now", NOW]
    for repo in (repos if repos is not None else [REPO]):
        cmd += ["--repo", repo]
    out = subprocess.run(cmd, capture_output=True, text=True, check=True).stdout
    return json.loads(out)


def one_run(updated_at: str, run_id: int = 100) -> list[dict]:
    return [{
        "databaseId": run_id,
        "name": "CI",
        "conclusion": "success",
        "createdAt": updated_at,
        "updatedAt": updated_at,
        "url": f"https://github.com/{REPO}/actions/runs/{run_id}",
    }]


class SilenceEvaluationTests(unittest.TestCase):
    def test_stale_timestamp_fires_exactly_one_tracking_issue(self):
        result = run_fixture({
            "runs": {REPO: one_run("2026-11-16T12:00:00Z")},  # 24h old
            "tracking": {REPO: None},
        })
        actions = [a for a in result["actions"] if a["action"] != "none"]
        self.assertEqual(len(actions), 1)
        self.assertEqual(actions[0]["action"], "file")
        self.assertEqual(actions[0]["repo"], REPO)
        ev = result["evaluations"][0]
        self.assertEqual(ev["status"], "silent")
        self.assertEqual(ev["age_hours"], 24.0)

    def test_fresh_timestamp_stays_quiet(self):
        result = run_fixture({
            "runs": {REPO: one_run("2026-11-17T11:30:00Z")},  # 30m old
            "tracking": {REPO: None},
        })
        self.assertEqual(result["actions"],
                         [{"action": "none", "repo": REPO, "issue": None}])
        self.assertEqual(result["evaluations"][0]["status"], "ok")

    def test_existing_tracking_issue_is_updated_not_duplicated(self):
        result = run_fixture({
            "runs": {REPO: one_run("2026-11-15T12:00:00Z")},  # 48h old
            "tracking": {REPO: 2274},
        })
        self.assertEqual(result["actions"],
                         [{"action": "update", "repo": REPO, "issue": 2274}])

    def test_no_completed_runs_at_all_fires(self):
        result = run_fixture({"runs": {REPO: []}, "tracking": {REPO: None}})
        self.assertEqual(result["evaluations"][0]["status"], "silent")
        self.assertIsNone(result["evaluations"][0]["last_run"])
        self.assertEqual(result["actions"][0]["action"], "file")

    def test_threshold_boundary(self):
        just_under = check_silence.evaluate(
            REPO, one_run("2026-11-17T06:00:01Z"),
            check_silence.parse_ts(NOW), 6.0)
        just_over = check_silence.evaluate(
            REPO, one_run("2026-11-17T05:59:59Z"),
            check_silence.parse_ts(NOW), 6.0)
        self.assertEqual(just_under["status"], "ok")
        self.assertEqual(just_over["status"], "silent")

    def test_newest_run_wins_regardless_of_order(self):
        runs = one_run("2026-11-17T11:00:00Z", 101) + one_run("2026-11-10T11:00:00Z", 99)
        ev = check_silence.evaluate(REPO, runs, check_silence.parse_ts(NOW), 6.0)
        self.assertEqual(ev["status"], "ok")
        self.assertEqual(ev["last_run"]["databaseId"], 101)

    def test_repos_are_independent(self):
        fresh = "developing-today/code"
        result = run_fixture({
            "runs": {
                REPO: one_run("2026-11-16T12:00:00Z"),
                fresh: one_run("2026-11-17T11:45:00Z"),
            },
            "tracking": {REPO: None, fresh: None},
        }, repos=[REPO, fresh])
        by_repo = {a["repo"]: a["action"] for a in result["actions"]}
        self.assertEqual(by_repo[REPO], "file")
        self.assertEqual(by_repo[fresh], "none")


if __name__ == "__main__":
    unittest.main()
