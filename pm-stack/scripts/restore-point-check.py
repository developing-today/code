#!/usr/bin/env python3
"""Fail closed unless a recent dashboard backup is a verified restore point.

Oracle: host-health normalization in pm-stack/aggregator/src/host-health.ts
requires a complete generation, archive identity, and verified local/R2/Drive
copies. The cutover policy additionally bounds age to 36 hours by default.
"""

import argparse
import json
import re
import sys
from datetime import datetime, timedelta, timezone
from pathlib import Path


def parse_args(argv):
    parser = argparse.ArgumentParser(allow_abbrev=False)
    parser.add_argument("--record", required=True)
    parser.add_argument("--max-age-hours", type=int, default=36)
    args = parser.parse_args(argv)
    if args.max_age_hours <= 0:
        parser.error("--max-age-hours must be a positive integer")
    return args


def check_record(path: Path, max_age_hours: int, now: datetime) -> str:
    try:
        record = json.loads(path.read_text())
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise ValueError("record is unreadable or invalid JSON") from exc
    return check_record_data(record, max_age_hours, now)


def check_record_data(record, max_age_hours: int, now: datetime) -> str:
    """Validate one parsed snapshot; consumers must not combine generations."""
    if not isinstance(record, dict):
        raise ValueError("record is not an object")
    if record.get("result") != "complete":
        raise ValueError("latest generation is not complete")
    if not isinstance(record.get("date"), str) or not record["date"]:
        raise ValueError("generation date is missing")
    digest = record.get("archiveSha256")
    if not isinstance(digest, str) or re.fullmatch(r"[0-9a-fA-F]{64}", digest) is None:
        raise ValueError("archive identity is missing or invalid")
    destinations = record.get("destinations")
    if not isinstance(destinations, dict) or any(
        destinations.get(name) != "ok" for name in ("local", "r2", "drive")
    ):
        raise ValueError("local, R2, and Drive copies must all be verified")
    finished = record.get("finishedAt")
    if not isinstance(finished, str):
        raise ValueError("finish time is missing")
    try:
        timestamp = datetime.fromisoformat(finished.replace("Z", "+00:00"))
    except ValueError as exc:
        raise ValueError("finish time is invalid") from exc
    if timestamp.tzinfo is None:
        raise ValueError("finish time must include a timezone")
    age = now - timestamp.astimezone(timezone.utc)
    if age < timedelta(0):
        raise ValueError("finish time is in the future")
    if age > timedelta(hours=max_age_hours):
        raise ValueError(f"generation is older than {max_age_hours} hours")
    return timestamp.astimezone(timezone.utc).isoformat().replace("+00:00", "Z")


def main(argv=None):
    args = parse_args(sys.argv[1:] if argv is None else argv)
    try:
        finished = check_record(
            Path(args.record), args.max_age_hours, datetime.now(timezone.utc)
        )
    except ValueError as exc:
        print(f"restore-point-check: unavailable restore point: {exc}", file=sys.stderr)
        return 1
    print(
        f"RESTORE POINT OK: complete generation finished {finished}; local/R2/Drive verified"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
