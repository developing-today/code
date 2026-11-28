#!/usr/bin/env python3
"""Tests for the run-silence heartbeat (code#10917).

Everything runs offline against fixtures: `--fixture` replaces the
GitHub reads, so no live GitHub access and no issue writes are needed.
The acceptance demonstrations from code#10917 are here by name:

- a stale newest-run timestamp fires exactly one tracking issue;
- a fresh timestamp stays quiet;
- a repo that already has a tracking issue is updated, not duplicated.
"""

import contextlib
import io
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
SCRIPT = HERE / "check_silence.py"
sys.path.insert(0, str(HERE))

import check_silence  # noqa: E402

NOW = "2026-11-17T12:00:00Z"
REPO = "developing-today-forks/openchamber"


def run_fixture(fixture: dict, repos: list[str] | None = None) -> dict:
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as fh:
        json.dump(fixture, fh)
        path = fh.name
    cmd = [sys.executable, str(SCRIPT), "--fixture", path, "--now", NOW]
    for repo in repos if repos is not None else [REPO]:
        cmd += ["--repo", repo]
    out = subprocess.run(cmd, capture_output=True, text=True, check=True).stdout
    return json.loads(out)


def one_run(updated_at: str, run_id: int = 100) -> list[dict]:
    return [
        {
            "databaseId": run_id,
            "name": "CI",
            "conclusion": "success",
            "createdAt": updated_at,
            "updatedAt": updated_at,
            "url": f"https://github.com/{REPO}/actions/runs/{run_id}",
        }
    ]


class SilenceEvaluationTests(unittest.TestCase):
    def test_stale_timestamp_fires_exactly_one_tracking_issue(self):
        result = run_fixture(
            {
                "runs": {REPO: one_run("2026-11-16T12:00:00Z")},  # 24h old
                "tracking": {REPO: None},
            }
        )
        actions = [a for a in result["actions"] if a["action"] != "none"]
        self.assertEqual(len(actions), 1)
        self.assertEqual(actions[0]["action"], "file")
        self.assertEqual(actions[0]["repo"], REPO)
        ev = result["evaluations"][0]
        self.assertEqual(ev["status"], "silent")
        self.assertEqual(ev["age_hours"], 24.0)

    def test_fresh_timestamp_stays_quiet(self):
        result = run_fixture(
            {
                "runs": {REPO: one_run("2026-11-17T11:30:00Z")},  # 30m old
                "tracking": {REPO: None},
            }
        )
        self.assertEqual(
            result["actions"], [{"action": "none", "repo": REPO, "issue": None}]
        )
        self.assertEqual(result["evaluations"][0]["status"], "ok")

    def test_existing_tracking_issue_is_updated_not_duplicated(self):
        result = run_fixture(
            {
                "runs": {REPO: one_run("2026-11-15T12:00:00Z")},  # 48h old
                "tracking": {REPO: 2274},
            }
        )
        self.assertEqual(
            result["actions"], [{"action": "update", "repo": REPO, "issue": 2274}]
        )

    def test_no_completed_runs_at_all_fires(self):
        result = run_fixture({"runs": {REPO: []}, "tracking": {REPO: None}})
        self.assertEqual(result["evaluations"][0]["status"], "silent")
        self.assertIsNone(result["evaluations"][0]["last_run"])
        self.assertEqual(result["actions"][0]["action"], "file")

    def test_threshold_boundary(self):
        just_under = check_silence.evaluate(
            REPO, one_run("2026-11-17T06:00:01Z"), check_silence.parse_ts(NOW), 6.0
        )
        just_over = check_silence.evaluate(
            REPO, one_run("2026-11-17T05:59:59Z"), check_silence.parse_ts(NOW), 6.0
        )
        self.assertEqual(just_under["status"], "ok")
        self.assertEqual(just_over["status"], "silent")

    def test_newest_run_wins_regardless_of_order(self):
        runs = one_run("2026-11-17T11:00:00Z", 101) + one_run(
            "2026-11-10T11:00:00Z", 99
        )
        ev = check_silence.evaluate(REPO, runs, check_silence.parse_ts(NOW), 6.0)
        self.assertEqual(ev["status"], "ok")
        self.assertEqual(ev["last_run"]["databaseId"], 101)

    def test_repos_are_independent(self):
        fresh = "developing-today/code"
        result = run_fixture(
            {
                "runs": {
                    REPO: one_run("2026-11-16T12:00:00Z"),
                    fresh: one_run("2026-11-17T11:45:00Z"),
                },
                "tracking": {REPO: None, fresh: None},
            },
            repos=[REPO, fresh],
        )
        by_repo = {a["repo"]: a["action"] for a in result["actions"]}
        self.assertEqual(by_repo[REPO], "file")
        self.assertEqual(by_repo[fresh], "none")


FLEET = check_silence.FLEET_REPOS
BROKEN = "developing-today-forks/openchamber"


def fake_gh(fail_repo=None, exc=None, fail_on=("run", "list"), write_fail_repo=None):
    """A stand-in for subprocess.run: every repo has a fresh-enough run
    except a 24h-stale one, and `fail_repo`'s `fail_on` call raises
    `exc`. Records each call's argv and timeout."""
    calls = []

    def run(cmd, **kwargs):
        calls.append((cmd, kwargs.get("timeout")))
        repo = cmd[cmd.index("--repo") + 1]
        if repo == fail_repo and tuple(cmd[1:3]) == fail_on:
            raise exc
        if cmd[1:3] == ["run", "list"]:
            stamp = (
                "2026-11-16T12:00:00Z" if repo == FLEET[0] else "2026-11-17T11:30:00Z"
            )
            out = json.dumps(one_run(stamp))
        elif cmd[1:3] == ["issue", "list"]:
            out = "[]"
        else:
            if repo == write_fail_repo:
                raise subprocess.CalledProcessError(1, cmd, stderr="HTTP 502")
            out = ""
        return subprocess.CompletedProcess(cmd, 0, stdout=out, stderr="")

    return run, calls


def run_live(run, argv=()):
    buf = io.StringIO()
    with (
        mock.patch.object(check_silence.subprocess, "run", run),
        contextlib.redirect_stdout(buf),
    ):
        code = check_silence.main(["--now", NOW, *argv])
    return code, json.loads(buf.getvalue())


class ProbeFailureTests(unittest.TestCase):
    """code#11468: a failing or hung gh call is one repo's error, not
    the end of the pass, and never reads as a clean pass."""

    def test_one_repo_failing_still_evaluates_the_others(self):
        err = subprocess.CalledProcessError(
            1, ["gh", "run", "list"], stderr="HTTP 502: Bad Gateway"
        )
        run, _ = fake_gh(fail_repo=BROKEN, exc=err)
        code, result = run_live(run)
        self.assertEqual(code, 1)
        by_repo = {ev["repo"]: ev for ev in result["evaluations"]}
        self.assertEqual(sorted(by_repo), sorted(FLEET))
        self.assertEqual(by_repo[BROKEN]["status"], "error")
        self.assertIn("HTTP 502", by_repo[BROKEN]["reason"])
        self.assertEqual(by_repo[FLEET[0]]["status"], "silent")
        actions = {a["repo"]: a["action"] for a in result["actions"]}
        self.assertEqual(actions[FLEET[0]], "file")
        self.assertEqual(actions[BROKEN], "none")

    def test_a_timeout_is_an_error_not_a_hang(self):
        err = subprocess.TimeoutExpired(
            ["gh", "run", "list"], check_silence.GH_TIMEOUT_S
        )
        run, calls = fake_gh(fail_repo=BROKEN, exc=err)
        code, result = run_live(run)
        self.assertEqual(code, 1)
        ev = next(e for e in result["evaluations"] if e["repo"] == BROKEN)
        self.assertEqual(ev["status"], "error")
        self.assertIn("timed out", ev["reason"])
        self.assertTrue(calls)
        self.assertTrue(
            all(timeout == check_silence.GH_TIMEOUT_S for _, timeout in calls),
            "every gh call carries the timeout",
        )

    def test_a_failed_tracking_lookup_is_an_error_too(self):
        err = subprocess.CalledProcessError(
            1, ["gh", "issue", "list"], stderr="rate limited"
        )
        run, _ = fake_gh(fail_repo=BROKEN, exc=err, fail_on=("issue", "list"))
        code, result = run_live(run)
        self.assertEqual(code, 1)
        ev = next(e for e in result["evaluations"] if e["repo"] == BROKEN)
        self.assertEqual(ev["status"], "error")

    def test_one_failed_write_does_not_block_the_rest(self):
        # Two silent repos; the first one's issue create fails.
        stale = FLEET[0]
        other = "dezren39/darwin-nix"

        def run(cmd, **kwargs):
            repo = cmd[cmd.index("--repo") + 1]
            if cmd[1:3] == ["run", "list"]:
                return subprocess.CompletedProcess(
                    cmd, 0, json.dumps(one_run("2026-11-16T12:00:00Z")), ""
                )
            if cmd[1:3] == ["issue", "list"]:
                return subprocess.CompletedProcess(cmd, 0, "[]", "")
            writes.append(repo)
            if repo == stale:
                raise subprocess.CalledProcessError(1, cmd, stderr="HTTP 502")
            return subprocess.CompletedProcess(cmd, 0, "", "")

        writes = []
        code, result = run_live(run, ["--apply", "--repo", stale, "--repo", other])
        self.assertEqual(code, 1)
        self.assertEqual(writes, [stale, other])
        self.assertEqual(
            [(e["repo"], e["action"]) for e in result["apply_errors"]],
            [(stale, "file")],
        )

    def test_a_clean_pass_still_exits_0_with_silence_found(self):
        run, _ = fake_gh()
        code, result = run_live(run)
        self.assertEqual(code, 0)
        self.assertEqual(result["apply_errors"], [])
        self.assertIn("silent", {ev["status"] for ev in result["evaluations"]})


if __name__ == "__main__":
    unittest.main()
