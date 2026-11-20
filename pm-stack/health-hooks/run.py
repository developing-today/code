#!/usr/bin/env python3
"""C3 health hook runner (issue #9571).

Failure-triggered, read-only troubleshoot + report for two monitored
targets: `pm-stack-reachable` and `opencode-working`.

Usage:
  python3 pm-stack/health-hooks/run.py --target pm-stack-reachable \\
      --caller lane-launch --observed-at 2026-...Z [--detail "..."]

  python3 pm-stack/health-hooks/run.py --target opencode-working \\
      --caller aggregator-tick

Exit codes:
  0  hook completed (incident recorded or transient — see stdout JSON)
  2  usage error
  1  unexpected internal error

The runner never mutates state: every probe is read-only. It writes an
incident record (JSON) to the incident store file and, best-effort,
posts the same record to the PM-stack alert throat. It prints one JSON
document on stdout describing what it did.

Environment:
  PM_HEALTH_HOOKS_INCIDENT_FILE  override incident store path
                                 (default ~/.local/state/pm-aggregator/health-hook-incidents.jsonl)
  ALERTS_INTAKE_URL             aggregator base (default http://127.0.0.1:4870)
  PM_AGG_STACK_HEALTH_FILE     stack-health state file (unused here, read for context)
  PM_PRIMARY_CHECKOUT_DIR      override primary checkout dir (default ~/git/code)
"""

import argparse
import json
import os
import socket
import subprocess
import sys
import time
import uuid
from datetime import datetime, timezone
from pathlib import Path
from urllib.request import Request, urlopen
from urllib.error import URLError, HTTPError

HOME = Path.home()
DEFAULT_INCIDENT_FILE = HOME / ".local" / "state" / "pm-aggregator" / "health-hook-incidents.jsonl"
DEFAULT_INTAKE = "http://127.0.0.1:4870"
PRIMARY_CHECKOUT = os.environ.get("PM_PRIMARY_CHECKOUT_DIR", "").strip() or str(HOME / "git" / "code")

TARGETS = {
    "pm-stack-reachable": "PM stack reachable",
    "opencode-working": "OpenCode working",
}

SEQ_CAP_SECONDS = 60


def now_iso() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def parse_args(argv):
    p = argparse.ArgumentParser(prog="run.py", allow_abbrev=False,
                                description="C3 health hook runner")
    p.add_argument("--target", required=True, choices=sorted(TARGETS))
    p.add_argument("--caller", default="unknown", help="who fired the hook")
    p.add_argument("--observed-at", default=None, help="caller's observation timestamp (ISO)")
    p.add_argument("--detail", default="", help="caller-supplied failure detail")
    p.add_argument("--json", action="store_true", help="print JSON (default)")
    args = p.parse_args(argv)
    return args


def tcp_probe(host, port, timeout=5):
    t0 = time.monotonic()
    try:
        with socket.create_connection((host, port), timeout=timeout):
            return {"ok": True, "latencyMs": int((time.monotonic() - t0) * 1000), "error": None}
    except Exception as e:  # noqa: BLE001
        return {"ok": False, "latencyMs": int((time.monotonic() - t0) * 1000), "error": str(e)}


def http_probe(url, timeout=5, alive=None):
    alive = alive or [200]
    t0 = time.monotonic()
    try:
        req = Request(url, method="GET")
        with urlopen(req, timeout=timeout) as resp:
            status = resp.getcode()
            body = resp.read(256)
            ok = (alive == "any") or (status in alive)
            return {"ok": ok, "status": status, "error": None,
                    "latencyMs": int((time.monotonic() - t0) * 1000),
                    "bodyPreview": body.decode("utf-8", "replace")[:120]}
    except HTTPError as e:
        status = e.code
        ok = (alive == "any") or (status in (alive if alive != "any" else []))
        return {"ok": ok, "status": status, "error": f"HTTP {status}",
                "latencyMs": int((time.monotonic() - t0) * 1000)}
    except (URLError, TimeoutError, OSError) as e:
        return {"ok": False, "status": None, "error": str(e),
                "latencyMs": int((time.monotonic() - t0) * 1000)}
    except Exception as e:  # noqa: BLE001
        return {"ok": False, "status": None, "error": str(e),
                "latencyMs": int((time.monotonic() - t0) * 1000)}


def run_cmd(cmd, timeout=5):
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
        return {"rc": proc.returncode, "stdout": proc.stdout.strip(),
                "stderr": proc.stderr.strip()}
    except subprocess.TimeoutExpired:
        return {"rc": -1, "stdout": "", "stderr": "timeout"}
    except FileNotFoundError:
        return {"rc": 127, "stdout": "", "stderr": "command not found"}
    except Exception as e:  # noqa: BLE001
        return {"rc": -2, "stdout": "", "stderr": str(e)}


def systemctl_state(unit):
    r = run_cmd(["systemctl", "--user", "is-active", unit], timeout=5)
    # gather properties
    props = {}
    show = run_cmd(["systemctl", "--user", "show", unit,
                    "--property=ActiveState,SubState,Result,ExecMainExitTimestamp,ActiveEnterTimestamp"],
                   timeout=5)
    if show["rc"] == 0:
        for line in show["stdout"].splitlines():
            if "=" in line:
                k, v = line.split("=", 1)
                props[k] = v
    if r["rc"] == 127:
        return {"unit": unit, "activeState": "unknown", "subState": "unknown",
                "result": "unknown", "exitTimestamp": None,
                "reason": "systemctl not available"}
    return {
        "unit": unit,
        "activeState": props.get("ActiveState") or ("active" if r["rc"] == 0 else "inactive"),
        "subState": props.get("SubState", ""),
        "result": props.get("Result", ""),
        "exitTimestamp": props.get("ExecMainExitTimestamp") or None,
        "activeEnterTimestamp": props.get("ActiveEnterTimestamp") or None,
    }


def git_recent_commits(repo, n=5):
    r = run_cmd(["git", "-C", repo, "log", "--oneline", f"-{n}"], timeout=5)
    if r["rc"] != 0:
        return {"ok": False, "commits": [], "error": r["stderr"] or "git failed"}
    commits = [ln for ln in r["stdout"].splitlines() if ln.strip()]
    return {"ok": True, "commits": commits, "error": None}


def bws_resolve(name):
    # Best-effort: try the bws CLI if present; never print the secret.
    r = run_cmd(["bws", "secret", "get", name], timeout=10)
    if r["rc"] == 127:
        return {"status": "unknown", "detail": "bws CLI not available"}
    if r["rc"] == 0 and r["stdout"]:
        return {"status": "resolved", "detail": "secret resolved by name"}
    return {"status": "missing", "detail": (r["stderr"] or "not resolved")[:200]}


def read_host_health():
    path = HOME / ".local" / "state" / "pm-aggregator" / "host-health.json"
    if not path.exists():
        return {"collectedAt": None, "unitsDown": [], "present": False}
    try:
        data = json.loads(path.read_text())
    except Exception as e:  # noqa: BLE001
        return {"collectedAt": None, "unitsDown": [], "present": True, "error": str(e)}
    units_down = []
    # host-health-collector records user unit states; accept a few shapes.
    units = data.get("units") or data.get("services") or {}
    if isinstance(units, dict):
        for name, info in units.items():
            st = (info or {}).get("state") or (info or {}).get("ActiveState")
            if isinstance(st, str) and st.lower() not in ("active", "up", "running"):
                units_down.append(name)
    elif isinstance(units, list):
        for info in units:
            name = info.get("id") or info.get("name")
            st = info.get("state") or info.get("ActiveState")
            if isinstance(st, str) and st.lower() not in ("active", "up", "running") and name:
                units_down.append(name)
    return {
        "collectedAt": data.get("collectedAt") or data.get("timestamp"),
        "unitsDown": units_down,
        "present": True,
    }


def opencode_route_ref():
    # Read-only: declared default model route from opencode.jsonc
    cfg = Path(PRIMARY_CHECKOUT) / ".opencode" / "opencode.jsonc"
    if not cfg.exists():
        # try relative to cwd
        cfg = Path(".opencode") / "opencode.jsonc"
    if not cfg.exists():
        return {"status": "unknown", "detail": "config not found"}
    try:
        text = cfg.read_text()
    except Exception as e:  # noqa: BLE001
        return {"status": "error", "detail": str(e)}
    # naive: find "model" key
    import re
    m = re.search(r'"model"\s*:\s*"([^"]+)"', text)
    if m:
        return {"status": "resolved", "detail": m.group(1)}
    return {"status": "unknown", "detail": "no model key found"}


def count_sessions_last_hour():
    # Best-effort: look for opencode session store; return None if unavailable
    candidates = [
        HOME / ".local" / "state" / "opencode" / "sessions",
        HOME / ".opencode" / "sessions",
    ]
    for c in candidates:
        if c.exists() and c.is_dir():
            try:
                n = sum(1 for _ in c.iterdir())
                return n
            except Exception:
                pass
    return None


def probe_pm_stack(caller_detail):
    evidence = {}
    # 1. confirm failure: re-probe aggregator
    intake = os.environ.get("ALERTS_INTAKE_URL", DEFAULT_INTAKE).rstrip("/")
    # derive host/port from intake
    host = "127.0.0.1"
    port = 4870
    try:
        from urllib.parse import urlparse
        u = urlparse(intake)
        host = u.hostname or host
        port = u.port or (443 if u.scheme == "https" else 80)
    except Exception:
        pass
    reprobe = http_probe(f"{intake}/board", timeout=5, alive=[200])
    evidence["confirmFailure"] = not reprobe["ok"]

    # 3. reachability
    tcp = tcp_probe(host, port, timeout=5)
    http = http_probe(f"{intake}/board", timeout=5, alive=[200])
    evidence["reachability"] = {
        "ok": http["ok"],
        "status": http.get("status"),
        "error": http.get("error"),
        "latencyMs": http.get("latencyMs"),
        "tcpOk": tcp["ok"],
        "tcpError": tcp.get("error"),
    }

    # 4. service state
    evidence["serviceState"] = systemctl_state("pm-aggregator")

    # 5. auth health
    evidence["authHealth"] = bws_resolve("pm-aggregator.alerts-intake-token")

    # 6. recent changes
    evidence["recentChanges"] = {
        "commits": git_recent_commits(PRIMARY_CHECKOUT, 5).get("commits", []),
        "gitOk": git_recent_commits(PRIMARY_CHECKOUT, 5).get("ok", False),
        "deployMarkerMtime": None,
        "gitSyncOk": None,
    }
    # host-health snapshot
    hh = read_host_health()
    evidence["hostHealth"] = {
        "collectedAt": hh.get("collectedAt"),
        "unitsDown": hh.get("unitsDown", []),
        "present": hh.get("present", False),
    }

    # verdict
    down = (not http["ok"]) and (
        evidence["serviceState"].get("activeState", "").lower() not in ("active", "up", "running")
        or not tcp["ok"]
    )
    if reprobe["ok"]:
        state = "transient"
    elif down:
        state = "down"
    else:
        state = "up"
    summary = {
        "down": f"PM stack unreachable: {http.get('error') or 'HTTP ' + str(http.get('status'))}",
        "transient": "PM stack probe recovered on re-check",
        "up": "PM stack reachable",
    }[state]
    return state, summary, evidence


def probe_opencode(caller_detail):
    evidence = {}
    url = "http://127.0.0.1:13420/global/health"
    reprobe = http_probe(url, timeout=5, alive=[200])
    evidence["confirmFailure"] = not reprobe["ok"]

    tcp = tcp_probe("127.0.0.1", 13420, timeout=5)
    http = http_probe(url, timeout=5, alive=[200])
    evidence["reachability"] = {
        "ok": http["ok"],
        "status": http.get("status"),
        "error": http.get("error"),
        "latencyMs": http.get("latencyMs"),
        "tcpOk": tcp["ok"],
        "tcpError": tcp.get("error"),
    }

    # service state: try opencode user unit
    ss = systemctl_state("opencode")
    if ss.get("activeState") == "unknown" and ss.get("reason") == "systemctl not available":
        pass
    # detect no-unit
    show = run_cmd(["systemctl", "--user", "show", "opencode", "--property=Id"], timeout=5)
    if show["rc"] != 0 or "opencode" not in (show["stdout"] or ""):
        ss["noUnit"] = True
    evidence["serviceState"] = ss

    evidence["authHealth"] = opencode_route_ref()

    rc = git_recent_commits(PRIMARY_CHECKOUT, 5)
    evidence["recentChanges"] = {
        "commits": rc.get("commits", []),
        "gitOk": rc.get("ok", False),
    }
    evidence["sessionLiveness"] = {"sessionsLastHour": count_sessions_last_hour()}

    down = (not http["ok"]) and (not tcp["ok"] or ss.get("activeState", "").lower() not in ("active", "up", "running"))
    if reprobe["ok"]:
        state = "transient"
    elif down:
        state = "down"
    else:
        state = "up"
    summary = {
        "down": f"OpenCode unreachable: {http.get('error') or 'HTTP ' + str(http.get('status'))}",
        "transient": "OpenCode probe recovered on re-check",
        "up": "OpenCode working",
    }[state]
    return state, summary, evidence


def post_to_throat(record):
    intake = os.environ.get("ALERTS_INTAKE_URL", DEFAULT_INTAKE).rstrip("/")
    # resolve token from bws best-effort; if unavailable, skip posting
    tok_result = bws_resolve("pm-aggregator.alerts-intake-token")
    token = None
    # bws_resolve doesn't return the secret; try reading via bws secret get value
    r = run_cmd(["bws", "secret", "get", "pm-aggregator.alerts-intake-token", "--output", "json"], timeout=10)
    if r["rc"] == 0 and r["stdout"]:
        try:
            obj = json.loads(r["stdout"])
            token = obj.get("value") or obj.get("secret")
        except Exception:
            # maybe raw value
            token = r["stdout"].strip() or None
    if not token:
        return {"posted": False, "reason": "intake token unavailable"}
    body = {
        "source": "health-hook",
        "key": record["key"],
        "severity": record["severity"],
        "title": record["title"],
        "body": record["body"],
        "stackHealth": {
            "serviceId": record["target"],
            "name": TARGETS[record["target"]],
            "state": "down" if record["state"] == "down" else "up",
            "since": record["firedAt"],
            "detail": record["body"],
        },
    }
    try:
        data = json.dumps(body).encode()
        req = Request(f"{intake}/api/alerts", data=data, method="POST",
                      headers={"Authorization": f"Bearer {token}",
                               "Content-Type": "application/json"})
        with urlopen(req, timeout=10) as resp:
            resp.read()
            return {"posted": True, "status": resp.getcode()}
    except Exception as e:  # noqa: BLE001
        return {"posted": False, "reason": str(e)}


def append_incident(record):
    path = Path(os.environ.get("PM_HEALTH_HOOKS_INCIDENT_FILE",
                               str(DEFAULT_INCIDENT_FILE)))
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a") as f:
        f.write(json.dumps(record) + "\n")
    return str(path)


def main(argv):
    args = parse_args(argv)
    target = args.target
    fired_at = now_iso()
    start = time.monotonic()

    if target == "pm-stack-reachable":
        state, summary, evidence = probe_pm_stack(args.detail)
    else:
        state, summary, evidence = probe_opencode(args.detail)

    # severity
    if target == "pm-stack-reachable" and state == "down":
        severity = "error"
    elif state == "down":
        severity = "warning"
    else:
        severity = "info"

    incident_id = str(uuid.uuid4())
    record = {
        "source": "health-hook",
        "key": f"health-hook:{target}:{incident_id}",
        "severity": severity,
        "title": f"Health hook: {TARGETS[target]} is {state}",
        "body": summary,
        "target": target,
        "state": state,
        "caller": args.caller,
        "firedAt": fired_at,
        "observedAt": args.observed_at,
        "incidentId": incident_id,
        "recoveryOf": None,
        "evidence": evidence,
    }

    # Only record incidents for down/transient? Record all for audit, but
    # transient ones are marked. We record down + recovery; transient
    # recorded as audit-only (no throat post) to keep noise low.
    throat_result = {"posted": False, "reason": "transient/not-posted"}
    stored_path = None
    if state in ("down", "up"):
        stored_path = append_incident(record)
        throat_result = post_to_throat(record)

    # category chat note via Muse is left to the caller environment; we
    # print the note text for the caller to dispatch.
    note = f"health-hook {target} {state} @{fired_at} caller={args.caller} incident={incident_id} — {summary}"

    elapsed = time.monotonic() - start
    out = {
        "target": target,
        "state": state,
        "severity": severity,
        "incidentId": incident_id,
        "summary": summary,
        "caller": args.caller,
        "firedAt": fired_at,
        "elapsedSeconds": round(elapsed, 3),
        "incidentFile": stored_path,
        "throat": throat_result,
        "categoryNote": note,
        "withinCap": elapsed <= SEQ_CAP_SECONDS,
    }
    print(json.dumps(out, indent=2))
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv[1:]))
    except SystemExit:
        raise
    except Exception as e:  # noqa: BLE001
        print(json.dumps({"error": str(e)}), file=sys.stderr)
        sys.exit(1)