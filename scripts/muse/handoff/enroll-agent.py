#!/usr/bin/env python3
"""Consume a one-time Muse credential handoff and enroll the agent."""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import urllib.error
import urllib.parse
import urllib.request

HANDOFF_ORIGIN_HOST_PREFIX = "muse-agent-handoff."
HANDOFF_ORIGIN_HOST_SUFFIX = ".workers.dev"
RAW_ROOT = "https://raw.githubusercontent.com/developing-today/code/main/scripts/muse/"
BOOTSTRAP_URL = RAW_ROOT + "bootstrap.sh"


def fail(message, code=1):
    print("muse-enroll: " + message, file=sys.stderr)
    raise SystemExit(code)


def validate_name(name):
    if not name or len(name) > 63 or not re.fullmatch(r"[a-z0-9][a-z0-9._-]*", name):
        fail("name must use lowercase letters, digits, dot, underscore, or hyphen")
    return name


def secure_write(path, value):
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    os.chmod(path.parent, 0o700)
    fd, temporary = tempfile.mkstemp(prefix=".handoff-", dir=path.parent)
    try:
        os.fchmod(fd, 0o600)
        with os.fdopen(fd, "w") as target:
            target.write(value)
            target.write("\n")
            target.flush()
            os.fsync(target.fileno())
        os.replace(temporary, path)
        os.chmod(path, 0o600)
    finally:
        try:
            os.unlink(temporary)
        except FileNotFoundError:
            pass


def get_credentials(args, config):
    if args.resume:
        try:
            return {
                "bws_access_token": (config / "bitwarden/access-token")
                .read_text()
                .strip(),
                "op_service_account_token": (config / "op/service-account-token")
                .read_text()
                .strip(),
                "agent_name": args.name,
            }
        except OSError:
            fail("resume files are missing; create a new handoff URL")
    parsed = urllib.parse.urlsplit(args.handoff_url)
    if parsed.scheme != "https" or parsed.query or parsed.username or parsed.password:
        fail("handoff URL must be HTTPS with no query string")
    if (
        not parsed.hostname
        or not parsed.hostname.startswith(HANDOFF_ORIGIN_HOST_PREFIX)
        or not parsed.hostname.endswith(HANDOFF_ORIGIN_HOST_SUFFIX)
    ):
        fail("URL is not from the Muse handoff service")
    if parsed.path not in ("", "/"):
        fail("handoff URL path is invalid")
    parts = parsed.fragment.split(".")
    if len(parts) != 2:
        fail("URL is missing its one-time claim code")
    handoff_id, claim = parts
    if not re.fullmatch(r"[0-9a-f-]{36}", handoff_id) or not re.fullmatch(
        r"[A-Za-z0-9_-]{40,48}", claim
    ):
        fail("one-time claim code is malformed")
    request = urllib.request.Request(
        "https://" + parsed.netloc + "/api/claim",
        data=json.dumps({"id": handoff_id, "claim": claim}).encode(),
        headers={
            "Content-Type": "application/json",
            "Cache-Control": "no-store",
            "User-Agent": "muse-enroll/1.1 (fleet enrollment helper)",
        },
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=20) as response:
            credentials = json.loads(response.read())
    except urllib.error.HTTPError as error:
        if error.code == 410:
            fail("handoff is expired or was already used; generate a fresh URL")
        fail("handoff service returned HTTP " + str(error.code))
    except Exception:
        fail(
            "could not reach the handoff service; if this link may have been consumed, generate a fresh one"
        )
    if credentials.get("agent_name") != args.name:
        fail("handoff is for a different agent name; do not enroll with this URL")
    for key in ("bws_access_token", "op_service_account_token"):
        if not isinstance(credentials.get(key), str) or not credentials[key]:
            fail("handoff payload is incomplete; generate a fresh URL")
    return credentials


def download(url, destination):
    request = urllib.request.Request(
        url,
        headers={
            "Cache-Control": "no-store",
            "User-Agent": "muse-enroll/1.1 (fleet enrollment helper)",
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            data = response.read(2_000_001)
    except Exception:
        fail("could not download the pinned bootstrap script")
    if not data or len(data) > 2_000_000:
        fail("bootstrap download was empty or unexpectedly large")
    destination.write_bytes(data)
    os.chmod(destination, 0o600)


def run(args):
    name = validate_name(args.name)
    kind = validate_name(args.kind)
    home = Path.home()
    config = home / ".config"
    credentials = get_credentials(args, config)
    secure_write(config / "bitwarden/access-token", credentials["bws_access_token"])
    secure_write(
        config / "op/service-account-token", credentials["op_service_account_token"]
    )

    env = os.environ.copy()
    env.update(
        {
            "HOME": str(home),
            "XDG_CONFIG_HOME": str(config),
            "FLEET_AGENT": name,
            "BWS_ACCESS_TOKEN": credentials["bws_access_token"],
            "OP_SERVICE_ACCOUNT_TOKEN": credentials["op_service_account_token"],
        }
    )
    del credentials
    saved_bootstrap = home / "tools/muse-bootstrap.sh"
    with tempfile.TemporaryDirectory(prefix=".muse-enroll-", dir=home) as temp:
        bootstrap = Path(temp) / "bootstrap.sh"
        if args.resume and saved_bootstrap.is_file():
            bootstrap = saved_bootstrap
        else:
            download(BOOTSTRAP_URL, bootstrap)
        if args.resume:
            print("Resuming the idempotent bootstrap with saved 0600 credentials.")
        else:
            print(
                "Running the pinned bootstrap (tools, token, tailnet, Fleet, and checks"
                + (", plus the Muse home restore" if kind == "muse" else "")
                + ")."
            )
        if kind == "muse":
            bootstrap_args = ["--all"]
        else:
            # A non-Muse seat must never restore the Muse home: skip the
            # restore, persist, and init phases (code#12858).
            bootstrap_args = ["--skip", "restore,persist,init"]
        completed = subprocess.run(
            ["bash", str(bootstrap), *bootstrap_args], env=env
        )
    if completed.returncode:
        print(
            "Bootstrap failed with exit " + str(completed.returncode) + ".",
            file=sys.stderr,
        )
        print(
            "Credentials are saved in 0600 files. Diagnose the named failed phase, then rerun:",
            file=sys.stderr,
        )
        print(
            "python3 enroll-agent.py --name "
            + name
            + " --kind "
            + kind
            + " --resume",
            file=sys.stderr,
        )
        raise SystemExit(completed.returncode)

    fleet = home / "tools/fleet"
    if not fleet.is_file():
        fail(
            "bootstrap completed but ~/tools/fleet is missing; rerun with --resume after repairing the fleet phase"
        )
    profile = config / "fleet/agent.json"
    enrolled_name = None
    try:
        enrolled_name = json.loads(profile.read_text()).get("name")
    except Exception:
        pass
    if enrolled_name != name:
        enrolled = subprocess.run(
            [str(fleet), "agent", "enroll", "--kind", kind, "--name", name], env=env
        )
        if enrolled.returncode:
            print(
                "Fleet enrollment did not verify. Preserve this name and run the doctor before retrying.",
                file=sys.stderr,
            )
            raise SystemExit(enrolled.returncode)
    else:
        print(
            "Fleet profile already exists for "
            + name
            + "; not enrolling it a second time."
        )
    doctor = subprocess.run([str(fleet), "agent", "doctor", "--agent", name], env=env)
    if doctor.returncode:
        print(
            "Enrollment profile exists, but one or more reachability checks failed. Record each failed leg and resume after fixing it.",
            file=sys.stderr,
        )
        raise SystemExit(doctor.returncode)
    print(
        "Enrollment complete: "
        + name
        + " is registered and the doctor found a usable intake rung."
    )
    print(
        "Next: run ~/tools/fleet agent next and follow the returned task; exit 5 means no task is available."
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--name", required=True, help="stable agent name bound to the handoff"
    )
    parser.add_argument(
        "--kind",
        default="muse",
        help="fleet agent kind to enroll (default: muse; non-muse kinds skip the Muse home restore)",
    )
    parser.add_argument(
        "--handoff-url", help="15-minute single-use URL; omitted with --resume"
    )
    parser.add_argument(
        "--resume",
        action="store_true",
        help="reuse locally saved credentials after a failed phase",
    )
    args = parser.parse_args()
    if args.resume == bool(args.handoff_url):
        fail("provide exactly one of --handoff-url or --resume")
    run(args)


if __name__ == "__main__":
    main()
