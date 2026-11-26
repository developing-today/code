#!/usr/bin/env python3
"""Conductor-silence heartbeat for the fleet conductor (code#15123).

The gap this closes: the fleet conductor (babysitter/dispatcher
successor since the 2026-10-30 cutover, code#2153) owns stall
detection / nudges / session recovery for non-lane sessions, but
nothing watched the conductor itself. Its cycle series simply stopped
being launched after the 2026-11-24T04:25Z cycle, and the silence was
only noticed by a meta-review ~33h later (code#15123). The existing
fleet-watchdog checks Muse cron jobs; the conductor was a laptop
OpenCode session series with no schedule definition on any surface,
so no job-based watchdog could see it stop.

This probe asserts on the one surface that cannot lie that way: the
conductor's own cycle log. It reads the newest cycle timestamp in
`conductor-log.md` and measures its age. Past the threshold (default
2h — the conductor ran ~hourly, 23 cycles in the 24h before it
stopped, so 2h is two missed cycles) the conductor is _silent_ and
gets exactly one tracking issue, filed in developing-today/code and
updated — never duplicated — on subsequent silent passes.

The probe never consults scheduler state (cron lists, OpenChamber
scheduledTasks, systemd timers) as evidence of health: the verified
cause of the 2026-11-24 silence is that no schedule existed at all,
so scheduler state is exactly the surface that read "nothing wrong".
Its only reads are the conductor log and the tracking-issue search;
its only writes are the tracking issue (create once, comment
thereafter).

Default is observe-only (dry-run): the probe prints the actions it
would take and writes nothing. Pass `--apply` to file/update.

Usage:
    check_conductor.py [--apply] [--threshold-hours H]
                       [--log-file PATH] [--now ISO8601]
                       [--fixture JSON]

`--fixture` replaces the log read with a JSON file shaped
{"cycles": [<ISO8601>, ...], "tracking": <issue number or null>}, so
the whole decision path runs offline in tests.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

TRACKING_REPO = "developing-today/code"

DEFAULT_THRESHOLD_HOURS = 2.0

# Every gh call is bounded (code#11468): a hung gh would otherwise hang
# the watchdog itself, which is the one outcome a silence probe cannot
# afford. A timeout is a probe error, never a verdict.
GH_TIMEOUT_S = 60.0

TRACKING_TITLE_PREFIX = "Conductor silence: "
TRACKING_LABELS = ["kind/bug", "source/ci-watch", "priority/high"]

# A cycle entry in conductor-log.md carries its cycle timestamp in
# ISO8601 UTC form (the last pre-silence entry is the
# 2026-11-24T04:25Z cycle, per code#15123's evidence).
CYCLE_TS_RE = re.compile(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(?::\d{2})?(?:Z|\+00:00)")


def parse_ts(value: str) -> datetime:
    dt = datetime.fromisoformat(value.replace("Z", "+00:00"))
    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=timezone.utc)
    return dt


def cycle_timestamps(log_text: str) -> list[datetime]:
    """Every cycle timestamp in the log, in file order."""
    return [parse_ts(m.group(0)) for m in CYCLE_TS_RE.finditer(log_text)]


def last_cycle(cycles: list[datetime], now: datetime) -> datetime | None:
    """The newest cycle at or before `now`.

    Future timestamps are ignored: a clock-skewed or pre-dated entry
    must not mask a silence that has already happened.
    """
    past = [c for c in cycles if c <= now]
    return max(past) if past else None


def evaluate(cycles: list[datetime], now: datetime, threshold_hours: float) -> dict:
    """The conductor's silence verdict from its cycle log only."""
    last = last_cycle(cycles, now)
    if last is None:
        return {
            "status": "silent",
            "age_hours": None,
            "threshold_hours": threshold_hours,
            "last_cycle": None,
            "reason": "no cycle timestamp at or before now in the conductor log",
        }
    age_hours = (now - last).total_seconds() / 3600.0
    return {
        "status": "silent" if age_hours > threshold_hours else "ok",
        "age_hours": round(age_hours, 3),
        "threshold_hours": threshold_hours,
        "last_cycle": last.isoformat(),
        "reason": None,
    }


def tracking_title() -> str:
    return (
        f"{TRACKING_TITLE_PREFIX}fleet-conductor — no completed cycle "
        "within threshold"
    )


def plan_actions(evaluation: dict, tracking: int | None) -> list[dict]:
    """File-or-update plan: exactly one tracking issue per silence."""
    if evaluation["status"] != "silent":
        return [{"action": "none", "issue": None}]
    if tracking:
        return [{"action": "update", "issue": tracking}]
    return [{"action": "file", "issue": None}]


def tracking_body(ev: dict) -> str:
    if ev["last_cycle"] is None:
        last_line = "Last completed cycle seen: none in the conductor log."
    else:
        last_line = (
            f"Last completed cycle seen: {ev['last_cycle']} "
            f"({ev['age_hours']}h ago at filing)."
        )
    return (
        "The conductor-silence heartbeat (scripts/fleet/conductor-silence, "
        "code#15123) found no completed fleet-conductor cycle within its "
        f"{ev['threshold_hours']}h threshold.\n\n{last_line}\n\n"
        "Silence is the conductor's failure mode: the 2026-11-24 stop "
        "created no failures to file on — the cycle series simply stopped "
        "being launched — and stayed invisible for ~33h until a "
        "meta-review noticed. This probe asserts on the conductor's own "
        "cycle log only; scheduler state is never consulted, because the "
        "absence of any schedule was the cause. The conductor owns stall "
        "detection / nudges / session recovery for non-lane sessions, so "
        "while it is silent a real stall in such a session has no owner.\n\n"
        "Filed by conductor-silence heartbeat (fleet). One tracking issue "
        "per silence; subsequent silent passes update this issue instead "
        "of filing duplicates. Close it when cycles resume; a later "
        "silence files a fresh issue."
    )


# --- GitHub access (gh CLI only; tracking-issue search is the only read) ---


def gh(args: list[str]) -> str:
    """Run one gh command, bounded by GH_TIMEOUT_S, and return stdout.

    Raises CalledProcessError, TimeoutExpired or OSError; callers turn
    those into an error record (see PROBE_ERRORS)."""
    return subprocess.run(
        ["gh"] + args,
        capture_output=True,
        text=True,
        check=True,
        timeout=GH_TIMEOUT_S,
    ).stdout


def gh_json(args: list[str]) -> list | dict:
    out = gh(args)
    return json.loads(out) if out.strip() else []


# What the GitHub read or write can raise. Each is recorded and the
# pass reports it instead of producing a verdict from partial data
# (code#11468).
PROBE_ERRORS = (
    subprocess.CalledProcessError,
    subprocess.TimeoutExpired,
    OSError,
    json.JSONDecodeError,
)


def describe_error(err: BaseException) -> str:
    if isinstance(err, subprocess.TimeoutExpired):
        return f"gh timed out after {err.timeout:g}s: {' '.join(err.cmd[:3])}"
    if isinstance(err, subprocess.CalledProcessError):
        detail = (err.stderr or "").strip().splitlines()
        tail = f": {detail[-1]}" if detail else ""
        return f"gh exited {err.returncode}: {' '.join(err.cmd[:3])}{tail}"
    if isinstance(err, json.JSONDecodeError):
        return f"gh returned unparseable JSON: {err.msg}"
    return f"{type(err).__name__}: {err}"


def error_evaluation(threshold_hours: float, err: BaseException) -> dict:
    """Input that could not be read: no verdict, an error."""
    return {
        "status": "error",
        "age_hours": None,
        "threshold_hours": threshold_hours,
        "last_cycle": None,
        "reason": describe_error(err),
    }


def find_tracking_issue() -> int | None:
    issues = gh_json(
        [
            "issue",
            "list",
            "--repo",
            TRACKING_REPO,
            "--state",
            "open",
            "--search",
            f'"{TRACKING_TITLE_PREFIX}" in:title',
            "--json",
            "number,title",
        ]
    )
    for issue in issues:  # type: ignore[union-attr]
        if issue["title"].startswith(TRACKING_TITLE_PREFIX):
            return issue["number"]
    return None


def apply_action(action: dict, ev: dict) -> None:
    if action["action"] == "file":
        args = [
            "issue",
            "create",
            "--repo",
            TRACKING_REPO,
            "--title",
            tracking_title(),
            "--body",
            tracking_body(ev),
        ]
        for label in TRACKING_LABELS:
            args += ["--label", label]
        gh(args)
    elif action["action"] == "update":
        gh(
            [
                "issue",
                "comment",
                str(action["issue"]),
                "--repo",
                TRACKING_REPO,
                "--body",
                f"Still silent at this pass: {tracking_body(ev)}",
            ]
        )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--apply",
        action="store_true",
        help="file/update the tracking issue (default: observe only)",
    )
    parser.add_argument(
        "--threshold-hours",
        type=float,
        default=DEFAULT_THRESHOLD_HOURS,
        help="silence threshold in hours (default: %(default)s)",
    )
    parser.add_argument(
        "--log-file",
        help="path to conductor-log.md (required unless --fixture is given)",
    )
    parser.add_argument("--now", help="ISO8601 'now' (tests/dry-runs)")
    parser.add_argument("--fixture", help="offline fixture JSON (see module docstring)")
    args = parser.parse_args(argv)

    now = parse_ts(args.now) if args.now else datetime.now(timezone.utc)

    fixture = None
    if args.fixture:
        with open(args.fixture) as fh:
            fixture = json.load(fh)

    evaluation = None
    tracking = None
    if fixture is not None:
        cycles = [parse_ts(c) for c in fixture.get("cycles", [])]
        tracking = fixture.get("tracking")
        evaluation = evaluate(cycles, now, args.threshold_hours)
    else:
        if not args.log_file:
            parser.error("--log-file is required unless --fixture is given")
        try:
            log_text = Path(args.log_file).read_text()
        except OSError as err:
            # An unreadable log is a probe error, never a silence
            # verdict: the log host being down must not file issues.
            evaluation = error_evaluation(args.threshold_hours, err)
        else:
            try:
                tracking = find_tracking_issue()
            except PROBE_ERRORS as err:
                evaluation = error_evaluation(args.threshold_hours, err)
            else:
                evaluation = evaluate(
                    cycle_timestamps(log_text), now, args.threshold_hours
                )

    actions = plan_actions(evaluation, tracking)
    apply_errors = []
    if args.apply and fixture is None and evaluation["status"] == "silent":
        for action in actions:
            if action["action"] == "none":
                continue
            try:
                apply_action(action, evaluation)
            except PROBE_ERRORS as err:
                apply_errors.append(
                    {"action": action["action"], "reason": describe_error(err)}
                )

    print(
        json.dumps(
            {
                "now": now.isoformat(),
                "applied": bool(args.apply and fixture is None),
                "evaluation": evaluation,
                "actions": actions,
                "apply_errors": apply_errors,
            },
            indent=2,
        )
    )
    # Silence is a finding, not a failure: a completed pass exits 0
    # whatever it found. A pass whose input could not be read, or
    # whose tracking write failed, exits 1 so a wired-in step cannot
    # read partial output as a clean pass.
    errored = apply_errors or evaluation["status"] == "error"
    return 1 if errored else 0


if __name__ == "__main__":
    sys.exit(main())
