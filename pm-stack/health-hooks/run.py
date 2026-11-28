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
DEFAULT_INCIDENT_FILE = (
    HOME / ".local" / "state" / "pm-aggregator" / "health-hook-incidents.jsonl"
)
DEFAULT_INTAKE = "http://127.0.0.1:4870"
PRIMARY_CHECKOUT = os.environ.get("PM_PRIMARY_CHECKOUT_DIR", "").strip() or str(
    HOME / "git" / "code"
)

TARGETS = {
    "pm-stack-reachable": "PM stack reachable",
    "opencode-working": "OpenCode working",
}

SEQ_CAP_SECONDS = 60


def remaining_seconds(deadline):
    if deadline is None:
        return None
    return max(0.0, deadline - time.monotonic())


def bounded_timeout(timeout, deadline):
    remaining = remaining_seconds(deadline)
    if remaining is None:
        return timeout
    return min(timeout, remaining)


def now_iso() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def parse_args(argv):
    # Each wrapper fixes its own --target; a second --target would let
    # argparse take the last one and run the other target's diagnosis.
    if sum(1 for a in argv if a == "--target" or a.startswith("--target=")) > 1:
        print("run.py: --target may be given only once", file=sys.stderr)
        raise SystemExit(2)
    p = argparse.ArgumentParser(
        prog="run.py", allow_abbrev=False, description="C3 health hook runner"
    )
    p.add_argument("--target", required=True, choices=sorted(TARGETS))
    p.add_argument("--caller", default="unknown", help="who fired the hook")
    p.add_argument(
        "--observed-at", default=None, help="caller's observation timestamp (ISO)"
    )
    p.add_argument("--detail", default="", help="caller-supplied failure detail")
    p.add_argument("--json", action="store_true", help="print JSON (default)")
    args = p.parse_args(argv)
    return args


def tcp_probe(host, port, timeout=5, deadline=None):
    t0 = time.monotonic()
    try:
        budget = bounded_timeout(timeout, deadline)
        if budget <= 0:
            raise TimeoutError("sequence deadline exceeded")
        with socket.create_connection((host, port), timeout=budget):
            return {
                "ok": True,
                "latencyMs": int((time.monotonic() - t0) * 1000),
                "error": None,
            }
    except Exception as e:  # noqa: BLE001
        return {
            "ok": False,
            "latencyMs": int((time.monotonic() - t0) * 1000),
            "error": str(e),
        }


def http_probe(url, timeout=5, alive=None, deadline=None):
    alive = alive or [200]
    t0 = time.monotonic()
    try:
        budget = bounded_timeout(timeout, deadline)
        if budget <= 0:
            raise TimeoutError("sequence deadline exceeded")
        req = Request(url, method="GET")
        with urlopen(req, timeout=budget) as resp:
            status = resp.getcode()
            body = resp.read(256)
            ok = (alive == "any") or (status in alive)
            return {
                "ok": ok,
                "status": status,
                "error": None,
                "latencyMs": int((time.monotonic() - t0) * 1000),
                "bodyPreview": body.decode("utf-8", "replace")[:120],
            }
    except HTTPError as e:
        status = e.code
        ok = (alive == "any") or (status in (alive if alive != "any" else []))
        return {
            "ok": ok,
            "status": status,
            "error": f"HTTP {status}",
            "latencyMs": int((time.monotonic() - t0) * 1000),
        }
    except (URLError, TimeoutError, OSError) as e:
        return {
            "ok": False,
            "status": None,
            "error": str(e),
            "latencyMs": int((time.monotonic() - t0) * 1000),
        }
    except Exception as e:  # noqa: BLE001
        return {
            "ok": False,
            "status": None,
            "error": str(e),
            "latencyMs": int((time.monotonic() - t0) * 1000),
        }


def run_cmd(cmd, timeout=5, deadline=None, env=None):
    try:
        budget = bounded_timeout(timeout, deadline)
        if budget <= 0:
            return {"rc": -1, "stdout": "", "stderr": "sequence deadline exceeded"}
        proc = subprocess.run(
            cmd, capture_output=True, text=True, timeout=budget, env=env
        )
        return {
            "rc": proc.returncode,
            "stdout": proc.stdout.strip(),
            "stderr": proc.stderr.strip(),
        }
    except subprocess.TimeoutExpired:
        return {"rc": -1, "stdout": "", "stderr": "timeout"}
    except FileNotFoundError:
        return {"rc": 127, "stdout": "", "stderr": "command not found"}
    except Exception as e:  # noqa: BLE001
        return {"rc": -2, "stdout": "", "stderr": str(e)}


def systemctl_state(unit, deadline=None):
    r = run_cmd(
        ["systemctl", "--user", "is-active", unit], timeout=5, deadline=deadline
    )
    # gather properties
    props = {}
    show = run_cmd(
        [
            "systemctl",
            "--user",
            "show",
            unit,
            "--property=ActiveState,SubState,Result,ExecMainExitTimestamp,ActiveEnterTimestamp",
        ],
        timeout=5,
        deadline=deadline,
    )
    if show["rc"] == 0:
        for line in show["stdout"].splitlines():
            if "=" in line:
                k, v = line.split("=", 1)
                props[k] = v
    if r["rc"] == 127:
        return {
            "unit": unit,
            "activeState": "unknown",
            "subState": "unknown",
            "result": "unknown",
            "exitTimestamp": None,
            "reason": "systemctl not available",
        }
    return {
        "unit": unit,
        "activeState": props.get("ActiveState")
        or ("active" if r["rc"] == 0 else "inactive"),
        "subState": props.get("SubState", ""),
        "result": props.get("Result", ""),
        "exitTimestamp": props.get("ExecMainExitTimestamp") or None,
        "activeEnterTimestamp": props.get("ActiveEnterTimestamp") or None,
    }


def git_recent_commits(repo, n=5, deadline=None):
    r = run_cmd(
        ["git", "-C", repo, "log", "--oneline", f"-{n}"], timeout=5, deadline=deadline
    )
    if r["rc"] != 0:
        return {"ok": False, "commits": [], "error": r["stderr"] or "git failed"}
    commits = [ln for ln in r["stdout"].splitlines() if ln.strip()]
    return {"ok": True, "commits": commits, "error": None}


def _first_existing(paths):
    for p in paths:
        if p and Path(p).exists():
            return str(p)
    return None


def bws_lookup(key, deadline=None):
    """Resolve a Bitwarden Secrets Manager secret by its key.

    `bws secret get` takes a secret ID, not its key, so this lists the
    secrets, finds the key's ID and fetches that, as the aggregator's
    secrets.ts does. Returns (status, value): status is "resolved",
    "missing" or "unknown"; value is only set when resolved and is never
    logged or put in evidence.
    """
    bws = (
        os.environ.get("BWS_BIN")
        or _first_existing(
            ["/run/current-system/sw/bin/bws", HOME / "tools" / "bin" / "bws"]
        )
        or "bws"
    )
    env = os.environ.copy()
    if not env.get("BWS_ACCESS_TOKEN"):
        token_file = os.environ.get("BWS_ACCESS_TOKEN_FILE") or _first_existing(
            [
                HOME / ".config" / "bws" / "access-token",
                HOME / ".config" / "bitwarden" / "access-token",
            ]
        )
        if not token_file:
            return "unknown", None
        try:
            access = Path(token_file).read_text().strip()
        except OSError:
            return "unknown", None
        if not access:
            return "unknown", None
        env["BWS_ACCESS_TOKEN"] = access
    listed = run_cmd(
        [bws, "secret", "list", "--output", "json"],
        timeout=10,
        deadline=deadline,
        env=env,
    )
    if listed["rc"] == 127:
        return "unknown", None
    if listed["rc"] != 0:
        return "unknown", None
    try:
        entries = json.loads(listed["stdout"] or "[]")
    except json.JSONDecodeError:
        return "unknown", None
    secret_id = next(
        (
            e.get("id")
            for e in entries
            if isinstance(e, dict) and e.get("key") == key and e.get("id")
        ),
        None,
    )
    if not secret_id:
        return "missing", None
    got = run_cmd(
        [bws, "secret", "get", secret_id, "--output", "json"],
        timeout=10,
        deadline=deadline,
        env=env,
    )
    if got["rc"] != 0:
        return "unknown", None
    try:
        value = (json.loads(got["stdout"] or "{}").get("value") or "").strip()
    except (json.JSONDecodeError, AttributeError):
        return "unknown", None
    return ("resolved", value) if value else ("missing", None)


def bws_resolve(name, deadline=None):
    # Evidence only: whether the secret resolves, never its value.
    status, _value = bws_lookup(name, deadline=deadline)
    detail = {
        "resolved": "secret resolved by key",
        "missing": "no secret with that key",
        "unknown": "bws unavailable or unreadable",
    }[status]
    return {"status": status, "detail": detail}


def read_host_health(path=None):
    path = (
        Path(path)
        if path
        else HOME / ".local" / "state" / "pm-aggregator" / "host-health.json"
    )
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
        items = list(units.items())
    elif isinstance(units, list):
        items = [
            ((info or {}).get("id") or (info or {}).get("name"), info) for info in units
        ]
    else:
        items = []
    for name, info in items:
        if name and unit_is_down(info or {}):
            units_down.append(name)
    return {
        "collectedAt": data.get("generatedAt")
        or data.get("collectedAt")
        or data.get("timestamp"),
        "unitsDown": units_down,
        "present": True,
    }


def unit_is_down(info):
    """A unit is down when it failed, not merely when it is inactive.

    Timer-driven oneshots (pm-git-sync, pm-projector-reconcile) sit at
    activeState "inactive" with result "success" between runs; that is
    healthy. A unit counts as down when systemd says it failed, or when
    it is inactive after a run whose result was not success.
    """
    st = info.get("activeState") or info.get("state") or info.get("ActiveState")
    if not isinstance(st, str):
        return False
    st = st.lower()
    if st in ("active", "up", "running", "activating", "reloading"):
        return False
    if st in ("failed", "down"):
        return True
    result = info.get("result") or info.get("Result")
    if isinstance(result, str) and result:
        return result.lower() != "success"
    # Inactive with no recorded result: no evidence of a failure.
    return False


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


def count_sessions_last_hour(db_path=None, now_ms=None):
    """Sessions updated in the trailing hour, read from the live OpenCode
    store (session_v2 in ~/.local/share/opencode/opencode.db, per
    pm-stack/opencode-usage-export/README.md). Opened read-only; returns
    None when the store or its v2 table is not there."""
    import sqlite3

    path = Path(
        db_path
        or os.environ.get("OPENCODE_USAGE_DB", "").strip()
        or HOME / ".local" / "share" / "opencode" / "opencode.db"
    )
    if not path.exists():
        return None
    now_ms = int(time.time() * 1000) if now_ms is None else now_ms
    try:
        db = sqlite3.connect(f"{path.resolve().as_uri()}?mode=ro", uri=True, timeout=2)
    except sqlite3.Error:
        return None
    try:
        row = db.execute(
            "SELECT count(*) FROM session_v2 WHERE time_updated >= ?",
            (now_ms - 3_600_000,),
        ).fetchone()
        return int(row[0]) if row else 0
    except sqlite3.Error:
        return None
    finally:
        db.close()


def probe_pm_stack(caller_detail, deadline=None):
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
    reprobe = http_probe(f"{intake}/board", timeout=5, alive=[200], deadline=deadline)
    evidence["confirmFailure"] = not reprobe["ok"]

    # 3. reachability
    tcp = tcp_probe(host, port, timeout=5, deadline=deadline)
    http = http_probe(f"{intake}/board", timeout=5, alive=[200], deadline=deadline)
    evidence["reachability"] = {
        "ok": http["ok"],
        "status": http.get("status"),
        "error": http.get("error"),
        "latencyMs": http.get("latencyMs"),
        "tcpOk": tcp["ok"],
        "tcpError": tcp.get("error"),
    }

    # 4. service state
    evidence["serviceState"] = systemctl_state("pm-aggregator", deadline=deadline)

    # 5. auth health
    evidence["authHealth"] = bws_resolve(
        "pm-aggregator.alerts-intake-token", deadline=deadline
    )

    # 6. recent changes
    recent = git_recent_commits(PRIMARY_CHECKOUT, 5, deadline=deadline)
    evidence["recentChanges"] = {
        "commits": recent.get("commits", []),
        "gitOk": recent.get("ok", False),
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
    down = not reprobe["ok"] and not http["ok"]
    if reprobe["ok"]:
        state = "transient"
    elif down:
        state = "down"
    else:
        state = "transient"
    summary = {
        "down": f"PM stack unreachable: {http.get('error') or 'HTTP ' + str(http.get('status'))}",
        "transient": "PM stack probe recovered on re-check",
        "up": "PM stack reachable",
    }[state]
    return state, summary, evidence


def probe_opencode(caller_detail, deadline=None):
    evidence = {}
    url = "http://127.0.0.1:13420/global/health"
    reprobe = http_probe(url, timeout=5, alive=[200], deadline=deadline)
    evidence["confirmFailure"] = not reprobe["ok"]

    tcp = tcp_probe("127.0.0.1", 13420, timeout=5, deadline=deadline)
    http = http_probe(url, timeout=5, alive=[200], deadline=deadline)
    evidence["reachability"] = {
        "ok": http["ok"],
        "status": http.get("status"),
        "error": http.get("error"),
        "latencyMs": http.get("latencyMs"),
        "tcpOk": tcp["ok"],
        "tcpError": tcp.get("error"),
    }

    # service state: try opencode user unit
    ss = systemctl_state("opencode", deadline=deadline)
    if (
        ss.get("activeState") == "unknown"
        and ss.get("reason") == "systemctl not available"
    ):
        pass
    # detect no-unit
    show = run_cmd(
        ["systemctl", "--user", "show", "opencode", "--property=Id"],
        timeout=5,
        deadline=deadline,
    )
    if show["rc"] != 0 or "opencode" not in (show["stdout"] or ""):
        ss["noUnit"] = True
    evidence["serviceState"] = ss

    evidence["authHealth"] = opencode_route_ref()

    rc = git_recent_commits(PRIMARY_CHECKOUT, 5, deadline=deadline)
    evidence["recentChanges"] = {
        "commits": rc.get("commits", []),
        "gitOk": rc.get("ok", False),
    }
    evidence["sessionLiveness"] = {"sessionsLastHour": count_sessions_last_hour()}

    down = not reprobe["ok"] and not http["ok"]
    if reprobe["ok"]:
        state = "transient"
    elif down:
        state = "down"
    else:
        state = "transient"
    summary = {
        "down": f"OpenCode unreachable: {http.get('error') or 'HTTP ' + str(http.get('status'))}",
        "transient": "OpenCode probe recovered on re-check",
        "up": "OpenCode working",
    }[state]
    return state, summary, evidence


def post_to_throat(record, deadline=None):
    intake = os.environ.get("ALERTS_INTAKE_URL", DEFAULT_INTAKE).rstrip("/")
    # resolve token from bws best-effort; if unavailable, skip posting
    _status, token = bws_lookup("pm-aggregator.alerts-intake-token", deadline=deadline)
    if not token:
        return {"posted": False, "reason": "intake token unavailable"}
    body = {
        "source": "health-hook",
        "key": record["key"],
        "severity": record["severity"],
        "title": record["title"],
        "body": record["body"],
        # The record's own stackHealth member, built once in main —
        # never rebuilt here, so the JSONL record, this POST body, and
        # the ledger-bound extension below share one shape (#12603).
        "stackHealth": record["stackHealth"],
        # The full incident record as the healthHook extension: the
        # aggregator stores it in the alerts ledger as the C3 record of
        # detail (#12599 decision b). Best-effort like the rest of this
        # post — a rejection never changes the verdict or the record.
        "healthHook": record,
    }
    try:
        data = json.dumps(body).encode()
        req = Request(
            f"{intake}/api/alerts",
            data=data,
            method="POST",
            headers={
                "Authorization": f"Bearer {token}",
                "Content-Type": "application/json",
            },
        )
        budget = bounded_timeout(10, deadline)
        if budget <= 0:
            return {"posted": False, "reason": "sequence deadline exceeded"}
        with urlopen(req, timeout=budget) as resp:
            resp.read()
            return {"posted": True, "status": resp.getcode()}
    except Exception as e:  # noqa: BLE001
        return {"posted": False, "reason": str(e)}


def append_incident(record):
    path = Path(
        os.environ.get("PM_HEALTH_HOOKS_INCIDENT_FILE", str(DEFAULT_INCIDENT_FILE))
    )
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a") as f:
        f.write(json.dumps(record) + "\n")
    return str(path)


def find_open_incident(target):
    path = Path(
        os.environ.get("PM_HEALTH_HOOKS_INCIDENT_FILE", str(DEFAULT_INCIDENT_FILE))
    )
    if not path.exists():
        return None
    open_by_id = {}
    try:
        with path.open() as f:
            for line in f:
                try:
                    record = json.loads(line)
                except (json.JSONDecodeError, TypeError):
                    continue
                if record.get("target") != target:
                    continue
                if record.get("state") == "down" and record.get("incidentId"):
                    open_by_id[record["incidentId"]] = record
                elif (
                    record.get("state") == "up"
                    and record.get("recoveryOf")
                    and record.get("recoveryDelivered", True)
                ):
                    # A recovery closes its outage only once the intake
                    # took it; an undelivered one leaves the outage open
                    # so the next hook run posts the recovery again.
                    # Records written before this field count as delivered.
                    open_by_id.pop(record["recoveryOf"], None)
    except OSError:
        return None
    return next(reversed(open_by_id.values()), None) if open_by_id else None


def main(argv):
    args = parse_args(argv)
    target = args.target
    fired_at = now_iso()
    start = time.monotonic()
    deadline = start + SEQ_CAP_SECONDS

    if target == "pm-stack-reachable":
        state, summary, evidence = probe_pm_stack(args.detail, deadline=deadline)
    else:
        state, summary, evidence = probe_opencode(args.detail, deadline=deadline)

    # A down diagnosis during an open outage reuses that outage (its id
    # and intake key) instead of opening a second alert that a single
    # recovery could not close.
    open_incident = (
        find_open_incident(target) if state in ("transient", "up", "down") else None
    )
    recovery_of = None
    if open_incident and state != "down":
        state = "up"
        recovery_of = open_incident["incidentId"]
        summary = f"{TARGETS[target]} recovered after incident {recovery_of}"

    # severity
    if target == "pm-stack-reachable" and state == "down":
        severity = "critical"
    elif state == "down":
        severity = "warning"
    else:
        severity = "info"

    if state == "down" and open_incident:
        incident_id = open_incident["incidentId"]
    else:
        incident_id = str(uuid.uuid4())
    # A recovery reuses its outage's intake key: the intake closes only
    # the incident whose source and key match.
    intake_key = (open_incident or {}).get(
        "key"
    ) or f"health-hook:{target}:{incident_id}"
    # A recovery reads "recovered", not "is up" (#12603); dedupe is by
    # key, never title, so the wording change cannot split an incident.
    title = (
        f"Health hook: {TARGETS[target]} recovered"
        if state == "up" and recovery_of
        else f"Health hook: {TARGETS[target]} is {state}"
    )
    record = {
        "source": "health-hook",
        "key": intake_key,
        "severity": severity,
        "title": title,
        "body": summary,
        "target": target,
        "state": state,
        "caller": args.caller,
        "firedAt": fired_at,
        "observedAt": args.observed_at,
        "incidentId": incident_id,
        "recoveryOf": recovery_of,
        "evidence": evidence,
        # Built once, here: the JSONL record, the throat POST body, and
        # the ledger-bound healthHook extension all carry this member
        # (#12603, one record shape).
        "stackHealth": {
            "serviceId": target,
            "name": TARGETS[target],
            "state": "down" if state == "down" else "up",
            "since": fired_at,
            "detail": summary,
        },
    }

    # Only record incidents for down/transient? Record all for audit, but
    # transient ones are marked. We record down + recovery; transient
    # recorded as audit-only (no throat post) to keep noise low.
    throat_result = {"posted": False, "reason": "transient/not-posted"}
    stored_path = None
    if state == "up" and recovery_of:
        # Post first, then record whether the intake took the recovery.
        throat_result = post_to_throat(record, deadline=deadline)
        record["recoveryDelivered"] = bool(throat_result.get("posted"))
        stored_path = append_incident(record)
    elif state in ("down", "up"):
        stored_path = append_incident(record)
        throat_result = post_to_throat(record, deadline=deadline)

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
