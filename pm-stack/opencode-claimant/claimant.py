#!/usr/bin/env python3
"""
OpenCode lane claimant/executor - Slice 1 implementation

This script implements the opencode execution lane claimant/executor for slice 1 of #3026.
It handles:
1. Supervision of bound sessions (checking for stuck/zero-message sessions)
2. Candidate discovery from the four opencode queues
3. Session creation and binding with atomic operations
4. Prompt delivery to worker sessions
5. Fire logging and reporting

Mode: shadow (default) or live (read from ~/.local/state/opencode-claimant/mode)

Usage: python3 pm-stack/opencode-claimant/claimant.py

Argument contract (#3352, doc/agents/operator-script-argument-contract.md):
this script takes NO command-line arguments. The command line is parsed
strictly (argparse) before any file read, subprocess, network, claim, or
launch step; any argument at all (including --help and unknown flags) is
a usage error with exit 2.
"""

import argparse
import base64
import fcntl
import json
import os
import subprocess
import sys
import time
import urllib.request
import urllib.error
from datetime import datetime, timezone
from pathlib import Path
from typing import Dict, List, Optional, Any


def _required_home() -> Path:
    """Home per the #5762 home-resolution contract (code#8221): a
    non-empty HOME, or a named error -- never the passwd fallback
    Path.home() applies when HOME is unset. The claimant is fired
    programmatically (OpenChamber cron), a context with no HOME
    guarantee, and every path anchored below is fleet state."""
    home = os.environ.get("HOME")
    if not home:
        raise RuntimeError(
            "HOME is unset or empty: opencode-claimant cannot resolve "
            "its state paths (#5762 home-resolution contract)"
        )
    return Path(home)


HOME_DIR = _required_home()


# Constants
MODE_FILE = HOME_DIR / ".local" / "state" / "opencode-claimant" / "mode"
FIRES_FILE = HOME_DIR / ".local" / "state" / "opencode-claimant" / "fires.jsonl"
STATE_DIR = HOME_DIR / ".local" / "state" / "opencode-claimant"

QUEUES = [
    "repo-health-code-pilot",
    "repo-health-opencode-pilot",
    "repo-health-openchamber-pilot",
    "repo-health-nix-pilot",
]

# Packaged fleet binary via its repo-queue argv0 alias
# (~/.local/bin/repo-queue -> ~/.local/bin/fleet); the hand-built
# ~/fleet-brokers/bin tree was retired by the code#5362 cutover
# (task 12731).
REPO_QUEUE_CMD = HOME_DIR / ".local" / "bin" / "repo-queue"
PRIMARY_CLONE = HOME_DIR / "git" / "code"

# OpenCode server coordinates: the standing server requires HTTP
# Basic auth (user "opencode", password in its state file — the same
# credential poll.py and the pm-dispatch client read). Calls made
# without it are rejected, so every session call below carries it.
OPENCODE_URL = os.environ.get("OPENCODE_URL", "http://127.0.0.1:13420")
AUTH_FILE = os.environ.get(
    "OPENCODE_AUTH_FILE",
    str(HOME_DIR / ".local/state/opencode/service-prod.json"),
)


def _auth_header_args() -> List[str]:
    """curl args carrying the server's Basic auth, or [] when the
    credential cannot be read (the server then rejects the call and
    the caller fails closed). The value is never logged."""
    try:
        with open(AUTH_FILE) as f:
            auth = json.load(f)
        password = auth.get("password") if isinstance(auth, dict) else None
        if not password:
            return []
        token = base64.b64encode(f"opencode:{password}".encode()).decode()
        return ["-H", f"Authorization: Basic {token}"]
    except Exception:
        return []


# --- Verified-read server transport (#7770 Phase 2: #7367, #7760) ---
#
# The seam contract (design #7770, decision 2): EVERY OpenCode server
# call in this module goes through _server_request - one authenticated
# transport - and every verdict is taken from the HTTP status plus the
# body's shape. A curl exit code is never a verdict: curl -s exits 0
# on a 401/404, which is how an auth error body used to read as a gone
# session and a failed DELETE as a completed one (#7367).


def _server_request(method: str, path: str, body: Any = None, timeout: int = 5):
    """One authenticated server call. Returns (status, payload):
    status is the HTTP status int, or None when no status was
    captured (transport failure - the reply is UNKNOWN); payload is
    the parsed JSON body, or None when the body is empty or not
    JSON. The auth header rides on every call; its value is never
    logged or returned."""
    cmd = ["curl", "-s", "-m", str(timeout), "-w", "\n%{http_code}"]
    if method != "GET":
        cmd += ["-X", method]
    if body is not None:
        cmd += ["-H", "Content-Type: application/json", "-d", json.dumps(body)]
    cmd += _auth_header_args()
    cmd.append(f"{OPENCODE_URL}{path}")
    try:
        result = subprocess.run(cmd, capture_output=True, text=True)
    except Exception:
        return None, None
    raw = result.stdout or ""
    text, sep, status_text = raw.rpartition("\n")
    if not sep or not status_text.strip().isdigit():
        # No status captured: without it there is no verified read,
        # whatever curl's exit code says.
        return None, None
    payload = None
    if text.strip():
        try:
            payload = json.loads(text)
        except Exception:
            payload = None
    return int(status_text.strip()), payload


def _unwrap_envelope(payload):
    """The writer's envelope: {"data": ...} unwraps to its data;
    anything else passes through unchanged."""
    if isinstance(payload, dict) and "data" in payload:
        return payload["data"]
    return payload


def _is_2xx(status) -> bool:
    return status is not None and 200 <= status < 300


def _get_session(session_id: str):
    """Verified per-id session read. Returns (verdict, session):
    "ok" with the session dict on a 2xx carrying a session object
    with an id; "gone" only on a 404 from this GET (contract
    decision 2: gone is a 404 verdict, never a body-shape
    accident); "unknown" with None for every other outcome - auth
    failures, 5xx, transport failures and mis-shaped 2xx bodies
    never authorize an action."""
    status, payload = _server_request("GET", f"/api/session/{session_id}", timeout=5)
    if status == 404:
        return "gone", None
    if _is_2xx(status):
        session = _unwrap_envelope(payload)
        if isinstance(session, dict) and session.get("id"):
            return "ok", session
    return "unknown", None


def _delete_session(session_id: str) -> bool:
    """DELETE through the verified transport: True only on a 2xx
    verdict. A non-2xx delete did not happen, whatever curl said."""
    status, _payload = _server_request(
        "DELETE", f"/api/session/{session_id}", timeout=5
    )
    return _is_2xx(status)


# Concurrency cap
CONCURRENCY_CAP = 36

# Recovery launched_by values
LAUNCHED_BY_RECOVERY_ZERO_MESSAGE = "opencode-claimant-recovery:zero-message-deleted"
LAUNCHED_BY_RECOVERY_GONE_SESSION = "opencode-claimant-recovery:gone-session"


# Where this process's mode came from (#10332): the file's path and
# state, and the mtimes that let a later reader date a flip. Set by
# read_mode(), written into every fires.jsonl entry by log_fire().
_mode_source: "Dict[str, Any] | None" = None


def _mtime(path: Path) -> "str | None":
    """An ISO UTC mtime for path, or None when it cannot be read."""
    try:
        return datetime.fromtimestamp(path.stat().st_mtime, timezone.utc).isoformat()
    except OSError:
        return None


def read_mode() -> str:
    """Read the execution mode from the mode file.

    Fail-safe default (#10141, close-authority program #10136 P5):
    a missing, unreadable, or invalid mode file still defaults to
    "shadow" — the non-mutating behaviour is the right safety
    default — but it is NEVER silent about it. Before #10141 a
    missing file was indistinguishable from an intentional shadow
    and summit dispatch stalled on it with no signal anywhere; now
    every such read logs a loud CLAIMANT-MODE-WARNING line, and the
    aggregator's host-health watchdog surfaces the same fact as a
    needs-attention tier item.

    Every read also records where the mode came from (#10332) in
    _mode_source, which log_fire() writes into fires.jsonl: the path,
    whether the file was ok, missing, unreadable or invalid, the
    file's mtime when it exists, and the state directory's mtime. A
    deletion changes the directory's mtime, so when a fire reads
    "missing", the directory mtime bounds when the file went.
    """
    global _mode_source
    source: Dict[str, Any] = {
        "path": str(MODE_FILE),
        "state": "ok",
        "file_mtime": None,
        "dir_mtime": _mtime(MODE_FILE.parent),
        "pid": os.getpid(),
    }
    _mode_source = source

    if not MODE_FILE.exists():
        source["state"] = "missing"
        print(
            f"CLAIMANT-MODE-WARNING (#10141): mode file {MODE_FILE} is MISSING "
            "- defaulting to shadow; dispatch will return mode_not_live and "
            "stall until the file is restored with the intended mode. "
            f"State directory last changed {source['dir_mtime']}.",
            file=sys.stderr,
        )
        return "shadow"

    source["file_mtime"] = _mtime(MODE_FILE)
    try:
        raw = MODE_FILE.read_text()
    except Exception as exc:
        source["state"] = "unreadable"
        print(
            f"CLAIMANT-MODE-WARNING (#10141): mode file {MODE_FILE} is "
            f"UNREADABLE ({exc}) - defaulting to shadow.",
            file=sys.stderr,
        )
        return "shadow"
    mode = raw.strip().lower()
    if mode in ("shadow", "live"):
        return mode
    source["state"] = "invalid"
    print(
        f"CLAIMANT-MODE-WARNING (#10141): mode file {MODE_FILE} content "
        f"{raw.strip()[:40]!r} is not live|shadow - defaulting to shadow.",
        file=sys.stderr,
    )
    return "shadow"


def ensure_state_dir() -> None:
    """Ensure the state directory exists."""
    STATE_DIR.mkdir(parents=True, exist_ok=True)


def log_fire(
    mode: str,
    candidates: "List[Dict] | None",
    launched: List[Dict],
    cap_room: "int | None",
    skips: List[Dict],
    health: "str | None" = None,
    provider_wall: "Dict | None" = None,
    hand_back: "Dict | None" = None,
    babysitter: "Dict | None" = None,
) -> None:
    """Append a fire log entry to fires.jsonl.

    cap_room is the measured available-slot count, or None when the
    cap is unknown (#7732): null means unknown, never a placeholder
    0 (which would read as a saturated fleet) and never the cap
    step's -1 sentinel, which is an internal return value, not a
    measurement. candidates is likewise None when candidate
    discovery itself failed: candidates_seen null (I could not
    read) is a different state from [] (I read, and the queue was
    empty) — the #7770 Phase 4 audit found the gate exit logging
    the two identically.
    """
    ensure_state_dir()

    fire_entry = {
        "timestamp": datetime.now(timezone.utc).isoformat(),
        "mode": mode,
        "mode_source": _mode_source,
        "candidates_seen": candidates,
        "sessions_launched": launched,
        "cap_room": cap_room,
        "skips": skips,
        "queue_health": health,
        "provider_wall": provider_wall,
        "hand_back": hand_back,
        "babysitter": babysitter,
    }

    with FIRES_FILE.open("a") as f:
        f.write(json.dumps(fire_entry) + "\n")


def run_poll() -> Dict[str, Any]:
    """Run poll.py to discover candidates and get system state."""
    poll_script = PRIMARY_CLONE / "pm-stack" / "opencode-claimant" / "poll.py"

    if not poll_script.exists():
        raise FileNotFoundError(f"poll.py not found at {poll_script}")

    # Run poll.py from the primary clone
    result = subprocess.run(
        ["python3", str(poll_script)],
        cwd=str(PRIMARY_CLONE),
        capture_output=True,
        text=True,
        timeout=30,
    )

    if result.returncode != 0:
        raise RuntimeError(f"poll.py failed: {result.stderr}")

    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError as e:
        raise RuntimeError(f"poll.py returned invalid JSON: {e}")


_poll_module = None


def _load_poll_module():
    """Load the sibling poll.py as a module (cached).

    Cap counting delegates to poll.py's pinned functions so both entry
    points read the same verified live session shape (root = no
    parentID/parent_id, activity = time.updated falling back to
    time.created, finished = time.idle or a top-level outcome, the
    server's active set authoritative) and can never diverge again
    (#6609)."""
    global _poll_module
    if _poll_module is None:
        import importlib.util

        poll_path = Path(__file__).resolve().parent / "poll.py"
        spec = importlib.util.spec_from_file_location("claimant_poll", poll_path)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        _poll_module = module
    return _poll_module


_babysitter_module = None


def _load_babysitter_module():
    """Load the sibling babysitter.py as a module (cached), the
    same pattern as poll.py: the babysitter's verdict rounds
    (#1338) ride the claimant fire, and babysitter.py owns the
    round engine so the fire only carries its record."""
    global _babysitter_module
    if _babysitter_module is None:
        import importlib.util

        babysitter_path = Path(__file__).resolve().parent / "babysitter.py"
        spec = importlib.util.spec_from_file_location(
            "claimant_babysitter", babysitter_path
        )
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        _babysitter_module = module
    return _babysitter_module


def get_active_root_session_count() -> int:
    """Get the count of active CLAIMANT root sessions from the OpenCode
    server.

    Delegates to poll.py: fetch the session list and the server's
    active set, then count_active_claimant_roots over both — scheduled
    fleet sessions (lane runners, the claimant's own fire) are outside
    the cap per ruling `session-cap-5016` (#5016). Returns -1
    (unknown) when either read fails, so cap_step fails closed."""
    try:
        poll = _load_poll_module()
        sessions = poll.fetch_sessions()
        if sessions is None:
            return -1  # Unknown
        active_ids = poll.fetch_active_ids()
        if active_ids is None:
            return -1  # Unknown
        count = poll.count_active_claimant_roots(
            sessions, int(time.time() * 1000), active_ids
        )
        return -1 if count is None else count
    except Exception as e:
        print(f"Warning: Could not get active session count: {e}")
        return -1  # Unknown


# Fire-start state recorded by gate_step (#8038): the candidate
# snapshot this fire saw first, and the queue-health line gate_step
# measured. main() threads both into every log_fire call so a fire
# record answers what was eligible and what the queues reported,
# instead of the hardcoded [] the #8038 audit found.
_fire_candidates = None
_fire_health = None


def _fetch_sessions():
    """poll.py's verified session list, or None when unreadable."""
    try:
        return _load_poll_module().fetch_sessions()
    except Exception:
        return None


def _fetch_active_ids():
    """poll.py's server active-id set, or None when unreadable."""
    try:
        return _load_poll_module().fetch_active_ids()
    except Exception:
        return None


def run_queue_health():
    """The fleet queue health line for the queues (prompt.md step
    2), or None when the health verb cannot be read: an unreadable
    health state is recorded as null, never invented."""
    try:
        result = subprocess.run(
            [str(REPO_QUEUE_CMD), "health"],
            capture_output=True,
            text=True,
            timeout=15,
        )
    except Exception:
        return None
    if result.returncode != 0:
        return None
    lines = [ln.strip() for ln in (result.stdout or "").splitlines() if ln.strip()]
    return " | ".join(lines)[:500] or None


def _fail_attempts_exhausted(task, session_id) -> bool:
    """Step 1e: terminally fail a bound task whose attempts are
    exhausted, as its worker_id, carrying the
    launch_failed_attempts_exhausted execution record so the bound
    session id survives in the record."""
    prog = task.get("progress") or {}
    worker_id = task.get("worker_id") or prog.get("worker_id") or CLAIMANT_WORKER
    execution = json.dumps(
        {
            "lane": "opencode",
            "session_id": session_id,
            "fallback_reason": "launch_failed_attempts_exhausted",
        }
    )
    try:
        result = subprocess.run(
            [
                str(REPO_QUEUE_CMD),
                "fail",
                str(task["id"]),
                "--worker",
                str(worker_id),
                "--error",
                f"bound session {session_id} unavailable without terminal outcome; attempts exhausted",
                "--reason-class",
                "deterministic",
                "--execution",
                execution,
            ],
            capture_output=True,
            text=True,
            timeout=10,
        )
    except Exception as e:
        print(f"Failed to terminally fail exhausted task {task.get('id')}: {e}")
        return False
    return result.returncode == 0


def supervision_step(mode: str) -> List[Dict]:
    """
    Step 1: Supervision - check bound sessions for recovery candidates.

    Selection consumes poll.py's is_stuck_bound (#8038): a present
    session is a recovery candidate only when poll.py selects it
    (marker-less root, NOT in the server's active set, silent for at
    least ACTIVE_WINDOW_MS, lease expired) - claimant.py no longer
    reimplements that selection, and its old inverted terminal-marker
    test is gone. A session the per-id GET proves gone (404) is the
    step-1d gone case, gated on an expired lease. Every other state
    (terminal marker, active-set membership, recent activity, live
    lease, any unreadable read) leaves the task bound.

    A bound task with attempts >= max_attempts is never skipped
    silently (step 1e): a stuck zero-message session is deleted
    first, a gone session's absence is already verified, and the
    task is terminally failed with the
    launch_failed_attempts_exhausted execution record.

    Returns the list of tasks acted on (recovery candidates and
    attempts-exhausted terminal fails).
    """
    if mode != "live":
        return []

    recovery_candidates = []

    # Get all claimed opencode-lane tasks with bound session IDs.
    # Rows come from poll.py's broker rich read: the repo-queue
    # binary's list verb prints a human table, and json-parsing it
    # silently supervised nothing (#7325).
    poll = _load_poll_module()
    now_ms = int(time.time() * 1000)
    sessions = _fetch_sessions()
    sessions_by_id = (
        {s.get("id"): s for s in sessions if isinstance(s, dict) and s.get("id")}
        if isinstance(sessions, list)
        else {}
    )
    active_ids = _fetch_active_ids()
    for queue in QUEUES:
        try:
            for task in poll.fetch_claimed_rows(queue):
                # Rich rows carry lane/session in progress; a
                # flattened row carries them at top level.
                prog = task.get("progress") or {}
                lane = prog.get("lane") or task.get("lane")
                session_id = prog.get("session_id") or task.get("session_id")
                if lane != "opencode" or not session_id:
                    continue
                # The verdict comes from the verified transport:
                # "gone" is a 404 from the per-id GET; "unknown"
                # (auth failure, 5xx, transport failure, mis-shaped
                # body) never authorizes recovery - the task stays
                # bound (#7367, #7770 decision 2).
                verdict, _session = _get_session(session_id)
                if verdict == "unknown":
                    continue
                attempts = task.get("attempts", 0)
                max_attempts = task.get("max_attempts", 5)
                exhausted = attempts >= max_attempts
                if verdict == "ok":
                    # Present session: poll.py is the only selector.
                    # A session absent from the verified list cannot
                    # be proven stuck-bound here (the gone case is
                    # the per-id 404 above, not a list absence).
                    list_entry = sessions_by_id.get(session_id)
                    if list_entry is None:
                        continue
                    poll_row = {
                        "progress": {"lane": lane, "session_id": session_id},
                        "lease_until": task.get("lease_until"),
                    }
                    if not poll.is_stuck_bound(
                        poll_row, list_entry, active_ids, now_ms
                    ):
                        continue
                    # poll.py surfaces the candidate only; zero
                    # messages is verified here before acting. An
                    # unreadable session is UNKNOWN, never empty
                    # (#3026).
                    messages = _get_messages(session_id)
                    if messages is None:
                        print(
                            f"Supervision: message read for session "
                            f"{session_id} was unknown; leaving task "
                            f"{task['id']} bound"
                        )
                        continue
                    if len(messages) > 0:
                        continue
                    if exhausted:
                        # Step 1e: verified delete first, then the
                        # terminal fail - never the other way round.
                        if _delete_session(session_id) and (
                            _fail_attempts_exhausted(task, session_id)
                        ):
                            recovery_candidates.append(
                                {
                                    "task_id": task["id"],
                                    "session_id": session_id,
                                    "reason": "attempts_exhausted",
                                }
                            )
                        continue
                    recovery_candidates.append(
                        {
                            "task_id": task["id"],
                            "session_id": session_id,
                            "reason": "zero_message",
                        }
                    )
                    continue
                # verdict == "gone" (step 1d): only an expired lease
                # is actionable - a live lease is still owned by a
                # worker that may be executing, and an unreadable
                # lease is not expired (poll.py's rule).
                lease_ms = poll.parse_lease_ms(task.get("lease_until"))
                if lease_ms is None or lease_ms > now_ms:
                    continue
                if exhausted:
                    # Step 1e for a gone session: absence is already
                    # verified by the 404, so fail terminally now.
                    if _fail_attempts_exhausted(task, session_id):
                        recovery_candidates.append(
                            {
                                "task_id": task["id"],
                                "session_id": session_id,
                                "reason": "attempts_exhausted",
                            }
                        )
                    continue
                messages = _get_messages(session_id)
                if messages is None:
                    print(
                        f"Supervision: message read for session "
                        f"{session_id} was unknown; leaving task "
                        f"{task['id']} bound"
                    )
                    continue
                if len(messages) == 0:
                    recovery_candidates.append(
                        {
                            "task_id": task["id"],
                            "session_id": session_id,
                            "reason": "zero_message",
                        }
                    )
        except Exception as e:
            print(f"Warning: Supervision check failed for queue {queue}: {e}")

    # Process zero-message recovery candidates (attempts-exhausted
    # candidates were already terminally failed above).
    for candidate in recovery_candidates:
        if candidate["reason"] != "zero_message":
            continue
        task_id = candidate["task_id"]
        session_id = candidate["session_id"]

        try:
            # Delete the zero-message session. Only a verified 2xx
            # DELETE counts: a non-2xx delete did not happen, and the
            # binding must not be cleared over a live session (#7367).
            if _delete_session(session_id):
                # Attempt recovery via atomic bind operation
                bind_result = subprocess.run(
                    [
                        str(REPO_QUEUE_CMD),
                        "bind",
                        str(task_id),
                        "--worker",
                        "opencode-claimant",
                        "--session-id",
                        session_id,
                        "--launched-by",
                        LAUNCHED_BY_RECOVERY_ZERO_MESSAGE,
                    ],
                    capture_output=True,
                    text=True,
                    timeout=10,
                )

                if bind_result.returncode == 0:
                    print(
                        f"Recovered task {task_id}: deleted zero-message session {session_id}"
                    )
                    # Task is now unbound and eligible for next fire
                else:
                    print(f"Failed to recover task {task_id}: {bind_result.stderr}")
            else:
                print(f"Failed to delete session {session_id} for task {task_id}")
        except Exception as e:
            print(f"Error during recovery of task {task_id}: {e}")

    return recovery_candidates


def gate_step() -> Optional[bool]:
    """
    Step 2: Gate - check if there are any unbound opencode-lane candidates.

    Returns True if there are candidates to process, False if the
    (verified) candidate list holds none, and None when discovery
    itself failed: an unreadable queue is UNKNOWN, not empty
    (#7770 contract — unknown never authorizes a side effect and
    is never recorded as a measurement).

    Side records (#8038): the gate runs the fleet queue health verb
    for the queues (prompt.md step 2) and stores the health line in
    the module's fire record (`_fire_health`), and stores the
    candidate list it read as the fire-start snapshot
    (`_fire_candidates`) that main() threads into every log_fire
    call and launch_step's candidate set.
    """
    global _fire_candidates, _fire_health
    _fire_candidates = None
    _fire_health = run_queue_health()
    try:
        poll_data = run_poll()
        candidates = poll_data.get("candidates", [])
        _fire_candidates = list(candidates)

        # Filter for unbound opencode-lane tasks
        unbound_opencode = [
            c
            for c in candidates
            if c.get("lane") == "opencode" and not c.get("session_id")
        ]

        if unbound_opencode:
            return True

        # No verified candidates. Before concluding the queue is
        # empty, check poll's per-queue read state (#8042): a queue
        # whose broker read failed is UNKNOWN, not empty — the fire
        # must log gate_unknown, never the false "no candidates"
        # (the 2026-11-09 12:17Z fire logged no_candidates while the
        # queue held four handed_off candidates it could not read).
        failed_queues = [
            q
            for q, qdata in (poll_data.get("queues") or {}).items()
            if isinstance(qdata, dict) and qdata.get("read_ok") is False
        ]
        if failed_queues:
            print(
                "Gate check: queue read failed for "
                + ", ".join(sorted(failed_queues))
                + "; candidate discovery unknown."
            )
            return None

        return False
    except Exception as e:
        print(f"Gate check failed: {e}")
        return None


def cap_step() -> int:
    """
    Step 3: Cap - check if we're at the concurrency limit.

    Returns cap_room (available slots) or -1 if unknown.
    """
    active_count = get_active_root_session_count()

    if active_count == -1:
        return -1

    cap_room = max(0, CONCURRENCY_CAP - active_count)
    return cap_room


OPENCODE_WORKING_HOOK = (
    Path(__file__).resolve().parent.parent / "health-hooks" / "opencode-working.sh"
)


def fire_opencode_working_hook(caller: str, detail: str) -> None:
    """C3 health hook (#9571): fire-and-forget opencode-working when a
    launch cannot read OpenCode's session state. Never raises and never
    waits; the hook owns probing, dedup and recording."""
    try:
        subprocess.Popen(
            [str(OPENCODE_WORKING_HOOK), "--caller", caller, "--detail", detail],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
        )
    except OSError as e:
        print(f"Warning: could not fire opencode-working hook: {e}")


def _post_prompt(session_id: str, prompt: str) -> bool:
    """One prompt POST through the verified transport; True iff the
    server answered 2xx. A non-2xx POST did not land, whatever
    curl's exit code says."""
    global _last_prompt_wall
    status, payload = _server_request(
        "POST", f"/api/session/{session_id}/prompt", body={"text": prompt}, timeout=15
    )
    wall = _provider_wall_error(payload)
    if wall is not None:
        _last_prompt_wall = wall
    return _is_2xx(status)


def _get_messages(session_id: str) -> Optional[List]:
    """The session's message list, or None when it cannot be read
    (transport failure, non-2xx status, non-JSON body, error object,
    or any body without a list of messages): an unreadable session
    is an UNKNOWN delivery state, never an empty one.

    The v2 endpoint is /message, not /messages: on the standing
    server the latter 404s with an empty body, which without -f used
    to parse as [] and turn every unknown read into "zero messages"
    (#3026). The verified transport makes that non-2xx a non-read
    and strict parsing keeps an empty or error body from
    masquerading as a confirmed list."""
    status, payload = _server_request(
        "GET", f"/api/session/{session_id}/message", timeout=5
    )
    if not _is_2xx(status):
        return None
    data = _unwrap_envelope(payload)
    return data if isinstance(data, list) else None


def _send_prompt(session_id: str, prompt: str) -> bool:
    """Deliver the execution prompt to a bound session. Returns True
    iff the prompt is confirmed delivered. A failed POST is
    reconciled before any resend (prompt.md step 4): if a message
    already landed, the worker may be executing and must never be
    double-prompted; only a successful reconcile that proves the
    message list is EMPTY earns one resend to the same session. An
    inconclusive reconcile (unreadable session) never resends — the
    binding stands and the caller flags the launch instead."""
    if _post_prompt(session_id, prompt):
        return True
    messages = _get_messages(session_id)
    if messages is None:
        return False
    if len(messages) > 0:
        return True
    return _post_prompt(session_id, prompt)


CLAIMANT_WORKER = "opencode-claimant"

# #8176: the model every session this claimant launches carries
# (Step 4b's session_create_body reads these same constants). The
# sink-collection claim stamps it: a task collected here is worked
# by exactly such a session, and any later stamped claim (a
# transfer, a re-dispatch) overwrites the stamp with that claim's
# session model.
LAUNCH_MODEL_PROVIDER = "opencode-route"
LAUNCH_MODEL_ID = "workhorse"


# #8166: the provider-capacity wall. The route's membership ceiling
# answers a launch with a tier_limit_exceeded body; that is a
# route-wide capacity verdict, not a per-task failure, so the first
# wall answer closes the launch gate for the rest of the fire. The
# route exposes its capacity only through launch-shaped answers, so
# the first launch attempt is the probe: a create or prompt answer
# carrying the wall stops every later launch in the fire.
LAUNCHED_BY_RECOVERY_PROVIDER_WALL = "opencode-claimant-recovery:provider-wall"

# The fire's provider-wall record ({"error", "route",
# "serving_leaf"}) once launch_step meets the wall, else None;
# main() threads it into log_fire like _fire_candidates /
# _fire_health.
_fire_provider_wall: "Dict | None" = None

# The verbatim provider error from the most recent prompt POST when
# that POST was answered with the wall (#8166): _post_prompt records
# it here because _send_prompt's bool cannot carry it.
_last_prompt_wall: "str | None" = None


def _provider_wall_error(payload) -> "str | None":
    """The provider's verbatim error text when a server reply body
    carries the tier_limit_exceeded wall (#8166), else None. The
    search walks the parsed body (dict values, list items, strings):
    the wall may sit at the top level or nested in an error object,
    and the string that names the code is the provider's own
    message, returned unmodified. A dict's message/error string is
    preferred over a bare code value."""
    if isinstance(payload, dict):
        for key in ("message", "error"):
            value = payload.get(key)
            if isinstance(value, str) and "tier_limit_exceeded" in value:
                return value
        for value in payload.values():
            found = _provider_wall_error(value)
            if found is not None:
                return found
        return None
    if isinstance(payload, list):
        for item in payload:
            found = _provider_wall_error(item)
            if found is not None:
                return found
        return None
    if isinstance(payload, str):
        return payload if "tier_limit_exceeded" in payload else None
    return None


def _provider_wall_record(error: str) -> Dict:
    """The fire-log provider_wall entry (#8166): the verbatim
    provider error, the route, and the serving leaf, so the record
    distinguishes "no candidates" from "nowhere to run them"."""
    return {
        "error": error,
        "route": LAUNCH_MODEL_PROVIDER,
        "serving_leaf": LAUNCH_MODEL_ID,
    }


def _take_prompt_wall() -> "str | None":
    """Read-and-clear the prompt POST's wall record (#8166)."""
    global _last_prompt_wall
    wall = _last_prompt_wall
    _last_prompt_wall = None
    return wall


def _collect_handed_offs(queue: str, want: int) -> Dict[int, Dict]:
    """Collect up to `want` recorded handoffs from `queue`'s
    handoff sink WITHOUT naming a task id (#8097 Rule 1): each
    `repo-queue claim --handoff` lets the BROKER pick the next
    sink row (its ordinary pick order), claimed as the claimant,
    so the launch flow below has a live claim to bind against.
    Returns {task_id: claimed row}.

    A handed-off task rests in the sink (status 'handed_off',
    #8234), so without collection it is never in the claimed state
    the launch flow binds against. Collection used to name the
    task (`claim --task <id>`); that by-id claim is now the
    special-case verb (one-off revivals), and this default path
    does not depend on it — the broker's handoff pick only ever
    returns a sink row (status 'handed_off', unbound opencode
    yield), never a live claim or a genuine wait park.

    The claim declares the claimant's family, --platform
    openchamber (#8097 Rule 2): a sink row whose item carries the
    `muse` selector is not the opencode claimant's to collect.
    """
    collected: Dict[int, Dict] = {}
    for _ in range(max(want, 0)):
        try:
            result = subprocess.run(
                [
                    str(REPO_QUEUE_CMD),
                    "claim",
                    "--queue",
                    queue,
                    "--worker",
                    CLAIMANT_WORKER,
                    "--platform",
                    "openchamber",
                    "--handoff",
                    "--model-provider",
                    LAUNCH_MODEL_PROVIDER,
                    "--model-id",
                    LAUNCH_MODEL_ID,
                ],
                capture_output=True,
                text=True,
                timeout=15,
            )
            data = json.loads(result.stdout) if result.stdout.strip() else None
        except Exception:
            break
        # cmd_claim prints the claimed row itself ({"task": null}
        # when nothing was claimed).
        row = data if isinstance(data, dict) else None
        claimed_id = row.get("id") if isinstance(row, dict) else None
        if claimed_id is None:
            break
        collected[claimed_id] = row
    return collected


HELD_ISSUE_LABELS = {"held", "user/owned", "do-not-merge", "do-not-auto-edit"}

_CREDENTIAL_KEY_PARTS = (
    "password",
    "secret",
    "token",
    "credential",
    "api_key",
    "apikey",
    "private_key",
)


def _github_issue_labels(repo: str, issue_number: int) -> set:
    """Live label names on an issue, read through the gh CLI.
    Raises on any failure so the caller can fail closed (#7670)."""
    result = subprocess.run(
        [
            "gh",
            "issue",
            "view",
            str(issue_number),
            "--repo",
            repo,
            "--json",
            "labels",
            "--jq",
            "[.labels[].name]",
        ],
        capture_output=True,
        text=True,
        timeout=15,
    )
    if result.returncode != 0:
        raise RuntimeError(f"gh issue view failed: {result.stderr.strip()[:200]}")
    names = json.loads(result.stdout)
    if not isinstance(names, list):
        raise RuntimeError("gh issue view returned a non-list label set")
    return {str(n) for n in names}


def _prompt_safe_packet(packet):
    """Deep-copy a packet for the execution prompt with any
    credential-shaped values redacted (#7670): a credential must
    never enter a prompt, log, or payload surface, and packets are
    assembled by many hands - strip by key name, recursively."""
    if isinstance(packet, dict):
        safe = {}
        for key, value in packet.items():
            lowered = str(key).lower()
            if any(part in lowered for part in _CREDENTIAL_KEY_PARTS):
                safe[key] = "[redacted]"
                continue
            safe[key] = _prompt_safe_packet(value)
        return safe
    if isinstance(packet, list):
        return [_prompt_safe_packet(v) for v in packet]
    return packet


def _post_launch_agent_note(task_id, session_id, candidate, packet) -> bool:
    """Step 4e (#8038): post the agent-note:v1 handed-off note on the
    task's issue naming the task id + session id, after a successful
    bind. The note text is a fixed template carrying only the two
    ids - no packet content enters it. A failed note never unbinds
    or fails the launch (the binding stands; supervision owns the
    session) - the caller flags the launch entry instead. Returns
    True iff the post is confirmed by the gh CLI's exit code."""
    _packet = packet if isinstance(packet, dict) else {}
    repo = _packet.get("repo") or ""
    issue_no = candidate.get("issue_number")
    if issue_no is None and isinstance(_packet.get("issue"), dict):
        issue_no = _packet["issue"].get("number")
    if issue_no is None:
        raw_issue = candidate.get("issue") or ""
        digits = "".join(ch for ch in str(raw_issue) if ch.isdigit())
        issue_no = digits or None
    if not repo or not issue_no:
        return False
    note = (
        "<!-- agent-note:v1 agent=opencode-claimant state=handed off "
        f"task={task_id} session={session_id} -->\n\n"
        f"Agent note - handed off: queue task {task_id} is bound to "
        f"opencode session {session_id}, launched by the claimant "
        f"(step 4e). The session works the task under its bound "
        f"claim; terminal state is recorded by the worker session."
    )
    try:
        result = subprocess.run(
            [
                "gh",
                "issue",
                "comment",
                str(int(str(issue_no).lstrip("#"))),
                "--repo",
                str(repo),
                "--body",
                note,
            ],
            capture_output=True,
            text=True,
            timeout=15,
        )
    except Exception:
        return False
    return result.returncode == 0


HANDBACK_REASON_CLASS = "executor-saturated"

HANDBACK_PARK_REASON = (
    "executor-saturated: the opencode session cap is saturated "
    f"({CONCURRENCY_CAP} active sessions), so no slot was available "
    "this round; handed back by opencode-claimant (#1187) - releases "
    "when capacity returns (the queue wake verb) or on an owner's "
    "explicit order; a Claude lane can still claim it if urgent"
)


def _claim_for_handback(candidate) -> bool:
    """Targeted-claim one candidate as the claimant so the
    hand-back park has a live claim to act on (the broker's park
    verb only parks a task its caller holds). True iff the claim
    answer names this candidate's id - a claim that lands on a
    different row, or no row, hands nothing back."""
    argv = [str(REPO_QUEUE_CMD), "claim"]
    if candidate.get("queue"):
        argv += ["--queue", str(candidate["queue"])]
    argv += [
        "--worker",
        CLAIMANT_WORKER,
        "--platform",
        "openchamber",
        "--task",
        str(candidate["id"]),
    ]
    try:
        result = subprocess.run(argv, capture_output=True, text=True, timeout=15)
        data = json.loads(result.stdout) if result.stdout.strip() else None
    except Exception:
        return False
    return isinstance(data, dict) and data.get("id") == candidate["id"]


def _park_handback(task_id) -> bool:
    """Park a claimed task as a capacity wait: reason class
    `executor-saturated` (#1187) plus the broker's capacity
    ready-condition, which floors the wait at the capacity scale
    and lets the pick take the task early when a slot frees -
    the hand-back is a pause with a named release, not a failure."""
    try:
        result = subprocess.run(
            [
                str(REPO_QUEUE_CMD),
                "park",
                str(task_id),
                "--worker",
                CLAIMANT_WORKER,
                "--reason",
                HANDBACK_PARK_REASON,
                "--ready-condition",
                json.dumps({"kind": "capacity"}),
            ],
            capture_output=True,
            text=True,
            timeout=15,
        )
    except Exception:
        return False
    return result.returncode == 0


def _post_handback_agent_note(task_id, candidate) -> bool:
    """Post the agent-note:v1 handed-back note on the task's issue
    (#1187: a hand-back is noted on the issue, never only in the
    local fire log). Mirrors _post_launch_agent_note's repo/issue
    resolution and failure contract: a failed note never undoes
    the park - the caller records it in the hand-back record.
    Returns True iff the post is confirmed by the gh CLI's exit
    code."""
    packet = (
        candidate.get("packet") if isinstance(candidate.get("packet"), dict) else {}
    )
    repo = packet.get("repo") or ""
    issue_no = candidate.get("issue_number")
    if issue_no is None and isinstance(packet.get("issue"), dict):
        issue_no = packet["issue"].get("number")
    if issue_no is None:
        raw_issue = candidate.get("issue") or ""
        digits = "".join(ch for ch in str(raw_issue) if ch.isdigit())
        issue_no = digits or None
    if not repo or not issue_no:
        return False
    note = (
        "<!-- agent-note:v1 agent=opencode-claimant state=handed back "
        f"task={task_id} -->\n\n"
        f"Agent note - handed back: queue task {task_id} was handed "
        "back by the opencode claimant (#1187): the opencode session "
        f"cap is saturated ({CONCURRENCY_CAP} active sessions), so no "
        "slot was available this round. The task is parked with reason "
        "`executor-saturated` and a capacity ready-condition - it "
        "releases when capacity returns (the queue's `wake` verb) or "
        "on an owner's explicit order. Saturation is a success state, "
        "not a failure: a Claude lane can still claim the task if "
        "urgent."
    )
    try:
        result = subprocess.run(
            [
                "gh",
                "issue",
                "comment",
                str(int(str(issue_no).lstrip("#"))),
                "--repo",
                str(repo),
                "--body",
                note,
            ],
            capture_output=True,
            text=True,
            timeout=15,
        )
    except Exception:
        return False
    return result.returncode == 0


def hand_back_step(mode, candidates, exclude_task_ids=None):
    """Hand-back (#1187 criterion 2): when the cap step finds the
    location saturated with launchable candidates waiting, hand
    the NEXT candidate back within the same fire - claim it, park
    it as an `executor-saturated` capacity wait, and note it on
    its issue - instead of leaving it silently queued with the
    only record in the local fire log.

    Live mode only: shadow fires mutate nothing and return None,
    as do fires with no launchable candidate waiting. The
    candidate is the first launchable one in the fire-start
    snapshot (launch_step's filter), skipping any task this
    fire's supervision recovered (next-fire-only, #8038 item 7).
    One hand-back per fire: the criterion is the next candidate,
    and a park per candidate per fire would churn the queue.
    Returns the hand-back record for the fire log."""
    if mode != "live":
        return None
    excluded = set(exclude_task_ids or ())
    candidate = None
    for c in candidates or []:
        if (
            c.get("lane") == "opencode"
            and not c.get("session_id")
            and c.get("stage") in ("handed_off", "launch_failed")
            and c.get("id") not in excluded
        ):
            candidate = c
            break
    if candidate is None:
        return None
    task_id = candidate["id"]
    if not _claim_for_handback(candidate):
        return {
            "task_id": task_id,
            "parked": False,
            "noted": False,
            "reason": "handback_claim_failed",
        }
    if not _park_handback(task_id):
        return {
            "task_id": task_id,
            "parked": False,
            "noted": False,
            "reason": "handback_park_failed",
        }
    noted = _post_handback_agent_note(task_id, candidate)
    return {
        "task_id": task_id,
        "parked": True,
        "noted": noted,
        "reason": HANDBACK_REASON_CLASS,
    }


def launch_step(
    mode: str,
    cap_room: int,
    exclude_task_ids=None,
    fire_start_candidates=None,
) -> List[Dict]:
    """
    Step 4: Launch - create sessions and bind tasks.

    Returns (launched, skips): the successfully launched sessions and
    a skip record for every candidate that was not launched, so the
    fire log can state why a zero-launch fire launched nothing
    (#6865).
    """
    global _fire_provider_wall
    if mode != "live":
        # Shadow fires observe only (#7366): report the candidates
        # that WOULD launch, bounded by cap room, and mutate nothing —
        # no session created, nothing bound, nothing prompted.
        try:
            if fire_start_candidates is not None:
                snapshot = fire_start_candidates
            else:
                snapshot = run_poll().get("candidates", [])
            candidates = [
                c
                for c in snapshot
                if c.get("lane") == "opencode"
                and not c.get("session_id")
                and c.get("stage") in ("handed_off", "launch_failed")
            ]
        except Exception as e:
            print(f"Shadow launch scan failed: {e}")
            return [], []
        skips = []
        room = cap_room
        for candidate in candidates:
            if room > 0:
                skips.append(
                    {"task_id": candidate["id"], "reason": "shadow_would_launch"}
                )
                room -= 1
            else:
                skips.append(
                    {"task_id": candidate["id"], "reason": "shadow_beyond_cap"}
                )
        return [], skips

    if cap_room <= 0:
        return [], []

    launched = []
    skips = []
    _fire_provider_wall = None
    provider_wall: "Dict | None" = None

    excluded = set(exclude_task_ids or ())
    try:
        if fire_start_candidates is not None:
            candidates = fire_start_candidates
        else:
            poll_data = run_poll()
            candidates = poll_data.get("candidates", [])

        # Sort by queue priority (order in QUEUES list)
        # Filter for unbound opencode-lane tasks with correct stage.
        # The candidate SET is the fire-start snapshot (#8038 item
        # 7): launch_step no longer re-polls to discover a different
        # set than the fire started with (per-candidate step-4a
        # re-verification below still re-reads state). A task
        # recovered during this fire's supervision is excluded by
        # id - it is eligible on the NEXT fire, not this one.
        unbound_opencode = []
        for c in candidates:
            if not (
                c.get("lane") == "opencode"
                and not c.get("session_id")
                and c.get("stage") in ("handed_off", "launch_failed")
            ):
                continue
            if c.get("id") in excluded:
                skips.append(
                    {
                        "task_id": c["id"],
                        "reason": "recovered_this_fire_next_fire_only",
                    }
                )
                continue
            unbound_opencode.append(c)

        # Parked handoff sink (#8097 Rule 1): collect this fire's
        # sink candidates' claims up front, grouped by queue,
        # without naming any task id — one broker-picked handoff
        # claim per discovered sink candidate. A sink candidate
        # whose id is not in `collected` when its turn comes was
        # not collected (its pick lost a race or the broker had
        # nothing claimable) and is skipped below — the binding is
        # never improvised.
        handed_off_by_queue: Dict[str, int] = {}
        for c in unbound_opencode:
            if c.get("status") == "handed_off":
                handed_off_by_queue[c["queue"]] = (
                    handed_off_by_queue.get(c["queue"], 0) + 1
                )
        collected: Dict[int, Dict] = {}
        for queue_name, want in handed_off_by_queue.items():
            collected.update(_collect_handed_offs(queue_name, want))

        for candidate in unbound_opencode:
            if provider_wall is not None:
                # #8166: the provider wall is route-wide - no later
                # launch this fire can succeed, and each attempt
                # would burn shared provider rpm against the wall.
                skips.append({"task_id": candidate["id"], "reason": "provider_wall"})
                continue

            if cap_room <= 0:
                skips.append({"task_id": candidate["id"], "reason": "cap_saturated"})
                continue

            task_id = candidate["id"]
            worker_id = candidate.get("worker_id", "unknown")
            issue = candidate.get("issue", "")
            packet = candidate.get("packet", {})

            # Handoff sink: the candidate was handed off by its
            # claiming lane and rests in the sink (status
            # 'handed_off'), so no live claim existed to bind
            # against until the collection above claimed it (as
            # the claimant; the launched session then works under
            # the claimant's worker identity). A candidate the
            # collection did not claim is skipped — the binding
            # is never improvised.
            if candidate.get("status") == "handed_off":
                if task_id not in collected:
                    skips.append({"task_id": task_id, "reason": "sink_claim_failed"})
                    continue
                candidate["status"] = "claimed"
                candidate["worker_id"] = CLAIMANT_WORKER
                worker_id = CLAIMANT_WORKER

            # Step 4a: Re-verify task state
            try:
                poll_data = run_poll()
                task = next(
                    (
                        t
                        for t in poll_data.get("candidates", [])
                        if t.get("id") == task_id
                    ),
                    None,
                )

                if not task or task.get("lane") != "opencode" or task.get("session_id"):
                    skips.append({"task_id": task_id, "reason": "task_state_changed"})
                    continue
            except Exception as e:
                skips.append(
                    {"task_id": task_id, "reason": f"verification_failed: {str(e)}"}
                )
                continue

            # Step 4a2: Held-label gate (#7670). A candidate whose
            # issue carries a held label (held, user/owned,
            # do-not-merge, do-not-auto-edit) must never be launched:
            # the queue-side hard stop is label-based, and the
            # claimant is the last line of defense before a session
            # exists. The check fails closed: an unreadable label set
            # is a skip, never a launch. Candidates without a
            # repo/issue reference have nothing to check.
            _packet = packet if isinstance(packet, dict) else {}
            _repo = _packet.get("repo") or ""
            _issue_no = candidate.get("issue_number")
            if _issue_no is None and isinstance(_packet.get("issue"), dict):
                _issue_no = _packet["issue"].get("number")
            if _repo and _issue_no:
                try:
                    _labels = _github_issue_labels(
                        str(_repo), int(str(_issue_no).lstrip("#"))
                    )
                except Exception as e:
                    skips.append(
                        {
                            "task_id": task_id,
                            "reason": f"held_label_check_failed: {str(e)}",
                        }
                    )
                    continue
                _held = sorted(_labels & HELD_ISSUE_LABELS)
                if _held:
                    skips.append(
                        {
                            "task_id": task_id,
                            "reason": f"held_issue_labels: {','.join(_held)}",
                        }
                    )
                    continue

            # Step 4b: Create worker session
            session_create_body = {
                "location": {"directory": str(PRIMARY_CLONE)},
                "title": f"Queue task {task_id} - {issue}",
                "model": {"id": LAUNCH_MODEL_ID, "providerID": LAUNCH_MODEL_PROVIDER},
                "agent": "build",
            }

            try:
                # Create session through the verified transport,
                # with the server's Basic auth - unauthenticated
                # calls are rejected (#7367).
                status, create_payload = _server_request(
                    "POST", "/api/session", body=session_create_body, timeout=10
                )
                create_wall = _provider_wall_error(create_payload)
                if create_wall is not None:
                    # #8166: the route is walled. This candidate was
                    # never bound or launched, so no attempt is
                    # consumed; the gate closes for the rest of the
                    # fire and the wall is recorded verbatim.
                    provider_wall = _provider_wall_record(create_wall)
                    _fire_provider_wall = provider_wall
                    skips.append({"task_id": task_id, "reason": "provider_wall"})
                    continue
                if not _is_2xx(status):
                    raise RuntimeError(
                        f"Session creation failed: http status "
                        f"{status if status is not None else 'unknown'}"
                        + (" (unauthorized)" if status == 401 else "")
                    )

                session = _unwrap_envelope(create_payload)
                session_id = session.get("id") if isinstance(session, dict) else None
                if not session_id:
                    raise RuntimeError(
                        f"Session creation returned no id: {str(create_payload)[:200]}"
                    )

                # Verify location before proceeding
                if session.get("location", {}).get("directory") != str(PRIMARY_CLONE):
                    # Wrong location - delete and skip
                    _delete_session(session_id)
                    skips.append(
                        {"task_id": task_id, "reason": "session_wrong_location"}
                    )
                    continue

                # Step 4c: Bind the session (atomic operation)
                bind_result = subprocess.run(
                    [
                        str(REPO_QUEUE_CMD),
                        "bind",
                        str(task_id),
                        "--worker",
                        "opencode-claimant",
                        "--session-id",
                        session_id,
                        "--launched-by",
                        "opencode-claimant",
                    ],
                    capture_output=True,
                    text=True,
                    timeout=10,
                )

                if bind_result.returncode != 0:
                    # Bind failed - delete the session and skip. The
                    # delete is not optional: bind refused, so this
                    # session is bound to nothing, and leaving it on
                    # the server orphans a created-but-unowned root
                    # (and leaks a cap slot until it ages out). #8042
                    # added the error text below; it must not drop
                    # this cleanup (a rewrite of this branch removed
                    # it once already).
                    _delete_session(session_id)
                    # Record WHY (#8042): bind's stderr names the
                    # exact failed precondition (lease lapsed / ledger
                    # row missing / already bound / task gone), and a
                    # repeat bind loop is undiagnosable without it.
                    # First non-empty line only, truncated — the bind
                    # verb's precondition texts carry no credentials.
                    bind_error = ""
                    for _line in (bind_result.stderr or "").splitlines():
                        if _line.strip():
                            bind_error = _line.strip()[:200]
                            break
                    if not bind_error:
                        for _line in (bind_result.stdout or "").splitlines():
                            if _line.strip():
                                bind_error = _line.strip()[:200]
                                break
                    skips.append(
                        {
                            "task_id": task_id,
                            "reason": "bind_failed",
                            "error": bind_error,
                        }
                    )
                    continue

                # Step 4d: Send the execution prompt to the bound
                # session (POST /api/session/{id}/prompt — the shape
                # pm-dispatch's OpenCode client uses). A bound session
                # that never receives its prompt is #4758's
                # zero-message case (#7356), so the send is verified,
                # never assumed: _send_prompt reconciles against the
                # session's messages before any resend. The binding
                # stands even if the prompt cannot be landed —
                # supervision owns a bound-but-unprompted session,
                # and unbinding here could double-launch over a
                # worker that is in fact executing.
                prompt = f"""
Execute queue task {task_id}
Issue: {issue}
Packet: {json.dumps(_prompt_safe_packet(packet), indent=2)}

You are a build agent working on this opencode-lane task.

QUEUE RUNBOOK (step 4d, inlined): this task is ALREADY claimed and
lane-bound to your session - do not claim another task and do not
re-claim this one. Work in checkpoints: checkpoint on claim
(stage claimed), at verified / reproduced, branch_created,
implementation_complete, targeted_tests_passed or
targeted_tests_failed, pr_opened, awaiting_ci, merge_ready, and
bookkeeping_complete, each storing the artifact reference and the
next action. Create a dedicated worktree for the task on this
machine (branch queue/<task-id>-<issue-number>-<short-slug>); never
work in a primary checkout. Poll CI for at most five minutes while
actively progressing - if CI is still pending, park the task as a
CI wait instead of holding the claim. Never just stop: if you
cannot finish, park with a reason and a resume condition, or block
with a reason class.

Use --worker {worker_id} for EVERY queue CLI operation (the queue
CLI on this machine is repo-queue, ~/.local/bin/repo-queue).
Lease-heartbeat cadence: heartbeat at least every 10 minutes while
working and at every checkpoint - heartbeat --extend-seconds 900 -
so the claim lease never lapses under a live session; a lease that
expires while you work hands the task to supervision/recovery.
Build discipline: run heavy cargo builds/tests through ~/tools/cargo-target-broker exec --repo <repo short name> --profile <test|dev|release> -- cargo ... so they use a leased pooled target dir (code#6797); never use a per-worktree CARGO_TARGET_DIR and never blank RUSTC_WRAPPER. ~/tools/fleet-build-lock is an accepted equivalent wrapper.
Finish with a queue terminal verb (complete, park, block, or fail) carrying --worker {worker_id} and --execution '{{"lane": "opencode", "session_id": "{session_id}"}}'.
"""

                entry = {
                    "task_id": task_id,
                    "session_id": session_id,
                    "worker_id": worker_id,
                    "queue": candidate.get("queue"),
                    "timestamp": datetime.now(timezone.utc).isoformat(),
                }
                _take_prompt_wall()
                if not _send_prompt(session_id, prompt):
                    entry["prompt_failed"] = True
                prompt_wall = _take_prompt_wall()
                if prompt_wall is not None:
                    # #8166 item 3: the bound session's first step
                    # died on the provider wall - the launch never
                    # became work, so it must not stand as a launch
                    # (a bound dead session is how the task used to
                    # pay an attempt for provider weather). Roll it
                    # back like supervision's zero-message recovery:
                    # verified delete, then the recovery bind clears
                    # the binding. The gate closes for the fire.
                    provider_wall = _provider_wall_record(prompt_wall)
                    _fire_provider_wall = provider_wall
                    if _delete_session(session_id):
                        unbind_result = subprocess.run(
                            [
                                str(REPO_QUEUE_CMD),
                                "bind",
                                str(task_id),
                                "--worker",
                                "opencode-claimant",
                                "--session-id",
                                session_id,
                                "--launched-by",
                                LAUNCHED_BY_RECOVERY_PROVIDER_WALL,
                            ],
                            capture_output=True,
                            text=True,
                            timeout=10,
                        )
                        if unbind_result.returncode == 0:
                            skips.append(
                                {"task_id": task_id, "reason": "provider_wall"}
                            )
                            continue
                # Step 4e: the launch agent-note on the issue,
                # naming task id + session id. Its failure flags
                # the entry but never unbinds the session.
                if _post_launch_agent_note(task_id, session_id, candidate, packet):
                    entry["agent_note"] = "posted"
                else:
                    entry["agent_note_failed"] = True
                launched.append(entry)

                cap_room -= 1

            except Exception as e:
                skips.append({"task_id": task_id, "reason": f"launch_error: {str(e)}"})

    except Exception as e:
        print(f"Launch step failed: {e}")

    return launched, skips


def acquire_fire_lock():
    """Take the single-instance fire lock, or return None if held (#8039).

    Two overlapping fires (the scheduled task plus an ad-hoc run) each
    computed cap_room from the same active-root count and could spend
    the same free slots or race one candidate. An exclusive,
    non-blocking flock on STATE_DIR/claimant.lock makes a second fire
    exit at once. The lock is released when the returned file is closed,
    or by the kernel when the process exits, so a crashed fire never
    leaves a stale lock.
    """
    ensure_state_dir()
    handle = (STATE_DIR / "claimant.lock").open("a")
    try:
        fcntl.flock(handle.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        handle.close()
        return None
    return handle


def main():
    """Main entry point for the claimant/executor."""
    mode = read_mode()
    lock = acquire_fire_lock()
    if lock is None:
        print("Another claimant fire is running. Exiting.")
        log_fire(mode, None, [], None, [{"reason": "already_running"}])
        return 0
    try:
        return _fire(mode)
    finally:
        lock.close()


def _fire(mode: str):
    """One claimant fire, run under the single-instance lock."""
    source = _mode_source or {}
    print(
        f"OpenCode claimant starting in {mode} mode "
        f"(mode file {source.get('path', MODE_FILE)}: {source.get('state', 'unknown')}, "
        f"modified {source.get('file_mtime')})"
    )

    # Step 1: Supervision (always runs in live mode). Tasks it
    # recovers are eligible on the NEXT fire, not this one (#8038
    # item 7): their ids are excluded from this fire's launch set.
    recovered_ids = []
    if mode == "live":
        print("Running supervision step...")
        recovered = supervision_step(mode)
        recovered_ids = [c["task_id"] for c in recovered]

    # Babysitter round (#1338): one verdict round over ALL sessions
    # per fire, whatever the launch outcome below - rounds are
    # about sessions, not candidates, so the round runs before the
    # gate and its record rides every fire-log entry from here on.
    # A failed round never breaks dispatch: the record says so.
    babysitter_record = None
    try:
        babysitter_record = _load_babysitter_module().babysitter_step(mode)
    except Exception as e:
        print(f"Warning: babysitter round failed: {e}")
        babysitter_record = {"error": f"babysitter round failed: {e}"}

    # Step 2: Gate (records the fire-start candidate snapshot and
    # the queue-health line in _fire_candidates / _fire_health).
    print("Running gate step...")
    has_candidates = gate_step()
    fire_candidates = _fire_candidates
    fire_health = _fire_health

    if has_candidates is None:
        print("Gate check failed: candidate discovery unknown. Exiting.")
        log_fire(
            mode,
            None,
            [],
            None,
            [{"reason": "gate_unknown"}],
            health=fire_health,
            babysitter=babysitter_record,
        )
        return 1

    if not has_candidates:
        print("No unbound opencode-lane candidates found. Exiting.")
        log_fire(
            mode,
            fire_candidates if fire_candidates is not None else [],
            [],
            None,
            [{"reason": "no_candidates"}],
            health=fire_health,
            babysitter=babysitter_record,
        )
        return 0

    # Step 3: Cap
    print("Running cap step...")
    cap_room = cap_step()

    if cap_room == -1:
        print("Warning: Could not determine cap room. Exiting.")
        fire_opencode_working_hook(
            "lane-launch", "claimant cap step could not read OpenCode sessions"
        )
        log_fire(
            mode,
            fire_candidates,
            [],
            None,
            [{"reason": "cap_unknown"}],
            health=fire_health,
            babysitter=babysitter_record,
        )
        return 1

    if cap_room <= 0:
        print(f"Cap saturated ({CONCURRENCY_CAP} active sessions). Exiting.")
        # #1187 criterion 2: saturation is reported on the task and
        # its issue, never only in this log - hand the next waiting
        # candidate back (live mode only; the step itself refuses
        # every other mode).
        hand_back = hand_back_step(
            mode, fire_candidates, exclude_task_ids=recovered_ids
        )
        log_fire(
            mode,
            fire_candidates,
            [],
            cap_room,
            [{"reason": "cap_saturated"}],
            health=fire_health,
            hand_back=hand_back,
            babysitter=babysitter_record,
        )
        return 0

    print(f"Cap room: {cap_room} sessions available")

    # Step 4: Launch, from the fire-start snapshot, excluding any
    # task supervision recovered during this same fire.
    print("Running launch step...")
    launched, skips = launch_step(
        mode,
        cap_room,
        exclude_task_ids=recovered_ids,
        fire_start_candidates=fire_candidates,
    )

    # Step 5: Terminal states (handled by workers)
    # Step 6: Log fire - candidates_seen is the real fire-start
    # snapshot, never a hardcoded [] (#8038 item 3).
    print(f"Fire complete. Launched {len(launched)} sessions.")

    log_fire(
        mode,
        fire_candidates,
        launched,
        cap_room,
        skips,
        health=fire_health,
        provider_wall=_fire_provider_wall,
        babysitter=babysitter_record,
    )

    return 0


def parse_args(argv):
    """Strictly parse the (empty) declared command line (#3352).

    claimant.py takes no arguments: anything on the command line is
    unaccounted for and argparse exits 2 with a usage error. --help is
    disabled (add_help=False) so it is rejected the same way instead of
    exiting 0.
    """
    parser = argparse.ArgumentParser(
        prog="claimant.py",
        add_help=False,
        allow_abbrev=False,
        description="OpenCode lane claimant/executor. Takes no arguments.",
    )
    return parser.parse_args(argv)


if __name__ == "__main__":
    # Strict parse first: a usage error (exit 2) must precede every
    # file read, subprocess, network, claim, and launch step in main().
    parse_args(sys.argv[1:])
    sys.exit(main())
