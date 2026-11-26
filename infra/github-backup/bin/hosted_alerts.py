#!/usr/bin/env python3
"""Deduplicated GitHub alerting for the hosted freshness report (#15001).

Slice 1 of #15001 (PR #15130) produces the hosted five-leg freshness
report and deliberately files nothing. This module is the alerting
slice: it consumes that report and plans — and, with --apply, executes
— the GitHub issue actions that keep exactly one open alert per
unhealthy (leg, repo) pair.

Deduplication key: every alert issue this module creates carries a
machine-readable marker in its body,

    <!-- hosted-freshness-alert: leg=<leg> repo=<repo> -->

and the planner matches existing open issues by that marker, never by
title text. The plan is a pure function of (report, existing alerts):

- create:  a (leg, repo) is not fresh and no open alert carries its key.
- update:  an open alert carries the key, the pair is still not fresh,
           and the evidence changed (status, detail, or run identity).
           Re-planning against the same run is a noop, so a repeated
           probe run never spams comments onto the same alert.
- resolve: an open alert carries the key and the pair is fresh again —
           successful re-verification closes the alert with the new
           run identity as evidence.
- noop:    fresh with no open alert, or an update whose evidence is
           identical to what the alert already records.

Alert bodies record the classification (stale / missing / partial /
failed are never collapsed into one wording), the probe detail, the
leg SLO, and the report's run identity and generation time, so an
alert always carries its evidence. The `source` leg defines truth and
is not alerted on its own: a failed source check already fails every
copy leg judged against it. The d1_export leg is inventory-global in
the report (no per-repo entries); it alerts under repo "-".

Without --apply the module is a dry run: it prints the planned actions
as JSON and touches nothing. With --apply it executes the plan through
`gh api` (create / comment+patch / close), with the executor injectable
for tests. Credentials arrive via gh's own environment only; this
module never reads credential files and never prints a credential.

Command line: --report FILE [--existing FILE] [--repo OWNER/NAME]
[--apply]. Arguments are parsed strictly before any file or network
access; anything unaccounted for is a usage error (exit 2, #3352).
Exit 0 when the plan contains no create/update (nothing unhealthy is
unalerted), 1 when it does, 2 on usage error.
"""

import argparse
import json
import re
import subprocess
import sys

ALERT_ID = "hosted-freshness-alert:v1"
MARKER_RE = re.compile(
    r"<!-- hosted-freshness-alert: leg=(?P<leg>\S+) repo=(?P<repo>\S+) -->"
)
RUN_RE = re.compile(r"run_id: (?P<run>\S+)")
NON_FRESH = ("stale", "missing", "partial", "failed")
GLOBAL_REPO = "-"


def marker(leg, repo):
    return f"<!-- hosted-freshness-alert: leg={leg} repo={repo} -->"


def alert_title(leg, repo, status):
    subject = leg if repo == GLOBAL_REPO else f"{leg} for {repo}"
    return f"[backup-freshness] {subject} is {status} (#15001)"


def alert_body(report, leg, repo, entry):
    lines = [
        marker(leg, repo),
        "",
        f"The hosted freshness probe reports **{entry['status']}** for "
        f"the `{leg}` leg" + ("" if repo == GLOBAL_REPO else f" of `{repo}`") + ".",
        "",
        f"- status: {entry['status']}",
        f"- detail: {entry.get('detail', '')}",
    ]
    slo = report["legs"][leg].get("slo_hours")
    if slo is not None:
        lines.append(f"- slo_hours: {slo}")
    for key in ("copy_head", "bundle_at", "exported_at"):
        if key in entry:
            lines.append(f"- {key}: {entry[key]}")
    lines += [
        f"- run_id: {report['run_id']}",
        f"- generated_at: {report['generated_at']}",
        f"- probe: {report['probe']}",
        "",
        "This alert is maintained by hosted_alerts.py (#15001): it is "
        "updated in place while the leg stays unhealthy and closed "
        "automatically when a later probe run verifies the copy fresh.",
    ]
    return "\n".join(lines) + "\n"


def unhealthy_entries(report):
    """Yield (leg, repo, entry) for every non-fresh classification.

    Per-repo entries come from each leg's `repos` map; a leg with no
    per-repo entries (d1_export) yields one global entry built from
    its leg-level status/detail. Fresh entries are yielded too, under
    status fresh, so the planner can resolve open alerts — callers
    distinguish by entry["status"].
    """
    for leg, leg_report in sorted(report["legs"].items()):
        if leg == "source":
            continue
        repos = leg_report.get("repos") or {}
        if repos:
            for repo, entry in sorted(repos.items()):
                yield leg, repo, entry
        else:
            yield (
                leg,
                GLOBAL_REPO,
                {
                    "status": leg_report["status"],
                    "detail": leg_report.get("detail", ""),
                    **{k: leg_report[k] for k in ("exported_at",) if k in leg_report},
                },
            )


def parse_existing(issues):
    """Map dedup key (leg, repo) -> issue dict for marked open issues."""
    found = {}
    for issue in issues:
        if issue.get("state", "open") != "open":
            continue
        match = MARKER_RE.search(issue.get("body") or "")
        if not match:
            continue
        found[(match.group("leg"), match.group("repo"))] = issue
    return found


def plan_actions(report, issues):
    """Pure planner: (report, existing issues) -> ordered action list."""
    existing = parse_existing(issues)
    actions = []
    for leg, repo, entry in unhealthy_entries(report):
        key = (leg, repo)
        current = existing.get(key)
        status = entry["status"]
        if status == "fresh":
            if current is not None:
                actions.append(
                    {
                        "action": "resolve",
                        "leg": leg,
                        "repo": repo,
                        "issue": current["number"],
                        "status": status,
                        "run_id": report["run_id"],
                        "comment": (
                            f"Resolved by hosted freshness run "
                            f"{report['run_id']} ({report['generated_at']}): "
                            f"{leg} verified fresh. {entry.get('detail', '')}"
                        ).strip(),
                    }
                )
            continue
        if status not in NON_FRESH:
            continue
        if current is None:
            actions.append(
                {
                    "action": "create",
                    "leg": leg,
                    "repo": repo,
                    "status": status,
                    "run_id": report["run_id"],
                    "title": alert_title(leg, repo, status),
                    "body": alert_body(report, leg, repo, entry),
                }
            )
            continue
        recorded = RUN_RE.search(current.get("body") or "")
        if recorded and recorded.group("run") == report["run_id"]:
            actions.append(
                {
                    "action": "noop",
                    "leg": leg,
                    "repo": repo,
                    "issue": current["number"],
                    "status": status,
                    "run_id": report["run_id"],
                }
            )
        else:
            actions.append(
                {
                    "action": "update",
                    "leg": leg,
                    "repo": repo,
                    "issue": current["number"],
                    "status": status,
                    "run_id": report["run_id"],
                    "title": alert_title(leg, repo, status),
                    "body": alert_body(report, leg, repo, entry),
                }
            )
    return actions


# ------------------------------------------------------------ execution
def _run(argv):
    completed = subprocess.run(argv, capture_output=True, text=True, timeout=120)
    if completed.returncode:
        detail = completed.stderr.strip() or completed.stdout.strip()
        raise RuntimeError(f"{' '.join(argv[:3])} failed: {detail[:400]}")
    return completed.stdout.strip()


def gh_executor(repo):
    """Real executor: applies one planned action via `gh api`."""

    def execute(action):
        kind = action["action"]
        if kind == "create":
            out = _run(
                [
                    "gh",
                    "api",
                    f"repos/{repo}/issues",
                    "--method",
                    "POST",
                    "-f",
                    f"title={action['title']}",
                    "-f",
                    f"body={action['body']}",
                    "--jq",
                    ".number",
                ]
            )
            return int(out)
        if kind == "update":
            _run(
                [
                    "gh",
                    "api",
                    f"repos/{repo}/issues/{action['issue']}",
                    "--method",
                    "PATCH",
                    "-f",
                    f"title={action['title']}",
                    "-f",
                    f"body={action['body']}",
                ]
            )
            return action["issue"]
        if kind == "resolve":
            _run(
                [
                    "gh",
                    "api",
                    f"repos/{repo}/issues/{action['issue']}/comments",
                    "--method",
                    "POST",
                    "-f",
                    f"body={action['comment']}",
                ]
            )
            _run(
                [
                    "gh",
                    "api",
                    f"repos/{repo}/issues/{action['issue']}",
                    "--method",
                    "PATCH",
                    "-f",
                    "state=closed",
                    "-f",
                    "state_reason=completed",
                ]
            )
            return action["issue"]
        return None  # noop: nothing to do

    return execute


def fetch_existing(repo):
    """Open issues in the alert repo, as {number, title, body, state}."""
    out = _run(
        [
            "gh",
            "api",
            f"repos/{repo}/issues?state=open&per_page=100",
        ]
    )
    return [
        {
            "number": issue["number"],
            "title": issue.get("title", ""),
            "body": issue.get("body") or "",
            "state": issue.get("state", "open"),
        }
        for issue in json.loads(out)
        if "pull_request" not in issue
    ]


def apply_actions(actions, executor):
    results = []
    for action in actions:
        number = executor(action)
        results.append({**action, "issue": number if number else action.get("issue")})
    return results


# ------------------------------------------------------------ CLI
def parse_args(argv):
    parser = argparse.ArgumentParser(
        prog="hosted_alerts.py",
        description="Deduplicated GitHub alerts for the hosted freshness report (#15001).",
    )
    parser.add_argument(
        "--report", required=True, help="hosted_freshness report JSON file"
    )
    parser.add_argument(
        "--existing",
        default=None,
        help="JSON file of existing open issues (default: fetch via gh api)",
    )
    parser.add_argument(
        "--repo",
        default="developing-today/code",
        help="repository that hosts the alert issues",
    )
    parser.add_argument(
        "--apply",
        action="store_true",
        help="execute the plan via gh api (default: dry-run plan only)",
    )
    return parser.parse_args(argv)


def main(argv=None):
    args = parse_args(argv)
    try:
        with open(args.report, encoding="utf-8") as handle:
            report = json.load(handle)
    except (OSError, json.JSONDecodeError) as exc:
        print(
            f"hosted_alerts: unusable --report {args.report!r}: {exc}", file=sys.stderr
        )
        return 2
    if args.existing:
        try:
            with open(args.existing, encoding="utf-8") as handle:
                issues = json.load(handle)
        except (OSError, json.JSONDecodeError) as exc:
            print(
                f"hosted_alerts: unusable --existing {args.existing!r}: {exc}",
                file=sys.stderr,
            )
            return 2
    elif args.apply:
        issues = fetch_existing(args.repo)
    else:
        issues = []
    actions = plan_actions(report, issues)
    if args.apply:
        actions = apply_actions(actions, gh_executor(args.repo))
    print(json.dumps({"alert": ALERT_ID, "actions": actions}, indent=2, sort_keys=True))
    return 1 if any(a["action"] in ("create", "update") for a in actions) else 0


if __name__ == "__main__":
    sys.exit(main())
