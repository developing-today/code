#!/usr/bin/env python3
"""program-tending runner (issue #1352, slice 2/3).

Daily scheduled care for the planning layer across the four fleet repos
(code, opencode, openchamber, nix). Slice 1 landed the scaffold (daily
13:00 UTC workflow + read-only runner, five verified no-action stubs).
Slice 2 implements the epics and roadmaps sections for real; projects,
stalled-PRs and loose-ends stay stubs until slice 3.

Ownership rules (from #1352; enforced by every section):
- never touch held / do-not-merge / user-owned PRs;
- Drew's own feature PRs get a check-in comment at 7 days, never a packet;
- user/review is reported, never re-decided.

Epics section: open `epic`-labelled issues get child progress (GitHub
sub-issues summary), last activity and body-vs-reality. An epic silent
7+ days gets one status comment (last movement, next step, blockers);
an epic with all children closed gets a completion comment proposing
closure. Never more than one comment per epic per run.

Roadmaps section: compares the checkout's ROADMAP.md + planning/roadmaps
tables against landed work (linked issue state/stateReason). A
forward-looking row whose linked issues all landed gets a concrete fix
(status -> shipped/dropped, never touching a Done section); anything
needing judgment (overview-list promotions, mixed open/closed links,
unresolvable links) gets `meta/roadmap-drift` + a comment naming the
uncertainty instead of a guessed PR.

Modes: `--run` is a dry run (read-only: no POST, no file edits; actions
are recorded in the log with dry_run=true). `--apply` performs the
writes (epic comments, drift labels + comments, roadmap file fixes) and
needs GH_TOKEN/GITHUB_TOKEN; the workflow opens the roadmap-drift PR
from the resulting file diff.

Usage: python3 -I scripts/program_tending.py --run [--apply] [--repo owner/name ...]
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import re
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

# Labels the runner must never act on (read-only signals).
HARD_STOPS = ("held", "user/owned", "do-not-merge", "do-not-auto-edit")

# The fleet monitor tends issues and queue state; this job reports the
# planning layer and never changes monitor or stewardship-cycle behavior.
OUT_OF_SCOPE = ("fleet monitor behavior", "stewardship-cycle behavior")

EPIC_LABEL = "epic"
EPIC_SILENCE_DAYS = 7
ROADMAP_DRIFT_LABEL = "meta/roadmap-drift"

# Slice 2 budget: each real section acts at most this often per run, and
# the whole run stays at ~12 actions (slice 3 inherits the run cap).
MAX_ACTIONS_PER_SECTION = 6
MAX_ACTIONS_PER_RUN = 12

# Roadmap table statuses that still promise future work. Anything else
# (shipped, dropped, done, ...) is terminal and never drift.
FORWARD_STATUSES = {"idea", "proposed", "shaped", "planned", "in-progress"}

# Closed-issue reason -> terminal row status. Decided by the issue's own
# stateReason, never guessed by the runner.
TERMINAL_BY_REASON = {"completed": "shipped", "not_planned": "dropped"}

# The checkout this script runs from (the workflow checks out the repo,
# so roadmap files are read locally, never through the API).
REPO_ROOT = Path(__file__).resolve().parents[1]


def _utcnow() -> str:
    return dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _parse_dt(value: object) -> dt.datetime | None:
    """Parse an ISO-8601 timestamp, tolerantly. None when unusable."""
    if not isinstance(value, str) or not value:
        return None
    text = value.strip()
    if text.endswith("Z"):
        text = text[:-1] + "+00:00"
    try:
        parsed = dt.datetime.fromisoformat(text)
    except Value :  # noqa: BLE001 - tolerant parse, None means unknown
        return None
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=dt.timezone.utc)
    return parsed


def _auth_headers() -> dict:
    headers = {"Accept": "application/vnd.github+json"}
    # The workflow hands this job a token (#6860): send it when present
    # so the reads/writes are authenticated; without one, behave exactly
    # as before. The value is only ever placed in the header — it is
    # never logged or included in any error or report string.
    token = os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN")
    if token:
        headers["Authorization"] = f"Bearer {token}"
    return headers


def _fetch(path: str, method: str = "GET", payload: dict | None = None) -> object:
    data = None
    headers = _auth_headers()
    if payload is not None:
        data = json.dumps(payload).encode("utf-8")
        headers["Content-Type"] = "application/json"
    req = urllib.request.Request(
        f"https://api.github.com{path}", data=data, headers=headers, method=method
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        return json.loads(resp.read().decode("utf-8"))


def _gh_api(path: str) -> list:
    """GET a GitHub REST collection, following pagination. Read-only."""
    items: list = []
    url = f"https://api.github.com{path}"
    while url:
        req = urllib.request.Request(url, headers=_auth_headers())
        with urllib.request.urlopen(req, timeout=30) as resp:
            items.extend(json.loads(resp.read().decode("utf-8")))
            nxt = resp.headers.get("Link", "")
        url = ""
        for part in nxt.split(","):
            if 'rel="next"' in part:
                url = part.split(";")[0].strip().strip("<>")
    return items


def _gh_api_one(path: str) -> dict:
    """GET a single GitHub REST object. Read-only; {} when unusable."""
    try:
        obj = _fetch(path)
    except Exception:  # noqa: BLE001 - callers treat failure as unverified
        return {}
    return obj if isinstance(obj, dict) else {}


def _gh_post(path: str, payload: dict) -> dict:
    """POST to the GitHub REST API. Only called in --apply mode."""
    try:
        obj = _fetch(path, method="POST", payload=payload)
    except Exception as exc:  # noqa: BLE001 - recorded on the action, never raised
        return {"ok": False, "error": str(exc)[:300]}
    if isinstance(obj, dict):
        return {"ok": True, **obj}
    return {"ok": True, "response": obj}


def _labels(item: dict) -> set[str]:
    return {label["name"] for label in item.get("labels", [])}


def _blocked_by_hard_stop(item: dict) -> bool:
    return bool(_labels(item) & set(HARD_STOPS))


# ---------------------------------------------------------------------------
# Epics section (slice 2).
# ---------------------------------------------------------------------------


def list_open_epics(repo: str, get=None) -> list[dict]:
    """Open `epic`-labelled issues for one repo, minus PRs and hard stops.

    Raises whatever `get` raises: the caller (tend_epics) counts the
    per-repo failure so an unreachable API can never read as clean."""
    get = globals()["_gh_api"] if get is None else get
    issues = get(f"/repos/{repo}/issues?state=open&labels={EPIC_LABEL}&per_page=100")
    epics = []
    for issue in issues:
        if not isinstance(issue, dict):
            continue
        if "pull_request" in issue:
            continue
        if _blocked_by_hard_stop(issue):
            continue
        epics.append(issue)
    return epics


def epic_child_summary(epic: dict) -> tuple[int, int] | None:
    """(total, completed) sub-issue counts, or None when not reported."""
    summary = epic.get("sub_issues_summary")
    if not isinstance(summary, dict):
        return None
    try:
        return (int(summary.get("total", 0)), int(summary.get("completed", 0)))
    except (TypeError, ValueError):
        return None


def epic_last_activity(epic: dict, comments: list | None = None) -> str:
    """Best-known last-movement timestamp (ISO string, may be empty)."""
    latest = _parse_dt(epic.get("updated_at"))
    if comments:
        for comment in comments:
            if not isinstance(comment, dict):
                continue
            seen = _parse_dt(comment.get("created_at") or comment.get("updated_at"))
            if seen and (latest is None or seen > latest):
                latest = seen
    if latest is None:
        return str(epic.get("updated_at") or "")
    return latest.strftime("%Y-%m-%dT%H:%M:%SZ")


def epic_blockers(epic: dict) -> str:
    """Blocker labels named on the epic, or an explicit none-recorded."""
    found = sorted(
        name for name in _labels(epic) if "blocked" in name or "hold" in name
    )
    if found:
        return ", ".join(f"`{name}`" for name in found)
    return "none recorded on the epic"


def epic_status_comment(
    epic: dict, days_silent: int, last_activity: str, children: tuple[int, int] | None
) -> str:
    number = epic.get("number", "?")
    if children is None:
        progress = "child progress is not reported on this epic yet"
        nxt = "confirm the child list so progress can be tracked"
    else:
        total, done = children
        progress = f"{total - done} of {total} sub-issues still open"
        nxt = (
            "work through the remaining open sub-issues (see the sub-issues tab)"
            if total - done > 0
            else "all sub-issues are closed — see the completion note"
        )
    return (
        f"Program tending (#1352): this epic has been silent for {days_silent}d. "
        f"Last movement: {last_activity or 'unknown'}. "
        f"Child progress: {progress}. "
        f"Next step: {nxt}. "
        f"Blockers: {epic_blockers(epic)}."
        f"\n\n(Epic #{number}: one status comment per run; no action taken beyond this note.)"
    )


def epic_completion_comment(epic: dict, children: tuple[int, int]) -> str:
    total, _ = children
    return (
        f"Program tending (#1352): all {total} sub-issues under this epic are "
        f"closed. Proposing closure — if the Definition of done holds, "
        f"closing this epic keeps the planning layer honest. "
        f"Blockers: {epic_blockers(epic)}."
        f"\n\n(Epic #{epic.get('number', '?')}: one completion comment per run; "
        f"the epic itself is left open for a human to close.)"
    )


def decide_epic_action(epic: dict, now: dt.datetime) -> dict | None:
    """One action (or None) for an open epic. Pure: no network, no writes."""
    number = epic.get("number")
    if not isinstance(number, int):
        return None
    updated = _parse_dt(epic.get("updated_at"))
    children = epic_child_summary(epic)
    # All children closed -> completion comment (even when recently active:
    # closure is the honest state, not a silence ping).
    if children is not None and children[0] > 0 and children[1] >= children[0]:
        return {
            "repo": epic.get("_repo", ""),
            "issue": number,
            "kind": "epic-completion",
            "detail": f"all {children[0]} sub-issues closed; proposing closure",
            "comment": epic_completion_comment(epic, children),
            "priority": (0, 0),
        }
    if updated is None:
        return None
    days_silent = (now - updated).days
    if days_silent < EPIC_SILENCE_DAYS:
        return None
    last_activity = epic_last_activity(epic)
    open_n = f"{children[0] - children[1]} of {children[0]} open" if children else "unknown"
    return {
        "repo": epic.get("_repo", ""),
        "issue": number,
        "kind": "epic-status",
        "detail": f"silent {days_silent}d (last {last_activity or 'unknown'}); children {open_n}",
        "comment": epic_status_comment(epic, days_silent, last_activity, children),
        "priority": (1, -days_silent),
    }


def issue_already_noted(repo: str, number: int, get, note_cache: dict) -> bool:
    """True when the latest comment on the issue is already a tending
    note: nothing new happened since we spoke, so re-pinging would spam.
    Lookup failures fail open (a duplicate note beats a missed one)."""
    key = (repo, number)
    if key not in note_cache:
        try:
            comments = get(f"/repos/{repo}/issues/{number}/comments?per_page=30")
        except Exception:  # noqa: BLE001 - fail open, attempt the note
            comments = []
        latest_ts = ""
        latest_is_tending = False
        if isinstance(comments, list):
            for comment in comments:
                if not isinstance(comment, dict):
                    continue
                body = comment.get("body")
                ts = str(comment.get("created_at") or comment.get("updated_at") or "")
                if ts >= latest_ts:
                    latest_ts = ts
                    latest_is_tending = (
                        isinstance(body, str) and body.startswith(TENDING_MARKER)
                    )
        note_cache[key] = latest_is_tending
    return note_cache[key]


def tend_epics(
    repos: list[str],
    now: dt.datetime | None = None,
    dry_run: bool = True,
    get=None,
    post=None,
) -> dict:
    """Tend open epics: status pings for the silent, closure proposals for
    the finished. At most MAX_ACTIONS_PER_SECTION actions per run and one
    comment per epic per run. Read-only when dry_run."""
    get = globals()["_gh_api"] if get is None else get
    post = globals()["_gh_post"] if post is None else post
    moment = now or dt.datetime.now(dt.timezone.utc)
    checked: list[dict] = []
    candidates: list[dict] = []
    errors = 0
    for repo in repos:
        try:
            epics = list_open_epics(repo, get=get)
        except Exception:  # noqa: BLE001 - per-repo failure never fails the run
            epics = []
            errors += 1
        for epic in epics:
            if not isinstance(epic.get("number"), int):
                continue
            if repo == TRACKING_REPO and epic["number"] == TRACKING_NUMBER:
                # The tending tracker tends; it is never itself pinged.
                checked.append(
                    {
                        "repo": repo,
                        "issue": epic["number"],
                        "title": str(epic.get("title", ""))[:120],
                        "decision": "tracking-issue",
                    }
                )
                continue
            epic = {**epic, "_repo": repo}
            children = epic_child_summary(epic)
            action = decide_epic_action(epic, moment)
            entry: dict = {
                "repo": repo,
                "issue": epic["number"],
                "title": str(epic.get("title", ""))[:120],
                "last_activity": epic_last_activity(epic),
                "children": (
                    {"total": children[0], "completed": children[1]}
                    if children
                    else "unreported"
                ),
                "decision": action["kind"] if action else "no-action",
            }
            if action is not None:
                entry["detail"] = action["detail"]
                candidates.append((action, entry))
            checked.append(entry)
    # Completions first, then the longest-silenced; cap the run budget and
    # record what the cap dropped so the log stays honest. Already-noted
    # epics (our note is still the latest word) are skipped, not re-pinged.
    candidates.sort(key=lambda pair: pair[0].pop("priority"))
    seen: set[tuple[str, int]] = set()
    note_cache: dict = {}
    final: list[dict] = []
    dropped = 0
    noted = 0
    for action, entry in candidates:
        key = (action["repo"], action["issue"])
        if key in seen:
            dropped += 1
            continue
        seen.add(key)
        if len(final) >= MAX_ACTIONS_PER_SECTION:
            dropped += 1
            continue
        if issue_already_noted(action["repo"], action["issue"], get, note_cache):
            entry["decision"] = "already-noted"
            noted += 1
            continue
        record = {k: v for k, v in action.items() if k != "comment"}
        record["dry_run"] = dry_run
        if not dry_run:
            resp = post(
                f"/repos/{action['repo']}/issues/{action['issue']}/comments",
                {"body": action["comment"]},
            )
            record["posted"] = bool(resp.get("ok"))
            if resp.get("ok"):
                record["comment_url"] = resp.get("html_url", "")
            else:
                record["error"] = str(resp.get("error", "post failed"))[:200]
        else:
            record["comment"] = action["comment"]
        final.append(record)
    if final:
        detail = (
            f"{len(final)} epic comment(s); {len(checked)} open epic(s) reviewed"
        )
    elif errors:
        detail = (
            f"unverified: {errors} repo(s) unreachable, "
            f"nothing reported as clean"
        )
    else:
        detail = f"{len(checked)} open epic(s) reviewed, none qualified"
    section: dict = {
        "section": "epics",
        "status": "action" if final else "no-action",
        "detail": detail,
        "checked": checked,
        "actions": final,
    }
    if dropped:
        section["dropped_due_to_cap"] = dropped
    if noted:
        section["already_noted"] = noted
    if errors:
        section["fetch_errors"] = errors
    return section


# ---------------------------------------------------------------------------
# Roadmaps section (slice 2).
# ---------------------------------------------------------------------------


def find_roadmap_files(root: Path | str | None = None) -> list[str]:
    """Roadmap files under a checkout root, relative paths, sorted."""
    base = Path(root) if root is not None else REPO_ROOT
    found: list[str] = []
    top = base / "ROADMAP.md"
    if top.is_file():
        found.append("ROADMAP.md")
    plans = base / "planning" / "roadmaps"
    if plans.is_dir():
        for path in sorted(plans.glob("*.md")):
            found.append(str(path.relative_to(base)))
    return found


def _heading(line: str) -> tuple[int, str] | None:
    match = re.match(r"^(#{1,6})\s+(.*?)\s*$", line)
    if not match:
        return None
    return (len(match.group(1)), match.group(2).strip())


def _split_row(line: str) -> list[str] | None:
    stripped = line.strip()
    if not stripped.startswith("|"):
        return None
    cells = [cell.strip() for cell in stripped.strip("|").split("|")]
    if len(cells) < 2:
        return None
    return cells


def _is_separator(cells: list[str]) -> bool:
    return all(re.fullmatch(r":?-{3,}:?", cell or "") is not None for cell in cells)


def parse_roadmap_rows(text: str) -> list[dict]:
    """Forward-looking roadmap rows with their links. Done sections are
    never returned, so the runner cannot rewrite history."""
    rows: list[dict] = []
    section = ""
    skip_level = 0
    lines = text.splitlines()
    idx = 0
    while idx < len(lines):
        head = _heading(lines[idx])
        if head is not None:
            level, title = head
            if skip_level and level <= skip_level:
                skip_level = 0
            if not skip_level and title.lower() == "done":
                skip_level = level
            if not skip_level:
                section = title
            idx += 1
            continue
        if skip_level:
            idx += 1
            continue
        cells = _split_row(lines[idx])
        header = cells is not None and any(c.lower() == "status" for c in cells)
        if header and idx + 1 < len(lines):
            sep = _split_row(lines[idx + 1])
            if sep is not None and _is_separator(sep):
                try:
                    status_at = next(
                        i for i, c in enumerate(cells) if c.lower() == "status"
                    )
                except StopIteration:
                    idx += 1
                    continue
                links_at = (
                    next(
                        i for i, c in enumerate(cells) if c.lower() == "links"
                    )
                    if any(c.lower() == "links" for c in cells)
                    else len(cells) - 1
                )
                idx += 2
                while idx < len(lines):
                    data = _split_row(lines[idx])
                    if data is None:
                        break
                    if _is_separator(data):
                        idx += 1
                        continue
                    status = data[status_at] if status_at < len(data) else ""
                    links = data[links_at] if links_at < len(data) else ""
                    title = data[0] if data else ""
                    if status.strip().lower() in FORWARD_STATUSES:
                        rows.append(
                            {
                                "line": idx + 1,
                                "section": section,
                                "title": title,
                                "status": status.strip(),
                                "links": links,
                                "raw": lines[idx],
                                "issues": [int(n) for n in ISSUE_REF_RE.findall(links)],
                                "status_at": status_at,
                            }
                        )
                    idx += 1
                continue
        idx += 1
    return rows


def parse_overview_lists(text: str) -> list[dict]:
    """Numbered-list drift candidates under ROADMAP.md Now/Next/Later.
    Done sections are never returned."""
    items: list[dict] = []
    section = ""
    skip_level = 0
    for lineno, line in enumerate(text.splitlines(), start=1):
        head = _heading(line)
        if head is not None:
            level, title = head
            if skip_level and level <= skip_level:
                skip_level = 0
            if not skip_level and title.lower() == "done":
                skip_level = level
            if not skip_level:
                section = title
            continue
        if skip_level:
            continue
        if section.lower().rstrip(", across the repo") not in ("now", "next", "later"):
            if not section.lower().startswith(("now", "next", "later")):
                continue
        match = re.match(r"^\s*\d+\.\s+(.*?)\s*$", line)
        if match:
            body = match.group(1)
            refs = [int(n) for n in ISSUE_REF_RE.findall(body)]
            if refs:
                items.append(
                    {"line": lineno, "section": section, "text": body, "issues": refs}
                )
    return items


def issue_state(
    repo: str, number: int, cache: dict, failures: set, get_one=None
) -> tuple[str, str]:
    """(state, state_reason) for an issue, cached per run. Unknowns are
    ('', '') so the caller treats the row as unverifiable, never drift.
    Every failed lookup is recorded in `failures` so the section can say
    it could not verify instead of reporting a clean it never earned."""
    get_one = globals()["_gh_api_one"] if get_one is None else get_one
    key = (repo, number)
    if key not in cache:
        try:
            obj = get_one(f"/repos/{repo}/issues/{number}")
        except Exception:  # noqa: BLE001 - unverifiable, not fatal
            obj = {}
        if isinstance(obj, dict) and obj:
            cache[key] = (
                str(obj.get("state", "") or ""),
                str(obj.get("state_reason", "") or ""),
            )
        else:
            cache[key] = ("", "")
            failures.add(key)
    return cache[key]


def rewrite_table_cell(line: str, idx: int, new_value: str) -> str:
    """Replace one cell of a markdown table row, keeping column padding."""
    stripped = line.rstrip("\n")
    leading = stripped[: len(stripped) - len(stripped.lstrip())]
    cells = stripped.strip().strip("|").split("|")
    if idx >= len(cells):
        return line
    width = max(len(cells[idx].strip()), len(new_value))
    cells[idx] = f" {new_value.ljust(width)} "
    return f"{leading}|{'|'.join(cells)}|"


def apply_roadmap_fix(
    root: Path | str | None, relpath: str, line_no: int, new_status: str
) -> bool:
    """Flip the Status cell of one roadmap table row on disk. Returns True
    when the file changed. The workflow turns the diff into the drift PR."""
    base = Path(root) if root is not None else REPO_ROOT
    path = base / relpath
    try:
        text = path.read_text()
    except OSError:
        return False
    lines = text.splitlines()
    if not (1 <= line_no <= len(lines)):
        return False
    rows = parse_roadmap_rows(text)
    match = next((r for r in rows if r["line"] == line_no), None)
    if match is None:
        return False
    new_line = rewrite_table_cell(lines[line_no - 1], match["status_at"], new_status)
    if new_line == lines[line_no - 1]:
        return False
    lines[line_no - 1] = new_line
    path.write_text("\n".join(lines) + "\n")
    return True


TRACKING_ISSUE = "developing-today/code#1352"
TRACKING_REPO = "developing-today/code"
TRACKING_NUMBER = 1352
TENDING_MARKER = "Program tending (#1352)"

# Same-repo issue refs only: `nix#203` and `owner/repo#203` name issues in
# another repo, which this checkout cannot verify. The lookbehind keeps
# those out so a foreign backlog number is never read as a local issue.
ISSUE_REF_RE = re.compile(r"(?<!\w)#(\d+)")

# A row carrying this marker is never auto-fixed: the owner kept it
# forward-looking on purpose (or its intent outruns its links), so the
# runner flags instead of guessing. Mark intent, e.g.:
#   <!-- program-tending: needs-owner -->
NEEDS_OWNER_RE = re.compile(r"program-tending\s*:\s*needs-owner", re.IGNORECASE)


def tend_roadmaps(
    repos: list[str] | None = None,
    root: Path | str | None = None,
    dry_run: bool = True,
    get_one=None,
    post=None,
) -> dict:
    """Compare roadmap rows against landed work. Concrete drift (every
    linked issue closed with a known reason) becomes a roadmap-pr action;
    judgment calls (overview promotions, mixed links, unknown reasons)
    become roadmap-flag actions carrying meta/roadmap-drift. Done rows
    are never parsed, so they can never be rewritten."""
    get_one = globals()["_gh_api_one"] if get_one is None else get_one
    post = globals()["_gh_post"] if post is None else post
    base = Path(root) if root is not None else REPO_ROOT
    repos = list(repos) if repos else list(FLEET_REPOS)
    checked: list[dict] = []
    actions: list[dict] = []
    cache: dict = {}
    failures: set = set()

    def room() -> bool:
        """Cap guard checked BEFORE any write: an action with no room is
        counted as dropped and never applied, posted or labelled, so the
        log can never under-report a mutation."""
        if len(actions) >= MAX_ACTIONS_PER_SECTION:
            push_dropped = getattr(room, "dropped", 0) + 1
            room.dropped = push_dropped  # type: ignore[attr-defined]
            return False
        return True

    room.dropped = 0  # type: ignore[attr-defined]
    label_cache: dict = {}
    tracker_flags: list[dict] = []

    def target_already_flagged(repo: str, open_issue: int | None) -> bool:
        """A target that already carries the drift label needs no new flag:
        re-flagging every run would spam the same issue daily."""
        number = open_issue if open_issue else TRACKING_NUMBER
        target_repo = repo if open_issue else TRACKING_REPO
        key = (target_repo, number)
        if key not in label_cache:
            try:
                obj = get_one(f"/repos/{target_repo}/issues/{number}")
            except Exception:  # noqa: BLE001 - fail open, attempt the flag
                obj = {}
            names = set()
            if isinstance(obj, dict):
                names = {l.get("name", "") for l in obj.get("labels", [])}
            label_cache[key] = ROADMAP_DRIFT_LABEL in names
        return label_cache[key]

    def flag(
        repo: str,
        relpath: str,
        row: dict,
        uncertainty: str,
        open_issue: int | None,
        file_note: dict,
    ) -> None:
        if target_already_flagged(repo, open_issue):
            file_note["already_flagged"] = file_note.get("already_flagged", 0) + 1
            return
        if open_issue is None:
            # Ambiguity with no owning issue is reported once per run on
            # the tracker, not once per row: the aggregate below takes
            # the single cap slot, so per-row room is not consulted here.
            tracker_flags.append(
                {
                    "repo": repo,
                    "file": relpath,
                    "line": row.get("line"),
                    "row": str(row.get("title") or row.get("text", ""))[:160],
                    "issues": row.get("issues", []),
                    "uncertainty": uncertainty,
                }
            )
            return
        if not room():
            file_note["dropped_due_to_cap"] = file_note.get("dropped_due_to_cap", 0) + 1
            return
        record = {
            "kind": "roadmap-flag",
            "repo": repo,
            "target": f"{repo}#{open_issue}" if open_issue else TRACKING_ISSUE,
            "label": ROADMAP_DRIFT_LABEL,
            "file": relpath,
            "line": row.get("line"),
            "row": str(row.get("title") or row.get("text", ""))[:160],
            "issues": row.get("issues", []),
            "uncertainty": uncertainty,
            "dry_run": dry_run,
        }
        if not dry_run:
            label_resp = post(
                f"/repos/{repo}/issues/{open_issue}/labels",
                {"labels": [ROADMAP_DRIFT_LABEL]},
            )
            comment_resp = post(
                f"/repos/{repo}/issues/{open_issue}/comments",
                {
                    "body": (
                        f"{TENDING_MARKER}: possible roadmap drift in "
                        f"`{relpath}` line {row.get('line')}: "
                        f"{record['row']}. Uncertainty: {uncertainty}. "
                        f"Not guessed — needs a human call."
                    )
                },
            )
            record["posted"] = bool(label_resp.get("ok") and comment_resp.get("ok"))
            if not record["posted"]:
                record["error"] = str(
                    label_resp.get("error") or comment_resp.get("error")
                )[:200]
        actions.append(record)

    def flush_tracker_flags() -> None:
        """One aggregate flag per run for every row needing a human call."""
        if not tracker_flags:
            return
        if not room():
            return
        lines = [
            f"- `{f['file']}` line {f['line']}: {f['row']} — {f['uncertainty']}"
            for f in tracker_flags
        ]
        record = {
            "kind": "roadmap-flag",
            "repo": TRACKING_REPO,
            "target": TRACKING_ISSUE,
            "label": ROADMAP_DRIFT_LABEL,
            "file": tracker_flags[0]["file"],
            "line": tracker_flags[0]["line"],
            "rows": [f["row"] for f in tracker_flags],
            "details": [
                {
                    "file": f["file"],
                    "line": f["line"],
                    "row": f["row"],
                    "uncertainty": f["uncertainty"],
                }
                for f in tracker_flags
            ],
            "files": sorted({f["file"] for f in tracker_flags}),
            "uncertainty": (
                f"{len(tracker_flags)} roadmap row(s) closed but kept "
                f"forward-looking; the promotion or drop needs a human call"
            ),
            "dry_run": dry_run,
        }
        if not dry_run:
            label_resp = post(
                f"/repos/{TRACKING_REPO}/issues/{TRACKING_NUMBER}/labels",
                {"labels": [ROADMAP_DRIFT_LABEL]},
            )
            comment_resp = post(
                f"/repos/{TRACKING_REPO}/issues/{TRACKING_NUMBER}/comments",
                {
                    "body": (
                        f"{TENDING_MARKER}: possible roadmap drift — "
                        f"{len(tracker_flags)} forward-looking row(s) whose "
                        f"linked work landed. No fix opened: each needs a "
                        f"human call on what promotes into its place.\n\n"
                        + "\n".join(lines)
                    )
                },
            )
            record["posted"] = bool(label_resp.get("ok") and comment_resp.get("ok"))
            if not record["posted"]:
                record["error"] = str(
                    label_resp.get("error") or comment_resp.get("error")
                )[:200]
        actions.append(record)

    if "developing-today/code" in repos:
        for relpath in find_roadmap_files(base):
            try:
                text = (base / relpath).read_text()
            except OSError as exc:
                checked.append({"file": relpath, "error": str(exc)[:200]})
                continue
            rows = parse_roadmap_rows(text)
            drift = 0
            unverifiable = 0
            file_note: dict = {"file": relpath}
            for row in rows:
                if not row["issues"]:
                    # Links name no same-repo issue (foreign `nix#N`
                    # refs, file paths, "none yet"): this checkout
                    # cannot verify the row, so it is counted, never
                    # guessed.
                    if row["links"].strip().lower() not in (
                        "",
                        "none",
                        "none yet",
                        "tbd",
                        "-",
                    ):
                        unverifiable += 1
                    continue
                states = [
                    issue_state("developing-today/code", n, cache, failures, get_one=get_one)
                    for n in row["issues"]
                ]
                if any(s == "" for s, _ in states):
                    unverifiable += 1
                    continue
                if any(s != "closed" for s, _ in states):
                    continue
                reasons = {r for _, r in states}
                if NEEDS_OWNER_RE.search(str(row.get("raw", ""))):
                    flag(
                        "developing-today/code",
                        relpath,
                        row,
                        (
                            "row is marked program-tending: needs-owner; its "
                            "linked issue(s) closed but the intent is kept "
                            "forward-looking on purpose"
                        ),
                        None,
                        file_note,
                    )
                    drift += 1
                    continue
                unknown = reasons - set(TERMINAL_BY_REASON)
                if unknown:
                    flag(
                        "developing-today/code",
                        relpath,
                        row,
                        (
                            f"linked issue(s) {sorted(row['issues'])} closed "
                            f"with no recorded close reason; shipped vs "
                            f"dropped is undecided"
                        ),
                        None,
                        file_note,
                    )
                    drift += 1
                    continue
                if len(reasons) > 1:
                    flag(
                        "developing-today/code",
                        relpath,
                        row,
                        (
                            f"linked issue(s) {sorted(row['issues'])} closed "
                            f"with disagreeing reasons {sorted(reasons)}; "
                            f"shipped vs dropped needs a call"
                        ),
                        None,
                        file_note,
                    )
                    drift += 1
                    continue
                reason = next(iter(reasons))
                new_status = TERMINAL_BY_REASON[reason]
                drift += 1
                if not room():
                    file_note["dropped_due_to_cap"] = (
                        file_note.get("dropped_due_to_cap", 0) + 1
                    )
                    continue
                record = {
                    "kind": "roadmap-pr",
                    "repo": "developing-today/code",
                    "file": relpath,
                    "line": row["line"],
                    "row": row["title"][:160],
                    "from_status": row["status"],
                    "to_status": new_status,
                    "issues": row["issues"],
                    "dry_run": dry_run,
                }
                if not dry_run:
                    record["applied"] = apply_roadmap_fix(
                        base, relpath, row["line"], new_status
                    )
                actions.append(record)
            # ROADMAP.md overview lists: any closed ref under Now/Next/Later
            # needs a promotion decision, so it is always a flag.
            overview_drift = 0
            if relpath == "ROADMAP.md":
                for item in parse_overview_lists(text):
                    states = [
                        issue_state("developing-today/code", n, cache, failures, get_one=get_one)
                        for n in item["issues"]
                    ]
                    closed = [
                        n
                        for n, (s, _) in zip(item["issues"], states)
                        if s == "closed"
                    ]
                    unknown = [n for n, (s, _) in zip(item["issues"], states) if s == ""]
                    if unknown and not closed:
                        unverifiable += 1
                        continue
                    if closed:
                        overview_drift += 1
                        flag(
                            "developing-today/code",
                            relpath,
                            item,
                            (
                                f"issue(s) {closed} closed but still listed under "
                                f"{item['section']}; what promotes into its place "
                                f"needs a human call"
                            ),
                            None,
                            file_note,
                        )
            file_note.update(
                {
                    "rows": len(rows),
                    "drift": drift + overview_drift,
                    "unverifiable": unverifiable,
                }
            )
            checked.append(file_note)
    flush_tracker_flags()
    for repo in repos:
        if repo == "developing-today/code":
            continue
        checked.append(
            {
                "repo": repo,
                "note": "deferred: roadmap files live in that repo's checkout; "
                "slice 2 covers the code checkout",
            }
        )
    if actions:
        detail = (
            f"{len(actions)} roadmap action(s) across "
            f"{sum(1 for c in checked if 'file' in c)} file(s)"
        )
    elif failures:
        detail = (
            f"unverified: {len(failures)} issue lookup(s) failed, "
            f"nothing reported as clean"
        )
    else:
        detail = "no forward-looking drift against landed work"
    section: dict = {
        "section": "roadmaps",
        "status": "action" if actions else "no-action",
        "detail": detail,
        "checked": checked,
        "actions": actions,
    }
    if room.dropped:  # type: ignore[attr-defined]
        section["dropped_due_to_cap"] = room.dropped  # type: ignore[attr-defined]
    if failures:
        section["lookup_failures"] = len(failures)
    return section


# ---------------------------------------------------------------------------
# Slice-1 stubs, kept for slice 3.
# ---------------------------------------------------------------------------


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


def enforce_run_cap(sections: list[dict]) -> int:
    """Trim section actions oldest-last so the whole run stays within
    MAX_ACTIONS_PER_RUN. Slice 3 inherits this; returns actions dropped."""
    kept = 0
    dropped = 0
    for section in sections:
        actions = section.get("actions") or []
        room = MAX_ACTIONS_PER_RUN - kept
        if room < 0:
            room = 0
        if len(actions) > room:
            dropped += len(actions) - room
            section["actions"] = actions[:room]
            section["dropped_due_to_run_cap"] = len(actions) - room
            section["status"] = "action" if room else "no-action"
        kept += min(len(actions), room)
    return dropped


def run(
    repos: list[str],
    dry_run: bool = True,
    now: dt.datetime | None = None,
    root: Path | str | None = None,
) -> dict:
    log = {
        "job": "program-tending",
        "slice": 2,
        "issue": "developing-today/code#1352",
        "run_at": _utcnow(),
        "repos": repos,
        "dry_run": dry_run,
        "hard_stops": list(HARD_STOPS),
        "out_of_scope": list(OUT_OF_SCOPE),
        "sections": [
            tend_epics(repos, now=now, dry_run=dry_run),
            tend_roadmaps(repos, root=root, dry_run=dry_run),
            stub_projects(repos),
            stub_stalled_prs(repos),
            stub_loose_ends(repos),
        ],
    }
    log["actions_taken"] = sum(len(s.get("actions", [])) for s in log["sections"])
    log["dropped_due_to_run_cap"] = enforce_run_cap(log["sections"])
    log["actions_taken"] = sum(len(s.get("actions", [])) for s in log["sections"])
    return log


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(
        description="program-tending runner (slice 2/3: epics + roadmaps real)"
    )
    parser.add_argument(
        "--run", action="store_true", help="execute one run and print the log"
    )
    parser.add_argument(
        "--apply",
        action="store_true",
        help="perform writes (comments, labels, roadmap fixes); needs a token",
    )
    parser.add_argument(
        "--repo", action="append", default=[], help="limit to owner/name (repeatable)"
    )
    args = parser.parse_args(argv)
    if not args.run and not args.apply:
        parser.print_help()
        return 2
    if args.apply and not (_auth_headers().get("Authorization")):
        print("refusing --apply without GH_TOKEN/GITHUB_TOKEN", file=sys.stderr)
        return 2
    repos = args.repo or list(FLEET_REPOS)
    print(json.dumps(run(repos, dry_run=not args.apply), indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
