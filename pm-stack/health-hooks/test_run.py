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
    r = subprocess.run(
        [sys.executable, str(RUNNER), "--target", "nope"],
        capture_output=True,
        text=True,
    )
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
    r = subprocess.run(
        [
            sys.executable,
            str(RUNNER),
            "--target",
            "opencode-working",
            "--caller",
            "test",
            "--json",
        ],
        capture_output=True,
        text=True,
        env=env,
    )
    assert r.returncode == 0, r.stderr
    obj = json.loads(r.stdout)
    assert obj["target"] == "opencode-working"
    assert obj["state"] in ("down", "transient", "up")
    assert "incidentId" in obj
    assert "categoryNote" in obj


def test_runner_pm_stack():
    env = os.environ.copy()
    env["PM_HEALTH_HOOKS_INCIDENT_FILE"] = str(Path(tempfile.mkdtemp()) / "inc.jsonl")
    r = subprocess.run(
        [
            sys.executable,
            str(RUNNER),
            "--target",
            "pm-stack-reachable",
            "--caller",
            "test",
        ],
        capture_output=True,
        text=True,
        env=env,
    )
    assert r.returncode == 0, r.stderr
    obj = json.loads(r.stdout)
    assert obj["target"] == "pm-stack-reachable"
    assert (
        "evidence" not in obj
    )  # evidence stays in record, not stdout? runner prints summary only
    # actually runner prints out without evidence; ok


def test_host_health_collector_schema():
    with tempfile.TemporaryDirectory() as td:
        path = Path(td) / "host-health.json"
        path.write_text(
            json.dumps(
                {
                    "generatedAt": "2026-11-20T01:00:00Z",
                    "units": {
                        "pm-aggregator.service": {"activeState": "failed"},
                        "opencode.service": {"activeState": "active"},
                    },
                }
            )
        )
        result = R.read_host_health(path)
    assert result["collectedAt"] == "2026-11-20T01:00:00Z"
    assert result["unitsDown"] == ["pm-aggregator.service"]


def test_http_500_is_down_even_when_tcp_and_unit_are_healthy():
    failed = {"ok": False, "status": 500, "error": "HTTP 500", "latencyMs": 1}
    healthy_tcp = {"ok": True, "latencyMs": 1, "error": None}
    with (
        mock.patch.object(R, "http_probe", return_value=failed),
        mock.patch.object(R, "tcp_probe", return_value=healthy_tcp),
        mock.patch.object(R, "systemctl_state", return_value={"activeState": "active"}),
        mock.patch.object(R, "bws_resolve", return_value={"status": "resolved"}),
        mock.patch.object(
            R, "git_recent_commits", return_value={"ok": True, "commits": []}
        ),
        mock.patch.object(
            R, "read_host_health", return_value={"collectedAt": None, "unitsDown": []}
        ),
    ):
        pm_state, _, _ = R.probe_pm_stack("")
    with (
        mock.patch.object(R, "http_probe", return_value=failed),
        mock.patch.object(R, "tcp_probe", return_value=healthy_tcp),
        mock.patch.object(R, "systemctl_state", return_value={"activeState": "active"}),
        mock.patch.object(
            R, "run_cmd", return_value={"rc": 0, "stdout": "opencode", "stderr": ""}
        ),
        mock.patch.object(R, "opencode_route_ref", return_value={"status": "resolved"}),
        mock.patch.object(
            R, "git_recent_commits", return_value={"ok": True, "commits": []}
        ),
        mock.patch.object(R, "count_sessions_last_hour", return_value=1),
    ):
        oc_state, _, _ = R.probe_opencode("")
    assert pm_state == "down"
    assert oc_state == "down"


def test_recovery_links_to_open_down_incident():
    with tempfile.TemporaryDirectory() as td:
        incident_file = Path(td) / "incidents.jsonl"
        incident_file.write_text(
            json.dumps(
                {
                    "target": "opencode-working",
                    "state": "down",
                    "incidentId": "down-123",
                }
            )
            + "\n"
        )
        env = mock.patch.dict(
            os.environ, {"PM_HEALTH_HOOKS_INCIDENT_FILE": str(incident_file)}
        )
        with (
            env,
            mock.patch.object(
                R, "probe_opencode", return_value=("transient", "probe recovered", {})
            ),
            mock.patch.object(R, "post_to_throat", return_value={"posted": True}),
        ):
            stdout = StringIO()
            with redirect_stdout(stdout):
                assert R.main(["--target", "opencode-working"]) == 0
        out = json.loads(stdout.getvalue())
        records = [json.loads(line) for line in incident_file.read_text().splitlines()]
    assert out["state"] == "up"
    assert len(records) == 2
    assert records[-1]["state"] == "up"
    assert records[-1]["recoveryOf"] == "down-123"


def test_undelivered_recovery_is_posted_again_on_the_next_run():
    with tempfile.TemporaryDirectory() as td:
        incident_file = Path(td) / "incidents.jsonl"
        incident_file.write_text(
            json.dumps(
                {
                    "target": "opencode-working",
                    "state": "down",
                    "incidentId": "down-123",
                    "key": "health-hook:opencode-working:down-123",
                }
            )
            + "\n"
        )
        env = mock.patch.dict(
            os.environ, {"PM_HEALTH_HOOKS_INCIDENT_FILE": str(incident_file)}
        )
        probe = mock.patch.object(
            R, "probe_opencode", return_value=("transient", "probe recovered", {})
        )
        with (
            env,
            probe,
            mock.patch.object(
                R,
                "post_to_throat",
                return_value={"posted": False, "reason": "HTTP 503"},
            ),
        ):
            with redirect_stdout(StringIO()):
                assert R.main(["--target", "opencode-working"]) == 0
            assert R.find_open_incident("opencode-working")["incidentId"] == "down-123"
        with (
            env,
            probe,
            mock.patch.object(
                R, "post_to_throat", return_value={"posted": True}
            ) as post,
        ):
            stdout = StringIO()
            with redirect_stdout(stdout):
                assert R.main(["--target", "opencode-working"]) == 0
            assert R.find_open_incident("opencode-working") is None
        out = json.loads(stdout.getvalue())
        records = [json.loads(line) for line in incident_file.read_text().splitlines()]
    assert out["state"] == "up"
    assert post.call_args.args[0]["key"] == "health-hook:opencode-working:down-123"
    assert [r.get("recoveryDelivered") for r in records] == [None, False, True]


def test_pm_stack_down_uses_supported_critical_severity():
    with tempfile.TemporaryDirectory() as td:
        incident_file = Path(td) / "incidents.jsonl"
        with (
            mock.patch.dict(
                os.environ, {"PM_HEALTH_HOOKS_INCIDENT_FILE": str(incident_file)}
            ),
            mock.patch.object(
                R, "probe_pm_stack", return_value=("down", "unreachable", {})
            ),
            mock.patch.object(R, "post_to_throat", return_value={"posted": False}),
        ):
            stdout = StringIO()
            with redirect_stdout(stdout):
                assert R.main(["--target", "pm-stack-reachable"]) == 0
        output = json.loads(stdout.getvalue())
        record = json.loads(incident_file.read_text().splitlines()[0])
    assert output["severity"] == record["severity"] == "critical"


def test_run_cmd_caps_timeout_to_shared_deadline_and_skips_expired_work():
    with mock.patch.object(
        R.subprocess, "run", return_value=mock.Mock(returncode=0, stdout="", stderr="")
    ) as run:
        R.run_cmd(["example"], timeout=30, deadline=time.monotonic() + 2)
        assert 0 < run.call_args.kwargs["timeout"] <= 2
        run.reset_mock()
        result = R.run_cmd(["example"], timeout=30, deadline=time.monotonic() - 1)
    run.assert_not_called()
    assert result["stderr"] == "sequence deadline exceeded"


def test_recovery_reuses_the_outage_intake_key():
    with tempfile.TemporaryDirectory() as td:
        incident_file = Path(td) / "incidents.jsonl"
        incident_file.write_text(
            json.dumps(
                {
                    "target": "opencode-working",
                    "state": "down",
                    "incidentId": "down-123",
                    "key": "health-hook:opencode-working:down-123",
                }
            )
            + "\n"
        )
        with (
            mock.patch.dict(
                os.environ, {"PM_HEALTH_HOOKS_INCIDENT_FILE": str(incident_file)}
            ),
            mock.patch.object(
                R, "probe_opencode", return_value=("transient", "ok", {})
            ),
            mock.patch.object(
                R, "post_to_throat", return_value={"posted": True}
            ) as post,
        ):
            with redirect_stdout(StringIO()):
                assert R.main(["--target", "opencode-working"]) == 0
    assert post.call_args.args[0]["key"] == "health-hook:opencode-working:down-123"


def test_repeated_down_reuses_the_open_outage():
    with tempfile.TemporaryDirectory() as td:
        incident_file = Path(td) / "incidents.jsonl"
        env = mock.patch.dict(
            os.environ, {"PM_HEALTH_HOOKS_INCIDENT_FILE": str(incident_file)}
        )
        with (
            env,
            mock.patch.object(
                R, "probe_opencode", return_value=("down", "unreachable", {})
            ),
            mock.patch.object(
                R, "post_to_throat", return_value={"posted": True}
            ) as post,
        ):
            with redirect_stdout(StringIO()):
                assert R.main(["--target", "opencode-working"]) == 0
                assert R.main(["--target", "opencode-working"]) == 0
        keys = [c.args[0]["key"] for c in post.call_args_list]
        with (
            env,
            mock.patch.object(
                R, "probe_opencode", return_value=("transient", "ok", {})
            ),
            mock.patch.object(
                R, "post_to_throat", return_value={"posted": True}
            ) as rec,
        ):
            with redirect_stdout(StringIO()):
                assert R.main(["--target", "opencode-working"]) == 0
            assert R.find_open_incident("opencode-working") is None
    assert keys[0] == keys[1]
    assert rec.call_args.args[0]["key"] == keys[0]


def test_wrapper_target_cannot_be_overridden():
    r = subprocess.run(
        [str(HERE / "pm-stack-reachable.sh"), "--target", "opencode-working"],
        capture_output=True,
        text=True,
    )
    assert r.returncode == 2, r
    assert "only once" in r.stderr


def test_bws_lookup_resolves_the_key_to_its_id_before_get():
    calls = []

    def fake(cmd, timeout=5, deadline=None, env=None):
        calls.append(cmd[1:])
        assert env and env.get("BWS_ACCESS_TOKEN") == "access"
        if cmd[1:3] == ["secret", "list"]:
            return {
                "rc": 0,
                "stdout": json.dumps(
                    [{"id": "id-1", "key": "other"}, {"id": "id-2", "key": "wanted"}]
                ),
                "stderr": "",
            }
        return {
            "rc": 0,
            "stdout": json.dumps({"id": "id-2", "value": " tok "}),
            "stderr": "",
        }

    with (
        mock.patch.dict(os.environ, {"BWS_ACCESS_TOKEN": "access", "BWS_BIN": "bws"}),
        mock.patch.object(R, "run_cmd", side_effect=fake),
    ):
        assert R.bws_lookup("wanted") == ("resolved", "tok")
        assert R.bws_lookup("absent")[0] == "missing"
        evidence = R.bws_resolve("wanted")
    assert calls[1] == ["secret", "get", "id-2", "--output", "json"]
    assert evidence["status"] == "resolved" and "tok" not in json.dumps(evidence)


def test_inactive_successful_oneshot_is_not_down():
    assert not R.unit_is_down({"activeState": "inactive", "result": "success"})
    assert not R.unit_is_down({"activeState": "active"})
    assert R.unit_is_down({"activeState": "failed"})
    assert R.unit_is_down({"activeState": "inactive", "result": "exit-code"})


def test_sessions_last_hour_reads_the_live_store():
    import sqlite3

    with tempfile.TemporaryDirectory() as td:
        db_path = Path(td) / "opencode.db"
        db = sqlite3.connect(db_path)
        db.execute("CREATE TABLE session_v2 (id text, time_updated integer)")
        now = 10_000_000_000
        db.executemany(
            "INSERT INTO session_v2 VALUES (?, ?)",
            [("a", now - 60_000), ("b", now - 3_000_000), ("c", now - 7_200_000)],
        )
        db.commit()
        db.close()
        assert R.count_sessions_last_hour(db_path, now_ms=now) == 2
        assert R.count_sessions_last_hour(Path(td) / "missing.db", now_ms=now) is None


def _capture_throat_body(record):
    """Run the real post_to_throat against a fake intake; return the
    decoded JSON body it sent. Oracle: the #12599 record shape."""
    captured = {}

    class Resp:
        def __enter__(self):
            return self

        def __exit__(self, *exc):
            return False

        def read(self):
            return b""

        def getcode(self):
            return 200

    def fake_urlopen(req, timeout=None):
        captured["body"] = json.loads(req.data.decode())
        return Resp()

    with (
        mock.patch.object(R, "bws_lookup", return_value=("resolved", "tok")),
        mock.patch.object(R, "urlopen", side_effect=fake_urlopen),
    ):
        result = R.post_to_throat(record)
    assert result["posted"] is True
    return captured["body"]


def test_record_shape_one_shape_for_jsonl_and_post():
    # #12603 item 1: the record is built once, in the full design shape
    # including stackHealth; the JSONL record and the record handed to
    # post_to_throat are the same shape.
    with tempfile.TemporaryDirectory() as td:
        incident_file = Path(td) / "incidents.jsonl"
        with (
            mock.patch.dict(
                os.environ, {"PM_HEALTH_HOOKS_INCIDENT_FILE": str(incident_file)}
            ),
            mock.patch.object(
                R, "probe_opencode", return_value=("down", "unreachable", {"tcp": {}})
            ),
            mock.patch.object(
                R, "post_to_throat", return_value={"posted": True}
            ) as post,
        ):
            with redirect_stdout(StringIO()):
                assert R.main(["--target", "opencode-working"]) == 0
        record = post.call_args.args[0]
        stored = json.loads(incident_file.read_text().splitlines()[0])
    assert stored == record
    assert record["stackHealth"] == {
        "serviceId": "opencode-working",
        "name": "OpenCode working",
        "state": "down",
        "since": record["firedAt"],
        "detail": record["body"],
    }
    assert record["title"] == "Health hook: OpenCode working is down"


def test_post_to_throat_posts_the_full_record_as_healthhook_extension():
    # #12603 item 2: the POST body carries the full record as the
    # healthHook extension alongside the pre-existing members, and its
    # stackHealth member is the record's own (reuse, not rebuild — a
    # sentinel detail proves the value came from the record).
    record = {
        "source": "health-hook",
        "key": "health-hook:opencode-working:inc-1",
        "severity": "warning",
        "title": "Health hook: OpenCode working is down",
        "body": "unreachable",
        "target": "opencode-working",
        "state": "down",
        "caller": "test",
        "firedAt": "2026-11-20T03:04:05Z",
        "observedAt": None,
        "incidentId": "inc-1",
        "recoveryOf": None,
        "evidence": {"tcp": {"ok": False}},
        "stackHealth": {
            "serviceId": "opencode-working",
            "name": "OpenCode working",
            "state": "down",
            "since": "2026-11-20T03:04:05Z",
            "detail": "sentinel-detail",
        },
    }
    body = _capture_throat_body(record)
    assert body["healthHook"] == record
    assert body["stackHealth"] == record["stackHealth"]
    assert body["stackHealth"]["detail"] == "sentinel-detail"
    assert body["source"] == "health-hook"
    assert body["key"] == record["key"]
    assert body["severity"] == record["severity"]
    assert body["title"] == record["title"]
    assert body["body"] == record["body"]


def test_recovery_title_reads_recovered_and_record_links_lineage():
    # #12603 item 3: a recovery's title is "Health hook: <name>
    # recovered"; its record carries recoveryOf lineage and an "up"
    # stackHealth member, and the posted extension is that same record.
    with tempfile.TemporaryDirectory() as td:
        incident_file = Path(td) / "incidents.jsonl"
        incident_file.write_text(
            json.dumps(
                {
                    "target": "opencode-working",
                    "state": "down",
                    "incidentId": "down-123",
                    "key": "health-hook:opencode-working:down-123",
                }
            )
            + "\n"
        )
        with (
            mock.patch.dict(
                os.environ, {"PM_HEALTH_HOOKS_INCIDENT_FILE": str(incident_file)}
            ),
            mock.patch.object(
                R, "probe_opencode", return_value=("transient", "probe recovered", {})
            ),
            mock.patch.object(
                R, "post_to_throat", return_value={"posted": True}
            ) as post,
        ):
            with redirect_stdout(StringIO()):
                assert R.main(["--target", "opencode-working"]) == 0
        record = post.call_args.args[0]
    assert record["title"] == "Health hook: OpenCode working recovered"
    assert record["state"] == "up"
    assert record["recoveryOf"] == "down-123"
    assert record["stackHealth"]["state"] == "up"
    body = _capture_throat_body(record)
    assert body["title"] == "Health hook: OpenCode working recovered"
    assert body["healthHook"]["recoveryOf"] == "down-123"
    assert body["healthHook"]["incidentId"] == record["incidentId"]


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
