#!/usr/bin/env python3
"""laptop-agent broker — keeps warm connections to the laptop-agent
daemon so single client calls skip the CONNECT handshake (code#5343).

The cold cost is the tunnel CONNECT through the DERP hairpin (measured
~1.8 s, variable); a request over an already-open tunnel is ~0.25 s.
Python client startup is ~25 ms — not the problem. So: this broker holds
a small pool of keep-alive HTTP connections (each over its own CONNECT
tunnel) and serves the CLI over a Unix socket, exactly like SSH's
ControlMaster/ControlPersist pattern. The client auto-starts it when the
socket is missing; it exits after an idle hour.

Protocol (length-prefixed JSON both ways):
  request  {"method", "path", "body", "port"?, "token_file"?}
  response {"status", "body"}

Timeout behaviour (2026-11-10): HTTP timeouts on pooled connections
are set per request from the caller's remaining deadline (header
"deadline_s"), never a flat 660 s, and a connection whose request
times out or errors is CLOSED, never returned to the pool — one
wedged tunnel cannot park or poison later callers. The broker writes
its pid to broker.pid so a client whose deadline fired while parked
on the broker socket can restart it for a clean pool.

`port` (default 4799, the laptop-agent) selects the laptop service —
the session proxy is the single entry point for all of them
(queue-broker on 4801, secrets-broker on 4800; code#5362). `token_file`
names the file whose contents are sent as X-Agent-Token for that
service (read per request, never stored); without it the laptop-agent
token is used. Pools are kept per port.
"""

import http.client
import importlib.util
import json
import os
import socket
import socketserver
import struct
import sys
import threading
import time
from pathlib import Path


def log(msg):
    """Timestamped broker log line (#9611 acceptance 4: every broker
    log line carries a timestamp, and tunnel death/recovery events
    are logged as they happen)."""
    ts = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    print(f"{ts} laptop-agent broker: {msg}", flush=True)


spec = importlib.util.spec_from_file_location(
    "la", str(Path.home() / "tools/laptop-agent.py")
)
la = importlib.util.module_from_spec(spec)
spec.loader.exec_module(la)

SOCK_DIR = Path.home() / ".cache/laptop-agent"
SOCK_PATH = SOCK_DIR / "broker.sock"
POOL_SIZE = int(os.environ.get("LAPTOP_AGENT_POOL", "12"))
IDLE_EXIT_S = 3600
KEEPALIVE_S = 180

_pools = {}
_pool_lock = threading.Lock()
_last_used = time.time()  # client requests only; keepalive must not
# reset this or the idle exit never fires


_inflight = {}  # client unix-sock -> tracker dict with "tun"
_inflight_lock = threading.Lock()
_watch_started = False


def _track(client_sock, tracker):
    global _watch_started
    with _inflight_lock:
        _inflight[client_sock] = tracker
        start = not _watch_started
        _watch_started = True
    if start:
        threading.Thread(target=_inflight_watchdog, daemon=True).start()


def _untrack(client_sock):
    with _inflight_lock:
        _inflight.pop(client_sock, None)


def _inflight_watchdog():
    """Every 0.2 s: if a client whose request is in flight has hung up
    (EOF on its unix socket), hard-close the tunnel that request is
    using. The handler thread blocked in getresponse() then errors
    immediately, and closing the tunnel is also the signal the daemon
    uses to reap the remote command group. This is what makes an
    uncatchable client death (SIGKILL) bounded on the laptop side."""
    while True:
        time.sleep(0.2)
        with _inflight_lock:
            items = list(_inflight.items())
        for client_sock, tracker in items:
            tun = tracker.get("tun")
            if tun is None:
                continue
            try:
                client_sock.setblocking(False)
                try:
                    data = client_sock.recv(1, socket.MSG_PEEK)
                    gone = data == b""
                except BlockingIOError:
                    gone = False
                except OSError:
                    gone = True
                finally:
                    client_sock.setblocking(True)
            except OSError:
                gone = True
            if gone:
                try:
                    tun.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass
                try:
                    tun.close()
                except OSError:
                    pass


TUNNEL_CONNECT_TIMEOUT_S = float(os.environ.get("LAPTOP_AGENT_TUNNEL_TIMEOUT", "5"))


def broker_tunnel_socket(port, timeout_s=None):
    """Broker-local CONNECT tunnel establishment (#9611 acceptance 2).

    Same wire behaviour as la.tunnel_socket (credential read from the
    seat's recorded tailnet proxy when one exists, else the HTTPS_PROXY
    env, and sent only inside the CONNECT request), but
    with a hard bounded timeout and ordinary exceptions: the client's
    tunnel_socket can block up to 60 s waiting for the CONNECT response
    and raises SystemExit on a closed proxy — inside a broker thread
    SystemExit is a BaseException that bypasses every `except
    Exception` guard and can hang or kill request handling silently,
    which is exactly the episode-2 wedge shape. Here a wedged upstream
    surfaces as a fast, logged OSError instead."""
    import base64
    import urllib.parse

    # The recorded tailnet proxy beats the ambient env (#12426): a
    # broker auto-started from a sandbox shell otherwise inherits the
    # platform egress proxy and every tunnel fails while it looks
    # alive. la is the deployed client; an older copy without the
    # resolver leaves the ambient env in charge.
    resolver = getattr(la, "tailnet_https_proxy", None)
    raw = (resolver() if resolver is not None else None) or os.environ.get(
        "HTTPS_PROXY"
    )
    if not raw:
        raise OSError("HTTPS_PROXY must be set")
    u = urllib.parse.urlparse(raw)
    user = urllib.parse.unquote(u.username or "")
    pw = urllib.parse.unquote(u.password or "")
    timeout = TUNNEL_CONNECT_TIMEOUT_S if timeout_s is None else timeout_s
    sock = socket.create_connection((u.hostname, la.PROXY_PORT), timeout=timeout)
    try:
        sock.settimeout(timeout)
        auth = base64.b64encode(f"{user}:{pw}".encode()).decode()
        req = (
            f"CONNECT {la.TARGET_HOST}:{port} HTTP/1.1\r\n"
            f"Host: {la.TARGET_HOST}:{port}\r\n"
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
    except Exception:
        try:
            sock.close()
        except OSError:
            pass
        raise
    sock.settimeout(None)
    return sock


# Transports (2026-11-15, code#10356): "cf" = the named Cloudflare
# tunnel (sandbox-link.security.cab, Access-gated) — primary for the
# laptop-agent daemon on port 4799 only; "ts" = the Tailscale session
# proxy CONNECT — the fallback for 4799 and the only transport for the
# other brokered ports (4800/4801 are not ingressed on the tunnel).
CF_PORT = 4799
CF_EDGE_ERROR_STATUSES = (502, 503, 530)


def open_conn(port=4799, transport="ts"):
    if transport == "cf":
        conn = la.make_cf_http()(la.CF_HOST, timeout=660)
        # Eager connect: establishment failure must surface here as a
        # request failure (same contract as broker_tunnel_socket),
        # not later inside a handler thread's first send.
        conn.connect()
        return conn
    conn = http.client.HTTPConnection(la.TARGET_HOST, port, timeout=660)
    conn.sock = broker_tunnel_socket(port)
    return conn


def pool_put(port, conn, transport="ts"):
    with _pool_lock:
        pool = _pools.setdefault((port, transport), [])
        if len(pool) < POOL_SIZE:
            pool.append(conn)
            return
    try:
        conn.close()
    except Exception:
        pass


def pool_get(port, transport="ts"):
    """Return (conn, from_pool). Tunnel establishment failures raise
    OSError from broker_tunnel_socket / la.cf_socket — callers must
    treat them as request failures, never let them escape as thread
    deaths."""
    with _pool_lock:
        pool = _pools.get((port, transport)) or []
        if pool:
            return pool.pop(), True
    return open_conn(port, transport), False


def _attempt_plan(port, pref, cf_ok):
    """Ordered (transport, source) attempts for one request. Cloudflare
    is primary for the daemon port when the caller allows it; Tailscale
    is the fallback leg (auto) and the only leg for other ports or
    when pinned. A pooled attempt precedes at most one fresh attempt
    per transport."""
    cf_in_play = (
        port == CF_PORT
        and cf_ok
        and pref in ("auto", "cf")
        and la._cf_access_available()
    )
    if cf_in_play:
        plan = [("cf", "pool"), ("cf", "fresh")]
        if pref == "auto":
            plan.append(("ts", "fresh"))
        return plan
    return [("ts", "pool"), ("ts", "fresh")]


def do_request(
    method,
    path,
    body,
    port=4799,
    token_file=None,
    client=True,
    trip=None,
    deadline_s=None,
    tracker=None,
    pref="auto",
    cf_ok=True,
):
    """One request on a pooled warm connection; retries per the
    transport attempt plan (stale pooled connection -> fresh one;
    Cloudflare failure -> Tailscale fallback). The socket timeout is
    the caller's remaining deadline (clamped), and a connection that
    fails or times out is closed — never returned to the pool.
    Returns (status, body, transport_used)."""
    global _last_used
    if client:
        _last_used = time.time()
    if token_file:
        token = Path(token_file).read_text().strip()
    else:
        token = la.TOKEN_FILE.read_text().strip()
    payload = json.dumps(body) if body is not None else None
    if deadline_s is None:
        timeout_s = 660
    else:
        timeout_s = max(1.0, min(660.0, float(deadline_s)))
    plan = _attempt_plan(port, pref, cf_ok)
    fresh_failed = set()
    last_err = None
    for i, (transport, source) in enumerate(plan):
        if source == "fresh" and transport in fresh_failed:
            continue  # a fresh connection of this transport just failed
        conn = None
        from_pool = False
        try:
            # Connection acquisition is INSIDE the try (#9611 acc. 2):
            # a tunnel-establishment failure is a request failure with
            # a fast, logged error — never an uncaught thread death.
            if source == "pool":
                conn, from_pool = pool_get(port, transport)
            else:
                conn = open_conn(port, transport)
            if tracker is not None:
                tracker["tun"] = conn.sock
            if conn.sock is not None:
                conn.sock.settimeout(timeout_s)
            headers = {"X-Agent-Token": token, "Content-Type": "application/json"}
            if transport == "cf":
                headers.update(la._cf_access_headers())
            if trip:
                headers["X-Trip-Id"] = trip
            conn.request(method, path, body=payload, headers=headers)
            resp = conn.getresponse()
            data = resp.read()
            if (
                transport == "cf"
                and resp.status in CF_EDGE_ERROR_STATUSES
                and i < len(plan) - 1
            ):
                # The edge answered but the tunnel origin is down:
                # drop the connection and take the fallback leg.
                try:
                    conn.close()
                except Exception:
                    pass
                last_err = OSError(f"cloudflare edge status {resp.status}")
                log(
                    f"request port={port} via cf: edge status "
                    f"{resp.status} (tunnel origin down; falling back)"
                )
                continue
            pool_put(port, conn, transport)
            return resp.status, json.loads(data or b"{}"), transport
        except Exception as e:
            last_err = e
            if not from_pool:
                fresh_failed.add(transport)
            log(
                f"request attempt {i + 1} port={port} "
                f"transport={transport} failed: "
                f"{type(e).__name__}: {e} (connection dropped, not repooled)"
            )
            if tracker is not None:
                tracker["tun"] = None
            if conn is not None:
                try:
                    conn.close()
                except Exception:
                    pass
            if isinstance(e, (TimeoutError, socket.timeout)):
                raise  # deadline fired; conn already dropped — no retry
            if not from_pool and i == len(plan) - 1:
                # The last plan leg already built a FRESH connection
                # and it failed: the upstream is wedged — fail this
                # client fast (#9611 acc. 2) instead of paying another
                # tunnel timeout.
                raise
    raise last_err


def preconnect():
    # Warm the primary (Cloudflare) pool for the daemon port; if the
    # tunnel is unreachable at broker start, warm Tailscale instead so
    # the pool is never empty behind a down primary.
    transport = "cf" if la._cf_access_available() else "ts"
    for _ in range(POOL_SIZE):
        try:
            pool_put(CF_PORT, open_conn(CF_PORT, transport), transport)
        except Exception:
            if transport == "cf":
                transport = "ts"
                continue
            break


def keepalive_loop():
    down = False
    while True:
        time.sleep(KEEPALIVE_S)
        if time.time() - _last_used > IDLE_EXIT_S:
            os._exit(0)
        try:
            do_request("GET", "/health", None, client=False)
            if down:
                down = False
                log("keepalive health recovered (a transport path is reachable again)")
        except Exception as e:
            if not down:
                down = True
                log(
                    f"keepalive health FAILED: {type(e).__name__}: {e} "
                    "(both transport paths may be wedged; #9611)"
                )


def read_msg(conn):
    hdr = b""
    while len(hdr) < 8:
        chunk = conn.recv(8 - len(hdr))
        if not chunk:
            return None
        hdr += chunk
    (n,) = struct.unpack(">Q", hdr)
    data = b""
    while len(data) < n:
        chunk = conn.recv(min(1 << 20, n - len(data)))
        if not chunk:
            return None
        data += chunk
    return json.loads(data)


def write_msg(conn, obj):
    data = json.dumps(obj).encode()
    conn.sendall(struct.pack(">Q", len(data)) + data)


class Handler(socketserver.BaseRequestHandler):
    def handle(self):
        while True:
            try:
                req = read_msg(self.request)
            except (BlockingIOError, OSError):
                # #9611 acceptance 3: the inflight watchdog toggles
                # setblocking() on this socket; a recv racing that
                # transition raises BlockingIOError out of read_msg.
                # That must end the handler quietly, never propagate
                # as an uncaught traceback that kills request handling.
                return
            if req is None:
                return
            tracker = {"tun": None}
            _track(self.request, tracker)
            try:
                body_in = req.get("body")
                deadline_s = req.get("deadline_s")
                if deadline_s is None and isinstance(body_in, dict):
                    ts = body_in.get("timeout_s")
                    if isinstance(ts, (int, float)):
                        deadline_s = float(ts) + 15.0
                status, body, transport = do_request(
                    req["method"],
                    req["path"],
                    body_in,
                    port=int(req.get("port", 4799)),
                    token_file=req.get("token_file"),
                    trip=req.get("trip"),
                    deadline_s=deadline_s,
                    tracker=tracker,
                    pref=req.get("transport", "auto"),
                    cf_ok=req.get("cf_ok", True),
                )
                write_msg(
                    self.request,
                    {"status": status, "body": body, "transport": transport},
                )
            except Exception as e:
                try:
                    write_msg(
                        self.request,
                        {
                            "status": 502,
                            "body": {"error": f"broker: {type(e).__name__}: {e}"},
                        },
                    )
                except OSError:
                    pass  # client already gone — nothing to answer
            finally:
                _untrack(self.request)


class Server(socketserver.ThreadingMixIn, socketserver.UnixStreamServer):
    daemon_threads = True
    allow_reuse_address = True


_singleton_lock_fd = None


def main():
    global _singleton_lock_fd
    SOCK_DIR.mkdir(parents=True, exist_ok=True)
    try:
        os.chmod(SOCK_DIR, 0o700)
    except OSError:
        pass
    # #9611 acceptance 1 (Python-implementation leg): hold an
    # exclusive flock on broker.lock for the process lifetime, so two
    # Python broker starts can never both proceed — the socket probe
    # below alone is racy (two starters can both see a stale socket).
    # The flock releases automatically when the holder dies, so a
    # crashed broker never wedges the path. NOTE: this binds only the
    # Python implementation; the fleet-proxy (Rust) leg of acceptance
    # 1 is tracked on issue #9611.
    try:
        import fcntl

        _singleton_lock_fd = os.open(
            str(SOCK_DIR / "broker.lock"), os.O_CREAT | os.O_RDWR, 0o600
        )
        fcntl.flock(_singleton_lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError:
        return  # another Python broker holds the singleton lock
    if SOCK_PATH.exists():
        # A live broker already owns the socket; exit quietly.
        probe = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        try:
            probe.connect(str(SOCK_PATH))
            probe.close()
            return
        except OSError:
            SOCK_PATH.unlink()
    srv = Server(str(SOCK_PATH), Handler)
    # broker.pid is written only after this process actually owns the
    # listening socket, so it names the socket owner (#9611 acc. 1).
    try:
        (SOCK_DIR / "broker.pid").write_text(str(os.getpid()))
    except OSError:
        pass
    threading.Thread(target=preconnect, daemon=True).start()
    threading.Thread(target=keepalive_loop, daemon=True).start()
    log(f"on {SOCK_PATH} pid={os.getpid()}")
    srv.serve_forever()


if __name__ == "__main__":
    main()
