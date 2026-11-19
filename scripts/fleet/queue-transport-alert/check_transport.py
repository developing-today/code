#!/usr/bin/env python3
"""Repo-queue broker transport alert (code#6369).

The gap this closes: when the queue-broker transport is down, every
worker lane exits silently at its Step 0 `repo-queue health` gate, by
design. A total queue outage therefore produced no alert at all; the
fleet just starved until someone noticed and filed by hand (#5708 on
2026-11-04, and again on 2026-11-18 when the broker host's Tailscale was
stopped for a rebuild from 18:51 to well past 21:00 CST).

This probe runs on its own cadence, independent of any lane shift, and
depends only on the transport it watches, never on the queue itself:

1. It runs one probe: `fleet queue health` by default (`--probe-cmd`),
   or a bare TCP connect to the broker (`--tcp host:port`), bounded by
   `--probe-timeout`. A non-zero exit, a timeout or a refused connect is
   a failed probe.
2. It keeps a small local state file: consecutive failures, when the
   failing window started, whether this window was alerted, and the
   incident issue it alerted on.
3. On `--threshold` consecutive failures (default 2, so about 20
   minutes at a 10-minute cadence) it alerts ONCE for the window: with
   `--apply` it first searches open issues for a transport incident and
   comments on that one if it exists, and otherwise files exactly one.
4. On the first passing probe after an alerted window it announces
   recovery ONCE: a comment with the outage span. It never closes the
   incident: automation may label and comment but must not close an
   issue (#10136), so a human closes it. A silent gap in lane reports
   is then distinguishable from a healthy idle queue.

Every pass prints one JSON document. Its `message` field is the one
line a scheduled run should deliver to the main chat: empty on a pass
with nothing new, the alert on the alerting pass, the recovery on the
recovering pass. `--format text` prints only that line.

Default is observe-only: the state file is kept (it is local, and the
consecutive count needs it) but nothing is written to GitHub. Pass
`--apply` to file and comment.

Exit status: 0 on a completed pass whatever the probe found (an outage
is a finding, not a probe failure); 1 when an `--apply` GitHub write
failed, so a wired-in step never reads a partial pass as clean; 2 on a
usage error.

Usage:
    check_transport.py [--apply] [--state PATH] [--threshold N]
                       [--probe-cmd CMD | --tcp HOST:PORT]
                       [--probe-timeout S] [--repo owner/name]
                       [--now ISO8601] [--probe-result ok|fail[:detail]]
                       [--format json|text]

`--probe-result` skips the real probe (used by the tests); `--now`
pins the clock.
"""

from __future__ import annotations

import argparse
import json
import os
import shlex
import socket
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

DEFAULT_REPO = "developing-today/code"
DEFAULT_PROBE_CMD = "fleet queue health"
DEFAULT_THRESHOLD = 2
DEFAULT_PROBE_TIMEOUT_S = 60.0
DEFAULT_STATE = Path.home() / ".local/state/queue-transport-alert/state.json"

# Every gh call is bounded, as in ci-silence (code#11468): a hung gh
# would otherwise hang the scheduled run and hide the very outage.
GH_TIMEOUT_S = 60.0

# The dedupe key. Every incident this probe files carries this title
# prefix, and the search for an open incident matches on it, so a
# hand-filed incident that uses the same prefix is reused too.
INCIDENT_TITLE_PREFIX = "Queue transport incident:"
INCIDENT_LABELS = ["kind/bug", "priority/high", "area/infra", "source/fleet-monitor"]

DETAIL_MAX = 400


def now_utc() -> datetime:
    return datetime.now(timezone.utc).replace(microsecond=0)


def parse_ts(value: str) -> datetime:
    ts = datetime.fromisoformat(value.replace("Z", "+00:00"))
    if ts.tzinfo is None:
        ts = ts.replace(tzinfo=timezone.utc)
    return ts.astimezone(timezone.utc)


def iso(ts: datetime) -> str:
    return ts.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def span(start: datetime, end: datetime) -> str:
    minutes = max(0, int((end - start).total_seconds() // 60))
    hours, mins = divmod(minutes, 60)
    return f"{hours}h{mins:02d}m" if hours else f"{mins}m"


# --- state ---------------------------------------------------------------

EMPTY_STATE = {
    "consecutive_failures": 0,
    "window_start": None,
    "last_failure_detail": None,
    "alerted": False,
    "incident": None,
    "incident_filed_by_probe": False,
    "last_ok": None,
    "last_probe": None,
}


def load_state(path: Path) -> dict:
    try:
        raw = json.loads(path.read_text())
    except FileNotFoundError:
        return dict(EMPTY_STATE)
    except (OSError, ValueError):
        # A corrupt state file must not stop the probe from alerting.
        # Starting fresh can at worst delay one alert by `threshold`
        # passes; it can never suppress one.
        return dict(EMPTY_STATE)
    state = dict(EMPTY_STATE)
    if isinstance(raw, dict):
        state.update({k: raw[k] for k in EMPTY_STATE if k in raw})
    return state


def save_state(path: Path, state: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(path.name + ".tmp")
    tmp.write_text(json.dumps(state, indent=2, sort_keys=True) + "\n")
    os.replace(tmp, path)


# --- probe ---------------------------------------------------------------


def clip(text: str) -> str:
    text = " ".join(text.split())
    return text if len(text) <= DETAIL_MAX else text[: DETAIL_MAX - 3] + "..."


def probe_command(cmd: list[str], timeout: float) -> tuple[bool, str]:
    try:
        proc = subprocess.run(
            cmd,
            stdin=subprocess.DEVNULL,
            capture_output=True,
            text=True,
            timeout=timeout,
            check=False,
        )
    except subprocess.TimeoutExpired:
        return False, f"`{shlex.join(cmd)}` timed out after {timeout:g}s"
    except OSError as err:
        return False, f"`{shlex.join(cmd)}` could not start: {err}"
    if proc.returncode == 0:
        return True, ""
    tail = clip((proc.stderr or proc.stdout or "").strip()[-DETAIL_MAX:])
    detail = f"`{shlex.join(cmd)}` exited {proc.returncode}"
    return False, f"{detail}: {tail}" if tail else detail


def probe_tcp(target: str, timeout: float) -> tuple[bool, str]:
    host, _, port = target.rpartition(":")
    try:
        with socket.create_connection((host, int(port)), timeout=timeout):
            return True, ""
    except (OSError, ValueError) as err:
        return False, f"TCP connect to {target} failed: {err}"


def parse_probe_result(value: str) -> tuple[bool, str]:
    if value == "ok":
        return True, ""
    if value == "fail" or value.startswith("fail:"):
        return False, value[5:] or "probe failed"
    raise ValueError(f"--probe-result must be ok or fail[:detail], got {value!r}")


# --- decision ------------------------------------------------------------


def step(
    state: dict, ok: bool, detail: str, now: datetime, threshold: int
) -> tuple[dict, str]:
    """Advance the state by one probe result. Returns (new state, event).

    event is one of: "ok", "recovered", "failing", "alert", "down".
    "alert" fires once per failing window, at the threshold-th
    consecutive failure; "recovered" fires once, on the first pass after
    an alerted window. A window that recovers before it reaches the
    threshold ends quietly ("ok").
    """
    state = dict(state)
    state["last_probe"] = iso(now)
    if ok:
        event = "recovered" if state["alerted"] else "ok"
        if event == "ok":
            state.update(EMPTY_STATE, last_ok=iso(now), last_probe=iso(now))
        else:
            state["consecutive_failures"] = 0
            state["last_ok"] = iso(now)
        return state, event
    state["consecutive_failures"] = int(state["consecutive_failures"]) + 1
    state["last_failure_detail"] = detail
    if not state["window_start"]:
        state["window_start"] = iso(now)
    if state["alerted"]:
        return state, "down"
    if state["consecutive_failures"] >= threshold:
        state["alerted"] = True
        return state, "alert"
    return state, "failing"


def finish_recovery(state: dict, now: datetime) -> dict:
    """The state after a recovery has been announced: a fresh window."""
    return dict(EMPTY_STATE, last_ok=iso(now), last_probe=iso(now))


def incident_title(window_start: datetime) -> str:
    return f"{INCIDENT_TITLE_PREFIX} repo-queue broker unreachable since {iso(window_start)}"


def alert_message(state: dict, now: datetime, incident: int | None) -> str:
    start = parse_ts(state["window_start"])
    where = f" Tracking on #{incident}." if incident else ""
    return (
        f"The repo-queue broker has been unreachable for {span(start, now)} "
        f"({state['consecutive_failures']} failed probes since {iso(start)}), "
        f"so worker lanes are starving at their health gate.{where}"
    )


def recovery_message(state: dict, now: datetime, incident: int | None) -> str:
    start = parse_ts(state["window_start"])
    where = f" Noted on #{incident}." if incident else ""
    return (
        f"The repo-queue broker is reachable again after about {span(start, now)} "
        f"down ({iso(start)} to {iso(now)}).{where}"
    )


def incident_body(state: dict, now: datetime, probe_desc: str) -> str:
    start = parse_ts(state["window_start"])
    return "\n".join(
        [
            (
                "Filed automatically by the queue transport probe "
                "(`scripts/fleet/queue-transport-alert/check_transport.py`, code#6369)."
            ),
            "",
            f"- **Probe:** {probe_desc}",
            f"- **Failing since:** {iso(start)}",
            f"- **Consecutive failed probes:** {state['consecutive_failures']}",
            f"- **Last failure:** {state['last_failure_detail'] or 'n/a'}",
            "",
            (
                "While the transport is down every worker lane exits at its Step 0 "
                "`repo-queue health` gate, so the fleet starves silently. The probe "
                "comments here once when the transport recovers; close this issue "
                "by hand once the outage is understood (automation never closes "
                "issues, #10136)."
            ),
        ]
    )


# --- GitHub access (gh CLI only) -----------------------------------------


def gh(args: list[str]) -> str:
    proc = subprocess.run(
        ["gh"] + args,
        stdin=subprocess.DEVNULL,
        capture_output=True,
        text=True,
        timeout=GH_TIMEOUT_S,
        check=True,
    )
    return proc.stdout


def describe_error(err: BaseException) -> str:
    if isinstance(err, subprocess.TimeoutExpired):
        return f"gh timed out after {err.timeout:g}s"
    if isinstance(err, subprocess.CalledProcessError):
        tail = clip((err.stderr or "").strip())
        return f"gh exited {err.returncode}" + (f": {tail}" if tail else "")
    return f"{type(err).__name__}: {err}"


def find_open_incident(repo: str) -> int | None:
    out = gh(
        [
            "issue",
            "list",
            "--repo",
            repo,
            "--state",
            "open",
            "--search",
            f'in:title "{INCIDENT_TITLE_PREFIX}"',
            "--json",
            "number,title",
            "--limit",
            "20",
        ]
    )
    matches = [
        row["number"]
        for row in json.loads(out or "[]")
        if str(row.get("title", "")).startswith(INCIDENT_TITLE_PREFIX)
    ]
    return min(matches) if matches else None


def file_incident(repo: str, title: str, body: str) -> int:
    args = ["issue", "create", "--repo", repo, "--title", title, "--body", body]
    for label in INCIDENT_LABELS:
        args += ["--label", label]
    url = gh(args).strip().splitlines()[-1]
    return int(url.rstrip("/").rsplit("/", 1)[-1])


def comment(repo: str, number: int, body: str) -> None:
    gh(["issue", "comment", str(number), "--repo", repo, "--body", body])


# --- main ----------------------------------------------------------------


def main(argv: list[str] | None = None, github=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--apply", action="store_true", help="file/comment on GitHub")
    ap.add_argument("--state", type=Path, default=DEFAULT_STATE)
    ap.add_argument("--threshold", type=int, default=DEFAULT_THRESHOLD)
    probe = ap.add_mutually_exclusive_group()
    probe.add_argument("--probe-cmd", default=DEFAULT_PROBE_CMD)
    probe.add_argument("--tcp", metavar="HOST:PORT")
    probe.add_argument("--probe-result", metavar="ok|fail[:detail]")
    ap.add_argument("--probe-timeout", type=float, default=DEFAULT_PROBE_TIMEOUT_S)
    ap.add_argument("--repo", default=DEFAULT_REPO)
    ap.add_argument("--now")
    ap.add_argument("--format", choices=["json", "text"], default="json")
    args = ap.parse_args(argv)
    if args.threshold < 1:
        ap.error("--threshold must be at least 1")

    gh_ops = github or {
        "find": find_open_incident,
        "file": file_incident,
        "comment": comment,
    }
    now = parse_ts(args.now) if args.now else now_utc()

    if args.probe_result is not None:
        try:
            ok, detail = parse_probe_result(args.probe_result)
        except ValueError as err:
            ap.error(str(err))
        probe_desc = f"fixed result {args.probe_result}"
    elif args.tcp:
        ok, detail = probe_tcp(args.tcp, args.probe_timeout)
        probe_desc = f"TCP connect to {args.tcp}"
    else:
        cmd = shlex.split(args.probe_cmd)
        ok, detail = probe_command(cmd, args.probe_timeout)
        probe_desc = f"`{shlex.join(cmd)}`"

    state = load_state(args.state)
    state, event = step(state, ok, detail, now, args.threshold)
    message = ""
    apply_errors: list[str] = []
    incident = state.get("incident")

    if event == "alert":
        if args.apply:
            try:
                existing = gh_ops["find"](args.repo)
                if existing:
                    incident = existing
                    gh_ops["comment"](
                        args.repo,
                        existing,
                        "The queue transport probe sees the broker unreachable again: "
                        + alert_message(state, now, None),
                    )
                else:
                    incident = gh_ops["file"](
                        args.repo,
                        incident_title(parse_ts(state["window_start"])),
                        incident_body(state, now, probe_desc),
                    )
                    state["incident_filed_by_probe"] = True
                state["incident"] = incident
            except Exception as err:  # noqa: BLE001 - reported, never swallowed
                apply_errors.append(describe_error(err))
                # Leave the window un-alerted so the next pass retries the
                # write instead of going quiet after a failed filing.
                state["alerted"] = False
        message = alert_message(state, now, state.get("incident"))
    elif event == "recovered":
        message = recovery_message(state, now, incident)
        if args.apply and incident:
            try:
                gh_ops["comment"](args.repo, incident, "Recovered: " + message)
            except Exception as err:  # noqa: BLE001
                apply_errors.append(describe_error(err))
        if not apply_errors:
            state = finish_recovery(state, now)

    save_state(args.state, state)

    report = {
        "checked_at": iso(now),
        "probe": probe_desc,
        "ok": ok,
        "detail": detail,
        "event": event,
        "message": message,
        "incident": state.get("incident") or incident,
        "mode": "apply" if args.apply else "observe",
        "apply_errors": apply_errors,
        "state": state,
    }
    if args.format == "text":
        if message:
            print(message)
    else:
        print(json.dumps(report, indent=2, sort_keys=True))
    return 1 if apply_errors else 0


if __name__ == "__main__":
    sys.exit(main())
