#!/usr/bin/env python3
"""send_to_opencode.py — the Muse→opencode push (#8210).

One call a Muse session/lane makes to hand a task it HOLDS to an
opencode session, with no stops in between:

    python3 -I send_to_opencode.py --queue <queue> --task <id> \
        --worker <sender worker> --from-session <muse session id> \
        --reason "<stated pass-off reason>" \
        (--notes "<text>" | --notes-file <path>) \
        [--worktree <existing worktree dir>] [--dry-run]

(--issue <n> may replace --task when the sender holds exactly one
claimed task anchored to that issue.)

The sequence (each step verified before the next):

  1. gates: claimant mode is live and the session cap has room
     (claimant.py's own gates, reused);
  2. the sender's claim is verified from the broker's claimed
     rows — the sender must currently HOLD this task — and the
     issue/PR's labels are read live through gh: a hard-stop
     label (held / user/owned / do-not-merge / do-not-auto-edit)
     refuses the send, exactly as launch_one refuses a launch;
  3. the worktree: the sender-provided dir (validated as a real
     git worktree of the target repo, never the primary checkout
     itself), or one created per the opencode session convention
     (~/.local/share/opencode/tmp/opencode/queue-<task>-...,
     branch queue/<task>-muse-send). The sender's notes are
     written into it as SEND_NOTES.md;
  4. the opencode assignment is recorded WITH the sender's
     reason (`repo-queue lane <task> --opencode-reason <reason>`,
     #8187: pass-off only with a stated reason — no reason, no
     send; the sender works the item direct instead);
  5. the sender checkpoints the pass (stage handoff_offered,
     progress carrying from_session, the reason, the worktree) —
     from_session in progress is one of the holder marks the
     broker's transfer guard accepts;
  6. the opencode session is created IN the worktree through
     claimant.py's verified transport, and its location is
     verified against the worktree before anything is claimed;
  7. the claim TRANSFER (`repo-queue claim --task <id>
     --from-session <muse ses> --session-id <new ses> --worker
     opencode-claimant --platform openchamber`, #8210): holder ->
     claimer in one broker operation, never through queued,
     parked, or the sink, binding the new session atomically. The returned
     row must show the claimant as worker and the new session
     bound;
  8. checkpoint `launched` as the claimant, then the execution
     prompt: a sender-handoff header + launch_one's standard
     prompt + a SENDER NOTES section carrying the Muse agent's
     notes verbatim, sent through claimant.py's verified
     _send_prompt; then the agent note on the issue/PR.

Failure semantics: any failure before the transfer lands deletes
the created session (if any) and flips the assignment back to
direct (--exclusion-reason), so the Muse sender keeps the item
under its still-live claim — never claimed-and-unlaunched. A
session that bound but whose prompt failed stays bound
(claimant.py's standing rule; supervision owns it) and the send
reports that explicitly instead of pretending a rollback.

On success the tool prints the JSON send record, which carries
the bound session_id and the worktree, and exits 0.
"""

import argparse
import importlib.util
import json
import os
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

HERE = Path(__file__).resolve().parent


def _load_sibling(name):
    """Load poll.py / claimant.py / launch_one.py from this file's
    own directory — the sibling in THIS checkout, as launch_one
    loads its siblings."""
    spec = importlib.util.spec_from_file_location(
        f"send_{name}", HERE / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


poll = _load_sibling("poll")
claimant = _load_sibling("claimant")
launch_one = _load_sibling("launch_one")

STATE_DIR = claimant.STATE_DIR
CLAIMANT_WORKER = launch_one.CLAIMANT_WORKER
REPOS = launch_one.REPOS
NOTES_FILE_NAME = "SEND_NOTES.md"
WORKTREE_ROOT = Path.home() / ".local" / "share" / "opencode" / "tmp" / "opencode"


def log(rec):
    claimant.ensure_state_dir()
    rec = {"at": datetime.now(timezone.utc).isoformat(), **rec}
    with open(STATE_DIR / "fires.jsonl", "a") as f:
        f.write(json.dumps(rec) + "\n")
    print(json.dumps(rec, indent=1))
    return rec


def journal(rec):
    claimant.ensure_state_dir()
    with open(STATE_DIR / "inflight.jsonl", "a") as f:
        f.write(json.dumps(
            {"at": datetime.now(timezone.utc).isoformat(), **rec}) + "\n")


def queue_json(*args, timeout=120):
    """Run a repo-queue verb through launch_one's runner and parse
    its stdout as JSON. Returns (rc, parsed_or_None, stderr). The
    claim verb wraps its row as {"task": row}; callers unwrap."""
    rc, out, err = launch_one.queue(*args, timeout=timeout)
    data = None
    if out.strip():
        try:
            data = json.loads(out)
        except ValueError:
            data = None
    return rc, data, err


# --- Sender / task facts ------------------------------------------------

def sender_row(queue_name, task_id, worker):
    """The claimed row for task_id when the sender holds it, else
    (None, error). Read through poll.py's broker fetch helpers."""
    for row in poll.fetch_claimed_rows(queue_name):
        if row.get("id") == task_id:
            if row.get("worker_id") != worker:
                return None, (f"task {task_id} is claimed by "
                              f"{row.get('worker_id')!r}, not by sender {worker!r}")
            return row, None
    return None, f"task {task_id} is not claimed (by anyone) in {queue_name}"


def resolve_task_by_issue(queue_name, issue_number, worker):
    """(task_id, error): the one task the sender holds in this
    queue whose anchor is issue <issue_number>."""
    hits = []
    for row in poll.fetch_claimed_rows(queue_name):
        if row.get("worker_id") != worker:
            continue
        ref = launch_one.ref_for(row)
        if ref == ("issue", issue_number):
            hits.append(row.get("id"))
    if len(hits) == 1:
        return hits[0], None
    if not hits:
        return None, (f"sender {worker} holds no claimed task for "
                      f"issue #{issue_number} in {queue_name}")
    return None, (f"sender holds {len(hits)} tasks for issue "
                  f"#{issue_number} ({hits}); name one with --task")


# --- Worktree --------------------------------------------------------------

def _git(primary, *args, timeout=120):
    p = subprocess.run(["git", "-C", str(primary), *args],
                       capture_output=True, text=True, timeout=timeout)
    return p.returncode, p.stdout, p.stderr


def is_worktree_of(primary, path):
    """True when path is a registered git worktree of the repo at
    primary (and not the primary itself)."""
    if not Path(path).is_dir():
        return False
    rc, out, _ = _git(primary, "worktree", "list", "--porcelain")
    if rc != 0:
        return False
    want = os.path.realpath(path)
    for line in out.splitlines():
        if line.startswith("worktree "):
            if os.path.realpath(line[len("worktree "):].strip()) == want:
                return want != os.path.realpath(primary)
    return False


def create_worktree(primary, task_id, ref):
    """Create the per-task worktree per the opencode session
    convention. Returns (path, None) or (None, error)."""
    suffix = f"-{ref[1]}" if ref and ref[0] == "issue" else ""
    path = WORKTREE_ROOT / f"queue-{task_id}{suffix}-muse-send"
    branch = f"queue/{task_id}-muse-send"
    if path.exists():
        if is_worktree_of(primary, str(path)):
            return str(path), None
        return None, f"{path} exists but is not a worktree of {primary}"
    path.parent.mkdir(parents=True, exist_ok=True)
    rc, _out, err = _git(primary, "worktree", "add", "-b", branch, str(path))
    if rc != 0:
        # A leftover branch with no worktree: attach it instead.
        rc2, _o2, err2 = _git(primary, "worktree", "add", str(path), branch)
        if rc2 != 0:
            return None, (f"git worktree add failed: {err.strip()[:200]} / "
                          f"{err2.strip()[:200]}")
    return str(path), None


def write_notes_file(worktree, notes, task_id, from_session, reason):
    path = Path(worktree) / NOTES_FILE_NAME
    header = (f"# Sender notes — queue task {task_id}\n\n"
              f"Sent by Muse session `{from_session}` via "
              f"send_to_opencode.py (#8210).\n"
              f"Pass-off reason: {reason}\n\n---\n\n")
    path.write_text(header + notes.rstrip() + "\n")
    return str(path)


# --- Queue steps -----------------------------------------------------------

def assign_opencode(task_id, worker, reason):
    """Record the opencode assignment with the sender's reason.
    Returns (ledger_row, None) or (None, error)."""
    rc, data, err = queue_json("lane", str(task_id), "--worker", worker,
                               "--opencode-reason", reason)
    row = data.get("row") if isinstance(data, dict) and "row" in data else data
    if rc != 0 or not isinstance(row, dict):
        return None, f"lane assignment failed: {(err.strip() or 'no row')[:300]}"
    if row.get("assigned_lane") != "opencode":
        return None, (f"lane assignment did not record opencode: "
                      f"assigned_lane={row.get('assigned_lane')!r}")
    return row, None


def ledger_row(task_id, worker):
    """The task's lane ledger row (the lane verb with no reason
    flags is the idempotent read), or None when unreadable."""
    rc, data, _err = queue_json("lane", str(task_id), "--worker", worker)
    row = data.get("row") if isinstance(data, dict) and "row" in data else data
    return row if rc == 0 and isinstance(row, dict) else None


def flip_back_direct(task_id, worker, why):
    """Roll the assignment back to direct (the #8187 flip). Returns
    True when the ledger reads direct afterwards."""
    rc, data, _err = queue_json("lane", str(task_id), "--worker", worker,
                                "--exclusion-reason", why)
    row = data.get("row") if isinstance(data, dict) and "row" in data else data
    return rc == 0 and isinstance(row, dict) and row.get("assigned_lane") == "direct"


def sender_checkpoint(task_id, worker, stage, progress):
    rc, _data, err = queue_json(
        "checkpoint", str(task_id), "--worker", worker, "--stage", stage,
        "--progress", json.dumps(progress))
    return rc == 0, (None if rc == 0 else err.strip()[:300])


def transfer_claim(queue_name, task_id, from_session, sid, prov, mdl):
    """The #8210 transfer claim. Returns (row, None) or
    (None, error): the row must show the claimant as worker and
    the new session bound."""
    rc, data, err = queue_json(
        "claim", "--queue", queue_name, "--worker", CLAIMANT_WORKER,
        "--platform", "openchamber", "--task", str(task_id),
        "--from-session", from_session, "--session-id", sid,
        "--model-provider", prov, "--model-id", mdl)
    row = data.get("task") if isinstance(data, dict) and "task" in data else data
    if rc != 0 or not isinstance(row, dict) or row.get("id") != task_id:
        detail = (err.strip() or (json.dumps(data)[:300] if data else "no row"))
        return None, f"transfer claim failed: {detail[:300]}"
    if row.get("worker_id") != CLAIMANT_WORKER:
        return None, (f"transfer returned the row under worker "
                      f"{row.get('worker_id')!r}, not {CLAIMANT_WORKER!r}")
    bound = (row.get("progress") or {}).get("session_id")
    if bound != sid:
        return None, (f"transfer did not bind the session: "
                      f"progress.session_id={bound!r}, want {sid!r}")
    return row, None


# --- The send ----------------------------------------------------------------

def fail_record(base, phase, reason, rolled_back=None, flipped=None, sid=None):
    rec = {**base, "kind": "send_to_opencode", "ok": False,
           "phase": phase, "reason": reason}
    if sid:
        rec["session_id"] = sid
    if rolled_back is not None:
        rec["rolled_back"] = rolled_back
    if flipped is not None:
        rec["assignment_flipped_back"] = flipped
    return log(rec)


def send(queue_name, task_id, sender_worker, from_session, reason,
         notes, worktree_arg, dry_run=False):
    base = {"task": task_id, "queue": queue_name, "sender_worker": sender_worker,
            "from_session": from_session}
    # Input gates — before anything is read or mutated.
    if not reason or not reason.strip():
        return fail_record(base, "validate",
                           "a stated --reason is required (#8187: pass-off "
                           "only with a reason); without one, work the item direct")
    if not notes or not notes.strip():
        return fail_record(base, "validate",
                           "sender --notes are required: the opencode session "
                           "must receive what to do, not just the task id")
    if not from_session or not from_session.strip():
        return fail_record(base, "validate",
                           "--from-session (the sending Muse session id) is "
                           "required: the transfer guard names the holder by it")

    mode = claimant.read_mode()
    if mode != "live":
        return fail_record(base, "gate", f"mode_not_live: mode file reads {mode!r}")
    cap_room = claimant.cap_step()
    if cap_room == -1:
        return fail_record(base, "gate", "cap_unknown: session count unreadable")
    if cap_room <= 0:
        return fail_record(base, "gate", f"cap_saturated: cap_room={cap_room}")

    row, err = sender_row(queue_name, task_id, sender_worker)
    if row is None:
        return fail_record(base, "verify_claim", err)
    if launch_one.exhausted(row):
        return fail_record(base, "verify_claim",
                           f"attempts exhausted ({row.get('attempts')}/"
                           f"{row.get('max_attempts')})")
    repo = launch_one.repo_for(row, queue_name)
    primary = REPOS[queue_name][1]
    ref = launch_one.ref_for(row)
    hs = launch_one.hard_stop_reason(repo, ref)
    if hs:
        return fail_record(base, "labels", hs)
    prov, mdl = launch_one.route_for(row)

    # Worktree: sender-provided (validated) or created.
    if worktree_arg:
        if os.path.realpath(worktree_arg) == os.path.realpath(primary):
            return fail_record(base, "worktree",
                               "the worktree must not be the primary checkout itself")
        if not is_worktree_of(primary, worktree_arg):
            return fail_record(base, "worktree",
                               f"{worktree_arg} is not a git worktree of {primary}")
        worktree, created = worktree_arg, False
    else:
        worktree, werr = create_worktree(primary, task_id, ref)
        if worktree is None:
            return fail_record(base, "worktree", werr)
        created = True

    if dry_run:
        return log({**base, "kind": "send_to_opencode", "ok": True,
                    "dry_run": True, "repo": repo, "ref": ref,
                    "route": f"{prov}/{mdl}", "worktree": worktree,
                    "worktree_created": created,
                    "would": [
                        f"repo-queue lane {task_id} --opencode-reason <reason>",
                        "checkpoint handoff_offered as the sender "
                        "(from_session + reason + worktree into progress)",
                        f"create an opencode session in {worktree}",
                        f"repo-queue claim --task {task_id} --from-session "
                        f"{from_session} --session-id <new> --worker {CLAIMANT_WORKER}",
                        "checkpoint launched; send prompt with SENDER NOTES; "
                        "post the agent note",
                    ]})

    # Platform limitation (#8210): a recorded limitation with a
    # reason binds this send — a task limited to muse refuses an
    # opencode send, naming the limitation and its reason. A
    # reason-less stored limitation is invalid: the broker
    # clears it at transfer time rather than enforcing it, so it
    # does not block the send here.
    led = ledger_row(task_id, sender_worker)
    if led:
        limit = led.get("platform_limit")
        limit_reason = led.get("platform_limit_reason")
        if limit and limit_reason and str(limit_reason).strip() \
                and limit != "openchamber":
            return fail_record(
                base, "platform_limit",
                f"task is limited to platform '{limit}' "
                f"(reason: {limit_reason}); an opencode send is "
                f"refused — the sender keeps the item (#8210)")
    notes_path = write_notes_file(worktree, notes, task_id, from_session, reason)

    # Assignment, with the reason recorded (#8187).
    _ledger, aerr = assign_opencode(task_id, sender_worker, reason)
    if aerr:
        return fail_record(base, "assign", aerr, rolled_back=False)

    def rollback(phase, why, sid=None):
        flipped = flip_back_direct(
            task_id, sender_worker,
            f"muse send to opencode failed ({phase}: {why}); "
            f"sender keeps the item (#8210)")
        if sid:
            claimant._delete_session(sid)
        return fail_record(base, phase, why, rolled_back=True,
                           flipped=flipped, sid=sid)

    # The sender's pass mark: from_session + reason + worktree
    # into progress, under the sender's own claim.
    ok_cp, cp_err = sender_checkpoint(
        task_id, sender_worker, "handoff_offered",
        {"from_session": from_session, "handoff_reason": reason,
         "worktree": worktree, "notes_file": NOTES_FILE_NAME,
         "sent_by": sender_worker})
    if not ok_cp:
        return rollback("checkpoint", f"sender checkpoint failed: {cp_err}")

    # Create the session IN the worktree (claimant transport).
    title = f"queue task {task_id}" + (f" · {ref[0]} #{ref[1]}" if ref else "") \
        + f" · {str(row.get('title'))[:70]}"
    body = {"title": title, "location": {"directory": worktree},
            "model": {"id": mdl, "providerID": prov}, "agent": "build"}
    status_code, payload = claimant._server_request(
        "POST", "/api/session", body=body, timeout=15)
    if not claimant._is_2xx(status_code):
        return rollback("create",
                        f"session creation failed: http status "
                        f"{status_code if status_code is not None else 'unknown'}")
    sess = claimant._unwrap_envelope(payload)
    sid = sess.get("id") if isinstance(sess, dict) else None
    if not sid:
        return rollback("create", "no session id in create response")
    journal({"task": task_id, "queue": queue_name, "session_id": sid,
             "phase": "created", "via": "send_to_opencode"})
    got = ((sess or {}).get("location") or {}).get("directory")
    if not got or os.path.realpath(got) != os.path.realpath(worktree):
        return rollback("location",
                        f"session location mismatch: want {worktree}, got {got}",
                        sid=sid)

    # THE TRANSFER — holder -> claimer, binding in the same op.
    trow, terr = transfer_claim(queue_name, task_id, from_session, sid, prov, mdl)
    if trow is None:
        return rollback("transfer", terr, sid=sid)
    journal({"task": task_id, "queue": queue_name, "session_id": sid,
             "phase": "transferred", "via": "send_to_opencode"})

    # Checkpoint launched, as the claiming worker.
    rc2, _o2, err2 = launch_one.queue(
        "checkpoint", str(task_id), "--worker", CLAIMANT_WORKER,
        "--stage", "launched",
        "--progress", json.dumps({"lane": "opencode", "session_id": sid,
                                  "launched_by": CLAIMANT_WORKER,
                                  "worktree": worktree,
                                  "sent_by": from_session,
                                  "sender_worker": sender_worker}))

    # Prompt: sender header + the standard launch prompt (which
    # names the worktree as the session directory) + the notes.
    prompt = build_send_prompt(trow, ref, repo, worktree, sid, notes,
                               from_session, sender_worker, reason)
    prompt_ok = claimant._send_prompt(sid, prompt)

    note_ok, note_err = (None, None)
    if ref:
        note_ok, note_err = launch_one.post_agent_note(trow, ref, repo, sid)

    rec = {**base, "kind": "send_to_opencode", "session_id": sid,
           "repo": repo, "ref": ref, "route": f"{prov}/{mdl}",
           "worktree": worktree, "worktree_created": created,
           "notes_file": notes_path, "reason": reason,
           "checkpoint_rc": rc2,
           "checkpoint_err": err2.strip()[:200] if rc2 != 0 else None,
           "prompt_ok": prompt_ok, "note_ok": note_ok,
           "note_error": note_err, "ok": prompt_ok}
    log(rec)
    journal({"task": task_id, "queue": queue_name, "session_id": sid,
             "phase": "prompted", "prompt_ok": prompt_ok,
             "via": "send_to_opencode"})
    if not prompt_ok:
        rec = {**rec, "ok": False, "rolled_back": False,
               "reason": "prompt_failed: the session is bound and the "
                         "transfer stands; supervision owns the bound "
                         "session (claimant rule) — the send did NOT "
                         "roll back"}
        print(json.dumps(rec, indent=1))
    return rec


def build_send_prompt(row, ref, repo, worktree, sid, notes,
                      from_session, sender_worker, reason):
    header = f"""SENDER HANDOFF — this task was sent to you directly by a Muse session (#8210), not picked from the queue.
- Sent by: session {from_session} (worker {sender_worker})
- Pass-off reason: {reason}
- Your worktree: {worktree} — it is already created; work HERE from the first step. Do NOT work in the primary checkout or any other directory.
- The sender's notes are in the SENDER NOTES section of this prompt and in {NOTES_FILE_NAME} in the worktree.

"""
    return header + launch_one.build_prompt(
        row, ref, repo, worktree, sid, CLAIMANT_WORKER, sender_notes=notes)


# --- CLI ---------------------------------------------------------------------

def main(argv):
    ap = argparse.ArgumentParser(
        description="Send a task you hold to an opencode session (#8210).")
    ap.add_argument("--queue", required=True, choices=sorted(REPOS))
    ap.add_argument("--task", type=int)
    ap.add_argument("--issue", type=int)
    ap.add_argument("--worker", required=True,
                    help="the sender's queue worker id (its claim must hold the task)")
    ap.add_argument("--from-session", required=True,
                    help="the sending Muse session id (the holder mark)")
    ap.add_argument("--reason", required=True,
                    help="the stated pass-off reason, recorded on the assignment")
    ap.add_argument("--notes")
    ap.add_argument("--notes-file")
    ap.add_argument("--worktree",
                    help="existing worktree dir for the target repo; "
                         "created per convention when omitted")
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args(argv[1:])

    notes = args.notes
    if args.notes_file:
        try:
            notes = Path(args.notes_file).read_text()
        except OSError as e:
            print(json.dumps({"ok": False, "phase": "validate",
                              "reason": f"cannot read --notes-file: {e}"}))
            return 2
    task_id = args.task
    if task_id is None:
        if args.issue is None:
            print(json.dumps({"ok": False, "phase": "validate",
                              "reason": "--task or --issue is required"}))
            return 2
        task_id, err = resolve_task_by_issue(args.queue, args.issue, args.worker)
        if task_id is None:
            print(json.dumps({"ok": False, "phase": "validate", "reason": err}))
            return 2
    rec = send(args.queue, task_id, args.worker, args.from_session,
               args.reason, notes or "", args.worktree,
               dry_run=args.dry_run)
    return 0 if rec.get("ok") else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
