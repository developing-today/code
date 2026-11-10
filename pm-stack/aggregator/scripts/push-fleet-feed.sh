#!/usr/bin/env bash
# Collect the fleet feed on this (sandbox) host and push it to the
# laptop aggregator over the tailnet proxy route (hatch-egress-proxy:3130).
#
# Security (issue #948):
# - The proxy credential is NEVER passed as a process argument. The
#   Python process below reads it from HTTPS_PROXY (agent runs) or
#   ~/tools/secrets.d/mcpx-daemon.env (systemd timer runs), opens an
#   HTTP CONNECT tunnel to the laptop itself, and sends the credential
#   only on the wire, in the Proxy-Authorization header. ssh/scp talk
#   to a throwaway 127.0.0.1 forwarder owned by that same process, so
#   no spawned process (ssh, scp, or any tunnel helper) carries the
#   credential in its argv.
# - The laptop host key is pinned: scripts/laptop_known_hosts is
#   checked in, and ssh verifies against it with
#   StrictHostKeyChecking=yes + HostKeyAlias=<laptop tailnet IP>.
#   A mismatched key fails the push.
# Guard: scripts/push-fleet-feed.guard.sh fails if either pattern
# regresses.
#
# RETIRED as a scheduled producer (code#3362/#3363): the collector moved
# to the laptop on 2026-10-31 and runs there as a user timer declared in
# the pm-stack NixOS module; the sandbox fleet-feed-collector timer that
# ran this script every ~5 min is deleted. Kept for manual/emergency use
# only -- running it overwrites the laptop feed with this host's collection.
set -euo pipefail

# Systemd system services get a minimal PATH (and no HOME); bun lives
# in the image bin.
if ! command -v bun >/dev/null 2>&1; then
  for d in /opt/hatch-image/bin "${HOME:-/nonexistent}/tools/go/bin"; do
    [ -x "$d/bun" ] && PATH="$d:$PATH" && break
  done
fi
export PATH

cd "$(dirname "$0")/.."
PKG="$PWD"
LAPTOP=user@100.99.113.103
# Write directly to the state directory. Keep the laptop's
# ~/pm-stack/aggregator -> ~/.local/state/pm-aggregator symlink in place
# until the sandbox has pulled this change and feed delivery is verified;
# older deployed push scripts still depend on that bridge.
DEST_DIR="~/.local/state/pm-aggregator"
KNOWN_HOSTS="${PUSH_FLEET_FEED_KNOWN_HOSTS:-$PKG/scripts/laptop_known_hosts}"

# The collector itself needs egress under the systemd unit's minimal
# env: Bitwarden (the GitHub Project token for the projects section)
# and api.github.com (the Project pool GraphQL read). Reuse the same
# proxy credential the push below uses, exported as env only — never
# argv (the #948 rule the guard enforces). NO_PROXY keeps the
# collector's loopback/tailnet reads (sandbox OpenCode server, laptop)
# off the proxy.
if [ -z "${HTTPS_PROXY:-}" ] && [ -f "$HOME/tools/secrets.d/mcpx-daemon.env" ]; then
  _proxy_url="$(python3 -c 'import os, re
try:
    t = open(os.path.expanduser("~/tools/secrets.d/mcpx-daemon.env")).read()
except OSError:
    t = ""
m = re.search(r"(https?://[^\s\"\x27]+@[^\s\"\x27]+)", t)
print(m.group(1) if m else "")' 2>/dev/null || true)"
  if [ -n "$_proxy_url" ]; then
    export HTTPS_PROXY="$_proxy_url" https_proxy="$_proxy_url"
  fi
  unset _proxy_url
fi
export NO_PROXY="${NO_PROXY:-127.0.0.1,localhost,100.99.113.103}" no_proxy="${no_proxy:-127.0.0.1,localhost,100.99.113.103}"

# Serialize the read/publish/confirm transaction for this state file. A
# standalone collection never confirms anything. Each push owns its feed, so
# another collector cannot replace the bytes being sent or acknowledged.
STATE="${CHARGES_SURFACED_STATE_PATH:-$HOME/workspace/goals/keep-the-four-repos-healthy/hidden_files/charges-surfaced-state.json}"
mkdir -p "$(dirname "$STATE")"
exec 9>"$STATE.lock"
flock 9
FEED="$(mktemp "$PKG/.fleet-feed.XXXXXXXX.json")"
trap 'rm -f "$FEED"' EXIT
bun run scripts/collect-fleet-feed.ts --out "$FEED"

python3 - "$FEED" "$LAPTOP" "$DEST_DIR" "$KNOWN_HOSTS" <<'PYEOF'
import base64, os, re, socket, subprocess, sys, threading, urllib.parse

feed, laptop, dest_dir, known_hosts = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
target_host = laptop.split("@", 1)[-1]
target_port = 22
# Tailnet tunnel port on the egress proxy (same route the old external
# proxy-helper command used; the HTTPS_PROXY URL port is the general
# egress port).
PROXY_PORT = 3130

if not os.path.isfile(known_hosts) or os.path.getsize(known_hosts) == 0:
    sys.exit(f"push-fleet-feed: pinned known_hosts missing/empty: {known_hosts}")

proxy = os.environ.get("HTTPS_PROXY") or os.environ.get("https_proxy")
if not proxy:
    try:
        env_file = open(os.path.expanduser("~/tools/secrets.d/mcpx-daemon.env")).read()
        m = re.search(r"(https?://[^\s\"']+@[^\s\"']+)", env_file)
        proxy = m.group(1) if m else None
    except OSError:
        proxy = None
if not proxy:
    sys.exit("push-fleet-feed: no proxy credential available (HTTPS_PROXY / mcpx-daemon.env)")

u = urllib.parse.urlparse(proxy)
user = urllib.parse.unquote(u.username or "")
pw = urllib.parse.unquote(u.password or "")
# Credential exists only in this process's memory and on the proxy wire.
auth = base64.b64encode(f"{user}:{pw}".encode()).decode()


def open_tunnel():
    """CONNECT through the egress proxy to the laptop sshd.
    Returns (socket, leftover_bytes_received_after_the_CONNECT_reply)."""
    s = socket.create_connection((u.hostname, PROXY_PORT), timeout=25)
    req = (
        f"CONNECT {target_host}:{target_port} HTTP/1.1\r\n"
        f"Host: {target_host}:{target_port}\r\n"
        f"Proxy-Authorization: Basic {auth}\r\n"
        "Proxy-Connection: keep-alive\r\n\r\n"
    )
    s.sendall(req.encode())
    buf = b""
    while b"\r\n\r\n" not in buf:
        chunk = s.recv(4096)
        if not chunk:
            s.close()
            raise OSError("proxy closed during CONNECT")
        buf += chunk
    head, leftover = buf.split(b"\r\n\r\n", 1)
    status = head.split(b"\r\n", 1)[0].decode("latin1")
    if " 200" not in status:
        s.close()
        raise OSError(f"proxy CONNECT failed: {status}")
    return s, leftover


def relay(src, dst, initial=b""):
    try:
        if initial:
            dst.sendall(initial)
        while True:
            data = src.recv(65536)
            if not data:
                break
            dst.sendall(data)
    except OSError:
        pass
    finally:
        try:
            dst.shutdown(socket.SHUT_WR)
        except OSError:
            pass


def handle(local):
    try:
        remote, leftover = open_tunnel()
    except OSError as e:
        print(f"push-fleet-feed: tunnel failed: {e}", file=sys.stderr)
        local.close()
        return
    t = threading.Thread(target=relay, args=(remote, local, leftover), daemon=True)
    t.start()
    relay(local, remote)
    t.join(timeout=5)
    local.close()
    remote.close()


listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
listener.bind(("127.0.0.1", 0))
listener.listen(5)
local_port = listener.getsockname()[1]


def accept_loop():
    while True:
        try:
            conn, _ = listener.accept()
        except OSError:
            return
        threading.Thread(target=handle, args=(conn,), daemon=True).start()


threading.Thread(target=accept_loop, daemon=True).start()

ssh_opts = [
    "-o", "StrictHostKeyChecking=yes",
    "-o", f"UserKnownHostsFile={known_hosts}",
    "-o", f"HostKeyAlias={target_host}",
    "-o", "IdentitiesOnly=yes",
    "-i", os.path.expanduser("~/.ssh/id_ed25519"),
    "-o", "ConnectTimeout=25",
    # No ControlMaster/ControlPersist: the tunnel lives in this process,
    # so a persisted master connection would outlive its forwarder.
    "-o", "ControlMaster=no",
]
dest = f"{laptop.split('@', 1)[0]}@127.0.0.1"
remote_tmp = f"{dest_dir}/{os.path.basename(feed)}.tmp"
try:
    subprocess.run(
        ["scp", "-P", str(local_port), *ssh_opts, feed,
         f"{dest}:{remote_tmp}"],
        check=True,
    )
    subprocess.run(
        ["ssh", "-p", str(local_port), *ssh_opts, dest,
         f"mv {remote_tmp} {dest_dir}/fleet-feed.json"],
        check=True,
    )
finally:
    listener.close()
print("push-fleet-feed: feed pushed to laptop")
PYEOF
bun run scripts/confirm-charges-surfaced.ts "$FEED"
