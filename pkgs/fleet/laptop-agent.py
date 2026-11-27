#!/usr/bin/env python3
"""laptop-agent client — talk to the laptop-agent daemon (code#5343).

Primary transport (2026-11-15, code#10356): the named Cloudflare tunnel
— HTTPS to sandbox-link.security.cab through the platform egress proxy,
Cloudflare Access service-token headers from fleet secrets
(`cloudflare.access-laptop-agent`, Bitwarden-backed; migrated from the
0600 file 2026-11-16, Drew-approved). Fallback transport: the
VM's tailnet tunnel proxy (HTTP CONNECT, same construction as
laptop-proxy-connect.py; the proxy credential is read from the seat's
recorded tailnet proxy (fleet-proxy.env) when one exists, else from
HTTPS_PROXY, and sent only inside the CONNECT request), used automatically when the
Cloudflare path fails and directly for /exec calls whose budget exceeds
the Cloudflare edge's ~100 s response cap. The agent token lives in
~/tools/laptop-agent-token (mode 600) and is sent as a header — never
in argv. LAPTOP_AGENT_TRANSPORT=auto|cf|ts pins the preference.

Usage:
  laptop-agent.py health
  laptop-agent.py fetch [--chunk BYTES] PATH OUT
  laptop-agent.py exec [--cwd DIR] [--timeout S] [--shell] -- cmd [args...]
  laptop-agent.py exec-batch FILE.json     # {"cmds":[...], "timeout_s":N}
  laptop-agent.py run  [--cwd DIR] [--shell] -- cmd [args...]
  laptop-agent.py poll JOB [--wait S]
  laptop-agent.py kill JOB
  laptop-agent.py jobs

Timeout semantics (fixed 2026-11-10, code#5343): a caller's deadline
bounds the WHOLE call, at every layer.

  * `exec --timeout S` is enforced in the daemon (the remote process
    GROUP is killed at S — children and grandchildren included) and
    also locally: the client gives up at S + TRANSPORT_GRACE_S and
    exits 124 even if the transport underneath it has wedged.
  * An external `timeout N` (SIGTERM) or Ctrl-C (SIGINT) no longer
    orphans the remote command: the client catches the signal, asks
    the daemon to cancel the in-flight exec (or kill the detached
    job), waits briefly for the kill acknowledgement, and exits 124.
  * SIGKILL cannot be caught by any program; the daemon reaps an
    orphaned exec on its own when the request connection dies, and a
    detached job keeps running by design (kill it with `kill JOB`).
  * The broker never parks a caller behind a wedged pooled tunnel:
    a request whose connection dies mid-flight is dropped from the
    pool, and the client's wait on the broker socket is bounded by
    the same deadline, expiry of which restarts the broker.

Loud-failure semantics (silence audit, 2026-11-10, code#5343): a
failed call never looks like an empty success.

  * Transport failures on the direct path (proxy down, CONNECT
    refused, daemon unreachable) exit 1 with a named one-line
    `laptop-agent: transport error: ...` on stderr — no tracebacks.
  * Output cut at the daemon's 1 MiB/stream capture cap is announced
    on stderr (`output truncated ... (tail kept)`); it used to be
    dropped silently, so partial output read as complete.
  * `exec-batch` exits with the worst per-command exit code (spawn
    error counts as 1, timeout as 124); it used to always exit 0.
  * A brokered reply that took >10 s beyond the daemon's own
    execution time is announced on stderr as a transport-layer wait,
    so a parked caller is distinguishable from a long remote command.

SSH fallback (2026-11-16, code#2879): every agent transport — the
broker, the Cloudflare tunnel, the Tailscale CONNECT — converges on
the one laptop daemon, so a dead daemon used to fail every call even
though plain SSH to the laptop still worked. Now, when all agent
transports fail, `request()` retries the operation over real SSH
(~/tools/laptop-ssh-real.sh, the preserved pre-agent transport):
/exec and /exec-batch run the commands over SSH with the same result
shape, /health is synthesised from /proc, and detached jobs launched
during an outage get `sshfb-` ids spooled under the laptop's
~/.local/state/laptop-agent/sshfb-jobs/ and stay on the SSH path for
their whole life (poll/kill included), so run/poll semantics survive
the outage and never strand a job when the daemon returns. The
fallback fires only on transport failure (never on a daemon answer,
including daemon HTTP errors), is announced once on stderr, honours
the caller's deadline, and is disabled by a pinned
LAPTOP_AGENT_TRANSPORT (a pin is a deliberate path proof) or by
LAPTOP_AGENT_NO_SSH_FALLBACK=1.
"""

import json
import math
import os
import signal
import socket
import sys
import time
from pathlib import Path

# NOTE: http.client / base64 / urllib.parse / argparse are imported lazily
# inside the direct-CONNECT path and main(). The broker fast path (the
# fleet default) needs only the modules above — at dozens of concurrent
# sessions, per-call interpreter+import cost is the sandbox footprint.

TARGET_HOST = "100.99.113.103"
TARGET_PORT = 4799
PROXY_PORT = 3130
TOKEN_FILE = Path.home() / "tools/laptop-agent-token"
OFFSET_DIR = Path.home() / ".cache/laptop-agent"
TRIP_LOG = Path.home() / ".local/state/laptop-agent/trips.jsonl"
TIMEOUT_EXIT = 124  # same code GNU timeout(1) uses
TRANSPORT_GRACE_S = 15  # local deadline = remote timeout + this
EXTERNAL_DEADLINE_ENV = "LAPTOP_AGENT_DEADLINE_S"  # seconds from now;
# wrappers (the shim) export this when THEY run under an outer deadline
# so the deadline reaches the daemon too (it reaps the remote command
# at the same instant even if every signal path fails).
_CANCEL_BUDGET_S = 3.0  # time the signal handler may spend cancelling

# --- deadline / cancellation state (single-threaded client) -------------
_deadline = None  # monotonic deadline for the current call
_cancel_hook = None  # callable() -> bool, set while in flight
_interrupted = None  # first caught signal number, if any


class CallTimeout(Exception):
    """The local deadline fired, or an external signal interrupted the call."""


class TransportDown(Exception):
    """Every agent transport (broker, Cloudflare, Tailscale) failed —
    the trigger for the SSH fallback in request()."""


class _SSHFallbackFailed(Exception):
    """The SSH fallback itself could not carry the request."""


def _remaining():
    if _deadline is None:
        return None
    return _deadline - time.monotonic()


def _set_deadline(timeout_s):
    global _deadline
    if timeout_s is None:
        _deadline = None
    else:
        _deadline = time.monotonic() + timeout_s + TRANSPORT_GRACE_S


def _alarm_handler(signum, frame):
    raise CallTimeout()


def _interrupt_handler(signum, frame):
    global _interrupted
    if _interrupted is None:
        _interrupted = signum
    raise CallTimeout()


def _install_signal_handlers():
    # SIGALRM drives the local deadline backstop; SIGTERM/SIGINT are
    # caught so an external `timeout`/Ctrl-C can cancel the remote
    # command instead of orphaning it. SIGKILL is uncatchable — the
    # daemon's disconnect reaper covers that case for /exec.
    signal.signal(signal.SIGALRM, _alarm_handler)
    signal.signal(signal.SIGTERM, _interrupt_handler)
    signal.signal(signal.SIGINT, _interrupt_handler)


def _arm_deadline_timer():
    if _deadline is not None:
        signal.setitimer(signal.ITIMER_REAL, max(0.05, _deadline - time.monotonic()))


def _disarm_deadline_timer():
    try:
        signal.setitimer(signal.ITIMER_REAL, 0)
    except (ValueError, OSError):
        pass


def _cancel_in_flight(trip):
    """Best-effort remote cancel from inside a signal handler.

    Uses a RAW direct CONNECT with the tight _CANCEL_BUDGET_S budget —
    never the broker, whose wedged tunnel may be exactly what the
    caller is escaping. Returns True if the daemon acknowledged."""
    global _deadline
    saved = _deadline
    _deadline = None
    _disarm_deadline_timer()
    try:
        token = TOKEN_FILE.read_text().strip()
        # Cloudflare first (the primary path), Tailscale as fallback —
        # a cancel matters most exactly when the primary path is sick.
        attempts = []
        if _transport_pref() != "ts" and _cf_access_available():
            attempts.append((make_cf_http(), CF_HOST, 443, _cf_access_headers()))
        if _transport_pref() != "cf":
            attempts.append((make_tunnel_http(), TARGET_HOST, TARGET_PORT, {}))
        for make, host, port, extra in attempts:
            try:
                conn = make(host, port, timeout=_CANCEL_BUDGET_S)
                headers = {
                    "X-Agent-Token": token,
                    "X-Trip-Id": trip,
                    "Content-Type": "application/json",
                }
                headers.update(extra)
                conn.request(
                    "POST",
                    f"/exec/{trip}/cancel",
                    body=json.dumps({"trip": trip}),
                    headers=headers,
                )
                resp = conn.getresponse()
                resp.read()
                conn.close()
                return resp.status == 200
            except BaseException:
                # Includes tunnel_socket()'s SystemExit on a closed
                # CONNECT: a failed cancel attempt, nothing more —
                # this hook runs on the timeout path and must never
                # escalate the caller's exit.
                continue
        return False
    finally:
        _deadline = saved


def _log_trip(
    trip,
    path,
    total_ms,
    status,
    t_sent_ms=None,
    t_returned_ms=None,
    t_received_ms=None,
    transport=None,
):
    """Append one client-side trip record (single O_APPEND write).
    Logging must never break or visibly delay a call: errors swallowed.

    Trip logging v2 (code#5702): the line carries millisecond epochs
    t_sent / t_returned minted here, plus the daemon-echoed t_received
    when the response carried one, so a join on `trip` with the daemon
    line splits the trip into outbound / daemon / return legs. v1
    fields (ts, total_ms, status) are kept verbatim for v1 readers."""
    try:
        rec = {
            "v": 2,
            "ts": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
            "trip": trip,
            "path": path,
            "total_ms": total_ms,
            "status": status,
        }
        if transport is not None:
            rec["transport"] = transport
        if t_sent_ms is not None:
            rec["t_sent"] = t_sent_ms
        if t_returned_ms is not None:
            rec["t_returned"] = t_returned_ms
        if t_received_ms is not None:
            rec["t_received"] = t_received_ms
        TRIP_LOG.parent.mkdir(parents=True, exist_ok=True)
        fd = os.open(TRIP_LOG, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o644)
        try:
            os.write(fd, (json.dumps(rec) + "\n").encode())
        finally:
            os.close(fd)
    except Exception:
        pass


# --- recorded tailnet proxy (code#12426, mirroring #12645) -------------
# In userspace mode the seat bootstrap writes the tailnet proxy URL to
# fleet-proxy.env beside tailscaled's node state. Tailnet CONNECTs use
# that recorded proxy in place of the ambient HTTPS_PROXY: a process
# started from a sandbox shell carries the platform egress proxy there,
# and tailnet tunnels through it fail while the process looks healthy.
# The Cloudflare leg keeps the ambient proxy by design (it is the
# internet egress). A kernel-mode seat has no file (the bootstrap
# removes it), so the ambient env stays in charge there. The candidate
# order, slug rule and parse rule mirror fleet-config's
# tailnet_https_proxy (pkgs/fleet/crates/fleet-config/src/lib.rs).

TAILNET_PROXY_ENV_FILE = "fleet-proxy.env"


def _agent_slug(agent):
    """The bootstrap's agent slug: lower-cased, every character outside
    [a-z0-9-] replaced by '-', leading and trailing dashes trimmed, cut
    to 55 characters, then one 'muse-' prefix dropped. None when
    nothing is left, as the bootstrap refuses such a name."""
    mapped = "".join(
        c if ("a" <= c <= "z" or "0" <= c <= "9" or c == "-") else "-"
        for c in agent.lower()
    )
    slug = mapped.strip("-")[:55]
    if slug.startswith("muse-"):
        slug = slug[len("muse-") :]
    return slug or None


def _tailnet_proxy_env_candidates(home, fleet_agent, explicit):
    """Where to look for the recorded tailnet proxy env, most specific
    first: an explicit $FLEET_PROXY_ENV path, then the named agent's
    isolated node state (~/.local/state/tailscale/agents/<slug>), then
    the legacy shared node state (~/.local/state/tailscale). Pure over
    its inputs so the order is testable."""
    out = []
    if explicit:
        p = Path(explicit)
        out.append(p if p.is_absolute() else home / p)
    state = home / ".local" / "state" / "tailscale"
    if fleet_agent:
        slug = _agent_slug(fleet_agent)
        if slug:
            out.append(state / "agents" / slug / TAILNET_PROXY_ENV_FILE)
    out.append(state / TAILNET_PROXY_ENV_FILE)
    return out


def _parse_tailnet_proxy_env(text):
    """The HTTPS_PROXY value an env file sets, from a line of the form
    [export ]HTTPS_PROXY=value (the value optionally in matching single
    or double quotes). The last such line with a value wins, as it
    would when the file is sourced."""
    value = None
    for line in text.splitlines():
        line = line.strip()
        if line.startswith("export "):
            line = line[len("export ") :].lstrip()
        if not line.startswith("HTTPS_PROXY="):
            continue
        v = line[len("HTTPS_PROXY=") :].strip()
        if len(v) >= 2 and v[0] == v[-1] and v[0] in "\"'":
            v = v[1:-1]
        if v:
            value = v
    return value


def tailnet_https_proxy():
    """The seat's recorded tailnet proxy: the first readable
    fleet-proxy.env among the candidates that sets HTTPS_PROXY.
    None (no file, as on a kernel-mode seat) leaves the ambient env
    in charge."""
    for path in _tailnet_proxy_env_candidates(
        Path.home(),
        os.environ.get("FLEET_AGENT"),
        os.environ.get("FLEET_PROXY_ENV"),
    ):
        try:
            text = path.read_text()
        except OSError:
            continue
        value = _parse_tailnet_proxy_env(text)
        if value:
            return value
    return None


def tailnet_proxy_url():
    """The proxy URL for a tailnet CONNECT: the recorded tailnet proxy
    when the seat has one, else the ambient HTTPS_PROXY."""
    return tailnet_https_proxy() or os.environ.get("HTTPS_PROXY")


def tunnel_socket(port=None):
    import base64
    import urllib.parse

    port = port or TARGET_PORT
    raw = tailnet_proxy_url()
    if not raw:
        sys.exit("laptop-agent: HTTPS_PROXY must be set")
    u = urllib.parse.urlparse(raw)
    user = urllib.parse.unquote(u.username or "")
    pw = urllib.parse.unquote(u.password or "")
    connect_timeout = 60
    if _deadline is not None:
        connect_timeout = max(1.0, min(60.0, _remaining()))
    sock = socket.create_connection((u.hostname, PROXY_PORT), timeout=connect_timeout)
    auth = base64.b64encode(f"{user}:{pw}".encode()).decode()
    req = (
        f"CONNECT {TARGET_HOST}:{port} HTTP/1.1\r\n"
        f"Host: {TARGET_HOST}:{port}\r\n"
        f"Proxy-Authorization: Basic {auth}\r\n"
        f"Proxy-Connection: Keep-Alive\r\n\r\n"
    )
    sock.sendall(req.encode())
    buf = b""
    while b"\r\n\r\n" not in buf:
        chunk = sock.recv(1)
        if not chunk:
            sys.exit("laptop-agent: proxy closed during CONNECT")
        buf += chunk
    status = buf.split(b"\r\n", 1)[0].decode("latin-1")
    if " 200 " not in status and not status.endswith(" 200"):
        sys.exit(f"laptop-agent: CONNECT refused: {status}")
    sock.settimeout(None)
    return sock


def make_tunnel_http():
    import http.client

    class TunnelHTTP(http.client.HTTPConnection):
        def connect(self):
            self.sock = tunnel_socket()

    return TunnelHTTP


# --- Cloudflare transport (primary since 2026-11-15, code#10356) -------
# Drew's transport ruling (2026-11-15): sandbox -> laptop traffic rides
# the named Cloudflare tunnel (sandbox-link.security.cab, Cloudflare
# Access service-token gated) instead of the Tailscale session proxy;
# the Tailscale path above stays as the automatic fallback. The tunnel
# ingress lands on the daemon's own tailnet-IP listener from the laptop
# side, so the daemon is untouched and its X-Agent-Token auth is
# preserved end to end. The Access client id/secret live in fleet
# secrets (Bitwarden-backed `cloudflare.access-laptop-agent`, holding
# the credential's exact JSON) and are sent only as headers, never in
# argv.
# LAPTOP_AGENT_TRANSPORT=auto|cf|ts overrides the preference (cf/ts
# disable the fallback — used to prove each path on its own).
CF_HOST = "sandbox-link.security.cab"
CF_ACCESS_SECRET = "cloudflare.access-laptop-agent"
# Safety valve only: an /exec whose own budget exceeds this goes
# straight to the Tailscale path. The classic Cloudflare ~100 s proxy
# cap does NOT bite this tunnel in practice — wire-proven 2026-11-15
# (code#10356): a 105 s /exec completed over the Cloudflare path with
# status 200 — so the valve sits at the daemon's own 660 s ceiling
# and the Cloudflare path carries every normal request.
CF_EDGE_LIMIT_S = 660


_cf_access_data_cache = None
_cf_access_data_failed_at = 0.0


def _cf_access_data():
    """Parsed Cloudflare Access credential, fetched once per process
    from fleet secrets (broker-first, Bitwarden read-through) and
    cached. A failed fetch returns {} and is retried at most once a
    minute, so callers treat the Cloudflare leg as absent exactly as
    they did when the credential file was missing."""
    global _cf_access_data_cache, _cf_access_data_failed_at
    if _cf_access_data_cache is not None:
        return _cf_access_data_cache
    if time.monotonic() - _cf_access_data_failed_at < 60:
        return {}
    try:
        import subprocess

        out = subprocess.run(
            ["fleet", "secrets", "get", CF_ACCESS_SECRET],
            capture_output=True,
            text=True,
            timeout=130,
        )
        data = json.loads(out.stdout) if out.returncode == 0 else {}
        if not (
            isinstance(data, dict)
            and data.get("client_id")
            and data.get("client_secret")
        ):
            raise ValueError("incomplete Cloudflare Access credential")
        _cf_access_data_cache = data
    except Exception:
        _cf_access_data_failed_at = time.monotonic()
        return {}
    return _cf_access_data_cache


def _cf_access_available():
    return bool(_cf_access_data())


def _cf_access_headers():
    try:
        data = _cf_access_data()
        return {
            "CF-Access-Client-Id": data["client_id"],
            "CF-Access-Client-Secret": data["client_secret"],
        }
    except Exception:
        return {}


def cf_socket(timeout=60):
    """TLS socket to the daemon via the Cloudflare tunnel: CONNECT to
    CF_HOST:443 through the platform egress proxy (HTTPS_PROXY's own
    host:port — NOT the 3130 tailnet session proxy), then TLS with
    normal certificate verification. Raises OSError on any failure so
    callers can fall back to the Tailscale path."""
    import base64
    import ssl
    import urllib.parse

    raw = os.environ.get("HTTPS_PROXY")
    if not raw:
        raise OSError("HTTPS_PROXY must be set")
    u = urllib.parse.urlparse(raw)
    user = urllib.parse.unquote(u.username or "")
    pw = urllib.parse.unquote(u.password or "")
    sock = socket.create_connection((u.hostname, u.port or 3128), timeout=timeout)
    try:
        sock.settimeout(timeout)
        auth = base64.b64encode(f"{user}:{pw}".encode()).decode()
        req = (
            f"CONNECT {CF_HOST}:443 HTTP/1.1\r\n"
            f"Host: {CF_HOST}:443\r\n"
            f"Proxy-Authorization: Basic {auth}\r\n"
            f"Proxy-Connection: Keep-Alive\r\n\r\n"
        )
        sock.sendall(req.encode())
        buf = b""
        while b"\r\n\r\n" not in buf:
            chunk = sock.recv(1)
            if not chunk:
                raise OSError("proxy closed during CONNECT")
            buf += chunk
            if len(buf) > 65536:
                raise OSError("CONNECT response too large")
        status = buf.split(b"\r\n", 1)[0].decode("latin-1")
        if " 200 " not in status and not status.endswith(" 200"):
            raise OSError(f"CONNECT refused: {status}")
        ctx = ssl.create_default_context()
        tls = ctx.wrap_socket(sock, server_hostname=CF_HOST)
        tls.settimeout(timeout)
        return tls
    except Exception:
        try:
            sock.close()
        except OSError:
            pass
        raise


def make_cf_http():
    import http.client

    class CFHTTPS(http.client.HTTPSConnection):
        def connect(self):
            self.sock = cf_socket(timeout=min(self.timeout or 60, 60))
            if self.timeout:
                self.sock.settimeout(self.timeout)

    return CFHTTPS


def _transport_pref():
    """LAPTOP_AGENT_TRANSPORT: auto (default) | cf | ts."""
    v = os.environ.get("LAPTOP_AGENT_TRANSPORT", "auto").lower()
    return v if v in ("auto", "cf", "ts") else "auto"


def _cf_suitable(path, body):
    """Cloudflare is unsuitable only for a long /exec (edge cap)."""
    if path == "/exec" and isinstance(body, dict):
        ts = body.get("timeout_s")
        if isinstance(ts, (int, float)) and ts > CF_EDGE_LIMIT_S:
            return False
    return True


BROKER_SOCK = OFFSET_DIR / "broker.sock"
BROKER_PY = Path.home() / "tools/laptop-agent-broker.py"
BROKER_PID = OFFSET_DIR / "broker.pid"


def _restart_broker():
    """Kill the broker process so the next call auto-starts a fresh one
    with a clean pool (used when a deadline fires while a request is
    parked on the broker — its thread may still be holding a tunnel)."""
    try:
        pid = int(BROKER_PID.read_text().strip())
        os.kill(pid, signal.SIGKILL)
    except Exception:
        pass
    try:
        BROKER_SOCK.unlink()
    except OSError:
        pass


def _broker_roundtrip(method, path, body, trip=None):
    """One request through the warm-connection broker. Returns
    (status, obj) or None when the broker is unreachable. The wait on
    the broker socket is bounded by the call deadline (if any)."""
    import struct
    import subprocess

    for attempt in (1, 2):
        s = None
        t_start = time.time()
        try:
            s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            wait = _remaining()
            s.settimeout(660 if wait is None else max(0.05, wait))
            s.connect(str(BROKER_SOCK))
            msg = {
                "method": method,
                "path": path,
                "body": body,
                "trip": trip,
                "transport": _transport_pref(),
                "cf_ok": _cf_suitable(path, body),
            }
            rem = _remaining()
            if rem is not None:
                msg["deadline_s"] = rem
            payload = json.dumps(msg).encode()
            s.sendall(struct.pack(">Q", len(payload)) + payload)
            hdr = b""
            while len(hdr) < 8:
                chunk = s.recv(8 - len(hdr))
                if not chunk:
                    raise ConnectionError("broker closed")
                hdr += chunk
            (n,) = struct.unpack(">Q", hdr)
            data = b""
            while len(data) < n:
                chunk = s.recv(min(1 << 20, n - len(data)))
                if not chunk:
                    raise ConnectionError("broker closed mid-reply")
                data += chunk
            s.close()
            obj = json.loads(data)
            # A reply wait far beyond a warm round trip (~0.4 s) with
            # no daemon-side execution to explain it means the caller
            # sat parked in the transport — a wedged broker thread, a
            # tunnel reconnect, or (historically) a permission wait.
            # Surface it once, on stderr, so the wait is never
            # mistaken for the remote command running (code#5343,
            # 2026-11-10). /exec replies carry per-command duration,
            # which is subtracted: a long REMOTE command is not a
            # transport wait and must not be labelled one.
            waited = time.time() - t_start
            if waited > 10:
                remote_ms = None
                try:
                    results = obj["body"].get("results") or []
                    remote_ms = sum(r.get("duration_ms") or 0 for r in results)
                except Exception:
                    remote_ms = None
                if remote_ms is None or waited * 1000 > remote_ms + 5000:
                    sys.stderr.write(
                        f"laptop-agent: note: this call waited "
                        f"{waited:.0f}s for its reply, beyond the "
                        f"time the laptop spent executing — the wait "
                        f"was in the broker/tunnel layer\n"
                    )
            return obj["status"], obj["body"], obj.get("transport")
        except CallTimeout:
            if s is not None:
                try:
                    s.close()
                except OSError:
                    pass
            raise
        except socket.timeout:
            if s is not None:
                try:
                    s.close()
                except OSError:
                    pass
            if _deadline is not None and _remaining() <= 0:
                raise CallTimeout()
            if attempt == 1:
                continue
            return None
        except (OSError, ConnectionError, KeyError, ValueError):
            if s is not None:
                try:
                    s.close()
                except OSError:
                    pass
            if attempt == 1 and not os.environ.get("LAPTOP_AGENT_NO_BROKER"):
                # Auto-start the broker (ControlMaster pattern), then retry.
                OFFSET_DIR.mkdir(parents=True, exist_ok=True)
                log = open(OFFSET_DIR / "broker.log", "ab")
                subprocess.Popen(
                    [sys.executable, str(BROKER_PY)],
                    stdout=log,
                    stderr=log,
                    start_new_session=True,
                )
                for _ in range(50):
                    if BROKER_SOCK.exists():
                        break
                    time.sleep(0.1)
                continue
            return None
    return None


def _agent_request(method, path, body=None):
    global _cancel_hook
    trip = os.urandom(6).hex()  # 12 hex chars, one per request
    if method == "POST" and isinstance(body, dict) and path in ("/exec", "/jobs"):
        # The trip id also rides inside the body: the session proxy
        # forwards bodies verbatim but not arbitrary socket-protocol
        # fields, so this is the carrier that always reaches the daemon.
        body = dict(body, trip=trip)
    cancellable = method == "POST" and path == "/exec"
    t0 = time.time()
    t_sent_ms = int(t0 * 1000)
    t_received_ms = None
    status = None
    transport_used = None
    if cancellable:
        _cancel_hook = lambda trip=trip: _cancel_in_flight(trip)
    try:
        _arm_deadline_timer()
        if not os.environ.get("LAPTOP_AGENT_NO_BROKER"):
            got = _broker_roundtrip(method, path, body, trip)
            if got is not None:
                status, obj, transport_used = got
                if (
                    status == 502
                    and isinstance(obj, dict)
                    and str(obj.get("error", "")).startswith("broker:")
                ):
                    # The broker itself failed the request (tunnel
                    # establishment failure, wedged pool) — that is a
                    # transport failure, not a daemon answer. Fall
                    # through to the direct path; if that also fails,
                    # request() escalates to the SSH fallback.
                    status, obj = None, None
                elif status >= 400:
                    sys.exit(f"laptop-agent: HTTP {status}: {obj.get('error', obj)}")
                if obj is not None:
                    if isinstance(obj, dict):
                        # Trip v2: strip the daemon's internal
                        # t_received echo before the result reaches
                        # callers (code#5702).
                        t_received_ms = obj.pop("_t_received", None)
                    return obj
        token = TOKEN_FILE.read_text().strip()
        wait = _remaining()
        timeout = 660 if wait is None else max(1.0, wait)
        payload = json.dumps(body) if body is not None else None

        def _attempt(transport):
            if transport == "cf":
                conn = make_cf_http()(CF_HOST, 443, timeout=timeout)
                headers = {
                    "X-Agent-Token": token,
                    "X-Trip-Id": trip,
                    "Content-Type": "application/json",
                }
                headers.update(_cf_access_headers())
            else:
                conn = make_tunnel_http()(TARGET_HOST, TARGET_PORT, timeout=timeout)
                headers = {
                    "X-Agent-Token": token,
                    "X-Trip-Id": trip,
                    "Content-Type": "application/json",
                }
            conn.request(method, path, body=payload, headers=headers)
            resp = conn.getresponse()
            st = resp.status
            data = resp.read()
            conn.close()
            return st, json.loads(data or b"{}")

        pref = _transport_pref()
        order = []
        if pref != "ts" and _cf_suitable(path, body) and _cf_access_available():
            order.append("cf")
        if pref != "cf":
            order.append("ts")
        if not order:
            order = ["ts"]
        last_err = None
        obj = None
        for transport in order:
            try:
                status, obj = _attempt(transport)
                transport_used = transport
                # The edge answering 502/503/530 means the tunnel
                # origin is down: fall back to Tailscale for this call.
                if (
                    transport == "cf"
                    and status in (502, 503, 530)
                    and transport != order[-1]
                ):
                    last_err = OSError(f"cloudflare edge status {status}")
                    status, obj = None, None
                    continue
                break
            except CallTimeout:
                raise
            except SystemExit as e:
                # tunnel_socket() sys.exits on a closed/refused
                # CONNECT — SystemExit is a BaseException, invisible
                # to `except Exception`, and it used to escape
                # request() entirely, masking the failure as a bare
                # proxy message and bypassing every fallback. It is
                # a transport failure like any other: convert it.
                last_err = OSError(str(e.code))
                status, obj = None, None
                continue
            except Exception as e:
                last_err = e
                status, obj = None, None
                continue
        if obj is None:
            # Transport failure on every agent path (proxy down,
            # CONNECT refused, daemon unreachable). Raised, not
            # exited: request() turns this into the SSH fallback, or
            # into the loud one-line error when the fallback is
            # disabled or also fails (code#5343 silence audit,
            # 2026-11-10; fallback added 2026-11-16, code#2879).
            raise TransportDown(f"{type(last_err).__name__}: {last_err}")
        if status >= 400:
            sys.exit(f"laptop-agent: HTTP {status}: {obj.get('error', obj)}")
        if isinstance(obj, dict):
            t_received_ms = obj.pop("_t_received", None)
        return obj
    finally:
        _disarm_deadline_timer()
        if cancellable:
            _cancel_hook = None
        _log_trip(
            trip,
            path,
            int((time.time() - t0) * 1000),
            status,
            t_sent_ms=t_sent_ms,
            t_returned_ms=int(time.time() * 1000),
            t_received_ms=t_received_ms,
            transport=transport_used,
        )


# --- SSH fallback transport (2026-11-16, code#2879) -------------------
# All agent transports converge on the single laptop daemon; when it
# dies, plain SSH still works (separate sshd, separate path). This
# layer re-implements the daemon's request surface over
# ~/tools/laptop-ssh-real.sh (the preserved pre-agent SSH transport:
# pinned host key, CONNECT via the egress proxy, ControlMaster mux —
# a warm master makes fallback calls ~0.4 s). It fires ONLY from
# request() after every agent transport failed, or for sshfb- jobs,
# which it owns for their whole life.
SSH_REAL = Path.home() / "tools/laptop-ssh-real.sh"
SSHFB_PREFIX = "sshfb-"
SSHFB_REMOTE_DIR = ".local/state/laptop-agent/sshfb-jobs"
_SSH_MAGIC = "__LAPTOP_AGENT_SSHFB_V1__"
_EXEC_CAP = 1 << 20  # per-stream capture cap, as the daemon's


def _ssh_fallback_allowed():
    if os.environ.get("LAPTOP_AGENT_NO_SSH_FALLBACK"):
        return False
    if not SSH_REAL.exists():
        return False
    # A pinned transport is a deliberate path proof (proving the CF or
    # TS leg on its own) — the fallback must never mask it.
    return _transport_pref() == "auto"


def _is_sshfb_path(path):
    if not path.startswith("/jobs/"):
        return False
    return path.split("/")[2].startswith(SSHFB_PREFIX)


def _ssh_run(remote, timeout_s):
    """Run one command string on the laptop over real SSH. Returns
    (rc, stdout_bytes, stderr_bytes); rc is None when the local wait
    timed out (the ssh process group was killed; the remote command
    may outlive it — plain SSH has no remote cancel, the documented
    limitation of this path). Raises _SSHFallbackFailed when the
    remote shell never started (magic line missing: proxy down,
    sshd down, auth failure) and CallTimeout when the caller's own
    deadline/signal fires mid-wait."""
    import subprocess

    argv = [str(SSH_REAL), f"echo {_SSH_MAGIC}; " + remote]
    try:
        proc = subprocess.Popen(
            argv,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            start_new_session=True,
        )
    except OSError as e:
        raise _SSHFallbackFailed(f"cannot launch ssh: {e}")
    try:
        out, err = proc.communicate(timeout=timeout_s)
        rc = proc.returncode
    except subprocess.TimeoutExpired:
        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except OSError:
            pass
        out, err = proc.communicate()
        rc = None
    except CallTimeout:
        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except OSError:
            pass
        try:
            proc.communicate(timeout=5)
        except Exception:
            pass
        raise
    marker = (_SSH_MAGIC + "\n").encode()
    idx = out.find(marker)
    if idx < 0:
        raise _SSHFallbackFailed(
            f"remote shell never started (rc={rc}): "
            f"{err.decode('utf-8', 'replace').strip()[:200]}"
        )
    return rc, out[idx + len(marker) :], err


def _ssh_inner(spec):
    """Flatten a {cwd, argv|shell, env?} spec into one command string,
    mirroring the daemon's shell mode (bash -lc happens one level up,
    in the remote string the caller builds)."""
    import shlex

    inner = ""
    env = spec.get("env") or {}
    if env:
        inner += (
            "; ".join(f"export {k}={shlex.quote(str(v))}" for k, v in env.items())
            + "; "
        )
    if spec.get("cwd"):
        inner += f"cd {shlex.quote(str(spec['cwd']))} || exit 126; "
    if spec.get("shell"):
        inner += str(spec["shell"])
    else:
        inner += shlex.join([str(a) for a in spec.get("argv") or []])
    return inner


def _cap_tail(data):
    if len(data) > _EXEC_CAP:
        return data[-_EXEC_CAP:], True
    return data, False


def _ssh_exec_one(spec, timeout_s):
    import shlex

    remote = f"exec bash -lc {shlex.quote(_ssh_inner(spec))}"
    t0 = time.time()
    rc, out, err = _ssh_run(remote, timeout_s + 15)
    duration_ms = int((time.time() - t0) * 1000)
    out, out_trunc = _cap_tail(out)
    err, err_trunc = _cap_tail(err)
    result = {
        "stdout": out.decode("utf-8", "replace"),
        "stderr": err.decode("utf-8", "replace"),
        "exit": rc,
        "duration_ms": duration_ms,
        "truncated": out_trunc or err_trunc,
    }
    if rc is None:
        result["timeout"] = True
    return result


def _ssh_job_dir(job_id):
    return f"$HOME/{SSHFB_REMOTE_DIR}/{job_id}"


def _ssh_launch_job(spec):
    import shlex

    job_id = SSHFB_PREFIX + os.urandom(6).hex()
    d = _ssh_job_dir(job_id)
    script = (
        f"echo $$ > {d}/pid; date +%s > {d}/start; "
        f"bash -lc {shlex.quote(_ssh_inner(spec))} "
        f"> {d}/out 2> {d}/err; echo $? > {d}/exit"
    )
    remote = (
        f'd={d}; mkdir -p "$d" && '
        f"setsid bash -c {shlex.quote(script)} "
        f"</dev/null >/dev/null 2>&1 & echo launched"
    )
    _ssh_run(remote, 30)
    return {"job_id": job_id}


def _ssh_poll_job(job_id, query):
    import base64
    import urllib.parse

    qs = urllib.parse.parse_qs(query)
    out_off = int(qs.get("out_off", ["0"])[0] or 0)
    err_off = int(qs.get("err_off", ["0"])[0] or 0)
    d = _ssh_job_dir(job_id)
    remote = (
        f"d={d}; "
        f'os=$(stat -c%s "$d/out" 2>/dev/null || echo 0); '
        f'es=$(stat -c%s "$d/err" 2>/dev/null || echo 0); '
        f'start=$(cat "$d/start" 2>/dev/null || date +%s); '
        f'if [ -f "$d/exit" ]; then st=done; ex=$(cat "$d/exit"); '
        f'fin=$(stat -c%Y "$d/exit"); else st=running; ex=; '
        f"fin=$(date +%s); fi; "
        f'echo "STATE $st"; echo "EXIT $ex"; '
        f'echo "DUR $(( (fin - start) * 1000 ))"; '
        f'echo "OUTNEXT $os"; echo "ERRNEXT $es"; '
        f"printf 'OUTB64 '; tail -c +$(( {out_off} + 1 )) \"$d/out\" "
        f"2>/dev/null | base64 -w0; echo; "
        f"printf 'ERRB64 '; tail -c +$(( {err_off} + 1 )) \"$d/err\" "
        f"2>/dev/null | base64 -w0; echo"
    )
    _, out, _ = _ssh_run(remote, 60)
    fields = {}
    for line in out.decode("utf-8", "replace").splitlines():
        key, _, rest = line.partition(" ")
        fields[key] = rest
    if "STATE" not in fields:
        raise _SSHFallbackFailed(f"unparsable poll reply for {job_id}")
    exit_v = fields.get("EXIT", "")
    return {
        "job_id": job_id,
        "state": fields["STATE"],
        "exit": int(exit_v) if exit_v.lstrip("-").isdigit() else None,
        "stdout": base64.b64decode(fields.get("OUTB64", "")).decode("utf-8", "replace"),
        "stderr": base64.b64decode(fields.get("ERRB64", "")).decode("utf-8", "replace"),
        "out_next": int(fields.get("OUTNEXT", out_off)),
        "err_next": int(fields.get("ERRNEXT", err_off)),
        "duration_ms": int(fields.get("DUR", 0)),
    }


def _ssh_kill_job(job_id):
    d = _ssh_job_dir(job_id)
    remote = (
        f'd={d}; pg=$(cat "$d/pid" 2>/dev/null); '
        f'if [ -n "$pg" ]; then '
        f"kill -TERM -$pg 2>/dev/null; sleep 2; "
        f"kill -KILL -$pg 2>/dev/null; fi; "
        f'( set -C; echo -15 > "$d/exit" ) 2>/dev/null; echo ok'
    )
    _ssh_run(remote, 30)
    return {"ok": True, "job_id": job_id}


def _ssh_list_jobs():
    remote = (
        f"for d in $HOME/{SSHFB_REMOTE_DIR}/*/; do "
        f'[ -d "$d" ] || continue; id=$(basename "$d"); '
        f'if [ -f "$d/exit" ]; then echo "$id done '
        f'$(cat "$d/exit")"; else echo "$id running"; fi; '
        f"done"
    )
    _, out, _ = _ssh_run(remote, 30)
    jobs = []
    for line in out.decode("utf-8", "replace").splitlines():
        parts = line.split()
        if len(parts) >= 2:
            jobs.append(
                {
                    "job_id": parts[0],
                    "state": parts[1],
                    "exit": int(parts[2])
                    if len(parts) > 2 and parts[2].lstrip("-").isdigit()
                    else None,
                }
            )
    return {"jobs": jobs}


def _ssh_health():
    remote = (
        "head -1 /proc/loadavg; head -1 /proc/uptime; "
        f"ls $HOME/.local/state/laptop-agent/jobs "
        f"$HOME/{SSHFB_REMOTE_DIR} 2>/dev/null | wc -l"
    )
    _, out, _ = _ssh_run(remote, 30)
    lines = out.decode("utf-8", "replace").splitlines()
    try:
        load1 = float(lines[0].split()[0])
        uptime_s = int(float(lines[1].split()[0]))
        jobs = int(lines[2].split()[0])
    except (IndexError, ValueError) as e:
        raise _SSHFallbackFailed(f"unparsable health reply: {e}")
    return {
        "ok": True,
        "uptime_s": uptime_s,
        "jobs": jobs,
        "load1": load1,
        "transport": "ssh-fallback",
    }


def _ssh_request(method, path, body=None):
    """The daemon's request surface over SSH. Raises
    _SSHFallbackFailed for anything it cannot carry."""
    if method == "GET" and path == "/health":
        return _ssh_health()
    if method == "POST" and path == "/exec":
        body = body or {}
        timeout_s = body.get("timeout_s", 120)
        return {
            "results": [_ssh_exec_one(spec, timeout_s) for spec in body.get("cmds", [])]
        }
    if method == "POST" and path == "/jobs":
        _enforce_launch_guard(body or {})
        return _ssh_launch_job(body or {})
    if method == "GET" and path == "/jobs":
        return _ssh_list_jobs()
    if path.startswith("/jobs/"):
        rest = path[len("/jobs/") :]
        job_id = rest.split("/")[0].partition("?")[0]
        _, _, query = rest.partition("?")
        if method == "GET":
            return _ssh_poll_job(job_id, query)
        if method == "POST" and rest.rstrip("/").endswith("/kill"):
            return _ssh_kill_job(job_id)
    raise _SSHFallbackFailed(f"no ssh fallback for {method} {path}")


def request(method, path, body=None):
    """One daemon request: the agent transports first (broker, then
    direct Cloudflare/Tailscale); on total transport failure, the SSH
    fallback — automatic, transparent, same result shape. Jobs with
    sshfb- ids live on the SSH path from birth and never switch."""
    if _is_sshfb_path(path):
        return _ssh_request(method, path, body)
    try:
        return _agent_request(method, path, body)
    except TransportDown as down:
        if _deadline is not None and _remaining() <= 0:
            raise CallTimeout()
        if not _ssh_fallback_allowed():
            sys.exit(f"laptop-agent: transport error: {down}")
        sys.stderr.write(
            f"laptop-agent: note: agent transport down ({down}); "
            f"retrying over ssh fallback\n"
        )
        try:
            _arm_deadline_timer()
            return _ssh_request(method, path, body)
        except _SSHFallbackFailed as fb:
            sys.exit(
                f"laptop-agent: transport error: {down}; ssh fallback also failed: {fb}"
            )
        finally:
            _disarm_deadline_timer()


def _direct_request(method, path, body=None, timeout_s=30):
    """One request over a fresh direct CONNECT, bypassing the broker.
    Used for job kills issued from the signal path, where the broker
    may be the wedged layer. Honours the timeout strictly."""
    if _is_sshfb_path(path):
        return _ssh_request(method, path, body)
    trip = os.urandom(6).hex()
    if method == "POST" and isinstance(body, dict) and path == "/jobs":
        body = dict(body, trip=trip)
    token = TOKEN_FILE.read_text().strip()
    payload = json.dumps(body) if body is not None else None

    def _attempt(transport):
        if transport == "cf":
            conn = make_cf_http()(CF_HOST, 443, timeout=timeout_s)
            headers = {
                "X-Agent-Token": token,
                "X-Trip-Id": trip,
                "Content-Type": "application/json",
            }
            headers.update(_cf_access_headers())
        else:
            conn = make_tunnel_http()(TARGET_HOST, TARGET_PORT, timeout=timeout_s)
            headers = {
                "X-Agent-Token": token,
                "X-Trip-Id": trip,
                "Content-Type": "application/json",
            }
        conn.request(method, path, body=payload, headers=headers)
        resp = conn.getresponse()
        data = resp.read()
        conn.close()
        return resp.status, json.loads(data or b"{}")

    pref = _transport_pref()
    order = []
    if pref != "ts" and _cf_access_available():
        order.append("cf")
    if pref != "cf":
        order.append("ts")
    if not order:
        order = ["ts"]
    last_err = None
    status, obj = None, None
    for transport in order:
        try:
            status, obj = _attempt(transport)
            if (
                transport == "cf"
                and status in (502, 503, 530)
                and transport != order[-1]
            ):
                last_err = OSError(f"cloudflare edge status {status}")
                status, obj = None, None
                continue
            break
        except SystemExit as e:
            # See _agent_request: tunnel_socket()'s sys.exit is a
            # transport failure, not a verdict.
            last_err = OSError(str(e.code))
            status, obj = None, None
        except Exception as e:
            last_err = e
            status, obj = None, None
    if obj is None:
        raise RuntimeError(f"transport error: {type(last_err).__name__}: {last_err}")
    if status >= 400:
        raise RuntimeError(f"HTTP {status}: {obj.get('error', obj)}")
    if isinstance(obj, dict):
        obj.pop("_t_received", None)  # trip v2 internal echo (code#5702)
    return obj


def _fetch_chunk(path, off, length, timeout_s=120):
    """One GET /file chunk over a fresh direct transport (the broker
    protocol is JSON-only, so raw bytes bypass it — same transports
    and order as _direct_request). Returns (data_bytes, total_size).
    On total transport failure, falls back to one SSH chunk (dd +
    base64) when the SSH fallback is allowed, mirroring request()."""
    import urllib.parse

    qs = urllib.parse.urlencode({"path": path, "off": off, "len": length})
    route = f"/file?{qs}"
    token = TOKEN_FILE.read_text().strip()

    def _attempt(transport):
        if transport == "cf":
            conn = make_cf_http()(CF_HOST, 443, timeout=timeout_s)
            headers = {"X-Agent-Token": token}
            headers.update(_cf_access_headers())
        else:
            conn = make_tunnel_http()(TARGET_HOST, TARGET_PORT, timeout=timeout_s)
            headers = {"X-Agent-Token": token}
        conn.request("GET", route, headers=headers)
        resp = conn.getresponse()
        data = resp.read()
        hdrs = {k.lower(): v for k, v in resp.getheaders()}
        conn.close()
        return resp.status, data, hdrs

    pref = _transport_pref()
    order = []
    if pref != "ts" and _cf_access_available():
        order.append("cf")
    if pref != "cf":
        order.append("ts")
    if not order:
        order = ["ts"]
    last_err = None
    for transport in order:
        try:
            status, data, hdrs = _attempt(transport)
            if (
                transport == "cf"
                and status in (502, 503, 530)
                and transport != order[-1]
            ):
                last_err = OSError(f"cloudflare edge status {status}")
                continue
            if status >= 400:
                raise RuntimeError(
                    f"HTTP {status}: {data[:200].decode('utf-8', 'replace')}"
                )
            return data, int(hdrs.get("x-file-size", off + len(data)))
        except SystemExit as e:
            last_err = OSError(str(e.code))
        except Exception as e:
            last_err = e
    if _ssh_fallback_allowed():
        import base64
        import shlex

        remote = (
            f"sz=$(stat -c%s {shlex.quote(path)} 2>/dev/null) || exit 1; "
            f'echo "SIZE $sz"; '
            f"dd if={shlex.quote(path)} bs=1 skip={off} "
            f"count={length} 2>/dev/null | base64 -w0; echo"
        )
        rc, out, _err = _ssh_run(remote, timeout_s + 15)
        lines = out.decode("utf-8", "replace").splitlines()
        if rc == 0 and lines and lines[0].startswith("SIZE "):
            size = int(lines[0].split()[1])
            return base64.b64decode(lines[1] if len(lines) > 1 else ""), size
        raise RuntimeError(f"ssh fallback fetch failed (rc={rc})")
    raise RuntimeError(f"transport error: {type(last_err).__name__}: {last_err}")


def split_cmd(args):
    """Everything after -- is the command."""
    if "--" in args:
        i = args.index("--")
        return args[:i], args[i + 1 :]
    return args, []


def cmd_spec(opt_args, cmd_args, cwd, shell):
    if not cmd_args:
        sys.exit("laptop-agent: no command given (use -- cmd...)")
    if shell:
        return {"cwd": cwd, "shell": " ".join(cmd_args)}
    return {"cwd": cwd, "argv": cmd_args}


# --- primary-checkout guard (code#10095, launcher half of #9849) -------
# Defense in depth for the worktree rule: refuse an exec whose target
# directory is a fleet repo's PRIMARY checkout on the laptop and whose
# command mutates git state there (checkout/switch/commit/clean/reset/
# stash/...). Read-only work in a primary (status, log, the sentinel's
# checks, pm-git-sync's observers) stays allowed, as does `git worktree
# add` FROM a primary — the sanctioned way fleet worktrees are created.
# The launch-path half of #10095 item 1 (refusing a job launch whose
# working directory IS the primary) is the launch guard further down;
# both halves only see the calls that pass through this client.
# Drew-ordered work in a
# primary (the charter's parked-dirt exception) sets
# LAPTOP_AGENT_PRIMARY_OK=1 for that call.
_PRIMARY_CHECKOUTS = (
    "/home/user/git/code",
    "/home/user/git/opencode",
    "/home/user/git/openchamber-fork",
    "/home/user/code/dezren39-nix",
)
_GIT_MUTATION_RE = None  # compiled lazily in _primary_guard_check


def _normalize_laptop_path(token):
    token = token.strip().strip("'\"").rstrip("/")
    if token in ("~", "$HOME"):
        return "/home/user"
    for prefix in ("~/", "$HOME/"):
        if token.startswith(prefix):
            return "/home/user/" + token[len(prefix) :]
    return token


def _primary_guard_check(spec):
    """Return a refusal string if `spec` targets a primary checkout with
    a git mutation, else None. Conservative by design: a mutation with
    no provable primary target (no --cwd, no cd, no git -C) is allowed —
    proving the session's directory is the launch path's job (#10095
    item 1), and a false refusal here would block legitimate fleet work.
    """
    import re

    global _GIT_MUTATION_RE
    if _GIT_MUTATION_RE is None:
        _GIT_MUTATION_RE = re.compile(
            r"\bgit\s+(?:-C\s+\S+\s+)?"
            r"(checkout|switch|commit|clean|reset|stash|rebase|merge|"
            r"cherry-pick|apply|restore|rm|mv)\b"
        )
    if os.environ.get("LAPTOP_AGENT_PRIMARY_OK") == "1":
        return None
    text = spec.get("shell") or " ".join(spec.get("argv") or [])
    if not _GIT_MUTATION_RE.search(text):
        return None
    targets = []
    if spec.get("cwd"):
        targets.append(spec["cwd"])
    targets += re.findall(r"(?:^|&&|;|\|\|)\s*cd\s+(\S+)", text)
    targets += re.findall(r"\bgit\s+-C\s+(\S+)", text)
    for target in targets:
        normalized = _normalize_laptop_path(target)
        if normalized in _PRIMARY_CHECKOUTS:
            return (
                f"target directory {target} is the primary checkout "
                f"{normalized} and the command mutates git state there. "
                "Work in a worktree under ~/git/fleet/ instead (create "
                "one with: git -C <primary> worktree add "
                "~/git/fleet/<name> -b <branch>). For Drew-ordered work "
                "in the primary itself, set LAPTOP_AGENT_PRIMARY_OK=1."
            )
    return None


def _enforce_primary_guard(spec):
    refusal = _primary_guard_check(spec)
    if refusal is not None:
        sys.stderr.write(
            f"laptop-agent: REFUSED (primary-checkout guard, code#10095): {refusal}\n"
        )
        sys.exit(3)


# --- primary-checkout LAUNCH guard (code#10095 item 1, launcher half) --
# A detached job (`run`) is a launched session of work: its working
# directory is where that session lives. Launching one whose working
# directory IS a primary checkout is refused here — the launch path
# this client controls — regardless of whether the job's commands
# mutate git state (that per-command check is item 2, above). The
# refusal names the reason and the worktree relocation; every refusal
# is also appended to ~/.cache/laptop-agent/primary-guard.log so the
# launch record exists even when the caller keeps no record of its
# own. One-off `exec` calls are NOT launches and stay under item 2
# only, so read-only primary observers (sentinel checks, pm-git-sync)
# keep working. Drew-ordered work in a primary sets
# LAPTOP_AGENT_PRIMARY_OK=1 for that call, as with item 2.
_LAUNCH_GUARD_LOG = os.path.expanduser("~/.cache/laptop-agent/primary-guard.log")


def _launch_guard_check(spec):
    """Return a refusal string if a job launch's working directory
    resolves to a primary checkout, else None. The working directory
    is the spec cwd, or — for a shell job with no cwd — a leading
    `cd` target (the directory its first command moves to)."""
    import re

    if os.environ.get("LAPTOP_AGENT_PRIMARY_OK") == "1":
        return None
    targets = []
    if spec.get("cwd"):
        targets.append(spec["cwd"])
    elif spec.get("shell"):
        m = re.match(r"\s*cd\s+(\S+)", spec["shell"])
        if m:
            targets.append(m.group(1))
    for target in targets:
        normalized = _normalize_laptop_path(target)
        if normalized in _PRIMARY_CHECKOUTS:
            return (
                f"launch working directory {target} is the primary "
                f"checkout {normalized}. Sessions/jobs are never "
                "launched in a primary checkout: relocate the launch "
                "to a worktree under ~/git/fleet/ (create one with: "
                "git -C <primary> worktree add ~/git/fleet/<name> "
                "-b <branch>) and pass that as --cwd. For "
                "Drew-ordered work in the primary itself, set "
                "LAPTOP_AGENT_PRIMARY_OK=1."
            )
    return None


def _enforce_launch_guard(spec):
    refusal = _launch_guard_check(spec)
    if refusal is None:
        return
    try:
        os.makedirs(os.path.dirname(_LAUNCH_GUARD_LOG), exist_ok=True)
        with open(_LAUNCH_GUARD_LOG, "a") as fh:
            fh.write(
                json.dumps(
                    {
                        "at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
                        "verb": "run",
                        "cwd": spec.get("cwd"),
                        "refusal": refusal,
                    }
                )
                + "\n"
            )
    except Exception:
        pass
    sys.stderr.write(
        f"laptop-agent: REFUSED (primary-checkout launch guard, "
        f"code#10095): {refusal}\n"
    )
    sys.exit(3)


def _exit_timeout():
    sys.stderr.write("laptop-agent: command timed out\n")
    sys.exit(TIMEOUT_EXIT)


def _run_guarded(fn):
    """Run fn(); on deadline/signal interruption, cancel the remote
    work best-effort and exit 124 (GNU timeout's code)."""
    global _cancel_hook
    try:
        return fn()
    except CallTimeout:
        hook, _cancel_hook = _cancel_hook, None
        was_broker = not os.environ.get("LAPTOP_AGENT_NO_BROKER")
        ok = False
        if hook is not None:
            try:
                ok = bool(hook())
            except Exception:
                ok = False
        if was_broker and not ok:
            # The broker reply never arrived; its handler thread may
            # still be parked on a tunnel holding this call. Restart
            # the broker so the pool is clean for the next caller.
            _restart_broker()
        _exit_timeout()


def main():
    _install_signal_handlers()
    argv = sys.argv[1:]
    if not argv:
        sys.exit(__doc__)
    sub = argv[0]
    rest = argv[1:]

    if sub == "health":
        print(json.dumps(request("GET", "/health")))
        return

    if sub == "jobs":
        print(json.dumps(request("GET", "/jobs"), indent=1))
        return

    if sub == "kill":
        print(json.dumps(request("POST", f"/jobs/{rest[0]}/kill", {})))
        return

    if sub == "exec-batch":
        body = json.loads(Path(rest[0]).read_text())
        for _cmd in body.get("cmds", []):
            _enforce_primary_guard(_cmd)
        _set_deadline(body.get("timeout_s", 120))

        def _batch():
            out = request("POST", "/exec", body)
            print(json.dumps(out, indent=1))
            # Fail LOUD (code#5343 silence audit, 2026-11-10): a batch
            # whose commands failed must not exit 0 — the caller would
            # read empty output as success. Exit with the worst exit
            # code; a command that could not run at all counts as 1.
            worst = 0
            for r in out.get("results", []):
                if r.get("truncated"):
                    sys.stderr.write(
                        "laptop-agent: warning: a command's output was "
                        "truncated at the 1 MiB/stream capture cap "
                        "(tail kept)\n"
                    )
                if r.get("error"):
                    worst = max(worst, 1)
                elif r.get("timeout"):
                    worst = max(worst, TIMEOUT_EXIT)
                elif r.get("exit"):
                    worst = max(worst, r["exit"])
            if worst:
                sys.exit(worst)

        _run_guarded(_batch)
        return

    if sub in ("exec", "run"):
        import argparse

        opt_args, cmd_args = split_cmd(rest)
        ap = argparse.ArgumentParser()
        ap.add_argument("--cwd", default=None)
        ap.add_argument("--timeout", type=int, default=120)
        ap.add_argument("--shell", action="store_true")
        opts = ap.parse_args(opt_args)
        spec = cmd_spec(opt_args, cmd_args, opts.cwd, opts.shell)
        _enforce_primary_guard(spec)
        if sub == "run":
            _enforce_launch_guard(spec)
        ext = os.environ.get(EXTERNAL_DEADLINE_ENV)
        if sub == "exec" and ext:
            try:
                opts.timeout = max(1, min(opts.timeout, int(float(ext))))
            except ValueError:
                pass
        if sub == "run":
            _set_deadline(30)
            out = _run_guarded(lambda: request("POST", "/jobs", spec))
            job_id = out["job_id"]
            print(job_id, flush=True)
            # A detached job is meant to survive client death, so an
            # external timeout on the SUBMIT call must not kill it —
            # unless the signal actually interrupted this process,
            # which means the caller's deadline did fire: honour it
            # by killing the job we just created.
            if _interrupted is not None:
                try:
                    _direct_request(
                        "POST", f"/jobs/{job_id}/kill", {}, timeout_s=_CANCEL_BUDGET_S
                    )
                except Exception:
                    pass
                _exit_timeout()
            return
        _set_deadline(opts.timeout)

        def _exec():
            out = request("POST", "/exec", {"cmds": [spec], "timeout_s": opts.timeout})
            r = out["results"][0]
            sys.stdout.write(r.get("stdout", ""))
            sys.stderr.write(r.get("stderr", ""))
            if r.get("truncated"):
                sys.stderr.write(
                    "laptop-agent: warning: output truncated at the "
                    "1 MiB/stream capture cap (tail kept)\n"
                )
            if r.get("error"):
                sys.stderr.write(f"laptop-agent: {r['error']}\n")
            if r.get("timeout"):
                _exit_timeout()
            sys.exit(r["exit"] if r["exit"] is not None else 1)

        _run_guarded(_exec)
        return

    if sub == "poll":
        job_id = rest[0]
        wait = 0
        if "--wait" in rest:
            wait = int(rest[rest.index("--wait") + 1])
        OFFSET_DIR.mkdir(parents=True, exist_ok=True)
        off_file = OFFSET_DIR / f"{job_id}.json"
        offs = {"out": 0, "err": 0}
        if off_file.exists():
            offs = json.loads(off_file.read_text())
        deadline = time.time() + wait
        # Interruption hook state: timing out a poll means the caller
        # is done waiting — kill the remote job, don't orphan it.
        state = {"done": False}

        def _poll_cancel():
            if state["done"]:
                return True
            try:
                _direct_request(
                    "POST", f"/jobs/{job_id}/kill", {}, timeout_s=_CANCEL_BUDGET_S
                )
                return True
            except Exception:
                return False

        global _cancel_hook
        _cancel_hook = _poll_cancel
        # Reply watchdog: if the process receives SIGTERM/SIGINT and
        # stays blocked in a C-level call that defers the Python-level
        # handler (observed with select() in socket recv), this thread
        # notices _interrupted and cancels the job itself.
        import threading as _threading

        def _watchdog():
            while not state["done"]:
                if _interrupted is not None:
                    _poll_cancel()
                    return
                time.sleep(0.2)

        _threading.Thread(target=_watchdog, daemon=True).start()
        try:
            while True:
                wait_s = max(0, min(10, math.ceil(deadline - time.time())))
                out = request(
                    "GET", f"/jobs/{job_id}?out_off={offs['out']}&err_off={offs['err']}&wait_s={wait_s}"
                )
                sys.stdout.write(out.get("stdout", ""))
                sys.stdout.flush()
                sys.stderr.write(out.get("stderr", ""))
                sys.stderr.flush()
                got = (out["out_next"] - offs["out"]) + (out["err_next"] - offs["err"])
                offs = {"out": out["out_next"], "err": out["err_next"]}
                off_file.write_text(json.dumps(offs))
                if out["state"] == "done":
                    # Drain a finished job's spool to the end (#15420):
                    # one reply carries at most 1 MiB per stream. Stop
                    # once the cursors reach the reported sizes, or a
                    # page carries nothing (daemons without sizes).
                    sizes = (out.get("out_size"), out.get("err_size"))
                    drained = got <= 0 or (
                        None not in sizes
                        and offs["out"] >= sizes[0]
                        and offs["err"] >= sizes[1]
                    )
                    if not drained:
                        continue
                    state["done"] = True
                    dropped = out.get("bytes_dropped") or 0
                    if dropped > 0:
                        # The daemon's spool cap discarded output; say
                        # so instead of truncating silently (#15420).
                        print(
                            f"[spool cap reached: {dropped} bytes dropped]",
                            file=sys.stderr,
                        )
                    print(
                        f"[laptop-agent] job {job_id} done "
                        f"exit={out['exit']} "
                        f"duration_ms={out['duration_ms']}",
                        file=sys.stderr,
                    )
                    sys.exit(out["exit"] if out["exit"] is not None else 1)
                if time.time() >= deadline:
                    print(f"[laptop-agent] job {job_id} still running", file=sys.stderr)
                    return
        except CallTimeout:
            _poll_cancel()
            _exit_timeout()
        finally:
            _cancel_hook = None

    if sub == "fetch":
        # fetch PATH OUT — chunked download (code#5757): 1 MiB chunks
        # by default, resume by offset (an existing OUT is continued
        # from its current size, never restarted).
        args = list(rest)
        chunk = 1 << 20
        if "--chunk" in args:
            i = args.index("--chunk")
            chunk = int(args[i + 1])
            del args[i : i + 2]
            if chunk <= 0 or chunk > (1 << 20):
                sys.exit(
                    "laptop-agent: --chunk must be 1..1048576 "
                    "(the daemon caps one request at 1 MiB)"
                )
        if len(args) != 2:
            sys.exit("usage: laptop-agent.py fetch [--chunk BYTES] PATH OUT")
        remote_path, out_path = args
        off = os.path.getsize(out_path) if os.path.exists(out_path) else 0
        resumed_from = off
        trips = 0
        total = None
        mode = "ab" if off else "wb"
        with open(out_path, mode) as fh:
            while total is None or off < total:
                data, total = _fetch_chunk(remote_path, off, chunk)
                trips += 1
                if off > total:
                    sys.exit(
                        f"laptop-agent: local {out_path} is {off} "
                        f"bytes but remote {remote_path} is only "
                        f"{total}; refusing to append — remove or "
                        f"rename the local file and retry"
                    )
                if not data and off < total:
                    sys.exit("laptop-agent: empty chunk before EOF; fetch incomplete")
                fh.write(data)
                off += len(data)
                if len(data) < chunk and off < total:
                    continue
        print(
            f"fetched {total} bytes from {remote_path} -> {out_path} "
            f"in {trips} round trip(s)"
            + (f" (resumed from offset {resumed_from})" if resumed_from else ""),
            file=sys.stderr,
        )
        return

    sys.exit(f"laptop-agent: unknown subcommand {sub!r}")


if __name__ == "__main__":
    main()
