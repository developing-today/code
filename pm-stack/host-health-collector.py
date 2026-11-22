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
import re
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

HOME = Path.home()
DEFAULT_OUTPUT = HOME / ".local" / "state" / "pm-aggregator" / "host-health.json"
DEFAULT_MODE_FILE = HOME / ".local" / "state" / "opencode-claimant" / "mode"

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


def main(argv=None):
    args = parse_args(argv)
    snapshot = {
        "generatedAt": datetime.now(timezone.utc).isoformat(),
        "claimantMode": read_claimant_mode(args.mode_file),
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
