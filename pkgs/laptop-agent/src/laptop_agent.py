#!/usr/bin/env python3
"""laptop-agent daemon — resident execution agent on Drew's laptop.

Design: developing-today/code#5343. Single file, Python stdlib only.
Binds the tailnet interface only; every request needs the X-Agent-Token
header (token in ~/.config/laptop-agent/token, mode 600). Startup fails
closed when that token is missing, empty, or whitespace-only.

API (JSON):
  GET  /health                    -> {ok, uptime_s, jobs, load1}
  POST /exec   {cmds:[{cwd, argv|shell}], timeout_s?, grace_s?}
                                  -> {results:[{exit, stdout, stderr,
                                      duration_ms, truncated}]}
                                      (grace_s 1-600 with one cmd:
                                      the trip id is the job id, and
                                      a dropped client can attach
                                      with GET /jobs/<trip> within
                                      grace_s, #15420)
  POST /jobs   {cwd, argv|shell, env?} -> {job_id}
  GET  /jobs                      -> {jobs:[{job_id, state, exit, ...}]}
  GET  /jobs/<id>?out_off&err_off -> {state, exit, stdout, stderr,
                                      out_next, err_next, out_size,
                                      err_size, duration_ms}
                                      (out_size/err_size: the spool
                                      sizes, so a client pages a done
                                      job to the end, #15420;
                                      out_dropped/err_dropped/
                                      bytes_dropped: bytes past the
                                      spool cap, see below)
  POST /jobs/<id>/kill            -> {ok, state}
  POST /exec/<trip>/cancel        -> {ok, cancelled} (POST /cancel
                                      {trip} is the legacy spelling)
  GET  /file?path=&off=&len=      -> raw bytes (code#5757): the byte
                                      range [off, off+len) of the file,
                                      len capped at 1 MiB; response
                                      headers X-File-Size (total size)
                                      and X-File-Off echo the range.
                                      Auth is the token header only,
                                      exactly like every other route.

Job stdout/stderr spool to ~/.local/state/laptop-agent/jobs/<id>/ so a
daemon restart does not lose results: job metadata is persisted next to
the output and the index is rebuilt on startup. Finished jobs reaped
after 24h, spool directory included.

Spool cap (#15420, design #10356 section 3.3): each stream's spool
stops appending at LAPTOP_AGENT_SPOOL_CAP bytes (default 256 MiB).
The process keeps running and is never killed because its log grew;
bytes past the cap are counted, not stored, and every reply carries
the counts so `fleet http attach` prints an explicit
"[spool cap reached: N bytes dropped]" marker. A small pump process
per job (its own session, like the job) reads the job's stdout and
stderr pipes and writes the spool, so a daemon restart neither kills
the job nor stops its spool, exactly as when the job wrote the files
directly. The counts live next to the spool as stdout.dropped and
stderr.dropped.

Exec grace window (#15420, design #10356 section 3.4): an /exec
with grace_s > 0 runs its one command as a spooled job named by the
client's trip id. A client that drops leaves the command in state
"grace" for grace_s seconds; GET /jobs/<trip> in that window turns
it into an ordinary job (`fleet http attach <trip>`), and otherwise
it is reaped TERM then KILL and stays listed with reaped=true and
its spool intact. grace_s 0, the default, is the immediate reap
described next.

Timeout semantics (2026-11-10, live lineage): every /exec command runs
in its OWN process group (start_new_session) and the whole group is
killed on timeout, on an explicit /cancel for its trip, or when the
client connection dies mid-exec — a caller-side timeout therefore
never leaves orphan processes behind on the laptop, grandchildren
included. Trip logging v2 (code#5702) appends one record per exec/job
to trips.jsonl in the state directory.

Reconciliation (code#10131): this file is the union of the two daemon
lineages — the live daemon (timeout semantics, trip logging, /exec
cancel, disconnect reap, /file) and the vendored darwin-nix package
lineage (persisted job index, fail-closed token + startup-int
validation, request-framing errors as 400s, job summaries, spool
reaping, bytes-space auth). It is vendored into developing-today/code
at pkgs/laptop-agent/src/ as the single daemon source; the fleet
round-trip parity for both lineages' surfaces is the acceptance
evidence on #10131.
"""

import base64
import binascii
import hashlib
import hmac
import json
import math
import os
import errno
import secrets
import shutil
import signal
import socket
import ssl
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlparse, parse_qs


def _parse_startup_int(name, raw, requirement, minimum, maximum=None):
    """Validate an integer startup env var, or exit 3 naming it (#1510).

    A bare int() at import crashed with a raw ValueError traceback for
    typos, and out-of-range values either tracebacked later at bind
    (PORT=99999), silently bound an ephemeral port (PORT=0), or started
    a healthy-looking daemon that could never run work (SLOTS=0).
    Fail at startup instead, in the read_token house style.
    """
    try:
        value = int(raw)
    except (TypeError, ValueError):
        value = None
    if value is None or value < minimum or (maximum is not None and value > maximum):
        print(
            f"laptop-agent: invalid {name}={raw!r}: {requirement}",
            file=sys.stderr,
            flush=True,
        )
        sys.exit(3)
    return value


BIND_HOST = os.environ.get("LAPTOP_AGENT_HOST", "100.99.113.103")
BIND_PORT = _parse_startup_int(
    "LAPTOP_AGENT_PORT",
    os.environ.get("LAPTOP_AGENT_PORT", "4799"),
    "must be an integer 1-65535",
    1,
    65535,
)


def _tls_context_from_env():
    """Broker TLS Phase 1 for the daemon endpoint (BT-4, #14402).

    Design of record: doc/2026-11-24T13-59-26Z_design_broker-tls
    §§4-6, mirroring the queue broker's BT-3. Additive and default
    off (rule M1): with neither variable set the daemon serves
    plaintext exactly as before. Both set: the endpoint presents the
    `broker-server` leaf chain (leaf first, then issuers) and its
    on-host key. Exactly one set, an unreadable file or an unusable
    pair exits 3 naming the variable, so a half-configured daemon
    never starts and silently serves plaintext. Token auth is
    unchanged: the certificate is identity, the token is still the
    authorisation.
    """
    cert = (os.environ.get("LAPTOP_AGENT_TLS_CERT_FILE") or "").strip()
    key = (os.environ.get("LAPTOP_AGENT_TLS_KEY_FILE") or "").strip()
    if not cert and not key:
        return None
    if not cert or not key:
        have, missing = (
            ("LAPTOP_AGENT_TLS_CERT_FILE", "LAPTOP_AGENT_TLS_KEY_FILE")
            if cert
            else ("LAPTOP_AGENT_TLS_KEY_FILE", "LAPTOP_AGENT_TLS_CERT_FILE")
        )
        print(
            f"laptop-agent: {have} is set but {missing} is not; "
            "endpoint TLS needs the leaf chain and its on-host key together",
            file=sys.stderr,
            flush=True,
        )
        sys.exit(3)
    ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    ctx.minimum_version = ssl.TLSVersion.TLSv1_2
    try:
        ctx.load_cert_chain(certfile=cert, keyfile=key)
    except (OSError, ssl.SSLError) as e:
        print(
            "laptop-agent: endpoint TLS config "
            f"(LAPTOP_AGENT_TLS_CERT_FILE={cert!r}, "
            f"LAPTOP_AGENT_TLS_KEY_FILE={key!r}) is unusable: {e}",
            file=sys.stderr,
            flush=True,
        )
        sys.exit(3)
    return ctx


TLS_CONTEXT = _tls_context_from_env()
# A TLS handshake must finish within this many seconds of the first
# byte, so a stalled peer cannot hold a handler thread.
TLS_HANDSHAKE_TIMEOUT = 10.0
# A TLS ClientHello opens with a handshake record: content type 0x16.
_TLS_HANDSHAKE_RECORD = 0x16

# Absolute interpreter for `shell` requests: NixOS has no /bin/bash, and
# the package wrapper sets this to the store path. The live lineage's
# resolution chain is the fallback when the wrapper did not set it
# (the 2026-11-04 laptop-ssh retirement verification caught shell mode
# failing with ENOENT for a hardcoded /bin/bash).
BASH = os.environ.get("LAPTOP_AGENT_BASH") or next(
    (
        p
        for p in (
            shutil.which("bash"),
            "/etc/profiles/per-user/user/bin/bash",
            "/run/current-system/sw/bin/bash",
            "/bin/bash",
        )
        if p and os.path.exists(p)
    ),
    "/bin/bash",
)
TOKEN_FILE = Path(
    os.environ.get(
        "LAPTOP_AGENT_TOKEN_FILE", Path.home() / ".config/laptop-agent/token"
    )
)
STATE_DIR = Path(
    os.environ.get("LAPTOP_AGENT_STATE", Path.home() / ".local/state/laptop-agent")
)
MAX_OUT = 1 << 20  # per-stream capture cap for /exec (tail kept)
MAX_BODY = 1 << 20  # request body cap for /exec and /jobs
FILE_CHUNK_MAX = 1 << 20  # max bytes served per GET /file request
# #5759 transfer routes (/files/*): the fleet-proxy client ships whole
# files in 2 MiB chunks, so a single /files/write body is much larger
# than the /exec MAX_BODY cap. Base64 inflates by 4/3, so the route
# body cap is the chunk size times 4/3 plus JSON envelope headroom.
FILES_CHUNK_SIZE = 2 << 20  # 2 MiB, matches fleet-proxy CHUNK_SIZE
FILES_BODY_MAX = (FILES_CHUNK_SIZE * 4) // 3 + (1 << 16)
FILES_TEMP_MARK = ".fleet-part-"  # ".{name}.fleet-part-{sha256[..12]}"
# Spool cap per stream for POST /jobs (#15420, design #10356 §3.3).
SPOOL_CAP = _parse_startup_int(
    "LAPTOP_AGENT_SPOOL_CAP",
    os.environ.get("LAPTOP_AGENT_SPOOL_CAP", str(256 << 20)),
    "must be an integer >= 1 (bytes per stream)",
    1,
)
# How long a finished job waits for its pump to drain the pipes before
# it reads as done. A grandchild that keeps the pipe open (a daemonised
# helper) must not hold the job in "running" forever.
PUMP_SETTLE_S = 2
EXEC_TIMEOUT_DEFAULT = 120
EXEC_TIMEOUT_MAX = 600
# Bound on one server-side long-poll wait for GET /jobs/<job>
# (developing-today/code#5757): the request is held at most this long,
# returning sooner on output or completion. A quiet ~60 s job then costs
# 2 trips (50 s + remainder), >=5x fewer than the 10 pre-long-poll
# backoff trips, while completion is still reported within about a
# second. Must match LONG_POLL_WAIT_S in
# pkgs/fleet/crates/fleet-laptop-agent/src/lib.rs.
JOBS_WAIT_MAX_S = 50
# Exec grace window (#15420, design #10356 section 3.4): with
# grace_s > 0 a dropped /exec client leaves its command running for
# grace_s seconds, during which GET /jobs/<trip> attaches to it as an
# ordinary job; after that it is reaped. 0 (the default, and what
# every older client sends by omission) is the immediate reap.
EXEC_GRACE_MAX = 600
JOB_TTL_S = 24 * 3600
REAP_INTERVAL_S = 3600  # background reaper cadence (#1504)
KILL_GRACE_S = 5
# Slot semantics (#1545): LAPTOP_AGENT_SLOTS is ONE shared pool for all
# work the daemon runs, not a per-route cap. POST /exec acquires a slot
# for the duration of the call; POST /jobs acquires a slot before
# spawning and holds it for the job's whole lifetime (the watch thread
# releases it when the process exits, including after a kill). Both
# routes wait up to 5s for a slot and then return 503 {"error": "busy"}:
# submissions are never queued indefinitely and never spawn past the cap.
SEMA = threading.Semaphore(
    _parse_startup_int(
        "LAPTOP_AGENT_SLOTS",
        os.environ.get("LAPTOP_AGENT_SLOTS", "32"),
        "must be an integer >= 1",
        1,
    )
)
STARTED = time.time()

JOBS = {}  # job_id -> dict
JOBS_LOCK = threading.Lock()
TRIPS_FILE = STATE_DIR / "trips.jsonl"
# Context propagation, build 1/5 (#14186, decision record for #10357
# mechanism 1): jobs record the caller's FLEET_CTX_* environment (ids
# and hashes only, never secrets) in the spool's meta.json so a queue
# task's id is visible in the laptop job spool. The exact FLEET_CTX_*
# variable list is owned by build 2/5 (#14188); the daemon carries any
# FLEET_CTX_-prefixed string entry without enumerating names.
FLEET_CTX_PREFIX = "FLEET_CTX_"
EXEC_PROCS = {}  # trip -> set[Popen] currently running for /exec
EXEC_PROCS_LOCK = threading.Lock()
# #5759: pending chmod modes for in-flight transfers, keyed by temp path.
# A mode rides from /files/begin to /files/commit so it is applied to
# the temp immediately before the atomic rename publishes it.
FILES_PENDING_MODES = {}
FILES_PENDING_MODES_LOCK = threading.Lock()


def read_token(path: Path) -> str:
    """Return the agent token, or exit non-zero before binding a listener.

    An empty file previously left TOKEN == "" so a request without the
    header (also "") authenticated successfully (codex P1 on PR #1459).
    """
    try:
        token = path.read_text().strip()
    except OSError as e:
        print(
            f"laptop-agent: cannot read token file {path}: {e}",
            file=sys.stderr,
            flush=True,
        )
        sys.exit(3)
    if not token:
        print(
            f"laptop-agent: token file {path} is empty; refusing to start",
            file=sys.stderr,
            flush=True,
        )
        sys.exit(3)
    return token


TOKEN = read_token(TOKEN_FILE)


def tail_bytes(data: bytes, cap: int):
    if len(data) <= cap:
        return data, False
    return data[-cap:], True


def _extract_ctx(env):
    """Return the FLEET_CTX_* subset of an env mapping (#14186).

    Only string values ride; non-strings are dropped (absence is
    representable — a partial record, not an error). Callers must put
    ids and hashes only in FLEET_CTX_*; the context is logged widely
    by design (#10357 constraints).
    """
    if not isinstance(env, dict):
        return {}
    return {
        k: v
        for k, v in env.items()
        if isinstance(k, str) and k.startswith(FLEET_CTX_PREFIX) and isinstance(v, str)
    }


def tail_file(path: Path, cap: int = MAX_OUT):
    """Return the last `cap` bytes of a spool file and whether it was cut.

    Never reads the whole file into memory, so a noisy command cannot
    exhaust the daemon.
    """
    try:
        with open(path, "rb") as f:
            f.seek(0, os.SEEK_END)
            size = f.tell()
            start = max(0, size - cap)
            f.seek(start)
            data = f.read(cap)
        return data, start > 0
    except OSError:
        return b"", False


def files_temp_path(path: str, sha256: str) -> str:
    """Deterministic transfer temp for (path, sha256) (#5759).

    Mirrors the shipped client's `temp_path_for`: a sibling named
    `.<name>.fleet-part-<sha256[..12]>`. An interrupted transfer
    resumes by re-deriving this name, and the destination path never
    shows partial content because commit renames the temp over it.
    """
    i = path.rfind("/")
    if i >= 0:
        directory, name = path[: i + 1], path[i + 1 :]
    else:
        directory, name = "", path
    return f"{directory}.{name}{FILES_TEMP_MARK}{sha256[:12]}"


def _files_validate(path):
    """Bound a remote path the way the client and design do (#5759).

    Non-empty, no NUL; no confinement (the daemon already executes
    arbitrary commands as the same user, so file routes are not a
    privilege change). Returns an error string, or None when valid.
    """
    if not isinstance(path, str) or not path:
        return "path required"
    if "\0" in path:
        return "path contains a NUL byte"
    return None


def _files_mode_ok(mode):
    """A chmod mode must be a plain int in 0..=0o7777."""
    if mode is None:
        return True
    if not isinstance(mode, int) or isinstance(mode, bool):
        return False
    return 0 <= mode <= 0o7777


def _files_sha_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def _fsync_dir(path: Path):
    """Durably record a rename by fsyncing the containing directory.

    Best-effort: a platform or filesystem that refuses a directory
    fsync must not fail an otherwise correct commit.
    """
    try:
        fd = os.open(path if str(path) else ".", os.O_RDONLY)
    except OSError:
        return
    try:
        os.fsync(fd)
    except OSError:
        pass
    finally:
        os.close(fd)


def _kill_group(pid: int, sig: int):
    try:
        os.killpg(pid, sig)
    except OSError:
        pass


def _group_alive(pid: int) -> bool:
    try:
        os.killpg(pid, 0)
        return True
    except OSError:
        return False


def log_trip(
    trip,
    kind,
    cmd_str,
    duration_ms,
    exit_code,
    out_bytes,
    err_bytes,
    t_received=None,
    ctx_id=None,
):
    """Append one trip record (single O_APPEND write) to trips.jsonl.

    Safety: only the command's first token and the sha256 of the full
    command string are recorded — never raw command text, environment,
    headers, or token values. Perf/safety: logging must never fail or
    visibly delay the request, so every error is swallowed.

    Trip logging v2 (code#5702): t_received is the millisecond epoch
    stamped at request entry in do_POST; with the client's t_sent /
    t_returned it splits the trip into outbound / daemon / return
    legs. v1 fields are kept verbatim for v1 readers.

    #14188: ctx_id is the command's FLEET_CTX_ID (the context join key,
    an id, never a secret), recorded so the trip joins the client's
    trip line and the brokers' per-trip context lines."""
    try:
        parts = (cmd_str or "").split()
        rec = {
            "v": 2,
            "ts": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
            "trip": trip,
            "kind": kind,
            "cmd0": parts[0] if parts else "",
            "cmd_sha256": hashlib.sha256(
                (cmd_str or "").encode("utf-8", "replace")
            ).hexdigest(),
            "duration_ms": duration_ms,
            "exit": exit_code,
            "out_bytes": out_bytes,
            "err_bytes": err_bytes,
        }
        if t_received is not None:
            rec["t_received"] = t_received
        if isinstance(ctx_id, str) and ctx_id:
            rec["ctx_id"] = ctx_id
        fd = os.open(TRIPS_FILE, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o644)
        try:
            os.write(fd, (json.dumps(rec) + "\n").encode())
        finally:
            os.close(fd)
    except Exception:
        pass


def kill_group(proc, grace_s=2.0):
    """TERM a process group, escalate to KILL after grace_s. The exec
    children run with start_new_session=True, so proc.pid is the pgid
    and this reaches grandchildren that a plain proc.kill() misses."""
    try:
        os.killpg(proc.pid, signal.SIGTERM)
    except OSError:
        pass
    deadline = time.time() + grace_s
    while time.time() < deadline:
        if proc.poll() is not None:
            break
        time.sleep(0.05)
    if proc.poll() is None:
        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except OSError:
            try:
                proc.kill()
            except OSError:
                pass
    try:
        proc.wait(timeout=5)
    except Exception:
        pass


def _drain(stream, buf):
    try:
        while True:
            chunk = stream.read(65536)
            if not chunk:
                break
            buf.append(chunk)
    except (OSError, ValueError):
        pass


def run_one(spec, timeout_s, trip=None, cancel_event=None):
    """Run one command in its own process group, capturing output.

    The group dies when ANY of these fires: timeout_s expires, the
    cancel_event is set (explicit /cancel or the client connection
    died — see Handler._client_gone), or the watchdog notices.
    Bytes are drained incrementally so a killed grandchild holding
    the pipes open can never wedge the reader (the classic
    subprocess.run+timeout hang)."""
    cwd = spec.get("cwd") or str(Path.home())
    env = dict(os.environ)
    env.update(spec.get("env") or {})
    if spec.get("shell") is not None:
        argv = [BASH, "-lc", spec["shell"]]
        cmd_str = spec["shell"]
    else:
        argv = list(spec["argv"])
        cmd_str = " ".join(argv)
    t0 = time.time()
    try:
        proc = subprocess.Popen(
            argv,
            cwd=cwd,
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            start_new_session=True,
        )
    except Exception as e:  # cwd missing, argv[0] missing, ...
        return {
            "exit": None,
            "error": f"{type(e).__name__}: {e}",
            "stdout": "",
            "stderr": "",
            "duration_ms": int((time.time() - t0) * 1000),
            "truncated": False,
            "_cmd": cmd_str,
            "_out_bytes": 0,
            "_err_bytes": 0,
        }
    if trip:
        with EXEC_PROCS_LOCK:
            EXEC_PROCS.setdefault(trip, set()).add(proc)
    out_parts, err_parts = [], []
    t_out = threading.Thread(target=_drain, args=(proc.stdout, out_parts), daemon=True)
    t_err = threading.Thread(target=_drain, args=(proc.stderr, err_parts), daemon=True)
    t_out.start()
    t_err.start()
    timed_out = False
    cancelled = False
    deadline = time.time() + timeout_s
    try:
        while True:
            if proc.poll() is not None:
                break
            if cancel_event is not None and cancel_event.is_set():
                cancelled = True
                kill_group(proc)
                break
            if time.time() >= deadline:
                timed_out = True
                kill_group(proc)
                break
            time.sleep(0.05)
    finally:
        if trip:
            with EXEC_PROCS_LOCK:
                s = EXEC_PROCS.get(trip)
                if s is not None:
                    s.discard(proc)
                    if not s:
                        EXEC_PROCS.pop(trip, None)
    # Collect whatever the pipes still hold, but never wait forever:
    # a detached grandchild can keep a pipe fd open after the group
    # leader dies; the capture cap makes a short grace sufficient.
    t_out.join(timeout=2)
    t_err.join(timeout=2)
    out = b"".join(out_parts)
    err = b"".join(err_parts)
    out_t, out_tr = tail_bytes(out, MAX_OUT)
    err_t, err_tr = tail_bytes(err, MAX_OUT)
    result = {
        "exit": proc.returncode,
        "stdout": out_t.decode("utf-8", "replace"),
        "stderr": err_t.decode("utf-8", "replace"),
        "duration_ms": int((time.time() - t0) * 1000),
        "truncated": out_tr or err_tr,
        "_cmd": cmd_str,
        "_out_bytes": len(out),
        "_err_bytes": len(err),
    }
    if timed_out:
        result["exit"] = None
        result["timeout"] = True
    if cancelled:
        result["exit"] = None
        result["timeout"] = True
        result["cancelled"] = True
    return result


def _jobs_root():
    state_root = STATE_DIR.resolve()
    jobs_root = (STATE_DIR / "jobs").resolve()
    if jobs_root.parent != state_root:
        raise ValueError("laptop-agent jobs directory escapes state directory")
    return jobs_root


def job_dir(job_id):
    if (
        not isinstance(job_id, str)
        or not job_id
        or job_id in (".", "..")
        or "/" in job_id
        or "\\" in job_id
    ):
        raise ValueError("invalid laptop-agent job id")
    base = _jobs_root()
    d = base / job_id
    if d.is_symlink():
        raise ValueError("laptop-agent job directory is a symlink")
    try:
        resolved = d.resolve()
    except (OSError, RuntimeError) as e:
        raise ValueError("cannot safely resolve laptop-agent job directory") from e
    if resolved.parent != base:
        raise ValueError("laptop-agent job directory escapes jobs directory")
    return resolved


# The per-job spool pump (#15420). Run as `python3 -I -c SRC cap dir
# out_fd err_fd` with the two pipe read ends passed through pass_fds.
# It appends each stream to <dir>/stdout and <dir>/stderr up to cap
# bytes, counts what it drops past the cap (or past a failed write,
# such as a full disk) in <dir>/<stream>.dropped, and exits when both
# pipes reach EOF. It never stops reading, so the job never blocks on
# a full pipe and is never killed because its output grew. Stdlib
# only, and self-contained: it does not import this module, whose
# import reads the token and binds nothing but does validate env.
SPOOL_PUMP_SRC = r"""
import os, selectors, sys, time
cap = int(sys.argv[1])
d = sys.argv[2]
sel = selectors.DefaultSelector()
streams = {}
for raw, name in ((sys.argv[3], "stdout"), (sys.argv[4], "stderr")):
    fd = int(raw)
    f = open(os.path.join(d, name), "ab", buffering=0)
    streams[fd] = {
        "f": f,
        "name": name,
        "written": os.fstat(f.fileno()).st_size,
        "dropped": 0,
        "noted": 0,
        "t": 0.0,
    }
    sel.register(fd, selectors.EVENT_READ)


def note(s, force):
    if s["dropped"] == s["noted"]:
        return
    now = time.monotonic()
    if not force and now - s["t"] < 1.0:
        return
    tmp = os.path.join(d, s["name"] + ".dropped.tmp")
    try:
        with open(tmp, "w") as t:
            t.write(str(s["dropped"]))
        os.replace(tmp, os.path.join(d, s["name"] + ".dropped"))
        s["noted"] = s["dropped"]
        s["t"] = now
    except OSError:
        pass


while streams:
    for key, _ in sel.select(timeout=1.0):
        fd = key.fd
        s = streams[fd]
        try:
            chunk = os.read(fd, 65536)
        except InterruptedError:
            continue
        except OSError:
            chunk = b""
        if not chunk:
            sel.unregister(fd)
            os.close(fd)
            note(s, True)
            s["f"].close()
            del streams[fd]
            continue
        before = s["written"]
        part = memoryview(chunk)[: max(0, cap - before)]
        while len(part):
            try:
                n = s["f"].write(part)
            except OSError:
                break
            if not n:
                break
            s["written"] += n
            part = part[n:]
        s["dropped"] += len(chunk) - (s["written"] - before)
        note(s, False)
    for s in streams.values():
        note(s, False)
"""


def _read_dropped(d, name):
    """Bytes the pump dropped past the spool cap for one stream.

    0 when the count file is missing (no drop yet, or a spool from
    before the cap) or unreadable; never negative. A malformed file
    reads as 0 rather than failing the reply that carries it.
    """
    if d is None:
        return 0
    try:
        value = int((d / f"{name}.dropped").read_text().strip())
    except (OSError, ValueError):
        return 0
    return value if value > 0 else 0


def _start_spool_pump(d):
    """Start the spool pump for job directory d (#15420).

    Returns (pump, out_w, err_w): the pump's Popen and the write ends
    of the two pipes, which the caller hands to the job as its stdout
    and stderr and then closes. The read ends are the pump's alone.
    """
    out_r, out_w = os.pipe()
    err_r, err_w = os.pipe()
    try:
        pump = subprocess.Popen(
            [
                sys.executable,
                "-I",
                "-c",
                SPOOL_PUMP_SRC,
                str(SPOOL_CAP),
                str(d),
                str(out_r),
                str(err_r),
            ],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            pass_fds=(out_r, err_r),
            start_new_session=True,
        )
    except BaseException:
        for fd in (out_r, out_w, err_r, err_w):
            try:
                os.close(fd)
            except OSError:
                pass
        raise
    os.close(out_r)
    os.close(err_r)
    return pump, out_w, err_w


def _write_meta(job):
    meta = {
        "id": job["id"],
        "cwd": job["cwd"],
        "cmd": job["cmd_repr"],
        "started_at": job["started_at"],
        "ended_at": job["ended_at"],
        "exit": job["exit"],
        "state": job["state"],
        "interrupted": job.get("interrupted", False),
        "ctx": job.get("ctx") or {},
    }
    try:
        d = job_dir(job["id"])
    except (RuntimeError, ValueError) as e:
        # A spool swapped for a symlink after load (#1575): job_dir()
        # raises ValueError, which must stay contained here exactly as
        # an OSError is -- job_state() calls this from both serializers,
        # so a leak would poison every read of the job. One stderr line
        # naming the job, as in the loader/reaper skip contract.
        print(
            f"laptop-agent: cannot write meta for job {job['id']}: {e}",
            file=sys.stderr,
            flush=True,
        )
        return
    try:
        d.mkdir(parents=True, exist_ok=True)
        tmp = d / "meta.json.tmp"
        tmp.write_text(json.dumps(meta))
        os.replace(tmp, d / "meta.json")
    except OSError as e:
        print(
            f"laptop-agent: cannot write meta for job {job['id']}: {e}",
            file=sys.stderr,
            flush=True,
        )


# Restore contract (design #1578, phase 1 -- issue #1579): the
# writer is the schema. _write_meta() writes exactly the nine
# fields in RESTORE_FIELD_TABLE; load_persisted_jobs() restores a
# persisted entry only through _restore_persisted_job(), driven by
# this table: for each field -- the shape the writer produces, the
# default when the field is missing/null, and the disposition when
# a present value violates the shape. Missing/null takes the named
# default; a present-but-malformed value is never laundered through
# a default -- the whole entry is skipped, one stderr line naming
# the directory and the field/reason, the spool left on disk
# untouched, and healthy siblings still restore (the fleet's
# established skip contract, stated once here instead of being
# restated in every per-field guard block).
#
# The exit/cwd/cmd rows originally recorded the writer shapes
# only, with enforcement owned by issue #1568; #1568 has since
# folded its checks into _restore_persisted_job() as table-driven
# rows below -- not as per-field guard blocks in the load loop --
# exactly as the fold-in rule first stated here required.
RESTORE_FIELD_TABLE = (
    # (field, writer shape, missing/null default, malformed disposition)
    (
        "id",
        "str job id (secrets.token_hex(6))",
        "the spool directory name (also for an empty string)",
        "skip: non-string, or a string that is not the directory "
        "name or carries '.'/'..'/a path separator (#1548/#1554)",
    ),
    (
        "cwd",
        "str working directory",
        "str(Path.home()) (via the restore tail's `or` default)",
        "skip: any non-string present value (#1568, folded in as "
        "a table row per #1579)",
    ),
    (
        "cmd",
        "str command representation",
        "'' (via the restore tail's `or` default)",
        "skip: any non-string present value (#1568, folded in as "
        "a table row per #1579)",
    ),
    (
        "started_at",
        "float seconds since the epoch",
        "0.0",
        "skip: non-numeric, bool, non-finite, or oversized int (#1513/#1523, PR #1534)",
    ),
    (
        "ended_at",
        "float seconds since the epoch, or None",
        "stamped with time.time() at load (the transitions below)",
        "skip: non-numeric, bool, non-finite, or oversized int (#1513/#1523, PR #1534)",
    ),
    (
        "exit",
        "int process returncode, or None while running",
        "None",
        "skip: non-integer or bool present value (#1568, folded "
        "in as a table row per #1579)",
    ),
    ("state", "'running' or 'done'", "'done'", "skip: any other present value (#1556)"),
    ("interrupted", "bool", "False", "skip: any non-boolean present value (#1556)"),
    (
        "ctx",
        "dict of FLEET_CTX_* str->str (ids/hashes only, #14186)",
        "{} (missing/null; pre-contract spools stay valid)",
        "skip: any present non-dict, or any non-string key/value (#14186)",
    ),
)

# The only coercions restore may perform (the table's transitions):
# persisted "running" -> "done" + interrupted=True + exit=None +
# ended_at stamped (nobody can poll a dead process), and "done"
# with a null/missing ended_at -> ended_at stamped at load (#1532),
# so the reaper's TTL and duration_ms behave. Both persist via
# _write_meta (as does a faithfully restored interrupted=True).
# No other value is ever rewritten.


def _bad_persisted_timestamp(value):
    """True when a present timestamp violates its table row.

    Missing/None is valid (the table default applies). A bool is
    malformed (bool is an int subclass, so float(True) == 1.0
    would otherwise slip through -- #1513), as are non-numeric
    values, non-finite floats (json.loads parses the NaN/Infinity
    literals -- #1523), and oversized ints, whose isfinite()
    conversion raises OverflowError instead of returning False
    (Codex review on PR #1534).
    """
    if value is None:
        return False
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return True
    try:
        return not math.isfinite(value)
    except OverflowError:
        return True


def _restore_persisted_job(meta, dir_name):
    """Validate one parsed meta.json against RESTORE_FIELD_TABLE.

    Returns (job, persist, reason): on success the restored job
    dict, whether the restore must be re-persisted via _write_meta
    (a transition fired, or interrupted=True was restored), and
    None; on a malformed entry, (None, False, the skip reason) --
    the caller prints exactly one stderr line naming the spool
    directory and this reason, and leaves the spool untouched.
    Behaviour-identical to the per-field guard blocks this
    replaces: #1522 (container), #1513/#1523 (timestamps),
    #1548/#1554 (id), #1556 (state/interrupted), #1532 (the
    ended_at transitions), #1568 (exit/cwd/cmd, folded in as
    table rows).
    """
    if not isinstance(meta, dict):
        return None, False, "meta.json is not a JSON object"
    _bad_fields = [
        name
        for name in ("started_at", "ended_at")
        if _bad_persisted_timestamp(meta.get(name))
    ]
    if _bad_fields:
        return None, False, f"bad {'/'.join(_bad_fields)} in meta.json"
    _raw_id = meta.get("id")
    if _raw_id is not None and not isinstance(_raw_id, str):
        return None, False, "non-string id in meta.json"
    job_id = _raw_id or dir_name
    if job_id != dir_name or job_id in (".", "..") or "/" in job_id or "\\" in job_id:
        return None, False, "invalid id in meta.json"
    try:
        job_dir(job_id)
    except (OSError, RuntimeError, ValueError):
        return None, False, "unsafe spool directory"
    _raw_state = meta.get("state")
    if _raw_state is not None and _raw_state not in ("running", "done"):
        return None, False, "invalid state in meta.json"
    _raw_interrupted = meta.get("interrupted")
    if _raw_interrupted is not None and not isinstance(_raw_interrupted, bool):
        return None, False, "non-boolean interrupted in meta.json"
    # Issue #1568, folded in as table rows per the fold-in rule at
    # the table above: _write_meta only ever writes exit as an int
    # returncode (negative for signal deaths) or None, and cwd/cmd
    # as strings. Present values of any other shape were restored
    # and served verbatim -- exit="0"/[0], cwd=123 and cmd={"x": 1}
    # all reached job_public/job_summary and were re-persisted by
    # _write_meta. Same skip contract as every other guarded
    # field. bool is rejected for exit explicitly: it is an int
    # subclass, but the writer never produces one. Missing/None
    # keeps the established defaults (None / home / "").
    _raw_exit = meta.get("exit")
    if _raw_exit is not None and (
        isinstance(_raw_exit, bool) or not isinstance(_raw_exit, int)
    ):
        return None, False, "non-integer exit in meta.json"
    if meta.get("cwd") is not None and not isinstance(meta.get("cwd"), str):
        return None, False, "non-string cwd in meta.json"
    if meta.get("cmd") is not None and not isinstance(meta.get("cmd"), str):
        return None, False, "non-string cmd in meta.json"
    _raw_ctx = meta.get("ctx")
    if _raw_ctx is not None and (
        not isinstance(_raw_ctx, dict)
        or any(
            not isinstance(k, str) or not isinstance(v, str)
            for k, v in _raw_ctx.items()
        )
    ):
        return None, False, "non-string-map ctx in meta.json"
    state = _raw_state or "done"
    exit_code = meta.get("exit")
    ended_at = meta.get("ended_at")
    interrupted = bool(_raw_interrupted)
    stamped = False
    if state == "running":
        # The process did not survive the restart: nobody can poll
        # it. A finite end time is what lets the reaper apply TTL
        # to the restored job's spool; leaving it null retains it
        # forever.
        state = "done"
        exit_code = None
        interrupted = True
        ended_at = time.time()
    elif ended_at is None:
        # A restored done job with a null/missing end time (#1532):
        # the reaper reads null as "now", so its spool would be
        # retained forever, and duration_ms would grow with
        # wall-clock time on every poll. Stamp a finite end time
        # at load and persist it, so the TTL runs from the stamp
        # and duration is stable.
        ended_at = time.time()
        stamped = True
    job = {
        "id": job_id,
        "proc": None,
        "cwd": meta.get("cwd") or str(Path.home()),
        "cmd_repr": meta.get("cmd") or "",
        "started_at": float(meta.get("started_at") or 0.0),
        "ended_at": ended_at,
        "exit": exit_code,
        "state": state,
        "interrupted": interrupted,
        "ctx": dict(meta.get("ctx") or {}),
        "_files": (),
    }
    return job, (interrupted or stamped), None


def load_persisted_jobs():
    """Rebuild JOBS from the spool directory so job IDs survive restarts.

    The loop does no field logic of its own (design #1578 rule 2):
    iterate, skip symlink/non-directory entries as before, parse
    JSON, validate through _restore_persisted_job(), then admit
    the job or print the one-line skip reason and continue.
    """
    try:
        base = _jobs_root()
        entries = list(base.iterdir())
    except (OSError, RuntimeError, ValueError):
        return
    for d in entries:
        if d.is_symlink():
            print(
                f"laptop-agent: skipping persisted job {d}: "
                "spool directory is a symlink",
                file=sys.stderr,
                flush=True,
            )
            continue
        if not d.is_dir():
            continue
        try:
            meta = json.loads((d / "meta.json").read_text())
        except (OSError, ValueError):
            continue
        job, persist, reason = _restore_persisted_job(meta, d.name)
        if job is None:
            print(
                f"laptop-agent: skipping persisted job {d}: {reason}",
                file=sys.stderr,
                flush=True,
            )
            continue
        JOBS[job["id"]] = job
        if persist:
            _write_meta(job)


def job_state(job):
    if job["state"] == "running":
        proc = job["proc"]
        if proc is not None and proc.poll() is None:
            return "running"
        if not job.get("_settled", True):
            # The process exited but its pump is still draining the
            # pipes into the spool (#15420): reading as done now would
            # let a client stop before the last bytes land. The watch
            # thread bounds this at PUMP_SETTLE_S.
            return "running"
        job["state"] = "done"
        if proc is not None:
            job["exit"] = proc.returncode
        if job["ended_at"] is None:
            job["ended_at"] = time.time()
        _write_meta(job)
    return job["state"]


def clamp_poll_wait(wait_s):
    """Bound one GET /jobs/<job> long-poll wait (#5757).

    The request is held at most JOBS_WAIT_MAX_S seconds, returning
    sooner on output or completion; larger asks are clamped, not
    rejected, so old and new clients stay interoperable.
    """
    return min(wait_s, JOBS_WAIT_MAX_S)


def job_public(job, out_off=0, err_off=0):
    state = job_state(job)
    try:
        d = job_dir(job["id"])
    except (RuntimeError, ValueError) as e:
        # Post-load spool swap (#1575): serve the job's in-memory
        # record with empty output instead of raising ValueError,
        # which do_GET would turn into a 400 blaming the request.
        print(
            f"laptop-agent: cannot read spool for job {job['id']}: {e}",
            file=sys.stderr,
            flush=True,
        )
        d = None
    out = b""
    err = b""
    # Spool sizes ride every reply (#15420) so a client draining a
    # finished job knows where the end is, instead of stopping at the
    # first page (at most MAX_OUT bytes per stream).
    out_size = 0
    err_size = 0
    if d is not None:
        try:
            with open(d / "stdout", "rb") as f:
                out_size = os.fstat(f.fileno()).st_size
                f.seek(out_off)
                out = f.read(MAX_OUT)
        except OSError:
            pass
        try:
            with open(d / "stderr", "rb") as f:
                err_size = os.fstat(f.fileno()).st_size
                f.seek(err_off)
                err = f.read(MAX_OUT)
        except OSError:
            pass
    out_dropped = _read_dropped(d, "stdout")
    err_dropped = _read_dropped(d, "stderr")
    return {
        "job_id": job["id"],
        "state": _public_state(job, state),
        "exit": job["exit"] if state == "done" else None,
        "cwd": job["cwd"],
        "cmd": job["cmd_repr"],
        "started_at": job["started_at"],
        "interrupted": job.get("interrupted", False),
        "ctx": job.get("ctx") or {},
        "duration_ms": int(
            ((job["ended_at"] or time.time()) - job["started_at"]) * 1000
        ),
        "stdout": out.decode("utf-8", "replace"),
        "stderr": err.decode("utf-8", "replace"),
        "out_next": out_off + len(out),
        "err_next": err_off + len(err),
        "out_size": max(out_size, out_off + len(out)),
        "err_size": max(err_size, err_off + len(err)),
        "out_dropped": out_dropped,
        "err_dropped": err_dropped,
        "bytes_dropped": out_dropped + err_dropped,
        "reaped": job.get("reaped", False),
    }


def job_summary(job):
    """Summary serializer for the GET /jobs list route.

    Returns the documented list shape (docstring above): identity,
    state, and spool sizes/cursors only.  Unlike job_public it never
    reads or embeds stdout/stderr payloads, so polling the list costs
    O(jobs), not O(jobs x 2 x MAX_OUT).  out_next/err_next are the
    spool sizes, i.e. the offsets a client would page from in the
    single-job route.
    """
    state = job_state(job)
    try:
        d = job_dir(job["id"])
    except (RuntimeError, ValueError) as e:
        # Post-load spool swap (#1575): one poisoned spool must not
        # fail the whole GET /jobs list -- report zero sizes for this
        # job (same shape as a missing spool file) and serve the rest.
        print(
            f"laptop-agent: cannot read spool for job {job['id']}: {e}",
            file=sys.stderr,
            flush=True,
        )
        d = None
    try:
        out_size = (d / "stdout").stat().st_size if d else 0
    except OSError:
        out_size = 0
    try:
        err_size = (d / "stderr").stat().st_size if d else 0
    except OSError:
        err_size = 0
    out_dropped = _read_dropped(d, "stdout")
    err_dropped = _read_dropped(d, "stderr")
    return {
        "job_id": job["id"],
        "state": _public_state(job, state),
        "exit": job["exit"] if state == "done" else None,
        "cwd": job["cwd"],
        "cmd": job["cmd_repr"],
        "started_at": job["started_at"],
        "interrupted": job.get("interrupted", False),
        "ctx": job.get("ctx") or {},
        "duration_ms": int(
            ((job["ended_at"] or time.time()) - job["started_at"]) * 1000
        ),
        "out_bytes": out_size,
        "err_bytes": err_size,
        "out_next": out_size,
        "err_next": err_size,
        "out_dropped": out_dropped,
        "err_dropped": err_dropped,
        "bytes_dropped": out_dropped + err_dropped,
        "reaped": job.get("reaped", False),
    }


def _spawn_job(job_id, argv, cmd_repr, cwd, req_env, trip, t_received_ms, kind="job"):
    """Start one spooled job under job_id and return its record.

    The caller holds one SEMA slot; from here the job's watch thread
    owns it and releases it when the process exits. On failure the
    slot is released, the half-made spool removed, and the error
    re-raised for the route to answer. Shared by POST /jobs and by
    /exec with a grace window, which adopts the trip id as the job id
    (#15420, design #10356 section 3.4). kind names the trip-log
    record ("job" or "exec").
    """
    d = None
    out_w = err_w = proc = pump = None
    watch_thread = None
    watch_started = False
    try:
        d = job_dir(job_id)
        d.mkdir(parents=True, exist_ok=True)
        env = dict(os.environ)
        env.update(req_env or {})
        # #14186: record the FLEET_CTX_* subset (merged daemon +
        # request env, request wins) in the spool; ids only.
        ctx = _extract_ctx(env)
        # The spool files exist from the start so a reader sees
        # empty output, not a missing spool; the pump appends
        # to them up to SPOOL_CAP (#15420).
        (d / "stdout").write_bytes(b"")
        (d / "stderr").write_bytes(b"")
        pump, out_w, err_w = _start_spool_pump(d)
        proc = subprocess.Popen(
            argv,
            cwd=cwd,
            env=env,
            stdout=out_w,
            stderr=err_w,
            start_new_session=True,
        )
        # Only the job holds the write ends now, so the pump
        # sees EOF when the job (and anything it left holding
        # them) exits.
        for fd in (out_w, err_w):
            os.close(fd)
        out_w = err_w = None
        job = {
            "id": job_id,
            "proc": proc,
            "cwd": cwd,
            "cmd_repr": cmd_repr,
            "started_at": time.time(),
            "ended_at": None,
            "exit": None,
            "state": "running",
            "interrupted": False,
            "ctx": ctx,
            "_files": (),
            "_pump": pump,
            "_settled": False,
            "trip": trip,
            "t_received": t_received_ms,
            "kind": kind,
        }
        _write_meta(job)
        with JOBS_LOCK:
            JOBS[job_id] = job

        def watch(j=job):
            try:
                j["proc"].wait()
            finally:
                SEMA.release()
            j["ended_at"] = time.time()
            # Let the pump drain what the job wrote before the
            # job reads as done, but never wait on a grandchild
            # that keeps the pipes open (#15420).
            try:
                j["_pump"].wait(timeout=PUMP_SETTLE_S)
            except subprocess.TimeoutExpired:
                pass
            j["_settled"] = True
            job_state(j)
            try:
                d = job_dir(j["id"])
                out_b = (d / "stdout").stat().st_size
                err_b = (d / "stderr").stat().st_size
            except (OSError, RuntimeError, ValueError):
                out_b = err_b = 0
            log_trip(
                j["trip"],
                j.get("kind", "job"),
                j["cmd_repr"],
                int((j["ended_at"] - j["started_at"]) * 1000),
                j["proc"].returncode,
                out_b,
                err_b,
                t_received=j.get("t_received"),
            )

        watch_thread = threading.Thread(target=watch, daemon=True)
        watch_thread.start()
        watch_started = True
    except Exception:
        # Thread.start() can raise after the new thread has begun.
        # In that case its finally block owns the acquired slot.
        if watch_thread is not None and watch_thread.ident is not None:
            watch_started = True
        if not watch_started:
            try:
                if proc is not None:
                    try:
                        proc.kill()
                        proc.wait(timeout=5)
                    except (OSError, subprocess.TimeoutExpired):
                        pass
                with JOBS_LOCK:
                    JOBS.pop(job_id, None)
                for fd in (out_w, err_w):
                    if fd is not None:
                        try:
                            os.close(fd)
                        except OSError:
                            pass
                if pump is not None:
                    # With the write ends closed the pump sees
                    # EOF; kill it only if it lingers.
                    try:
                        pump.wait(timeout=PUMP_SETTLE_S)
                    except subprocess.TimeoutExpired:
                        pump.kill()
                        pump.wait(timeout=5)
                if d is not None:
                    shutil.rmtree(d, ignore_errors=True)
            except Exception:
                pass
            finally:
                SEMA.release()
        raise
    return job


def _exec_procs_discard(trip, proc):
    with EXEC_PROCS_LOCK:
        procs = EXEC_PROCS.get(trip)
        if procs is not None:
            procs.discard(proc)
            if not procs:
                EXEC_PROCS.pop(trip, None)


def _grace_reap(job):
    """Hold a dropped exec for its grace window, then reap it.

    An attach (GET /jobs/<trip>) or the command finishing ends the
    window early. Otherwise the group is killed TERM then KILL, as a
    grace of 0 does at once, and the job stays listed with reaped
    set and its spool intact (#15420, design #10356 §3.4).
    """
    proc = job["proc"]
    try:
        while time.time() < job["grace_until"]:
            if job.get("attached") or job_state(job) == "done":
                return
            time.sleep(0.25)
        if not job.get("attached") and job_state(job) == "running":
            job["reaped"] = True
            kill_group(proc)
    finally:
        _exec_procs_discard(job["trip"], proc)


def _exec_job_result(job, timed_out):
    """The /exec result for a finished exec job: run_one's shape.

    Output is the tail of each spool (at most MAX_OUT per stream);
    truncated is set when either tail was cut or the spool cap
    dropped bytes.
    """
    try:
        d = job_dir(job["id"])
    except (RuntimeError, ValueError):
        d = None
    if d is not None:
        out, out_cut = tail_file(d / "stdout")
        err, err_cut = tail_file(d / "stderr")
    else:
        out, out_cut, err, err_cut = b"", False, b"", False
    dropped = _read_dropped(d, "stdout") + _read_dropped(d, "stderr")
    ended = job["ended_at"] or time.time()
    result = {
        "exit": job["exit"],
        "stdout": out.decode("utf-8", "replace"),
        "stderr": err.decode("utf-8", "replace"),
        "duration_ms": int((ended - job["started_at"]) * 1000),
        "truncated": out_cut or err_cut or dropped > 0,
        "job_id": job["id"],
    }
    if timed_out:
        result["exit"] = None
        result["timeout"] = True
    if job.get("cancelled"):
        result["exit"] = None
        result["timeout"] = True
        result["cancelled"] = True
    return result


def _public_state(job, state):
    """The state a reply shows: "grace" for a dropped exec in its
    window, otherwise the job's own state (#15420)."""
    if state == "running" and job.get("grace_until") and not job.get("attached"):
        return "grace"
    return state


def reap_old_jobs():
    now = time.time()
    with JOBS_LOCK:
        dead = []
        for j in list(JOBS.values()):
            if job_state(j) == "done" and (j["ended_at"] or now) < now - JOB_TTL_S:
                dead.append(j)
        for j in dead:
            JOBS.pop(j["id"], None)
    for j in dead:
        # Output is uncapped, so retention must remove the spool too.
        # job_dir() raises ValueError for an unsafe spool (#1555) and
        # ignore_errors covers only rmtree's own errors, so guard each
        # job: one unsafe spool must not abort the whole reap (#1560).
        # The dead jobs are already popped from JOBS, so a spool that
        # cannot be removed is log-and-drop -- one stderr line naming
        # the job -- and the reap continues with the remaining jobs;
        # without the line the leak would be silent and permanent,
        # because no later reap pass still knows these jobs.
        try:
            shutil.rmtree(job_dir(j["id"]), ignore_errors=True)
        except (OSError, RuntimeError, ValueError) as e:
            print(
                f"laptop-agent: reap: cannot remove spool for job {j['id']}: {e}",
                file=sys.stderr,
                flush=True,
            )


class FramingError(ValueError):
    """Invalid request framing: the next request boundary is unknown,
    so the connection must be closed after the 400 response."""


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *args):  # quiet; journal gets tracebacks only
        pass

    def _send(self, code, obj, close=False):
        body = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        if close:
            self.send_header("Connection", "close")
            self.close_connection = True
        self.end_headers()
        self.wfile.write(body)

    def _send_bytes(self, code, data, extra_headers=None):
        self.send_response(code)
        self.send_header("Content-Type", "application/octet-stream")
        self.send_header("Content-Length", str(len(data)))
        for k, v in (extra_headers or {}).items():
            self.send_header(k, str(v))
        self.end_headers()
        self.wfile.write(data)

    def _authed(self):
        given = self.headers.get("X-Agent-Token") or ""
        if not given:
            return False
        # Compare in bytes space (#1741): hmac.compare_digest on str
        # raises TypeError for non-ASCII operands, which the request
        # handlers turned into a 400 leaking the exception message --
        # and a non-ASCII token file produced a daemon that could
        # never authenticate. http.server decodes header bytes as
        # latin-1, so try both the utf-8 encoding of the decoded
        # string (a latin-1 sender of the token's characters) and the
        # raw wire bytes it recovers (a utf-8 sender).
        expected = TOKEN.encode("utf-8")
        candidates = {given.encode("utf-8")}
        try:
            candidates.add(given.encode("latin-1"))
        except UnicodeEncodeError:
            pass
        return any(hmac.compare_digest(c, expected) for c in candidates)

    def _client_gone(self):
        """True once the client connection has died (EOF/error).

        ThreadingHTTPServer handlers never poll the socket while a
        /exec runs, so a client killed by an outer `timeout` used to
        leave its command running to the daemon-side cap — or longer,
        for grandchildren. Closing the write side makes the peer see
        FIN at once; the handler thread then re-checks between drain
        slices. Used as run_one's disconnect watchdog trigger."""
        if isinstance(self.connection, ssl.SSLSocket):
            return _raw_peer_gone(self.connection)
        try:
            self.connection.setblocking(False)
            try:
                data = self.connection.recv(1, socket.MSG_PEEK)
                return data == b""
            except BlockingIOError:
                return False
            except OSError:
                return True
            finally:
                self.connection.setblocking(True)
        except OSError:
            return True

    def _body(self):
        try:
            n = int(self.headers.get("Content-Length") or 0)
        except (TypeError, ValueError):
            raise FramingError("invalid Content-Length header")
        if n < 0:
            raise FramingError("invalid Content-Length header")
        if n > MAX_BODY:
            raise FramingError("request body too large")
        raw = self.rfile.read(n) if n else b"{}"
        try:
            body = json.loads(raw or b"{}")
        except ValueError:
            raise ValueError("request body is not valid JSON")
        if not isinstance(body, dict):
            raise ValueError("request body must be a JSON object")
        return body

    def _send_bad_request(self, e):
        try:
            self._send(
                400,
                {"error": str(e) or "bad request"},
                close=isinstance(e, FramingError),
            )
        except Exception:
            pass

    def do_GET(self):
        try:
            self._do_GET()
        except (ValueError, KeyError, TypeError, AttributeError, IndexError) as e:
            self._send_bad_request(e)
        except (BrokenPipeError, ConnectionResetError):
            pass

    def _do_GET(self):
        if not self._authed():
            return self._send(401, {"error": "bad token"})
        path = urlparse(self.path)
        if path.path == "/health":
            with JOBS_LOCK:
                n = len(JOBS)
            return self._send(
                200,
                {
                    "ok": True,
                    "uptime_s": int(time.time() - STARTED),
                    "jobs": n,
                    "load1": os.getloadavg()[0],
                },
            )
        if path.path == "/file":
            q = parse_qs(path.query)
            fpath = (q.get("path") or [""])[0]
            if not fpath:
                return self._send(400, {"error": "path required"})
            try:
                off = int((q.get("off") or ["0"])[0])
                length = int((q.get("len") or [str(FILE_CHUNK_MAX)])[0])
            except ValueError:
                return self._send(400, {"error": "off/len must be ints"})
            if off < 0 or length <= 0:
                return self._send(400, {"error": "off>=0, len>=1 required"})
            length = min(length, FILE_CHUNK_MAX)
            try:
                size = os.path.getsize(fpath)
                if not os.path.isfile(fpath):
                    raise OSError("not a regular file")
            except OSError as e:
                return self._send(404, {"error": f"no such file: {e}"})
            if off > size:
                return self._send(
                    416, {"error": "off beyond end of file", "size": size}
                )
            with open(fpath, "rb") as fh:
                fh.seek(off)
                data = fh.read(length)
            return self._send_bytes(200, data, {"X-File-Size": size, "X-File-Off": off})
        if path.path == "/jobs":
            with JOBS_LOCK:
                items = [job_summary(j) for j in JOBS.values()]
            return self._send(200, {"jobs": items})
        if path.path.startswith("/jobs/"):
            job_id = path.path.split("/")[2]
            q = parse_qs(path.query)
            with JOBS_LOCK:
                job = JOBS.get(job_id)
            if job is None:
                return self._send(404, {"error": "no such job"})
            try:
                out_off = int(q.get("out_off", [0])[0])
                err_off = int(q.get("err_off", [0])[0])
                wait_s = int(q.get("wait_s", [0])[0])
            except (TypeError, ValueError):
                raise ValueError("out_off/err_off/wait_s must be integers")
            if out_off < 0 or err_off < 0:
                raise ValueError("out_off/err_off must be >= 0")
            if wait_s < 0:
                raise ValueError("wait_s must be >= 0")
            deadline = time.monotonic() + clamp_poll_wait(wait_s)
            while True:
                result = job_public(job, out_off, err_off)
                if (
                    result["state"] == "done"
                    or result["stdout"]
                    or result["stderr"]
                    or time.monotonic() >= deadline
                ):
                    return self._send(200, result)
                time.sleep(min(0.1, max(0, deadline - time.monotonic())))
            if job.get("kind") == "exec":
                # Reading an exec's spool is `fleet http attach`: it
                # turns the exec into an ordinary job, so its grace
                # reap and its exec timeout no longer apply (#15420).
                job["attached"] = True
            return self._send(200, job_public(job, out_off, err_off))
        return self._send(404, {"error": "no such route"})

    def _exec_with_grace(self, trip, cmds, timeout, grace, t_received_ms):
        """POST /exec with a grace window (#15420, design #10356 §3.4).

        The trip id becomes the job id and the command runs as a
        spooled job from the start, so a client that drops keeps a
        name to come back with. While the client is connected this
        behaves like a plain /exec: the reply carries the exit and the
        tail of each stream, the timeout and /exec/<trip>/cancel kill
        the group. When the connection drops, the command gets grace
        seconds; GET /jobs/<trip> in that window attaches to it as an
        ordinary job, and otherwise it is reaped TERM then KILL and
        left in the list as reaped, spool intact.
        """
        if len(cmds) != 1:
            return self._send(400, {"error": "grace_s needs exactly one cmd"})
        try:
            job_dir(trip)
        except (OSError, RuntimeError, ValueError):
            return self._send(400, {"error": "trip is not a valid job id"})
        with JOBS_LOCK:
            exists = trip in JOBS
        if exists:
            return self._send(409, {"error": f"job {trip} already exists"})
        c = cmds[0]
        if c.get("shell") is not None:
            argv = [BASH, "-lc", c["shell"]]
            cmd_repr = c["shell"]
        else:
            argv = list(c["argv"])
            cmd_repr = " ".join(str(a) for a in argv)
        cwd = c.get("cwd") or str(Path.home())
        if not SEMA.acquire(timeout=5):
            return self._send(503, {"error": "busy"})
        t0 = time.time()
        try:
            job = _spawn_job(
                trip,
                argv,
                cmd_repr,
                cwd,
                c.get("env"),
                trip,
                t_received_ms,
                kind="exec",
            )
        except Exception as e:  # cwd missing, argv[0] missing, ...
            log_trip(
                trip,
                "exec",
                cmd_repr,
                int((time.time() - t0) * 1000),
                None,
                0,
                0,
                t_received=t_received_ms,
            )
            result = {
                "exit": None,
                "error": f"{type(e).__name__}: {e}",
                "stdout": "",
                "stderr": "",
                "duration_ms": int((time.time() - t0) * 1000),
                "truncated": False,
            }
            return self._send(200, {"results": [result], "_t_received": t_received_ms})
        proc = job["proc"]
        with EXEC_PROCS_LOCK:
            EXEC_PROCS.setdefault(trip, set()).add(proc)
        deadline = time.time() + timeout
        timed_out = False
        dropped = False
        ticks = 0
        try:
            while job_state(job) != "done":
                if time.time() >= deadline and not job.get("attached"):
                    timed_out = True
                    kill_group(proc)
                    settle = time.time() + PUMP_SETTLE_S + 5
                    while job_state(job) != "done" and time.time() < settle:
                        time.sleep(0.05)
                    break
                # The socket is checked at 4 Hz, as the plain path does.
                if ticks % 5 == 0 and self._client_gone():
                    dropped = True
                    break
                ticks += 1
                time.sleep(0.05)
        finally:
            if not dropped:
                _exec_procs_discard(trip, proc)
        if dropped:
            job["grace_until"] = time.time() + grace
            threading.Thread(target=_grace_reap, args=(job,), daemon=True).start()
            return None
        result = _exec_job_result(job, timed_out)
        return self._send(200, {"results": [result], "_t_received": t_received_ms})

    def do_POST(self):
        try:
            self._do_POST()
        except (ValueError, KeyError, TypeError, AttributeError, IndexError) as e:
            self._send_bad_request(e)
        except (BrokenPipeError, ConnectionResetError):
            pass

    def _do_POST(self):
        # Trip v2 (code#5702): stamp reception at request entry; the
        # value is echoed to the client as the internal _t_received
        # response field (stripped client-side) and logged daemon-side.
        t_received_ms = int(time.time() * 1000)
        if not self._authed():
            return self._send(401, {"error": "bad token"})
        path = urlparse(self.path).path
        if path == "/exec":
            body = self._body()
            trip = self.headers.get("X-Trip-Id") or body.get("trip")
            if not isinstance(trip, str) or not trip:
                trip = secrets.token_hex(6)
            try:
                timeout = int(body.get("timeout_s", EXEC_TIMEOUT_DEFAULT))
            except (TypeError, ValueError, OverflowError):
                return self._send(400, {"error": "timeout_s must be an integer"})
            timeout = max(1, min(timeout, EXEC_TIMEOUT_MAX))
            grace = body.get("grace_s")
            if grace is None:
                grace = 0
            if (
                isinstance(grace, bool)
                or not isinstance(grace, int)
                or not 0 <= grace <= EXEC_GRACE_MAX
            ):
                return self._send(
                    400,
                    {"error": f"grace_s must be an integer 0-{EXEC_GRACE_MAX}"},
                )
            cmds = body.get("cmds") or []
            if not isinstance(cmds, list) or not cmds:
                return self._send(400, {"error": "cmds required"})
            for c in cmds:
                if not isinstance(c, dict) or (
                    c.get("shell") is None
                    and not (isinstance(c.get("argv"), (list, tuple)) and c["argv"])
                ):
                    return self._send(
                        400, {"error": "each cmd requires 'argv' or 'shell'"}
                    )
                if c.get("env") is not None and not isinstance(c.get("env"), dict):
                    return self._send(400, {"error": "cmd env must be an object"})
            if grace > 0:
                return self._exec_with_grace(trip, cmds, timeout, grace, t_received_ms)
            if not SEMA.acquire(timeout=5):
                return self._send(503, {"error": "busy"})
            cancel_event = threading.Event()

            def watch_disconnect(ev=cancel_event):
                # Poll the socket at 4 Hz while the exec runs; a dead
                # client connection kills the trip's process groups
                # so no orphan work survives a caller-side timeout.
                while not ev.is_set():
                    if self._client_gone():
                        ev.set()
                        with EXEC_PROCS_LOCK:
                            procs = list(EXEC_PROCS.get(trip, ()))
                        for p in procs:
                            if p.poll() is None:
                                kill_group(p)
                        return
                    ev.wait(0.25)

            watcher = threading.Thread(target=watch_disconnect, daemon=True)
            watcher.start()
            try:
                results = [
                    run_one(c, timeout, trip=trip, cancel_event=cancel_event)
                    for c in cmds
                ]
            finally:
                cancel_event.set()
                watcher.join(timeout=1)
                SEMA.release()
            for c, r in zip(cmds, results):
                cmd_str = r.pop("_cmd", "")
                out_b = r.pop("_out_bytes", 0)
                err_b = r.pop("_err_bytes", 0)
                # #14188: the command's effective context (daemon env
                # merged with the request's, request wins) — its id
                # only, as the job spool records the full subset.
                merged = dict(os.environ)
                merged.update(c.get("env") or {})
                log_trip(
                    trip,
                    "exec",
                    cmd_str,
                    r.get("duration_ms"),
                    r.get("exit"),
                    out_b,
                    err_b,
                    t_received=t_received_ms,
                    ctx_id=_extract_ctx(merged).get("FLEET_CTX_ID"),
                )
            try:
                return self._send(
                    200, {"results": results, "_t_received": t_received_ms}
                )
            except (BrokenPipeError, ConnectionResetError):
                pass  # client already gone; the work is done/killed
            return
        if path == "/cancel" or (
            path.startswith("/exec/") and path.endswith("/cancel")
        ):
            body = self._body()
            trip = (
                body.get("trip")
                or self.headers.get("X-Trip-Id")
                or (path.split("/")[2] if path.startswith("/exec/") else None)
            )
            if not isinstance(trip, str) or not trip:
                return self._send(400, {"error": "trip required"})
            with EXEC_PROCS_LOCK:
                procs = list(EXEC_PROCS.get(trip, ()))
            with JOBS_LOCK:
                exec_job = JOBS.get(trip)
            if exec_job is not None and exec_job.get("kind") == "exec" and procs:
                exec_job["cancelled"] = True
            killed = 0
            for p in procs:
                if p.poll() is None:
                    kill_group(p)
                    killed += 1
            return self._send(200, {"ok": True, "cancelled": killed})
        if path == "/jobs":
            body = self._body()
            trip = self.headers.get("X-Trip-Id") or body.get("trip")
            if not isinstance(trip, str) or not trip:
                trip = secrets.token_hex(6)
            cwd = body.get("cwd") or str(Path.home())
            if body.get("env") is not None and not isinstance(body.get("env"), dict):
                return self._send(400, {"error": "env must be an object"})
            if body.get("shell") is not None:
                argv = [BASH, "-lc", body["shell"]]
                cmd_repr = body["shell"]
            elif isinstance(body.get("argv"), (list, tuple)) and body["argv"]:
                argv = list(body["argv"])
                cmd_repr = " ".join(str(a) for a in argv)
            else:
                return self._send(400, {"error": "argv or shell required"})
            if not SEMA.acquire(timeout=5):
                return self._send(503, {"error": "busy"})
            job_id = secrets.token_hex(6)
            try:
                _spawn_job(
                    job_id,
                    argv,
                    cmd_repr,
                    cwd,
                    body.get("env"),
                    trip,
                    t_received_ms,
                )
            except Exception as e:
                return self._send(400, {"error": f"{type(e).__name__}: {e}"})
            reap_old_jobs()
            return self._send(200, {"job_id": job_id, "_t_received": t_received_ms})
        if path.startswith("/jobs/") and path.endswith("/kill"):
            job_id = path.split("/")[2]
            with JOBS_LOCK:
                job = JOBS.get(job_id)
            if job is None:
                return self._send(404, {"error": "no such job"})
            if job_state(job) == "running" and job["proc"] is not None:
                pid = job["proc"].pid
                _kill_group(pid, signal.SIGTERM)

                def hard_kill(p=pid):
                    deadline = time.time() + KILL_GRACE_S
                    while time.time() < deadline:
                        if not _group_alive(p):
                            return
                        time.sleep(0.1)
                    if _group_alive(p):
                        _kill_group(p, signal.SIGKILL)

                threading.Thread(target=hard_kill, daemon=True).start()
            return self._send(200, {"ok": True, "job_id": job_id})
        if path.startswith("/files/"):
            return self._files_route(path, t_received_ms)
        return self._send(404, {"error": "no such route"})

    # -- #5759 file transfer routes -------------------------------------
    #
    # Route semantics are fixed by the shipped fleet-proxy client
    # (pkgs/fleet/crates/fleet-proxy/src/files.rs); its in-memory fake
    # daemon in that module's tests is the oracle these handlers
    # follow. Design invariants: deterministic transfer temps
    # (.<name>.fleet-part-<sha12>), offset-idempotent writes,
    # hash-checked commit followed by an atomic rename, so partial
    # content is never visible at the destination path, and reads are
    # bounded by FILES_CHUNK_SIZE.

    def _files_body(self):
        """Read a /files/* request body under the transfer cap.

        The chunk bodies the client sends exceed the /exec MAX_BODY;
        this reads them while still refusing an unbounded body.
        """
        try:
            n = int(self.headers.get("Content-Length") or 0)
        except (TypeError, ValueError):
            raise FramingError("invalid Content-Length header")
        if n < 0:
            raise FramingError("invalid Content-Length header")
        if n > FILES_BODY_MAX:
            raise FramingError("request body too large")
        raw = self.rfile.read(n) if n else b"{}"
        try:
            body = json.loads(raw or b"{}")
        except ValueError:
            raise ValueError("request body is not valid JSON")
        if not isinstance(body, dict):
            raise ValueError("request body must be a JSON object")
        return body

    def _files_route(self, path, t_received_ms):
        try:
            body = self._files_body()
        except FramingError as e:
            return self._send(400, {"error": str(e) or "bad request"}, close=True)
        handler = {
            "/files/stat": self._files_stat,
            "/files/begin": self._files_begin,
            "/files/write": self._files_write,
            "/files/commit": self._files_commit,
            "/files/abort": self._files_abort,
            "/files/read": self._files_read,
        }.get(path)
        if handler is None:
            return self._send(404, {"error": "no such route"})
        reply = handler(body)
        if "_t_received" not in reply:
            reply["_t_received"] = t_received_ms
        return self._send(200, reply)

    def _files_stat(self, body):
        path = body.get("path")
        err = _files_validate(path)
        if err:
            return {"error": err}
        try:
            st = os.stat(path)
        except FileNotFoundError:
            return {"exists": False, "type": "missing", "size": 0}
        except OSError as e:
            return {"error": f"stat {path}: {e}"}
        is_dir = os.path.isdir(path)
        out = {
            "exists": True,
            "type": "dir" if is_dir else "file",
            "size": 0 if is_dir else st.st_size,
            "mtime": int(st.st_mtime),
        }
        # The client asks for the hash only on a get, and only the hash
        # of a regular file: hashing a directory or a device is not
        # meaningful and could block.
        if body.get("sha256") and not is_dir and os.path.isfile(path):
            try:
                out["sha256"] = _files_sha_file(Path(path))
            except OSError as e:
                return {"error": f"hash {path}: {e}"}
        return out

    def _files_begin(self, body):
        path = body.get("path")
        err = _files_validate(path)
        if err:
            return {"error": err}
        sha = body.get("sha256")
        if not isinstance(sha, str) or not sha:
            return {"error": "sha256 required"}
        mode = body.get("mode")
        if not _files_mode_ok(mode):
            return {"error": "mode must be an integer in 0..0o7777"}
        temp = Path(files_temp_path(path, sha))
        parent = temp.parent
        try:
            if not parent.exists():
                return {"error": f"no such directory: {parent}"}
            if not parent.is_dir():
                return {"error": f"not a directory: {parent}"}
            # Resume: an existing temp for the same (path, sha) is kept
            # and never truncated -- the client re-derives the name and
            # stat()s it to learn where to resume.
            fd = os.open(temp, os.O_WRONLY | os.O_CREAT, 0o600)
            os.close(fd)
        except OSError as e:
            return {"error": f"begin {temp}: {e}"}
        if mode is not None:
            with FILES_PENDING_MODES_LOCK:
                FILES_PENDING_MODES[str(temp)] = int(mode)
        return {"ok": True}

    def _files_write(self, body):
        path = body.get("path")
        err = _files_validate(path)
        if err:
            return {"error": err}
        sha = body.get("sha256")
        if not isinstance(sha, str) or not sha:
            return {"error": "sha256 required"}
        try:
            offset = int(body.get("offset", 0))
        except (TypeError, ValueError):
            return {"error": "offset must be an integer"}
        if offset < 0:
            return {"error": "offset must be >= 0"}
        data_b64 = body.get("data_b64")
        if not isinstance(data_b64, str):
            return {"error": "data_b64 required"}
        try:
            data = base64.b64decode(data_b64, validate=True)
        except (binascii.Error, ValueError):
            return {"error": "data_b64 is not valid base64"}
        if len(data) > FILES_CHUNK_SIZE:
            return {"error": f"chunk exceeds {FILES_CHUNK_SIZE} bytes"}
        temp = Path(files_temp_path(path, sha))
        try:
            with open(temp, "r+b" if temp.exists() else "w+b") as f:
                f.seek(0, os.SEEK_END)
                have = f.tell()
                if offset > have:
                    return {"error": "offset beyond end of temp"}
                # Offset-idempotent: bytes already present must match a
                # re-sent chunk (a resumed write replays the boundary
                # chunk), and a genuine conflict is refused rather than
                # silently corrupting the transfer.
                overlap = min(have - offset, len(data))
                if overlap > 0:
                    f.seek(offset)
                    existing = f.read(overlap)
                    if existing != data[:overlap]:
                        return {"error": "conflicting rewrite at offset"}
                f.seek(offset + overlap)
                f.write(data[overlap:])
                f.flush()
                os.fsync(f.fileno())
                # Report the durable temp length, not the end of this
                # write: a chunk fully interior to the temp (an
                # identical replay of already-durable bytes) leaves the
                # longer tail intact, so `have` wins (#14601).
                size = max(have, offset + len(data))
            return {"size": size}
        except OSError as e:
            return {"error": f"write {temp}: {e}"}

    def _files_commit(self, body):
        path = body.get("path")
        err = _files_validate(path)
        if err:
            return {"error": err}
        sha = body.get("sha256")
        if not isinstance(sha, str) or not sha:
            return {"error": "sha256 required"}
        temp = Path(files_temp_path(path, sha))
        if not temp.exists():
            return {"error": "no transfer in progress for this path and sha256"}
        try:
            actual = _files_sha_file(temp)
        except OSError as e:
            return {"error": f"hash {temp}: {e}"}
        if actual != sha:
            # Keep the temp: the client surfaces the daemon's words and
            # a later attempt can resume or abort it.
            return {"error": "sha256 mismatch", "actual": actual}
        size = temp.stat().st_size
        with FILES_PENDING_MODES_LOCK:
            mode = FILES_PENDING_MODES.pop(str(temp), None)
        try:
            if mode is not None:
                os.chmod(temp, mode)
            # Atomic publish: the rename is what makes partial content
            # at the destination impossible.
            os.replace(temp, path)
            _fsync_dir(Path(path).parent)
        except OSError as e:
            return {"error": f"commit {temp} -> {path}: {e}"}
        return {"size": size, "sha256": actual}

    def _files_abort(self, body):
        path = body.get("path")
        err = _files_validate(path)
        if err:
            return {"error": err}
        sha = body.get("sha256")
        if not isinstance(sha, str) or not sha:
            return {"error": "sha256 required"}
        temp = Path(files_temp_path(path, sha))
        with FILES_PENDING_MODES_LOCK:
            FILES_PENDING_MODES.pop(str(temp), None)
        try:
            temp.unlink()
        except FileNotFoundError:
            pass
        except OSError as e:
            return {"error": f"abort {temp}: {e}"}
        return {"ok": True}

    def _files_read(self, body):
        path = body.get("path")
        err = _files_validate(path)
        if err:
            return {"error": err}
        try:
            offset = int(body.get("offset", 0))
            length = int(body.get("length", FILES_CHUNK_SIZE))
        except (TypeError, ValueError):
            return {"error": "offset/length must be integers"}
        if offset < 0:
            return {"error": "offset must be >= 0"}
        if length <= 0:
            return {"error": "length must be >= 1"}
        length = min(length, FILES_CHUNK_SIZE)
        try:
            st = os.stat(path)
        except FileNotFoundError:
            return {"error": "no such file"}
        except OSError as e:
            return {"error": f"stat {path}: {e}"}
        if not os.path.isfile(path):
            return {"error": "not a regular file"}
        if offset > st.st_size:
            return {"error": "offset beyond end of file", "size": st.st_size}
        try:
            with open(path, "rb") as f:
                f.seek(offset)
                data = f.read(length)
        except OSError as e:
            return {"error": f"read {path}: {e}"}
        return {
            "data_b64": base64.b64encode(data).decode("ascii"),
            "eof": offset + len(data) >= st.st_size,
            "size": st.st_size,
            "mtime": int(st.st_mtime),
        }


def _reaper_loop():
    """Reap on a timer so the TTL fires without a new job (#1504).

    reap_old_jobs() used to run only from the POST /jobs path, so a
    daemon serving only /exec + /health + job polls never reaped and
    finished jobs' uncapped spool dirs accumulated forever.
    """
    while True:
        time.sleep(REAP_INTERVAL_S)
        try:
            reap_old_jobs()
        except Exception as e:  # a reaper fault must not kill the daemon
            print(
                f"laptop-agent: background reap failed: {e}",
                file=sys.stderr,
                flush=True,
            )


def _raw_peer_gone(tls_sock):
    """Has the peer under a TLS socket closed the TCP connection?

    SSLSocket refuses recv flags, so peek the TCP stream through a
    duplicate of the descriptor: EOF there means the client is gone;
    pending bytes (TLS records, an alert) or nothing yet do not."""
    try:
        raw = socket.socket(fileno=os.dup(tls_sock.fileno()))
    except OSError:
        return True
    try:
        raw.setblocking(False)
        try:
            return raw.recv(1, socket.MSG_PEEK) == b""
        except BlockingIOError:
            return False
        except OSError:
            return True
    finally:
        raw.close()


class AgentServer(ThreadingHTTPServer):
    """The daemon's HTTP server, with the BT-4 Phase 1 accept path.

    With no TLS context it is ThreadingHTTPServer unchanged. With one,
    each connection is sniffed in its own handler thread (so a slow
    peer never stalls accept): a first byte of 0x16 takes the TLS
    path, anything else is served as plaintext Phase 0 on the same
    port (rule M2) and logged, so adoption is visible before a later
    phase makes TLS mandatory. A failed handshake is logged and the
    connection dropped; it never falls back to plaintext."""

    def __init__(self, address, handler, tls_context=None):
        self.tls_context = tls_context
        super().__init__(address, handler)

    def finish_request(self, request, client_address):
        if self.tls_context is None:
            return super().finish_request(request, client_address)
        try:
            request.settimeout(TLS_HANDSHAKE_TIMEOUT)
            first = request.recv(1, socket.MSG_PEEK)
        except OSError as e:
            print(
                f"laptop-agent: TLS accept from {client_address[0]}: peek failed: {e}",
                flush=True,
            )
            return None
        if not first:
            return None
        if first[0] != _TLS_HANDSHAKE_RECORD:
            request.settimeout(None)
            print(
                f"laptop-agent: plaintext Phase 0 accept from {client_address[0]} "
                "on a TLS-configured endpoint",
                flush=True,
            )
            return super().finish_request(request, client_address)
        try:
            tls = self.tls_context.wrap_socket(request, server_side=True)
        except (OSError, ssl.SSLError) as e:
            print(
                f"laptop-agent: TLS handshake from {client_address[0]} failed: {e}",
                flush=True,
            )
            return None
        try:
            tls.settimeout(None)
            return super().finish_request(tls, client_address)
        finally:
            # Send close_notify, but never wait long for the peer's.
            try:
                tls.settimeout(2.0)
                tls.unwrap().close()
            except (OSError, ValueError, ssl.SSLError):
                tls.close()


# #15533: startup bind guard — the daemon starts before Tailscale
# assigns the tailnet address, so a one-shot bind fails with
# EADDRNOTAVAIL (errno 99) and the service burns its systemd restart
# budget before the address appears. Retry ONLY that signature within
# a 10-minute budget (1s doubling to a 15s cap); every other bind
# error (EADDRINUSE, permission, bad port) raises at once exactly as
# before, so a real misconfiguration never spins.
BIND_RETRY_BUDGET_S = 600.0
BIND_RETRY_BASE_S = 1.0
BIND_RETRY_CAP_S = 15.0


def bind_retry_delay(attempt):
    """Delay before the retry after `attempt` failures (0-based)."""
    return min(BIND_RETRY_BASE_S * (2 ** min(attempt, 4)), BIND_RETRY_CAP_S)


def is_retryable_bind_error(exc):
    """True only for the tailnet-not-ready signature (errno 99)."""
    return getattr(exc, "errno", None) == errno.EADDRNOTAVAIL


def bind_with_retry(make_server, budget_s=BIND_RETRY_BUDGET_S, sleep=None):
    """Call `make_server()` until it binds, the error is fatal, or the
    budget elapses. At least one attempt always runs."""
    if sleep is None:
        sleep = time.sleep
    deadline = time.monotonic() + budget_s
    attempt = 0
    while True:
        try:
            return make_server()
        except OSError as e:
            remaining = deadline - time.monotonic()
            if not is_retryable_bind_error(e) or remaining <= 0:
                raise
            delay = min(bind_retry_delay(attempt), remaining)
            print(
                f"laptop-agent: cannot bind {BIND_HOST}:{BIND_PORT} yet "
                f"({e}); retrying in {delay:g}s",
                flush=True,
            )
            sleep(delay)
            attempt += 1


def main():
    (STATE_DIR / "jobs").mkdir(parents=True, exist_ok=True)
    (STATE_DIR / "exec").mkdir(parents=True, exist_ok=True)
    load_persisted_jobs()
    reap_old_jobs()
    threading.Thread(target=_reaper_loop, daemon=True).start()
    srv = bind_with_retry(
        lambda: AgentServer((BIND_HOST, BIND_PORT), Handler, tls_context=TLS_CONTEXT)
    )
    mode = "TLS (dual-accept, Phase 1)" if TLS_CONTEXT is not None else "plaintext"
    print(f"laptop-agent listening on {BIND_HOST}:{BIND_PORT} ({mode})", flush=True)
    srv.serve_forever()


if __name__ == "__main__":
    main()
