#!/usr/bin/env python3
"""Five-copy freshness readout for #8584 slice 6 backups.

Reads a JSON status doc describing the last verified success per copy and
reports lag behind the wall clock against each copy's freshness target.

Status doc shape:
  {"generated_at": "<ISO 8601>",
   "copies": {"<name>": {"last_success": "<ISO 8601>" | null,
                          "target_hours": <number>}, ...}}

The five canonical copies from #8584 are turso_export, d1_copy,
cloudflare_rehost, plain_mirror and r2_bundles. A canonical copy that is
absent from the doc, has a null last_success, or is older than its target
is STALE. An unreadable file, bad JSON, bad timestamp, or missing target
is INVALID — reported, never silent.

Command line: --status-json PATH [--now ISO] [--format human|json].
Arguments are parsed strictly before any file is read; anything
unaccounted for is a usage error (exit 2, #3352).

Exit codes: 0 every copy fresh, 1 any copy stale/invalid, 2 usage error.
"""

import argparse
import json
import sys
from datetime import datetime, timezone

CANONICAL_COPIES = (
    "turso_export",
    "d1_copy",
    "cloudflare_rehost",
    "plain_mirror",
    "r2_bundles",
)


def parse_time(value, what):
    try:
        parsed = datetime.fromisoformat(str(value).replace("Z", "+00:00"))
    except (ValueError, TypeError):
        print(
            f"freshness_readout: {what} {value!r} is not an ISO 8601 timestamp",
            file=sys.stderr,
        )
        return None
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)
    return parsed


def load_status(path):
    try:
        with open(path, encoding="utf-8") as fh:
            doc = json.load(fh)
    except OSError as exc:
        return None, [f"cannot read status file: {exc}"]
    except json.JSONDecodeError as exc:
        return None, [f"invalid JSON in status file: {exc}"]
    if not isinstance(doc, dict) or not isinstance(doc.get("copies"), dict):
        return None, ["status file must hold a {'copies': {...}} object"]
    return doc, []


def judge(doc, now):
    results = []
    for name in CANONICAL_COPIES:
        entry = (doc.get("copies") or {}).get(name)
        if entry is None:
            results.append(
                {
                    "copy": name,
                    "verdict": "STALE",
                    "detail": "missing from status doc",
                    "lag_hours": None,
                    "target_hours": None,
                }
            )
            continue
        if not isinstance(entry, dict):
            results.append(
                {
                    "copy": name,
                    "verdict": "INVALID",
                    "detail": "entry must be an object",
                    "lag_hours": None,
                    "target_hours": None,
                }
            )
            continue
        target = entry.get("target_hours")
        if not isinstance(target, (int, float)) or target <= 0:
            results.append(
                {
                    "copy": name,
                    "verdict": "INVALID",
                    "detail": "target_hours must be a positive number",
                    "lag_hours": None,
                    "target_hours": None,
                }
            )
            continue
        last = entry.get("last_success")
        if last is None:
            results.append(
                {
                    "copy": name,
                    "verdict": "STALE",
                    "detail": "never verified",
                    "lag_hours": None,
                    "target_hours": target,
                }
            )
            continue
        when = parse_time(last, f"{name} last_success")
        if when is None:
            results.append(
                {
                    "copy": name,
                    "verdict": "INVALID",
                    "detail": f"bad timestamp {last!r}",
                    "lag_hours": None,
                    "target_hours": target,
                }
            )
            continue
        lag = max(0.0, (now - when).total_seconds() / 3600.0)
        if lag > target:
            verdict, detail = "STALE", f"lag {lag:.1f}h exceeds target {target}h"
        else:
            verdict, detail = "PASS", f"lag {lag:.1f}h within target {target}h"
        results.append(
            {
                "copy": name,
                "verdict": verdict,
                "detail": detail,
                "lag_hours": round(lag, 2),
                "target_hours": target,
            }
        )
    extra = sorted(set((doc.get("copies") or {})) - set(CANONICAL_COPIES))
    for name in extra:
        results.append(
            {
                "copy": name,
                "verdict": "INVALID",
                "detail": "unknown copy; expected one of " + ",".join(CANONICAL_COPIES),
                "lag_hours": None,
                "target_hours": None,
            }
        )
    return results


def _parser():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--status-json", required=True, help="status doc path")
    parser.add_argument("--now", help="pin the clock (ISO 8601, tests only)")
    parser.add_argument(
        "--format", choices=("human", "json"), default="human", help="output format"
    )
    return parser


def main(argv=None):
    args = _parser().parse_args(argv)
    now = parse_time(args.now, "--now") if args.now else datetime.now(timezone.utc)
    if now is None:
        return 2
    doc, problems = load_status(args.status_json)
    if doc is None:
        for problem in problems:
            print(f"freshness_readout: {problem}", file=sys.stderr)
        return 1
    results = judge(doc, now)
    if args.format == "json":
        print(json.dumps({"results": results}, indent=2, sort_keys=True))
    else:
        for row in results:
            print(
                f"freshness_readout: {row['copy']}: {row['verdict']}: {row['detail']}"
            )
        stale = [r["copy"] for r in results if r["verdict"] != "PASS"]
        if stale:
            print(
                "freshness_readout: STALE/INVALID copies: " + ", ".join(stale),
                file=sys.stderr,
            )
        else:
            print("freshness_readout: all five copies fresh")
    return 0 if all(r["verdict"] == "PASS" for r in results) else 1


if __name__ == "__main__":
    raise SystemExit(main())
