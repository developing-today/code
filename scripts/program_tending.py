#!/usr/bin/env python3
"""program-tending scaffold (issue #1352, slice 1/3).

Daily scheduled care for the planning layer across the four fleet repos
(code, opencode, openchamber, nix). This slice is the scaffold only: the
runner entrypoint, per-section stubs that verify and report no-actions,
and the structured run log. Full section implementations (epics +
roadmaps in slice 2, projects + stalled-PRs + loose-ends in slice 3)
land after this scaffold does — see the triage decomposition on #1352.

Ownership rules (from #1352; enforced by every section stub):
- never touch held / do-not-merge / user-owned PRs;
- Drew's own feature PRs get a check-in comment at 7 days, never a packet;
- user/review is reported, never re-decided.

Usage: python3 -I scripts/program_tending.py --run [--repo owner/name ...]
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import sys
import urllib.request

SECTIONS = ("epics", "roadmaps", "projects", "stalled-prs", "loose-ends")

FLEET_REPOS = (
    "developing-today/code",
    "developing-today-forks/opencode",
    "developing-today-forks/openchamber",
    "developing-today/nix",
)

# Labels the stubs must never act on (read-only signals).
HARD_STOPS = ("held", "user/owned", "do-not-merge", "do-not-auto-edit")

# The fleet monitor tends issues and queue state; this job reports the
# planning layer and never changes monitor or stewardship-cycle behavior.
OUT_OF_SCOPE = ("fleet monitor behavior", "stewardship-cycle behavior")


def _utcnow() -> str:
    return dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _gh_api(path: str) -> list:
    """GET a GitHub REST collection, following pagination. Read-only."""
    items: list = []
    url = f"https://api.github.com{path}"
    headers = {"Accept": "application/vnd.github+json"}
    # The workflow hands this job a token (#6860): send it when present
    # so the reads are authenticated; without one, behave exactly as
    # before. The value is only ever placed in the header — it is
    # never logged or included in any error or report string.
    token = os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN")
    if token:
        headers["Authorization"] = f"Bearer {token}"
    while url:
        req = urllib.request.Request(url, headers=headers)
        with urllib.request.urlopen(req, timeout=30) as resp:
            items.extend(json.loads(resp.read().decode("utf-8")))
            nxt = resp.headers.get("Link", "")
        url = ""
        for part in nxt.split(","):
            if 'rel="next"' in part:
                url = part.split(";")[0].strip().strip("<>")
    return items


def _labels(item: dict) -> set[str]:
    return {label["name"] for label in item.get("labels", [])}


def stub_epics(repos: list[str]) -> dict:
    """Slice 2 owns this section. Slice 1 verifies reachability only."""
    checked = [{"repo": repo, "open_epics": "unverified (slice 2)"} for repo in repos]
    return {
        "section": "epics",
        "status": "no-action",
        "detail": "scaffold only: per-epic child-progress checks land in slice 2",
        "checked": checked,
    }


def stub_roadmaps(repos: list[str]) -> dict:
    """Slice 2 owns this section. Slice 1 verifies reachability only."""
    return {
        "section": "roadmaps",
        "status": "no-action",
        "detail": "scaffold only: ROADMAP.md vs landed-work drift checks land in slice 2",
        "checked": [{"repo": repo, "drift": "unverified (slice 2)"} for repo in repos],
    }


def stub_projects(repos: list[str]) -> dict:
    """Slice 3 owns this section. Slice 1 verifies reachability only."""
    return {
        "section": "projects",
        "status": "no-action",
        "detail": "scaffold only: board-vs-reality correction checks land in slice 3",
        "checked": [
            {"repo": repo, "board_state": "unverified (slice 3)"} for repo in repos
        ],
    }


def stub_stalled_prs(repos: list[str]) -> dict:
    """Slice 3 owns this section. Slice 1 counts, but never comments."""
    counts = []
    for repo in repos:
        try:
            prs = _gh_api(f"/repos/{repo}/pulls?state=open&per_page=100")
            counts.append(
                {
                    "repo": repo,
                    "open_prs": len(prs),
                    "note": "48h-rule evaluation lands in slice 3",
                }
            )
        except Exception as exc:  # noqa: BLE001 - scaffold reports, never fails
            counts.append({"repo": repo, "open_prs": "unverified", "error": str(exc)})
    return {
        "section": "stalled-prs",
        "status": "no-action",
        "detail": "scaffold only: 48h check-ins and pr_shepherd packets land in slice 3; no comments posted",
        "checked": counts,
    }


def stub_loose_ends(repos: list[str]) -> dict:
    """Slice 3 owns this section. Slice 1 verifies reachability only."""
    return {
        "section": "loose-ends",
        "status": "no-action",
        "detail": "scaffold only: notifications, duplicate cross-links, user/review surfacing land in slice 3",
        "checked": [
            {"repo": repo, "loose_ends": "unverified (slice 3)"} for repo in repos
        ],
    }


def run(repos: list[str]) -> dict:
    log = {
        "job": "program-tending",
        "slice": 1,
        "issue": "developing-today/code#1352",
        "run_at": _utcnow(),
        "repos": repos,
        "hard_stops": list(HARD_STOPS),
        "out_of_scope": list(OUT_OF_SCOPE),
        "sections": [
            stub_epics(repos),
            stub_roadmaps(repos),
            stub_projects(repos),
            stub_stalled_prs(repos),
            stub_loose_ends(repos),
        ],
    }
    log["actions_taken"] = sum(1 for s in log["sections"] if s["status"] != "no-action")
    return log


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(
        description="program-tending scaffold runner (slice 1/3)"
    )
    parser.add_argument(
        "--run", action="store_true", help="execute one scaffold run and print the log"
    )
    parser.add_argument(
        "--repo", action="append", default=[], help="limit to owner/name (repeatable)"
    )
    args = parser.parse_args(argv)
    if not args.run:
        parser.print_help()
        return 2
    repos = args.repo or list(FLEET_REPOS)
    print(json.dumps(run(repos), indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
