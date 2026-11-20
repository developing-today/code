#!/usr/bin/env python3
"""Tests for C3 health hook runner (pm-stack/health-hooks/run.py)."""
import json
import os
import subprocess
import sys
import tempfile
import time
from contextlib import redirect_stdout
from io import StringIO
from pathlib import Path
from unittest import mock

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


def test_host_health_collector_schema():
    with tempfile.TemporaryDirectory() as td:
        path = Path(td) / "host-health.json"
        path.write_text(json.dumps({
            "generatedAt": "2026-11-20T01:00:00Z",
            "units": {
                "pm-aggregator.service": {"activeState": "failed"},
                "opencode.service": {"activeState": "active"},
            },
        }))
        result = R.read_host_health(path)
    assert result["collectedAt"] == "2026-11-20T01:00:00Z"
    assert result["unitsDown"] == ["pm-aggregator.service"]


def test_http_500_is_down_even_when_tcp_and_unit_are_healthy():
    failed = {"ok": False, "status": 500, "error": "HTTP 500", "latencyMs": 1}
    healthy_tcp = {"ok": True, "latencyMs": 1, "error": None}
    with mock.patch.object(R, "http_probe", return_value=failed), \
         mock.patch.object(R, "tcp_probe", return_value=healthy_tcp), \
         mock.patch.object(R, "systemctl_state", return_value={"activeState": "active"}), \
         mock.patch.object(R, "bws_resolve", return_value={"status": "resolved"}), \
         mock.patch.object(R, "git_recent_commits", return_value={"ok": True, "commits": []}), \
         mock.patch.object(R, "read_host_health", return_value={"collectedAt": None, "unitsDown": []}):
        pm_state, _, _ = R.probe_pm_stack("")
    with mock.patch.object(R, "http_probe", return_value=failed), \
         mock.patch.object(R, "tcp_probe", return_value=healthy_tcp), \
         mock.patch.object(R, "systemctl_state", return_value={"activeState": "active"}), \
         mock.patch.object(R, "run_cmd", return_value={"rc": 0, "stdout": "opencode", "stderr": ""}), \
         mock.patch.object(R, "opencode_route_ref", return_value={"status": "resolved"}), \
         mock.patch.object(R, "git_recent_commits", return_value={"ok": True, "commits": []}), \
         mock.patch.object(R, "count_sessions_last_hour", return_value=1):
        oc_state, _, _ = R.probe_opencode("")
    assert pm_state == "down"
    assert oc_state == "down"


def test_recovery_links_to_open_down_incident():
    with tempfile.TemporaryDirectory() as td:
        incident_file = Path(td) / "incidents.jsonl"
        incident_file.write_text(json.dumps({
            "target": "opencode-working", "state": "down", "incidentId": "down-123",
        }) + "\n")
        env = mock.patch.dict(os.environ, {"PM_HEALTH_HOOKS_INCIDENT_FILE": str(incident_file)})
        with env, mock.patch.object(R, "probe_opencode", return_value=("transient", "probe recovered", {})), \
             mock.patch.object(R, "post_to_throat", return_value={"posted": True}):
            stdout = StringIO()
            with redirect_stdout(stdout):
                assert R.main(["--target", "opencode-working"]) == 0
        out = json.loads(stdout.getvalue())
        records = [json.loads(line) for line in incident_file.read_text().splitlines()]
    assert out["state"] == "up"
    assert len(records) == 2
    assert records[-1]["state"] == "up"
    assert records[-1]["recoveryOf"] == "down-123"


def test_pm_stack_down_uses_supported_critical_severity():
    with tempfile.TemporaryDirectory() as td:
        incident_file = Path(td) / "incidents.jsonl"
        with mock.patch.dict(os.environ, {"PM_HEALTH_HOOKS_INCIDENT_FILE": str(incident_file)}), \
             mock.patch.object(R, "probe_pm_stack", return_value=("down", "unreachable", {})), \
             mock.patch.object(R, "post_to_throat", return_value={"posted": False}):
            stdout = StringIO()
            with redirect_stdout(stdout):
                assert R.main(["--target", "pm-stack-reachable"]) == 0
        output = json.loads(stdout.getvalue())
        record = json.loads(incident_file.read_text().splitlines()[0])
    assert output["severity"] == record["severity"] == "critical"


def test_run_cmd_caps_timeout_to_shared_deadline_and_skips_expired_work():
    with mock.patch.object(R.subprocess, "run", return_value=mock.Mock(returncode=0, stdout="", stderr="")) as run:
        R.run_cmd(["example"], timeout=30, deadline=time.monotonic() + 2)
        assert 0 < run.call_args.kwargs["timeout"] <= 2
        run.reset_mock()
        result = R.run_cmd(["example"], timeout=30, deadline=time.monotonic() - 1)
    run.assert_not_called()
    assert result["stderr"] == "sequence deadline exceeded"


def test_shell_wrappers_are_executable():
    for wrapper in (HERE / "pm-stack-reachable.sh", HERE / "opencode-working.sh"):
        assert wrapper.stat().st_mode & 0o111, wrapper


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
