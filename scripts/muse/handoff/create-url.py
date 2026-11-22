#!/usr/bin/env python3
"""Create a 15-minute, single-use Muse credential handoff URL."""
import argparse
import getpass
import json
import os
from pathlib import Path
import secrets
import subprocess
import sys
import urllib.error
import urllib.request
from datetime import datetime, timezone

CONFIG = Path(os.environ.get("XDG_CONFIG_HOME", Path.home() / ".config")) / "muse-agent-handoff"


def fail(message):
    print("muse-handoff: " + message, file=sys.stderr)
    raise SystemExit(1)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--name", help="stable Fleet agent name; generated if omitted")
    parser.add_argument("--bws-token-file", help="read the BWS access token from a local 0600 file")
    parser.add_argument("--op-from-bws-key", help="read the OP service token from this BWS secret key")
    args = parser.parse_args()

    name = args.name or "muse-" + datetime.now(timezone.utc).strftime("%Y%m%d") + "-" + secrets.token_hex(2)
    if not name or len(name) > 63 or not name[0].isalnum() or any(c not in "abcdefghijklmnopqrstuvwxyz0123456789._-" for c in name):
        fail("name must use lowercase letters, digits, dot, underscore, or hyphen")
    origin_file = CONFIG / "origin"
    admin_file = CONFIG / "admin-token"
    try:
        origin = origin_file.read_text().strip().rstrip("/")
        admin_token = admin_file.read_text().strip()
    except OSError:
        fail("generator is not configured; run scripts/muse/handoff/deploy.sh first")
    if not origin.startswith("https://muse-agent-handoff.") or not origin.endswith(".workers.dev"):
        fail("configured service origin is invalid")

    if args.bws_token_file:
        try:
            bws = Path(args.bws_token_file).expanduser().read_text().strip()
        except OSError:
            fail("could not read the selected BWS token file")
    else:
        bws = getpass.getpass("BWS access token: ").strip()
    if args.op_from_bws_key:
        env = os.environ.copy()
        env["BWS_ACCESS_TOKEN"] = bws
        try:
            result = subprocess.run(
                ["bws", "secret", "list", "-o", "json"],
                check=True, capture_output=True, text=True, env=env,
            )
            matches = [x for x in json.loads(result.stdout) if x.get("key") == args.op_from_bws_key]
        except Exception:
            fail("could not resolve the selected OP token from BWS")
        if len(matches) != 1 or not isinstance(matches[0].get("value"), str):
            fail("the selected BWS secret key must identify exactly one string value")
        op = matches[0]["value"].strip()
    else:
        op = getpass.getpass("1Password service-account token: ").strip()
    if not bws or not op:
        fail("both credentials are required")
    body = json.dumps({
        "name": name,
        "bws_access_token": bws,
        "op_service_account_token": op,
    }).encode()
    request = urllib.request.Request(
        origin + "/api/create",
        data=body,
        headers={
            "Authorization": "Bearer " + admin_token,
            "Content-Type": "application/json",
            "Cache-Control": "no-store",
        },
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=20) as response:
            result = json.loads(response.read())
    except urllib.error.HTTPError as error:
        fail("service rejected creation (HTTP " + str(error.code) + "); check deployment and local admin token")
    except Exception:
        fail("could not reach handoff service; credentials were not printed or saved")
    print("Agent name: " + name)
    print("Expires in: " + str(result.get("expires_in_seconds", 900)) + " seconds")
    print("One-time handoff URL (put this URL in the agent prompt):")
    print(result["claim_url"])


if __name__ == "__main__":
    main()
