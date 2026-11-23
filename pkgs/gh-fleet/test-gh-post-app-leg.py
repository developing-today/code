#!/usr/bin/env python3
"""Regression test for gh-post's App leg (issue #7935).

`laptop_paperwork()` ships fleet-paperwork arguments to the laptop as ONE
shell string. Before home-repo commit 02bf081ca it joined the arguments
unquoted, so the laptop shell split a multi-word `--title` and
fleet-paperwork died on unrecognized arguments — every titled App-route
create failed silently into the outbox.

This test reproduces the transport faithfully with stubs: a fake
laptop-ssh that hands its single argument to a real shell (`bash -c`),
exactly like ssh + a login shell do, and a fake fleet-paperwork that
reports the argv it received. No network, no GitHub, no laptop.

Run: python3 ~/tools/test-gh-post-app-leg.py  (exit 0 = pass)
"""
import hashlib
import importlib.machinery
import importlib.util
import json
import os
import stat
import sys
import tempfile
from pathlib import Path

GH_POST = Path.home() / "tools/gh-post"


def load_gh_post():
    loader = importlib.machinery.SourceFileLoader("gh_post", str(GH_POST))
    spec = importlib.util.spec_from_loader("gh_post", loader)
    mod = importlib.util.module_from_spec(spec)
    loader.exec_module(mod)
    return mod


def main() -> int:
    mod = load_gh_post()
    tmp = Path(tempfile.mkdtemp(prefix="gh-post-test-"))

    # Fake laptop-ssh: execute the one shell-string argument in a shell,
    # which is precisely where the unquoted title used to split.
    fake_ssh = tmp / "laptop-ssh.sh"
    fake_ssh.write_text('#!/bin/sh\nexec bash -c "$1"\n')
    fake_ssh.chmod(fake_ssh.stat().st_mode | stat.S_IEXEC)

    # Fake fleet-paperwork: report argv, plus a digest of the shipped
    # body file so the test also proves the body transfer still works.
    stub = tmp / "fleet_paperwork.py"
    stub.write_text(
        "import hashlib, json, sys\n"
        "args = sys.argv[1:]\n"
        "out = {'argv': args}\n"
        "if '--body-file' in args:\n"
        "    data = open(args[args.index('--body-file') + 1], 'rb').read()\n"
        "    out['body_sha256'] = hashlib.sha256(data).hexdigest()\n"
        "print(json.dumps(out))\n"
    )

    mod.LAPTOP_SSH = str(fake_ssh)
    mod.PAPERWORK = str(stub)

    body = tmp / "body.md"
    body.write_text("regression body for a multi word title\n")
    body_sha = hashlib.sha256(body.read_bytes()).hexdigest()

    failures = []

    # Create leg: the multi-word title must arrive as ONE argument.
    title = "a multi word title with several spaces"
    ok, out = mod.laptop_paperwork(
        ["create", "--repo", "developing-today/code", "--title", title,
         "--body-file", str(body)],
        body,
    )
    if not ok:
        failures.append(f"create leg transport failed: {out}")
    else:
        got = json.loads(out.strip().splitlines()[-1])
        argv = got["argv"]
        if argv[:4] != ["create", "--repo", "developing-today/code", "--title"] \
                or len(argv) < 5 or argv[4] != title:
            failures.append(f"title not intact in argv: {argv}")
        if len(argv) < 7 or argv[5] != "--body-file" or argv[6] == str(body):
            failures.append(f"body-file not remapped to remote path: {argv}")
        if got.get("body_sha256") != body_sha:
            failures.append("shipped body content mismatch")

    # Comment leg: single-token arguments behave exactly as before.
    ok, out = mod.laptop_paperwork(
        ["comment", "--repo", "developing-today/code", "--issue", "7935",
         "--body-file", str(body)],
        body,
    )
    if not ok:
        failures.append(f"comment leg transport failed: {out}")
    else:
        got = json.loads(out.strip().splitlines()[-1])
        if got["argv"][:6] != ["comment", "--repo", "developing-today/code",
                               "--issue", "7935", "--body-file"]:
            failures.append(f"comment argv changed: {got['argv']}")
        if got.get("body_sha256") != body_sha:
            failures.append("comment-leg body content mismatch")

    if failures:
        for f in failures:
            print(f"FAIL: {f}")
        return 1
    print("PASS: gh-post App leg keeps a multi-word title intact; "
          "comment leg unchanged; body transfer verified")
    return 0


if __name__ == "__main__":
    sys.exit(main())
