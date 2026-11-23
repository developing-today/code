#!/usr/bin/env python3
"""nudges.py - stall nudges + escalation over babysitter verdicts (#1187).

The actuation half of the babysitter loop (#1234's scope: "progress
verdicts, one stall nudge then escalation"). babysitter.py (#1338)
classifies; it deliberately drives nothing. This module consumes a
round record's CONFIRMED stalls and acts on them:

- a bound session in a confirmed stall gets a nudge - a prompt
  injected through the claimant's verified delivery transport -
  at most MAX_NUDGES times per stall episode;
- a bound session still confirmed-stalled after its nudges are
  spent is escalated to the fleet as a stuck-holder case: an
  agent-note on the bound task's issue, once per episode;
- an unbound session is observed, never driven (babysitter.py's
  rule): its confirmed stall is recorded, nothing is sent.

An episode is a run of consecutive rounds in which the session
reads confirmed-stalled. Episode identity does not need the
activity stamp here: the babysitter's own confirmation already
keys on it (a moved session loses confirmation for at least one
round - suspect precedes confirmed - which closes the episode in
this module's state too). Nudge state persists between fires in
nudge-state.json in the claimant state dir:

    {"sessions": {"<session id>": {"task_id": int,
                                   "nudges_sent": int,
                                   "escalated": bool,
                                   "first_round": int,
                                   "last_round": int}}}

An entry drops the first round its session is not confirmed-
stalled, so a later, separate stall starts a fresh episode with
a fresh nudge budget.

Delivery discipline, matching the claimant's house style:
- a nudge counts only when the injected sender confirms delivery
  (claimant._send_prompt: a 2xx POST, or a reconcile proving the
  prompt landed). A failed send consumes no budget and retries
  next round - a nudge that never arrived must not burn one of
  the session's two;
- escalation is the note: when the note post fails, the episode
  is not marked escalated and the escalation retries next round.
  When the binding carries no addressable issue (no repo/issue
  on the claimed row), the escalation lands in this record and
  the state only, marked unaddressed - a repo is never guessed;
- shadow fires plan and record, and mutate nothing: no prompt,
  no note, no state write (the hand-back rule);
- an absent or errored round produces no actions and touches no
  state: actuation never runs on a round that could not see.

The sender and the note poster are injected by claimant.py (its
verified _send_prompt transport and a gh poster in the hand-back
note's pattern), so the whole rule is testable at the effect
boundary without a server or a network.
"""

import json
import os
from pathlib import Path

# #1338's nudge budget: "Nudge at most twice ... then escalate."
MAX_NUDGES = 2

STATE_FILE_NAME = "nudge-state.json"

NUDGE_TEXT = (
    "Nudge from the OpenCode claimant's babysitter (#1187): this "
    "session has been silent past the stall threshold with no live "
    "CI on its bound PR, and the stall is confirmed. If you are "
    "working, reply with a progress note now. If you are blocked, "
    "say what you are blocked on. If the work is done, report the "
    "terminal state (the PR link or the finished outcome) so the "
    "task can be harvested."
)


def _required_home() -> Path:
    """Home per the #5762 home-resolution contract (code#8221),
    mirroring babysitter.py/poll.py: a non-empty HOME, or a named
    error - never the passwd fallback."""
    home = os.environ.get("HOME")
    if not home:
        raise RuntimeError(
            "HOME is unset or empty: nudges cannot resolve its "
            "state paths (#5762 home-resolution contract)"
        )
    return Path(home)


def _default_state_dir() -> Path:
    return _required_home() / ".local" / "state" / "opencode-claimant"


_babysitter_module = None


def _load_babysitter_module():
    """Load the sibling babysitter.py as a module (cached), the
    same way claimant.py loads it: babysitter.py owns the binding
    read this module's escalation addressing reuses."""
    global _babysitter_module
    if _babysitter_module is None:
        import importlib.util

        path = Path(__file__).resolve().parent / "babysitter.py"
        spec = importlib.util.spec_from_file_location("nudges_babysitter", path)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        _babysitter_module = module
    return _babysitter_module


def confirmed_stalls(record):
    """session_id -> verdict entry for the round's confirmed
    stalls (verdict "stalled" with confirmed true). Anything else
    in the record - suspects, other verdicts, an error record -
    yields nothing: actuation runs on confirmed stalls only."""
    if not isinstance(record, dict) or record.get("error"):
        return {}
    verdicts = record.get("verdicts")
    if not isinstance(verdicts, dict):
        return {}
    return {
        sid: entry
        for sid, entry in verdicts.items()
        if isinstance(entry, dict)
        and entry.get("verdict") == "stalled"
        and entry.get("confirmed")
    }


def plan_actions(record, prior_sessions):
    """The round's planned actions, pure: one per confirmed stall.

    Bound stall, budget left -> "nudge"; bound stall, budget
    spent, not yet escalated -> "escalate"; bound stall already
    escalated -> no action (the fleet has the case). Unbound
    stall -> "observed" (recorded, never driven)."""
    actions = []
    for sid, entry in sorted(confirmed_stalls(record).items()):
        task_id = entry.get("task_id")
        if task_id is None:
            actions.append({"session_id": sid, "kind": "observed", "task_id": None})
            continue
        prior = prior_sessions.get(sid) or {}
        sent = prior.get("nudges_sent")
        sent = sent if isinstance(sent, int) else 0
        if prior.get("escalated"):
            continue
        if sent < MAX_NUDGES:
            actions.append(
                {
                    "session_id": sid,
                    "kind": "nudge",
                    "task_id": task_id,
                    "nudges_sent": sent,
                }
            )
        else:
            actions.append(
                {
                    "session_id": sid,
                    "kind": "escalate",
                    "task_id": task_id,
                    "nudges_sent": sent,
                }
            )
    return actions


def _load_state(state_dir):
    try:
        data = json.loads((state_dir / STATE_FILE_NAME).read_text())
        sessions = data.get("sessions") if isinstance(data, dict) else None
        return sessions if isinstance(sessions, dict) else {}
    except Exception:
        return {}


def nudge_step(
    mode, record, *, send_prompt=None, post_note=None, bindings=None, state_dir=None
):
    """Run one nudge pass over a babysitter round record and
    return its record for the fire log.

    Live mode applies the planned effects through the injected
    send_prompt(session_id, text) -> bool and
    post_note(task_id=, session_id=, repo=, issue_number=,
    nudges_sent=) -> bool, and persists the nudge state. Every
    other mode plans and mutates nothing. An absent or errored
    round is skipped whole: no actions, no state change.
    """
    if state_dir is None:
        state_dir = _default_state_dir()
    if (
        not isinstance(record, dict)
        or record.get("error") is not None
        or not isinstance(record.get("verdicts"), dict)
    ):
        return {"mode": mode, "skipped": "no_round", "actions": []}

    prior_sessions = _load_state(state_dir)
    planned = plan_actions(record, prior_sessions)
    round_no = record.get("round")

    if mode != "live":
        return {
            "mode": mode,
            "round": round_no,
            "actions": [dict(a, applied=False) for a in planned],
        }

    if bindings is None:
        babysitter = _load_babysitter_module()
        bindings = babysitter.build_bindings(babysitter._load_poll_module())

    stalls = confirmed_stalls(record)
    outcomes = []
    new_sessions = {}
    for action in planned:
        sid = action["session_id"]
        kind = action["kind"]
        prior = prior_sessions.get(sid) or {}
        sent = prior.get("nudges_sent")
        sent = sent if isinstance(sent, int) else 0
        entry = {
            "task_id": action["task_id"],
            "nudges_sent": sent,
            "escalated": bool(prior.get("escalated")),
            "first_round": prior.get("first_round", round_no),
            "last_round": round_no,
        }
        if kind == "observed":
            outcomes.append(dict(action, applied=False, detail="observed_only"))
            continue
        if kind == "nudge":
            delivered = bool(send_prompt and send_prompt(sid, NUDGE_TEXT))
            if delivered:
                entry["nudges_sent"] = sent + 1
            outcomes.append(
                dict(
                    action,
                    applied=delivered,
                    nudges_sent=entry["nudges_sent"],
                    detail="delivered" if delivered else "send_failed",
                )
            )
        elif kind == "escalate":
            target = bindings.get(sid) or {}
            repo = target.get("repo")
            issue_number = target.get("issue_number")
            if repo and issue_number:
                noted = bool(
                    post_note
                    and post_note(
                        task_id=action["task_id"],
                        session_id=sid,
                        repo=repo,
                        issue_number=issue_number,
                        nudges_sent=sent,
                    )
                )
                if noted:
                    entry["escalated"] = True
                outcomes.append(
                    dict(
                        action,
                        applied=noted,
                        detail="noted" if noted else "note_failed",
                    )
                )
            else:
                # No addressable issue on the row: the escalation
                # lands here and in the state, once - a repo is
                # never guessed, and the episode does not re-plan
                # an impossible note every round.
                entry["escalated"] = True
                outcomes.append(dict(action, applied=False, detail="no_issue_address"))
        new_sessions[sid] = entry

    # Episodes still open but actionless this round (already
    # escalated) carry their state; episodes whose session is no
    # longer confirmed-stalled - or no longer bound - close by
    # dropping out.
    for sid, prior in prior_sessions.items():
        if (
            sid in stalls
            and sid not in new_sessions
            and stalls[sid].get("task_id") is not None
        ):
            carried = dict(prior)
            carried["last_round"] = round_no
            new_sessions[sid] = carried

    state_dir.mkdir(parents=True, exist_ok=True)
    (state_dir / STATE_FILE_NAME).write_text(json.dumps({"sessions": new_sessions}))
    return {"mode": mode, "round": round_no, "actions": outcomes}
