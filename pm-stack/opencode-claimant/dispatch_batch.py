#!/usr/bin/env python3
"""dispatch_batch.py — batch claim + per-item dedicated sub-task dispatch (#8217).

One call a Muse lane makes to run part of a bundle in DISPATCH
mode: batch-claim up to the broker cap, then hand EVERY claimed item
to its own dedicated sub-task session inside the same touch:

    python3 -I dispatch_batch.py --queue <queue> --count <n<=5> \
        --to opencode|Muse --worker <lane worker> --lane <lane id> \
        --reason "<batch dispatch reason>" \
        --notes "<lane notes for the items>" \
        [--session-id <dispatching session id>] \
        [--bind <task_id>:<session_id> ...] [--release-failures] \
        [--lease-seconds <s>] [--dry-run]

Serial single-session work stays the queue's default (#8187);
dispatch mode is an allowed STATED mode. The batch reason is the
lane's stated reason for dispatching rather than working the items
itself, and it is recorded on every item (the assignment event for
opencode items, a dispatch checkpoint for Muse items).

Flow:
  1. ONE batch claim (`repo-queue claim --count N --platform
     <target family>`). The claim declares the family that will
     WORK the items, so a task carrying a recorded platform
     limitation (lane ledger, #8210: limit + reason) is never
     served into the wrong family's batch — the broker's pick
     enforces the limitation; a family-less claim is served only
     unlimited rows.
  2. Per item, in claim order:
     - its own worktree on the laptop, per runbook SS3
       (~/git/fleet/queue-<task>-<issue>-dispatch, branch
       queue/<task>-<issue>-dispatch) — created here, never the
       primary checkout, never shared between items;
     - opencode: send_to_opencode.py's send() — records the
       opencode assignment WITH the batch reason, checkpoints the
       pass, creates the session IN the worktree, transfers the
       claim from the dispatching session with no queued/parked
       stop, and sends the prompt carrying the lane's notes;
     - Muse: the assignment stays direct (lane verb), the batch
       reason + notes + worktree are checkpointed on the item, and
       the item is bound to the dedicated Muse sub-session the
       lane spawned for it (`--bind <task>:<session>` — the lane
       spawns the sub-session with its own machinery, then this
       tool binds + verifies it; an item with no --bind is
       reported `prepared_unbound` and fails the batch: the lane
       must not let go of an unbound item).
  3. Every bind is VERIFIED from live queue state (the task's
     progress.session_id equals the launched/bound session id)
     before the batch ends.
  4. An item that fails to launch/bind is never left claimed-idle:
     the send's own rollback leaves the assignment direct under
     the dispatching lane's still-live claim (outcome
     `held_by_lane_direct` — its owner is present and works it
     serially), or with --release-failures it is released straight
     back to queued via `repo-queue release` with the failure as
     the release reason.

Output: one JSON record per item plus a summary record, printed to
stdout; exit 0 only when every item is bound (or, in --dry-run,
would be). The claim itself is the broker's: `claim --count` runs
the ordinary atomic pick N times (same eligibility, same
priority/age order, same per-item events and marks).
"""

import argparse
import importlib.util
import json
import os
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent


def _load_sibling(name):
    """Load poll.py / claimant.py / launch_one.py /
    send_to_opencode.py from this file's own directory — the
    sibling in THIS checkout, as launch_one loads its siblings."""
    spec = importlib.util.spec_from_file_location(
        f"dispatch_{name}", HERE / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


poll = _load_sibling("poll")
claimant = _load_sibling("claimant")
launch_one = _load_sibling("launch_one")
send_mod = _load_sibling("send_to_opencode")

REPOS = launch_one.REPOS
BATCH_MAX = 5  # mirrors the broker's BATCH_CLAIM_MAX (#8217)
FLEET_WORKTREE_ROOT = Path.home() / "git" / "fleet"
TARGET_FAMILY = {"opencode": "openchamber", "Muse": "muse"}


# --- seams (patched by tests) ------------------------------------------------

def queue(*args, timeout=120):
    """Run a repo-queue verb through launch_one's runner."""
    return launch_one.queue(*args, timeout=timeout)


def queue_json(*args, timeout=120):
    """repo-queue verb -> (rc, parsed_json_or_None, stderr)."""
    rc, out, err = queue(*args, timeout=timeout)
    data = None
    if out and out.strip():
        try:
            data = json.loads(out)
        except ValueError:
            data = None
    return rc, data, err


def run_send(queue_name, task_id, worker, from_session, reason,
             notes, worktree):
    """One opencode send via send_to_opencode.send() — the #8210
    launcher, reused whole (no third launcher). Returns its record."""
    return send_mod.send(queue_name, task_id, worker, from_session,
                         reason, notes, worktree)


def fetch_task_row(queue_name, task_id):
    """The task's current row from the broker's rich list (claimed
    rows first, then queued — a released item reads back queued)."""
    for status in ("claimed", "queued", "parked"):
        try:
            data = poll._broker_post(
                "/list", {"queue": queue_name, "status": status,
                          "rich": True, "limit": 500})
        except Exception:
            continue
        rows = data.get("rows") if isinstance(data, dict) else data
        for row in rows or []:
            if isinstance(row, dict) and row.get("id") == task_id:
                return poll.normalize_row(row)
    return None


def git(primary, *args, timeout=120):
    p = subprocess.run(["git", "-C", str(primary), *args],
                       capture_output=True, text=True, timeout=timeout)
    return p.returncode, p.stdout.strip(), p.stderr.strip()


# --- worktrees ---------------------------------------------------------------

def worktree_for(primary, task_id, issue):
    """The item's dedicated worktree (runbook SS3 naming):
    ~/git/fleet/queue-<task>-<issue>-dispatch on branch
    queue/<task>-<issue>-dispatch. Created when missing; an
    existing dir must already be a worktree of the primary."""
    name = f"queue-{task_id}-{issue}-dispatch"
    path = FLEET_WORKTREE_ROOT / name
    branch = f"queue/{task_id}-{issue}-dispatch"
    if path.exists():
        rc, out, _ = git(primary, "worktree", "list", "--porcelain")
        known = {ln.split()[1] for ln in out.splitlines()
                 if ln.startswith("worktree ")} if rc == 0 else set()
        if str(path) in known:
            return str(path), False, None
        return None, False, f"{path} exists but is not a worktree of {primary}"
    FLEET_WORKTREE_ROOT.mkdir(parents=True, exist_ok=True)
    rc, _out, err = git(primary, "worktree", "add", "-b", branch,
                        str(path), "main")
    if rc != 0:
        # Branch may already exist from a prior dispatch of this
        # task: attach it instead of failing.
        rc2, _o2, err2 = git(primary, "worktree", "add", str(path), branch)
        if rc2 != 0:
            return None, False, f"git worktree add failed: {err or err2}"
    return str(path), True, None


# --- batch claim -------------------------------------------------------------

def claim_batch(queue_name, worker, lane, count, family,
                session_id, lease_seconds):
    """The broker batch claim. Returns (rows, error)."""
    args = ["claim", "--queue", queue_name, "--worker", worker,
            "--count", str(count), "--platform", family,
            "--lease-seconds", str(lease_seconds)]
    if lane:
        args += ["--lane", lane]
    if session_id:
        args += ["--session-id", session_id]
    rc, data, err = queue_json(*args)
    if data is None:
        return [], f"batch claim unparseable (rc={rc}): {err.strip()[:200]}"
    tasks = data.get("tasks")
    if isinstance(tasks, list):
        return tasks, None
    # count == 1 IS the single claim (#8217: count absent or 1 is
    # the ordinary claim), and the client answers a single claim
    # with the bare row — or {"task": null} when nothing is
    # claimable. Normalize both to the batch shape.
    if isinstance(data.get("id"), int):
        return [data], None
    if data.get("task") is None and "task" in data:
        return [], None
    return [], f"batch claim returned no tasks array: {str(data)[:200]}"


def release_task(task_id, worker, reason):
    rc, _data, err = queue_json(
        "release", str(task_id), "--worker", worker,
        "--reason", reason)
    return rc == 0, err.strip()[:200]


def lane_direct(task_id, worker):
    rc, _data, err = queue_json("lane", str(task_id), "--worker", worker)
    return rc == 0, err.strip()[:200]


def checkpoint(task_id, worker, stage, progress):
    rc, _data, err = queue_json(
        "checkpoint", str(task_id), "--worker", worker,
        "--stage", stage, "--progress", json.dumps(progress))
    return rc == 0, err.strip()[:200]


def bind_session(task_id, worker, session_id):
    rc, _data, err = queue_json(
        "bind", str(task_id), "--worker", worker,
        "--session-id", session_id, "--launched-by", worker)
    return rc == 0, err.strip()[:200]


def bound_session_id(queue_name, task_id):
    row = fetch_task_row(queue_name, task_id)
    if not row:
        return None, None
    progress = row.get("progress") or {}
    return progress.get("session_id"), row


# --- per-item dispatch -------------------------------------------------------

def dispatch_opencode_item(queue_name, row, worker, from_session,
                           reason, notes, release_failures):
    task_id = row["id"]
    ref = task_ref(row)
    primary = REPOS[queue_name][1]
    rec = {"task": task_id, "target": "opencode", "issue": ref[1] if ref else None}
    worktree, created, werr = worktree_for(primary, task_id, rec["issue"] or 0)
    if werr:
        return fail_item(rec, "worktree", werr, row, worker, release_failures)
    rec["worktree"] = worktree
    rec["worktree_created"] = created
    srec = run_send(queue_name, task_id, worker, from_session,
                    reason, notes, worktree)
    if not srec or not srec.get("ok"):
        why = (srec or {}).get("reason") or "send failed"
        phase = (srec or {}).get("phase") or "send"
        return fail_item(rec, phase, str(why), row, worker,
                         release_failures)
    sid = srec.get("session_id")
    rec["session_id"] = sid
    bound, _row2 = bound_session_id(queue_name, task_id)
    if bound != sid:
        return fail_item(
            rec, "verify",
            f"bind not visible in live queue state (want {sid}, read {bound})",
            row, worker, release_failures)
    rec["ok"] = True
    rec["outcome"] = "bound"
    return rec


def dispatch_claude_item(queue_name, row, worker, reason, notes,
                         binds, release_failures):
    task_id = row["id"]
    ref = task_ref(row)
    primary = REPOS[queue_name][1]
    rec = {"task": task_id, "target": "Muse", "issue": ref[1] if ref else None}
    worktree, created, werr = worktree_for(primary, task_id, rec["issue"] or 0)
    if werr:
        return fail_item(rec, "worktree", werr, row, worker, release_failures)
    rec["worktree"] = worktree
    rec["worktree_created"] = created
    ok_lane, lerr = lane_direct(task_id, worker)
    if not ok_lane:
        return fail_item(rec, "lane", lerr, row, worker, release_failures)
    ok_cp, cperr = checkpoint(
        task_id, worker, "dispatched",
        {"dispatch_mode": "batch", "dispatch_reason": reason,
         "worktree": worktree, "notes": notes,
         "target": "Muse"})
    if not ok_cp:
        return fail_item(rec, "checkpoint", cperr, row, worker,
                         release_failures)
    sid = binds.get(task_id)
    if not sid:
        rec["ok"] = False
        rec["outcome"] = "prepared_unbound"
        rec["reason"] = ("no --bind <task>:<session> for this item: "
                         "the lane must spawn its dedicated sub-session "
                         "and bind it before letting go")
        rec["bind_command"] = (
            f"fleet queue bind {task_id} --worker {worker} "
            f"--session-id <sub-session id>")
        return rec
    ok_bind, berr = bind_session(task_id, worker, sid)
    if not ok_bind:
        return fail_item(rec, "bind", berr, row, worker, release_failures)
    bound, _row2 = bound_session_id(queue_name, task_id)
    if bound != sid:
        return fail_item(
            rec, "verify",
            f"bind not visible in live queue state (want {sid}, read {bound})",
            row, worker, release_failures)
    rec["session_id"] = sid
    rec["ok"] = True
    rec["outcome"] = "bound"
    return rec


def fail_item(rec, phase, why, row, worker, release_failures):
    """A failed item is never left claimed-idle: release it back to
    queued when asked, else it stays under the dispatching lane's
    live claim with a direct assignment (held_by_lane_direct)."""
    rec["ok"] = False
    rec["phase"] = phase
    rec["reason"] = why
    if release_failures:
        ok_rel, rerr = release_task(
            rec["task"], worker,
            f"dispatch failed ({phase}: {why}); released by "
            f"dispatch_batch (#8217)")
        rec["outcome"] = "released_to_queue" if ok_rel else "release_failed"
        if not ok_rel:
            rec["release_error"] = rerr
    else:
        rec["outcome"] = "held_by_lane_direct"
    return rec


def task_ref(row):
    """(repo, issue_number) from a claimed row's payload, or None."""
    payload = row.get("payload") or {}
    repo = payload.get("repo")
    issue = (payload.get("issue") or {}).get("number")
    if repo and issue:
        return repo, issue
    return None


# --- main --------------------------------------------------------------------

def dispatch(queue_name, count, target, worker, lane, reason, notes,
             session_id, binds, release_failures, lease_seconds,
             dry_run=False):
    family = TARGET_FAMILY[target]
    rows, err = claim_batch(queue_name, worker, lane, count, family,
                            session_id, lease_seconds)
    if err:
        return [{"kind": "dispatch_batch", "ok": False,
                 "phase": "claim", "reason": err}], False
    if not rows:
        return [{"kind": "dispatch_batch", "ok": False,
                 "phase": "claim",
                 "reason": "no claimable task for this batch"}], False
    summary = {"kind": "dispatch_batch", "queue": queue_name,
               "target": target, "family": family,
               "requested": count, "claimed": len(rows),
               "reason": reason}
    if dry_run:
        summary["ok"] = True
        summary["dry_run"] = True
        summary["tasks"] = [r.get("id") for r in rows]
        return [summary], True
    records = [summary]
    all_ok = True
    for row in rows:
        if target == "opencode":
            rec = dispatch_opencode_item(
                queue_name, row, worker, session_id, reason, notes,
                release_failures)
        else:
            rec = dispatch_claude_item(
                queue_name, row, worker, reason, notes, binds,
                release_failures)
        records.append(rec)
        all_ok = all_ok and bool(rec.get("ok"))
    summary["ok"] = all_ok
    summary["bound"] = sum(1 for r in records[1:]
                            if r.get("outcome") == "bound")
    return records, all_ok


def main(argv):
    ap = argparse.ArgumentParser(
        description="Batch claim + per-item dedicated sub-task "
                    "dispatch (#8217).")
    ap.add_argument("--queue", required=True, choices=sorted(REPOS))
    ap.add_argument("--count", type=int, required=True)
    ap.add_argument("--to", required=True,
                    choices=sorted(TARGET_FAMILY), dest="target")
    ap.add_argument("--worker", required=True,
                    help="the dispatching lane's queue worker id")
    ap.add_argument("--lane", help="the dispatching lane id")
    ap.add_argument("--reason", required=True,
                    help="the stated batch dispatch reason, recorded "
                         "on every item")
    ap.add_argument("--notes", default="",
                    help="the lane's notes, carried into every item")
    ap.add_argument("--session-id",
                    help="the dispatching session's own id (the "
                         "holder mark the opencode transfer names)")
    ap.add_argument("--bind", action="append", default=[],
                    metavar="TASK:SESSION",
                    help="for --to Muse: the dedicated sub-session "
                         "id the lane spawned for that task")
    ap.add_argument("--release-failures", action="store_true",
                    help="release failed items back to queued "
                         "instead of leaving them with the lane")
    ap.add_argument("--lease-seconds", type=int, default=2700)
    ap.add_argument("--dry-run", action="store_true",
                    help="claim the batch and report it, dispatch "
                         "nothing (items stay the lane's own claim)")
    args = ap.parse_args(argv[1:])

    if not (1 <= args.count <= BATCH_MAX):
        print(json.dumps({"ok": False, "phase": "validate",
                          "reason": f"--count must be 1..{BATCH_MAX} "
                                    f"(the batch cap, #8217)"}))
        return 2
    if args.target == "opencode" and not args.session_id:
        print(json.dumps({"ok": False, "phase": "validate",
                          "reason": "--session-id is required for "
                                    "--to opencode (the transfer "
                                    "names the holding session)"}))
        return 2
    binds = {}
    for b in args.bind:
        try:
            t, s = b.split(":", 1)
            binds[int(t)] = s
        except ValueError:
            print(json.dumps({"ok": False, "phase": "validate",
                              "reason": f"bad --bind {b!r} "
                                        f"(want TASK:SESSION)"}))
            return 2
    records, ok = dispatch(
        args.queue, args.count, args.target, args.worker, args.lane,
        args.reason, args.notes, args.session_id, binds,
        args.release_failures, args.lease_seconds,
        dry_run=args.dry_run)
    for rec in records:
        print(json.dumps(rec))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
