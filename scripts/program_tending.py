#!/usr/bin/env python3
"""program-tending (issue #1352, slices 1+2 of 3).

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

Projects, stalled-PRs and loose-ends remain slice 3 stubs.

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
    "developing-today/nix",
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

_REF_RE = re.compile(r"(?<![\w/])#(\d+)")
_URL_REF_RE = re.compile(r"github\.com/([\w.-]+/[\w.-]+)/(?:issues|pull)/(\d+)")
_CHECKBOX_RE = re.compile(r"^\s*[-*]\s+\[([ xX])\]\s+(.*)$")
_LIST_ITEM_RE = re.compile(r"^\s*(?:[-*]|\d+\.)\s+\S")


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
    for match in _REF_RE.finditer(text):
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
    blocked = [
        c for c in open_children if "status/blocked" in c.get("labels", [])
    ]
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


def section_epics(
    repos: list[str], live: bool, now: dt.datetime, budget: int
) -> dict:
    checked: list[dict] = []
    actions: list[dict] = []
    cache: dict = {}
    for repo in repos:
        try:
            epics = _gh_api(
                f"/repos/{repo}/issues?state=open&labels=epic&per_page=100"
            )
        except Exception as exc:  # noqa: BLE001 - report, never fail
            checked.append({"repo": repo, "open_epics": "unverified", "error": str(exc)})
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
            if verified and all(c["state"] == "closed" for c in verified) \
                    and entry["children"]["unverified"] == 0:
                kind = "completion"
            elif silent_days >= EPIC_SILENCE_DAYS:
                kind = "status"
            if kind is None:
                entry["disposition"] = "verified: no action due"
                checked.append(entry)
                continue
            if kind == "status":
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
                posted = _gh_write(
                    f"/repos/{repo}/issues/{epic.get('number')}/comments",
                    {"body": _epic_comment(kind, epic, children)},
                )
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
        for lineno, line in enumerate(
            path.read_text().splitlines(), start=1
        ):
            if line.startswith("##"):
                kind = _section_kind(line)
                continue
            if kind == "neutral" or not _LIST_ITEM_RE.match(line):
                continue
            refs = _refs_in(line, HOME_REPO)
            if not refs:
                continue
            resolved = [
                (ref, states.get(ref, "unverified")) for ref in refs
            ]
            verified = [s for _, s in resolved if s in ("open", "closed")]
            if not verified:
                continue
            entry = {
                "file": str(path.relative_to(root)),
                "line": lineno,
                "text": line.strip()[:200],
                "refs": [f"{r[0]}#{r[1]}:{s}" for r, s in resolved],
            }
            if kind == "forward" and all(
                s == "closed" for _, s in resolved
            ):
                certain.append(entry)
            elif kind == "forward" and "closed" in verified \
                    and "open" in verified:
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
        f"- `{e['file']}` line {e['line']}: {e['text']} "
        f"(refs: {', '.join(e['refs'])})"
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
        ["git", "commit", "-m", "docs(roadmap): program-tending drift fix — remove landed forward items"],
        root,
    )
    _run_cmd(["git", "push", "-u", "origin", branch], root)
    proc = subprocess.run(
        [
            "gh", "pr", "create", "--repo", HOME_REPO, "--base", "main",
            "--head", branch, "--title",
            "docs(roadmap): program-tending drift fix (landed forward items)",
            "--body", body,
        ],
        cwd=str(root), capture_output=True, text=True, check=True,
    )
    return {"kind": "roadmap-drift-pr", "branch": branch,
            "pr": proc.stdout.strip().splitlines()[-1] if proc.stdout else "",
            "items_removed": len(certain), "live": True}


def section_roadmaps(
    repos: list[str], live: bool, now: dt.datetime, budget: int, root: Path
) -> dict:
    checked: list[dict] = []
    actions: list[dict] = []
    for repo in repos:
        if repo != HOME_REPO:
            checked.append(
                {"repo": repo,
                 "drift": "not evaluated: roadmap files live in the checked-out home repo"}
            )
    if HOME_REPO not in repos:
        return {
            "section": "roadmaps", "status": "no-action",
            "detail": "home repo not in this run's repo set",
            "checked": checked, "actions": actions,
        }
    files = _roadmap_files(root)
    if not files:
        checked.append({"repo": HOME_REPO, "drift": "no roadmap files found"})
        return {
            "section": "roadmaps", "status": "no-action",
            "detail": "ROADMAP.md + planning/roadmaps vs landed work",
            "checked": checked, "actions": actions,
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
                actions.append(
                    _open_drift_pr(root, certain, now.strftime("%Y-%m-%d"))
                )
            except Exception as exc:  # noqa: BLE001 - report, never fail
                checked.append(
                    {"repo": HOME_REPO, "drift_pr_error": str(exc)[:300]}
                )
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
    sections = [
        epics,
        roadmaps,
        stub_projects(repos),
        stub_stalled_prs(repos),
        stub_loose_ends(repos),
    ]
    log = {
        "job": "program-tending",
        "slice": 2,
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
        description="program-tending runner (slices 1+2 of 3)"
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
