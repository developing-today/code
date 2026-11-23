#!/usr/bin/env python3
"""Fleet machine watchdog — per-machine health reporting (#2179 §1.5).

Writes the `health` block of fleet machine registry records
(doc/fleet-machine-registry.yaml). Health is a watchdog-written cache
(§3): this script is the only writer of that block, and it never touches
any other field — identity, capabilities, capacity, and secrets in a
record are reviewed declarations, not probes.

Argument contract (#3352): this script declares its whole command line —
exactly one positional argument, REGISTRY (path to the registry YAML),
plus the optional flags below — and parses strictly against that
declaration before any file is read, any probe runs, or any write
happens. Anything unaccounted for is a usage error (exit 2).

The §1.5 minimum health record, refreshed on the watchdog's cadence
(30 minutes — the fleet watchdog's cadence since 2026-11-22, when
Drewry moved the cron from 15 to 30 minutes; a `last_seen` older than
two cadences is stale, and a stale machine is treated as having left
the pool for placement purposes, §1.5/§4):

- reachable: SSH (the probe transport answering on the host) AND the
  OpenCode server responding at the record's declared endpoint;
- load_headroom: 15-minute load average against declared cpu_cores;
- disk_headroom: free space on the worktree volume (the host's home
  volume — the volume §3 capacity declares);
- sessions: the server's session count via the on-host consumer facade
  (OpenChamber's attach API — the way consumers attach today, §1.3).
  The API pages over session history, so the record carries the
  recent-page count and the count updated within one cadence as
  facts; neither is concurrency, so the declared session cap (a
  placement limit, §1.3) is recorded beside them, not gated on —
  this check earns ok by the API answering, unknown when it cannot;
- last_queue_job: the newest `done` task in the fleet queue store
  (informational; recorded, never a pass/fail gate — a queue may be
  legitimately idle). amd-6 is the pool's only execution node, so the
  queue-wide newest completion is its last successful job; the row's
  queue and worker are recorded with it so the attribution is visible.

Health is earned, never guessed. Every check ends in one of
ok / failed / unknown, and unknown always carries the reason the check
could not run. Status derivation:

- the probe transport itself failed -> unknown (nothing was learned
  about the machine; a broken probe path is not a dead machine);
- reachable ran and either half did not respond -> unreachable;
- any gating check (load / disk / sessions) failed -> degraded;
- any gating check unknown -> unknown (healthy must be earned whole);
- every gating check ok -> healthy.

`last_seen` is the last time the machine was actually seen (reachable
ok). It is NOT advanced by a run that could not reach the machine —
that is what makes staleness meaningful. `checks.reported_at` records
when each evaluation ran, so a live watchdog watching a dead machine
is distinguishable from a dead watchdog.

The registry write is surgical: only the target record's health block
is replaced, every other byte of the file is preserved, and the result
must pass validate-fleet-machine-registry.py's validate() before it is
kept — an invalid result restores the original bytes and exits 1.

Transports: `fleet-shell` (default; probes run on the machine via
`fleet shell`, the sandbox seat's path to amd-6) and `local` (the
watchdog running on the machine itself). The queue-store read uses the
estate's Turso Platform flow (as ~/tools/turso-exec.py): the platform
token is read from the settings file named by --turso-settings and is
never printed; without it the queue check reports unknown with that
reason. Requires PyYAML. No other dependencies.
"""

import argparse
import base64
import json
import os
import re
import shutil
import subprocess
import sys
import urllib.error
import urllib.request
from datetime import datetime, timedelta, timezone

import yaml

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import importlib.util

_spec = importlib.util.spec_from_file_location(
    "validate_fleet_machine_registry",
    os.path.join(HERE, "validate-fleet-machine-registry.py"),
)
validator = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(validator)

CADENCE_SECONDS = 30 * 60
STALE_AFTER_SECONDS = 2 * CADENCE_SECONDS
DISK_FREE_MIN_PCT = 10.0
LOAD15_PER_CORE_MAX = 1.0
PROBE_TIMEOUT_SECONDS = 60
GATING_CHECKS = ("reachable", "load_headroom", "disk_headroom", "sessions")
ALL_CHECKS = GATING_CHECKS + ("last_queue_job",)

# The probe runs on the target host and prints exactly one JSON object.
# It gathers facts only; every judgement happens back in this script.
PROBE_PY = r"""
import json, os, shutil, urllib.request, urllib.error

out = {"ssh": True}

def http_status(url, timeout=5):
    try:
        req = urllib.request.Request(url, method="GET")
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return resp.status, resp.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as exc:
        return exc.code, ""
    except Exception as exc:
        return None, str(exc)

# OpenCode server responding: any HTTP answer counts (the API requires
# auth; responding is the §1.5 claim, not authenticating).
status, _ = http_status(os.environ["WD_SERVER_URL"])
out["opencode_server_http"] = status

# Session count via the on-host consumer facade (OpenChamber attach API).
code, body = http_status(os.environ["WD_SESSIONS_URL"])
if code == 200:
    try:
        data = json.loads(body).get("data", [])
        now_ms = int(os.environ["WD_NOW_MS"])
        window_ms = int(os.environ["WD_WINDOW_MS"])
        out["sessions_listed"] = len(data)
        out["sessions_active"] = sum(
            1 for s in data
            if isinstance(s.get("time"), dict)
            and isinstance(s["time"].get("updated"), (int, float))
            and now_ms - s["time"]["updated"] <= window_ms
        )
    except Exception as exc:
        out["sessions_error"] = str(exc)
else:
    out["sessions_error"] = "attach API HTTP %s" % code

try:
    with open("/proc/loadavg", encoding="utf-8") as fh:
        parts = fh.read().split()
    out["load"] = [float(parts[0]), float(parts[1]), float(parts[2])]
except Exception as exc:
    out["load_error"] = str(exc)

try:
    usage = shutil.disk_usage(os.path.expanduser("~"))
    gib = 1024 ** 3  # the registry declares capacity in GiB (§3)
    out["disk"] = {
        "total_gb": round(usage.total / gib),
        "free_gb": round(usage.free / gib),
        "free_pct": round(100.0 * usage.free / usage.total, 1),
    }
except Exception as exc:
    out["disk_error"] = str(exc)

print(json.dumps(out))
"""


def iso(dt):
    return dt.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def parse_iso(text):
    if not isinstance(text, str):
        return None
    try:
        return datetime.strptime(text, "%Y-%m-%dT%H:%M:%SZ").replace(
            tzinfo=timezone.utc
        )
    except ValueError:
        return None


def placement_view(health, now):
    """The reader-side rule of §1.5/§4: what placement may conclude.

    Returns "left" when the health cache is stale (silence is a state
    change, never a pass), otherwise the recorded status. The declared
    record state is never consulted or changed here.
    """
    last_seen = parse_iso((health or {}).get("last_seen"))
    if last_seen is None or (now - last_seen).total_seconds() > STALE_AFTER_SECONDS:
        return "left"
    return health.get("status", "unknown")


def run_probe(record, transport, fleet_cmd, now):
    """Run the on-host probe; return (facts|None, transport_error|None)."""
    env = {
        "WD_SERVER_URL": record["access"]["opencode_server"],
        "WD_SESSIONS_URL": "http://127.0.0.1:3000/api/session",
        "WD_NOW_MS": str(int(now.timestamp() * 1000)),
        "WD_WINDOW_MS": str(CADENCE_SECONDS * 1000),
    }
    if transport == "local":
        local_env = dict(os.environ, **env)
        try:
            proc = subprocess.run(
                [sys.executable, "-c", PROBE_PY],
                capture_output=True,
                text=True,
                timeout=PROBE_TIMEOUT_SECONDS,
                env=local_env,
            )
        except (OSError, subprocess.TimeoutExpired) as exc:
            return None, f"local probe failed to run: {exc}"
    else:
        exports = " ".join(f"{k}={v}" for k, v in env.items())
        payload = base64.b64encode(PROBE_PY.encode()).decode()
        remote = f"{exports} python3 -c \"import base64,sys; exec(base64.b64decode('{payload}'))\""
        try:
            proc = subprocess.run(
                fleet_cmd + ["shell", "--", "bash", "-lc", remote],
                capture_output=True,
                text=True,
                timeout=PROBE_TIMEOUT_SECONDS,
            )
        except (OSError, subprocess.TimeoutExpired) as exc:
            return None, f"fleet shell transport failed: {exc}"
    if proc.returncode != 0:
        return None, f"probe exited {proc.returncode}: {proc.stderr.strip()[:200]}"
    for line in reversed(proc.stdout.strip().splitlines()):
        try:
            return json.loads(line), None
        except ValueError:
            continue
    return None, "probe produced no JSON output"


def queue_last_done(db, settings_path):
    """Newest done task in the fleet queue store, or (None, reason)."""
    try:
        with open(settings_path, encoding="utf-8") as fh:
            platform = json.load(fh)["token"]
    except (OSError, KeyError, ValueError) as exc:
        return None, f"queue store settings unavailable: {exc}"

    def req(method, url, tok, body=None):
        data = json.dumps(body).encode() if body is not None else None
        r = urllib.request.Request(url, data=data, method=method)
        r.add_header("Authorization", f"Bearer {tok}")
        r.add_header("Content-Type", "application/json")
        try:
            with urllib.request.urlopen(r, timeout=30) as resp:
                return resp.status, resp.read().decode()
        except urllib.error.HTTPError as exc:
            return exc.code, exc.read().decode()
        except (urllib.error.URLError, OSError) as exc:
            return None, str(exc)

    org = "dezren39"
    status, body = req(
        "POST",
        f"https://api.turso.tech/v1/organizations/{org}/databases/{db}/auth/tokens",
        platform,
        {},
    )
    if status is None:
        return None, f"queue store unreachable at token mint: {body}"
    if status != 200:
        return None, f"queue store token mint failed (HTTP {status})"
    dbtok = json.loads(body)["jwt"]
    sql = (
        "SELECT id, queue, worker_id, completed_at FROM tasks "
        "WHERE status = 'done' AND completed_at IS NOT NULL "
        "ORDER BY completed_at DESC LIMIT 1"
    )
    status, body = req(
        "POST",
        f"https://{db}-{org}.aws-us-east-1.turso.io/v2/pipeline",
        dbtok,
        {"requests": [{"type": "execute", "stmt": {"sql": sql}}, {"type": "close"}]},
    )
    if status is None:
        return None, f"queue store unreachable at query: {body}"
    if status != 200:
        return None, f"queue store query failed (HTTP {status})"
    try:
        rows = json.loads(body)["results"][0]["response"]["result"]["rows"]
        if not rows:
            return None, "queue store holds no done tasks"
        vals = [c.get("value") for c in rows[0]]
        return {
            "task_id": vals[0],
            "queue": vals[1],
            "worker": vals[2],
            "completed_at": vals[3],
        }, None
    except (KeyError, IndexError, TypeError, ValueError) as exc:
        return None, f"queue store reply unparsable: {exc}"


def build_health(
    record, facts, transport_error, queue_result, queue_error, now, prior_health
):
    """Compute the new health block from probe facts. Pure; unit-tested."""
    checks = {"reported_at": iso(now)}
    if transport_error is not None:
        for name in ALL_CHECKS:
            checks[name] = {"state": "unknown", "reason": transport_error}
        status = "unknown"
        last_seen = prior_health.get("last_seen")
        return {
            "source": "watchdog",
            "last_seen": last_seen,
            "status": status,
            "checks": checks,
        }

    ssh_ok = bool(facts.get("ssh"))
    server_http = facts.get("opencode_server_http")
    reachable = {
        "state": "ok" if (ssh_ok and server_http is not None) else "failed",
        "ssh": ssh_ok,
        "opencode_server_http": server_http,
    }
    checks["reachable"] = reachable

    cores = record["capacity"].get("cpu_cores") or 0
    if facts.get("load") and cores:
        load15 = facts["load"][2]
        ratio = round(load15 / cores, 3)
        checks["load_headroom"] = {
            "state": "ok" if ratio <= LOAD15_PER_CORE_MAX else "failed",
            "load15": facts["load"][2],
            "cpu_cores": cores,
            "load15_per_core": ratio,
        }
    else:
        checks["load_headroom"] = {
            "state": "unknown",
            "reason": facts.get("load_error") or "declared cpu_cores missing",
        }

    if facts.get("disk"):
        disk = facts["disk"]
        checks["disk_headroom"] = {
            "state": "ok" if disk["free_pct"] >= DISK_FREE_MIN_PCT else "failed",
            "volume": "worktree (home)",
            "total_gb": disk["total_gb"],
            "free_gb": disk["free_gb"],
            "free_pct": disk["free_pct"],
        }
    else:
        checks["disk_headroom"] = {
            "state": "unknown",
            "reason": facts.get("disk_error") or "no disk reading",
        }

    cap = record["capacity"].get("max_sessions") or 0
    if facts.get("sessions_listed") is not None:
        checks["sessions"] = {
            "state": "ok",
            "listed": facts["sessions_listed"],
            "active_within_cadence": facts.get("sessions_active"),
            "max_sessions": cap,
            "source": "openchamber attach api",
        }
    else:
        checks["sessions"] = {
            "state": "unknown",
            "reason": facts.get("sessions_error") or "no session reading",
            "source": "openchamber attach api",
        }

    if queue_result is not None:
        checks["last_queue_job"] = dict({"state": "ok"}, **queue_result)
    else:
        checks["last_queue_job"] = {"state": "unknown", "reason": queue_error}

    seen = reachable["state"] == "ok"
    if not seen:
        status = "unreachable"
    elif any(checks[n]["state"] == "failed" for n in GATING_CHECKS if n != "reachable"):
        status = "degraded"
    elif any(checks[n]["state"] == "unknown" for n in GATING_CHECKS):
        status = "unknown"
    else:
        status = "healthy"
    last_seen = iso(now) if seen else prior_health.get("last_seen")
    return {
        "source": "watchdog",
        "last_seen": last_seen,
        "status": status,
        "checks": checks,
    }


def render_health_block(health):
    """Render the health block at registry indentation (4/6/8 spaces)."""
    lines = [
        "    health:",
        "      # Watchdog-written cache (§1.5) — refreshed by",
        "      # scripts/fleet-machine-watchdog.py; never hand-edited (the",
        "      # validator requires the earned checks record below).",
    ]
    body = yaml.safe_dump(health, sort_keys=False, default_flow_style=False)
    for line in body.rstrip("\n").splitlines():
        lines.append("      " + line)
    return lines


def replace_health_block(text, machine_id, health):
    """Replace only machine_id's health block; preserve all other bytes."""
    lines = text.splitlines()
    start = None
    for i, line in enumerate(lines):
        m = re.match(r"^  - id:\s*(\S+)", line)
        if m and m.group(1) == machine_id:
            start = i
            break
    if start is None:
        raise ValueError(f"machine {machine_id!r} not in registry")
    end = len(lines)
    for j in range(start + 1, len(lines)):
        if re.match(r"^  - ", lines[j]):
            end = j
            break
    hstart = None
    for j in range(start, end):
        if re.match(r"^    health:\s*$", lines[j]):
            hstart = j
            break
    if hstart is None:
        raise ValueError(f"machine {machine_id!r} has no health block")
    hend = end
    for j in range(hstart + 1, end):
        if re.match(r"^    \S", lines[j]):  # sibling key at record depth
            hend = j
            break
    new_lines = lines[:hstart] + render_health_block(health) + lines[hend:]
    return "\n".join(new_lines) + "\n"


def main(argv=None):
    parser = argparse.ArgumentParser(
        description="Refresh registry health blocks from live probes (#2179 §1.5)."
    )
    parser.add_argument("registry", help="path to the fleet machine registry YAML")
    parser.add_argument("--machine", help="only this machine id (default: all records)")
    parser.add_argument(
        "--transport",
        choices=["fleet-shell", "local"],
        default="fleet-shell",
        help="how probes reach the machine (default: fleet-shell)",
    )
    parser.add_argument(
        "--fleet",
        default=None,
        help="fleet binary for the fleet-shell transport "
        "(default: `fleet` on PATH, else ~/tools/fleet)",
    )
    parser.add_argument(
        "--queue-db",
        default="fleet-queue",
        help="Turso database holding the fleet queue (default: fleet-queue)",
    )
    parser.add_argument(
        "--turso-settings",
        default="~/.config/turso/settings.json",
        help="Turso platform settings file for the queue read",
    )
    parser.add_argument(
        "--now",
        default=None,
        help="evaluation time, ISO-8601 UTC (default: now); "
        "exists so runs are reproducible and testable",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="print the health blocks that would be written; change nothing",
    )
    args = parser.parse_args(argv)

    now = parse_iso(args.now) if args.now else datetime.now(timezone.utc)
    if now is None:
        parser.error(f"--now {args.now!r} is not ISO-8601 UTC (YYYY-MM-DDTHH:MM:SSZ)")

    fleet_cmd = []
    if args.transport == "fleet-shell":
        fleet_path = (
            args.fleet or shutil.which("fleet") or os.path.expanduser("~/tools/fleet")
        )
        fleet_cmd = [fleet_path]

    try:
        with open(args.registry, encoding="utf-8") as fh:
            original = fh.read()
        doc = yaml.safe_load(original)
    except (OSError, yaml.YAMLError) as exc:
        print(f"error: cannot read registry: {exc}", file=sys.stderr)
        return 1
    errors = validator.validate(doc)
    if errors:
        for err in errors:
            print(
                f"error: registry invalid before watchdog run: {err}", file=sys.stderr
            )
        return 1

    targets = [
        r for r in doc["machines"] if args.machine is None or r["id"] == args.machine
    ]
    if not targets:
        print(f"error: machine {args.machine!r} not in registry", file=sys.stderr)
        return 1

    queue_result, queue_error = queue_last_done(
        args.queue_db, os.path.expanduser(args.turso_settings)
    )

    text = original
    for record in targets:
        facts, transport_error = (None, None)
        if record["state"] == "retired":
            print(f"{record['id']}: retired — skipped")
            continue
        facts, transport_error = run_probe(record, args.transport, fleet_cmd, now)
        health = build_health(
            record,
            facts,
            transport_error,
            queue_result,
            queue_error,
            now,
            record.get("health") or {},
        )
        view = placement_view(health, now)
        summary = ", ".join(
            f"{name}={health['checks'][name]['state']}" for name in ALL_CHECKS
        )
        print(
            f"{record['id']}: status={health['status']} placement={view} "
            f"last_seen={health['last_seen']} [{summary}]"
        )
        if args.dry_run:
            print("\n".join(render_health_block(health)))
            continue
        text = replace_health_block(text, record["id"], health)

    if args.dry_run:
        return 0
    new_doc = yaml.safe_load(text)
    errors = validator.validate(new_doc)
    if errors:
        for err in errors:
            print(
                f"error: watchdog write would invalidate the registry: {err}",
                file=sys.stderr,
            )
        return 1
    with open(args.registry, "w", encoding="utf-8") as fh:
        fh.write(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
