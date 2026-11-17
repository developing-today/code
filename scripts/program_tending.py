#!/usr/bin/env python3
"""program-tending (issue #1352, slices 1+2+3 of 3).

Daily scheduled care for the planning layer across the four fleet repos
(code, opencode, openchamber, nix). Slice 1 landed the scaffold: the
runner entrypoint, per-section stubs, and the structured run log.
Slice 2 (this version) implements two sections for real:

- epics: open `epic` issues get child progress (from the issue refs in
  the body, verified against live issue state), body-vs-reality
  mismatch counts, a status comment when the epic has been silent 7+
  days (last movement, next step, blockers), and a completion comment
  proposing closure when every child is closed. One comment per epic
  per run, and an epic already tended within 7 days is skipped.
- roadmaps: ROADMAP.md + planning/roadmaps/*.md in the checked-out
  home repo are compared against landed work. Certain drift — a
  forward-looking (Now/Next/Later) list item whose referenced issues
  are ALL closed — is fixed by PR (the item is removed; per ROADMAP.md
  itself, completed work is recorded in GitHub, not in a Done row).
  Done rows are never edited. Ambiguous drift (mixed open/closed refs
  on a forward item, or a Done-section item whose issue is still open)
  is reported on the tracking issue with the `meta/roadmap-drift`
  label instead of a guess — the precedent set by the 2026-11-01 run
  and tracked in #4412.

Slice 3 (this version) implements the remaining three sections:

- projects: Projects v2 boards linked to each repo are read through
  GraphQL. A board item whose Status contradicts its linked issue/PR
  reality (board Done while the content is open, or board not-done
  while the content is closed/merged) is corrected in live mode when
  the board's Status field and option ids are known; otherwise the
  contradiction is reported. Unreadable board state (no token, API
  failure, no linked board) is reported as unverified, never
  fabricated.
- stalled-prs: open PRs with no activity in 48h+ (and no hard-stop
  label) get one check-in comment naming what they are waiting on
  and the next step (marker-suppressed, so a PR is never spammed).
  `user/owned` PRs are the standing exception: a check-in at 7 days,
  never a packet. Fleet-owned PRs (author dezren39 / the Actions
  bot) additionally emit a structured `pr_shepherd` packet in the
  run log instead of only a comment.
- loose-ends: a child issue closed within the last 7 days whose body
  names a parent (`Child of #N`) gets one notification comment on
  the still-open parent; duplicate cross-links in open issue bodies
  are counted and reported; the aging `user/review` list is surfaced
  once a day on the tracking issue — reported, never re-decided.

Ownership rules (from #1352; enforced by every section):
- never touch held / do-not-merge / user-owned PRs (hold labels only:
  `held`, `user/owned`, `do-not-merge`, `do-not-auto-edit`);
- PRs labeled `user/owned` get a check-in comment at 7 days, never a packet
  (authorship alone never marks a PR held: a dezren39-authored PR with no
  hold label is workable like any other — Drew ruling 2026-11-07, code#7077);
- user/review is reported, never re-decided.

Actions are capped at MAX_ACTIONS_PER_RUN per run (#1352: ~12). The
runner is dry-run by default: it reports the actions it WOULD take.
--live performs them (comments, the drift label, the drift PR) and is
what the scheduled workflow runs. Without a token in the environment
--live degrades to dry-run reporting, because every write would fail.

Usage: python3 -I scripts/program_tending.py --run [--live] [--repo owner/name ...]
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import re
import subprocess
import sys
import urllib.request
from pathlib import Path

SECTIONS = ("epics", "roadmaps", "projects", "stalled-prs", "loose-ends")

FLEET_REPOS = (
    "developing-today/code",
    "developing-today-forks/opencode",
    "developing-today-forks/openchamber",
    "dezren39/darwin-nix",
)

# The repo this runner is checked out in when the workflow runs; the
# roadmap files and the tracking issue live here.
HOME_REPO = "developing-today/code"
TENDING_ISSUE = 1352
ROADMAP_DRIFT_LABEL = "meta/roadmap-drift"

# Labels the sections must never act on (read-only signals).
HARD_STOPS = ("held", "user/owned", "do-not-merge", "do-not-auto-edit")

# The fleet monitor tends issues and queue state; this job reports the
# planning layer and never changes monitor or stewardship-cycle behavior.
OUT_OF_SCOPE = ("fleet monitor behavior", "stewardship-cycle behavior")

MAX_ACTIONS_PER_RUN = 12
EPIC_SILENCE_DAYS = 7
EPIC_MARKER = "<!-- program-tending:epics -->"
ROADMAP_MARKER = "<!-- program-tending:roadmaps -->"
PROJECTS_MARKER = "<!-- program-tending:projects -->"
STALLED_MARKER = "<!-- program-tending:stalled-prs -->"
LOOSE_MARKER = "<!-- program-tending:loose-ends -->"
STALLED_PR_HOURS = 48
USER_OWNED_CHECKIN_DAYS = 7
LOOSE_CHILD_WINDOW_DAYS = 7
# The fleet's working identities: PRs by these authors that stall get
# a pr_shepherd packet, not only a comment (#1352). Authorship by the
# fleet account never makes a PR held (Drew ruling 2026-11-07).
FLEET_PR_AUTHORS = ("dezren39", "github-actions[bot]")

_REF_RE = re.compile(r"(?<![\w/])#(\d+)")
_URL_REF_RE = re.compile(r"github\.com/([\w.-]+/[\w.-]+)/(?:issues|pull)/(\d+)")
_MD_LINK_RE = re.compile(r"\[[^\]]*\]\([^)]*\)")
_CHECKBOX_RE = re.compile(r"^\s*[-*]\s+\[([ xX])\]\s+(.*)$")
_LIST_ITEM_RE = re.compile(r"^\s*(?:[-*]|\d+\.)\s+\S")
_TABLE_ROW_RE = re.compile(r"^\s*\|.*\|\s*$")
_TABLE_SEP_RE = re.compile(r"^\s*\|[\s:|-]+\|\s*$")


def _is_roadmap_item(line: str) -> bool:
    """A roadmap entry line: a list item, or a Markdown table row.

    The project roadmaps under planning/roadmaps/ express their
    Now/Next/Later entries as table rows, not bullets (Codex,
    PR #8377); separator rows (|---|---|) are not entries.
    """
    if _LIST_ITEM_RE.match(line):
        return True
    return bool(_TABLE_ROW_RE.match(line)) and not _TABLE_SEP_RE.match(line)


def _utcnow() -> str:
    return dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _utcnow_dt() -> dt.datetime:
    return dt.datetime.now(dt.timezone.utc)


def _parse_ts(value: str) -> dt.datetime | None:
    try:
        return dt.datetime.strptime(value, "%Y-%m-%dT%H:%M:%SZ").replace(
            tzinfo=dt.timezone.utc
        )
    except (TypeError, ValueError):
        return None


def _gh_headers() -> dict:
    headers = {"Accept": "application/vnd.github+json"}
    # The workflow hands this job a token (#6860): send it when present
    # so the reads are authenticated; without one, behave exactly as
    # before. The value is only ever placed in the header — it is
    # never logged or included in any error or report string.
    token = os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN")
    if token:
        headers["Authorization"] = f"Bearer {token}"
    return headers


def _gh_api(path: str) -> list:
    """GET a GitHub REST path, following pagination.

    Collection endpoints return their items; a single-object endpoint
    returns a one-item list holding the object, so callers have one
    shape to handle.
    """
    items: list = []
    url = f"https://api.github.com{path}"
    headers = _gh_headers()
    while url:
        req = urllib.request.Request(url, headers=headers)
        with urllib.request.urlopen(req, timeout=30) as resp:
            data = json.loads(resp.read().decode("utf-8"))
            nxt = resp.headers.get("Link", "")
        if isinstance(data, list):
            items.extend(data)
        else:
            items.append(data)
        url = ""
        for part in nxt.split(","):
            if 'rel="next"' in part:
                url = part.split(";")[0].strip().strip("<>")
    return items


def _gh_write(path: str, payload: dict) -> dict:
    """POST a GitHub REST path with a JSON body (comments, labels)."""
    req = urllib.request.Request(
        f"https://api.github.com{path}",
        data=json.dumps(payload).encode("utf-8"),
        headers={**_gh_headers(), "Content-Type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        return json.loads(resp.read().decode("utf-8"))


def _labels(item: dict) -> set[str]:
    return {label["name"] for label in item.get("labels", [])}


def _refs_in(text: str, default_repo: str) -> list[tuple[str, int]]:
    """Issue/PR references in a body of text, as (repo, number) pairs."""
    refs: list[tuple[str, int]] = []
    seen: set[tuple[str, int]] = set()
    for match in _URL_REF_RE.finditer(text):
        key = (match.group(1), int(match.group(2)))
        if key not in seen:
            seen.add(key)
            refs.append(key)
    # Bare refs are read from the text with Markdown links masked out:
    # a link's label ("[#42](https://github.com/other/repo/issues/42)")
    # is not a local reference -- the URL pass above already recorded
    # the cross-repo ref, and counting the label too would invent a
    # phantom local child (Codex, PR #8377).
    for match in _REF_RE.finditer(_MD_LINK_RE.sub(" ", text)):
        key = (default_repo, int(match.group(1)))
        if key not in seen:
            seen.add(key)
            refs.append(key)
    return refs


def _issue_state(repo: str, number: int, cache: dict) -> dict | None:
    """Live issue state via the API, memoised per run. None = unverified."""
    key = (repo, number)
    if key not in cache:
        try:
            found = _gh_api(f"/repos/{repo}/issues/{number}")
            cache[key] = found[0] if found else None
        except Exception:  # noqa: BLE001 - a section reports, never fails
            cache[key] = None
    return cache[key]


def _gh_graphql(query: str, variables: dict | None = None) -> dict:
    # GraphQL POST (Projects v2 has no REST surface). Same header
    # rules as _gh_api: the token goes in the header only, never in
    # any returned or logged string.
    payload: dict = {"query": query}
    if variables:
        payload["variables"] = variables
    req = urllib.request.Request(
        "https://api.github.com/graphql",
        data=json.dumps(payload).encode("utf-8"),
        headers={**_gh_headers(), "Content-Type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        data = json.loads(resp.read().decode("utf-8"))
    if isinstance(data, dict) and data.get("errors"):
        raise RuntimeError(f"graphql errors: {data['errors']}")
    return data.get("data") or {}


# ---------------------------------------------------------------- epics


def _epic_children(epic: dict, repo: str, cache: dict) -> list[dict]:
    """Child progress for one epic from the refs in its body."""
    body = epic.get("body") or ""
    checked_in_body: dict[tuple[str, int], bool] = {}
    for line in body.splitlines():
        match = _CHECKBOX_RE.match(line)
        if match:
            for ref in _refs_in(match.group(2), repo):
                checked_in_body[ref] = match.group(1).lower() == "x"
    children = []
    for ref_repo, number in _refs_in(body, repo):
        if ref_repo == repo and number == epic.get("number"):
            continue
        state = _issue_state(ref_repo, number, cache)
        children.append(
            {
                "repo": ref_repo,
                "number": number,
                "state": state.get("state") if state else "unverified",
                "title": state.get("title", "") if state else "",
                "labels": sorted(_labels(state)) if state else [],
                "body_checked": checked_in_body.get((ref_repo, number)),
            }
        )
    return children


def _epic_comment(kind: str, epic: dict, children: list[dict]) -> str:
    total = len(children)
    closed = [c for c in children if c["state"] == "closed"]
    open_children = [c for c in children if c["state"] == "open"]
    blocked = [c for c in open_children if "status/blocked" in c.get("labels", [])]
    lines = [
        f"**Program tending — epic {'completion' if kind == 'completion' else 'check-in'}**"
        f" (tracking: #{TENDING_ISSUE})",
        f"- Child progress: {len(closed)} of {total} children closed"
        f" ({len(open_children)} open).",
        f"- Last movement: {epic.get('updated_at', 'unknown')}.",
    ]
    if kind == "completion":
        lines.append(
            "- All recorded children are closed. Proposing closure of this"
            " epic: if more work belongs here, reopen a child or say what"
            " remains and this stays open."
        )
    else:
        if open_children:
            nxt = open_children[0]
            lines.append(
                f"- Next step: #{nxt['number']} — {nxt['title'] or 'untitled'}"
                " is the first open child recorded in the body."
            )
        else:
            lines.append(
                "- Next step: no open children are recorded in the body;"
                " add the next child or propose closure."
            )
        if blocked:
            names = ", ".join(f"#{c['number']}" for c in blocked)
            lines.append(f"- Blockers: {names} carry `status/blocked`.")
        else:
            lines.append("- Blockers: none recorded (`status/blocked` on a child).")
    lines.append(EPIC_MARKER)
    return "\n".join(lines)


def section_epics(repos: list[str], live: bool, now: dt.datetime, budget: int) -> dict:
    checked: list[dict] = []
    actions: list[dict] = []
    cache: dict = {}
    for repo in repos:
        try:
            epics = _gh_api(f"/repos/{repo}/issues?state=open&labels=epic&per_page=100")
        except Exception as exc:  # noqa: BLE001 - report, never fail
            checked.append(
                {"repo": repo, "open_epics": "unverified", "error": str(exc)}
            )
            continue
        for epic in epics:
            entry: dict = {"repo": repo, "epic": epic.get("number")}
            if "pull_request" in epic:
                # The issues endpoint also returns PRs; an epic is an
                # issue by definition, never a PR.
                entry["disposition"] = "skipped: pull request, not an epic"
                checked.append(entry)
                continue
            if _labels(epic) & set(HARD_STOPS):
                entry["disposition"] = "skipped: hard-stop label"
                checked.append(entry)
                continue
            children = _epic_children(epic, repo, cache)
            verified = [c for c in children if c["state"] in ("open", "closed")]
            entry["children"] = {
                "total": len(children),
                "open": sum(1 for c in verified if c["state"] == "open"),
                "closed": sum(1 for c in verified if c["state"] == "closed"),
                "unverified": len(children) - len(verified),
            }
            entry["body_vs_reality_mismatches"] = sum(
                1
                for c in verified
                if c["body_checked"] is not None
                and c["body_checked"] != (c["state"] == "closed")
            )
            updated = _parse_ts(epic.get("updated_at", ""))
            silent_days = (now - updated).days if updated else 0
            entry["silent_days"] = silent_days
            kind = None
            if (
                verified
                and all(c["state"] == "closed" for c in verified)
                and entry["children"]["unverified"] == 0
            ):
                kind = "completion"
            elif silent_days >= EPIC_SILENCE_DAYS:
                kind = "status"
            if kind is None:
                entry["disposition"] = "verified: no action due"
                checked.append(entry)
                continue
            # The recent-tending suppression applies to both kinds: a
            # completion proposal reposted daily would spam the epic
            # and burn the shared action budget (Codex, PR #8377).
            if kind in ("status", "completion"):
                try:
                    comments = _gh_api(
                        f"/repos/{repo}/issues/{epic.get('number')}/comments"
                        "?per_page=100"
                    )
                except Exception:  # noqa: BLE001 - treat as not tended
                    comments = []
                recently_tended = any(
                    EPIC_MARKER in (c.get("body") or "")
                    and (_parse_ts(c.get("created_at", "")) or now)
                    >= now - dt.timedelta(days=EPIC_SILENCE_DAYS)
                    for c in comments
                )
                if recently_tended:
                    entry["disposition"] = "verified: tended within 7 days"
                    checked.append(entry)
                    continue
            if len(actions) >= budget:
                entry["disposition"] = "deferred: run action cap reached"
                checked.append(entry)
                continue
            action = {
                "kind": f"epic-{kind}-comment",
                "repo": repo,
                "issue": epic.get("number"),
                "live": False,
            }
            if live:
                try:
                    posted = _gh_write(
                        f"/repos/{repo}/issues/{epic.get('number')}/comments",
                        {"body": _epic_comment(kind, epic, children)},
                    )
                except Exception as exc:  # noqa: BLE001 - isolate the failure
                    # One repo's write failure (e.g. a token without
                    # access there) must not abort the run before the
                    # remaining repos and sections are processed
                    # (Codex, PR #8377).
                    action["error"] = f"write failed: {exc}"
                    actions.append(action)
                    entry["disposition"] = f"{kind} comment failed: {exc}"
                    checked.append(entry)
                    continue
                action["live"] = True
                action["comment_id"] = posted.get("id")
            actions.append(action)
            entry["disposition"] = f"{kind} comment {'posted' if live else 'planned'}"
            checked.append(entry)
    return {
        "section": "epics",
        "status": "action" if actions else "no-action",
        "detail": (
            "per-epic child progress verified against live issue state; "
            f"{EPIC_SILENCE_DAYS}-day silence check-ins and completion "
            "proposals, one comment per epic per run"
        ),
        "checked": checked,
        "actions": actions,
    }


# ------------------------------------------------------------- roadmaps


def _roadmap_files(root: Path) -> list[Path]:
    files = []
    top = root / "ROADMAP.md"
    if top.is_file():
        files.append(top)
    planning = root / "planning" / "roadmaps"
    if planning.is_dir():
        files.extend(sorted(planning.glob("*.md")))
    return files


def _section_kind(heading: str) -> str:
    text = heading.lstrip("#").strip().lower()
    if text.startswith("done"):
        return "done"
    if text.startswith(("now", "next", "later")):
        return "forward"
    return "neutral"


def _roadmap_drift(root: Path) -> tuple[list[dict], list[dict]]:
    """Certain and ambiguous drift entries across the roadmap files.

    Certain: a forward-section list item whose refs are all verified
    closed. Ambiguous: a forward item with mixed open/closed refs, or
    a Done-section item with a verified-open ref. Refs whose state
    cannot be verified are neither — the run reports them unverified.
    """
    states: dict[tuple[str, int], str] = {}
    try:
        issues = _gh_api(f"/repos/{HOME_REPO}/issues?state=all&per_page=100")
        for issue in issues:
            if isinstance(issue, dict) and issue.get("number"):
                states[(HOME_REPO, int(issue["number"]))] = issue.get(
                    "state", "unverified"
                )
    except Exception:  # noqa: BLE001 - all refs stay unverified
        pass
    certain: list[dict] = []
    ambiguous: list[dict] = []
    for path in _roadmap_files(root):
        kind = "neutral"
        for lineno, line in enumerate(path.read_text().splitlines(), start=1):
            if line.startswith("##"):
                kind = _section_kind(line)
                continue
            if kind == "neutral" or not _is_roadmap_item(line):
                continue
            refs = _refs_in(line, HOME_REPO)
            if not refs:
                continue
            resolved = [(ref, states.get(ref, "unverified")) for ref in refs]
            verified = [s for _, s in resolved if s in ("open", "closed")]
            if not verified:
                continue
            entry = {
                "file": str(path.relative_to(root)),
                "line": lineno,
                "text": line.strip()[:200],
                "refs": [f"{r[0]}#{r[1]}:{s}" for r, s in resolved],
            }
            if kind == "forward" and all(s == "closed" for _, s in resolved):
                certain.append(entry)
            elif kind == "forward" and "closed" in verified and "open" in verified:
                entry["reason"] = "forward item mixes closed and open refs"
                ambiguous.append(entry)
            elif kind == "done" and "open" in verified:
                entry["reason"] = "Done-section item references an open issue"
                ambiguous.append(entry)
    return certain, ambiguous


def _remove_lines(path: Path, linenos: set[int]) -> None:
    lines = path.read_text().splitlines()
    kept = [l for i, l in enumerate(lines, start=1) if i not in linenos]
    path.write_text("\n".join(kept) + "\n")


def _run_cmd(argv: list[str], cwd: Path) -> None:
    subprocess.run(argv, cwd=str(cwd), check=True, capture_output=True, text=True)


def _open_drift_pr(root: Path, certain: list[dict], today: str) -> dict:
    """Apply the certain-drift removals and open the fix PR via git+gh."""
    by_file: dict[str, set[int]] = {}
    for entry in certain:
        by_file.setdefault(entry["file"], set()).add(entry["line"])
    for rel, linenos in by_file.items():
        _remove_lines(root / rel, linenos)
    branch = f"program-tending/roadmap-drift-{today}"
    removed = "\n".join(
        f"- `{e['file']}` line {e['line']}: {e['text']} (refs: {', '.join(e['refs'])})"
        for e in certain
    )
    body = (
        "Program-tending roadmap drift fix (tracking: "
        f"#{TENDING_ISSUE}). Each removed item is a forward-looking "
        "roadmap entry whose referenced issues are all closed; per "
        "ROADMAP.md, completed work is recorded in GitHub, not in a "
        "Done row. No Done rows were edited.\n\n" + removed + "\n"
    )
    _run_cmd(["git", "checkout", "-b", branch], root)
    _run_cmd(["git", "add", *sorted(by_file)], root)
    _run_cmd(
        [
            "git",
            "-c",
            "user.name=github-actions[bot]",
            "-c",
            "user.email=41898282+github-actions[bot]@users.noreply.github.com",
            "commit",
            "-m",
            "docs(roadmap): program-tending drift fix — remove landed forward items",
        ],
        root,
    )
    _run_cmd(["git", "push", "-u", "origin", branch], root)
    proc = subprocess.run(
        [
            "gh",
            "pr",
            "create",
            "--repo",
            HOME_REPO,
            "--base",
            "main",
            "--head",
            branch,
            "--title",
            "docs(roadmap): program-tending drift fix (landed forward items)",
            "--body",
            body,
        ],
        cwd=str(root),
        capture_output=True,
        text=True,
        check=True,
    )
    return {
        "kind": "roadmap-drift-pr",
        "branch": branch,
        "pr": proc.stdout.strip().splitlines()[-1] if proc.stdout else "",
        "items_removed": len(certain),
        "live": True,
    }


def section_roadmaps(
    repos: list[str], live: bool, now: dt.datetime, budget: int, root: Path
) -> dict:
    checked: list[dict] = []
    actions: list[dict] = []
    for repo in repos:
        if repo != HOME_REPO:
            checked.append(
                {
                    "repo": repo,
                    "drift": "not evaluated: roadmap files live in the checked-out home repo",
                }
            )
    if HOME_REPO not in repos:
        return {
            "section": "roadmaps",
            "status": "no-action",
            "detail": "home repo not in this run's repo set",
            "checked": checked,
            "actions": actions,
        }
    files = _roadmap_files(root)
    if not files:
        checked.append({"repo": HOME_REPO, "drift": "no roadmap files found"})
        return {
            "section": "roadmaps",
            "status": "no-action",
            "detail": "ROADMAP.md + planning/roadmaps vs landed work",
            "checked": checked,
            "actions": actions,
        }
    certain, ambiguous = _roadmap_drift(root)
    checked.append(
        {
            "repo": HOME_REPO,
            "files": [str(p.relative_to(root)) for p in files],
            "certain_drift": certain,
            "ambiguous_drift": ambiguous,
        }
    )
    if certain and len(actions) < budget:
        if live:
            try:
                actions.append(_open_drift_pr(root, certain, now.strftime("%Y-%m-%d")))
            except Exception as exc:  # noqa: BLE001 - report, never fail
                checked.append({"repo": HOME_REPO, "drift_pr_error": str(exc)[:300]})
        else:
            actions.append(
                {
                    "kind": "roadmap-drift-pr",
                    "items_removed": len(certain),
                    "files": sorted({e["file"] for e in certain}),
                    "live": False,
                }
            )
    if ambiguous and len(actions) < budget:
        action = {
            "kind": "roadmap-drift-flag",
            "repo": HOME_REPO,
            "issue": TENDING_ISSUE,
            "items": len(ambiguous),
            "live": False,
        }
        if live:
            body = "\n".join(
                [
                    "**Program tending — ambiguous roadmap drift** "
                    f"(tracking: #{TENDING_ISSUE}). Not fixed by guess; "
                    f"`{ROADMAP_DRIFT_LABEL}` applied here per the "
                    "2026-11-01 precedent:",
                ]
                + [
                    f"- `{e['file']}` line {e['line']}: {e['text']} — "
                    f"{e['reason']} (refs: {', '.join(e['refs'])})"
                    for e in ambiguous
                ]
                + [ROADMAP_MARKER],
            )
            posted = _gh_write(
                f"/repos/{HOME_REPO}/issues/{TENDING_ISSUE}/comments",
                {"body": body},
            )
            _gh_write(
                f"/repos/{HOME_REPO}/issues/{TENDING_ISSUE}/labels",
                {"labels": [ROADMAP_DRIFT_LABEL]},
            )
            action["live"] = True
            action["comment_id"] = posted.get("id")
        actions.append(action)
    return {
        "section": "roadmaps",
        "status": "action" if actions else "no-action",
        "detail": (
            "ROADMAP.md + planning/roadmaps vs landed work: certain drift "
            "fixed by PR (forward rows only, never Done rows); ambiguous "
            f"drift flagged on #{TENDING_ISSUE} with `{ROADMAP_DRIFT_LABEL}`"
        ),
        "checked": checked,
        "actions": actions,
    }


# ------------------------------------------------------------- projects

_PROJECTS_QUERY = """
query($owner: String!, $name: String!) {
  repository(owner: $owner, name: $name) {
    projectsV2(first: 10) {
      nodes {
        id
        title
        fields(first: 30) {
          nodes {
            ... on ProjectV2SingleSelectField { id name options { id name } }
          }
        }
        items(first: 100) {
          nodes {
            id
            fieldValueByName(name: "Status") {
              ... on ProjectV2ItemFieldSingleSelectValue { name }
            }
            content {
              ... on Issue { number state title }
              ... on PullRequest { number state title merged }
            }
          }
        }
      }
    }
  }
}
"""

_PROJECT_CORRECT_MUTATION = """
mutation($project: ID!, $item: ID!, $field: ID!, $option: ID!) {
  updateProjectV2ItemFieldValue(input: {
    projectId: $project, itemId: $item, fieldId: $field,
    value: { singleSelectOptionId: $option }
  }) { projectV2Item { id } }
}
"""

_BOARD_DONE = ("done", "closed", "complete", "completed")
_BOARD_NOT_DONE = (
    "todo",
    "to do",
    "backlog",
    "in progress",
    "in review",
    "not started",
)


def _flatten_project_items(data: dict, repo: str) -> list[dict]:
    items: list[dict] = []
    repository = (data or {}).get("repository") or {}
    projects = (repository.get("projectsV2") or {}).get("nodes") or []
    for project in projects:
        if not project:
            continue
        status_field = None
        for field in (project.get("fields") or {}).get("nodes") or []:
            if field and field.get("name") == "Status" and field.get("id"):
                status_field = {
                    "id": field["id"],
                    "options": {
                        o["name"]: o["id"]
                        for o in (field.get("options") or [])
                        if o and o.get("name") and o.get("id")
                    },
                }
        for node in (project.get("items") or {}).get("nodes") or []:
            if not node:
                continue
            content = node.get("content") or {}
            if not content.get("number"):
                continue
            state = str(content.get("state") or "").lower()
            if content.get("merged"):
                state = "closed"
            items.append(
                {
                    "repo": repo,
                    "project_id": project.get("id"),
                    "project_title": project.get("title", ""),
                    "item_id": node.get("id"),
                    "board_status": (node.get("fieldValueByName") or {}).get("name")
                    or "",
                    "content_number": int(content["number"]),
                    "content_state": state,
                    "content_title": content.get("title", ""),
                    "status_field": status_field,
                }
            )
    return items


def _project_contradiction(item: dict) -> str | None:
    board = (item.get("board_status") or "").strip().lower()
    state = item.get("content_state")
    if not board or state not in ("open", "closed"):
        return None
    if board in _BOARD_DONE and state == "open":
        return "board status says done but the linked item is open"
    if board in _BOARD_NOT_DONE and state == "closed":
        return "board status says not done but the linked item is closed"
    return None


def _correction_option(item: dict) -> dict | None:
    field = item.get("status_field")
    if not field:
        return None
    wanted = (
        ("Done", "Closed", "Complete", "Completed")
        if item.get("content_state") == "closed"
        else ("In Progress", "Todo", "To Do", "Backlog")
    )
    for name in wanted:
        if name in field["options"]:
            return {
                "field_id": field["id"],
                "option_id": field["options"][name],
                "option_name": name,
            }
    return None


def section_projects(
    repos: list[str], live: bool, now: dt.datetime, budget: int
) -> dict:
    checked: list[dict] = []
    actions: list[dict] = []
    if not (os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN")):
        # Board reads need an authenticated GraphQL call; without a
        # token the board state is unreadable — report it, never
        # fabricate it (#1352 projects rule).
        for repo in repos:
            checked.append(
                {"repo": repo, "board_state": "unverified: no token in environment"}
            )
        return {
            "section": "projects",
            "status": "no-action",
            "detail": "Projects v2 board-vs-reality checks need an authenticated "
            "GraphQL read; without a token the board state is reported "
            "unverified, never fabricated",
            "checked": checked,
            "actions": actions,
        }
    for repo in repos:
        owner, _, name = repo.partition("/")
        try:
            data = _gh_graphql(_PROJECTS_QUERY, {"owner": owner, "name": name})
        except Exception as exc:  # noqa: BLE001 - report, never fail
            checked.append(
                {"repo": repo, "board_state": "unverified", "error": str(exc)}
            )
            continue
        items = _flatten_project_items(data, repo)
        entry: dict = {
            "repo": repo,
            "board_items": len(items),
            "contradictions": [],
        }
        for item in items:
            reason = _project_contradiction(item)
            if reason is None:
                continue
            entry["contradictions"].append(
                {
                    "project": item["project_title"],
                    "content": item["content_number"],
                    "board_status": item["board_status"],
                    "content_state": item["content_state"],
                    "reason": reason,
                }
            )
            if len(actions) >= budget:
                entry["deferred"] = entry.get("deferred", 0) + 1
                continue
            action = {
                "kind": "project-board-correction",
                "repo": repo,
                "project": item["project_title"],
                "content": item["content_number"],
                "reason": reason,
                "live": False,
            }
            correction = _correction_option(item)
            if live and correction:
                try:
                    _gh_graphql(
                        _PROJECT_CORRECT_MUTATION,
                        {
                            "project": item["project_id"],
                            "item": item["item_id"],
                            "field": correction["field_id"],
                            "option": correction["option_id"],
                        },
                    )
                    action["live"] = True
                    action["corrected_to"] = correction["option_name"]
                except Exception as exc:  # noqa: BLE001 - isolate the failure
                    action["error"] = f"correction failed: {exc}"
            elif live:
                action["note"] = (
                    "no Status field/option mapping available; "
                    "contradiction reported, not corrected"
                )
            actions.append(action)
        checked.append(entry)
    return {
        "section": "projects",
        "status": "action" if actions else "no-action",
        "detail": "Projects v2 boards vs linked issue/PR reality: contradictions "
        "corrected in live mode where the Status field mapping allows; "
        "unreadable board state reported, never fabricated",
        "checked": checked,
        "actions": actions,
    }


# ---------------------------------------------------------- stalled-prs


def _stalled_waiting_on(pr: dict) -> tuple[str, str]:
    labels = _labels(pr)
    if pr.get("draft"):
        return (
            "draft — waiting on its author",
            "mark the PR ready for review, or post what the draft is waiting on",
        )
    if "status/blocked" in labels:
        return (
            "blocked (`status/blocked` label)",
            "clear the recorded blocker, or update the PR with the current blocker",
        )
    if "user/review" in labels:
        return (
            "awaiting `user/review`",
            "the reviewer works the `user/review` item and posts the outcome here",
        )
    return (
        "no recorded blocker",
        "post the current state and the exact next step on the PR",
    )


def _stalled_comment(pr: dict, waiting: str, next_step: str, silent_days: int) -> str:
    return "\n".join(
        [
            f"**Program tending — stalled PR check-in** (tracking: #{TENDING_ISSUE})",
            f"- No activity for {silent_days} day(s); last update "
            f"{pr.get('updated_at', 'unknown')}.",
            f"- Waiting on: {waiting}.",
            f"- Next step: {next_step}.",
            STALLED_MARKER,
        ]
    )


def section_stalled_prs(
    repos: list[str], live: bool, now: dt.datetime, budget: int
) -> dict:
    checked: list[dict] = []
    actions: list[dict] = []
    hard = set(HARD_STOPS) - {"user/owned"}
    for repo in repos:
        try:
            prs = _gh_api(f"/repos/{repo}/pulls?state=open&per_page=100")
        except Exception as exc:  # noqa: BLE001 - report, never fail
            checked.append({"repo": repo, "open_prs": "unverified", "error": str(exc)})
            continue
        entry: dict = {
            "repo": repo,
            "open_prs": len(prs),
            "stalled": [],
            "skipped_hard_stop": 0,
            "already_tended": [],
        }
        for pr in prs:
            number = pr.get("number")
            if not number:
                continue
            labels = _labels(pr)
            if labels & hard:
                entry["skipped_hard_stop"] += 1
                continue
            updated = _parse_ts(pr.get("updated_at", ""))
            if updated is None:
                continue
            silent = now - updated
            user_owned = "user/owned" in labels
            if user_owned:
                if silent.days < USER_OWNED_CHECKIN_DAYS:
                    continue
            elif silent < dt.timedelta(hours=STALLED_PR_HOURS):
                continue
            try:
                comments = _gh_api(
                    f"/repos/{repo}/issues/{number}/comments?per_page=100"
                )
            except Exception:  # noqa: BLE001 - treat as not tended
                comments = []
            if any(STALLED_MARKER in (c.get("body") or "") for c in comments):
                entry["already_tended"].append(number)
                continue
            entry["stalled"].append(number)
            if len(actions) >= budget:
                entry["deferred"] = entry.get("deferred", 0) + 1
                continue
            waiting, next_step = _stalled_waiting_on(pr)
            author = (pr.get("user") or {}).get("login", "")
            action = {
                "kind": "stalled-pr-user-owned-checkin"
                if user_owned
                else "stalled-pr-checkin",
                "repo": repo,
                "pr": number,
                "silent_days": silent.days,
                "waiting_on": waiting,
                "live": False,
            }
            if not user_owned and author in FLEET_PR_AUTHORS:
                # Fleet-owned PRs that merely need shepherding get a
                # packet instead of only a comment (#1352).
                action["packet"] = {
                    "kind": "pr_shepherd",
                    "repo": repo,
                    "pr": number,
                    "dedupe_key": f"repo-health:v1:shepherd:{repo}:pr:{number}",
                }
            if live:
                try:
                    posted = _gh_write(
                        f"/repos/{repo}/issues/{number}/comments",
                        {"body": _stalled_comment(pr, waiting, next_step, silent.days)},
                    )
                    action["live"] = True
                    action["comment_id"] = posted.get("id")
                except Exception as exc:  # noqa: BLE001 - isolate the failure
                    action["error"] = f"write failed: {exc}"
            actions.append(action)
        checked.append(entry)
    return {
        "section": "stalled-prs",
        "status": "action" if actions else "no-action",
        "detail": f"open PRs silent {STALLED_PR_HOURS}h+ get one marker-suppressed "
        "check-in naming what they wait on; user/owned PRs at "
        f"{USER_OWNED_CHECKIN_DAYS} days, comment only, never a packet; "
        "fleet-owned PRs also emit a pr_shepherd packet",
        "checked": checked,
        "actions": actions,
    }


# ----------------------------------------------------------- loose-ends

_PARENT_RE = re.compile(
    r"child of\s+(?:epic\s+|issue\s+)?(?:https://github\.com/([\w.-]+/[\w.-]+)/(?:issues|pull)/(\d+)|#(\d+))",
    re.IGNORECASE,
)


def _parent_ref(text: str, default_repo: str) -> tuple[str, int] | None:
    match = _PARENT_RE.search(text or "")
    if not match:
        return None
    if match.group(3):
        return (default_repo, int(match.group(3)))
    return (match.group(1), int(match.group(2)))


def _duplicate_ref_counts(text: str, default_repo: str) -> dict:
    counts: dict[str, int] = {}
    for match in _URL_REF_RE.finditer(text or ""):
        key = f"{match.group(1)}#{match.group(2)}"
        counts[key] = counts.get(key, 0) + 1
    for match in _REF_RE.finditer(_MD_LINK_RE.sub(" ", text or "")):
        key = f"{default_repo}#{match.group(1)}"
        counts[key] = counts.get(key, 0) + 1
    return {k: c for k, c in counts.items() if c > 1}


def section_loose_ends(
    repos: list[str], live: bool, now: dt.datetime, budget: int
) -> dict:
    checked: list[dict] = []
    actions: list[dict] = []
    cache: dict = {}
    since = (now - dt.timedelta(days=LOOSE_CHILD_WINDOW_DAYS)).strftime(
        "%Y-%m-%dT%H:%M:%SZ"
    )
    aging_all: list[dict] = []
    for repo in repos:
        entry: dict = {"repo": repo, "parent_notifications": 0}
        # Parent notification when a child closes.
        try:
            closed = _gh_api(
                f"/repos/{repo}/issues?state=closed&since={since}&per_page=100"
            )
        except Exception as exc:  # noqa: BLE001 - report, never fail
            closed = []
            entry["closed_children"] = "unverified"
            entry["error"] = str(exc)
        for issue in closed:
            if "pull_request" in issue:
                continue
            number = issue.get("number")
            if not number:
                continue
            parent = _parent_ref(issue.get("body") or "", repo)
            if parent is None:
                continue
            parent_repo, parent_number = parent
            parent_state = _issue_state(parent_repo, parent_number, cache)
            if not parent_state or parent_state.get("state") != "open":
                continue
            try:
                comments = _gh_api(
                    f"/repos/{parent_repo}/issues/{parent_number}/comments?per_page=100"
                )
            except Exception:  # noqa: BLE001 - treat as not notified
                comments = []
            if any(
                LOOSE_MARKER in (c.get("body") or "")
                and f"child #{number} closed" in (c.get("body") or "")
                for c in comments
            ):
                continue
            if len(actions) >= budget:
                entry["deferred"] = entry.get("deferred", 0) + 1
                continue
            action = {
                "kind": "loose-end-parent-notification",
                "repo": parent_repo,
                "issue": parent_number,
                "child": number,
                "live": False,
            }
            if live:
                body = "\n".join(
                    [
                        f"**Program tending — loose end** (tracking: #{TENDING_ISSUE})",
                        f"- Child #{number} — {issue.get('title', 'untitled')} — "
                        "has closed; this parent may be ready to update or close.",
                        f"child #{number} closed",
                        LOOSE_MARKER,
                    ]
                )
                try:
                    posted = _gh_write(
                        f"/repos/{parent_repo}/issues/{parent_number}/comments",
                        {"body": body},
                    )
                    action["live"] = True
                    action["comment_id"] = posted.get("id")
                except Exception as exc:  # noqa: BLE001 - isolate the failure
                    action["error"] = f"write failed: {exc}"
            actions.append(action)
            entry["parent_notifications"] += 1
        # Duplicate cross-links in open issue bodies (report only).
        try:
            open_issues = _gh_api(f"/repos/{repo}/issues?state=open&per_page=100")
        except Exception:  # noqa: BLE001 - report, never fail
            open_issues = []
        dups = []
        for issue in open_issues:
            if "pull_request" in issue or not issue.get("number"):
                continue
            counts = _duplicate_ref_counts(issue.get("body") or "", repo)
            if counts:
                dups.append({"issue": issue["number"], "duplicate_refs": counts})
        entry["duplicate_cross_links"] = dups
        # Aging user/review list (reported, never re-decided).
        try:
            review = _gh_api(
                f"/repos/{repo}/issues?state=open&labels=user/review&per_page=100"
            )
        except Exception:  # noqa: BLE001 - report, never fail
            review = []
        aging = []
        for item in review:
            if not item.get("number"):
                continue
            updated = _parse_ts(item.get("updated_at", ""))
            aging.append(
                {
                    "repo": repo,
                    "number": item["number"],
                    "is_pr": "pull_request" in item,
                    "updated_at": item.get("updated_at", ""),
                    "age_days": (now - updated).days if updated else None,
                }
            )
        aging.sort(key=lambda a: -(a["age_days"] or 0))
        entry["user_review_aging"] = aging
        aging_all.extend(aging)
        checked.append(entry)
    # Surface the aging user/review list once a day on the tracking
    # issue (live), or as a planned action (dry-run).
    if aging_all and len(actions) < budget:
        today = now.strftime("%Y-%m-%d")
        already = False
        if live:
            try:
                tcomments = _gh_api(
                    f"/repos/{HOME_REPO}/issues/{TENDING_ISSUE}/comments?per_page=100"
                )
            except Exception:  # noqa: BLE001 - treat as not surfaced
                tcomments = []
            already = any(
                LOOSE_MARKER in (c.get("body") or "")
                and f"user/review surface {today}" in (c.get("body") or "")
                for c in tcomments
            )
        if not already:
            aging_all.sort(key=lambda a: -(a["age_days"] or 0))
            action = {
                "kind": "loose-end-user-review-surface",
                "repo": HOME_REPO,
                "issue": TENDING_ISSUE,
                "items": len(aging_all),
                "live": False,
            }
            if live:
                lines = [
                    f"**Program tending — aging `user/review`** (tracking: #{TENDING_ISSUE}). "
                    "Reported, not re-decided:",
                ]
                for a in aging_all[:20]:
                    kind = "PR" if a["is_pr"] else "issue"
                    lines.append(
                        f"- {a['repo']} {kind} #{a['number']} — age "
                        f"{a['age_days']}d, last update {a['updated_at']}"
                    )
                lines.append(f"user/review surface {today}")
                lines.append(LOOSE_MARKER)
                try:
                    posted = _gh_write(
                        f"/repos/{HOME_REPO}/issues/{TENDING_ISSUE}/comments",
                        {"body": "\n".join(lines)},
                    )
                    action["live"] = True
                    action["comment_id"] = posted.get("id")
                except Exception as exc:  # noqa: BLE001 - isolate the failure
                    action["error"] = f"write failed: {exc}"
            actions.append(action)
    return {
        "section": "loose-ends",
        "status": "action" if actions else "no-action",
        "detail": "parents notified once when a child closes; duplicate "
        "cross-links counted; the aging user/review list surfaced daily "
        "on the tracking issue, reported and never re-decided",
        "checked": checked,
        "actions": actions,
    }


# ------------------------------------------------------- slice 3 stubs


def stub_projects(repos: list[str]) -> dict:
    """Slice 3 owns this section. Slice 2 verifies reachability only."""
    return {
        "section": "projects",
        "status": "no-action",
        "detail": "scaffold only: board-vs-reality correction checks land in slice 3",
        "checked": [
            {"repo": repo, "board_state": "unverified (slice 3)"} for repo in repos
        ],
    }


def stub_stalled_prs(repos: list[str]) -> dict:
    """Slice 3 owns this section. Slice 2 counts, but never comments."""
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
    """Slice 3 owns this section. Slice 2 verifies reachability only."""
    return {
        "section": "loose-ends",
        "status": "no-action",
        "detail": "scaffold only: notifications, duplicate cross-links, user/review surfacing land in slice 3",
        "checked": [
            {"repo": repo, "loose_ends": "unverified (slice 3)"} for repo in repos
        ],
    }


# ------------------------------------------------------------------ run


def _default_roadmap_root() -> Path:
    return Path(__file__).resolve().parents[1]


def run(
    repos: list[str],
    live: bool = False,
    now: dt.datetime | None = None,
    roadmap_root: Path | None = None,
) -> dict:
    now = now or _utcnow_dt()
    root = roadmap_root or _default_roadmap_root()
    # Writes need a token; without one, --live degrades to dry-run
    # reporting rather than a run of failed writes.
    if live and not (os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN")):
        live = False
    epics = section_epics(repos, live, now, MAX_ACTIONS_PER_RUN)
    remaining = MAX_ACTIONS_PER_RUN - len(epics["actions"])
    roadmaps = section_roadmaps(repos, live, now, remaining, root)
    remaining -= len(roadmaps.get("actions", []))
    projects = section_projects(repos, live, now, remaining)
    remaining -= len(projects.get("actions", []))
    stalled = section_stalled_prs(repos, live, now, remaining)
    remaining -= len(stalled.get("actions", []))
    loose = section_loose_ends(repos, live, now, remaining)
    sections = [epics, roadmaps, projects, stalled, loose]
    log = {
        "job": "program-tending",
        "slice": 3,
        "issue": "developing-today/code#1352",
        "run_at": _utcnow(),
        "mode": "live" if live else "dry-run",
        "repos": repos,
        "hard_stops": list(HARD_STOPS),
        "out_of_scope": list(OUT_OF_SCOPE),
        "sections": sections,
    }
    log["actions_taken"] = sum(len(s.get("actions", [])) for s in sections)
    return log


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(
        description="program-tending runner (slices 1+2+3 of 3)"
    )
    parser.add_argument(
        "--run", action="store_true", help="execute one run and print the log"
    )
    parser.add_argument(
        "--live",
        action="store_true",
        help="perform the actions (comments, drift label, drift PR); "
        "default is dry-run reporting",
    )
    parser.add_argument(
        "--repo", action="append", default=[], help="limit to owner/name (repeatable)"
    )
    args = parser.parse_args(argv)
    if not args.run:
        parser.print_help()
        return 2
    repos = args.repo or list(FLEET_REPOS)
    print(json.dumps(run(repos, live=args.live), indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
