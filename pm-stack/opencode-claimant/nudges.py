#!/usr/bin/env python3
"""nudges.py - retries, stall nudges + escalation over babysitter verdicts.

The actuation half of the babysitter loop (#1234's scope: "progress
verdicts, one stall nudge then escalation"). babysitter.py (#1338)
classifies; it deliberately drives nothing. This module consumes a
round record's CONFIRMED stalls and acts on them:

- a bound session in a confirmed stall gets a nudge - a prompt
  injected through the claimant's verified delivery transport -
  at most MAX_NUDGES times per stall episode;
- a bound terminal error is recorded as a one-shot retry action; the
  claimant's supervision step performs the delete + compare-clear,
  and the next fire relaunches through the task's route;
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

Intervention accounting is binding-aware (#1656 AC4): every
actuation belongs to a class - "queue" for interventions on
queue-bound sessions (nudges and escalations; a binding exists
only because a queue claimed the session), "other" for the
unbound sessions this module only ever observes. The state file
carries cumulative per-class counts alongside the episodes:

    {"sessions": {...},
     "counts": {"queue": {"nudges": int, "escalations": int},
                "other": {"observed": int}}}

so a burst of queue-session failures is visible as its own
number, never merged into the run's other supervision. Only
applied actuations count: a failed send is not an intervention,
and an escalation that could not be noted is not one either.
Queue-class interventions are capped per round at
MAX_QUEUE_INTERVENTIONS_PER_ROUND; planned interventions past
the cap are overflow - reported in the round record and
deferred, not dropped: the episode state is carried untouched,
so the intervention re-plans next round.

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

# #1656 AC4: the per-round budget for queue-class interventions
# (nudges + escalations on queue-bound sessions). Planned queue
# interventions past this cap in one round are overflow: reported
# in the round record and deferred to the next round, never
# dropped - a burst of queue-session failures cannot silently
# consume the whole run's intervention budget.
MAX_QUEUE_INTERVENTIONS_PER_ROUND = 10

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


def error_stopped_sessions(record):
    """session_id -> binding for retryable bound error outcomes.

    Error sessions are acted on once by the queue claimant. Unbound
    error sessions remain observation-only, just like unbound stalls.
    """
    if not isinstance(record, dict) or record.get("error"):
        return {}
    verdicts = record.get("verdicts")
    if not isinstance(verdicts, dict):
        return {}
    return {
        sid: entry
        for sid, entry in verdicts.items()
        if isinstance(entry, dict)
        and entry.get("verdict") == "error-stopped"
        and entry.get("retryable")
        and entry.get("task_id") is not None
    }


def plan_actions(record, prior_sessions):
    """The round's planned actions, pure: retries and stall actions.

    A bound terminal error -> one "retry" through the claimant's
    atomic recovery path. Bound stall, budget left -> "nudge"; bound stall, budget
    spent, not yet escalated -> "escalate"; bound stall already
    escalated -> no action (the fleet has the case). Unbound
    stall -> "observed" (recorded, never driven)."""
    actions = []
    round_no = record.get("round") if isinstance(record, dict) else None
    for sid, entry in sorted(error_stopped_sessions(record).items()):
        prior = prior_sessions.get(sid) or {}
        if prior.get("error_retried"):
            continue
        # A failed retry backs off (#16066): it re-plans only once
        # its cooldown round arrives, so a non-actionable recovery
        # cannot consume intervention capacity every round.
        next_round = prior.get("error_retry_next_round")
        if (
            isinstance(next_round, int)
            and isinstance(round_no, int)
            and round_no < next_round
        ):
            continue
        failures = prior.get("error_retry_failures")
        actions.append(
            {
                "session_id": sid,
                "kind": "retry",
                "task_id": entry["task_id"],
                "outcome": entry.get("outcome"),
                "binding": entry.get("binding") or {},
                "error_retry_failures": failures if isinstance(failures, int) else 0,
            }
        )
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


def _empty_counts():
    return {
        "queue": {"retries": 0, "nudges": 0, "escalations": 0},
        "other": {"observed": 0},
    }


def _load_counts(state_dir):
    """Cumulative per-class intervention counts (#1656 AC4) from
    the state file. A state file written before the counts
    existed - or an unreadable one - reads as zeros: history the
    file does not carry is never invented."""
    counts = _empty_counts()
    try:
        data = json.loads((state_dir / STATE_FILE_NAME).read_text())
        raw = data.get("counts") if isinstance(data, dict) else None
        if isinstance(raw, dict):
            for cls, kinds in counts.items():
                saved = raw.get(cls)
                if isinstance(saved, dict):
                    for kind in kinds:
                        value = saved.get(kind)
                        if isinstance(value, int):
                            kinds[kind] = value
    except Exception:
        pass
    return counts


def split_plan(planned):
    """Split a round's planned actions into (to_apply, overflow)
    under the per-round queue-intervention cap (#1656 AC4).

    Observed actions are not interventions and are never capped.
    Queue interventions past MAX_QUEUE_INTERVENTIONS_PER_ROUND
    are overflow: the caller reports them in the round record
    and defers them - their episode state carries untouched, so
    they re-plan next round instead of being dropped. The cap
    considers queue interventions least-served first (lowest
    nudges_sent, then session id): a session deferred by one
    burst is served before the sessions that consumed budget
    ahead of it, so overflow defers but never starves."""
    queue = [a for a in planned if a["kind"] in ("retry", "nudge", "escalate")]
    # Executable stall work (nudges/escalations) is served before
    # error-stopped retries (#16066): a retry carries no nudges_sent
    # and previously sorted as least-served, so repeated recovery
    # attempts could sort ahead of real nudges and starve them.
    queue.sort(
        key=lambda a: (
            1 if a["kind"] == "retry" else 0,
            a.get("nudges_sent") or 0,
            a["session_id"],
        )
    )
    overflow_ids = {id(a) for a in queue[MAX_QUEUE_INTERVENTIONS_PER_ROUND:]}
    to_apply = [a for a in planned if id(a) not in overflow_ids]
    overflow = [a for a in planned if id(a) in overflow_ids]
    return to_apply, overflow


def nudge_step(
    mode,
    record,
    *,
    send_prompt=None,
    post_note=None,
    recover_error=None,
    bindings=None,
    state_dir=None,
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
    to_apply, overflow = split_plan(planned)
    round_no = record.get("round")

    if mode != "live":
        return {
            "mode": mode,
            "round": round_no,
            "actions": [dict(a, applied=False) for a in to_apply],
            "overflow": [
                dict(a, applied=False, detail="over_queue_cap") for a in overflow
            ],
            "interventions": {
                "queue": {
                    "retries": sum(1 for a in to_apply if a["kind"] == "retry"),
                    "nudges": sum(1 for a in to_apply if a["kind"] == "nudge"),
                    "escalations": sum(1 for a in to_apply if a["kind"] == "escalate"),
                },
                "other": {
                    "observed": sum(1 for a in to_apply if a["kind"] == "observed")
                },
                "cumulative": _load_counts(state_dir),
            },
        }

    if bindings is None:
        babysitter = _load_babysitter_module()
        bindings = babysitter.build_bindings(babysitter._load_poll_module())

    stalls = confirmed_stalls(record)
    errors = error_stopped_sessions(record)
    outcomes = []
    new_sessions = {}
    for action in to_apply:
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
        if kind == "retry":
            recovered = bool(
                recover_error
                and recover_error(sid, action["task_id"], action.get("binding") or {})
            )
            entry["error_retried"] = recovered
            if not recovered:
                # Backoff state (#16066): the failure count grows the
                # cooldown (failures + 1 rounds) before plan_actions
                # will re-plan this retry, so a persistently
                # non-actionable recovery stops consuming the shared
                # intervention cap while executable work proceeds.
                failures = action.get("error_retry_failures")
                failures = failures if isinstance(failures, int) else 0
                entry["error_retry_failures"] = failures + 1
                if isinstance(round_no, int):
                    entry["error_retry_next_round"] = round_no + failures + 2
            outcomes.append(
                dict(
                    action,
                    applied=recovered,
                    detail="requeued_retry" if recovered else "recovery_failed",
                )
            )
            if not recovered:
                new_sessions[sid] = entry
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

    # Overflow interventions are deferred, not dropped (#1656
    # AC4): their episodes carry with budget and escalation state
    # untouched, so the same intervention re-plans next round.
    for action in overflow:
        sid = action["session_id"]
        if (sid not in stalls and sid not in errors) or sid in new_sessions:
            continue
        prior = prior_sessions.get(sid) or {}
        if action["kind"] == "retry":
            new_sessions[sid] = {
                "task_id": action["task_id"],
                "error_retried": False,
                "first_round": prior.get("first_round", round_no),
                "last_round": round_no,
            }
            continue
        sent = prior.get("nudges_sent")
        new_sessions[sid] = {
            "task_id": action["task_id"],
            "nudges_sent": sent if isinstance(sent, int) else 0,
            "escalated": bool(prior.get("escalated")),
            "first_round": prior.get("first_round", round_no),
            "last_round": round_no,
        }

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
        elif sid in errors and sid not in new_sessions:
            carried = dict(prior)
            carried["last_round"] = round_no
            new_sessions[sid] = carried

    # Per-class intervention accounting (#1656 AC4): only applied
    # actuations count - a failed send and an unnoted escalation
    # are not interventions. Counts accumulate in the state file
    # across rounds and episode churn, queue class separate from
    # the observed-only unbound class.
    round_counts = _empty_counts()
    for outcome in outcomes:
        if not outcome.get("applied"):
            continue
        if outcome["kind"] == "retry":
            round_counts["queue"]["retries"] += 1
        elif outcome["kind"] == "nudge":
            round_counts["queue"]["nudges"] += 1
        elif outcome["kind"] == "escalate":
            round_counts["queue"]["escalations"] += 1
    round_counts["other"]["observed"] = sum(
        1 for outcome in outcomes if outcome["kind"] == "observed"
    )
    cumulative = _load_counts(state_dir)
    for cls, kinds in cumulative.items():
        for kind in kinds:
            kinds[kind] += round_counts[cls][kind]

    state_dir.mkdir(parents=True, exist_ok=True)
    (state_dir / STATE_FILE_NAME).write_text(
        json.dumps({"sessions": new_sessions, "counts": cumulative})
    )
    return {
        "mode": mode,
        "round": round_no,
        "actions": outcomes,
        "overflow": [dict(a, applied=False, detail="over_queue_cap") for a in overflow],
        "interventions": {
            "queue": round_counts["queue"],
            "other": round_counts["other"],
            "cumulative": cumulative,
        },
    }
