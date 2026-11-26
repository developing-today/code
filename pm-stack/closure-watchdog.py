#!/usr/bin/env python3
"""Closure watchdog (code#10105) — reopen issues automation wrongly closes.

The developing-today[bot] status automation (Kaneo's GitHub sync) has
closed issues seconds after a PR merge that merely *referenced* them —
no closing keyword, work deliberately left open (defect #7610, fixed at
the source 2026-11-14 by moving Kaneo's onPRMerge transition to
in-review, and asserted continuously by pm-stack/kaneo-sync-guard.py).
This watchdog is the outer safety net for every close path the guard
cannot see (a re-connect restoring the upstream default, a task moved
to Done inside Kaneo, a future regression): it polls recent issue-close
events in the app-covered repos and reopens an automation close when

  (a) the issue still has OPEN native sub-issues, or
  (b) the closing PR referenced the issue WITHOUT a closing keyword
      (Closes/Fixes/Resolves #n) in its title/body — a Refs-only merge.

On a wrongful close it reopens the issue, removes the status:done
label, and posts a short evidence comment naming the closing actor,
the close time, the closing PR, and which rule fired.

Legitimate closes are left untouched: a closing keyword in the closing
PR makes the close legitimate, and a human close (including a human
close within HUMAN_GUARD_SECONDS before a bot echo close) is never
reopened.

State is a JSON watermark file: per repo, the newest close-event time
processed. Each close event is processed once; the watermark advances
past skipped events too.

The original 2026-11-14 implementation lived only in the Muse sandbox
(~/workspace/closure-watchdog/) and was lost in the 2026-11-21 sandbox
recovery — the live cron job and the script both vanished while
doc/fleet-schedule-registry.md kept listing the job as enabled. This
repo-owned copy is the durable replacement; the sandbox cron job runs
it from a checkout of this repo.

Run (gh CLI only, on the Muse sandbox):
  python3 pm-stack/closure-watchdog.py [--dry-run] [--state PATH]
"""

import argparse
import json
import re
import subprocess
import sys
import urllib.parse
from datetime import datetime, timedelta, timezone
from pathlib import Path

REPOS = [
    "developing-today/code",
    "developing-today-forks/opencode",
    "developing-today-forks/openchamber",
]

# Actors whose closes this watchdog polices. Human closes are never
# reopened; other bots are left alone unless listed here.
AUTOMATION_ACTORS = {"developing-today[bot]"}

DONE_LABEL = "status:done"

# A human close this many seconds before a bot close marks the bot
# close as an echo of the human's action, not an automation decision.
HUMAN_GUARD_SECONDS = 120

# A merged PR counts as the closing PR when it merged within this
# window before the close (the observed bot closes land seconds after
# the merge; the window is generous to timeline read lag).
PR_MERGE_WINDOW_SECONDS = 900

# Re-scan overlap behind the watermark so a close whose timeline read
# lagged a run is still seen exactly once (the watermark comparison,
# not the scan window, decides what is new).
SCAN_OVERLAP_SECONDS = 3600

CLOSING_KEYWORD = re.compile(
    r"\b(?:close[sd]?|fix(?:e[sd])?|resolve[sd]?)\s+#(\d+)", re.IGNORECASE
)


def parse_iso(ts):
    return datetime.fromisoformat(ts.replace("Z", "+00:00"))


def has_closing_keyword(text, issue_number):
    """True when text carries a closing keyword for this issue."""
    if not text:
        return False
    return any(int(m.group(1)) == issue_number for m in CLOSING_KEYWORD.finditer(text))


def find_closing_pr(events, close_time):
    """The merged PR whose merge most plausibly triggered this close.

    Timeline cross-referenced events carry the source issue; a source
    that is a pull request merged inside the window before the close
    qualifies. The latest such merge wins.
    """
    best = None
    for ev in events:
        if ev.get("event") != "cross-referenced":
            continue
        src = (ev.get("source") or {}).get("issue") or {}
        pr = src.get("pull_request") or {}
        merged_at = pr.get("merged_at")
        if not merged_at:
            continue
        delta = (close_time - parse_iso(merged_at)).total_seconds()
        if 0 <= delta <= PR_MERGE_WINDOW_SECONDS:
            if best is None or merged_at > best["merged_at"]:
                best = {
                    "number": src.get("number"),
                    "title": src.get("title") or "",
                    "body": src.get("body") or "",
                    "merged_at": merged_at,
                }
    return best


def decide_close(
    *, actor, human_close_before, open_sub_issues, closing_pr, issue_number
):
    """Verdict for one close event.

    Returns (rules_fired, reason). A non-empty rules_fired means the
    close is wrongful and the issue must be reopened; an empty list
    with a reason means the close is left untouched.
    """
    if actor not in AUTOMATION_ACTORS:
        return [], f"close actor {actor!r} is not a policed automation"
    if human_close_before:
        return [], "human close inside the guard window before this bot close"
    rules = []
    if open_sub_issues > 0:
        rules.append("open-sub-issues")
    if closing_pr is not None and not has_closing_keyword(
        f"{closing_pr['title']}\n{closing_pr['body']}", issue_number
    ):
        rules.append("refs-only-merge")
    if rules:
        return rules, "wrongful automation close"
    if closing_pr is not None:
        return [], "closing PR carries a closing keyword for this issue"
    return [], "no open sub-issues and no linked merged PR"


def evidence_comment(*, actor, closed_at, closing_pr, rules):
    lines = [
        "Closure watchdog (#10105): reopening this issue — it was closed "
        f"by {actor} at {closed_at} and the close matches the wrongful-close "
        "rule(s): " + ", ".join(rules) + ".",
    ]
    if closing_pr is not None:
        lines.append(
            f"Closing PR: #{closing_pr['number']} (merged {closing_pr['merged_at']})."
        )
    if "open-sub-issues" in rules:
        lines.append("The issue still has open sub-issues.")
    if "refs-only-merge" in rules:
        lines.append(
            "The closing PR referenced this issue without a closing keyword "
            "(Closes/Fixes/Resolves) — a Refs-only merge must not close it."
        )
    return "\n".join(lines)


class GhClient:
    """The gh boundary. Tests replace this with an in-memory fake."""

    def run(self, args, check=True):
        out = subprocess.run(["gh"] + args, capture_output=True, text=True)
        if check and out.returncode != 0:
            raise RuntimeError(
                f"gh {' '.join(args[:3])} failed: {out.stderr.strip()[:300]}"
            )
        return out

    def api(self, path):
        out = self.run(["api", path])
        return json.loads(out.stdout) if out.stdout.strip() else None

    def recently_closed_issues(self, repo, since_iso):
        # Percent-encode the query: since_iso carries a "+00:00" UTC
        # offset, and a raw "+" in a query string decodes as a space,
        # which makes GitHub silently return an empty list (found
        # live 2026-11-25 during the #10105 fixture verification --
        # the watchdog saw no closes at all).
        query = urllib.parse.urlencode(
            {
                "state": "closed",
                "since": since_iso,
                "sort": "updated",
                "direction": "asc",
                "per_page": 100,
            }
        )
        issues = self.api(f"repos/{repo}/issues?{query}")
        return [i for i in issues or [] if "pull_request" not in i]

    def timeline(self, repo, number):
        return self.api(f"repos/{repo}/issues/{number}/timeline?per_page=100") or []

    def open_sub_issue_count(self, repo, number):
        subs = self.api(f"repos/{repo}/issues/{number}/sub_issues?per_page=100") or []
        return sum(1 for s in subs if s.get("state") == "open")

    def reopen(self, repo, number):
        self.run(
            ["api", "-X", "PATCH", f"repos/{repo}/issues/{number}", "-f", "state=open"]
        )

    def remove_label(self, repo, number, label):
        self.run(
            ["api", "-X", "DELETE", f"repos/{repo}/issues/{number}/labels/{label}"],
            check=False,
        )

    def comment(self, repo, number, body):
        self.run(
            [
                "api",
                "-X",
                "POST",
                f"repos/{repo}/issues/{number}/comments",
                "-f",
                f"body={body}",
            ]
        )


def latest_close(events):
    """(closed_at_iso, actor, human_close_before) of the newest close."""
    closes = [e for e in events if e.get("event") == "closed"]
    if not closes:
        return None
    last = max(closes, key=lambda e: e["created_at"])
    close_time = parse_iso(last["created_at"])
    human_before = any(
        e is not last
        and (e.get("actor") or {}).get("login") not in AUTOMATION_ACTORS
        and 0
        <= (close_time - parse_iso(e["created_at"])).total_seconds()
        <= HUMAN_GUARD_SECONDS
        for e in closes
    )
    return last["created_at"], (last.get("actor") or {}).get("login"), human_before


def process_repo(client, repo, watermark_iso, dry_run=False):
    """Process every close newer than the watermark.

    Returns (actions, new_watermark_iso). actions is a list of dicts
    describing each reopen performed (or, in dry-run, that would be
    performed). The watermark advances past every close examined,
    fired or skipped, so each close is processed once.
    """
    if watermark_iso:
        since = parse_iso(watermark_iso) - timedelta(seconds=SCAN_OVERLAP_SECONDS)
    else:
        since = datetime.now(timezone.utc) - timedelta(hours=1)
    actions = []
    new_watermark = watermark_iso
    for issue in client.recently_closed_issues(repo, since.isoformat()):
        number = issue["number"]
        events = client.timeline(repo, number)
        close = latest_close(events)
        if close is None:
            continue
        closed_at, actor, human_before = close
        if watermark_iso and closed_at <= watermark_iso:
            continue
        if new_watermark is None or closed_at > new_watermark:
            new_watermark = closed_at
        closing_pr = find_closing_pr(events, parse_iso(closed_at))
        open_subs = client.open_sub_issue_count(repo, number)
        rules, _reason = decide_close(
            actor=actor,
            human_close_before=human_before,
            open_sub_issues=open_subs,
            closing_pr=closing_pr,
            issue_number=number,
        )
        if not rules:
            continue
        action = {
            "repo": repo,
            "issue": number,
            "actor": actor,
            "closed_at": closed_at,
            "closing_pr": closing_pr["number"] if closing_pr else None,
            "rules": rules,
            "dry_run": dry_run,
        }
        if not dry_run:
            client.reopen(repo, number)
            client.remove_label(repo, number, DONE_LABEL)
            client.comment(
                repo,
                number,
                evidence_comment(
                    actor=actor,
                    closed_at=closed_at,
                    closing_pr=closing_pr,
                    rules=rules,
                ),
            )
        actions.append(action)
    return actions, new_watermark


def load_state(path):
    if path and Path(path).exists():
        return json.loads(Path(path).read_text())
    return {}


def save_state(path, state):
    if path:
        Path(path).write_text(json.dumps(state, indent=2, sort_keys=True) + "\n")


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument(
        "--dry-run",
        action="store_true",
        help="report wrongful closes without reopening",
    )
    ap.add_argument(
        "--state",
        default=str(Path(__file__).with_name("state.json")),
        help="watermark state file (default: state.json beside the script)",
    )
    ap.add_argument(
        "--repo",
        action="append",
        dest="repos",
        help="repo to police (repeatable; default: the app-covered set)",
    )
    args = ap.parse_args(argv)

    state = load_state(args.state)
    client = GhClient()
    total = 0
    for repo in args.repos or REPOS:
        actions, new_watermark = process_repo(
            client, repo, state.get(repo), dry_run=args.dry_run
        )
        state[repo] = new_watermark
        total += len(actions)
        for a in actions:
            verb = "WOULD REOPEN" if a["dry_run"] else "REOPENED"
            print(
                f"{verb} {a['repo']}#{a['issue']} closed by {a['actor']} "
                f"at {a['closed_at']} rules={','.join(a['rules'])} "
                f"closing_pr={a['closing_pr']}"
            )
    if not args.dry_run:
        save_state(args.state, state)
    print(f"closure-watchdog: {total} wrongful close(s) handled")
    return 0


if __name__ == "__main__":
    sys.exit(main())
