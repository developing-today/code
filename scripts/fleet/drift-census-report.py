#!/usr/bin/env python3
"""Daily drift census report — the scheduled report path for #14134 U1 slice 2.

Argument contract (#3352): this script declares its whole command line —
`--root PATH`, `--issue N`, `--repo SLUG`, `--fixture PATH`,
`--poster {gh,stdout,none}`, `--now ISO`, `--window-s N` — and parses
strictly against that declaration before any probe or post. Anything
unaccounted for is a usage error with exit 2.

Oracle: the design of record
`doc/2026-11-24T06-25-30Z_design_auto-update-system/` section 4 — one
daily, read-only census across the three estate planes; silence means
converged; anything held or drifted is filed on the tracking issue.

The `drift-census-daily` Muse cron schedule (registry:
`doc/fleet-schedule-registry.md`, Surface 1) runs this wrapper daily at
06:47 America/Chicago from a dedicated checkout. The wrapper runs the
census exactly as the slice's done check names it —
`python3 -I scripts/drift-census.py --format json` — folding in the one
observation this seat cannot make locally: the laptop's running
configuration revision, read over `fleet shell` (`nixos-version
--json`, a read-only probe) and handed to the census as a fixture
observation. A caller `--fixture` (e.g. the #14107 Cloudflare snapshot)
is merged into the same fixture.

Read-only rule: neither the census nor this wrapper ever fixes. The
wrapper's only writes are a temporary fixture file and, when the census
is not silent, the report comment itself. Exit 1 means the census run
failed; no report is posted on a guessed result.
"""

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
import tempfile
from datetime import datetime, timezone
from pathlib import Path

SCHEDULE_ID = "drift-census-daily"
CADENCE = "daily@06:47:00"
TIMEZONE = "America/Chicago"
CENSUS_COMMAND_LINE = "python3 -I scripts/drift-census.py --format json"
DEFAULT_ISSUE = 14134
DEFAULT_REPO = "developing-today/code"
DEFAULT_WINDOW_S = 24 * 60 * 60


def run_read(argv: list[str], timeout_s: int = 120) -> str | None:
    """One read-only subprocess. Returns stdout text or None on any failure."""
    try:
        proc = subprocess.run(argv, capture_output=True, text=True, timeout=timeout_s)
    except (OSError, subprocess.SubprocessError):
        return None
    if proc.returncode != 0:
        return None
    return proc.stdout


def fleet_binary() -> str:
    return shutil.which("fleet") or str(Path.home() / "tools" / "fleet")


def laptop_running_revision(runner) -> str | None:
    """The laptop's running configuration revision, read over fleet shell."""
    out = runner(
        [
            fleet_binary(),
            "shell",
            "--target",
            "laptop",
            "--",
            "nixos-version",
            "--json",
        ]
    )
    if not out:
        return None
    try:
        return json.loads(out).get("configurationRevision") or None
    except ValueError:
        return None


def build_fixture(extra: dict | None, laptop_revision: str | None) -> dict:
    """The census fixture: caller observations plus the laptop revision."""
    fixture = dict(extra or {})
    if laptop_revision:
        laptop = dict(fixture.get("laptop") or {})
        laptop.setdefault("running_revision", laptop_revision)
        fixture["laptop"] = laptop
    return fixture


def run_census(
    root: Path, fixture: dict, window_s: int, now: str | None, runner
) -> dict | None:
    """Run the census (CENSUS_COMMAND_LINE) and return its JSON summary."""
    argv = [
        "python3",
        "-I",
        str(root / "scripts" / "drift-census.py"),
        "--format",
        "json",
        "--root",
        str(root),
        "--window-s",
        str(window_s),
    ]
    if now:
        argv += ["--now", now]
    with tempfile.TemporaryDirectory() as td:
        if fixture:
            fixture_path = Path(td) / "fixture.json"
            fixture_path.write_text(json.dumps(fixture))
            argv += ["--fixture", str(fixture_path)]
        out = runner(argv)
    if not out:
        return None
    try:
        return json.loads(out)
    except ValueError:
        return None


def format_report(report: dict, day: str) -> str:
    """The concise issue report: counts, the non-converged rows, the verdict."""
    counts = report["counts"]
    lines = [
        f"## Drift census — daily report ({day})",
        "",
        f"{report['total']} rows: "
        + ", ".join(f"{state}={counts[state]}" for state in counts)
        + ".",
        "",
        "| plane | service | state | pinned | running | lag | note |",
        "| --- | --- | --- | --- | --- | --- | --- |",
    ]
    for row in report["rows"]:
        if row["state"] == "converged":
            continue
        lag = "?" if row["lag_s"] is None else f"{row['lag_s']}s"
        lines.append(
            f"| {row['plane']} | {row['service']} | {row['state']} "
            f"| {row['pinned']} | {row['running']} | {lag} | {row['reason']} |"
        )
    lines += [
        "",
        f"Needs you: {', '.join(report['needs_you'])}.",
        "",
        f"Read-only census (#14134 U1, schedule `{SCHEDULE_ID}`): the census "
        "never fixes — each item routes to its slice (U2/U3/U5/U6). "
        "Silence means converged; this comment is the report.",
    ]
    return "\n".join(lines)


def deliver(text: str, poster: str, issue: int, repo: str, runner) -> bool:
    """Deliver the report. Returns True when the report reached its surface."""
    if poster == "none":
        return False
    if poster == "stdout":
        print(text)
        return True
    out = runner(
        ["gh", "issue", "comment", str(issue), "--repo", repo, "--body", text]
    )
    return out is not None


def execute(
    root: Path,
    issue: int,
    repo: str,
    fixture_extra: dict | None,
    poster: str,
    now: str | None,
    window_s: int,
    runner,
) -> int:
    fixture = build_fixture(fixture_extra, laptop_running_revision(runner))
    report = run_census(root, fixture, window_s, now, runner)
    if report is None:
        print(
            "drift-census-report: census run failed; no report posted",
            file=sys.stderr,
        )
        return 1
    if report["silent"]:
        print("drift-census-report: converged — silent (no report posted)")
        return 0
    day = (now or datetime.now(timezone.utc).isoformat())[:10]
    text = format_report(report, day)
    if poster == "none":
        print(text)
        print("drift-census-report: poster=none — report printed, not posted")
        return 0
    if not deliver(text, poster, issue, repo, runner):
        print(
            "drift-census-report: delivery failed; report not posted",
            file=sys.stderr,
        )
        return 1
    print(f"drift-census-report: report posted on {repo}#{issue}")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description=(
            "Daily drift census report (#14134 U1 slice 2): run the read-only "
            "census and file its report on the tracking issue when not silent."
        ),
        allow_abbrev=False,
    )
    parser.add_argument(
        "--root",
        type=Path,
        default=Path(__file__).resolve().parents[2],
        help="repo checkout root (default: this file's repo)",
    )
    parser.add_argument("--issue", type=int, default=DEFAULT_ISSUE)
    parser.add_argument("--repo", default=DEFAULT_REPO)
    parser.add_argument(
        "--fixture",
        type=Path,
        default=None,
        help="JSON observations merged into the census fixture (e.g. #14107 snapshot)",
    )
    parser.add_argument(
        "--poster", choices=("gh", "stdout", "none"), default="gh"
    )
    parser.add_argument("--now", default=None, help="ISO-8601 instant (tests)")
    parser.add_argument("--window-s", type=int, default=DEFAULT_WINDOW_S)
    args = parser.parse_args(argv)

    fixture_extra = None
    if args.fixture is not None:
        if not args.fixture.is_file():
            print(
                f"drift-census-report: fixture not found: {args.fixture}",
                file=sys.stderr,
            )
            return 2
        fixture_extra = json.loads(args.fixture.read_text())

    return execute(
        args.root,
        args.issue,
        args.repo,
        fixture_extra,
        args.poster,
        args.now,
        args.window_s,
        run_read,
    )


if __name__ == "__main__":
    sys.exit(main())
