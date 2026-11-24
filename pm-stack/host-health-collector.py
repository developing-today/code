#!/usr/bin/env python3
"""PM host-health collector (close-authority program code#10136, slice #10141).

The data half of silent-failure surfacing. The aggregator's watchdog
tier items are DERIVED, never persisted — but the aggregator runs as a
packaged system service and cannot see the laptop user's systemd
manager or the claimant's state directory. This collector runs as a
laptop user service on a 5-minute timer, reads the host facts the
close-authority design (P5: fail-safe = no-mutation + a signal) says
must never be silent, and writes one JSON snapshot the aggregator
derives its watchdog items from:

  - the opencode claimant mode file (~/.local/state/opencode-claimant/mode):
    missing / unreadable / invalid contents, or the live|shadow value;
  - the active host name, so a watchdog card names which host it saw;
  - the pm-dashboard-backup run record (~/backups/dashboard/.last-run.json):
    the newest generation's date, finish time, result, failing step,
    archive sha, and per-leg outcomes — the restore point the watchdog
    names when a generation fails (#3989 criterion 6);
  - user unit states: pm-git-sync (the deploy-by-merge path, #8171),
    pm-projector-reconcile, pm-kaneo-sync-guard (#10137), and
    pm-dashboard-backup (daily ~04:30 cadence; last success recorded so
    the aggregator can flag staleness past 36h);
  - the matching .timer states, so a dead timer is as visible as a
    failed service.

Snapshot: ~/.local/state/pm-aggregator/host-health.json (override with
--output), written atomically (tmp + rename). A fact that cannot be
read is recorded as state "unknown" WITH the reason — this collector
never invents a healthy value and never fails the run over one bad
fact; it exits non-zero only when the snapshot itself cannot be
written, or on usage errors (exit 2).

Argument contract (repo #3352): the whole command line is declared
below and parsed strictly before any subprocess or file write;
unknown flags, stray positionals, and valueless options are usage
errors naming the offending argument.

Run on the laptop:
  python3 pm-stack/host-health-collector.py
  python3 pm-stack/host-health-collector.py --output /tmp/host-health.json
"""

import argparse
import json
import os
import re
import socket
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

HOME = Path.home()
DEFAULT_OUTPUT = HOME / ".local" / "state" / "pm-aggregator" / "host-health.json"
DEFAULT_MODE_FILE = HOME / ".local" / "state" / "opencode-claimant" / "mode"
DEFAULT_RUN_RECORD = HOME / "backups" / "dashboard" / ".last-run.json"
DEFAULT_FLAKE_LOCK = Path(__file__).resolve().parents[1] / "flake.lock"

UNITS = [
    "pm-git-sync",
    "pm-projector-reconcile",
    "pm-kaneo-sync-guard",
    "pm-dashboard-backup",
]
TIMERS = [f"{u}.timer" for u in UNITS] + ["pm-host-health.timer"]

SHOW_PROPS = [
    "Id",
    "ActiveState",
    "SubState",
    "Result",
    "ExecMainCode",
    "ExecMainStatus",
    "ExecMainExitTimestamp",
    "InactiveEnterTimestamp",
    "ActiveEnterTimestamp",
    "StateChangeTimestamp",
]


def parse_args(argv):
    parser = argparse.ArgumentParser(
        prog="host-health-collector.py",
        allow_abbrev=False,
        description="Collect claimant-mode + PM unit health into the "
        "aggregator's host-health snapshot (code#10141).",
    )
    parser.add_argument(
        "--output",
        default=str(DEFAULT_OUTPUT),
        metavar="PATH",
        help="snapshot path (default: %(default)s).",
    )
    parser.add_argument(
        "--mode-file",
        default=str(DEFAULT_MODE_FILE),
        metavar="PATH",
        help="claimant mode file (default: %(default)s).",
    )
    parser.add_argument(
        "--run-record",
        default=str(DEFAULT_RUN_RECORD),
        metavar="PATH",
        help="pm-dashboard-backup JSON run record (default: %(default)s).",
    )
    return parser.parse_args(argv)


def iso_from_systemd(value):
    """systemd timestamp string -> ISO 8601 UTC, or None.

    `systemctl show` prints timestamps in the host's local time with a
    tz abbreviation strptime cannot reliably parse, so the wall-clock
    part is extracted and interpreted as local time via time.mktime
    (which resolves the correct UTC offset for that date)."""
    if not value or value in ("n/a", "0"):
        return None
    m = re.search(r"(\d{4})-(\d{2})-(\d{2})[T ](\d{2}):(\d{2}):(\d{2})", value)
    if not m:
        return None
    parts = tuple(int(g) for g in m.groups())
    try:
        datetime(*parts)  # mktime may normalize an invalid calendar date.
        epoch = time.mktime(parts + (0, 0, -1))
        return datetime.fromtimestamp(epoch, timezone.utc).isoformat()
    except (ValueError, OverflowError, OSError):
        return None


def systemd_int(value):
    try:
        return int(value)
    except (TypeError, ValueError):
        return None


def completed_outcome(props, record):
    """Return only an evidenced unit outcome and successful exit time."""
    result = props.get("Result") or None
    if result != "success":
        # A definitive failure may concern a step other than the main process.
        return result, None

    exited_at = iso_from_systemd(props.get("ExecMainExitTimestamp"))
    if not exited_at:
        return None, None

    active = record["activeState"]
    if active == "activating":
        # Result resets at retry start. Only a retained exit strictly before
        # the start state tells us about a previous run. A later transition
        # such as start-post can follow this attempt's main exit. Equal
        # timestamps are ambiguous at systemctl's display granularity.
        changed_at = record["stateChangedAt"]
        if record["subState"] != "start" or not changed_at or exited_at >= changed_at:
            return None, None
    elif active != "inactive" and (active, record["subState"]) != ("active", "exited"):
        return None, None

    code = systemd_int(props.get("ExecMainCode"))
    status = record["execMainStatus"]
    if status is None or status < 0:
        return None, None
    # systemd exposes Linux wait codes: CLD_EXITED, CLD_KILLED, CLD_DUMPED.
    if code == 1:
        return ("success", exited_at) if status == 0 else ("exit-code", None)
    if status > 0 and code in (2, 3):
        return ("signal" if code == 2 else "core-dump"), None
    return None, None


def show_unit(unit):
    out = subprocess.run(
        ["systemctl", "--user", "show", unit, "--property=" + ",".join(SHOW_PROPS)],
        capture_output=True,
        text=True,
    )
    if out.returncode != 0:
        return {
            "state": "unknown",
            "detail": f"systemctl show failed: {out.stderr.strip()[:200]}",
        }
    props = {}
    for line in out.stdout.splitlines():
        key, _, val = line.partition("=")
        props[key.strip()] = val.strip()
    if not props.get("Id"):
        return {"state": "unknown", "detail": "unit not found"}
    record = {
        "activeState": props.get("ActiveState") or "unknown",
        "subState": props.get("SubState") or None,
        "execMainStatus": systemd_int(props.get("ExecMainStatus")),
        "stateChangedAt": iso_from_systemd(props.get("StateChangeTimestamp")),
        "activeEnterAt": iso_from_systemd(props.get("ActiveEnterTimestamp")),
        "inactiveEnterAt": iso_from_systemd(props.get("InactiveEnterTimestamp")),
    }
    record["result"], record["lastSuccessAt"] = completed_outcome(props, record)
    return record


def read_claimant_mode(mode_file):
    path = Path(mode_file)
    if not path.exists():
        return {
            "state": "missing",
            "detail": f"mode file {path} does not exist; claimant "
            "read_mode() defaults to shadow (#10141)",
        }
    try:
        raw = path.read_text()
    except Exception as exc:
        return {"state": "unreadable", "detail": f"mode file unreadable: {exc}"}
    value = raw.strip().lower()
    if value in ("live", "shadow"):
        return {"state": value, "detail": f"mode file content: {value}"}
    return {
        "state": "invalid",
        "detail": f"mode file content {raw.strip()[:40]!r} is not live|shadow; "
        "claimant read_mode() defaults to shadow",
    }


def read_host_name():
    """Active host name for the snapshot (#3989 criterion 6).

    socket.gethostname() cannot fail loudly here: an empty value reads
    as unknown, never as a blank host the watchdog would trust."""
    try:
        name = socket.gethostname().strip()
    except Exception as exc:
        return {"state": "unknown", "detail": f"hostname unreadable: {exc}"}
    if not name:
        return {"state": "unknown", "detail": "hostname is empty"}
    return {"host": name}


def read_opencode_runtime(
    proc_root=Path("/proc"), flake_lock=DEFAULT_FLAKE_LOCK
):
    """Report the OpenCode service executable revision against this repo's pin.

    The process executable is read through /proc/<MainPID>/exe, not inferred
    from systemd's configured ExecStart. A path without an embedded build
    revision is unknown rather than treated as a match.
    """
    try:
        result = subprocess.run(
            [
                "systemctl",
                "--user",
                "show",
                "opencode.service",
                "--property=ActiveState,MainPID",
            ],
            capture_output=True,
            text=True,
            timeout=3,
        )
    except Exception as exc:
        return {"state": "unknown", "detail": f"cannot inspect opencode.service: {exc}"}
    if result.returncode != 0:
        return {
            "state": "unknown",
            "detail": f"systemctl show opencode.service failed: {result.stderr.strip()[:200]}",
        }
    props = {}
    for line in result.stdout.splitlines():
        key, _, value = line.partition("=")
        props[key.strip()] = value.strip()
    if props.get("ActiveState") != "active":
        return {
            "state": "down",
            "activeState": props.get("ActiveState") or "unknown",
            "detail": "opencode.service is not active",
        }
    try:
        pid = int(props.get("MainPID", ""))
        if pid <= 0:
            raise ValueError("MainPID is not positive")
        executable = os.readlink(Path(proc_root) / str(pid) / "exe")
    except Exception as exc:
        return {"state": "unknown", "detail": f"cannot read running opencode executable: {exc}"}

    deleted = executable.endswith(" (deleted)")
    path = executable.removesuffix(" (deleted)")
    match = re.search(r"(?:^|/)opencode-[^/]*\+([0-9a-f]{7,40})(?:/|$)", path)
    runtime_revision = match.group(1) if match else None
    try:
        lock = json.loads(Path(flake_lock).read_text())
        root = lock["nodes"][lock["root"]]
        input_node = root["inputs"]["opencode"]
        if isinstance(input_node, dict):
            input_node = input_node.get("follows")
            if isinstance(input_node, list):
                input_node = input_node[-1]
        expected_revision = lock["nodes"][input_node]["locked"]["rev"]
        if not isinstance(expected_revision, str) or not re.fullmatch(r"[0-9a-f]{40}", expected_revision):
            raise ValueError("locked opencode revision is not a full commit hash")
    except Exception as exc:
        return {
            "state": "unknown",
            "pid": pid,
            "executable": executable,
            "deleted": deleted,
            "runtimeRevision": runtime_revision,
            "detail": f"cannot read pinned opencode revision: {exc}",
        }
    if not runtime_revision or deleted:
        return {
            "state": "unknown",
            "pid": pid,
            "executable": executable,
            "deleted": deleted,
            "runtimeRevision": runtime_revision,
            "expectedRevision": expected_revision,
            "detail": "running executable has no usable embedded revision",
        }
    state = "match" if expected_revision.startswith(runtime_revision) else "mismatch"
    return {
        "state": state,
        "pid": pid,
        "executable": executable,
        "deleted": deleted,
        "runtimeRevision": runtime_revision,
        "expectedRevision": expected_revision,
    }


BACKUP_RESULTS = ("complete", "failed")


def read_backup_record(run_record):
    """Newest verified backup generation (#3989 criterion 6).

    Passes through the pm-dashboard-backup run record
    (~/backups/dashboard/.last-run.json): generation date, finish
    time, result, failing step, archive sha, and per-leg outcomes —
    the restore point the watchdog names. A missing, unreadable,
    unparseable, or shapeless record reads as state "unknown" WITH
    the reason: this collector never invents a generation."""
    path = Path(run_record)
    try:
        raw = path.read_text()
    except Exception as exc:
        return {"state": "unknown", "detail": f"run record unreadable: {exc}"}
    try:
        parsed = json.loads(raw)
    except Exception as exc:
        return {"state": "unknown", "detail": f"run record unparseable: {exc}"}
    if not isinstance(parsed, dict):
        return {"state": "unknown", "detail": "run record is not a JSON object"}
    result = parsed.get("result")
    date = parsed.get("date")
    finished_at = parsed.get("finished_at")
    if result not in BACKUP_RESULTS or not isinstance(date, str) or not date:
        return {
            "state": "unknown",
            "detail": "run record has no complete|failed result with a date",
        }
    if not isinstance(finished_at, str) or not finished_at:
        return {
            "state": "unknown",
            "detail": "run record has no finished_at timestamp",
        }
    destinations = parsed.get("destinations")
    if not isinstance(destinations, dict):
        destinations = {}
    # Snapshot convention is camelCase (cf. show_unit translating
    # ActiveState -> activeState); translate once, at this boundary.
    started = parsed.get("started_at")
    finished = parsed.get("finished_at")
    failing = parsed.get("failing_step")
    sha = parsed.get("archive_sha256")
    exit_code = parsed.get("exit_code")
    return {
        "date": date,
        "startedAt": started if isinstance(started, str) else None,
        "finishedAt": finished,
        "result": result,
        "exitCode": exit_code if isinstance(exit_code, int) else None,
        "failingStep": failing if isinstance(failing, str) else None,
        "archiveSha256": sha if isinstance(sha, str) else None,
        "destinations": {str(k): str(v) for k, v in destinations.items()},
    }


def main(argv=None):
    args = parse_args(argv)
    snapshot = {
        "generatedAt": datetime.now(timezone.utc).isoformat(),
        "host": read_host_name(),
        "claimantMode": read_claimant_mode(args.mode_file),
        "backup": read_backup_record(args.run_record),
        "opencode": read_opencode_runtime(),
        "units": {},
        "timers": {},
    }
    for unit in UNITS:
        try:
            snapshot["units"][unit] = show_unit(f"{unit}.service")
        except Exception as exc:
            snapshot["units"][unit] = {"state": "unknown", "detail": str(exc)[:200]}
    for timer in TIMERS:
        try:
            rec = show_unit(timer)
            snapshot["timers"][timer] = {
                "active": rec.get("activeState") == "active",
                "activeState": rec.get("activeState"),
            }
        except Exception as exc:
            snapshot["timers"][timer] = {"active": None, "detail": str(exc)[:200]}

    output = Path(args.output)
    try:
        output.parent.mkdir(parents=True, exist_ok=True)
        tmp = output.with_suffix(".json.tmp")
        tmp.write_text(json.dumps(snapshot, indent=2) + "\n")
        tmp.replace(output)
    except Exception as exc:
        print(
            f"host-health-collector: FAIL cannot write {output}: {exc}", file=sys.stderr
        )
        return 1
    bad_units = [
        u
        for u, r in snapshot["units"].items()
        if r.get("activeState") == "failed"
        or (r.get("result") not in (None, "success"))
    ]
    print(
        f"host-health-collector: claimant={snapshot['claimantMode']['state']} "
        f"units_bad={bad_units or 'none'} -> {output}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
