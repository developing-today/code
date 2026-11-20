#!/usr/bin/env python3
"""Tests for C3 health hook runner (pm-stack/health-hooks/run.py)."""
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
RUNNER = HERE / "run.py"
sys.path.insert(0, str(HERE))
import run as R  # noqa: E402


def test_argparse_requires_target():
    r = subprocess.run([sys.executable, str(RUNNER)], capture_output=True, text=True)
    assert r.returncode == 2, r


def test_argparse_rejects_bad_target():
    r = subprocess.run([sys.executable, str(RUNNER), "--target", "nope"], capture_output=True, text=True)
    assert r.returncode == 2, r


def test_targets_defined():
    assert set(R.TARGETS) == {"pm-stack-reachable", "opencode-working"}


def test_tcp_probe_shape():
    res = R.tcp_probe("127.0.0.1", 1, timeout=1)
    assert "ok" in res and "latencyMs" in res and "error" in res
    assert res["ok"] is False


def test_http_probe_shape():
    res = R.http_probe("http://127.0.0.1:1/nope", timeout=1, alive=[200])
    assert res["ok"] is False
    assert res["status"] is None


def test_systemctl_state_shape():
    st = R.systemctl_state("definitely-not-a-unit-xyz")
    assert "activeState" in st
    assert "unit" in st


def test_git_recent_commits():
    r = R.git_recent_commits(str(Path(__file__).resolve().parents[2]), 3)
    assert "commits" in r
    # may be ok or not depending on git; shape check
    assert isinstance(r["commits"], list)


def test_runner_runs_and_prints_json():
    env = os.environ.copy()
    env["PM_HEALTH_HOOKS_INCIDENT_FILE"] = str(Path(tempfile.mkdtemp()) / "inc.jsonl")
    r = subprocess.run([sys.executable, str(RUNNER), "--target", "opencode-working",
                        "--caller", "test", "--json"], capture_output=True, text=True, env=env)
    assert r.returncode == 0, r.stderr
    obj = json.loads(r.stdout)
    assert obj["target"] == "opencode-working"
    assert obj["state"] in ("down", "transient", "up")
    assert "incidentId" in obj
    assert "categoryNote" in obj


def test_runner_pm_stack():
    env = os.environ.copy()
    env["PM_HEALTH_HOOKS_INCIDENT_FILE"] = str(Path(tempfile.mkdtemp()) / "inc.jsonl")
    r = subprocess.run([sys.executable, str(RUNNER), "--target", "pm-stack-reachable",
                        "--caller", "test"], capture_output=True, text=True, env=env)
    assert r.returncode == 0, r.stderr
    obj = json.loads(r.stdout)
    assert obj["target"] == "pm-stack-reachable"
    assert "evidence" not in obj  # evidence stays in record, not stdout? runner prints summary only
    # actually runner prints out without evidence; ok


if __name__ == "__main__":
    import traceback
    tests = [v for k, v in globals().items() if k.startswith("test_")]
    failed = 0
    for t in tests:
        try:
            t()
            print("PASS", t.__name__)
        except Exception as e:
            failed += 1
            print("FAIL", t.__name__, "-", repr(e))
            traceback.print_exc()
    sys.exit(1 if failed else 0)