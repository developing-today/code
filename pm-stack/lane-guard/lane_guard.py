#!/usr/bin/env python3
"""lane_guard.py — deterministic ownership and park-note checks for
queue lanes (OpenChamber scheduled lanes, Muse cron lanes, the #3026
claimant).

Why this exists: `.github/workflows/repo-sweeper.yml` assigned the
maintainer (`dezren39`, Drew) to every open issue and PR, and the lane
runbook says "never touch a held or Drew-owned PR". Lanes read the
blanket assignment as Drew ownership, parked the issue until an
"explicit ownership release/handoff" that nobody ever gives, and
re-posted the same park note on every 30-minute cycle (#7616 collected
50+ such notes in two days with no code).

Two checks, both pure functions over GitHub JSON so they are testable
offline, plus a thin `gh` CLI wrapper for lanes to call:

  hold      Is this issue/PR held for a human? Only an explicit marker
            counts: a hard-stop label (`held`, `user/owned`,
            `do-not-auto-edit`). Assignees never count, and neither does
            a `work/assigned` label (the queue's atomic claim, not a
            label, is the record of which worker holds a task).
            `do-not-merge` blocks merging only, not working.

  note      Should a lane post this agent note? A park/pick-up note is
            skipped when the newest agent note on the item is already a
            park/pick-up note for the same queue task, posted less than
            DEDUPE_WINDOW ago, with no human comment since. Progress
            notes (pr_opened, blocked, completed, ...) always post.

Usage (exit 0 = go ahead, 3 = held / skip the note, 2 = usage or I/O
error; JSON verdict on stdout either way):

  lane_guard.py hold --repo developing-today/code --issue 7616
  lane_guard.py note --repo developing-today/code --issue 7616 \
      --task 20028 --state parked
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from datetime import datetime, timedelta, timezone

# Explicit "a human holds this" markers. Kept equal to
# pm-stack/opencode-claimant/launch_one.py HARD_STOP minus the
# merge-only label (pinned by test_lane_guard.py).
HOLD_LABELS = frozenset({"held", "user/owned", "do-not-auto-edit"})
MERGE_ONLY_HOLD_LABELS = frozenset({"do-not-merge"})

# Labels and fields that look like ownership but are not a hold.
NOT_A_HOLD_LABELS = frozenset({"work/assigned"})

# Note states that carry no progress: re-posting them every cycle is
# the churn this guard removes.
QUIET_STATES = frozenset({"parked", "picked_up", "picked-up"})

DEDUPE_WINDOW = timedelta(hours=24)

EXIT_GO = 0
EXIT_USAGE = 2
EXIT_STOP = 3


def _names(items, key):
    out = []
    for it in items or []:
        if isinstance(it, dict):
            v = it.get(key)
        else:
            v = it
        if isinstance(v, str) and v:
            out.append(v)
    return out


def hold_verdict(item: dict) -> dict:
    """Verdict for one issue/PR JSON object (REST or `gh` shape).

    Returns {"held": bool, "merge_held": bool, "reason": str,
    "ignored": [...]} where `ignored` lists the ownership-looking
    signals that were deliberately not treated as a hold, so a lane can
    say so in its note instead of parking on them.
    """
    labels = set(_names(item.get("labels"), "name"))
    assignees = _names(item.get("assignees"), "login")
    state = str(item.get("state") or "").lower()

    ignored = []
    if assignees:
        ignored.append("assignees:" + ",".join(sorted(assignees)))
    for lab in sorted(labels & NOT_A_HOLD_LABELS):
        ignored.append("label:" + lab)

    hits = sorted(labels & HOLD_LABELS)
    merge_hits = sorted(labels & MERGE_ONLY_HOLD_LABELS)
    if state and state != "open":
        reason = f"not open ({state})"
        held = True
    elif hits:
        reason = "hold label: " + ",".join(hits)
        held = True
    else:
        reason = "available: no hold label"
        held = False
    return {
        "held": held,
        "merge_held": bool(hits or merge_hits),
        "reason": reason,
        "ignored": ignored,
    }


# --- park-note dedupe ---------------------------------------------------

_MARKER = re.compile(r"agent-note[:-]v1|\*\*Agent note\*\*", re.IGNORECASE)
# `state: parked`, `state=parked`, `transition: picked-up -> parked`,
# `kind: parked`; the value after the last arrow wins.
_FIELD_RE = re.compile(
    r"(?:^|[\s;(<-])(?:state|transition|kind)\s*[:=]\s*((?:[\w -]+?\s*->\s*)*[A-Za-z_ -]+)",
    re.IGNORECASE | re.MULTILINE,
)
_TASK_RE = re.compile(r"\btask(?:_id)?\s*[:=]?\s*#?(\d{3,})", re.IGNORECASE)
# Lanes write notes free-form; these map the words they use to states.
_STATE_WORDS = (
    (
        "picked_up",
        re.compile(r"^\s*(?:picked[ _-]?up|pick[ _-]?up|claimed|resum)", re.IGNORECASE),
    ),
    ("parked", re.compile(r"^\s*park", re.IGNORECASE)),
)
# First-line fallbacks for notes with no state field ("**Pickup —",
# "- picked_up: code-f", "picked up by repo-worker-code").
_HEAD_WORDS = (
    ("parked", re.compile(r"\bpark(?:ed)?\b", re.IGNORECASE)),
    ("picked_up", re.compile(r"\b(?:picked[ _-]?up|pickup)\b", re.IGNORECASE)),
)


def _norm_state(raw: str) -> str:
    value = raw.split("->")[-1].strip()
    for state, pat in _STATE_WORDS:
        if pat.search(value):
            return state
    word = re.match(r"[A-Za-z_-]+", value)
    return word.group(0).lower().replace("-", "_") if word else ""


def parse_note(body: str) -> dict | None:
    """{"state", "task"} for an agent note body, None for other comments.

    Accepts the shapes lanes actually post: the YAML-ish header
    (`state: parked` / `task: 20028`), the HTML-comment form
    (`<!-- agent-note:v1 state=parked task=20028 -->`), arrows
    (`transition: picked-up -> parked`), and free-form first lines
    (`**Pickup — queue task 20028**`). State is None when nothing
    recognisable is present; such a note never counts as a duplicate.
    """
    if not isinstance(body, str) or not _MARKER.search(body):
        return None
    head = body[:600]
    state = None
    for m in _FIELD_RE.finditer(head):
        state = _norm_state(m.group(1))
        if state:
            break
    if not state:
        lines = [
            ln
            for ln in head.splitlines()
            if ln.strip() and not _MARKER.fullmatch(ln.strip(" <!->"))
        ]
        first = lines[0] if lines else ""
        state = next((s for s, pat in _HEAD_WORDS if pat.search(first)), None)
    m_task = _TASK_RE.search(head)
    return {"state": state or None, "task": m_task.group(1) if m_task else None}


def note_header(task: str, state: str) -> str:
    """Canonical first line for a lane's agent note, so later dedupe
    checks parse it exactly."""
    return f"<!-- agent-note:v1 state={state} task={str(task).lstrip('#')} -->"


def _ts(value):
    if isinstance(value, datetime):
        return value if value.tzinfo else value.replace(tzinfo=timezone.utc)
    return datetime.fromisoformat(str(value).replace("Z", "+00:00"))


def note_verdict(comments: list, task: str, state: str, now=None) -> dict:
    """Should a note of `state` for queue `task` be posted, given the
    item's comments (oldest first, REST shape: body, created_at,
    user.type)?
    """
    now = _ts(now) if now is not None else datetime.now(timezone.utc)
    state = str(state).lower().replace("-", "_")
    task = str(task).lstrip("#")
    if state not in {s.replace("-", "_") for s in QUIET_STATES}:
        return {"post": True, "reason": f"progress state {state} always posts"}

    for c in reversed(comments or []):
        note = parse_note(c.get("body"))
        if note is None:
            user = c.get("user") or {}
            if user.get("type") == "Bot":
                continue  # bots (CI, sweepers) are not news
            return {
                "post": True,
                "reason": "a non-agent comment arrived since the last agent note",
            }
        if note["task"] != task:
            return {"post": True, "reason": "newest agent note is for another task"}
        if note["state"] not in {"parked", "picked_up"}:
            return {"post": True, "reason": f"newest agent note was {note['state']}"}
        age = now - _ts(c.get("created_at"))
        if age >= DEDUPE_WINDOW:
            return {
                "post": True,
                "reason": "newest park note is older than the dedupe window",
            }
        return {
            "post": False,
            "reason": (
                f"duplicate: newest agent note is already {note['state']} for task "
                f"{task}, {int(age.total_seconds() // 60)} min ago; update the queue "
                "checkpoint instead of commenting"
            ),
        }
    return {"post": True, "reason": "no earlier agent note"}


# --- thin gh wrapper ----------------------------------------------------


def _gh_json(path: str):
    p = subprocess.run(
        ["gh", "api", "--paginate", "--slurp", path],
        capture_output=True,
        text=True,
        timeout=120,
        check=False,
    )
    if p.returncode != 0:
        raise RuntimeError(p.stderr.strip()[:300] or "gh api failed")
    pages = json.loads(p.stdout)
    if pages and isinstance(pages[0], list):
        return [x for page in pages for x in page]
    return pages[0] if pages else None


def main(argv=None, gh=_gh_json, now=None) -> int:
    ap = argparse.ArgumentParser(prog="lane_guard.py")
    sub = ap.add_subparsers(dest="cmd", required=True)
    h = sub.add_parser("hold", help="is this issue/PR held for a human?")
    h.add_argument("--repo", required=True)
    h.add_argument("--issue", required=True, type=int)
    n = sub.add_parser("note", help="should this agent note be posted?")
    n.add_argument("--repo", required=True)
    n.add_argument("--issue", required=True, type=int)
    n.add_argument("--task", required=True)
    n.add_argument("--state", required=True)
    args = ap.parse_args(argv)

    try:
        if args.cmd == "hold":
            item = gh(f"repos/{args.repo}/issues/{args.issue}")
            verdict = hold_verdict(item)
            code = EXIT_STOP if verdict["held"] else EXIT_GO
        else:
            comments = gh(
                f"repos/{args.repo}/issues/{args.issue}/comments?per_page=100"
            )
            verdict = note_verdict(comments, args.task, args.state, now=now)
            verdict["header"] = note_header(args.task, args.state)
            code = EXIT_GO if verdict["post"] else EXIT_STOP
    except (
        OSError,
        ValueError,
        KeyError,
        TypeError,
        RuntimeError,
        subprocess.SubprocessError,
    ) as e:  # exit 2: could not check; the lane decides as before
        verdict = {"error": str(e)[:300]}
        code = EXIT_USAGE
    print(json.dumps(verdict))
    return code


if __name__ == "__main__":
    sys.exit(main())
