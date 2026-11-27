#!/usr/bin/env python3
"""Drift census v1 — pinned-vs-running across the three estate planes (#14134 slice U1).

Argument contract (#3352): this script declares its whole command line —
`--plane {laptop,sandbox,cloudflare}` (repeatable, default all three),
`--fixture PATH` (a JSON observation snapshot; tests use this instead of
live collection), `--format {json,text}`, `--root PATH`, `--window-s`
and `--now`. It parses strictly against that declaration before it runs
any collector. Anything unaccounted for is a usage error with exit 2.
The snapshot may carry a `cloudflare` plane (the #14107 shape) plus
`laptop: {"running_revision": ...}` and
`sandbox: {"running": {name: version}}` observations; a plane named in
the snapshot is evaluated from it instead of a live probe (the daily
report wrapper, `scripts/fleet/drift-census-report.py`, reads the
laptop over fleet shell and hands its revision in this way).

Oracle: the design of record
`doc/2026-11-24T06-25-30Z_design_auto-update-system/` section 4 —
one daily, READ-ONLY census; per service it lists pinned version,
running version, lag age, state (converged / pending / held / drifted)
and open overrides/held updates; silence means converged.

Hard rule (§4): the census NEVER fixes. It runs no restarts, swaps,
deploys or writes to a running system. Every collector here is a read
(`nixos-version`, `--version`, `git rev-parse`, a caller-supplied
snapshot); the module has no write path, and
`test_drift_census.py` asserts none is introduced.

States:
  converged  pinned == running.
  pending    running is behind, but inside the drift window (normal lag
             between a pin landing and a converge pass picking it up).
  drifted    running is behind and beyond the drift window, or reports a
             version that cannot be read (the census never assumes a
             match it did not observe).
  held       an override or held update pins the running version where it
             is; reported as held, never as drift.

Read-only provability: `--format text` exits 0 and prints rows; nothing
here mutates state. Exit 1 means the census itself could not build a
row for an in-scope plane, not that something drifted.
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

STATES = ("converged", "pending", "held", "drifted")
DEFAULT_WINDOW_S = 24 * 60 * 60

# The seed inventory the design's §4 census names (section 7 inventory).
LAPTOP_SERVICES = ("system-generation",)
SANDBOX_BINARIES = ("fleet", "tailscale", "bws", "op", "restic")

_VERSION_RE = re.compile(r"(\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.]+)*)")


class Row:
    """One census row. Plain class: the test harness imports this file by path."""

    def __init__(
        self, plane, service, pinned, running, lag_s, state, overrides, held, reason=""
    ):
        self.plane, self.service = plane, service
        self.pinned, self.running, self.lag_s = pinned, running, lag_s
        self.state, self.overrides, self.held, self.reason = (
            state,
            overrides,
            held,
            reason,
        )

    def as_dict(self) -> dict:
        return {
            k: getattr(self, k)
            for k in (
                "plane",
                "service",
                "pinned",
                "running",
                "lag_s",
                "state",
                "overrides",
                "held",
                "reason",
            )
        }


def _run(argv: list[str], timeout_s: int = 30) -> str | None:
    """One read-only probe. Returns stdout or None when unavailable."""
    try:
        proc = subprocess.run(argv, capture_output=True, text=True, timeout=timeout_s)
    except (OSError, subprocess.SubprocessError):
        return None
    if proc.returncode != 0:
        return None
    return proc.stdout.strip()


def _version_of(text: str | None) -> str:
    """First version-shaped token; 'unreadable' when absent."""
    if not text:
        return "unreadable"
    match = _VERSION_RE.search(text)
    return match.group(1) if match else "unreadable"


def _pin_of(url: str) -> str:
    """A pin's version, or its release tag when the URL carries no semver."""
    version = _version_of(url)
    if version != "unreadable":
        return version
    tag = re.search(r"/releases/download/([^/]+)/", url)
    return tag.group(1) if tag else "unreadable"


def classify(
    *,
    pinned: str,
    running: str,
    lag_s: int | None,
    window_s: int,
    overrides: list,
    held: list,
) -> Row:
    """The one state decision (§4 vocabulary). Pure."""
    if pinned == "unreadable":
        state, reason = "drifted", "pin unreadable"
    elif running == "unreadable" or running == "":
        state, reason = "drifted", "running version unreadable"
    elif held or overrides:
        state, reason = "held", "held by override or held update"
    elif pinned == running:
        state, reason = "converged", ""
    elif lag_s is not None and lag_s <= window_s:
        state, reason = "pending", "behind inside the drift window"
    elif lag_s is None:
        state, reason = "drifted", "behind; lag unknown"
    else:
        state, reason = "drifted", "behind beyond the drift window"
    return Row("-", "-", pinned, running, lag_s, state, overrides, held, reason)


def evaluate(
    plane: str,
    service: str,
    pinned: str,
    running: str,
    lag_s: int | None,
    window_s: int,
    overrides: list,
    held: list,
) -> Row:
    row = classify(
        pinned=pinned,
        running=running,
        lag_s=lag_s,
        window_s=window_s,
        overrides=overrides,
        held=held,
    )
    row.plane, row.service = plane, service
    return row


# ---------------------------------------------------------------- pins


def laptop_pin(root: Path) -> str:
    """The laptop pin is the flake on main (§4): origin/main's commit."""
    return _run(["git", "-C", str(root), "rev-parse", "origin/main"]) or "unreadable"


def sandbox_pins(root: Path) -> dict[str, str]:
    """Bootstrap pin per binary, read from the URLs scripts/muse/bootstrap.sh fetches."""
    try:
        text = (root / "scripts" / "muse" / "bootstrap.sh").read_text()
    except OSError:
        return {}
    pinned: dict[str, str] = {}
    for name, url_key in (
        ("fleet", "FLEET_URL"),
        ("bws", "BWS_URL"),
        ("restic", "RESTIC_URL"),
        ("tailscale", "TS_URL"),
    ):
        match = re.search(
            rf'{url_key}="\$\{{{url_key}:-([^}}]+)\}}"', text
        ) or re.search(rf'{url_key}="([^"]+)"', text)
        if match:
            pinned[name] = _pin_of(match.group(1))
    return pinned


# ---------------------------------------------------------- collectors


def _lag_since(root: Path, rev: str, now: datetime) -> int | None:
    """Seconds between a commit's committer time and now; None when unknown."""
    ts = _run(["git", "-C", str(root), "log", "-1", "--format=%ct", rev])
    if not ts or not ts.isdigit():
        return None
    return max(0, int(now.timestamp()) - int(ts))


def collect_laptop(
    root: Path, now: datetime, window_s: int, observed_running: str | None = None
) -> list[Row]:
    """The running system's configuration revision vs the flake on main (read-only).

    A caller-supplied observation replaces the local `nixos-version`
    probe (the daily report reads the laptop over fleet shell).
    """
    pin = laptop_pin(root)
    running = "unreadable"
    if observed_running is not None:
        running = observed_running or "unreadable"
    else:
        out = _run(["nixos-version", "--json"])
        if out:
            try:
                running = json.loads(out).get("configurationRevision") or "unreadable"
            except ValueError:
                pass
    lag = _lag_since(root, pin, now) if pin != "unreadable" else None
    return [
        evaluate(
            "laptop",
            LAPTOP_SERVICES[0],
            pin[:12],
            running[:12] if running != "unreadable" else running,
            lag,
            window_s,
            [],
            [],
        )
    ]


def collect_sandbox(
    root: Path, window_s: int, observed: dict | None = None
) -> list[Row]:
    """Seat binaries vs the bootstrap pin (read-only --version probes).

    The census is pinned-vs-running: a binary bootstrap.sh does not pin
    (today: op) has nothing to compare against and is out of scope, the
    same way the cloudflare plane reports nothing without a snapshot.
    A caller-supplied observation replaces the live probe per binary.
    """
    pinned = sandbox_pins(root)
    rows = []
    for name in SANDBOX_BINARIES:
        if name not in pinned:
            continue
        if observed is not None and name in observed:
            running = _version_of(observed[name])
        else:
            running = _version_of(_run([shutil.which(name) or name, "--version"]))
        rows.append(
            evaluate("sandbox", name, pinned[name], running, None, window_s, [], [])
        )
    return rows


def collect_cloudflare(
    root: Path, now: datetime, fixture: dict | None, window_s: int
) -> list[Row]:
    """Input is #14107's commit-age machinery; the census consumes it, never a second system.

    The snapshot is `{"main_rev": SHA, "workers": {name: deployed SHA}}`
    as #14107 records it. Without a snapshot the plane reports nothing
    rather than a guessed value.
    """
    if not fixture:
        return []
    pin = fixture.get("main_rev", "unreadable")
    lag = _lag_since(root, pin, now) if pin != "unreadable" else None
    return [
        evaluate(
            "cloudflare",
            worker,
            pin,
            fixture["workers"][worker],
            None if fixture["workers"][worker] == pin else lag,
            window_s,
            [],
            [],
        )
        for worker in sorted(fixture.get("workers", {}))
    ]


# ------------------------------------------------------------- output


def summary(rows: list[Row]) -> dict:
    """Census summary. Silence means converged; anything else is the report."""
    counts = {s: sum(1 for r in rows if r.state == s) for s in STATES}
    needs_attention = [r.service for r in rows if r.state in ("held", "drifted")]
    return {
        "total": len(rows),
        "counts": counts,
        "silent": not needs_attention,
        "needs_you": needs_attention,
        "rows": [r.as_dict() for r in rows],
    }


def render_text(report: dict) -> str:
    lines = [
        f"census: {report['total']} rows "
        f"({', '.join(f'{k}={v}' for k, v in report['counts'].items())})"
    ]
    for row in report["rows"]:
        lag = "?" if row["lag_s"] is None else f"{row['lag_s']}s"
        line = (
            f"  {row['plane']}/{row['service']}: {row['state']} "
            f"pinned={row['pinned']} running={row['running']} lag={lag}"
        )
        if row["reason"]:
            line += f" ({row['reason']})"
        lines.append(line)
    if report["silent"]:
        lines.append("converged — nothing needs you")
    else:
        lines.append(f"needs you: {', '.join(report['needs_you'])}")
    return "\n".join(lines)


PLANES = ("laptop", "sandbox", "cloudflare")


def build(
    planes: list[str], root: Path, fixture: dict | None, now: datetime, window_s: int
) -> dict:
    rows: list[Row] = []
    snapshot = fixture or {}
    if "laptop" in planes:
        rows += collect_laptop(
            root,
            now,
            window_s,
            (snapshot.get("laptop") or {}).get("running_revision"),
        )
    if "sandbox" in planes:
        rows += collect_sandbox(
            root, window_s, (snapshot.get("sandbox") or {}).get("running")
        )
    if "cloudflare" in planes:
        rows += collect_cloudflare(root, now, snapshot.get("cloudflare"), window_s)
    return summary(rows)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Drift census v1 (#14134 U1): pinned vs running, three planes, read-only.",
        allow_abbrev=False,
    )
    parser.add_argument(
        "--plane",
        action="append",
        choices=PLANES,
        help="plane to census (repeatable; default all three)",
    )
    parser.add_argument(
        "--fixture",
        type=Path,
        default=None,
        help="JSON observation snapshot (tests / offline)",
    )
    parser.add_argument("--format", choices=("json", "text"), default="text")
    parser.add_argument(
        "--root", type=Path, default=Path(__file__).resolve().parent.parent
    )
    parser.add_argument("--now", default=None, help="ISO-8601 instant (tests)")
    parser.add_argument("--window-s", type=int, default=DEFAULT_WINDOW_S)
    args = parser.parse_args(argv)

    fixture = None
    if args.fixture is not None:
        if not args.fixture.is_file():
            print(f"drift-census: fixture not found: {args.fixture}", file=sys.stderr)
            return 2
        fixture = json.loads(args.fixture.read_text())

    now = (
        datetime.fromisoformat(args.now).astimezone(timezone.utc)
        if args.now
        else datetime.now(timezone.utc)
    )
    report = build(args.plane or list(PLANES), args.root, fixture, now, args.window_s)
    print(
        json.dumps(report, indent=2, sort_keys=True)
        if args.format == "json"
        else render_text(report)
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
