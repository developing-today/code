#!/usr/bin/env python3
"""Repair the three shell-substitution manglings in PR #13915's body, in place."""

import json, subprocess, sys

REPO = "developing-today/code"
PR = "13915"

out = subprocess.run(
    ["gh", "api", f"repos/{REPO}/pulls/{PR}", "--jq", ".body"],
    capture_output=True,
    text=True,
    check=True,
)
body = out.stdout
if body.endswith("\n"):
    body_core = body[:-1]
else:
    body_core = body

splice = (
    "verified accurate against live fleet bridge — Fleet messaging through the message bridge: "
    "central sends messages or commands to an agent, a group or all; agents read their mailbox, "
    "answer and ack.\n\nUsage: fleet bridge <command> [args...]\n"
    "Commands: send, agents, log, hello, inbox, post, ack, health\n"
    "Status: included in this build (options: fleet bridge <command> --help) and "
    "pkgs/message-bridge source, except:"
)
fixes = [
    (
        "## (A) Commander worker — new chapter\n, registered in SUMMARY",
        "## (A) Commander worker — new chapter\n`pkgs/fleet/book/concepts/commander.md`, registered in SUMMARY",
    ),
    (
        splice,
        "verified accurate against live `fleet bridge --help` and pkgs/message-bridge source, except:",
    ),
    (
        "-  green: /nix/store/69zsm1mffr27fzkc8xp30lq0469ypyhf-fleet-docs",
        "- `nix build .#fleet-docs` green: /nix/store/69zsm1mffr27fzkc8xp30lq0469ypyhf-fleet-docs",
    ),
]
for old, new in fixes:
    n = body_core.count(old)
    if n != 1:
        print(f"MISS ({n}x): {old[:60]!r}")
        sys.exit(1)
    body_core = body_core.replace(old, new)
    print(f"OK: {old[:50]!r}")

payload = json.dumps({"body": body_core})
res = subprocess.run(
    ["gh", "api", "-X", "PATCH", f"repos/{REPO}/pulls/{PR}", "--input", "-"],
    input=payload,
    capture_output=True,
    text=True,
)
if res.returncode != 0:
    print("PATCH FAILED:", res.stderr[:500])
    sys.exit(1)
print("PR body repaired")
