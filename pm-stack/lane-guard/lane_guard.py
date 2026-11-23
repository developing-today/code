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

Three checks, all pure functions over GitHub JSON so they are testable
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

            A triage verdict (state `triaged`, ideally with a recorded
            `--verdict` token) is skipped when the newest agent note on
            the item already records the same verdict — across task
            ids, with no time window — and nothing has changed since:
            no human comment, no commit or label change on the issue
            (read from its timeline), and no body change (a fingerprint
            of the body recorded in the verdict note, compared with the
            issue's current body). A verdict stands until the issue
            changes; re-posting it is the repeat-triage churn
            #12233 removes (#1062 collected four identical verdicts).

  coverage  Is this issue covered by a recorded verdict (#12233 F2b,
            decision record on the issue)? An open `effort/l` or
            `effort/m` issue whose newest agent note is a triage
            verdict with a recorded verdict token and a valid resume
            condition (`resume=<kind>:<target>` — a slice landed, a
            dependency closed) is covered: the verdict note is the
            park record for the triage disposition, so coverage must
            not enqueue it for triage again. The verdict covers only
            while it is the newest signal, under the same freshness
            rule as the verdict dedupe above.

Usage (exit 0 = go ahead, 3 = held / skip the note / covered, 2 =
usage or I/O error; JSON verdict on stdout either way):

  lane_guard.py hold --repo developing-today/code --issue 7616
  lane_guard.py note --repo developing-today/code --issue 7616 \
      --task 20028 --state parked
  lane_guard.py note --repo developing-today/code --issue 1062 \
      --task 2334 --state triaged --verdict still_blocked
  lane_guard.py coverage --repo developing-today/code --issue 1062
"""

from __future__ import annotations

import argparse
import hashlib
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

# States that record a triage verdict. A verdict note is deduped by its
# verdict, not by task or age: see note_verdict's verdict path.
VERDICT_STATES = frozenset({"triaged"})

# Effort classes a recorded verdict can cover (#12233 F2b, decision
# record on the issue): a verdict parks large and medium issues; small
# ones are meant to be picked up, not parked by a verdict.
COVERAGE_EFFORT_LABELS = frozenset({"effort/l", "effort/m"})

# Placeholder tokens that record no outcome: a note carrying one has
# not triaged anything, so it cannot cover the issue.
NON_VERDICTS = frozenset({"none", "unknown", "untriaged", "needs_triage"})

# Resume-condition kinds a verdict park may wait on (F2b decision
# record, rule 3): an event a later pass can check, never a date or a
# person. Recorded as `resume=<kind>:<target>`.
RESUME_KINDS = frozenset({"slice_landed", "dependency_closed"})

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
# The recorded verdict of a triage note: `verdict: still_blocked` /
# `verdict=still_blocked` in the header area. A short token, so two
# lanes recording the same outcome produce the same key. The value is
# a run of words on ONE line: whitespace inside it is horizontal
# only, so a YAML note whose verdict is the last field never swallows
# the blank line and narrative below it into the verdict. A word
# followed by `=` or `:` ends the value, so a later header field
# (`body_fp=...`, `resume=...`, `task=...`) is never swallowed into
# it either.
_VERDICT_RE = re.compile(
    r"\bverdict[ \t]*[:=][ \t]*([A-Za-z0-9][\w-]*(?:[ \t]+(?![\w-]+\s*[:=])[\w-]+)*)",
    re.IGNORECASE,
)
# The body fingerprint of a triage note: `body_fp: <16 hex>` /
# `body_fp=<16 hex>` in the header area. See body_fingerprint.
_BODY_FP_RE = re.compile(r"\bbody_fp\s*[:=]\s*([0-9a-f]{16})\b", re.IGNORECASE)
# The resume condition of a verdict note: the `resume` field's value
# runs to the end of its line (or the header), and parse_resume picks
# the condition out of it.
_RESUME_FIELD_RE = re.compile(r"\bresume\s*[:=]\s*([^\n;]+)", re.IGNORECASE)
_RESUME_VALUE_RE = re.compile(r"([A-Za-z][A-Za-z_ -]*?)\s*(?::|#|\s)\s*#?(\d+)\b")


def parse_resume(raw) -> str | None:
    """Canonical `kind:target` for a resume condition, or None when
    the text is not one: the kind must be a checkable event
    (RESUME_KINDS) and the target an issue/PR number. A date, a
    person, or a kind with no target is not a resume condition — a
    park nothing can check is a stall (F2b decision record, rule 3).
    Accepts `dependency_closed:539`, `slice landed #12543`, and the
    raw field value a note carries after `resume=`/`resume:`."""
    if not isinstance(raw, str):
        return None
    m = _RESUME_VALUE_RE.search(raw.strip())
    if not m:
        return None
    kind = _norm_verdict(m.group(1))
    if kind not in RESUME_KINDS:
        return None
    return f"{kind}:{m.group(2)}"
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


def _norm_verdict(raw: str) -> str:
    """Canonical verdict key: `Still blocked!` and `still_blocked` are
    the same verdict."""
    return re.sub(r"[^a-z0-9]+", "_", str(raw).lower()).strip("_")


def body_fingerprint(body: str) -> str:
    """First 16 hex chars of the sha256 of the issue body, normalised
    for line endings and surrounding whitespace. The fingerprint is
    what a verdict note records, so a later lane can tell whether the
    body changed without the timeline ever saying so (GitHub's REST
    timeline exposes no `edited` event: see
    https://docs.github.com/en/rest/using-the-rest-api/issue-event-types,
    which lists no such event type)."""
    text = str(body).replace("\r\n", "\n").strip()
    return hashlib.sha256(text.encode("utf-8")).hexdigest()[:16]


def parse_note(body: str) -> dict | None:
    """{"state", "task"} for an agent note body, None for other comments.

    Accepts the shapes lanes actually post: the YAML-ish header
    (`state: parked` / `task: 20028`), the HTML-comment form
    (`<!-- agent-note:v1 state=parked task=20028 -->`), arrows
    (`transition: picked-up -> parked`), and free-form first lines
    (`**Pickup — queue task 20028**`). State is None when nothing
    recognisable is present; such a note never counts as a duplicate.
    A triage note that records a `verdict:`/`verdict=` field also
    carries a normalised "verdict" key, and one that records a
    `body_fp:`/`body_fp=` field carries a "body_fp" key, and one that
    records a valid `resume:`/`resume=` condition a canonical
    "resume" key (`kind:target`, see parse_resume); an invalid resume
    condition is not carried, so coverage can treat its absence as
    "no condition".
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
    note = {"state": state or None, "task": m_task.group(1) if m_task else None}
    m_verdict = _VERDICT_RE.search(head)
    if m_verdict:
        verdict = _norm_verdict(m_verdict.group(1))
        if verdict:
            note["verdict"] = verdict
    m_body_fp = _BODY_FP_RE.search(head)
    if m_body_fp:
        note["body_fp"] = m_body_fp.group(1).lower()
    m_resume = _RESUME_FIELD_RE.search(head)
    if m_resume:
        resume = parse_resume(m_resume.group(1))
        if resume:
            note["resume"] = resume
    return note


def note_header(task: str, state: str, verdict: str | None = None,
                body: str | None = None, resume: str | None = None) -> str:
    """Canonical first line for a lane's agent note, so later dedupe
    checks parse it exactly. A triage note passes its verdict so the
    next lane's check can match it, and — when the issue body is
    known — its body fingerprint, so a later lane can tell whether
    the body changed since the verdict was recorded, and its resume
    condition so coverage (F2b) can read the park. The verdict stays
    last: its free-form value runs to the end of the header, so a
    field after it would be parsed as part of the verdict."""
    header = f"<!-- agent-note:v1 state={state} task={str(task).lstrip('#')}"
    if resume:
        header += f" resume={parse_resume(resume) or _norm_verdict(resume)}"
    if body is not None:
        header += f" body_fp={body_fingerprint(body)}"
    if verdict:
        header += f" verdict={_norm_verdict(verdict)}"
    return header + " -->"


def _ts(value):
    if isinstance(value, datetime):
        return value if value.tzinfo else value.replace(tzinfo=timezone.utc)
    return datetime.fromisoformat(str(value).replace("Z", "+00:00"))


# Timeline event -> the kind of issue change it proves, for verdict
# freshness. `commented` is deliberately absent: comments are judged
# from the comment list itself, where human vs bot is visible.
# There is deliberately no body entry here: GitHub's REST timeline
# exposes no `edited` event (see body_fingerprint), so body changes
# are detected by fingerprint, not from the timeline.
_TIMELINE_CHANGES = {"labeled": "label", "unlabeled": "label"}


def changes_from_timeline(events: list) -> list:
    """[{"kind", "at"}] for the issue changes a verdict must not
    outlive: a commit referenced on the issue, a label change. Input
    is REST timeline JSON (oldest first); a `referenced` event only
    counts as a commit when it carries a commit_id (a cross-reference
    from another issue is not one). Body edits are not in this list:
    they are detected by comparing body fingerprints (see
    _verdict_note_verdict), because the timeline never reports them.
    """
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


def _verdict_key(state, verdict) -> str:
    """The comparable key of a verdict: the recorded verdict token
    when there is one, else the state it was recorded under. A note
    with a recorded verdict never matches one without (or vice
    versa): the recorded form is what later checks — and coverage —
    read."""
    if verdict:
        return f"verdict:{verdict}"
    return f"state:{state}"


def _is_verdict_request(state, verdict) -> bool:
    if verdict:
        return True
    state = str(state).lower().replace("-", "_")
    return state in VERDICT_STATES or _norm_state(state) in VERDICT_STATES


def _verdict_note_verdict(comments, state, verdict, changes, now, body=None, resume=None) -> dict:
    """The verdict path of note_verdict (#12233 F2a): a triage verdict
    is skipped when the newest agent note already records the same
    verdict — for any task id, however long ago — and nothing has
    changed since: no human comment, no commit or label change (from
    `changes`, see changes_from_timeline) after it, and no body
    change. Body changes are detected by fingerprint: `body` is the
    issue's current body text, compared with the `body_fp` recorded
    in the verdict note. When `body` is not supplied the fingerprint
    check is skipped (the timeline cannot supply it); a recorded
    verdict with no fingerprint cannot prove the body unchanged, so
    with a current body supplied it posts once, and the new note
    records a fingerprint. A newly supplied resume condition still
    posts: when the stored verdict note lacks the requested
    condition, or records a different one, the note is the upgrade
    that gives the verdict its F2b coverage, so dedupe must not
    swallow it.
    """
    want = _verdict_key(state, verdict)
    current_fp = body_fingerprint(body) if body is not None else None
    want_resume = parse_resume(resume) if resume else None
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
        if note["state"] not in VERDICT_STATES and not note.get("verdict"):
            return {
                "post": True,
                "reason": f"newest agent note is not a triage verdict (it was {note['state']})",
            }
        have = _verdict_key(note["state"], note.get("verdict"))
        if have != want:
            return {
                "post": True,
                "reason": f"newest recorded verdict is {have}, this is {want}",
            }
        if current_fp is not None:
            stored_fp = note.get("body_fp")
            if stored_fp is None:
                return {
                    "post": True,
                    "reason": (
                        "the recorded verdict carries no body fingerprint, "
                        "so a body edit since the verdict cannot be ruled "
                        "out; re-triage once to record one"
                    ),
                }
            if stored_fp != current_fp:
                return {
                    "post": True,
                    "reason": (
                        "the issue body changed since the verdict "
                        f"(body fingerprint {stored_fp} -> {current_fp})"
                    ),
                }
        since = _ts(c.get("created_at"))
        for ch in changes or ():
            if _ts(ch["at"]) > since:
                return {
                    "post": True,
                    "reason": (
                        f"the issue changed since the verdict: a {ch['kind']} "
                        f"change at {ch['at']}"
                    ),
                }
        if want_resume is not None and note.get("resume") != want_resume:
            return {
                "post": True,
                "reason": (
                    "resume condition added or changed: the newest verdict "
                    f"records {note.get('resume') or 'no resume condition'}, "
                    f"this note carries {want_resume}; post to record the "
                    "F2b resume condition"
                ),
            }
        age = now - since
        return {
            "post": False,
            "reason": (
                f"duplicate verdict: newest agent note already records {want} "
                f"for this issue (task {note['task']}, "
                f"{int(age.total_seconds() // 60)} min ago) and no commit, "
                "label, body or human comment has changed since; record "
                "the outcome on the queue task instead of re-triaging"
            ),
        }
    return {"post": True, "reason": "no earlier agent note"}


def note_verdict(
    comments: list, task: str, state: str, now=None, verdict=None, changes=(),
    body=None, resume=None,
) -> dict:
    """Should a note of `state` for queue `task` be posted, given the
    item's comments (oldest first, REST shape: body, created_at,
    user.type)?

    A triage verdict (state `triaged`, or any note with a `verdict`)
    takes the verdict path: `changes` is the issue's change list from
    changes_from_timeline, and `body` is the issue's current body
    text when known, fingerprinted against the verdict note (see
    _verdict_note_verdict); `resume` is the resume condition the note
    would record; a verdict that already stands still posts when the
    condition adds or changes the stored one. Everything else takes
    the park path.
    """
    now = _ts(now) if now is not None else datetime.now(timezone.utc)
    state = str(state).lower().replace("-", "_")
    task = str(task).lstrip("#")
    verdict = _norm_verdict(verdict) if verdict else None
    if _is_verdict_request(state, verdict):
        return _verdict_note_verdict(comments, state, verdict, changes, now, body, resume)
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


# --- verdict coverage (#12233 F2b) ---------------------------------------


def coverage_verdict(item: dict, comments: list, changes=(), resume_fulfilled=None) -> dict:
    """Is this issue covered by a recorded triage verdict?

    The F2b decision record on #12233: an open `effort/l` or
    `effort/m` issue whose newest agent note is a triage verdict with
    a recorded verdict token and a valid resume condition is covered
    — the verdict note is the park record for the triage disposition,
    so coverage must not enqueue the issue for triage again. The
    verdict covers only while it is the newest signal, under the same
    freshness rule as the F2a dedupe: a human comment, a newer agent
    note of any other state, or a commit, label or body change after
    the verdict (from `changes`, see changes_from_timeline) ends the
    coverage.

    `resume_fulfilled`, when given, is a callable taking the
    canonical resume condition (`kind:target`) and returning True
    when the wait is over (the dependency closed, the slice landed),
    False when its target is known not to be, or None when the
    state cannot be read. A fulfilled condition ends coverage: the
    issue is due for re-triage. An unreadable state never ends it.

    `item` is one issue JSON object (REST or `gh` shape), `comments`
    its comments oldest first. Returns {"covered": bool, "reason":
    str} plus "verdict" / "resume" when a verdict note was found.
    """
    state = str(item.get("state") or "").lower()
    if state and state != "open":
        return {"covered": False, "reason": f"issue is not open ({state})"}
    labels = set(_names(item.get("labels"), "name"))
    if not labels & COVERAGE_EFFORT_LABELS:
        return {
            "covered": False,
            "reason": "no effort/l or effort/m label: a verdict parks "
            "only large or medium issues",
        }
    for c in reversed(comments or []):
        note = parse_note(c.get("body"))
        if note is None:
            user = c.get("user") or {}
            if user.get("type") == "Bot":
                continue  # bots (CI, sweepers) are not news
            return {
                "covered": False,
                "reason": "a non-agent comment arrived since the last agent note",
            }
        if note["state"] not in VERDICT_STATES or not note.get("verdict"):
            return {
                "covered": False,
                "reason": "newest agent note is not a recorded triage "
                f"verdict (it was {note['state']})",
            }
        verdict = note["verdict"]
        if verdict in NON_VERDICTS:
            return {
                "covered": False,
                "verdict": verdict,
                "reason": f"recorded verdict {verdict} records no outcome",
            }
        if not note.get("resume"):
            return {
                "covered": False,
                "verdict": verdict,
                "reason": "the verdict note carries no valid resume "
                "condition (resume=<slice_landed|dependency_closed>:"
                "<issue-or-pr-number>)",
            }
        if resume_fulfilled is not None:
            try:
                fulfilled = resume_fulfilled(note["resume"])
            except Exception:
                fulfilled = None  # unreadable: the wait is not over
            if fulfilled:
                return {
                    "covered": False,
                    "verdict": verdict,
                    "resume": note["resume"],
                    "reason": "the resume condition "
                    f"{note['resume']} is fulfilled; the issue is "
                    "due for re-triage",
                }
        since = _ts(c.get("created_at"))
        for ch in changes or ():
            if _ts(ch["at"]) > since:
                return {
                    "covered": False,
                    "verdict": verdict,
                    "resume": note["resume"],
                    "reason": "the issue changed since the verdict: a "
                    f"{ch['kind']} change at {ch['at']}",
                }
        return {
            "covered": True,
            "verdict": verdict,
            "resume": note["resume"],
            "reason": f"verdict-covered: verdict {verdict} recorded for "
            f"task {note['task']} with resume condition {note['resume']}; "
            "the verdict note is the park record — do not re-enqueue "
            "for triage",
        }
    return {"covered": False, "reason": "no earlier agent note"}


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
    n.add_argument(
        "--verdict",
        default=None,
        help="the triage verdict this note records (state triaged); "
        "an unchanged issue with the same verdict already recorded "
        "skips the note",
    )
    n.add_argument(
        "--resume",
        default=None,
        help="the resume condition this verdict parks on (state "
        "triaged): slice_landed:<pr> or dependency_closed:<issue>; "
        "a verdict note with a valid condition covers the issue "
        "for coverage (F2b)",
    )
    c = sub.add_parser(
        "coverage", help="is this issue covered by a recorded verdict?"
    )
    c.add_argument("--repo", required=True)
    c.add_argument("--issue", required=True, type=int)
    args = ap.parse_args(argv)

    try:
        if args.cmd == "hold":
            item = gh(f"repos/{args.repo}/issues/{args.issue}")
            verdict = hold_verdict(item)
            code = EXIT_STOP if verdict["held"] else EXIT_GO
        elif args.cmd == "coverage":
            item = gh(f"repos/{args.repo}/issues/{args.issue}")
            comments = gh(
                f"repos/{args.repo}/issues/{args.issue}/comments?per_page=100"
            )
            timeline = gh(
                f"repos/{args.repo}/issues/{args.issue}/timeline?per_page=100"
            )

            # The verdict's wait is over when its resume target reads
            # fulfilled live: a closed dependency, a merged slice (a
            # PR closed without merging did not land it). An
            # unreadable target is unknown, never fulfilled.
            def _resume_fulfilled(resume):
                try:
                    kind, _, target = resume.partition(":")
                    if kind == "dependency_closed":
                        target_item = gh(f"repos/{args.repo}/issues/{target}")
                        return (target_item or {}).get("state") == "closed"
                    target_pr = gh(f"repos/{args.repo}/pulls/{target}")
                    return bool((target_pr or {}).get("merged_at"))
                except Exception:
                    return None

            verdict = coverage_verdict(
                item,
                comments,
                changes=changes_from_timeline(timeline),
                resume_fulfilled=_resume_fulfilled,
            )
            code = EXIT_STOP if verdict["covered"] else EXIT_GO
        else:
            comments = gh(
                f"repos/{args.repo}/issues/{args.issue}/comments?per_page=100"
            )
            changes = ()
            body = None
            if _is_verdict_request(args.state, args.verdict):
                # The issue body, for the verdict fingerprint: the
                # timeline never reports body edits, so the verdict
                # note records a fingerprint of the body it was
                # triaged against, and this check compares it. A
                # null body is a known-empty body, not an unknown
                # one: normalise it to "" so clearing the body
                # invalidates the stored fingerprint (and an
                # initially empty body still gets one). None stays
                # for a response that carries no body at all.
                item = gh(f"repos/{args.repo}/issues/{args.issue}")
                body = None
                if isinstance(item, dict):
                    # A null body is a known-empty body, not an
                    # unknown one: normalise it to "" so clearing the
                    # body invalidates the stored fingerprint (and an
                    # initially empty body still gets one).
                    body = item.get("body") or ""
                timeline = gh(
                    f"repos/{args.repo}/issues/{args.issue}/timeline?per_page=100"
                )
                changes = changes_from_timeline(timeline)
            verdict = note_verdict(
                comments,
                args.task,
                args.state,
                now=now,
                verdict=args.verdict,
                resume=args.resume,
                changes=changes,
                body=body,
            )
            verdict["header"] = note_header(
                args.task, args.state, verdict=args.verdict, body=body, resume=args.resume
            )
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
