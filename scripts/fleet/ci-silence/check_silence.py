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

# Every gh call is bounded (code#11468): a hung gh would otherwise hang
# the watchdog itself, which is the one outcome a silence probe cannot
# afford. A timeout is a probe error for that repo, never a verdict.
GH_TIMEOUT_S = 60.0

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


def evaluate(
    repo: str, runs: list[dict], now: datetime, threshold_hours: float
) -> dict:
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


def plan_actions(
    evaluations: list[dict], tracking: dict[str, int | None]
) -> list[dict]:
    """File-or-update plan: exactly one tracking issue per silent repo."""
    actions = []
    for ev in evaluations:
        if ev["status"] != "silent":
            actions.append({"action": "none", "repo": ev["repo"], "issue": None})
        elif tracking.get(ev["repo"]):
            actions.append(
                {"action": "update", "repo": ev["repo"], "issue": tracking[ev["repo"]]}
            )
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


def gh(args: list[str]) -> str:
    """Run one gh command, bounded by GH_TIMEOUT_S, and return stdout.

    Raises CalledProcessError, TimeoutExpired or OSError; callers turn
    those into a per-repo error record (see PROBE_ERRORS)."""
    return subprocess.run(
        ["gh"] + args,
        capture_output=True,
        text=True,
        check=True,
        timeout=GH_TIMEOUT_S,
    ).stdout


def gh_json(args: list[str]) -> list | dict:
    out = gh(args)
    return json.loads(out) if out.strip() else []


# What one repo's GitHub read or write can raise. Each is recorded on
# that repo and the pass carries on with the next one (code#11468).
PROBE_ERRORS = (
    subprocess.CalledProcessError,
    subprocess.TimeoutExpired,
    OSError,
    json.JSONDecodeError,
)


def describe_error(err: BaseException) -> str:
    if isinstance(err, subprocess.TimeoutExpired):
        return f"gh timed out after {err.timeout:g}s: {' '.join(err.cmd[:3])}"
    if isinstance(err, subprocess.CalledProcessError):
        detail = (err.stderr or "").strip().splitlines()
        tail = f": {detail[-1]}" if detail else ""
        return f"gh exited {err.returncode}: {' '.join(err.cmd[:3])}{tail}"
    if isinstance(err, json.JSONDecodeError):
        return f"gh returned unparseable JSON: {err.msg}"
    return f"{type(err).__name__}: {err}"


def error_evaluation(repo: str, threshold_hours: float, err: BaseException) -> dict:
    """A repo whose history could not be read: no verdict, an error."""
    return {
        "repo": repo,
        "status": "error",
        "age_hours": None,
        "threshold_hours": threshold_hours,
        "last_run": None,
        "reason": describe_error(err),
    }


def fetch_runs(repo: str) -> list[dict]:
    return gh_json(
        [
            "run",
            "list",
            "--repo",
            repo,
            "--status",
            "completed",
            "--limit",
            "30",
            "--json",
            "databaseId,name,conclusion,createdAt,updatedAt,url",
        ]
    )  # type: ignore[return-value]


def find_tracking_issue(repo: str) -> int | None:
    issues = gh_json(
        [
            "issue",
            "list",
            "--repo",
            repo,
            "--state",
            "open",
            "--search",
            f'"{TRACKING_TITLE_PREFIX}{repo}" in:title',
            "--json",
            "number,title",
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
            "issue",
            "create",
            "--repo",
            repo,
            "--title",
            tracking_title(repo),
            "--body",
            tracking_body(ev),
        ]
        for label in TRACKING_LABELS:
            args += ["--label", label]
        gh(args)
    elif action["action"] == "update":
        gh(
            [
                "issue",
                "comment",
                str(action["issue"]),
                "--repo",
                repo,
                "--body",
                f"Still silent at this pass: {tracking_body(ev)}",
            ]
        )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--apply",
        action="store_true",
        help="file/update tracking issues (default: observe only)",
    )
    parser.add_argument(
        "--threshold-hours",
        type=float,
        default=None,
        help="override every repo's silence threshold",
    )
    parser.add_argument(
        "--repo",
        action="append",
        dest="repos",
        help="check only this repo (repeatable)",
    )
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
            # One repo's failed or hung read is that repo's error; the
            # others are still evaluated (code#11468).
            try:
                runs = fetch_runs(repo)
                tracking[repo] = find_tracking_issue(repo)
            except PROBE_ERRORS as err:
                tracking[repo] = None
                evaluations.append(error_evaluation(repo, threshold, err))
                continue
        evaluations.append(evaluate(repo, runs, now, threshold))

    actions = plan_actions(evaluations, tracking)
    by_repo = {ev["repo"]: ev for ev in evaluations}
    apply_errors = []
    if args.apply and fixture is None:
        for action in actions:
            if action["action"] == "none":
                continue
            try:
                apply_action(action, by_repo[action["repo"]])
            except PROBE_ERRORS as err:
                apply_errors.append(
                    {
                        "repo": action["repo"],
                        "action": action["action"],
                        "reason": describe_error(err),
                    }
                )

    print(
        json.dumps(
            {
                "now": now.isoformat(),
                "applied": bool(args.apply and fixture is None),
                "evaluations": evaluations,
                "actions": actions,
                "apply_errors": apply_errors,
            },
            indent=2,
        )
    )
    # Silence is a finding, not a failure: a completed pass exits 0
    # whatever it found. A pass where any repo could not be read, or
    # any tracking write failed, exits 1 so a wired-in step cannot
    # read partial output as a clean pass.
    errored = apply_errors or any(ev["status"] == "error" for ev in evaluations)
    return 1 if errored else 0


if __name__ == "__main__":
    sys.exit(main())
