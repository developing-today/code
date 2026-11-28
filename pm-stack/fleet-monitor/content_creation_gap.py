#!/usr/bin/env python3
"""Report-only fleet-monitor adapter for the content-creation gap detector.

Spec item 6 of developing-today/code#2227 (design #2013, record PR
#2226): alert when queued filing work exists and no content creation
has been seen for more than 5 minutes. The detector itself lives with
the throttle (dezren39/hatch#13,
tools/content-throttle/content_throttle.py); this module is the
monitor-side integration that throttle's README names: the monitor
runs `content_throttle.py gap-check` on its cycle and alerts on
exit 2.

Integration surface (fixed by the throttle side):
  exit 0 = clear, exit 2 = alert, anything else = the detector
  itself is broken.

Usage: python3 -I pm-stack/fleet-monitor/content_creation_gap.py [--throttle-py PATH] [--outbox PATH]
The throttle path also honours CONTENT_THROTTLE_PY. Report-only:
exit 0 prints one JSON record; detector or runner failures exit
non-zero and never print a clear record.
"""

import argparse
import ast
import json
import os
import re
import subprocess
import sys
from pathlib import Path

CHECK_NAME = "content_creation_gap"
THROTTLE_ENV_VAR = "CONTENT_THROTTLE_PY"
DEFAULT_THROTTLE_PY = (
    Path.home() / "hatch" / "tools" / "content-throttle"
    / "content_throttle.py"
)

ALERT_RE = re.compile(
    r"ALERT gap: queued filing work (\{.*\}) and no creation for "
    r"([0-9.]+)s \(threshold ([0-9.]+)s; last creation ([^)]*)\)"
)
OK_RE = re.compile(
    r"ok: no filing gap \(queued (\{.*\}), last creation ([^)]*)\)"
)


def parse_args(argv):
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument(
        "--throttle-py",
        default=os.environ.get(THROTTLE_ENV_VAR, str(DEFAULT_THROTTLE_PY)),
        help="path to content_throttle.py (or set CONTENT_THROTTLE_PY)",
    )
    parser.add_argument(
        "--outbox",
        default=None,
        help="prepared-outbox file the gap check also counts",
    )
    args = parser.parse_args(argv)
    if not args.throttle_py:
        parser.error("--throttle-py is empty and CONTENT_THROTTLE_PY is unset")
    return args


def _parse_queued(text):
    try:
        queued = ast.literal_eval(text)
    except (SyntaxError, ValueError):
        return None
    if not isinstance(queued, dict):
        return None
    return queued


def _none_if_blank(text):
    text = (text or "").strip()
    return text if text and text != "None" else None


def monitor_record(returncode, stdout):
    """Turn the gap-check exit code and output into one monitor record.

    The signal comes from the exit code alone: 0 is clear, 2 is an
    alert. Anything else means the detector itself failed and raises
    instead of faking a record. The human-readable line is parsed
    best-effort for detail fields; when it does not match the known
    format the record keeps the signal and the raw line.
    """
    if returncode not in (0, 2):
        raise ValueError(
            f"gap check exited {returncode}: the detector failed, "
            "not a clear monitor"
        )
    lines = (stdout or "").strip().splitlines()
    detail = lines[-1].strip() if lines else ""
    record = {
        "check": CHECK_NAME,
        "status": "alert" if returncode == 2 else "clear",
        "queued": None,
        "last_creation": None,
        "gap_seconds": None,
        "threshold_seconds": None,
    }
    match = (ALERT_RE.search(detail) if returncode == 2
             else OK_RE.search(detail))
    if match is None:
        record["detail_raw"] = detail
        return record
    if returncode == 2:
        queued_text, gap_text, threshold_text, last_text = match.groups()
        record["gap_seconds"] = float(gap_text)
        record["threshold_seconds"] = float(threshold_text)
    else:
        queued_text, last_text = match.groups()
    record["queued"] = _parse_queued(queued_text)
    record["last_creation"] = _none_if_blank(last_text)
    return record


def run_gap_check(throttle_py, outbox=None, runner=subprocess.run):
    """Run the throttle's gap-check CLI; return (returncode, stdout).

    Raises RuntimeError before any subprocess when the throttle
    script is missing, so a misconfigured monitor fails loudly
    instead of reporting a silent clear.
    """
    if not Path(throttle_py).is_file():
        raise RuntimeError(
            f"throttle script not found: {throttle_py} "
            f"(pass --throttle-py or set {THROTTLE_ENV_VAR})"
        )
    command = [sys.executable, "-I", str(throttle_py), "gap-check"]
    if outbox is not None:
        command.extend(("--outbox", outbox))
    result = runner(command, capture_output=True, text=True, check=False)
    return result.returncode, result.stdout


def main(argv=None, runner=subprocess.run):
    # Parse and validate the complete command line before any subprocess call.
    args = parse_args(sys.argv[1:] if argv is None else argv)
    try:
        returncode, stdout = run_gap_check(
            args.throttle_py, outbox=args.outbox, runner=runner)
        print(json.dumps(monitor_record(returncode, stdout),
                         sort_keys=True))
    except Exception as exc:  # runner/detector failures, never a fake clear
        print(f"content_creation_gap: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
