#!/usr/bin/env python3
"""launch_one.py — per-task launch trigger for the opencode claimant (#8187).

The standing claimant (claimant.py) is a single-shot script fired by an
OpenChamber cron every few minutes: it discovers every candidate and
launches under the fleet cap. It has no per-task trigger, so a handoff
that should launch *now* ("handoff = immediate launch", #8187) waits
for the next fire — and a parked handoff sink can loop unlaunched.
This tool performs exactly the claimant's launch sequence (prompt.md
step 4) for ONE named task:

    python3 -I launch_one.py --preflight <queue>:<task_id> [...]
    python3 -I launch_one.py --launch <queue>:<task_id>

--preflight is read-only and always allowed: it reports whether the
task is a launch candidate (claimed, or a parked recorded handoff —
progress.lane == "opencode", no session_id, stage handed_off /
launch_failed), what blocks it if anything (attempts exhausted, held
labels, cap saturated, mode not live), and the exact steps a launch
would take. It mutates nothing: no claim, no session, no journal.

--launch performs the sequence for that one task, and only when the
mode file (~/.local/state/opencode-claimant/mode, claimant.py's gate)
reads "live":

  1. re-verify the candidate through poll.py's classifiers
     (is_candidate for claimed rows, is_handoff_parked for parked
     rows — the query logic lives in poll.py, not here);
  2. fail closed on exhausted attempts and on held labels
     (held / user/owned / do-not-merge / do-not-auto-edit, read live
     through `gh`; an unreadable label set is a refusal, not a pass);
  3. check the concurrency cap through claimant.py's cap_step
     (unknown cap room launches nothing);
  4. a PARKED handoff has no live claim to bind against, so collect
     its claim first with the broker's by-id special-case verb
     (`repo-queue claim --task <id> --worker opencode-claimant
     --platform openchamber`; prompt.md reserves the by-id claim for
     one-off revivals, which is exactly what a named per-task launch
     is — the claimant's default no-id `--handoff` collection cannot
     promise the broker picks THIS task). The session then works
     under --worker opencode-claimant, as in claimant.py's parked
     path;
  5. create the worker session on the OpenCode server through
     claimant.py's verified transport, journal its id to
     inflight.jsonl immediately, and verify its location is the
     queue repo's laptop primary BEFORE binding — a mismatch deletes
     the session, checkpoints launch_failed on the task, and stops;
  6. bind (`repo-queue bind --worker opencode-claimant
     --launched-by opencode-claimant`); a bind failure deletes the
     created session so no orphan is left, and stops;
  7. checkpoint stage "launched" as the claiming worker (the verb
     enforces claim ownership), then send the execution prompt
     through claimant.py's verified _send_prompt (a failed POST is
     reconciled against the session's messages before one resend);
  8. post the agent-note on the issue/PR (non-fatal, as in the
     helper this file replaces).

On success --launch prints the JSON launch record, which carries the
bound session_id, and exits 0. On any failure it prints the record
with the reason and exits non-zero; a session created but not bound
is deleted, while a bound session whose prompt failed is left bound
(supervision owns it — unbinding could double-launch over a worker
that is in fact executing, claimant.py's rule).

The repo-queue binary resolves to ~/.local/bin/repo-queue (the
packaged fleet binary's argv0 alias; the hand-built
~/fleet-brokers/bin tree was retired by the code#5362 cutover), with
an override in the REPO_QUEUE_BIN (or REPO_QUEUE_CMD) env var.

This file supersedes the state-dir helper of the same name
(~/.local/state/opencode-claimant/launch_one.py), which broke when
poll.fetch_claimed was replaced by fetch_claimed_rows /
fetch_parked_rows and its REPO_QUEUE path went stale.
"""

import importlib.util
import json
import os
import re
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

HERE = Path(__file__).resolve().parent


def _load_sibling(name):
    """Load poll.py / claimant.py from this file's own directory, the
    same by-path loading claimant.py uses for poll.py — the sibling
    in THIS checkout, never the primary clone's copy."""
    spec = importlib.util.spec_from_file_location(
        f"launch_one_{name}", HERE / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


poll = _load_sibling("poll")
claimant = _load_sibling("claimant")

STATE_DIR = claimant.STATE_DIR
CLAIMANT_WORKER = "opencode-claimant"

REPOS = {
    "repo-health-code-pilot": ("developing-today/code", "/home/user/git/code"),
    "repo-health-opencode-pilot": ("developing-today-forks/opencode", "/home/user/git/opencode"),
    "repo-health-openchamber-pilot": ("developing-today-forks/openchamber", "/home/user/git/openchamber-fork"),
    "repo-health-nix-pilot": ("dezren39/nix", "/home/user/git/nix"),
}
HARD_STOP = {"held", "user/owned", "do-not-merge", "do-not-auto-edit"}
YIELD_STAGES = ("handed_off", "launch_failed")


def repo_queue_cmd():
    """The repo-queue binary: env override, else the packaged fleet
    binary via its ~/.local/bin/repo-queue argv0 alias."""
    override = os.environ.get("REPO_QUEUE_BIN") or os.environ.get("REPO_QUEUE_CMD")
    if override:
        return override
    return str(Path.home() / ".local" / "bin" / "repo-queue")


def log(rec):
    claimant.ensure_state_dir()
    rec = {"at": datetime.now(timezone.utc).isoformat(), **rec}
    with open(STATE_DIR / "fires.jsonl", "a") as f:
        f.write(json.dumps(rec) + "\n")
    print(json.dumps(rec))


def journal(rec):
    claimant.ensure_state_dir()
    with open(STATE_DIR / "inflight.jsonl", "a") as f:
        f.write(json.dumps(
            {"at": datetime.now(timezone.utc).isoformat(), **rec}) + "\n")


def queue(*args, timeout=120):
    p = subprocess.run([repo_queue_cmd(), *args],
                       capture_output=True, text=True, timeout=timeout)
    return p.returncode, p.stdout, p.stderr


# --- Candidate discovery (delegated to poll.py) ---------------------

def explain_not_candidate(row):
    """Why poll.is_candidate / is_handoff_parked said no, in words a
    coordinator can act on. Mirrors the classifiers' checks; the
    classifiers themselves stay the verdict."""
    prog = row.get("progress") or {}
    reasons = []
    if prog.get("lane") != "opencode":
        reasons.append(f"progress.lane is {prog.get('lane')!r}, not 'opencode'")
    if prog.get("session_id"):
        reasons.append(f"already bound to session {prog.get('session_id')}")
    if prog.get("stage") not in YIELD_STAGES:
        reasons.append(
            f"stage is {prog.get('stage')!r}, not a recorded yield "
            f"({'/'.join(YIELD_STAGES)})")
    lane_id = row.get("lane_id")
    if lane_id and poll.UUID_LANE.match(str(lane_id)):
        reasons.append(f"claiming lane {lane_id} is an OpenChamber scheduled lane")
    if row.get("status") not in (None, "claimed", "parked"):
        reasons.append(f"status is {row.get('status')!r}")
    return reasons or ["not a candidate"]


def find_row(queue_name, task_id):
    """(row, status, candidate, reasons) for one task, read through
    poll.py's broker fetch helpers: claimed rows first, then the
    parked handoff sink. (None, None, False, [...]) when the task is
    in neither set for this queue."""
    for row in poll.fetch_claimed_rows(queue_name):
        if row.get("id") == task_id:
            if poll.is_candidate(row):
                return row, "claimed", True, []
            return row, "claimed", False, explain_not_candidate(row)
    for row in poll.fetch_parked_rows(queue_name):
        if row.get("id") == task_id:
            if poll.is_handoff_parked(row):
                return row, "parked", True, []
            return row, "parked", False, explain_not_candidate(row)
    return None, None, False, [f"task {task_id} not found in queue {queue_name}"]


# --- Task facts ------------------------------------------------------

def packet_for(row):
    """The task packet: poll.normalize_row lifts the payload column
    to `packet`; the raw broker row carries it as `payload`."""
    packet = row.get("packet")
    if isinstance(packet, dict):
        return packet
    payload = row.get("payload")
    if isinstance(payload, str):
        try:
            payload = json.loads(payload)
        except ValueError:
            payload = None
    return payload if isinstance(payload, dict) else {}


def repo_for(row, queue_name):
    repo = packet_for(row).get("repo")
    return repo if isinstance(repo, str) and repo else REPOS[queue_name][0]


def ref_for(row):
    """(kind, number) of the issue/PR anchor, or None: progress
    fields first, then the packet's issue object, then the dedupe
    key's `issue:<n>` / `pr:<n>` suffix."""
    prog = row.get("progress") or {}
    for field in ("issue_number", "issue"):
        v = prog.get(field)
        if isinstance(v, int):
            return ("issue", v)
        if isinstance(v, str) and v.lstrip("#").isdigit():
            return ("issue", int(v.lstrip("#")))
    issue = packet_for(row).get("issue")
    if isinstance(issue, dict) and isinstance(issue.get("number"), int):
        return ("issue", issue["number"])
    m = re.search(r"(issue|pr):(\d+)$", row.get("dedupe_key") or "")
    if m:
        return (m.group(1), int(m.group(2)))
    return None


def hard_stop_reason(repo, ref):
    """None when the anchor carries no hard-stop label; a reason
    string otherwise — including when the label set cannot be read
    (fail closed, claimant.py #7670)."""
    if not ref:
        return None
    kind, num = ref
    cmd = (["pr", "view", str(num), "--repo", repo, "--json", "labels,state,title"]
           if kind == "pr" else
           ["issue", "view", str(num), "--repo", repo, "--json", "labels,state,title"])
    try:
        p = subprocess.run(["gh", *cmd], capture_output=True, text=True, timeout=60)
        if p.returncode != 0:
            return f"label-check-failed:{p.stderr.strip()[:160]}"
        d = json.loads(p.stdout)
    except Exception as e:
        return f"label-check-failed:{e}"
    labels = {l.get("name") for l in d.get("labels") or []}
    hit = sorted(labels & HARD_STOP)
    if hit:
        return "hard-stop:" + ",".join(hit)
    return None


def route_for(row):
    """(provider, model): the recorded route (progress, then packet)
    when it names one, else the Workhorse default — prompt.md's rule,
    never improvised."""
    for source in (row.get("progress") or {}, packet_for(row)):
        r = source.get("route")
        if isinstance(r, str) and "/" in r:
            prov, mdl = r.split("/", 1)
            if prov and mdl:
                return prov, mdl
        if isinstance(r, dict):
            prov = r.get("providerID") or r.get("provider")
            mdl = r.get("id") or r.get("model")
            if prov and mdl:
                return prov, mdl
    return "opencode-route", "workhorse"


def exhausted(row):
    return (row.get("attempts") or 0) >= (row.get("max_attempts") or 0)


# --- Prompt / agent note (the helper's shapes, claimant-safe) --------

def build_prompt(row, ref, repo, directory, sid, worker_id):
    prog = claimant._prompt_safe_packet(row.get("progress") or {})
    prov, mdl = route_for(row)
    reftext = f"{ref[0]} #{ref[1]}" if ref else "none recorded"
    rq = repo_queue_cmd()
    return f"""You are an OpenCode queue worker launched by the opencode claimant for an already-claimed queue task. Work ONLY this task. Do NOT claim another task and do NOT run `repo-queue claim`.

TASK
- Queue: {row.get('queue') or ''}
- Task id: {row.get('id')}
- Title: {row.get('title')}
- Dedupe key: {row.get('dedupe_key')}
- Issue/PR anchor: {reftext}
- Priority: {row.get('priority')}
- Attempts: {row.get('attempts')}/{row.get('max_attempts')}
- Claiming worker (pass --worker {worker_id} on EVERY repo-queue verb): {worker_id}
- Bound session id (use this exact id in every --execution record): {sid}
- Target repo: {repo}
- Work in the laptop primary: {directory}
- Route: {prov}/{mdl}

QUEUE CLI
- `repo-queue` is {rq} (also on PATH).
- Heartbeat while you work so the lease stays alive:
  `repo-queue heartbeat {row.get('id')} --worker {worker_id} --progress '{{"note":"..."}}'`
- Record progress:
  `repo-queue checkpoint {row.get('id')} --worker {worker_id} --stage <stage> --progress '<json object>'`
- Finish with exactly ONE terminal verb, as {worker_id}, carrying the execution record:
  - `repo-queue complete {row.get('id')} --worker {worker_id} --result '<json object>' --execution '{{"lane":"opencode","session_id":"{sid}"}}'`
  - `repo-queue park {row.get('id')} --worker {worker_id} --reason '<text>' --delay-seconds <n> --execution '{{"lane":"opencode","session_id":"{sid}"}}'`
  - `repo-queue fail {row.get('id')} --worker {worker_id} --error '<text>' --reason-class transient|deterministic|blocked --execution '{{"lane":"opencode","session_id":"{sid}"}}'`
  - `repo-queue block {row.get('id')} --worker {worker_id} --reason-class <class> --blocker '<text>'` (block takes no --execution flag)
  Do not leave the task claimed without a terminal record.

RULES
- Revalidate fresh state (GitHub issue/PR, current main, queue task) before acting; the recorded progress below may be stale.
- Respect hard stops: if the issue/PR carries `held`, `user/owned`, `do-not-merge`, or `do-not-auto-edit`, do not edit it; park or block with the reason.
- Follow {repo}'s AGENTS.md. For code changes use a dedicated worktree; never modify the primary clone's working tree. Preserve unrelated dirty/untracked files.
- Never put a credential value in a prompt, log, PR, issue, or task record.
- The task is already claimed and lane-bound to this session. It must not be claimed again.

RECORDED TASK PROGRESS / PACKET CONTEXT (JSON)
{json.dumps(prog, indent=2, sort_keys=True)}
"""


def note_body(row, ref, sid, url):
    return f"""🤖 **Agent note** · handed off

- **Agent:** opencode claimant (opencode-claimant)
- **Session / ID:** queue task {row.get('id')}; bound session `{sid}`
- **Found via:** queue task handoff on the opencode execution lane
- **Task:** {row.get('title')}
- **Expectation:** the bound worker session works task {row.get('id')} to a terminal queue state and must not claim another task. It revalidates the issue/PR before acting.
- **Links:** {url or 'none recorded'}

<!-- agent-note:v1 agent=opencode-claimant state=handed-off task={row.get('id')} session={sid} at={datetime.now(timezone.utc).isoformat()} -->"""


def post_agent_note(row, ref, repo, sid):
    """(ok, error): the launch agent-note on the issue/PR.
    Non-fatal to the launch, as in the helper."""
    url = f"https://github.com/{repo}/{'pull' if ref[0] == 'pr' else 'issues'}/{ref[1]}"
    path = STATE_DIR / f"note-{row.get('id')}.md"
    claimant.ensure_state_dir()
    with open(path, "w") as f:
        f.write(note_body(row, ref, sid, url))
    ncmd = (["pr", "comment", str(ref[1]), "--repo", repo, "--body-file", str(path)]
            if ref[0] == "pr" else
            ["issue", "comment", str(ref[1]), "--repo", repo, "--body-file", str(path)])
    try:
        p = subprocess.run(["gh", *ncmd], capture_output=True, text=True, timeout=60)
        return p.returncode == 0, (None if p.returncode == 0 else p.stderr.strip()[:300])
    except Exception as e:
        return False, str(e)[:300]


# --- The launch sequence ----------------------------------------------

def collect_parked_claim(queue_name, task_id):
    """Claim THIS parked handoff by id, as the claimant. Returns
    (claimed_row, None) or (None, error). The claim output is the
    claimed row itself, a {"task": row} wrapper, or {"task": null}
    when the broker would not hand it over; only a row whose id is
    the requested task counts."""
    try:
        rc, out, err = queue("claim", "--queue", queue_name,
                             "--worker", CLAIMANT_WORKER,
                             "--platform", "openchamber",
                             "--task", str(task_id))
    except Exception as e:
        return None, str(e)[:300]
    data = None
    if out.strip():
        try:
            data = json.loads(out)
        except ValueError:
            data = None
    row = data.get("task") if isinstance(data, dict) and "task" in data else data
    if rc != 0 or not isinstance(row, dict) or row.get("id") != task_id:
        detail = (err.strip() or out.strip())[:300]
        return None, f"by-id claim did not return task {task_id}: {detail}"
    return row, None


def launch(queue_name, task_id):
    mode = claimant.read_mode()
    if mode != "live":
        return {"task": task_id, "queue": queue_name, "ok": False,
                "reason": f"mode_not_live: mode file reads {mode!r}; "
                          f"--launch requires 'live' (preflight is read-only)"}

    row, status, candidate, reasons = find_row(queue_name, task_id)
    if row is None or not candidate:
        return {"task": task_id, "queue": queue_name, "ok": False,
                "reason": "not_a_candidate", "detail": reasons}
    if exhausted(row):
        return {"task": task_id, "queue": queue_name, "ok": False,
                "reason": "attempts_exhausted",
                "attempts": f"{row.get('attempts')}/{row.get('max_attempts')}"}

    repo = repo_for(row, queue_name)
    directory = REPOS[queue_name][1]
    ref = ref_for(row)
    hs = hard_stop_reason(repo, ref)
    if hs:
        return {"task": task_id, "queue": queue_name, "ok": False, "reason": hs}

    cap_room = claimant.cap_step()
    if cap_room == -1:
        return {"task": task_id, "queue": queue_name, "ok": False,
                "reason": "cap_unknown: session count unreadable; launching nothing"}
    if cap_room <= 0:
        return {"task": task_id, "queue": queue_name, "ok": False,
                "reason": "cap_saturated", "cap_room": cap_room}

    worker_id = row.get("worker_id")
    if status == "parked":
        # No live claim exists to bind/checkpoint against: collect
        # this task's claim first. The successful by-id claim is the
        # broker's own verification that the row is a parked recorded
        # opencode handoff; the session then works as the claimant,
        # exactly as in claimant.py's parked path.
        claimed, claim_err = collect_parked_claim(queue_name, task_id)
        if claimed is None:
            return {"task": task_id, "queue": queue_name, "ok": False,
                    "reason": "parked_claim_failed", "detail": claim_err}
        worker_id = CLAIMANT_WORKER
    else:
        # Re-verify immediately before creating anything (claimant
        # step 4a): a state change means someone else owns it now.
        row2, _s2, cand2, reasons2 = find_row(queue_name, task_id)
        if row2 is None or not cand2:
            return {"task": task_id, "queue": queue_name, "ok": False,
                    "reason": "task_state_changed", "detail": reasons2}
        row = row2

    prov, mdl = route_for(row)
    title = f"queue task {task_id}" + (f" · {ref[0]} #{ref[1]}" if ref else "") \
        + f" · {str(row.get('title'))[:70]}"
    body = {"title": title, "location": {"directory": directory},
            "model": {"id": mdl, "providerID": prov}, "agent": "build"}

    # CREATE — through claimant.py's verified transport.
    status_code, payload = claimant._server_request(
        "POST", "/api/session", body=body, timeout=15)
    if not claimant._is_2xx(status_code):
        rec = {"kind": "launch_one", "task": task_id, "ok": False,
               "phase": "create",
               "error": f"session creation failed: http status "
                        f"{status_code if status_code is not None else 'unknown'}"}
        log(rec)
        return {"task": task_id, "queue": queue_name, **rec}
    sess = claimant._unwrap_envelope(payload)
    sid = sess.get("id") if isinstance(sess, dict) else None
    if not sid:
        rec = {"kind": "launch_one", "task": task_id, "ok": False,
               "phase": "create", "error": "no session id in create response"}
        log(rec)
        return {"task": task_id, "queue": queue_name, **rec}
    journal({"task": task_id, "queue": queue_name, "session_id": sid,
             "phase": "created"})

    # VERIFY LOCATION before binding.
    loc = (sess or {}).get("location") or {}
    got = loc.get("directory")
    if not got or os.path.realpath(got) != os.path.realpath(directory):
        deleted = claimant._delete_session(sid)
        args = ["checkpoint", str(task_id), "--worker", worker_id,
                "--stage", "launch_failed",
                "--progress", json.dumps({
                    "lane": "opencode", "launched_by": "opencode-claimant",
                    "launch_failed": "location_mismatch",
                    "requested_directory": directory,
                    "created_directory": got,
                    "session_id": None,
                    "launch_failed_session_id": sid})]
        rc, _out, err = queue(*args)
        rec = {"kind": "launch_one", "task": task_id, "session": sid,
               "ok": False, "phase": "location", "requested": directory,
               "got": got, "session_cleanup": "deleted" if deleted else "delete_unverified",
               "checkpoint_rc": rc, "checkpoint_err": err.strip()[:200]}
        log(rec)
        return {"task": task_id, "queue": queue_name, **rec}

    # BIND — a failure deletes the created session: no orphan.
    rc, _out, err = queue("bind", str(task_id), "--worker", CLAIMANT_WORKER,
                          "--session-id", sid, "--launched-by", CLAIMANT_WORKER)
    if rc != 0:
        deleted = claimant._delete_session(sid)
        rec = {"kind": "launch_one", "task": task_id, "session": sid,
               "ok": False, "phase": "bind", "error": err.strip()[:300],
               "session_cleanup": "deleted" if deleted else "delete_unverified"}
        log(rec)
        return {"task": task_id, "queue": queue_name, **rec}
    journal({"task": task_id, "queue": queue_name, "session_id": sid,
             "phase": "bound"})

    # CHECKPOINT launched, as the claiming worker (ownership).
    rc2, _o2, err2 = queue(
        "checkpoint", str(task_id), "--worker", worker_id,
        "--stage", "launched",
        "--progress", json.dumps({"lane": "opencode", "session_id": sid,
                                  "launched_by": "opencode-claimant"}))

    # PROMPT — verified send with reconcile-before-resend inside
    # claimant._send_prompt. A bound session that cannot be prompted
    # stays bound (supervision owns it); the launch reports failure.
    prompt = build_prompt(row, ref, repo, directory, sid, worker_id)
    prompt_ok = claimant._send_prompt(sid, prompt)

    # AGENT NOTE (non-fatal).
    note_ok, note_err = (None, None)
    if ref:
        note_ok, note_err = post_agent_note(row, ref, repo, sid)

    rec = {"kind": "launch_one", "task": task_id, "queue": queue_name,
           "session_id": sid, "session": sid, "repo": repo,
           "directory": directory, "route": f"{prov}/{mdl}",
           "worker_id": worker_id, "source_status": status,
           "ok": prompt_ok, "checkpoint_rc": rc2,
           "checkpoint_err": err2.strip()[:200] if rc2 != 0 else None,
           "prompt_ok": prompt_ok, "note_ok": note_ok,
           "note_error": note_err, "ref": ref, "title": row.get("title")}
    log(rec)
    journal({"task": task_id, "queue": queue_name, "session_id": sid,
             "phase": "prompted", "prompt_ok": prompt_ok})
    if not prompt_ok:
        rec = {**rec, "reason": "prompt_failed: binding stands; "
                                 "supervision owns the bound session"}
    return rec


# --- Preflight (read-only) ---------------------------------------------

def would_steps(queue_name, row, status, worker_id):
    directory = REPOS[queue_name][1]
    prov, mdl = route_for(row)
    steps = []
    if status == "parked":
        steps.append(
            f"collect the parked handoff's claim by id: repo-queue claim "
            f"--queue {queue_name} --worker {CLAIMANT_WORKER} "
            f"--platform openchamber --task {row.get('id')}")
    steps += [
        f"create an OpenCode session in {directory} (route {prov}/{mdl}, agent build)",
        "verify the created session's location.directory equals that primary; "
        "on mismatch delete it, checkpoint launch_failed, stop",
        f"bind it: repo-queue bind {row.get('id')} --worker {CLAIMANT_WORKER} "
        f"--session-id <new session> --launched-by {CLAIMANT_WORKER} "
        f"(a bind failure deletes the created session)",
        f"checkpoint stage launched as --worker {worker_id}",
        "send the execution prompt (verified; reconcile before one resend)",
        "post the agent-note on the issue/PR",
    ]
    return steps


def preflight(queue_name, task_id):
    row, status, candidate, reasons = find_row(queue_name, task_id)
    if row is None:
        return {"task": task_id, "queue": queue_name, "candidate": False,
                "ok": False, "reasons": reasons, "blockers": reasons,
                "would": []}
    repo = repo_for(row, queue_name)
    ref = ref_for(row)
    blockers = list(reasons)
    exh = exhausted(row)
    if candidate and exh:
        blockers.append(
            f"attempts exhausted ({row.get('attempts')}/{row.get('max_attempts')})")
    hs = hard_stop_reason(repo, ref) if candidate and not exh else None
    if hs:
        blockers.append(hs)
    cap_room = claimant.cap_step()
    if cap_room == -1:
        blockers.append("cap_unknown: session count unreadable; a launch would refuse")
    elif cap_room <= 0:
        blockers.append(f"cap_saturated: cap_room={cap_room}")
    mode = claimant.read_mode()
    if mode != "live":
        blockers.append(f"mode_not_live: mode file reads {mode!r}; --launch would refuse")
    worker_id = row.get("worker_id") or (
        CLAIMANT_WORKER if status == "parked" else None)
    return {
        "task": task_id, "queue": queue_name, "candidate": candidate,
        "ok": candidate and not blockers,
        "status": status, "stage": (row.get("progress") or {}).get("stage"),
        "title": row.get("title"), "worker": worker_id,
        "attempts": f"{row.get('attempts')}/{row.get('max_attempts')}",
        "exhausted": exh, "ref": ref, "hard_stop": hs, "repo": repo,
        "directory": REPOS[queue_name][1], "route": route_for(row),
        "cap_room": cap_room if cap_room != -1 else None, "mode": mode,
        "blockers": blockers,
        "would": would_steps(queue_name, row, status, worker_id) if candidate else [],
    }


# --- CLI -----------------------------------------------------------------

def parse_spec(spec):
    queue_name, sep, task_text = spec.partition(":")
    if not sep or queue_name not in REPOS or not task_text.isdigit():
        raise ValueError(
            f"bad spec {spec!r}: expected <queue>:<task_id> with queue in "
            f"{sorted(REPOS)}")
    return queue_name, int(task_text)


def main(argv):
    if len(argv) >= 2 and argv[1] in ("--help", "-h", "help"):
        print(__doc__)
        return 0
    if len(argv) < 3 or argv[1] not in ("--preflight", "--launch"):
        print(__doc__)
        return 2
    mode = argv[1]
    try:
        specs = [parse_spec(s) for s in argv[2:]]
    except ValueError as e:
        print(f"error: {e}", file=sys.stderr)
        return 2
    if mode == "--launch" and len(specs) != 1:
        print("error: --launch takes exactly one <queue>:<task_id> "
              "(this is the per-task trigger)", file=sys.stderr)
        return 2
    if mode == "--preflight":
        out = [preflight(q, t) for q, t in specs]
        print(json.dumps(out, indent=1))
        return 0
    rec = launch(specs[0][0], specs[0][1])
    print(json.dumps(rec, indent=1))
    if rec.get("ok"):
        print(f"session_id: {rec.get('session_id')}")
        return 0
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
