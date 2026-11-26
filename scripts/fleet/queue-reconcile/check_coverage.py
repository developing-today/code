#!/usr/bin/env python3
"""Standing issue<->queue coverage reconciliation (#7016).

The gap this closes: issues enter the fleet queue only when a
creator-class lane files them (file + enqueue in the same pass). Issues
filed any other way — a work lane filing a blocker mid-task, an audit
lane, by hand, or predating the queue (epics) — silently never get a
queue item, so no lane ever dispositions them. The 2026-11-07
reconciliation found 23 such issues across the four fleet repos; the
import that day was a one-off, and this script is the standing version.

The check, per Drew's doctrine (2026-11-07): **every open issue must
hold a LIVE task (queued/claimed/parked/blocked) in its own pilot
queue.** A done task is not coverage (done means triage or a slice, not
a resolved issue), and neither is a task in the shadow queue.

Two halves, deliberately split:

- **The diff** is done by the queue broker (`POST /coverage`), which
  owns the Turso connection. It is read-only and safe to run any time.
- **The import** is done here, through the sanctioned CLI enqueue path,
  so packets are validated and dedupe-keyed exactly like every other
  task. The dedupe key makes a re-run idempotent by construction; when
  the canonical key belongs only to a completed task, a terminal-dedupe
  response is retried with a deterministic dispatch suffix so the still-
  uncovered issue receives a new live task.

Default is observe-only: the run reports what it *would* import and
writes nothing. Pass `--import` to actually enqueue.

Usage:
    check_coverage.py [--observe | --import] [--repo owner/name ...]
                      [--broker-url URL] [--token-file PATH]
                      [--state-dir DIR] [--now ISO8601]
                      [--fixture JSON]

`--fixture` replaces both the GitHub fetch and the broker call with a
JSON file shaped `{"issues": {"owner/repo": [...]}, "coverage":
{"owner/repo": {"uncovered_issues": [...]}}}`, so the whole decision
path runs offline in tests.

Open-PR coverage (#4951): the broker diff only knows about tasks, so
an issue whose fix task completed `pr_opened` reads as uncovered and
was re-triaged while its PR bit-rotted. Before declaring an issue
uncovered, this script also consults the repo's OPEN PRs (branch
`queue/<task>-<issue>-*`, a closing keyword, or a trailing `(#n)`
title reference). An issue with an open fix PR is in-flight, not
ownerless: it gets no triage packet. In `--import` mode it gets one
dedupe-keyed merge-watch packet per PR instead, whose job is the
~24h recheck the queue otherwise never does — re-test mergibility,
refresh the branch, merge, or close with a salvage note. Fixtures
carry the PR set under a top-level `"pulls": {"owner/repo": [...]}`.

Ready-PR shepherd fallback (#12280): the stale-PR warden is the
merge sweep, but a sweep that depends on one scheduled job fails
silently when that job stops running — exactly what happened in
November 2026, when the warden's live job vanished in an environment
recovery and ready PRs sat unmerged for days. So this reconciliation
also diffs the repo's ready (non-draft) PRs against live queue tasks:
the broker `/coverage` call carries the PR numbers and answers which
of them a LIVE task names. Merge-watch and ready-PR shepherd packets
use the same canonical key, `repo-health:v1:pr:<repo>:pr:<n>`, so the
warden and reconciliation dedupe onto one live task whichever files
first. In `--import` mode, any ready PR older than 6 hours with no
live task naming it gets one `pr_shepherd` packet under that key. A PR
already receiving a merge-watch packet this run is not double-filed. Fixtures carry the
broker's PR answer as `"uncovered_pulls": [{"number": n}]` inside
each repo's coverage object; a fixture without it treats every PR as
uncovered (the pre-#12280 fixture semantics).

Verdict coverage (#12233 F2b, decision record on the issue): an open
`effort/l` or `effort/m` issue whose newest agent note records a
triage verdict with a valid resume condition
(`resume=<slice_landed|dependency_closed>:<number>`) is parked by
that verdict — the note is the park record for the triage
disposition — so the broker calling it uncovered does not get it
re-enqueued for triage. The lane-guard `coverage` check
(`pm-stack/lane-guard/lane_guard.py`) applies the same rules to a
single issue. Fixtures carry the fields on the issue objects:
`labels`, `verdict`, `resume_condition`. In live mode the labels
come with the enumeration and the verdict is read from the issue's
comments, for broker-uncovered effort issues only.

The coverage is released when the verdict stops parking the issue:
the resume condition is evaluated against live state (a closed
dependency, a merged slice), and a label, commit or body change
after the verdict ends it — the same freshness rule the lane
guard applies. A verdict that cannot be read (a failed comments
fetch) is never treated as absent: the issue is skipped from the
import, reported as an error, and the run exits 3.

Exit codes: 0 = run completed (uncovered may be > 0 in observe mode),
2 = configuration/usage error, 3 = a fetch or broker call failed (the
report names which; a failure never masquerades as "0 uncovered").
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

# The four pilot queues and the repo each one covers. A repo outside this
# map is not reconciled — the pilot is scoped to these four by design
# (queue-worker-pilot-design-2026-10-25 v1.1).
PILOT_QUEUES: dict[str, str] = {
    "repo-health-code-pilot": "developing-today/code",
    "repo-health-opencode-pilot": "developing-today-forks/opencode",
    "repo-health-openchamber-pilot": "developing-today-forks/openchamber",
    "repo-health-nix-pilot": "dezren39/darwin-nix",
}

DEFAULT_STATE_DIR = Path.home() / ".local/state/queue-reconcile"
DEFAULT_TOKEN_FILE = Path.home() / ".config/queue-broker/token"
DEFAULT_BROKER_URL = "http://100.99.113.103:4801"
DEFAULT_BACKLOG = Path.home() / "workspace/code-backlog.md"


class FetchError(RuntimeError):
    """A GitHub or broker call failed; the run must not report success."""


def now_iso(override: str | None = None) -> str:
    if override:
        return override
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


# ---------------------------------------------------------------------------
# GitHub: enumerate open issues (PRs excluded)
# ---------------------------------------------------------------------------


def fetch_open_issues(repo: str, gh: str = "gh") -> list[dict]:
    """All open issues in `repo`, newest first, PRs filtered out.

    Uses `gh api` with `--paginate` so the result is complete rather
    than the first page — a partial enumeration would read as coverage
    that does not exist.
    """
    cmd = [
        gh,
        "api",
        "--paginate",
        f"repos/{repo}/issues?state=open&per_page=100",
        "--jq",
        '.[] | select(has("pull_request") | not) | '
        "{number, title, labels: [.labels[].name]}",
    ]
    try:
        out = subprocess.run(cmd, capture_output=True, text=True, timeout=180)
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise FetchError(f"github enumeration failed for {repo}: {exc}") from exc
    if out.returncode != 0:
        raise FetchError(
            f"github enumeration failed for {repo} (exit {out.returncode}): "
            f"{out.stderr.strip()[:400]}"
        )
    issues: list[dict] = []
    for line in out.stdout.splitlines():
        line = line.strip()
        if not line:
            continue
        issues.append(json.loads(line))
    return issues


# ---------------------------------------------------------------------------
# GitHub: enumerate open PRs and map them to the issues they fix (#4951)
# ---------------------------------------------------------------------------

_BRANCH_ISSUE = re.compile(r"^queue/\d+-(\d+)(?:-|$)")
_CLOSING_REF = re.compile(r"(?i)\b(?:close[sd]?|fix(?:e[sd])?|resolve[sd]?)\s+#(\d+)")
_TITLE_REF = re.compile(r"\(#(\d+)\)\s*$")


def fetch_open_prs(repo: str, gh: str = "gh") -> list[dict]:
    """All open PRs in `repo`, with the fields the issue map needs.

    A failed or partial enumeration must never read as "no open PRs"
    (that would re-triage issues whose PRs are in flight), so any
    failure is a FetchError, exactly like the issue enumeration.
    """
    cmd = [
        gh,
        "api",
        "--paginate",
        f"repos/{repo}/pulls?state=open&per_page=100",
        "--jq",
        '.[] | {number, title, body: ((.body // "") | .[0:2000]), head: {ref: .head.ref}, draft: .draft, created_at: .created_at}',
    ]
    try:
        out = subprocess.run(cmd, capture_output=True, text=True, timeout=180)
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise FetchError(f"github PR enumeration failed for {repo}: {exc}") from exc
    if out.returncode != 0:
        raise FetchError(
            f"github PR enumeration failed for {repo} (exit {out.returncode}): "
            f"{out.stderr.strip()[:400]}"
        )
    prs: list[dict] = []
    for line in out.stdout.splitlines():
        line = line.strip()
        if not line:
            continue
        prs.append(json.loads(line))
    return prs


def pr_issue_numbers(pr: dict) -> set[int]:
    """Issue numbers an open PR is a fix for, from fleet-strong signals:
    the `queue/<task>-<issue>-*` branch namespace, a closing keyword in
    the body (Closes/Fixes/Resolves #n), or a trailing `(#n)` in the
    title. A bare `#n` mention in prose is NOT a signal — it would
    suppress triage for issues a PR merely discusses."""
    numbers: set[int] = set()
    ref = (pr.get("head") or {}).get("ref") or ""
    m = _BRANCH_ISSUE.match(ref)
    if m:
        numbers.add(int(m.group(1)))
    for m in _CLOSING_REF.finditer(pr.get("body") or ""):
        numbers.add(int(m.group(1)))
    m = _TITLE_REF.search(pr.get("title") or "")
    if m:
        numbers.add(int(m.group(1)))
    return numbers


def build_pr_coverage(prs: list[dict]) -> dict[int, list[dict]]:
    """Issue number -> all open PRs covering it, in PR enumeration order."""
    coverage: dict[int, list[dict]] = {}
    for pr in prs:
        info = {
            "number": pr["number"],
            "title": pr.get("title") or "",
            "branch": ((pr.get("head") or {}).get("ref") or ""),
        }
        for n in pr_issue_numbers(pr):
            coverage.setdefault(n, []).append(info)
    return coverage


# ---------------------------------------------------------------------------
# Verdict coverage (#12233 F2b): a recorded verdict parks a large issue
# ---------------------------------------------------------------------------

# The rules in this section are the F2b decision record on #12233 and
# are kept in step with pm-stack/lane-guard/lane_guard.py, whose
# `coverage` check applies them to a single issue: an open effort/l
# or effort/m issue whose newest agent note is a triage verdict with
# a recorded verdict token and a valid resume condition is covered.
VERDICT_EFFORT_LABELS = frozenset({"effort/l", "effort/m"})
RESUME_KINDS = frozenset({"slice_landed", "dependency_closed"})
NON_VERDICTS = frozenset({"none", "unknown", "untriaged", "needs_triage"})

_RESUME_VALUE_RE = re.compile(r"([A-Za-z][A-Za-z_ -]*?)\s*(?::|#|\s)\s*#?(\d+)\b")
_AGENT_NOTE_MARKER = re.compile(r"agent-note[:-]v1|\*\*Agent note\*\*", re.IGNORECASE)
_NOTE_STATE_RE = re.compile(r"\bstate\s*[:=]\s*([A-Za-z_-]+)", re.IGNORECASE)
_NOTE_VERDICT_RE = re.compile(r"\bverdict\s*[:=]\s*([A-Za-z0-9][\w -]*)", re.IGNORECASE)
_NOTE_RESUME_RE = re.compile(r"\bresume\s*[:=]\s*([^\n;]+)", re.IGNORECASE)


def _issue_label_names(issue: dict) -> list[str]:
    names: list[str] = []
    for lab in issue.get("labels") or []:
        if isinstance(lab, dict):
            lab = lab.get("name")
        if isinstance(lab, str) and lab:
            names.append(lab)
    return names


def _norm_token(raw: str) -> str:
    return re.sub(r"[^a-z0-9]+", "_", str(raw).lower()).strip("_")


def parse_resume_condition(value) -> str | None:
    """Canonical `kind:target` for a resume condition, or None.

    Accepts the string forms a note or fixture carries
    (`dependency_closed:539`, `slice landed #12543`) and the dict
    form (`{"kind": ..., "target": ...}`). The kind must be a
    checkable event and the target an issue/PR number: a date, a
    person, or a kind with no target is not a resume condition.
    """
    if isinstance(value, dict):
        kind = _norm_token(value.get("kind") or "")
        target = str(value.get("target") or "").lstrip("#")
        if kind in RESUME_KINDS and target.isdigit():
            return f"{kind}:{target}"
        return None
    if not isinstance(value, str):
        return None
    m = _RESUME_VALUE_RE.search(value.strip())
    if not m:
        return None
    kind = _norm_token(m.group(1))
    if kind not in RESUME_KINDS:
        return None
    return f"{kind}:{m.group(2)}"


def verdict_coverage(issue: dict) -> dict | None:
    """{"verdict", "resume"} when the issue's recorded verdict parks
    it under F2b, else None. The issue dict carries `labels`,
    `verdict` and `resume_condition` (fixture fields, or the live
    enrichment from fetch_issue_verdict)."""
    if not (set(_issue_label_names(issue)) & VERDICT_EFFORT_LABELS):
        return None
    verdict = _norm_token(issue.get("verdict") or "")
    if not verdict or verdict in NON_VERDICTS:
        return None
    resume = parse_resume_condition(
        issue.get("resume_condition") or issue.get("resume")
    )
    if not resume:
        return None
    return {"verdict": verdict, "resume": resume}


def verdict_from_comments(comments: list) -> dict:
    """{"verdict", "resume_condition"} read from the newest agent
    note, or {} when that note is not a recorded triage verdict.

    Mirrors the lane-guard freshness rule: the scan stops at the
    newest non-bot comment — a human comment or a newer agent note
    of any other state means the verdict is no longer the newest
    signal, so it does not cover.
    """
    for c in reversed(comments or []):
        body = c.get("body") or ""
        if not _AGENT_NOTE_MARKER.search(body):
            user = c.get("user") or {}
            if user.get("type") == "Bot":
                continue  # bots (CI, sweepers) are not news
            return {}
        head = body[:600]
        m_state = _NOTE_STATE_RE.search(head)
        m_verdict = _NOTE_VERDICT_RE.search(head)
        if (
            not m_state
            or m_state.group(1).lower().replace("-", "_") != "triaged"
            or not m_verdict
        ):
            return {}
        out = {"verdict": _norm_token(m_verdict.group(1))}
        m_resume = _NOTE_RESUME_RE.search(head)
        if m_resume:
            resume = parse_resume_condition(m_resume.group(1))
            if resume:
                out["resume_condition"] = resume
        return out
    return {}


def verdict_from_comments(comments: list) -> dict:
    """{"verdict", "resume_condition"} read from the newest agent
    note, or {} when that note is not a recorded triage verdict.

    Mirrors the lane-guard freshness rule: the scan stops at the
    newest non-bot comment — a human comment or a newer agent note
    of any other state means the verdict is no longer the newest
    signal, so it does not cover. When the note comment carries a
    timestamp it is returned as `verdict_at`, so the caller can
    check the timeline for changes after the verdict.
    """
    for c in reversed(comments or []):
        body = c.get("body") or ""
        if not _AGENT_NOTE_MARKER.search(body):
            user = c.get("user") or {}
            if user.get("type") == "Bot":
                continue  # bots (CI, sweepers) are not news
            return {}
        head = body[:600]
        m_state = _NOTE_STATE_RE.search(head)
        m_verdict = _NOTE_VERDICT_RE.search(head)
        if (
            not m_state
            or m_state.group(1).lower().replace("-", "_") != "triaged"
            or not m_verdict
        ):
            return {}
        out = {"verdict": _norm_token(m_verdict.group(1))}
        m_resume = _NOTE_RESUME_RE.search(head)
        if m_resume:
            resume = parse_resume_condition(m_resume.group(1))
            if resume:
                out["resume_condition"] = resume
        if c.get("created_at"):
            out["verdict_at"] = c["created_at"]
        return out
    return {}


def _parse_ts(value) -> datetime | None:
    """An ISO-8601 timestamp, or None when it cannot be read."""
    try:
        return datetime.fromisoformat(str(value).replace("Z", "+00:00"))
    except (ValueError, TypeError):
        return None


# Timeline event -> the kind of issue change it proves, kept in step
# with lane_guard.changes_from_timeline: a commit referenced on the
# issue, a label change, a body edit. `commented` is deliberately
# absent: comments are judged from the comment list itself, where
# human vs bot is visible.
_TIMELINE_CHANGES = {"labeled": "label", "unlabeled": "label", "edited": "body"}


def changes_from_timeline(events: list) -> list:
    """[{"kind", "at"}] for the issue changes a verdict must not
    outlive. A `referenced` event only counts as a commit when it
    carries a commit_id (a cross-reference from another issue is
    not one)."""
    out = []
    for e in events or []:
        if not isinstance(e, dict):
            continue
        kind = _TIMELINE_CHANGES.get(e.get("event"))
        if e.get("event") == "referenced" and e.get("commit_id"):
            kind = "commit"
        if kind and e.get("created_at"):
            out.append({"kind": kind, "at": e["created_at"]})
    return out


def verdict_fresh(issue: dict) -> bool:
    """True unless a recorded timeline change postdates the verdict.

    The issue carries `verdict_at` (the verdict note's timestamp)
    and `timeline_changes` (from fetch_issue_verdict, or a fixture).
    With neither recorded there is nothing to compare, so the
    verdict stands; an unreadable timestamp never invents staleness.
    """
    verdict_at = _parse_ts(issue.get("verdict_at"))
    if verdict_at is None:
        return True
    for ch in issue.get("timeline_changes") or []:
        at = _parse_ts(ch.get("at") if isinstance(ch, dict) else None)
        if at is not None and at > verdict_at:
            return False
    return True


def fetch_issue_timeline(repo: str, number: int, gh: str = "gh") -> list:
    """The issue changes a verdict must not outlive, from the
    issue timeline. A failed fetch is a FetchError: without the
    timeline the verdict's freshness is unknown, so the caller
    must not treat the verdict as fresh."""
    cmd = [
        gh,
        "api",
        "--paginate",
        f"repos/{repo}/issues/{number}/timeline?per_page=100",
        "--jq",
        ".[] | {event, created_at, commit_id}",
    ]
    try:
        out = subprocess.run(cmd, capture_output=True, text=True, timeout=180)
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise FetchError(f"github timeline failed for {repo}#{number}: {exc}") from exc
    if out.returncode != 0:
        raise FetchError(
            f"github timeline failed for {repo}#{number} (exit {out.returncode}): "
            f"{out.stderr.strip()[:400]}"
        )
    events: list[dict] = []
    for line in out.stdout.splitlines():
        line = line.strip()
        if line:
            events.append(json.loads(line))
    return changes_from_timeline(events)


def fetch_issue_verdict(repo: str, number: int, gh: str = "gh") -> dict:
    """The recorded verdict on one issue, from its comments.

    A failed fetch is a FetchError, exactly like the enumerations:
    an unread issue must never read as verdict-covered (which would
    suppress triage) or silently as verdict-free. When a verdict is
    found, its timeline is fetched too and returned as
    `timeline_changes`, so the caller can end coverage on a label,
    commit or body change after the verdict (verdict_fresh).
    """
    cmd = [
        gh,
        "api",
        "--paginate",
        f"repos/{repo}/issues/{number}/comments?per_page=100",
        "--jq",
        ".[] | {body, created_at, user: {type: .user.type}}",
    ]
    try:
        out = subprocess.run(cmd, capture_output=True, text=True, timeout=180)
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise FetchError(f"github comments failed for {repo}#{number}: {exc}") from exc
    if out.returncode != 0:
        raise FetchError(
            f"github comments failed for {repo}#{number} (exit {out.returncode}): "
            f"{out.stderr.strip()[:400]}"
        )
    comments: list[dict] = []
    for line in out.stdout.splitlines():
        line = line.strip()
        if line:
            comments.append(json.loads(line))
    verdict = verdict_from_comments(comments)
    if verdict:
        verdict["timeline_changes"] = fetch_issue_timeline(repo, number, gh)
    return verdict


# ---------------------------------------------------------------------------
# Resume conditions (#12233 F2b): is the verdict's wait over?
# ---------------------------------------------------------------------------
#
# Decision (FDR-0013: decide, record, continue — no owner gate): a
# resume condition is fulfilled by live GitHub state, checked at
# reconciliation time. `dependency_closed:N` is fulfilled iff issue
# N is closed. `slice_landed:N` is fulfilled iff PR N is merged; a
# PR closed without merging did not land the slice, so it does not
# release the verdict. A target whose state cannot be read is
# unknown, never fulfilled: the issue stays covered and the failure
# is reported (exit 3), so a transient outage cannot re-triage a
# parked issue or park one whose wait is over.


def resume_fulfilled(resume: str, state: str | None) -> bool | None:
    """True/False when `state` settles the resume condition, None
    when the state is unknown or unrecognised."""
    kind, _, _ = resume.partition(":")
    if kind == "dependency_closed":
        if state == "closed":
            return True
        if state == "open":
            return False
        return None
    if kind == "slice_landed":
        if state == "merged":
            return True
        if state in ("open", "closed"):
            return False
        return None
    return None


def fetch_resume_state(repo: str, resume: str, gh: str = "gh") -> str:
    """The live state of a resume condition's target: an issue's
    state, or a PR's state with "merged" when it has a merge
    timestamp. A failed fetch is a FetchError."""
    kind, _, target = resume.partition(":")
    if kind == "dependency_closed":
        cmd = [gh, "api", f"repos/{repo}/issues/{target}", "--jq", ".state"]
    elif kind == "slice_landed":
        cmd = [
            gh,
            "api",
            f"repos/{repo}/pulls/{target}",
            "--jq",
            "{merged: (.merged_at != null), state: .state}",
        ]
    else:
        raise FetchError(f"unknown resume kind in {resume!r}")
    try:
        out = subprocess.run(cmd, capture_output=True, text=True, timeout=180)
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise FetchError(
            f"github resume target failed for {repo} {resume}: {exc}"
        ) from exc
    if out.returncode != 0:
        raise FetchError(
            f"github resume target failed for {repo} {resume} "
            f"(exit {out.returncode}): {out.stderr.strip()[:400]}"
        )
    if kind == "dependency_closed":
        # `--jq .state` emits the raw state word (`closed` / `open`),
        # not JSON — read it as stripped text. Decoding it as JSON
        # raised on every live lookup and left closed dependencies
        # classified unknown.
        return out.stdout.strip()
    try:
        value = json.loads(out.stdout.strip())
    except json.JSONDecodeError as exc:
        raise FetchError(
            f"github resume target failed for {repo} {resume}: unreadable reply"
        ) from exc
    if kind == "slice_landed":
        return "merged" if value.get("merged") else str(value.get("state") or "")
    return str(value or "")


def resolve_resume(
    repo: str, resume: str, states: dict | None = None, gh: str = "gh"
) -> bool | None:
    """True when the resume condition is fulfilled, False when its
    target is known not to be, None when fulfilment is unknown.
    `states` is the fixture-mode map of canonical resume condition
    to target state (`"closed"` / `"open"` for a dependency,
    `"merged"` / `"open"` / `"closed"` for a slice); with no map the
    target is read live. A resume condition absent from the map is
    unknown, never assumed fulfilled."""
    if states is not None:
        if resume not in states:
            return None
        return resume_fulfilled(resume, states[resume])
    return resume_fulfilled(resume, fetch_resume_state(repo, resume, gh))


# ---------------------------------------------------------------------------
# Broker: the coverage diff
# ---------------------------------------------------------------------------


def broker_coverage(
    broker_url: str,
    token: str,
    queue: str,
    repo: str,
    issues: list[dict],
    pulls: list[dict] | None = None,
) -> dict:
    request: dict = {"queue": queue, "repo": repo, "issues": issues}
    if pulls is not None:
        # #12280: the broker answers PR coverage (covered_pulls /
        # uncovered_pulls) for the PR numbers it is given, under the
        # same live-task rule as issues.
        request["pulls"] = [{"number": pr["number"]} for pr in pulls]
    body = json.dumps(request).encode()
    req = urllib.request.Request(
        broker_url.rstrip("/") + "/coverage",
        data=body,
        headers={"X-Agent-Token": token, "Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=120) as resp:
            payload = json.loads(resp.read())
    except (urllib.error.URLError, OSError, json.JSONDecodeError) as exc:
        raise FetchError(f"broker /coverage failed for {queue}: {exc}") from exc
    if not payload.get("ok"):
        raise FetchError(
            f"broker /coverage rejected {queue}: {payload.get('error', payload)}"
        )
    return payload


# ---------------------------------------------------------------------------
# Import: one triage packet per uncovered issue, dedupe-keyed
# ---------------------------------------------------------------------------


def triage_packet(repo: str, issue: dict, checked_at: str) -> dict:
    number = issue["number"]
    title = issue.get("title") or f"issue {number}"
    url = f"https://github.com/{repo}/issues/{number}"
    return {
        "title": f"Triage {repo}#{number}: {title}"[:180],
        "body": (
            f"Imported by the standing queue coverage reconciliation (#7016): "
            f"{url} is open with no live task in its pilot queue."
        ),
        "priority": 30,
        "dedupe_key": f"repo-health:v1:triage:{repo}:issue:{number}",
        "payload": {
            "schema": "repo-task/v1",
            "kind": "triage",
            "repo": repo,
            "queue": None,  # filled by the caller with the pilot queue
            "issue": {
                "number": number,
                "url": url,
                "title": title,
                "state_checked_at": checked_at,
            },
            "problem": (
                "No live queue task covers this open issue: it was filed outside a "
                "creator run (or predates the queue), so no lane would ever "
                "disposition it. Filed by the standing coverage reconciliation "
                "(#7016)."
            ),
            "evidence": [
                f"Coverage reconciliation {checked_at}: issue #{number} is open with no "
                "queued/claimed/parked/blocked task in its pilot queue",
                url,
            ],
            "acceptance": [
                "Issue triaged per the standing triage doctrine: annotate, validate, and "
                "either fix it in a small edit (merge with a closing keyword) or return it "
                "to the queue for real work",
            ],
            "tests": [
                "Triage verdict recorded on the issue; if a small fix, the merged PR "
                "carries a closing keyword",
            ],
            "scope": {
                "include": [f"Triage of {repo}#{number}"],
                "exclude": ["Secrets, credentials, purchases, or Drew-only decisions"],
            },
            "risk": {"level": "low", "flags": ["reconciliation-import"]},
            "priority_reason": (
                "Normal (30): imported by coverage reconciliation, not filed by a creator run."
            ),
            "timebox_minutes": 45,
            "max_attempts": 1000,  # #8973: new-task default (Drew order 2026-11-12)
            "context_refs": [url],
            "dedupe": {
                "checked_at": checked_at,
                "basis": "Coverage reconciliation (#7016): no live task held this issue at check time.",
            },
        },
    }


def merge_watch_packet(repo: str, issue: dict, pr: dict, checked_at: str) -> dict:
    """The ~24h merge-watch recheck for work that ended `pr_opened` (#4951).

    One packet per open PR, dedupe-keyed on the PR, so re-runs of the
    reconciliation never stack duplicates while a watch is live. The
    packet shepherds the EXISTING PR — it never re-implements the fix.
    """
    number = issue["number"]
    pr_number = pr["number"]
    issue_url = f"https://github.com/{repo}/issues/{number}"
    pr_url = f"https://github.com/{repo}/pull/{pr_number}"
    return {
        "title": f"Merge-watch {repo}#{pr_number}: recheck fix PR for issue #{number}"[
            :180
        ],
        "body": (
            f"Merge-watch recheck (#4951): {pr_url} is the open fix PR for {issue_url}, "
            "whose fix task completed pr_opened with no live task watching the PR. "
            "Re-test mergibility against current main; refresh the branch if the fix "
            "is still desired, merge it when verified, or close it with a salvage "
            "note if main has superseded it."
        ),
        "priority": 30,
        # Share the warden's canonical key so all shepherd paths dedupe
        # onto one live task per PR and the broker's coverage matcher sees it.
        "dedupe_key": f"repo-health:v1:pr:{repo}:pr:{pr_number}",
        "payload": {
            "schema": "repo-task/v1",
            "kind": "pr_shepherd",
            "repo": repo,
            "queue": None,  # filled by the caller with the pilot queue
            "prs": [
                {"number": pr_number, "url": pr_url, "title": pr.get("title") or ""}
            ],
            "issue": {
                "number": number,
                "url": issue_url,
                "title": issue.get("title") or f"issue {number}",
                "state_checked_at": checked_at,
            },
            "pr": {
                "number": pr_number,
                "url": pr_url,
                "title": pr.get("title") or f"PR {pr_number}",
                "branch": pr.get("branch") or "",
                "state_checked_at": checked_at,
            },
            "problem": (
                "A task completed with outcome pr_opened leaves the live set, so "
                "nothing rechecks whether its PR still merges. Coverage "
                "reconciliation (#4951) found this PR open with no live task."
            ),
            "evidence": [
                f"Coverage reconciliation {checked_at}: issue #{number} holds no live "
                f"task but open PR #{pr_number} ({pr.get('branch') or 'branch unknown'}) "
                "is its in-flight fix",
                pr_url,
            ],
            "acceptance": [
                "Mergibility re-tested against current main (git merge-tree or the PR's mergeable state) and the result recorded on the task/PR",
                "Branch refreshed (rebase/conflict resolution) if the fix is still desired, then merged when checks are green and review findings are dispositioned",
                "OR the PR is closed with a salvage note per protocol §8 (diff against main first; 'nothing unique' counts) if main has superseded it",
                "The fix is NOT re-implemented by a fresh task",
            ],
            "tests": [
                "Merge-tree / mergeable verdict recorded; if merged, the PR's closing keyword disposition of the issue is verified",
            ],
            "scope": {
                "include": [
                    f"Shepherd existing PR {repo}#{pr_number} for issue #{number}"
                ],
                "exclude": [
                    "Re-implementing the fix",
                    "Unrelated issues, pull requests, or repositories",
                ],
            },
            "risk": {"level": "low", "flags": ["merge-watch"]},
            "priority_reason": (
                "Normal (30): merge-watch recheck for pr_opened work, filed by "
                "coverage reconciliation (#4951)."
            ),
            "timebox_minutes": 45,
            "max_attempts": 1000,
            "context_refs": [issue_url, pr_url],
            "dedupe": {
                "checked_at": checked_at,
                "basis": "Merge-watch (#4951): open fix PR with no live task at check time; one watch per PR.",
            },
        },
    }


def shepherd_packet(repo: str, pr: dict, checked_at: str) -> dict:
    """One `pr_shepherd` packet for a ready PR no live task names (#12280).

    Keyed with the stale-PR warden's own dedupe key
    (`repo-health:v1:pr:<repo>:pr:<n>`), so this fallback and the
    warden dedupe onto a single live task whichever files first. The
    done rule is the warden's, stated in the packet: the task is done
    only when the PR is merged or closed with a reason.
    """
    pr_number = pr["number"]
    pr_url = f"https://github.com/{repo}/pull/{pr_number}"
    title = pr.get("title") or f"PR {pr_number}"
    return {
        "title": f"Shepherd ready PR #{pr_number}: {title}"[:180],
        "body": (
            f"Filed by the standing queue coverage reconciliation (#12280 "
            f"fallback): {pr_url} is a ready (non-draft) PR older than 6 hours "
            "with no live task naming it — the fallback for when the stale-PR "
            "warden has not run. Shepherd it to a finish: answer or fix every "
            "review finding, re-check mergeability against current main, "
            "refresh the branch if the change is still desired, and merge it "
            "when verified — or close it with a reason (salvage note per "
            "protocol §8) if main has superseded it. This task is done only "
            "when the PR is merged or closed with a reason recorded."
        ),
        "priority": 40,
        "dedupe_key": f"repo-health:v1:pr:{repo}:pr:{pr_number}",
        "payload": {
            "schema": "repo-task/v1",
            "kind": "pr_shepherd",
            "repo": repo,
            "queue": None,  # filled by the caller with the pilot queue
            # PR references, not bare numbers: the queue validator
            # requires every prs element to be an object with a
            # positive-integer `number` (#5549).
            "prs": [{"number": pr_number, "url": pr_url, "title": title}],
            "pr": {
                "number": pr_number,
                "url": pr_url,
                "title": title,
                "branch": pr.get("branch") or ((pr.get("head") or {}).get("ref") or ""),
                "state_checked_at": checked_at,
            },
            "problem": (
                "A ready PR with no live task naming it has no owner for its "
                "review findings or its merge, so it sits until the branch "
                "drifts into conflict. The stale-PR warden normally files "
                "this task; the coverage reconciliation is its fallback "
                "(#12280)."
            ),
            "evidence": [
                f"Coverage reconciliation {checked_at}: PR #{pr_number} is "
                "ready (non-draft), older than 6 hours, and no live task "
                "names it",
                pr_url,
            ],
            "acceptance": [
                "Every review finding on the PR is answered or fixed",
                "Mergeability re-checked against current main immediately before merging",
                "The PR is merged when verified, OR closed with a reason recorded (salvage note per protocol §8 if superseded)",
                "The PR's change is NOT re-implemented by a fresh task",
            ],
            "tests": [
                "Merge / mergeable verdict recorded on the task; if merged, the PR's closing-keyword disposition of its issue is verified",
            ],
            "scope": {
                "include": [
                    f"Shepherd existing PR {repo}#{pr_number} to merged or closed-with-reason"
                ],
                "exclude": [
                    "Re-implementing the PR's change",
                    "Unrelated issues, pull requests, or repositories",
                ],
            },
            "risk": {"level": "low", "flags": ["pr-shepherd-fallback"]},
            "priority_reason": (
                "Warden parity (40): a ready PR older than 6 hours with no "
                "live owner, filed by coverage reconciliation (#12280)."
            ),
            "timebox_minutes": 45,
            "max_attempts": 4,
            "context_refs": [pr_url],
            "dedupe": {
                "checked_at": checked_at,
                "basis": "Ready-PR shepherd fallback (#12280): ready PR older than 6h with no live task naming it at check time; warden dedupe key, so the warden and this fallback share one live task per PR.",
            },
        },
    }


# A ready PR becomes a shepherd candidate once it is this old. Six
# hours matches #12280's done-when: long enough that a producing lane
# has had its chance to own the merge, short enough that the branch
# has not drifted far.
SHEPHERD_MIN_AGE_HOURS = 6


def _parse_ts(value) -> datetime | None:
    if value is None:
        return None
    try:
        return datetime.fromisoformat(str(value).replace("Z", "+00:00"))
    except (ValueError, TypeError):
        return None


def ready_shepherd_candidates(
    pulls: list[dict],
    uncovered_pull_numbers: set[int],
    watch_pull_numbers: set[int],
    checked_at: str,
) -> list[dict]:
    """Ready PRs the fallback files a shepherd for (#12280).

    Oracle: the issue's done-when — a `pr_shepherd` task exists for
    each ready PR older than 6 hours, checked by the queue store
    against `gh pr list`. A PR qualifies iff it is non-draft, its
    created_at is at least SHEPHERD_MIN_AGE_HOURS before the check,
    the broker's live-task diff names no live task for it (it is in
    `uncovered_pull_numbers`), and it is not already receiving a
    merge-watch packet this run. A PR whose draft state or age cannot
    be proven from the enumeration is never filed on — the same
    fail-closed rule as the fetch paths.
    """
    now = _parse_ts(checked_at)
    candidates: list[dict] = []
    for pr in pulls:
        number = pr.get("number")
        if number not in uncovered_pull_numbers:
            continue
        if number in watch_pull_numbers:
            continue
        if pr.get("draft") is not False:
            continue
        created = pr.get("created_at")
        if not created:
            continue
        age = now - _parse_ts(created)
        if age.total_seconds() < SHEPHERD_MIN_AGE_HOURS * 3600:
            continue
        candidates.append(pr)
    return candidates


def _enqueue_response(stdout: str) -> dict:
    """Read the broker row JSON after the CLI's human status lines."""
    start = stdout.find("{")
    if start < 0:
        return {}
    try:
        response = json.loads(stdout[start:])
    except json.JSONDecodeError:
        return {}
    return response if isinstance(response, dict) else {}


_TERMINAL_TASK_STATUSES = {"done", "failed", "cancelled"}


def _is_terminal_dedupe_response(response: dict) -> bool:
    """Recognize both broker metadata and the CLI's returned terminal row."""
    dedupe_match = response.get("dedupe_match") or {}
    return (
        dedupe_match.get("rule") == "dedupe_key_terminal"
        or response.get("status") in _TERMINAL_TASK_STATUSES
    )


def _terminal_retry_key(packet: dict) -> str | None:
    """Give a previously completed packet one new, deterministic key.

    Coverage imports are keyed canonically per issue/PR. A completed row
    still owns that key but is not live coverage; the queue explicitly
    instructs producers to suffix the key for a later dispatch. Using the
    packet's checked_at timestamp makes two workers in the same pass
    dedupe together while allowing a later reconciliation pass to requeue
    the still-open item.
    """
    key = packet.get("dedupe_key")
    payload = packet.get("payload") or {}
    subject = payload.get("issue") or payload.get("pr") or {}
    repo = payload.get("repo")
    number = subject.get("number")
    dedupe = payload.get("dedupe") or {}
    checked_at = dedupe.get("checked_at") or subject.get("state_checked_at")
    if (
        not isinstance(key, str)
        or ":dispatch:" in key
        or not isinstance(repo, str)
        or not isinstance(number, int)
        or not isinstance(checked_at, str)
    ):
        return None
    seed = f"{key}:{repo}:{number}:{checked_at}".encode("utf-8")
    suffix = hashlib.sha256(seed).hexdigest()[:8]
    return f"{key}:dispatch:{suffix}"


def enqueue_packet(
    packet: dict,
    queue: str,
    fleet: str,
    broker_url: str,
    token_file: str,
) -> tuple[bool, str]:
    """Enqueue via the sanctioned CLI path. Returns (ok, detail).

    The child gets the same broker URL and token file this run selected
    (`--broker-url` / `--token-file`), so it cannot enqueue into a
    different broker than the one whose coverage was read. The packet
    file is unique per enqueue: an ad-hoc run overlapping the scheduled
    one must never overwrite or delete the other's packet.
    """
    def run_once(candidate: dict) -> subprocess.CompletedProcess:
        payload = candidate["payload"]
        repo_slug = payload["repo"].replace("/", "-")
        # Issue packets key the temp file on the issue; PR packets
        # (merge-watch, shepherd) key it on the PR.
        number = (payload.get("issue") or payload["pr"])["number"]
        fd, tmp_name = tempfile.mkstemp(
            prefix=f"coverage-{repo_slug}-{number}-",
            suffix=".json",
            dir=os.environ.get("TMPDIR") or None,
        )
        tmp = Path(tmp_name)
        try:
            with os.fdopen(fd, "w") as fh:
                json.dump(candidate, fh)
            env = os.environ.copy()
            env["REPO_QUEUE_BROKER_URL"] = broker_url
            env["REPO_QUEUE_TOKEN_FILE"] = str(token_file)
            return subprocess.run(
                [fleet, "queue", "enqueue", str(tmp), "--queue", queue],
                capture_output=True,
                text=True,
                timeout=120,
                env=env,
            )
        finally:
            tmp.unlink(missing_ok=True)

    try:
        out = run_once(packet)
    except (OSError, subprocess.TimeoutExpired) as exc:
        # A launch failure or timeout is an enqueue failure, not a
        # reason to abandon the remaining issues and the report.
        return False, f"could not run enqueue: {exc}"
    if out.returncode != 0:
        return False, (out.stderr or out.stdout).strip()[:300]

    response = _enqueue_response(out.stdout)
    if not _is_terminal_dedupe_response(response):
        return True, out.stdout.strip()[:300]

    retry_key = _terminal_retry_key(packet)
    if retry_key is None:
        return False, (
            "terminal dedupe hit, but packet has no safe dispatch suffix: "
            f"{out.stdout.strip()[:240]}"
        )
    retry_packet = {**packet, "dedupe_key": retry_key}
    try:
        retry = run_once(retry_packet)
    except (OSError, subprocess.TimeoutExpired) as exc:
        return False, f"terminal dedupe retry could not run: {exc}"
    if retry.returncode != 0:
        return False, (retry.stderr or retry.stdout).strip()[:300]
    if _is_terminal_dedupe_response(_enqueue_response(retry.stdout)):
        return False, (
            "dispatch-suffixed terminal dedupe retry was not filed: "
            f"{retry.stdout.strip()[:240]}"
        )
    return True, retry.stdout.strip()[:300]


# ---------------------------------------------------------------------------
# Report
# ---------------------------------------------------------------------------


def render_report(
    checked_at: str,
    mode: str,
    results: list[dict],
    errors: list[str],
) -> str:
    lines = [
        f"# Queue coverage reconciliation — {checked_at}",
        "",
        f"Mode: **{mode}**. Doctrine: every open issue must hold a LIVE task "
        "(queued/claimed/parked/blocked) in its own pilot queue; a done task is not coverage, "
        "but a recorded verdict with a resume condition is parked coverage (#12233 F2b).",
        "",
        "| repo | queue | open issues | covered | open-PR covered | verdict covered | uncovered | imported | watch imported |",
        "|---|---|---|---|---|---|---|---|---|",
    ]
    for r in results:
        lines.append(
            f"| {r['repo']} | {r['queue']} | {r['open_issues']} | {r['covered']} "
            f"| {r.get('pr_covered', 0)} | {r.get('verdict_covered', 0)} "
            f"| {r['uncovered']} | {r['imported']} "
            f"| {r.get('watch_imported', 0)} |"
        )
    total_uncovered = sum(r["uncovered"] for r in results)
    total_imported = sum(r["imported"] for r in results)
    total_pr = sum(r.get("pr_covered", 0) for r in results)
    total_verdict = sum(r.get("verdict_covered", 0) for r in results)
    total_watch = sum(r.get("watch_imported", 0) for r in results)
    total_shepherd = sum(len(r.get("shepherd_candidates", [])) for r in results)
    total_shepherd_imported = sum(r.get("shepherd_imported", 0) for r in results)
    total_unread = sum(len(r.get("verdict_unread_issues", [])) for r in results)
    total_fulfilled = sum(len(r.get("resume_fulfilled_issues", [])) for r in results)
    total_stale = sum(len(r.get("stale_issues", [])) for r in results)
    lines += [
        "",
        f"**Total uncovered: {total_uncovered}** · imported this run: {total_imported}",
        f"Open-PR covered (#4951): {total_pr} · merge-watch imported this run: {total_watch}",
        f"Ready-PR shepherd candidates (#12280): {total_shepherd} · shepherd imported this run: {total_shepherd_imported}",
        f"Verdict covered (#12233 F2b): {total_verdict}",
        f"Verdict unread (fetch failed — skipped from import): {total_unread} "
        f"· resume condition fulfilled (due for re-triage): {total_fulfilled} "
        f"· verdict stale (changed after the verdict): {total_stale}",
        "",
    ]
    if errors:
        lines += ["## ERRORS", ""] + [f"- {e}" for e in errors] + [""]
    unread_any = [(r, i) for r in results for i in r.get("verdict_unread_issues", [])]
    if unread_any:
        lines += [
            "## Verdict unread — fetch failed, skipped from import (#12233 F2b)",
            "",
        ]
        for r, i in unread_any:
            lines.append(
                f"- {r['repo']}#{i['number']} — the recorded verdict could not be "
                "read, so no triage packet was enqueued for it"
            )
        lines.append("")
    fulfilled_any = [
        (r, i) for r in results for i in r.get("resume_fulfilled_issues", [])
    ]
    if fulfilled_any:
        lines += [
            "## Resume condition fulfilled — due for re-triage (#12233 F2b)",
            "",
        ]
        for r, i in fulfilled_any:
            lines.append(
                f"- {r['repo']}#{i['number']} — verdict {i.get('verdict')} "
                f"(resume {i.get('resume')} fulfilled)"
            )
        lines.append("")
    stale_any = [(r, i) for r in results for i in r.get("stale_issues", [])]
    if stale_any:
        lines += [
            "## Verdict stale — issue changed after the verdict (#12233 F2b)",
            "",
        ]
        for r, i in stale_any:
            lines.append(
                f"- {r['repo']}#{i['number']} — verdict {i.get('verdict')} "
                "(a label, commit or body change postdates the verdict)"
            )
        lines.append("")
    pr_any = [(r, i) for r in results for i in r.get("pr_covered_issues", [])]
    if pr_any:
        lines += [
            "## Covered by an open PR — merge-watch, no triage packet (#4951)",
            "",
        ]
        for r, i in pr_any:
            for pr in i.get("prs", []):
                lines.append(
                    f"- {r['repo']}#{i['number']} — PR #{pr.get('number')} "
                    f"({pr.get('branch') or 'branch unknown'})"
                )
        lines.append("")
    verdict_any = [(r, i) for r in results for i in r.get("verdict_covered_issues", [])]
    if verdict_any:
        lines += [
            "## Covered by a recorded verdict — parked, no triage packet (#12233 F2b)",
            "",
        ]
        for r, i in verdict_any:
            lines.append(
                f"- {r['repo']}#{i['number']} — verdict {i.get('verdict')} "
                f"(resume {i.get('resume')})"
            )
        lines.append("")
    uncovered_any = [(r, i) for r in results for i in r.get("uncovered_issues", [])]
    if uncovered_any:
        lines += ["## Uncovered issues", ""]
        for r, i in uncovered_any:
            lines.append(f"- {r['repo']}#{i['number']} — {i.get('title', '')}")
        lines.append("")
    shepherd_any = [(r, p) for r in results for p in r.get("shepherd_candidates", [])]
    if shepherd_any:
        lines += [
            "## Ready PRs with no live task — shepherd candidates (#12280)",
            "",
        ]
        for r, p in shepherd_any:
            lines.append(f"- {r['repo']}#{p['number']} — {p.get('title', '')}")
        lines.append("")
    return "\n".join(lines)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    mode = ap.add_mutually_exclusive_group()
    mode.add_argument("--observe", action="store_true", help="report only (default)")
    mode.add_argument(
        "--import",
        dest="do_import",
        action="store_true",
        help="enqueue uncovered issues",
    )
    ap.add_argument(
        "--repo", action="append", help="restrict to these repos (repeatable)"
    )
    ap.add_argument(
        "--broker-url",
        default=os.environ.get("REPO_QUEUE_BROKER_URL", DEFAULT_BROKER_URL),
    )
    ap.add_argument("--token-file", default=str(DEFAULT_TOKEN_FILE))
    ap.add_argument("--state-dir", default=str(DEFAULT_STATE_DIR))
    ap.add_argument("--backlog", default=str(DEFAULT_BACKLOG))
    ap.add_argument("--fleet", default=os.environ.get("FLEET_BIN", "fleet"))
    ap.add_argument("--now", default=None)
    ap.add_argument("--fixture", default=None)
    args = ap.parse_args(argv)

    checked_at = now_iso(args.now)
    do_import = bool(args.do_import)
    mode_name = "import" if do_import else "observe"

    selected = list(PILOT_QUEUES.items())
    if args.repo:
        wanted = set(args.repo)
        selected = [(q, r) for q, r in selected if r in wanted]
        unknown = wanted - {r for _, r in selected}
        if unknown:
            print(f"error: no pilot queue covers {sorted(unknown)}", file=sys.stderr)
            return 2

    fixture = None
    if args.fixture:
        fixture = json.loads(Path(args.fixture).read_text())

    token = ""
    if not fixture:
        try:
            token = Path(args.token_file).read_text().strip()
        except OSError as exc:
            print(
                f"error: cannot read broker token {args.token_file}: {exc}",
                file=sys.stderr,
            )
            return 2
        if not token:
            print(
                f"error: broker token file {args.token_file} is empty", file=sys.stderr
            )
            return 2

    results: list[dict] = []
    errors: list[str] = []
    # Fixture mode only: canonical resume condition -> target state
    # ("closed"/"open" for a dependency, "merged"/"open"/"closed"
    # for a slice), so the resume-fulfilment check runs offline. An
    # absent map is an empty map (unknown, never fulfilled), never a
    # live lookup: fixtures must not touch the network.
    resume_states = (fixture.get("resume_targets") or {}) if fixture else None
    for queue, repo in selected:
        try:
            if fixture:
                issues = fixture["issues"][repo]
                cov = fixture["coverage"][repo]
                pulls = (fixture.get("pulls") or {}).get(repo, [])
            else:
                issues = fetch_open_issues(repo)
                pulls = fetch_open_prs(repo)
                cov = broker_coverage(
                    args.broker_url, token, queue, repo, issues, pulls
                )
        except (FetchError, KeyError) as exc:
            errors.append(str(exc))
            continue

        # A verdict read that failed leaves the issue's verdict
        # unknown: it is skipped from the import below, never
        # treated as verdict-free.
        verdict_read_failed: set[int] = set()
        if not fixture:
            # #12233 F2b: the broker diff cannot see verdicts. For
            # the effort issues it calls uncovered, read the recorded
            # verdict from the issue's comments and merge it into the
            # issue dict the verdict split below reads. A failed read
            # is recorded as an error, never as "no verdict".
            by_number = {i["number"]: i for i in issues}
            for entry in cov.get("uncovered_issues", []):
                issue = by_number.get(entry["number"])
                if issue is None or not (
                    set(_issue_label_names(issue)) & VERDICT_EFFORT_LABELS
                ):
                    continue
                try:
                    issue.update(fetch_issue_verdict(repo, issue["number"]))
                except FetchError as exc:
                    errors.append(str(exc))
                    verdict_read_failed.add(issue["number"])

        # #4951: an issue the broker calls uncovered whose fix PR is
        # still open is in-flight, not ownerless — no triage packet.
        pr_coverage = build_pr_coverage(pulls)
        broker_uncovered = cov.get("uncovered_issues", [])
        uncovered = [i for i in broker_uncovered if i["number"] not in pr_coverage]
        pr_covered = [
            {**i, "prs": pr_coverage[i["number"]]}
            for i in broker_uncovered
            if i["number"] in pr_coverage
        ]

        # #12233 F2b: an issue the broker calls uncovered whose
        # recorded verdict parks it is verdict-covered — the verdict
        # note is the park record, so no triage packet. Two releases
        # end the coverage: a timeline change after the verdict
        # (verdict_fresh), and a fulfilled resume condition
        # (resolve_resume). An unread verdict is neither covered nor
        # uncovered — it is skipped from the import entirely.
        by_number = {i["number"]: i for i in issues}
        verdict_covered: list[dict] = []
        verdict_unread: list[dict] = []
        resume_fulfilled_issues: list[dict] = []
        stale_issues: list[dict] = []
        still_uncovered: list[dict] = []
        for entry in uncovered:
            issue = by_number.get(entry["number"], entry)
            if entry["number"] in verdict_read_failed:
                verdict_unread.append(entry)
                continue
            info = verdict_coverage(issue)
            if not info:
                still_uncovered.append(entry)
                continue
            if not verdict_fresh(issue):
                stale_issues.append({**entry, **info})
                still_uncovered.append(entry)
                continue
            try:
                fulfilled = resolve_resume(repo, info["resume"], resume_states)
            except FetchError as exc:
                errors.append(str(exc))
                fulfilled = None
            if fulfilled:
                resume_fulfilled_issues.append({**entry, **info})
                still_uncovered.append(entry)
            else:
                verdict_covered.append({**entry, **info})
        uncovered = still_uncovered

        imported = 0
        if do_import and uncovered:
            for issue in uncovered:
                packet = triage_packet(repo, issue, checked_at)
                ok, detail = enqueue_packet(
                    packet, queue, args.fleet, args.broker_url, args.token_file
                )
                if ok:
                    imported += 1
                else:
                    errors.append(
                        f"enqueue failed for {repo}#{issue['number']}: {detail}"
                    )

        watch_imported = 0
        if do_import and pr_covered:
            for issue in pr_covered:
                for pr in issue["prs"]:
                    packet = merge_watch_packet(repo, issue, pr, checked_at)
                    ok, detail = enqueue_packet(
                        packet, queue, args.fleet, args.broker_url, args.token_file
                    )
                    if ok:
                        watch_imported += 1
                    else:
                        errors.append(
                            f"merge-watch enqueue failed for {repo}#{pr['number']}: {detail}"
                        )

        # #12280: ready PRs older than 6h with no live task naming
        # them get a warden-keyed shepherd packet. The broker's PR
        # diff is the authority on "live task naming it"; a live-mode
        # answer without a PR section means the broker predates the
        # PR half — an error, never a licence to file blind.
        if fixture:
            fixture_uncovered = cov.get("uncovered_pulls")
            uncovered_pull_numbers = (
                {p["number"] for p in fixture_uncovered}
                if fixture_uncovered is not None
                else {pr["number"] for pr in pulls}
            )
            pr_diff_known = True
        else:
            live_uncovered = cov.get("uncovered_pulls")
            pr_diff_known = live_uncovered is not None
            uncovered_pull_numbers = (
                {p["number"] for p in live_uncovered} if pr_diff_known else set()
            )
            if not pr_diff_known:
                errors.append(
                    f"broker /coverage returned no PR coverage for {queue}: "
                    "ready-PR shepherd fallback skipped for this repo"
                )
        shepherd_candidates = (
            ready_shepherd_candidates(
                pulls,
                uncovered_pull_numbers,
                {pr["number"] for i in pr_covered for pr in i["prs"]},
                checked_at,
            )
            if pr_diff_known
            else []
        )
        shepherd_imported = 0
        if do_import and shepherd_candidates:
            for pr in shepherd_candidates:
                packet = shepherd_packet(repo, pr, checked_at)
                ok, detail = enqueue_packet(
                    packet, queue, args.fleet, args.broker_url, args.token_file
                )
                if ok:
                    shepherd_imported += 1
                else:
                    errors.append(
                        f"shepherd enqueue failed for {repo}#{pr['number']}: {detail}"
                    )

        results.append(
            {
                "repo": repo,
                "queue": queue,
                "open_issues": cov.get("open_issues", len(issues)),
                "covered": cov.get("covered", 0),
                "pr_covered": len(pr_covered),
                "pr_covered_issues": pr_covered,
                "verdict_covered": len(verdict_covered),
                "verdict_covered_issues": verdict_covered,
                "verdict_unread": len(verdict_unread),
                "verdict_unread_issues": verdict_unread,
                "resume_fulfilled": len(resume_fulfilled_issues),
                "resume_fulfilled_issues": resume_fulfilled_issues,
                "stale_issues": stale_issues,
                "uncovered": len(uncovered),
                "imported": imported,
                "watch_imported": watch_imported,
                "shepherd_candidates": shepherd_candidates,
                "shepherd_imported": shepherd_imported,
                "uncovered_issues": uncovered,
            }
        )

    report = render_report(checked_at, mode_name, results, errors)

    state_dir = Path(args.state_dir)
    state_dir.mkdir(parents=True, exist_ok=True)
    stamp = checked_at.replace(":", "").replace("-", "")
    report_path = state_dir / f"report-{stamp}.md"
    report_path.write_text(report)
    (state_dir / "latest.md").write_text(report)

    print(report)

    if errors:
        return 3
    return 0


if __name__ == "__main__":
    sys.exit(main())
