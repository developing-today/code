#!/usr/bin/env python3
"""Render a drift-census JSON report as a #14134 issue comment (#14134 U1 slice 2).

Argument contract (#3352): this script declares its whole command line —
`--report PATH` (a `drift-census.py --format json` document; `-` reads
stdin), `--run-url URL` (optional workflow-run link for the footer) and
`--issue N` (default 14134; informational only — this script never posts,
the scheduled workflow does). It parses strictly against that declaration
before any work. Anything unaccounted for is a usage error with exit 2.

Oracle: `scripts/drift-census.py` `summary()` — `silent` true means every
row is converged or pending (normal lag), so the run stays silent: this
script prints nothing. Anything else is the report: a concise markdown
body with the counts, the `needs_you` services, and one line per
held/drifted row.

Read-only: this module formats text only. It opens the report for
reading, never writes, never shells out, never touches the
network. `test_drift_census_report.py` asserts none is introduced.
"""

from __future__ import annotations

import argparse
import json
import sys

ISSUE = 14134
ATTENTION = ("held", "drifted")


def load_report(text: str) -> dict:
    try:
        report = json.loads(text)
    except json.JSONDecodeError as e:
        raise ValueError(f"not a drift-census JSON report: {e}") from e
    if not isinstance(report, dict):
        raise ValueError("not a drift-census JSON report: top level is not an object")
    for key in ("total", "counts", "silent", "needs_you", "rows"):
        if key not in report:
            raise ValueError(f"not a drift-census JSON report: missing {key!r}")
    return report


def render_markdown(report: dict, run_url: str | None) -> str:
    """Concise issue body for a non-silent census, or '' when silent."""
    if report["silent"]:
        return ""
    counts = report["counts"]
    attention = [r for r in report["rows"] if r.get("state") in ATTENTION]
    lines = [
        f"## drift census: {len(report['needs_you'])} of {report['total']} "
        f"rows need you "
        f"(held={counts.get('held', 0)}, drifted={counts.get('drifted', 0)})",
        "",
    ]
    for row in attention:
        lag = "?" if row.get("lag_s") is None else f"{row['lag_s']}s"
        line = (
            f"- `{row['plane']}/{row['service']}`: {row['state']} — "
            f"pinned={row['pinned']} running={row['running']} lag={lag}"
        )
        if row.get("reason"):
            line += f" ({row['reason']})"
        lines.append(line)
    lines += [
        "",
        f"Full counts: {', '.join(f'{k}={counts.get(k, 0)}' for k in ('converged', 'pending', 'held', 'drifted'))}.",
    ]
    if run_url:
        lines.append(f"Census run: {run_url}")
    lines.append(
        "Read-only census; it never fixes (§4 of the design). "
        "Converged runs stay silent."
    )
    return "\n".join(lines) + "\n"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Render a drift-census JSON report as a #14134 comment body.",
        allow_abbrev=False,
    )
    parser.add_argument(
        "--report",
        required=True,
        help="census --format json document, or - for stdin",
    )
    parser.add_argument("--run-url", default=None, help="workflow run link for the footer")
    parser.add_argument(
        "--issue", type=int, default=ISSUE, help="issue the workflow posts to (default 14134)"
    )
    args = parser.parse_args(argv)

    if args.report == "-":
        text = sys.stdin.read()
    else:
        try:
            with open(args.report, encoding="utf-8") as f:
                text = f.read()
        except OSError as e:
            print(f"drift-census-report: cannot read {args.report}: {e}", file=sys.stderr)
            return 2
    try:
        report = load_report(text)
    except ValueError as e:
        print(f"drift-census-report: {e}", file=sys.stderr)
        return 1
    body = render_markdown(report, args.run_url)
    if body:
        sys.stdout.write(body)
    return 0


if __name__ == "__main__":
    sys.exit(main())
