#!/usr/bin/env python3
"""Client-protocol parity round-trip for the reconciled daemon (#10131).
Drives the union daemon on a scratch port/state dir with the exact wire
shapes the real client (~/tools/laptop-agent.py) uses."""

import json
import os
import subprocess
import sys
import threading
import time
import urllib.request
import urllib.error
from pathlib import Path

DAEMON = Path(__file__).resolve().parents[1] / "src" / "laptop_agent.py"

BASE = "http://127.0.0.1:4798"
TOKEN = "parity-token-123"
STATE = "/tmp/parity-state"
FAILS = []
# The laptop shell exports proxy vars; loopback must not go through them.
OPENER = urllib.request.build_opener(urllib.request.ProxyHandler({}))


def call(method, path, body=None, headers=None, raw=None, timeout=30):
    data = raw
    if body is not None:
        data = json.dumps(body).encode()
    req = urllib.request.Request(BASE + path, data=data, method=method)
    req.add_header("X-Agent-Token", TOKEN)
    if body is not None:
        req.add_header("Content-Type", "application/json")
    for k, v in (headers or {}).items():
        req.add_header(k, v)
    try:
        with OPENER.open(req, timeout=timeout) as r:
            return r.status, dict(r.headers), r.read()
    except urllib.error.HTTPError as e:
        return e.code, dict(e.headers), e.read()


def jcall(method, path, body=None, **kw):
    st, hd, raw = call(method, path, body=body, **kw)
    return st, json.loads(raw) if raw else {}


def check(name, cond, extra=""):
    print(("PASS " if cond else "FAIL ") + name + (f" [{extra}]" if extra else ""))
    if not cond:
        FAILS.append(name)


def start_daemon():
    env = dict(
        os.environ,
        LAPTOP_AGENT_STATE=STATE,
        LAPTOP_AGENT_TOKEN_FILE="/tmp/parity-token",
        LAPTOP_AGENT_PORT="4798",
        LAPTOP_AGENT_HOST="127.0.0.1",
    )
    logf = open("/tmp/parity-daemon.log", "ab")
    p = subprocess.Popen(
        [sys.executable, str(DAEMON)], env=env, stdout=logf, stderr=subprocess.STDOUT
    )
    for _ in range(75):
        if p.poll() is not None:
            raise RuntimeError(
                f"daemon exited early rc={p.returncode}; see /tmp/parity-daemon.log"
            )
        try:
            st, _h, _r = call("GET", "/health", timeout=2)
            if st == 200:
                return p
        except Exception:
            pass
        time.sleep(0.2)
    raise RuntimeError("daemon did not come up; see /tmp/parity-daemon.log")


def main():
    subprocess.run(["rm", "-rf", STATE], check=True)
    with open("/tmp/parity-token", "w") as f:
        f.write(TOKEN + "\n")
    with open("/tmp/parity-file.txt", "wb") as f:
        f.write(bytes(range(100)))
    d = start_daemon()
    try:
        st, b = jcall("GET", "/health")
        check("health", st == 200 and b.get("ok") is True)
        st, _ = jcall("GET", "/health", headers={})
        # (auth header always added by call(); do a raw unauth check)
        req = urllib.request.Request(BASE + "/health", method="GET")
        try:
            OPENER.open(req, timeout=5)
            check("unauth 401", False)
        except urllib.error.HTTPError as e:
            check("unauth 401", e.code == 401)

        st, b = jcall(
            "POST",
            "/exec",
            {
                "cmds": [
                    {"argv": ["echo", "hello-union"]},
                    {"shell": "echo shell-$((6*7))"},
                ],
                "timeout_s": 10,
            },
        )
        rs = b.get("results", [])
        check(
            "exec argv+shell",
            st == 200
            and len(rs) == 2
            and rs[0]["stdout"].strip() == "hello-union"
            and rs[1]["stdout"].strip() == "shell-42"
            and "_t_received" in b,
        )

        st, b = jcall(
            "POST", "/exec", {"cmds": [{"argv": ["sleep", "311"]}], "timeout_s": 1}
        )
        r = b["results"][0]
        check(
            "exec timeout kills",
            r.get("timeout") is True,
            f"dur={r.get('duration_ms')}",
        )
        time.sleep(0.5)
        ps = subprocess.run(
            ["pgrep", "-f", "sleep 311"], capture_output=True, text=True
        ).stdout.strip()
        check("no orphan after timeout", ps == "", f"pgrep={ps!r}")

        holder = {}

        def long_exec():
            holder["r"] = jcall(
                "POST",
                "/exec",
                {"cmds": [{"argv": ["sleep", "312"]}], "timeout_s": 60},
                headers={"X-Trip-Id": "parity-trip-1"},
                timeout=70,
            )

        t = threading.Thread(target=long_exec)
        t.start()
        time.sleep(1.0)
        st, b = jcall("POST", "/exec/parity-trip-1/cancel", {})
        check("cancel accepted", st == 200 and b.get("cancelled", 0) >= 1, str(b))
        t.join(timeout=15)
        r = holder["r"][1]["results"][0]
        # Live contract: /cancel kills via the trip's proc registry, so
        # the exec returns promptly, killed — the cancelled flag is set
        # only on the disconnect-watchdog path.
        check(
            "exec cancelled result",
            r.get("duration_ms", 999999) < 15000 and r.get("exit") in (None, -15, -9),
            f"dur={r.get('duration_ms')} exit={r.get('exit')}",
        )

        st, b = jcall("POST", "/exec", {"cmds": [{"nope": 1}]})
        check("bad cmd 400", st == 400, str(st))
        st, b = jcall("POST", "/exec", {"cmds": [{"argv": ["true"]}], "timeout_s": "x"})
        check("bad timeout 400", st == 400, str(st))
        st, hd, raw = call("POST", "/exec", raw=b"{not json")
        check("malformed json 400", st == 400 and json.loads(raw).get("error"), str(st))

        st, b = jcall("POST", "/jobs", {"shell": "echo job-out; sleep 2"})
        jid = b.get("job_id")
        check("job created", st == 200 and bool(jid) and "_t_received" in b)
        done = None
        for _ in range(40):
            st, jb = jcall("GET", f"/jobs/{jid}")
            if jb.get("state") == "done":
                done = jb
                break
            time.sleep(0.5)
        check(
            "job completes with output",
            done is not None
            and done.get("exit") == 0
            and "job-out" in done.get("stdout", ""),
            str(done)[:120],
        )

        st, b = jcall("POST", "/jobs", {"argv": ["sleep", "313"]})
        jid2 = b["job_id"]
        st, b = jcall("GET", "/jobs")
        summ = [x for x in b.get("jobs", []) if x["job_id"] == jid2]
        check(
            "jobs list summary shape",
            bool(summ) and "stdout" not in summ[0] and summ[0]["state"] == "running",
        )
        st, b = jcall("POST", f"/jobs/{jid2}/kill", {})
        check("job kill", st == 200 and b.get("ok") is True)
        for _ in range(20):
            st, jb = jcall("GET", f"/jobs/{jid2}")
            if jb.get("state") == "done":
                break
            time.sleep(0.5)
        check("killed job done", jb.get("state") == "done")

        st, hd, raw = call("GET", "/file?path=/tmp/parity-file.txt&off=10&len=5")
        check(
            "file range",
            st == 200
            and raw == bytes(range(10, 15))
            and hd.get("X-File-Size") == "100",
            f"{st} {raw!r}",
        )

        # Restart persistence: running job becomes done+interrupted;
        # finished job stays pollable.
        st, b = jcall("POST", "/jobs", {"argv": ["sleep", "314"]})
        jid3 = b["job_id"]
        time.sleep(0.5)
    finally:
        d.terminate()
        d.wait(timeout=10)
    d2 = start_daemon()
    try:
        st, jb = jcall("GET", f"/jobs/{jid3}")
        check(
            "restart: running job interrupted",
            jb.get("state") == "done" and jb.get("interrupted") is True,
            str({k: jb.get(k) for k in ("state", "interrupted")}),
        )
        st, jb = jcall("GET", f"/jobs/{jid}")
        check(
            "restart: finished job pollable",
            jb.get("state") == "done"
            and jb.get("exit") == 0
            and "job-out" in jb.get("stdout", ""),
            str(jb)[:120],
        )
        trips = open(os.path.join(STATE, "trips.jsonl")).read()
        kinds = [json.loads(ln)["kind"] for ln in trips.splitlines() if ln.strip()]
        check(
            "trips.jsonl has exec+job records",
            "exec" in kinds and "job" in kinds,
            str(kinds),
        )
        # Cleanup: the interrupted job's sleep outlived the first
        # daemon (daemon SIGTERM does not reap job groups); kill any
        # straggler sleeps from this test's unique durations.
        for dur in ("311", "312", "313", "314"):
            subprocess.run(["pkill", "-f", f"sleep {dur}"], capture_output=True)
    finally:
        d2.terminate()
        d2.wait(timeout=10)
    print("RESULT:", "ALL PASS" if not FAILS else f"FAILURES: {FAILS}")
    return 1 if FAILS else 0


if __name__ == "__main__":
    sys.exit(main())
