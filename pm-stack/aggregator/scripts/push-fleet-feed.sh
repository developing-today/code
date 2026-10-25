#!/usr/bin/env bash
# Collect the fleet feed on this (sandbox) host and push it to the
# laptop aggregator, over the same tailnet proxy route laptop-ssh.sh
# uses (socat PROXY via hatch-egress-proxy:3130). The proxy credential
# comes from this process's HTTPS_PROXY when present (agent runs), or
# from ~/tools/secrets.d/mcpx-daemon.env (systemd timer runs) — it is
# read at runtime and never stored here.
#
# Cadence: ~5 min via fleet-feed-collector.timer (systemd, sandbox).
set -euo pipefail

cd "$(dirname "$0")/.."
PKG="$PWD"
FEED="$PKG/fleet-feed.json"
LAPTOP=user@100.99.113.103
DEST_DIR="~/pm-stack/aggregator"

bun run scripts/collect-fleet-feed.ts --out "$FEED"

exec python3 - "$FEED" "$LAPTOP" "$DEST_DIR" <<'PYEOF'
import os, re, subprocess, sys, urllib.parse

feed, laptop, dest_dir = sys.argv[1], sys.argv[2], sys.argv[3]

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
proxycmd = f"socat - PROXY:{u.hostname}:100.99.113.103:22,proxyport=3130,proxyauth={user}:{pw}"
ssh_opts = [
    "-o", "StrictHostKeyChecking=no",
    "-o", "UserKnownHostsFile=/dev/null",
    "-o", "IdentitiesOnly=yes",
    "-i", os.path.expanduser("~/.ssh/id_ed25519"),
    "-o", f"ProxyCommand={proxycmd}",
    "-o", "ConnectTimeout=25",
    "-o", "ControlMaster=auto",
    "-o", "ControlPath=" + os.path.expanduser("~/.ssh/cm-%r@%h:%p"),
    "-o", "ControlPersist=120",
]
subprocess.run(
    ["scp", *ssh_opts, feed, f"{laptop}:{dest_dir}/fleet-feed.json.tmp"],
    check=True,
)
subprocess.run(
    ["ssh", *ssh_opts, laptop, f"mv {dest_dir}/fleet-feed.json.tmp {dest_dir}/fleet-feed.json"],
    check=True,
)
print("push-fleet-feed: feed pushed to laptop")
PYEOF
