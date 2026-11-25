#!/usr/bin/env python3
"""Offline promotion gate for the plan's one-hour Kaneo RPO (#6816).

Argument contract: --record FILE --source-dsn-file FILE, no positionals or
abbreviations; parse before reads. No network, database calls or writes.
The independent verifier supplies a complete local/R2/Drive archive record
plus source {host, port, database} and recoveryPointAt (data capture time,
not upload/verification completion). This validates evidence, not its producer.
"""

import argparse
from datetime import datetime, timedelta, timezone
from pathlib import Path
import json
import runpy
import sys


def check_recovery(record_path, source_dsn_file, now):
    directory = Path(__file__).resolve().parent
    backup = runpy.run_path(str(directory / "restore-point-check.py"))
    identity = runpy.run_path(str(directory / "kaneo-target-preflight.py"))
    finished = backup["check_record"](Path(record_path), 1, now)
    record = json.loads(Path(record_path).read_text())
    host, port, database = identity["identity"](source_dsn_file)
    source = record.get("source")
    if not isinstance(source, dict) or source != {
        "host": host, "port": port, "database": database
    } or type(source.get("port")) is not int or database != "kaneo":
        raise ValueError("recovery evidence does not identify the source Kaneo database")
    captured = record.get("recoveryPointAt")
    if not isinstance(captured, str):
        raise ValueError("data capture time is missing")
    captured = datetime.fromisoformat(captured.replace("Z", "+00:00"))
    if captured.tzinfo is None:
        raise ValueError("data capture time must include a timezone")
    if captured > datetime.fromisoformat(finished.replace("Z", "+00:00")):
        raise ValueError("data capture time is after verification completion")
    age = now - captured
    if not timedelta(0) <= age <= timedelta(hours=1):
        raise ValueError("data capture time is future-dated or older than one hour")


def main():
    parser = argparse.ArgumentParser(allow_abbrev=False)
    parser.add_argument("--record", required=True)
    parser.add_argument("--source-dsn-file", required=True)
    args = parser.parse_args()
    try:
        # Keep the same complete-generation definition as the existing gate.
        check_recovery(args.record, args.source_dsn_file, datetime.now(timezone.utc))
    except (ValueError, OSError, UnicodeError, TypeError):
        # Never include raw input (in particular the DSN or arbitrary JSON).
        print("Kaneo recovery point REFUSED: require verified source-bound archive with data captured within one hour", file=sys.stderr)
        return 1
    print("Kaneo recovery point OK: source-bound archive; data captured within one hour; local/R2/Drive verified")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
