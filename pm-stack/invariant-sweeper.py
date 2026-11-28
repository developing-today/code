#!/usr/bin/env python3
"""Epic-invariant sweeper (close-authority program code#10136, slice #10139).

A slow-cadence, deep sweep across all four Kaneo-synced repos
(developing-today/code, developing-today-forks/opencode,
developing-today-forks/openchamber, dezren39/nix) for the state the
closure watchdog (code#10105/#10138) cannot see: closes that predate
its watermark, drift that never produced a close event, and
Kaneo<->GitHub disagreement. Three invariants per run:

  (a) a CLOSED issue whose sub-issues are not all completed
      (the epic invariant: a parent with open children is never done);
  (b) an OPEN issue carrying Kaneo's `status:done` label;
  (c) Kaneo<->GitHub state mismatch for linked tasks (Kaneo task done
      while the GitHub issue is open, or not-done while it is closed).

Policy (design doc doc/2026-11-14T17-00-00Z_design_pm-stack-close-authority,
P4): REPORT FIRST. The default mode inventories and reports; it changes
nothing on GitHub. Auto-repair is a separate, explicit mode (--act) to
be enabled only after the first inventory has been reviewed: in --act
mode the sweeper reopens invariant-(a) issues that a bot closed with
open sub-issues and no closing keyword (the watchdog's guards, incl.
the human-echo guard), strips bot-applied `status:done` labels under
invariant (b), and still only reports invariant (c) — Kaneo is a board
view, GitHub is the truth, and a mismatch is for the sync or a human
to heal, never for this sweeper to force.

Data discipline: enumeration is GraphQL-first via `fleet gh api
graphql` (Drew, 2026-11-14: bulk/automated calls never use the
personal token — `fleet gh` rotates personal -> fleet App ->
developing.today App -> backup App with failover; the personal token
rate-limiting the closure watchdog's cycle is the failure this
avoids). GraphQL also has its own pool, separate from the REST core
budget the 15-minute watchdog shares. REST is used only by --act
repairs, likewise via `fleet gh`. If `fleet` is not on PATH the
sweeper falls back to bare `gh` and records that loudly in the report
(`gh_route`) — a silent fallback would recreate the problem. Every
repo is swept in isolation: one repo's failure is recorded in the
report and the sweep continues; the run exits 1 when any part failed,
0 when the sweep completed (findings are data, not failure), 2 on
usage errors.

Known-positive verification: code#9060 (bot-closed 2026-11-12 with all
12 sub-issues open) is evaluated by the same classifier on every run
and reported by name. If it is closed when a run happens, it is a
live invariant-(a) finding; if the closure-audit pass has already
reopened it, the report says so (resolved-by-audit) with the close
actor/timestamp evidence the acceptance asks for.

State: ~/.local/state/pm-invariant-sweeper/state.json (override with
--state-file) — survives restarts, records run history, the last
violation fingerprint, and the fingerprint last posted to the epic so
the daily run comments on code#10136 only when the violation set
changes (or with --force-post). Reports are written next to it as
last-report.md / last-report.json unless --report-dir says otherwise.

Argument contract (repo #3352): the whole command line is declared
below and parsed strictly before any subprocess, network, or file
write; unknown flags, stray positionals, `--opt=value` forms for
flags that take no value, and valueless options are usage errors
(exit 2) naming the offending argument.

Run on the laptop (Kaneo's Postgres lives there):
  python3 pm-stack/invariant-sweeper.py              # report (default)
  python3 pm-stack/invariant-sweeper.py --post-epic  # + epic comment if changed
  python3 pm-stack/invariant-sweeper.py --act        # after review (see above)
"""

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
from datetime import datetime, timedelta, timezone
from pathlib import Path

DEFAULT_REPOS = [
    "developing-today/code",
    "developing-today-forks/opencode",
    "developing-today-forks/openchamber",
    "dezren39/nix",
]
EPIC_REPO = "developing-today/code"
EPIC_NUMBER = 10136
KNOWN_POSITIVE = ("developing-today/code", 9060)
BOT_LOGINS = {"developing-today[bot]", "developing-today"}
DONE_LABEL = "status:done"
CLOSE_WINDOW = timedelta(minutes=15)  # PR merge -> bot close gap (watchdog parity)
HUMAN_ECHO_WINDOW = timedelta(seconds=120)  # human close just before a bot echo close
MAX_EPIC_ROWS = 50

STATE_DIR = Path.home() / ".local" / "state" / "pm-invariant-sweeper"

CLOSING_KEYWORDS = r"(?:close|closes|closed|fix|fixes|fixed|resolve|resolves|resolved)"


def parse_args(argv):
    """Strict declaration of the whole command line (#3352)."""
    parser = argparse.ArgumentParser(
        prog="invariant-sweeper.py",
        allow_abbrev=False,
        description="Epic-invariant sweeper (code#10139): inventory closed-with-"
        "open-sub-issues, open-with-status:done, and Kaneo<->GitHub "
        "mismatches across the four Kaneo-synced repos.",
    )
    parser.add_argument(
        "--act",
        action="store_true",
        help="enable auto-repair (reopen eligible invariant-(a) "
        "issues, strip bot-applied status:done). Default is "
        "report-only, per the report-first policy.",
    )
    parser.add_argument(
        "--post-epic",
        action="store_true",
        help="comment the inventory on epic code#10136 when the "
        "violation fingerprint changed since the last post.",
    )
    parser.add_argument(
        "--force-post",
        action="store_true",
        help="with --post-epic, post even if the fingerprint is unchanged.",
    )
    parser.add_argument(
        "--repo",
        action="append",
        default=None,
        metavar="OWNER/NAME",
        help="limit the sweep to this repo (repeatable). Default: "
        "all four Kaneo-synced repos.",
    )
    parser.add_argument(
        "--state-file",
        default=str(STATE_DIR / "state.json"),
        metavar="PATH",
        help="state file path.",
    )
    parser.add_argument(
        "--report-dir",
        default=str(STATE_DIR),
        metavar="PATH",
        help="directory for last-report.md/.json.",
    )
    parser.add_argument(
        "--skip-kaneo",
        action="store_true",
        help="skip invariant (c) (Kaneo Postgres unreachable from "
        "here); the report records the section as skipped.",
    )
    args = parser.parse_args(argv)
    repos = args.repo if args.repo else list(DEFAULT_REPOS)
    for repo in repos:
        if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repo):
            parser.error(f"--repo: invalid owner/name {repo!r}")
    args.repos = repos
    return args


# ---------------------------------------------------------------- gh access

# Resolved in main() before any gh call: ["fleet", "gh"] where the
# fleet binary exists (the laptop), else ["gh"] with the fallback
# recorded in the report. See the header's data-discipline note.
GH_CMD = ["gh"]
GH_ROUTE = "gh (unresolved)"


def resolve_gh_route():
    global GH_CMD, GH_ROUTE
    import shutil

    if shutil.which("fleet"):
        GH_CMD = ["fleet", "gh"]
        GH_ROUTE = "fleet gh (rotating app tiers)"
    else:
        GH_CMD = ["gh"]
        GH_ROUTE = "gh (FALLBACK: fleet binary not on PATH — personal token)"


# #11416: every subprocess this sweeper starts is bounded. The timer
# runs a oneshot unit, so one wedged gh/psql call would otherwise hang
# the daily sweep forever. Override with PM_SWEEPER_SUBPROCESS_TIMEOUT
# (seconds).
SUBPROCESS_TIMEOUT_SECS = int(os.environ.get("PM_SWEEPER_SUBPROCESS_TIMEOUT", "120"))


def run_bounded(cmd, what):
    """subprocess.run with a timeout; a timeout becomes a RuntimeError
    naming the call, like every other failure of these helpers."""
    try:
        return subprocess.run(
            cmd, capture_output=True, text=True, timeout=SUBPROCESS_TIMEOUT_SECS
        )
    except subprocess.TimeoutExpired:
        raise RuntimeError(
            f"{what} timed out after {SUBPROCESS_TIMEOUT_SECS}s"
        ) from None


def gh_graphql(query, **variables):
    """One GraphQL call via the gh CLI. Raises RuntimeError on failure."""
    cmd = [*GH_CMD, "api", "graphql", "-f", f"query={query}"]
    for key, value in variables.items():
        # -F types the value (ints stay ints); -f keeps strings verbatim
        # (pagination cursors must never be re-typed).
        cmd += ["-F" if isinstance(value, int) else "-f", f"{key}={value}"]
    out = run_bounded(cmd, "gh api graphql")
    if out.returncode != 0:
        raise RuntimeError(f"gh api graphql failed: {out.stderr.strip()[:300]}")
    data = json.loads(out.stdout)
    if data.get("errors"):
        raise RuntimeError(f"graphql errors: {json.dumps(data['errors'])[:300]}")
    return data["data"]


def gh_run(*args):
    out = run_bounded([*GH_CMD, *args], f"gh {' '.join(args)}")
    if out.returncode != 0:
        raise RuntimeError(f"gh {' '.join(args)} failed: {out.stderr.strip()[:300]}")
    return out.stdout


ISSUES_QUERY = """
query($owner:String!,$name:String!,$cursor:String){
  repository(owner:$owner,name:$name){
    issues(first:100, after:$cursor, states:[OPEN,CLOSED],
           orderBy:{field:CREATED_AT,direction:ASC}){
      pageInfo{ hasNextPage endCursor }
      nodes{ number state title
             labels(first:30){ nodes{ name } }
             subIssuesSummary{ total completed } }
    }
  }
}
"""

CLOSE_EVIDENCE_QUERY = """
query($owner:String!,$name:String!,$number:Int!){
  repository(owner:$owner,name:$name){
    issue(number:$number){
      number state title
      subIssuesSummary{ total completed }
      timelineItems(itemTypes:[CLOSED_EVENT], last:5){
        nodes{ ... on ClosedEvent{ createdAt actor{ login } } }
      }
      crossRefs: timelineItems(itemTypes:[CROSS_REFERENCED_EVENT], last:50){
        nodes{ ... on CrossReferencedEvent{
          source{ ... on PullRequest{ number merged mergedAt title body } } } }
      }
    }
  }
}
"""

LABELED_QUERY = """
query($owner:String!,$name:String!,$number:Int!){
  repository(owner:$owner,name:$name){
    issue(number:$number){
      timelineItems(itemTypes:[LABELED_EVENT], last:100){
        nodes{ ... on LabeledEvent{ createdAt actor{ login } label{ name } } }
      }
    }
  }
}
"""


def enumerate_issues(repo):
    owner, name = repo.split("/", 1)
    issues, cursor = [], None
    while True:
        data = gh_graphql(
            ISSUES_QUERY,
            owner=owner,
            name=name,
            **({"cursor": cursor} if cursor else {}),
        )
        conn = data["repository"]["issues"]
        issues.extend(conn["nodes"])
        if not conn["pageInfo"]["hasNextPage"]:
            return issues
        cursor = conn["pageInfo"]["endCursor"]


def parse_ts(s):
    return datetime.fromisoformat(s.replace("Z", "+00:00"))


def is_bot(login):
    return bool(login) and (login.endswith("[bot]") or login in BOT_LOGINS)


def closing_keyword_re(repo, number):
    owner_repo = re.escape(repo)
    num = re.escape(str(number))
    ref = rf"(?:https://github\.com/{owner_repo}/issues/{num}|{owner_repo}#{num}|#{num}\b)"
    return re.compile(rf"\b{CLOSING_KEYWORDS}\b\s*:?\s*{ref}", re.IGNORECASE)


def classify_close(repo, number):
    """Close evidence for one issue: last close actor/time, trigger PR,
    closing-keyword presence, human-echo guard. Mirrors the watchdog."""
    owner, name = repo.split("/", 1)
    data = gh_graphql(CLOSE_EVIDENCE_QUERY, owner=owner, name=name, number=number)
    issue = data["repository"]["issue"]
    closes = [n for n in issue["timelineItems"]["nodes"] if n]
    evidence = {
        "open_sub_issues": None,
        "close_actor": None,
        "closed_at": None,
        "trigger_pr": None,
        "closing_keyword": False,
        "human_echo": None,
    }
    summary = issue.get("subIssuesSummary") or {}
    total, completed = summary.get("total") or 0, summary.get("completed") or 0
    evidence["open_sub_issues"] = total - completed
    if not closes:
        return evidence
    last = closes[-1]
    evidence["close_actor"] = (last.get("actor") or {}).get("login")
    evidence["closed_at"] = last.get("createdAt")
    closed_at = parse_ts(last["createdAt"])
    # Human-echo guard: a human close within 120s before the last close
    # means the last close is Kaneo echoing the human, not a bot act.
    for ev in closes[:-1]:
        actor = (ev.get("actor") or {}).get("login", "")
        if is_bot(actor):
            continue
        at = parse_ts(ev["createdAt"])
        if closed_at - HUMAN_ECHO_WINDOW <= at <= closed_at:
            evidence["human_echo"] = actor
    kw_re = closing_keyword_re(repo, number)
    for node in issue["crossRefs"]["nodes"]:
        src = (node or {}).get("source") or {}
        if not src.get("merged") or not src.get("mergedAt"):
            continue
        merged_at = parse_ts(src["mergedAt"])
        if closed_at - CLOSE_WINDOW <= merged_at <= closed_at + timedelta(minutes=2):
            evidence["trigger_pr"] = src["number"]
            text = f"{src.get('title', '')}\n{src.get('body') or ''}"
            evidence["closing_keyword"] = bool(kw_re.search(text))
            break
    return evidence


def label_actor(repo, number, label):
    """Actor of the most recent application of `label`, or None."""
    owner, name = repo.split("/", 1)
    data = gh_graphql(LABELED_QUERY, owner=owner, name=name, number=number)
    actor = None
    for node in data["repository"]["issue"]["timelineItems"]["nodes"]:
        if node and (node.get("label") or {}).get("name") == label:
            actor = (node.get("actor") or {}).get("login")
    return actor


# ---------------------------------------------------------------- Kaneo (c)

KAN_CONTAINER = "pm-stack-postgres-1"
KAN_PSQL = [
    "docker",
    "exec",
    KAN_CONTAINER,
    "psql",
    "-U",
    "kaneo",
    "-d",
    "kaneo",
    "-tA",
    "-v",
    "ON_ERROR_STOP=1",
]


def kaneo_links():
    """{(repo, issue_number): kaneo_task_status} for GitHub issue links."""
    sql = (
        "SELECT el.url, t.status FROM external_link el "
        "JOIN task t ON t.id = el.task_id "
        "WHERE el.resource_type='issue'"
    )
    out = run_bounded(KAN_PSQL + ["-c", sql], "kaneo psql")
    if out.returncode != 0:
        raise RuntimeError(f"kaneo psql failed: {out.stderr.strip()[:300]}")
    links = {}
    pat = re.compile(r"^https://github\.com/([^/]+/[^/]+)/issues/(\d+)$")
    for line in out.stdout.splitlines():
        url, _, status = line.partition("|")
        m = pat.match(url.strip())
        if m:
            links[(m.group(1), int(m.group(2)))] = status.strip()
    return links


# ---------------------------------------------------------------- repairs


def act_on_finding(kind, repo, number, evidence):
    """--act repairs. Returns an action string. Only called for findings
    the design's policy makes eligible; everything else stays report-only."""
    if kind == "a":
        gh_run("issue", "reopen", str(number), "--repo", repo)
        issue_labels = evidence.get("labels") or []
        if DONE_LABEL in issue_labels:
            gh_run(
                "issue",
                "edit",
                str(number),
                "--repo",
                repo,
                "--remove-label",
                DONE_LABEL,
            )
        body = (
            "**Invariant sweeper (code#10139): reopening a wrongful auto-close.**\n\n"
            f"- Closed by `{evidence.get('close_actor')}` at {evidence.get('closed_at')}.\n"
            f"- The issue still has {evidence.get('open_sub_issues')} open sub-issue(s); "
            "a parent with open children is never done (epic invariant, code#10136).\n"
            f"- Trigger PR: #{evidence.get('trigger_pr')} (no closing keyword for this issue).\n"
            "- Reopened under the sweeper's post-review auto-reopen policy "
            "(design: doc/2026-11-14T17-00-00Z_design_pm-stack-close-authority). "
            "Defect: #7610. Watchdog: #10105."
        )
        gh_run("issue", "comment", str(number), "--repo", repo, "--body", body)
        return "reopened"
    if kind == "b":
        gh_run(
            "issue", "edit", str(number), "--repo", repo, "--remove-label", DONE_LABEL
        )
        gh_run(
            "issue",
            "comment",
            str(number),
            "--repo",
            repo,
            "--body",
            "**Invariant sweeper (code#10139): removed a bot-applied "
            "`status:done` label from an open issue.** The label is Kaneo's "
            "mirror of a Done task; on an open issue it is contradictory "
            "state (close-authority program code#10136).",
        )
        return "stripped-done-label"
    return "report-only"


# ---------------------------------------------------------------- the sweep


def sweep_repo(repo, act):
    """Returns (section_dict, error_or_None). Never raises: per-repo
    isolation is part of the design (one repo down must not blind the rest)."""
    section = {
        "repo": repo,
        "issues_total": 0,
        "closed_total": 0,
        "open_total": 0,
        "a_closed_open_subs": [],
        "b_open_done_label": [],
        "actions": [],
    }
    try:
        issues = enumerate_issues(repo)
    except Exception as exc:
        return section, str(exc)[:300]
    section["issues_total"] = len(issues)
    by_number = {}
    for issue in issues:
        by_number[issue["number"]] = issue
        labels = [l["name"] for l in issue["labels"]["nodes"]]
        if issue["state"] == "CLOSED":
            section["closed_total"] += 1
            summary = issue.get("subIssuesSummary") or {}
            open_subs = (summary.get("total") or 0) - (summary.get("completed") or 0)
            if open_subs > 0:
                finding = {
                    "number": issue["number"],
                    "title": issue["title"],
                    "open_sub_issues": open_subs,
                    "labels": labels,
                }
                try:
                    ev = classify_close(repo, issue["number"])
                    finding.update(
                        {
                            k: ev[k]
                            for k in (
                                "close_actor",
                                "closed_at",
                                "trigger_pr",
                                "closing_keyword",
                                "human_echo",
                            )
                        }
                    )
                    finding["eligible_reopen"] = bool(
                        is_bot(ev["close_actor"])
                        and not ev["closing_keyword"]
                        and not ev["human_echo"]
                    )
                except Exception as exc:
                    finding["evidence_error"] = str(exc)[:200]
                    finding["eligible_reopen"] = False
                section["a_closed_open_subs"].append(finding)
        else:
            section["open_total"] += 1
            if DONE_LABEL in labels:
                finding = {"number": issue["number"], "title": issue["title"]}
                try:
                    finding["label_actor"] = label_actor(
                        repo, issue["number"], DONE_LABEL
                    )
                except Exception as exc:
                    finding["label_actor_error"] = str(exc)[:200]
                section["b_open_done_label"].append(finding)
    if act:
        for finding in section["a_closed_open_subs"]:
            if finding.get("eligible_reopen"):
                try:
                    action = act_on_finding("a", repo, finding["number"], finding)
                    section["actions"].append(
                        {"number": finding["number"], "action": action}
                    )
                    finding["action"] = action
                except Exception as exc:
                    section["actions"].append(
                        {"number": finding["number"], "error": str(exc)[:200]}
                    )
        for finding in section["b_open_done_label"]:
            if is_bot(finding.get("label_actor")):
                try:
                    action = act_on_finding("b", repo, finding["number"], finding)
                    section["actions"].append(
                        {"number": finding["number"], "action": action}
                    )
                    finding["action"] = action
                except Exception as exc:
                    section["actions"].append(
                        {"number": finding["number"], "error": str(exc)[:200]}
                    )
    section["_by_number"] = by_number
    return section, None


def known_positive_status():
    """Evaluate code#9060 with the same classifier the sweep uses."""
    repo, number = KNOWN_POSITIVE
    out = {"repo": repo, "number": number}
    try:
        owner, name = repo.split("/", 1)
        data = gh_graphql(CLOSE_EVIDENCE_QUERY, owner=owner, name=name, number=number)
        issue = data["repository"]["issue"]
        out["state"] = issue["state"]
        summary = issue.get("subIssuesSummary") or {}
        out["open_sub_issues"] = (summary.get("total") or 0) - (
            summary.get("completed") or 0
        )
        closes = [n for n in issue["timelineItems"]["nodes"] if n]
        if closes:
            last = closes[-1]
            out["last_close_actor"] = (last.get("actor") or {}).get("login")
            out["last_closed_at"] = last.get("createdAt")
        out["detected_as_wrongful_close"] = bool(
            closes
            and is_bot(out.get("last_close_actor"))
            and out["open_sub_issues"] > 0
        )
        out["verdict"] = (
            "live-violation" if issue["state"] == "CLOSED" else "resolved-by-audit"
        )
    except Exception as exc:
        out["error"] = str(exc)[:300]
    return out


def fingerprint(report):
    parts = []
    for sec in report["repos"]:
        for f in sec["a_closed_open_subs"]:
            parts.append(f"a:{sec['repo']}#{f['number']}")
        for f in sec["b_open_done_label"]:
            parts.append(f"b:{sec['repo']}#{f['number']}")
    for m in report.get("c_kaneo_mismatches", {}).get("items", []):
        parts.append(f"c:{m['repo']}#{m['number']}:{m['kind']}")
    return hashlib.sha256("\n".join(sorted(parts)).encode()).hexdigest()[:16]


def render_markdown(report):
    lines = [
        f"## Invariant sweeper inventory — {report['run_at']}",
        "",
        f"Mode: **{report['mode']}** · Program: code#10136 · Sweeper: code#10139 · "
        "Watchdog: code#10105",
        f"GitHub route: {report.get('gh_route', 'unknown')}",
        "",
    ]
    kp = report["known_positive"]
    lines += ["### Known positive — code#9060", ""]
    if kp.get("error"):
        lines.append(f"- evaluation error: {kp['error']}")
    else:
        lines.append(
            f"- code#9060: closed by `{kp.get('last_close_actor')}` at "
            f"{kp.get('last_closed_at')} with {kp.get('open_sub_issues')} sub-issue(s) "
            f"still open — classifier verdict: "
            f"{'WRONGFUL CLOSE detected' if kp.get('detected_as_wrongful_close') else 'not detected'}; "
            f"current state **{kp.get('state')}** "
            f"({kp.get('verdict')})."
        )
    lines.append("")
    for sec in report["repos"]:
        lines += [f"### {sec['repo']}", ""]
        if sec.get("error"):
            lines += [f"- **SWEEP ERROR:** {sec['error']}", ""]
            continue
        lines.append(
            f"- Issues enumerated: {sec['issues_total']} "
            f"({sec['closed_total']} closed, {sec['open_total']} open)."
        )
        a = sec["a_closed_open_subs"]
        lines.append(f"- (a) Closed with open sub-issues: **{len(a)}**")
        for f in a[:MAX_EPIC_ROWS]:
            lines.append(
                f"  - #{f['number']} — {f['open_sub_issues']} open sub-issue(s); "
                f"closed by `{f.get('close_actor')}` at {f.get('closed_at')}; "
                f"trigger PR: {('#' + str(f['trigger_pr'])) if f.get('trigger_pr') else 'none found'}; "
                f"closing keyword: {'yes' if f.get('closing_keyword') else 'no'}"
                + (f"; action: {f['action']}" if f.get("action") else "")
                + ("; **eligible for auto-reopen**" if f.get("eligible_reopen") else "")
            )
        if len(a) > MAX_EPIC_ROWS:
            lines.append(
                f"  - … and {len(a) - MAX_EPIC_ROWS} more (full list in the run's JSON report)."
            )
        b = sec["b_open_done_label"]
        lines.append(f"- (b) Open with `status:done` label: **{len(b)}**")
        for f in b[:MAX_EPIC_ROWS]:
            lines.append(
                f"  - #{f['number']} — label applied by `{f.get('label_actor')}`"
                + (f"; action: {f['action']}" if f.get("action") else "")
            )
        if len(b) > MAX_EPIC_ROWS:
            lines.append(f"  - … and {len(b) - MAX_EPIC_ROWS} more.")
        if sec["actions"]:
            lines.append(f"- Actions taken: {json.dumps(sec['actions'])}")
        lines.append("")
    c = report.get("c_kaneo_mismatches")
    lines += ["### Kaneo ↔ GitHub mismatches (c)", ""]
    if c is None:
        lines.append("- Section skipped (--skip-kaneo).")
    elif c.get("error"):
        lines.append(f"- **UNAVAILABLE:** {c['error']}")
    else:
        lines.append(
            f"- Linked GitHub issues in Kaneo: {c['linked']}; "
            f"mismatches: **{c['count']}** "
            f"(Kaneo done / GitHub open: {c['kaneo_done_gh_open']}; "
            f"Kaneo not-done / GitHub closed: {c['kaneo_open_gh_closed']}). "
            "Report-only by policy: GitHub is the truth; the sync or a "
            "human heals Kaneo."
        )
        for m in c["items"][:MAX_EPIC_ROWS]:
            lines.append(
                f"  - {m['repo']}#{m['number']} — Kaneo task `{m['kaneo_status']}` "
                f"vs GitHub `{m['github_state']}`"
            )
        if len(c["items"]) > MAX_EPIC_ROWS:
            lines.append(f"  - … and {len(c['items']) - MAX_EPIC_ROWS} more.")
    lines.append("")
    lines.append(f"Violation fingerprint: `{report['fingerprint']}`")
    return "\n".join(lines) + "\n"


def main(argv=None):
    args = parse_args(argv)
    resolve_gh_route()
    run_at = datetime.now(timezone.utc).isoformat()
    report = {
        "run_at": run_at,
        "mode": "act" if args.act else "report",
        "gh_route": GH_ROUTE,
        "repos": [],
        "errors": [],
    }
    sections = []
    for repo in args.repos:
        section, error = sweep_repo(repo, args.act)
        if error:
            section["error"] = error
            report["errors"].append(f"{repo}: {error}")
        by_number = section.pop("_by_number", {})
        sections.append((section, by_number))
        report["repos"].append(section)

    # Invariant (c): cross-reference the Kaneo link dump with the
    # states this same sweep just enumerated (no second GitHub pass).
    if args.skip_kaneo:
        report["c_kaneo_mismatches"] = None
    else:
        try:
            links = kaneo_links()
            gh_state = {}
            for section, by_number in sections:
                for number, issue in by_number.items():
                    gh_state[(section["repo"], number)] = issue["state"]
            items, done_open, open_closed = [], 0, 0
            for (repo, number), status in sorted(links.items()):
                state = gh_state.get((repo, number))
                if state is None:
                    continue
                if status == "done" and state == "OPEN":
                    done_open += 1
                    items.append(
                        {
                            "repo": repo,
                            "number": number,
                            "kind": "kaneo-done-gh-open",
                            "kaneo_status": status,
                            "github_state": state,
                        }
                    )
                elif status != "done" and state == "CLOSED":
                    open_closed += 1
                    items.append(
                        {
                            "repo": repo,
                            "number": number,
                            "kind": "kaneo-open-gh-closed",
                            "kaneo_status": status,
                            "github_state": state,
                        }
                    )
            report["c_kaneo_mismatches"] = {
                "linked": len(links),
                "count": len(items),
                "kaneo_done_gh_open": done_open,
                "kaneo_open_gh_closed": open_closed,
                "items": items,
            }
        except Exception as exc:
            report["c_kaneo_mismatches"] = {"error": str(exc)[:300]}
            report["errors"].append(f"kaneo: {exc}"[:300])

    report["known_positive"] = known_positive_status()
    report["fingerprint"] = fingerprint(report)

    # State + report files (state survives restarts; the fingerprint
    # history is what makes epic posting change-only).
    state_file = Path(args.state_file)
    state = {}
    if state_file.exists():
        try:
            state = json.loads(state_file.read_text())
        except Exception:
            state = {}
    report_dir = Path(args.report_dir)
    report_dir.mkdir(parents=True, exist_ok=True)
    (report_dir / "last-report.json").write_text(json.dumps(report, indent=2) + "\n")
    md = render_markdown(report)
    (report_dir / "last-report.md").write_text(md)

    posted = False
    if args.post_epic and (
        args.force_post or state.get("last_posted_fingerprint") != report["fingerprint"]
    ):
        body_file = report_dir / "epic-comment.md"
        body_file.write_text(md)
        gh_run(
            "issue",
            "comment",
            str(EPIC_NUMBER),
            "--repo",
            EPIC_REPO,
            "--body-file",
            str(body_file),
        )
        posted = True
        state["last_posted_fingerprint"] = report["fingerprint"]

    state["last_run"] = run_at
    state["last_fingerprint"] = report["fingerprint"]
    state["mode"] = report["mode"]
    runs = state.setdefault("runs", [])
    runs.append(
        {
            "at": run_at,
            "mode": report["mode"],
            "fingerprint": report["fingerprint"],
            "posted_epic": posted,
            "counts": {
                sec["repo"]: {
                    "a": len(sec["a_closed_open_subs"]),
                    "b": len(sec["b_open_done_label"]),
                }
                for sec in report["repos"]
            },
            "errors": report["errors"],
        }
    )
    state["runs"] = runs[-30:]
    state_file.parent.mkdir(parents=True, exist_ok=True)
    tmp = state_file.with_suffix(".json.tmp")
    tmp.write_text(json.dumps(state, indent=2) + "\n")
    tmp.replace(state_file)

    summary = {
        "run_at": run_at,
        "mode": report["mode"],
        "gh_route": GH_ROUTE,
        "fingerprint": report["fingerprint"],
        "posted_epic": posted,
        "known_positive": report["known_positive"],
        "counts": {
            sec["repo"]: {
                "total": sec["issues_total"],
                "a": len(sec["a_closed_open_subs"]),
                "b": len(sec["b_open_done_label"]),
                "actions": sec["actions"],
            }
            for sec in report["repos"]
        },
        "c_kaneo_mismatches": (
            None
            if report["c_kaneo_mismatches"] is None
            else {k: v for k, v in report["c_kaneo_mismatches"].items() if k != "items"}
        ),
        "errors": report["errors"],
    }
    print(json.dumps(summary, indent=2))
    return 1 if report["errors"] else 0


if __name__ == "__main__":
    sys.exit(main())
