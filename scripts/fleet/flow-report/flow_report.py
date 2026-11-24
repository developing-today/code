#!/usr/bin/env python3
"""flow-report: where the fleet's output goes, measured from GitHub and git.

The scoreboard behind planning/plans/2026-11-21_results-first.md. It
answers one question with numbers a second person can reproduce: of
everything the fleet opens, comments on and merges, how much becomes
finished `id` capability, and how much is motion.

Two subcommands, both read-only against the repository:

  fetch   Page every issue and PR of one repo through `gh api` into a
          JSONL file (one compact row per item; no bodies, no comments).
  report  Read that JSONL, and optionally the first-parent history of a
          git checkout, and print the scoreboard as markdown or JSON.

Argument contract (#3352): the whole command line is declared below and
parsed strictly by argparse before any subprocess, network call or file
write. An unknown flag, a stray positional or a valueless option is a
usage error with exit 2. `--opt=value` and `--opt value` are the same.

Exit codes: 0 report or fetch completed; 2 usage error; 3 a `gh` or
`git` call failed (a failure never prints a partial scoreboard).

Every number this prints is a count over the input files. Nothing is
estimated, and a section whose input was not given says so instead of
printing zeros.
"""

from __future__ import annotations

import argparse
import collections
import datetime as dt
import json
import re
import statistics
import subprocess
import sys
from pathlib import Path

ROW_JQ = (
    ".[] | {n:.number, t:.title, s:.state, sr:.state_reason, "
    "pr:(.pull_request!=null), merged:(.pull_request.merged_at // null), "
    "d:(.draft // false), "
    "c:.created_at, x:.closed_at, u:.updated_at, l:[.labels[].name], "
    "a:.user.login, cm:.comments} | @json"
)

# Mainline commits are classified by the files they change. A commit's
# area is the area holding most of its changed files; ties go to the
# earlier row. Order matters: the first matching prefix wins.
AREAS: list[tuple[str, tuple[str, ...]]] = [
    ("id", ("pkgs/id/",)),
    ("mcpx", ("pkgs/mcpx/",)),
    ("atlas", ("pkgs/atlas",)),
    (
        "fleet-tooling",
        (
            "pm-stack/",
            "pkgs/fleet",
            "pkgs/pm-",
            "pkgs/gh-fleet/",
            "pkgs/lane-overlap-check/",
            "pkgs/queue-transport-alert/",
            "pkgs/laptop-agent/",
            "pkgs/lab-ops-hub/",
            "pkgs/agent-",
            "pkgs/cold-store-reaper/",
            "pkgs/opencode-config-normalize/",
            "pkgs/message-bridge/",
            "pkgs/dot-bridge/",
            "pkgs/github-ci/",
            "pkgs/deploy/",
            "scripts/fleet/",
            ".opencode/",
            ".agents/",
            "bin/",
            "home/openclaw/",
            "infra/github-backup/",
            "doc/agents/",
            "doc/decisions/",
            "doc/fleet",
            "doc/living/",
        ),
    ),
    ("ci", (".github/",)),
    ("docs-planning", ("doc/", "planning/", "ROADMAP.md", "AGENTS.md", "CLAUDE.md")),
    (
        "nixos-infra",
        ("nixos/", "home/", "lib/", "infra/", "flake.", "secrets/", "sops/", "nix/"),
    ),
]

ID_LABELS = frozenset(
    {
        "app/id",
        "epic/id-program",
        "epic/id-platform",
        "design/id-phase-14",
        "epic/id-crate-split",
    }
)
STALLED_LABELS = frozenset(
    {"status/blocked", "status/parked", "status/waiting-dependency"}
)
EFFORTS = ("effort/xs", "effort/s", "effort/m", "effort/l")
MAIN_BROKEN_TITLE = re.compile(r"^(main-broken|main CI failure)", re.IGNORECASE)
# "main-broken: owner/repo <workflow words> @sha": the first workflow word
# names the component (id, mcpx, nix).
MAIN_BROKEN_COMPONENT = re.compile(r"^main-broken: \S+/\S+ ([A-Za-z][\w.-]*)")
SELF_MIRROR = "[{repo}#"
CONVENTIONAL = re.compile(r"^\s*(\w+)(\([^)]*\))?!?:")
PR_REF = re.compile(r"^Merge pull request #(\d+)|\(#(\d+)\)\s*$")


class ToolError(Exception):
    """A gh or git call failed; the run reports it and exits 3."""


def parse_time(value: str) -> dt.datetime:
    return dt.datetime.fromisoformat(value.replace("Z", "+00:00"))


def iso_week(value: str) -> str:
    year, week, _ = parse_time(value).isocalendar()
    return f"{year}-W{week:02d}"


def percent(part: int, whole: int) -> float | None:
    return round(100 * part / whole, 1) if whole else None


def load_rows(path: Path) -> list[dict]:
    rows = []
    try:
        handle = path.open(encoding="utf-8")
    except OSError as error:
        raise ToolError(f"cannot read {path}: {error.strerror}") from error
    with handle:
        for number, line in enumerate(handle, 1):
            line = line.strip()
            if not line:
                continue
            try:
                rows.append(json.loads(line))
            except json.JSONDecodeError as error:
                raise ToolError(f"{path}:{number}: not JSON ({error})") from error
    if not rows:
        raise ToolError(f"{path}: no rows; refusing to print an all-zero scoreboard")
    return rows


def area_of(path: str) -> str:
    for name, prefixes in AREAS:
        if path.startswith(prefixes):
            return name
    return "other"


def read_mainline(git_dir: Path, rev: str, since: str | None) -> list[dict]:
    """First-parent commits of `rev` with the files each one changed.

    `-m --first-parent` diffs a merge against its first parent, so a
    merged PR shows the files that PR brought in. Needs trees only, so
    a blob-less clone (`--filter=blob:none`) is enough.
    """
    command = [
        "git",
        "-C",
        str(git_dir),
        "log",
        "--first-parent",
        "-m",
        "--name-status",
        "--no-renames",
        "--format=@@%H|%aI|%s",
    ]
    if since:
        command.append(f"--since={since}")
    command.append(rev)
    result = subprocess.run(
        command, capture_output=True, text=True, errors="replace", check=False
    )
    if result.returncode != 0:
        raise ToolError(f"git log failed: {result.stderr.strip()[:300]}")
    commits: list[dict] = []
    for line in result.stdout.splitlines():
        if line.startswith("@@"):
            sha, when, subject = line[2:].split("|", 2)
            commits.append({"sha": sha, "when": when, "subject": subject, "files": []})
        elif line and commits:
            parts = line.split("\t")
            if len(parts) >= 2:
                commits[-1]["files"].append(parts[-1])
    return commits


def mainline_section(commits: list[dict], titles: dict[int, str]) -> dict:
    primary: collections.Counter[str] = collections.Counter()
    id_types: collections.Counter[str] = collections.Counter()
    id_touching = 0
    counted = 0
    for commit in commits:
        if not commit["files"]:
            continue
        counted += 1
        areas = collections.Counter(area_of(path) for path in commit["files"])
        order = [name for name, _ in AREAS] + ["other"]
        top = max(areas, key=lambda name: (areas[name], -order.index(name)))
        primary[top] += 1
        if areas.get("id"):
            id_touching += 1
            title = commit["subject"]
            match = PR_REF.search(title)
            if match:
                number = int(match.group(1) or match.group(2))
                title = titles.get(number, title)
            kind = CONVENTIONAL.match(title)
            id_types[kind.group(1).lower() if kind else "(untyped)"] += 1
    return {
        "commits": counted,
        "first": commits[-1]["when"][:10] if commits else None,
        "last": commits[0]["when"][:10] if commits else None,
        "primary_area": {
            name: {"commits": count, "pct": percent(count, counted)}
            for name, count in primary.most_common()
        },
        "id_touching": id_touching,
        "id_touching_types": dict(id_types.most_common()),
        "id_fix_pct": percent(id_types.get("fix", 0), id_touching),
        "id_feat_pct": percent(id_types.get("feat", 0), id_touching),
    }


def issue_section(rows: list[dict], repo: str, now: dt.datetime) -> dict:
    issues = [row for row in rows if not row["pr"]]
    pulls = [row for row in rows if row["pr"]]
    open_issues = [row for row in issues if row["s"] == "open"]

    weekly: dict[str, dict[str, int]] = collections.defaultdict(
        lambda: {"issues_opened": 0, "issues_closed": 0, "prs_merged": 0}
    )
    for row in issues:
        weekly[iso_week(row["c"])]["issues_opened"] += 1
        if row["x"]:
            weekly[iso_week(row["x"])]["issues_closed"] += 1
    for row in pulls:
        if row["merged"]:
            weekly[iso_week(row["merged"])]["prs_merged"] += 1

    effort = {}
    for label in EFFORTS:
        tagged = [row for row in issues if label in row["l"]]
        still_open = sum(row["s"] == "open" for row in tagged)
        effort[label] = {
            "issues": len(tagged),
            "open": still_open,
            "closed_pct": percent(len(tagged) - still_open, len(tagged)),
        }

    stalled = sum(1 for row in open_issues if STALLED_LABELS & set(row["l"]))
    id_open = [row for row in open_issues if ID_LABELS & set(row["l"])]
    id_stalled = sum(1 for row in id_open if STALLED_LABELS & set(row["l"]))

    main_broken = [
        row
        for row in issues
        if "main-broken" in row["l"] or MAIN_BROKEN_TITLE.match(row["t"])
    ]
    components: collections.Counter[str] = collections.Counter()
    id_last_week = 0
    week_ago = now - dt.timedelta(days=7)
    for row in main_broken:
        match = MAIN_BROKEN_COMPONENT.search(row["t"])
        name = match.group(1) if match else "(other title shape)"
        components[name] += 1
        if (name == "id" or name.startswith("id-")) and parse_time(
            row["c"]
        ) >= week_ago:
            id_last_week += 1

    # #12280's soak metric: ready (non-draft) PRs still open a day
    # after they were opened. Rows fetched before `d` was recorded
    # read as ready (the fetch defaults draft to false), matching how
    # the warden's scan treats an unmarked PR.
    day_ago = now - dt.timedelta(hours=24)
    prs_open_ready_over_24h = sum(
        1
        for row in pulls
        if row["s"] == "open"
        and not row.get("d", False)
        and parse_time(row["c"]) <= day_ago
    )

    comments = sorted(row["cm"] for row in issues)
    merge_minutes = sorted(
        (parse_time(row["merged"]) - parse_time(row["c"])).total_seconds() / 60
        for row in pulls
        if row["merged"]
    )
    origins = collections.Counter()
    for row in issues:
        sources = [label for label in row["l"] if label.startswith("source/")]
        if "source/human" in sources:
            origins["source/human"] += 1
        elif sources:
            origins["fleet-generated (any other source/*)"] += 1
        else:
            origins["no source label"] += 1

    return {
        "as_of": now.isoformat(timespec="seconds"),
        "issues": len(issues),
        "issues_open": len(open_issues),
        "prs": len(pulls),
        "prs_merged": sum(bool(row["merged"]) for row in pulls),
        "prs_closed_unmerged": sum(
            row["s"] == "closed" and not row["merged"] for row in pulls
        ),
        "prs_open_ready_over_24h": prs_open_ready_over_24h,
        "weekly": dict(sorted(weekly.items())),
        "effort": effort,
        "open_stalled": stalled,
        "open_stalled_pct": percent(stalled, len(open_issues)),
        "open_user_escalated": sum("user/escalated" in row["l"] for row in open_issues),
        "open_design": sum("kind/design" in row["l"] for row in open_issues),
        "id_open": len(id_open),
        "id_open_stalled": id_stalled,
        "id_open_stalled_pct": percent(id_stalled, len(id_open)),
        "main_broken": len(main_broken),
        "main_broken_components": dict(components.most_common()),
        "main_broken_id_last_7_days": id_last_week,
        "comments_total": sum(comments),
        "comments_median": statistics.median(comments) if comments else None,
        "open_issues_20plus_comments": sum(row["cm"] >= 20 for row in open_issues),
        "self_mirror_issues": sum(
            row["t"].startswith(SELF_MIRROR.format(repo=repo)) for row in issues
        ),
        "issue_origin": dict(origins.most_common()),
        "pr_minutes_to_merge_median": (
            round(statistics.median(merge_minutes)) if merge_minutes else None
        ),
        "pr_merged_within_10_min_pct": percent(
            sum(minutes < 10 for minutes in merge_minutes), len(merge_minutes)
        ),
    }


def render_markdown(report: dict) -> str:
    issues = report["issues"]
    out = [
        f"# Flow report: {report['repo']}",
        "",
        f"Age calculations use {issues['as_of']}. "
        f"Issue collection started: {report['inputs']['issues_fetch_started_at'] or 'unknown'}. "
        f"Source: `{report['inputs']['issues']}`"
        + (
            f", git `{report['inputs']['git']}`"
            if report["inputs"].get("git")
            else " (no git history given)"
        )
        + ".",
        "",
        "## Totals",
        "",
        "| Measure | Value |",
        "| --- | ---: |",
        f"| Issues (open) | {issues['issues']} ({issues['issues_open']}) |",
        f"| PRs merged / closed unmerged | {issues['prs_merged']} / {issues['prs_closed_unmerged']} |",
        f"| Open ready PRs older than 24 hours | {issues['prs_open_ready_over_24h']} |",
        f"| Open issues blocked, parked or waiting | {issues['open_stalled']} ({issues['open_stalled_pct']}%) |",
        f"| Open issues labelled `user/escalated` | {issues['open_user_escalated']} |",
        f"| Open `kind/design` issues | {issues['open_design']} |",
        f"| Open `id` issues (stalled) | {issues['id_open']} ({issues['id_open_stalled']}, {issues['id_open_stalled_pct']}%) |",
        f"| main-broken issues | {issues['main_broken']} |",
        f"| main-broken issues for `id`, last 7 days | {issues['main_broken_id_last_7_days']} |",
        f"| Issue comments (median per issue) | {issues['comments_total']} ({issues['comments_median']}) |",
        f"| Open issues with 20+ comments | {issues['open_issues_20plus_comments']} |",
        f"| PR median minutes to merge (merged < 10 min) | {issues['pr_minutes_to_merge_median']} ({issues['pr_merged_within_10_min_pct']}%) |",
        "",
        "## Closure by effort label",
        "",
        "| Effort | Issues | Open | Closed |",
        "| --- | ---: | ---: | ---: |",
    ]
    for label, row in issues["effort"].items():
        out.append(
            f"| `{label}` | {row['issues']} | {row['open']} | {row['closed_pct']}% |"
        )
    out += [
        "",
        "## Weekly flow",
        "",
        "| Week | Issues opened | Issues closed | PRs merged |",
        "| --- | ---: | ---: | ---: |",
    ]
    for week, row in list(issues["weekly"].items())[-8:]:
        out.append(
            f"| {week} | {row['issues_opened']} | {row['issues_closed']} | {row['prs_merged']} |"
        )
    out += [
        "",
        "## main-broken by component",
        "",
        "| Component | Issues |",
        "| --- | ---: |",
    ]
    for name, count in issues["main_broken_components"].items():
        out.append(f"| {name} | {count} |")
    mainline = report.get("mainline")
    out += ["", "## Mainline commits by area", ""]
    if not mainline:
        out.append("Not measured: no `--git-dir` given.")
    else:
        out += [
            f"{mainline['commits']} first-parent commits, {mainline['first']} to {mainline['last']}.",
            "",
            "| Area | Commits | Share |",
            "| --- | ---: | ---: |",
        ]
        for name, row in mainline["primary_area"].items():
            out.append(f"| {name} | {row['commits']} | {row['pct']}% |")
        out += [
            "",
            (
                f"Commits touching `pkgs/id`: {mainline['id_touching']}. "
                f"`fix`: {mainline['id_fix_pct']}%, `feat`: {mainline['id_feat_pct']}% "
                "(by PR title prefix)."
            ),
        ]
    return "\n".join(out) + "\n"


def issue_fetch_started_at(rows: list[dict]) -> str | None:
    # Legacy and combined files do not have a single collection clock.
    stamp = rows[0].get("fetch_started_at")
    if not isinstance(stamp, str) or any(row.get("fetch_started_at") != stamp for row in rows):
        return None
    try:
        parsed = parse_time(stamp)
    except ValueError:
        return None
    return parsed.isoformat() if parsed.tzinfo is not None else None


def cmd_fetch(args: argparse.Namespace) -> int:
    endpoint = (
        f"repos/{args.repo}/issues?state=all&per_page=100&sort=created&direction=asc"
    )
    started = dt.datetime.now(dt.timezone.utc).isoformat(timespec="microseconds")
    command = ["gh", "api", "--paginate", endpoint, "--jq", ROW_JQ]
    try:
        result = subprocess.run(command, capture_output=True, text=True, check=False)
    except FileNotFoundError as error:
        raise ToolError("gh is not installed") from error
    if result.returncode != 0:
        raise ToolError(f"gh api failed: {result.stderr.strip()[:300]}")
    if not result.stdout.strip():
        raise ToolError(f"gh api returned no rows for {args.repo}")
    lines = []
    for number, line in enumerate(result.stdout.splitlines(), start=1):
        try:
            row = json.loads(line)
            row["fetch_started_at"] = started
        except (ValueError, TypeError) as error:
            raise ToolError(f"gh api returned an invalid issue row on line {number}") from error
        lines.append(json.dumps(row, ensure_ascii=False, separators=(",", ":")))
    args.out.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"{args.out}: {len(lines)} rows from {args.repo}")
    return 0


def cmd_report(args: argparse.Namespace) -> int:
    rows = load_rows(args.issues)
    now = parse_time(args.now) if args.now else dt.datetime.now(dt.timezone.utc)
    report = {
        "repo": args.repo,
        "inputs": {
            "issues": str(args.issues),
            "issues_fetch_started_at": issue_fetch_started_at(rows),
            "git": None,
        },
        "issues": issue_section(rows, args.repo, now),
    }
    if args.git_dir:
        titles = {row["n"]: row["t"] for row in rows if row["pr"]}
        commits = read_mainline(args.git_dir, args.rev, args.since)
        report["inputs"]["git"] = f"{args.git_dir} {args.rev}" + (
            f" since {args.since}" if args.since else ""
        )
        report["mainline"] = mainline_section(commits, titles)
    if args.format == "json":
        print(json.dumps(report, indent=2))
    else:
        print(render_markdown(report), end="")
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="flow_report.py",
        description="Measure where the fleet's output goes (read-only).",
        allow_abbrev=False,
    )
    commands = parser.add_subparsers(dest="command", required=True)

    fetch = commands.add_parser(
        "fetch", allow_abbrev=False, help="page issues and PRs into a JSONL file"
    )
    fetch.add_argument("--repo", required=True, help="owner/name")
    fetch.add_argument("--out", required=True, type=Path, help="JSONL file to write")
    fetch.set_defaults(run=cmd_fetch)

    report = commands.add_parser(
        "report", allow_abbrev=False, help="print the scoreboard"
    )
    report.add_argument("--repo", required=True, help="owner/name the JSONL came from")
    report.add_argument("--issues", required=True, type=Path, help="JSONL from fetch")
    report.add_argument("--git-dir", type=Path, help="checkout or bare repo to read")
    report.add_argument(
        "--rev", default="HEAD", help="mainline revision (default HEAD)"
    )
    report.add_argument("--since", help="only commits after this date (git --since)")
    report.add_argument(
        "--now", help="reference time for age calculations (ISO time, default: now)"
    )
    report.add_argument("--format", choices=("md", "json"), default="md")
    report.set_defaults(run=cmd_report)
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        return args.run(args)
    except ToolError as error:
        print(f"flow-report: {error}", file=sys.stderr)
        return 3


if __name__ == "__main__":
    sys.exit(main())
