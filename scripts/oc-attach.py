#!/usr/bin/env python3
"""oc-attach — attach-first OpenCode session client for fleet workers.

One documented entry point for programmatic OpenCode work against the
laptop's background service (developing-today/code#1357, from the #1344
access-pattern benchmark). Programmatic work defaults to ATTACH: a
persistent SSH forward to the laptop service, raw API control, and
`opencode run --server` for turns — never a fresh local server per task,
so sessions survive transport death and are re-discoverable afterwards
(no orphan sessions).

What it does:
  discover  Read the laptop's ~/.local/state/opencode/service-prod.json
            over the laptop agent (~/tools/laptop-agent.py) and
            provision {url, password}
            into a local 0600 secret file (~/tools/secrets.d/, dir 700).
            The password is never printed, never logged, never argv.
  forward   Open (or reuse) the multiplexed SSH forward to the service:
            a dedicated ControlMaster socket, health-checked against the
            local port; a dead forward is reopened. Idempotent.
  health    GET /api/info through the forward (sanitized output).
  list      List sessions on the laptop service (id, title, directory).
  prompt    Raw-API turn: create a session (location in the POST body —
            the ?directory= query param is silently ignored) or reuse
            --session, POST the prompt, wait, print the reply.
  run       `opencode run --server <forward-url>` with stdin CLOSED by
            default (an open non-tty stdin hangs `opencode run`, #1359)
            and the password passed via OPENCODE_SERVER_PASSWORD env,
            never argv.

Every subcommand except `discover` auto-provisions the secret file and
the forward first, so the one-command form is:

  oc-attach prompt --directory /home/user/git/code --text "…"
  oc-attach run    --directory /home/user/git/code "…"
  oc-attach list

Local forward port defaults to 13421 — the service's own port number
(13420) is already held on the sandbox by an unrelated opencode wrapper
answering with different credentials — and can be overridden with
OC_ATTACH_LOCAL_PORT. The forward is reached with the
same pinned-host-key ssh options the retired ~/tools/laptop-ssh.sh used
(a command helper cannot express the forward itself: it appends its
args as a remote command, so -N/-L flags have to be built here).
"""

from __future__ import annotations

import argparse
import base64
import json
import os
import pathlib
import subprocess
import sys
import urllib.error
import urllib.request

HOME = pathlib.Path.home()
SECRET_DIR = HOME / "tools" / "secrets.d"
SECRET_FILE = SECRET_DIR / "opencode-laptop-service.json"
CONTROL_SOCKET = HOME / ".ssh" / "cm-ocattach@100.99.113.103:22"
LAPTOP = "user@100.99.113.103"
DEFAULT_LOCAL_PORT = 13421


def die(msg: str) -> "SystemExit":
    print(f"oc-attach: {msg}", file=sys.stderr)
    return SystemExit(1)


# ---------------------------------------------------------------- discovery

def fetch_discovery() -> dict:
    """Read service-prod.json on the laptop; return the parsed dict.

    Only ever called by ensure_secret(); the caller writes the password
    to the 0600 file and never prints the dict.
    """
    remote = (
        "python3 -c \"import json,pathlib;"
        "print(json.dumps(json.loads((pathlib.Path.home()"
        "/'.local/state/opencode/service-prod.json').read_text())))\""
    )
    out = subprocess.run(
        [str(HOME / "tools" / "laptop-agent.py"),
         "exec", "--shell", "--", remote],
        capture_output=True, text=True, timeout=120,
    )
    if out.returncode != 0:
        raise die(f"laptop discovery failed: {out.stderr.strip()[:200]}")
    try:
        disc = json.loads(out.stdout)
        _ = disc["url"], disc["password"]
    except (ValueError, KeyError) as exc:
        raise die(f"unexpected service-prod.json shape: {exc}")
    return disc


def ensure_secret() -> dict:
    """Return the discovery dict, provisioning the 0600 file if stale.

    Staleness check: the file's url/pid are re-read live only when the
    file is missing; a running helper trusts the file for one invocation
    (the forward health check is what catches a restarted service, and
    `discover` forces a refresh).
    """
    if SECRET_FILE.exists():
        try:
            disc = json.loads(SECRET_FILE.read_text())
            if disc.get("url") and disc.get("password"):
                return disc
        except ValueError:
            pass
    disc = fetch_discovery()
    SECRET_DIR.mkdir(mode=0o700, exist_ok=True)
    os.chmod(SECRET_DIR, 0o700)
    tmp = SECRET_FILE.with_suffix(".tmp")
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as f:
        json.dump(disc, f)
    os.chmod(tmp, 0o600)
    tmp.replace(SECRET_FILE)
    return disc


def remote_port(disc: dict) -> int:
    return int(disc["url"].rsplit(":", 1)[1].split("/")[0])


def local_port() -> int:
    return int(os.environ.get("OC_ATTACH_LOCAL_PORT", DEFAULT_LOCAL_PORT))


def local_url() -> str:
    return f"http://127.0.0.1:{local_port()}"


# ------------------------------------------------------------------ forward

def ssh_base() -> list[str]:
    if not os.environ.get("HTTPS_PROXY"):
        raise die("HTTPS_PROXY must be set (runtime proxy env, as the retired laptop-ssh.sh required)")
    return [
        "ssh",
        "-o", f"ProxyCommand=python3 {HOME}/tools/laptop-proxy-connect.py",
        "-o", "StrictHostKeyChecking=yes",
        "-o", f"UserKnownHostsFile={HOME}/.ssh/known_hosts_laptop",
        "-o", "IdentitiesOnly=yes", "-i", str(HOME / ".ssh" / "id_ed25519"),
        "-o", "ConnectTimeout=60",
        "-o", "ServerAliveInterval=15", "-o", "ServerAliveCountMax=8",
    ]


def forward_alive() -> bool:
    if not CONTROL_SOCKET.exists():
        return False
    out = subprocess.run(
        ssh_base() + ["-S", str(CONTROL_SOCKET), "-O", "check", LAPTOP],
        capture_output=True, text=True, timeout=30,
    )
    return out.returncode == 0


def api(path: str, disc: dict, method: str = "GET", body: dict | None = None,
        timeout: int = 30) -> tuple[int, object]:
    req = urllib.request.Request(local_url() + path, method=method)
    cred = base64.b64encode(f"opencode:{disc['password']}".encode()).decode()
    req.add_header("Authorization", f"Basic {cred}")
    data = None
    if body is not None:
        data = json.dumps(body).encode()
        req.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(req, data=data, timeout=timeout) as resp:
            raw = resp.read()
            return resp.status, (json.loads(raw) if raw else None)
    except urllib.error.HTTPError as e:
        raw = e.read()
        try:
            return e.code, json.loads(raw) if raw else None
        except ValueError:
            return e.code, None


def forward_healthy(disc: dict) -> bool:
    try:
        status, _ = api("/api/info", disc, timeout=10)
        return status == 200
    except Exception:
        return False


def ensure_forward(disc: dict) -> None:
    """Open the multiplexed forward if needed; reopen it when dead."""
    if forward_alive() and forward_healthy(disc):
        return
    if CONTROL_SOCKET.exists():
        subprocess.run(
            ssh_base() + ["-S", str(CONTROL_SOCKET), "-O", "exit", LAPTOP],
            capture_output=True, text=True, timeout=30,
        )
    cmd = ssh_base() + [
        "-M", "-S", str(CONTROL_SOCKET), "-f", "-N",
        "-L", f"127.0.0.1:{local_port()}:127.0.0.1:{remote_port(disc)}",
        "-o", "ExitOnForwardFailure=yes",
        LAPTOP,
    ]
    # NOT capture_output: the backgrounded master inherits the pipes and a
    # run() would block on them long after the forward is up. Poll health.
    proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL,
                            stderr=subprocess.DEVNULL, start_new_session=True)
    import time
    for _ in range(45):
        if forward_healthy(disc):
            return
        if proc.poll() is not None:
            raise die(f"ssh forward exited early with {proc.returncode}")
        time.sleep(1)
    raise die("forward opened but the service did not answer /api/info in 45s")


# ------------------------------------------------------------------ commands

def cmd_discover(_args) -> int:
    SECRET_FILE.unlink(missing_ok=True)
    disc = ensure_secret()
    print(f"service: {disc['url']} (version {disc.get('version')}, pid {disc.get('pid')})")
    print(f"secret provisioned: {SECRET_FILE} (0600; password not shown)")
    print(f"forward url once open: {local_url()}")
    return 0


def cmd_forward(_args) -> int:
    disc = ensure_secret()
    ensure_forward(disc)
    print(f"forward up: {local_url()} -> laptop {disc['url']} (control {CONTROL_SOCKET})")
    return 0


def cmd_health(_args) -> int:
    disc = ensure_secret()
    ensure_forward(disc)
    status, info = api("/api/info", disc)
    if status != 200:
        raise die(f"/api/info returned {status}")
    print(json.dumps(info, indent=2))
    return 0


def unwrap(data: object) -> object:
    """v2 list endpoints answer {data: [...], cursor: {...}} envelopes."""
    if isinstance(data, dict) and isinstance(data.get("data"), (list, dict)):
        return data["data"]
    return data


def cmd_list(_args) -> int:
    disc = ensure_secret()
    ensure_forward(disc)
    status, sessions = api("/api/session", disc)
    sessions = unwrap(sessions)
    if status != 200 or not isinstance(sessions, list):
        raise die(f"/api/session returned {status}")
    for s in sessions:
        loc = (s.get("location") or {}).get("directory", "")
        print(f"{s.get('id')}  {s.get('title') or '(untitled)'}  {loc}")
    print(f"{len(sessions)} session(s) on {disc['url']}")
    return 0


def session_create(disc: dict, directory: str, title: str | None,
                   model: str | None = None) -> str:
    body: dict = {"location": {"directory": directory}}
    if title:
        body["title"] = title
    if model:
        provider, _, model_id = model.partition("/")
        if not provider or not model_id:
            raise die("--model must be provider/model (e.g. opencode-route/workhorse)")
        body["model"] = {"id": model_id, "providerID": provider}
    status, sess = api("/api/session", disc, method="POST", body=body)
    sess = unwrap(sess)
    if status not in (200, 201) or not isinstance(sess, dict) or not sess.get("id"):
        raise die(f"session create returned {status}: {str(sess)[:200]}")
    return str(sess["id"])


def cmd_prompt(args) -> int:
    disc = ensure_secret()
    ensure_forward(disc)
    session_id = args.session or session_create(
        disc, args.directory, args.title, args.model)
    status, _resp = api(
        f"/api/session/{session_id}/prompt", disc, method="POST",
        body={"text": args.text}, timeout=60,
    )
    if status not in (200, 201, 202, 204):
        raise die(f"prompt returned {status}")
    # Wait for the turn to settle, then print the latest assistant text.
    api(f"/api/experimental/session/{session_id}/wait", disc, method="POST",
        body={}, timeout=args.timeout)
    status, msgs = api(f"/api/session/{session_id}/message", disc, timeout=60)
    msgs = unwrap(msgs)
    if status != 200 or not isinstance(msgs, list):
        raise die(f"message list returned {status}")
    # v2 message shape: flat objects, type = user|assistant|system|idle;
    # assistant text lives in content[] parts ({type: "text", text}).
    texts: list[str] = []
    error: str | None = None
    for m in msgs:
        if m.get("type") != "assistant":
            continue
        for part in m.get("content") or []:
            if isinstance(part, dict) and part.get("type") == "text" and part.get("text"):
                texts.append(str(part["text"]))
        if isinstance(m.get("text"), str) and m["text"]:
            texts.append(m["text"])
        err = m.get("error") or {}
        if isinstance(err, dict) and err.get("message"):
            error = str(err["message"])[:300]
    print(f"session: {session_id}")
    if texts:
        print(texts[-1])
        return 0
    if error:
        print(f"oc-attach: turn failed: {error}", file=sys.stderr)
        return 1
    print("(no assistant text yet — session may still be running; "
          f"re-check with: oc-attach prompt --session {session_id} --directory "
          f"{args.directory} --text <follow-up>)")
    return 0


def cmd_run(args) -> int:
    disc = ensure_secret()
    ensure_forward(disc)
    env = dict(os.environ)
    env["OPENCODE_SERVER_PASSWORD"] = disc["password"]  # env, never argv
    # v2 `opencode run` has no --directory flag, and the session location
    # must travel in the create POST body anyway — so create (or reuse)
    # the session through the API first, then attach the CLI to it.
    session_id = args.session or session_create(
        disc, args.directory, args.title, args.model)
    cmd = ["opencode", "run", "--server", local_url(),
           "--session", session_id]
    if args.model:
        cmd += ["--model", args.model]
    cmd.append(args.text)
    # stdin CLOSED by default: an open non-tty stdin hangs `opencode run`.
    out = subprocess.run(cmd, env=env, stdin=subprocess.DEVNULL,
                         capture_output=True, text=True, timeout=args.timeout)
    sys.stdout.write(out.stdout)
    if out.returncode != 0:
        print(f"oc-attach: opencode run exited {out.returncode}: "
              f"{out.stderr.strip()[:300]}", file=sys.stderr)
        return out.returncode
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(prog="oc-attach", description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("discover", help="refresh the 0600 secret file from the laptop")
    sub.add_parser("forward", help="open/reuse the multiplexed SSH forward")
    sub.add_parser("health", help="GET /api/info through the forward")
    sub.add_parser("list", help="list sessions on the laptop service")
    p = sub.add_parser("prompt", help="raw-API prompt turn (location in POST body)")
    p.add_argument("--directory", required=True, help="laptop project directory")
    p.add_argument("--text", required=True)
    p.add_argument("--session", help="reuse an existing session id")
    p.add_argument("--title")
    p.add_argument("--model", help="provider/model for a newly created session")
    p.add_argument("--timeout", type=int, default=600)
    r = sub.add_parser("run", help="opencode run --server, stdin closed")
    r.add_argument("--directory", required=True, help="laptop project directory")
    r.add_argument("--session")
    r.add_argument("--title")
    r.add_argument("--model")
    r.add_argument("--timeout", type=int, default=600)
    r.add_argument("text")
    args = ap.parse_args()
    return {
        "discover": cmd_discover, "forward": cmd_forward, "health": cmd_health,
        "list": cmd_list, "prompt": cmd_prompt, "run": cmd_run,
    }[args.cmd](args)


if __name__ == "__main__":
    sys.exit(main())
