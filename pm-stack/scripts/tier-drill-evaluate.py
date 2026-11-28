#!/usr/bin/env python3
"""Evaluate a CF-12 (#14952) tier-drill evidence package. Read-only.

Argument contract: --before/--during/--after probe JSON records plus drill
metadata (--outage-start/--outage-end/--restored-at ISO timestamps, --seat,
--command); no positionals/abbreviations. Validate all arguments before
reading evidence files. Stdout is one JSON verdict; exit 0 complete, 1
incomplete with named gaps, 2 invalid arguments or unreadable evidence.
A complete verdict is evidence completeness, not drill success: it says the
package names its surfaces, keeps every record labelled provesDrill false,
and orders the outage window honestly. Every observation must carry an
ordered startedAt/finishedAt pair so mistimed or undated evidence cannot
verdict complete. It never stops services, flips
routes, or declares a drill passed.
"""

import argparse
from datetime import datetime
import json
import sys

EXPECTED_KIND = "tier-probe-observations"
SCHEMA_VERSION = 1


def parse_timestamp(raw):
    try:
        moment = datetime.fromisoformat(str(raw).replace("Z", "+00:00"))
    except (TypeError, ValueError):
        return None
    if moment.tzinfo is None:
        return None
    return moment


def arguments():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--before", required=True)
    parser.add_argument("--during", required=True)
    parser.add_argument("--after", required=True)
    parser.add_argument("--outage-start", required=True)
    parser.add_argument("--outage-end", required=True)
    parser.add_argument("--restored-at", required=True)
    parser.add_argument("--seat", required=True)
    parser.add_argument("--command", required=True)
    args = parser.parse_args()
    if not args.seat.strip():
        parser.error("--seat must be non-empty")
    if not args.command.strip():
        parser.error("--command must be non-empty")
    window = {}
    for flag in ("--outage-start", "--outage-end", "--restored-at"):
        moment = parse_timestamp(getattr(args, flag.lstrip("-").replace("-", "_")))
        if moment is None:
            parser.error("%s must be an ISO-8601 timestamp with a timezone" % flag)
        window[flag] = moment
    if (
        not window["--outage-start"]
        <= window["--outage-end"]
        <= window["--restored-at"]
    ):
        parser.error("--outage-start <= --outage-end <= --restored-at is required")
    args.window = window
    return args


def load_record(path):
    try:
        with open(path, "r", encoding="utf-8") as handle:
            return json.load(handle)
    except (OSError, ValueError) as error:
        print("unreadable evidence %s: %s" % (path, error), file=sys.stderr)
        raise SystemExit(2)


def record_bounds(name, record, gaps):
    started = parse_timestamp(record.get("startedAt"))
    finished = parse_timestamp(record.get("finishedAt"))
    if started is None or finished is None or not started <= finished:
        gaps.append("%s: startedAt/finishedAt are not an ordered timestamp pair" % name)
        return None, None
    return started, finished


def evaluate(before, during, after, window):
    gaps = []
    records = (("before", before), ("during", during), ("after", after))
    usable = {}
    for name, record in records:
        if not isinstance(record, dict) or record.get("kind") != EXPECTED_KIND:
            gaps.append("%s: not a %s record" % (name, EXPECTED_KIND))
            continue
        if record.get("provesDrill") is not False:
            gaps.append(
                "%s: provesDrill is not false — a completion claim "
                "inside the evidence is rejected" % name
            )
        observations = record.get("observations")
        if not isinstance(observations, list) or not observations:
            gaps.append("%s: no observations" % name)
            continue
        urls = []
        for observation in observations:
            if not isinstance(observation, dict) or not observation.get("url"):
                gaps.append("%s: an observation has no url" % name)
                continue
            if "status" not in observation or "error" not in observation:
                gaps.append(
                    "%s: observation %s is missing status/error"
                    % (name, observation.get("url"))
                )
            started = parse_timestamp(observation.get("startedAt"))
            finished = parse_timestamp(observation.get("finishedAt"))
            if started is None or finished is None or not started <= finished:
                gaps.append(
                    "%s: observation %s has no ordered startedAt/finishedAt"
                    % (name, observation.get("url"))
                )
            urls.append(observation["url"])
        if len(set(urls)) != len(urls):
            gaps.append("%s: duplicate observation urls" % name)
        usable[name] = urls
    if "before" in usable:
        expected = set(usable["before"])
        for name in ("during", "after"):
            if name not in usable:
                continue
            actual = set(usable[name])
            for missing in sorted(expected - actual):
                gaps.append("%s: missing surface %s" % (name, missing))
            for extra in sorted(actual - expected):
                gaps.append("%s: surface %s not in the before set" % (name, extra))
    bounds = {}
    for name, record in records:
        started, finished = record_bounds(name, record, gaps)
        if record.get("kind") == EXPECTED_KIND:
            bounds[name] = (started, finished)
    outage_start = window["--outage-start"]
    outage_end = window["--outage-end"]
    restored_at = window["--restored-at"]
    if bounds.get("before") and bounds["before"][0] is not None:
        if not bounds["before"][1] <= outage_start:
            gaps.append("before: record finishes after the outage started")
    if bounds.get("during") and bounds["during"][0] is not None:
        started, finished = bounds["during"]
        if not outage_start <= started and finished <= outage_end:
            gaps.append("during: record is not inside the outage window")
    if bounds.get("after") and bounds["after"][0] is not None:
        if not bounds["after"][0] >= restored_at:
            gaps.append("after: record starts before restoration")
    return gaps


def main():
    args = arguments()
    before = load_record(args.before)
    during = load_record(args.during)
    after = load_record(args.after)
    gaps = evaluate(before, during, after, args.window)
    verdict = {
        "schemaVersion": SCHEMA_VERSION,
        "kind": "tier-drill-evaluation",
        "verdict": "complete" if not gaps else "incomplete",
        "gaps": gaps,
    }
    verdict["records"] = {
        "before": args.before,
        "during": args.during,
        "after": args.after,
    }
    print(json.dumps(verdict, indent=2))
    return 0 if not gaps else 1


if __name__ == "__main__":
    raise SystemExit(main())
