#!/usr/bin/env python3
"""Run-silence heartbeat for the four fleet repos (code#10917).

The gap this closes: the fleet's CI watch files on FAILED runs, not on
SILENCE. During the 2026-11-11 Actions blackout on
developing-today-forks/openchamber, a repo-level Actions disable meant
no runs were created at all for ~2 days — no failures to file on —
while every other API surface read healthy (workflows active,
permissions enabled:true). The fleet merged unverified work before a
human noticed.

This probe asserts on the one surface that cannot lie that way:
observable run history. For each fleet repo it reads the newest
COMPLETED Actions run of any kind (`gh run list --status completed`)
and measures its age. A repo whose newest completed run is older than
its threshold (default 6h, tuned to each repo's quietest normal gap)
gets exactly one tracking issue, filed in that repo and updated — never
duplicated — on subsequent silent passes.

The probe never consults workflow state or the actions/permissions
API as evidence of health: both lied during the blackout. Its only
GitHub read is run history; its only writes are the tracking issue
(create once, comment thereafter).

Default is observe-only (dry-run): the probe prints the actions it
would take and writes nothing. Pass `--apply` to file/update.

Usage:
    check_silence.py [--apply] [--threshold-hours H]
                     [--repo owner/name ...] [--now ISO8601]
                     [--fixture JSON]

`--fixture` replaces the GitHub reads with a JSON file shaped
{"runs": {"owner/repo": [run, ...]}, "tracking": {"owner/repo":
<issue number or null>}}, so the whole decision path runs offline in
tests. A run object carries the `gh run list` fields this probe uses:
databaseId, name, conclusion, createdAt, updatedAt, url.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from datetime import datetime, timezone

FLEET_REPOS = [
    "developing-today/code",
    "developing-today-forks/opencode",
    "developing-today-forks/openchamber",
    "dezren39/darwin-nix",
]

DEFAULT_THRESHOLD_HOURS = 6.0

# Per-repo thresholds (hours). A repo's threshold must exceed its
# quietest normal gap between completed runs; 6h is the fleet default
# from code#10917. Repos that legitimately run less often get a wider
# window here rather than training operators to ignore the alert.
REPO_THRESHOLDS_HOURS = {
    "developing-today/code": 6.0,
    "developing-today-forks/opencode": 6.0,
    "developing-today-forks/openchamber": 6.0,
    "dezren39/darwin-nix": 6.0,
}

TRACKING_TITLE_PREFIX = "CI run silence: "
TRACKING_LABELS = ["kind/bug", "source/ci-watch", "priority/high"]


def parse_ts(value: str) -> datetime:
    dt = datetime.fromisoformat(value.replace("Z", "+00:00"))
    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=timezone.utc)
    return dt


def newest_completed_run(runs: list[dict]) -> dict | None:
    """The newest completed run: latest updatedAt, run id breaks ties."""
    if not runs:
        return None
    return max(runs, key=lambda r: (parse_ts(r["updatedAt"]), r.get("databaseId", 0)))


def evaluate(repo: str, runs: list[dict], now: datetime, threshold_hours: float) -> dict:
    """One repo's silence verdict from its completed-run history only."""
    last = newest_completed_run(runs)
    if last is None:
        return {
            "repo": repo,
            "status": "silent",
            "age_hours": None,
            "threshold_hours": threshold_hours,
            "last_run": None,
            "reason": "no completed Actions run in the fetched history",
        }
    age_hours = (now - parse_ts(last["updatedAt"])).total_seconds() / 3600.0
    return {
        "repo": repo,
        "status": "silent" if age_hours > threshold_hours else "ok",
        "age_hours": round(age_hours, 3),
        "threshold_hours": threshold_hours,
        "last_run": {
            "databaseId": last.get("databaseId"),
            "name": last.get("name"),
            "conclusion": last.get("conclusion"),
            "updatedAt": last.get("updatedAt"),
            "url": last.get("url"),
        },
        "reason": None,
    }


def tracking_title(repo: str) -> str:
    return f"{TRACKING_TITLE_PREFIX}{repo} — no completed Actions run within threshold"


def plan_actions(evaluations: list[dict], tracking: dict[str, int | None]) -> list[dict]:
    """File-or-update plan: exactly one tracking issue per silent repo."""
    actions = []
    for ev in evaluations:
        if ev["status"] != "silent":
            actions.append({"action": "none", "repo": ev["repo"], "issue": None})
        elif tracking.get(ev["repo"]):
            actions.append({"action": "update", "repo": ev["repo"], "issue": tracking[ev["repo"]]})
        else:
            actions.append({"action": "file", "repo": ev["repo"], "issue": None})
    return actions


def tracking_body(ev: dict) -> str:
    last = ev["last_run"]
    if last is None:
        last_line = "Last completed run seen: none in the fetched run history."
    else:
        last_line = (
            f"Last completed run seen: [{last['name']} #{last['databaseId']}]"
            f"({last['url']}) — conclusion {last['conclusion']}, "
            f"completed {last['updatedAt']} ({ev['age_hours']}h ago at filing)."
        )
    return (
        f"The run-silence heartbeat (scripts/fleet/ci-silence, code#10917) "
        f"found no completed GitHub Actions run for `{ev['repo']}` within "
        f"its {ev['threshold_hours']}h threshold.\n\n{last_line}\n\n"
        "Silence is the failure mode of a repo-level Actions disable: no "
        "runs are created, nothing fails, and workflow/permissions API "
        "state still reads healthy (the 2026-11-11 openchamber blackout "
        "stayed invisible for ~2 days this way). This probe asserts on "
        "run history only.\n\n"
        "Filed by ci-silence heartbeat (fleet). One tracking issue per "
        "silent repo; subsequent silent passes update this issue instead "
        "of filing duplicates. Close it when runs resume; a later silence "
        "files a fresh issue."
    )


# --- GitHub access (gh CLI only; run history is the only read) ---

def gh_json(args: list[str]) -> list | dict:
    out = subprocess.run(
        ["gh"] + args, capture_output=True, text=True, check=True
    ).stdout
    return json.loads(out) if out.strip() else []


def fetch_runs(repo: str) -> list[dict]:
    return gh_json(
        [
            "run", "list", "--repo", repo, "--status", "completed",
            "--limit", "30", "--json",
            "databaseId,name,conclusion,createdAt,updatedAt,url",
        ]
    )  # type: ignore[return-value]


def find_tracking_issue(repo: str) -> int | None:
    issues = gh_json(
        [
            "issue", "list", "--repo", repo, "--state", "open",
            "--search", f'"{TRACKING_TITLE_PREFIX}{repo}" in:title',
            "--json", "number,title",
        ]
    )
    for issue in issues:  # type: ignore[union-attr]
        if issue["title"].startswith(f"{TRACKING_TITLE_PREFIX}{repo}"):
            return issue["number"]
    return None


def apply_action(action: dict, ev: dict) -> None:
    repo = action["repo"]
    if action["action"] == "file":
        args = [
            "issue", "create", "--repo", repo,
            "--title", tracking_title(repo),
            "--body", tracking_body(ev),
        ]
        for label in TRACKING_LABELS:
            args += ["--label", label]
        subprocess.run(["gh"] + args, capture_output=True, text=True, check=True)
    elif action["action"] == "update":
        subprocess.run(
            [
                "gh", "issue", "comment", str(action["issue"]),
                "--repo", repo, "--body",
                f"Still silent at this pass: {tracking_body(ev)}",
            ],
            capture_output=True, text=True, check=True,
        )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--apply", action="store_true",
                        help="file/update tracking issues (default: observe only)")
    parser.add_argument("--threshold-hours", type=float, default=None,
                        help="override every repo's silence threshold")
    parser.add_argument("--repo", action="append", dest="repos",
                        help="check only this repo (repeatable)")
    parser.add_argument("--now", help="ISO8601 'now' (tests/dry-runs)")
    parser.add_argument("--fixture", help="offline fixture JSON (see module docstring)")
    args = parser.parse_args(argv)

    now = parse_ts(args.now) if args.now else datetime.now(timezone.utc)
    repos = args.repos or FLEET_REPOS

    fixture = None
    if args.fixture:
        with open(args.fixture) as fh:
            fixture = json.load(fh)

    evaluations, tracking = [], {}
    for repo in repos:
        threshold = (
            args.threshold_hours
            if args.threshold_hours is not None
            else REPO_THRESHOLDS_HOURS.get(repo, DEFAULT_THRESHOLD_HOURS)
        )
        if fixture is not None:
            runs = fixture.get("runs", {}).get(repo, [])
            tracking[repo] = fixture.get("tracking", {}).get(repo)
        else:
            runs = fetch_runs(repo)
            tracking[repo] = find_tracking_issue(repo)
        evaluations.append(evaluate(repo, runs, now, threshold))

    actions = plan_actions(evaluations, tracking)
    by_repo = {ev["repo"]: ev for ev in evaluations}
    if args.apply and fixture is None:
        for action in actions:
            if action["action"] != "none":
                apply_action(action, by_repo[action["repo"]])

    print(json.dumps({
        "now": now.isoformat(),
        "applied": bool(args.apply and fixture is None),
        "evaluations": evaluations,
        "actions": actions,
    }, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
