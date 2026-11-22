#!/usr/bin/env python3
"""babysitter.py - verdict rounds over all OpenCode sessions (#1338).

One round enumerates EVERY root session at the location (bound or
unbound - unbound sessions are observed, never driven; this module
drives nothing, it only classifies) and gives each a verdict from
#1338's vocabulary: progressing / stalled / done-unreported /
blocked / walled (402-class) / idle.

The stall rule is #1338's, with #1192's detector: suspect when
>= STALL_SUSPECT_MS (20 min) since the session's last activity AND
no live CI on its bound PR; confirm on a second suspect round (the
prior round flagged the same session stalled at the same activity
stamp) or on expiry-equivalent silence - silence past poll.py's
ACTIVE_SET_STALE_MS zombie bound, where a session is
indistinguishable from expired. Confirmation therefore needs the
prior round's state, persisted in babysitter-state.json between
fires; a round with no readable prior state can suspect but (past
the expiry bound aside) cannot confirm.

Verdicts ride the claimant fire (the seat's only always-running
code sweep, every 10 min - inside #1338's 20-min round cadence):
claimant.py runs one round per fire and carries the record in
fires.jsonl under the `babysitter` key; live fires also append the
record to babysitter-rounds.jsonl in the claimant state dir, the
code-side rounds log the pool-intelligence and meta-reviewer
consumers can read. Shadow fires compute the round and mutate
nothing (no state write, no log append) - the hand-back rule.

Session facts are poll.py's verified shapes, never re-derived:
roots are poll.is_root_session, timing is poll._session_timing
(activity = time.updated falling back to time.created, finished =
time.idle or a top-level outcome), and the zombie bound is
poll.ACTIVE_SET_STALE_MS. Those two poll helpers are private by
name only; claimant.py already treats poll.py as the single
session-shape contract (#6609), and a second timing implementation
here is exactly the divergence that rule exists to prevent.

Pool attribution is the session's model provider (the aggregator's
session mapper reads the same fields: pool = model.providerID,
displayed as provider/model). Pool wall state is an input, read
from pool-state.json in the claimant state dir - an operator/
tooling-written file in the mode file's pattern:

    {"pools": {"<providerID>": {"walled": bool,
                                "percent": number|null,
                                "state": str|null}}}

A missing or unreadable file reads as pool_state "unread" and no
pool is walled: unknown never invents a wall, and never hides one
either - the record says the state was unread. Wiring the quota
gatherer to write that file is the follow-up slice; this slice is
the verdict mechanism it feeds.

Fail-closed rules, matching the claimant's house style:
- a session whose timing cannot be read, that is not in the
  server's active set and carries no terminal marker, is named in
  the round's `unclassified` list - enumerated, never verdicted on
  evidence the round does not have;
- an unreadable session list produces an error record and writes
  nothing: a round that cannot enumerate fabricates no verdicts;
- unknown CI on a bound PR (ci_live None) can never prove the
  "no live CI" half of the stall rule, so the session reads idle
  with the reason recorded, not stalled;
- a timeless session (no readable activity stamp) that is not in
  the active set is indistinguishable from expired and reads
  stalled/confirmed, with the reason saying so.

Nudges are deliberately NOT here: they are #1187's next slice,
which consumes this round's confirmed stalls. This module is the
sweep that successor was waiting for - detection as data, in code,
on a running cadence - where the disabled conductor carried it as
prose.
"""

import json
import os
import time
from pathlib import Path

# #1338's stall threshold: suspect at >= 20 min since last activity.
# (poll.py's ACTIVE_WINDOW_MS is 30 min, but that is the cap
# counter's zombie window - a different rule with a different job.)
STALL_SUSPECT_MS = 20 * 60 * 1000

STATE_FILE_NAME = "babysitter-state.json"
ROUNDS_LOG_NAME = "babysitter-rounds.jsonl"
POOL_STATE_FILE_NAME = "pool-state.json"

VERDICTS = (
    "progressing",
    "stalled",
    "done-unreported",
    "blocked",
    "walled",
    "idle",
)


def _required_home() -> Path:
    """Home per the #5762 home-resolution contract (code#8221),
    mirroring poll.py/claimant.py: a non-empty HOME, or a named
    error - never the passwd fallback."""
    home = os.environ.get("HOME")
    if not home:
        raise RuntimeError(
            "HOME is unset or empty: babysitter cannot resolve its "
            "state paths (#5762 home-resolution contract)"
        )
    return Path(home)


def _default_state_dir() -> Path:
    return _required_home() / ".local" / "state" / "opencode-claimant"


_poll_module = None


def _load_poll_module():
    """Load the sibling poll.py as a module (cached), the same way
    claimant.py does: poll.py is the session-shape contract."""
    global _poll_module
    if _poll_module is None:
        import importlib.util

        poll_path = Path(__file__).resolve().parent / "poll.py"
        spec = importlib.util.spec_from_file_location("babysitter_poll", poll_path)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        _poll_module = module
    return _poll_module


def session_pool(session):
    """(provider_id, display) for a session's pool, from its model
    fields - the aggregator's session mapper reads the same shape
    (provider = model.providerID). (None, None) when the session
    carries no readable model."""
    model = session.get("model") if isinstance(session, dict) else None
    if not isinstance(model, dict):
        return None, None
    provider = model.get("providerID")
    model_id = model.get("id")
    provider = provider if isinstance(provider, str) and provider else None
    model_id = model_id if isinstance(model_id, str) and model_id else None
    if provider and model_id:
        return provider, f"{provider}/{model_id}"
    return provider or model_id, provider or model_id


def classify_session(session, *, binding, active_ids, pools, prior_entry,
                     now_ms):
    """One session's verdict entry, or None when the session cannot
    be classified on readable evidence (the caller names it in the
    round's unclassified list).

    `binding` is the queue binding for the session (dict with
    task_id / ci_live / pr_merged / blocked_reason) or None when
    unbound. `pools` maps provider id -> {"walled": bool, ...}.
    `prior_entry` is the session's prior-round state entry or None.
    """
    poll = _load_poll_module()
    session_id = session.get("id")
    provider, pool_display = session_pool(session)
    pool_walled = bool(provider and (pools.get(provider) or {}).get("walled"))
    entry = {
        "pool": pool_display,
        "pool_walled": pool_walled,
        "task_id": binding.get("task_id") if binding else None,
    }

    timing = poll._session_timing(session)
    member = active_ids is not None and session_id in active_ids
    if timing is not None:
        idle_marker, updated = timing
    else:
        t = session.get("time")
        idle_marker = t.get("idle") if isinstance(t, dict) else None
        updated = 0
    finished_markers = bool(idle_marker or session.get("outcome"))
    # Active-set membership overrides stored terminal markers (a
    # resumed session keeps its earlier run's idle marker) - unless
    # the member's clock has not moved for poll.py's zombie bound,
    # when membership itself is the stale fact (#5016).
    zombie_member = (
        member
        and timing is not None
        and updated != 0
        and now_ms - updated >= poll.ACTIVE_SET_STALE_MS
    )
    progressing_now = member and not zombie_member
    finished = finished_markers and not progressing_now

    if timing is None and not progressing_now and not finished:
        # No readable clock, no terminal marker, no live membership:
        # poll.py's UNKNOWN - never verdicted on absent evidence.
        return None

    if finished:
        if binding:
            entry["verdict"] = "done-unreported"
            entry["reason"] = "finished; bound task still open"
            pr_merged = binding.get("pr_merged")
            entry["harvest_ready"] = (
                pr_merged if isinstance(pr_merged, bool) else None
            )
        else:
            entry["verdict"] = "idle"
            entry["reason"] = "finished; nothing bound"
        return entry

    if progressing_now:
        entry["verdict"] = "progressing"
        entry["reason"] = "in the server's active set"
        return entry

    silence_ms = now_ms - updated
    if silence_ms < STALL_SUSPECT_MS:
        entry["verdict"] = "idle"
        entry["reason"] = "quiet, under the stall threshold"
        return entry

    if pool_walled:
        entry["verdict"] = "walled"
        entry["reason"] = "pool walled; silence is the wall's, not a stall"
        return entry

    if binding and binding.get("blocked_reason"):
        entry["verdict"] = "blocked"
        entry["reason"] = f"blocked: {binding['blocked_reason']}"
        return entry

    if binding and binding.get("ci_live") is True:
        entry["verdict"] = "progressing"
        entry["reason"] = "silent, but live CI on the bound PR"
        return entry

    if binding and binding.get("ci_live") is None:
        entry["verdict"] = "idle"
        entry["reason"] = (
            "stall unproven: bound PR ci unknown, and unknown ci "
            "never proves 'no live ci'"
        )
        return entry

    # Stall path: unbound (no PR, so no CI can be live) or bound
    # with ci_live proven False.
    entry["verdict"] = "stalled"
    if updated == 0:
        entry["reason"] = "no readable activity stamp; treated as expired"
    else:
        entry["reason"] = "silent past the stall threshold, no live ci"
    entry["confirmed"] = bool(
        silence_ms >= poll.ACTIVE_SET_STALE_MS
        or (
            prior_entry
            and prior_entry.get("verdict") == "stalled"
            and prior_entry.get("updated_ms") == updated
        )
    )
    return entry


def run_round(sessions, *, bindings, active_ids, pools, prior_state,
              now_ms):
    """One round over the session list. Pure: inputs in, (record,
    new_state) out - no I/O, so the rule's truth table is testable
    without a server.

    The record carries the full verdict table plus `changes`, the
    changed-only delta the report is made of (new / verdict-changed
    / gone sessions). new_state persists only verdicted sessions -
    an unclassified session is re-evaluated next round, and if it
    was verdicted before, its disappearance from the table is
    reported as `gone` rather than silently dropped.
    """
    poll = _load_poll_module()
    prior_sessions = {}
    prior_round = 0
    if isinstance(prior_state, dict):
        if isinstance(prior_state.get("sessions"), dict):
            prior_sessions = prior_state["sessions"]
        if isinstance(prior_state.get("round"), int):
            prior_round = prior_state["round"]

    verdicts = {}
    unclassified = []
    child_sessions = 0
    updated_by_id = {}
    for s in sessions or []:
        if not isinstance(s, dict) or not s.get("id"):
            continue
        if not poll.is_root_session(s):
            child_sessions += 1
            continue
        sid = s["id"]
        entry = classify_session(
            s,
            binding=(bindings or {}).get(sid),
            active_ids=active_ids,
            pools=pools or {},
            prior_entry=prior_sessions.get(sid),
            now_ms=now_ms,
        )
        if entry is None:
            unclassified.append(
                {"session_id": sid, "reason": "timing unreadable"}
            )
            continue
        verdicts[sid] = entry
        timing = poll._session_timing(s)
        updated_by_id[sid] = timing[1] if timing else 0

    changes = []
    new_sessions_state = {}
    for sid, entry in verdicts.items():
        updated_ms = updated_by_id.get(sid, 0)
        new_sessions_state[sid] = {
            "verdict": entry["verdict"],
            "confirmed": bool(entry.get("confirmed", False)),
            "updated_ms": updated_ms,
            "pool": entry["pool"],
        }
        prior = prior_sessions.get(sid)
        if prior is None:
            changes.append(
                {"session_id": sid, "change": "new",
                 "verdict": entry["verdict"]}
            )
        elif (
            prior.get("verdict") != entry["verdict"]
            or bool(prior.get("confirmed", False))
            != bool(entry.get("confirmed", False))
            or prior.get("pool") != entry["pool"]
        ):
            changes.append(
                {"session_id": sid, "change": "verdict",
                 "from": prior.get("verdict"), "verdict": entry["verdict"]}
            )
    unclassified_ids = {u["session_id"] for u in unclassified}
    for sid, prior in prior_sessions.items():
        if sid not in verdicts and sid not in unclassified_ids:
            changes.append(
                {"session_id": sid, "change": "gone",
                 "from": prior.get("verdict")}
            )

    pool_summary = {}
    for provider, p in (pools or {}).items():
        pool_summary[provider] = {
            "walled": bool((p or {}).get("walled")),
            "percent": (p or {}).get("percent"),
            "sessions": 0,
        }
    for entry in verdicts.values():
        if entry["pool"]:
            provider = entry["pool"].split("/", 1)[0]
            slot = pool_summary.setdefault(
                provider, {"walled": entry["pool_walled"],
                           "percent": None, "sessions": 0}
            )
            slot["sessions"] += 1

    record = {
        "round": prior_round + 1,
        "at_ms": now_ms,
        "sessions": len(verdicts) + len(unclassified),
        "child_sessions": child_sessions,
        "verdicts": verdicts,
        "unclassified": unclassified,
        "changes": changes,
        "pools": pool_summary,
    }
    new_state = {"round": prior_round + 1, "sessions": new_sessions_state}
    return record, new_state


def build_bindings(poll):
    """session_id -> binding from the queues' claimed rows (the
    same broker rich read supervision uses). ci_live / pr_merged /
    blocked_reason ride the row's progress when a lane recorded
    them as booleans/strings; anything else reads as unknown
    (None), never invented."""
    bindings = {}
    for queue in poll.QUEUES:
        try:
            rows = poll.fetch_claimed_rows(queue)
        except Exception:
            continue
        for row in rows or []:
            prog = row.get("progress") or {}
            sid = prog.get("session_id") or row.get("session_id")
            if not sid or sid in bindings:
                continue
            ci_live = prog.get("ci_live")
            pr_merged = prog.get("pr_merged")
            bindings[sid] = {
                "task_id": row.get("id"),
                "queue": queue,
                "issue_number": prog.get("issue_number") or prog.get("issue"),
                "ci_live": ci_live if isinstance(ci_live, bool) else None,
                "pr_merged": pr_merged if isinstance(pr_merged, bool) else None,
                "blocked_reason": prog.get("blocked_reason") or None,
            }
    return bindings


def read_pool_state(state_dir):
    """(pools, status) from pool-state.json in the state dir.
    status is "read" or "unread"; an unread file means no pool is
    known walled - the round record carries the status so a reader
    never mistakes unread for all-clear."""
    try:
        data = json.loads((state_dir / POOL_STATE_FILE_NAME).read_text())
        pools = data.get("pools")
        if not isinstance(pools, dict):
            return {}, "unread"
        out = {}
        for provider, p in pools.items():
            if isinstance(p, dict):
                out[provider] = {
                    "walled": bool(p.get("walled")),
                    "percent": p.get("percent"),
                    "state": p.get("state"),
                }
        return out, "read"
    except Exception:
        return {}, "unread"


def _load_state(state_dir):
    try:
        data = json.loads((state_dir / STATE_FILE_NAME).read_text())
        return data if isinstance(data, dict) else {}
    except Exception:
        return {}


_UNSET = object()


def babysitter_step(mode, *, now_ms=None, sessions=_UNSET,
                    active_ids=_UNSET, bindings=None, pools=None,
                    state_dir=None):
    """Run one round for a claimant fire and return its record.

    Live mode persists the round state and appends the record to
    babysitter-rounds.jsonl; every other mode computes the round
    and mutates nothing. An unreadable session list returns an
    error record and writes nothing in any mode.
    """
    poll = _load_poll_module()
    if state_dir is None:
        state_dir = _default_state_dir()
    if now_ms is None:
        now_ms = int(time.time() * 1000)
    if sessions is _UNSET:
        sessions = poll.fetch_sessions()
    if sessions is None:
        return {"error": "sessions_unreadable", "verdicts": {},
                "changes": [], "mode": mode}
    if active_ids is _UNSET:
        active_ids = poll.fetch_active_ids()
    if bindings is None:
        bindings = build_bindings(poll)
    if pools is None:
        pools, pool_state = read_pool_state(state_dir)
    else:
        pool_state = "injected"
    prior_state = _load_state(state_dir)
    record, new_state = run_round(
        sessions,
        bindings=bindings,
        active_ids=active_ids,
        pools=pools,
        prior_state=prior_state,
        now_ms=now_ms,
    )
    record["pool_state"] = pool_state
    record["mode"] = mode
    if mode == "live":
        state_dir.mkdir(parents=True, exist_ok=True)
        with (state_dir / ROUNDS_LOG_NAME).open("a") as f:
            f.write(json.dumps(record) + "\n")
        (state_dir / STATE_FILE_NAME).write_text(json.dumps(new_state))
    return record
