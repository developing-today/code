#!/usr/bin/env python3
"""Reconstruct the four hosted-publisher inputs directly from GitHub.

Command line: --root PATH. The complete command line is parsed strictly
before creating the root or starting a subprocess (#3352). Each source is
fetched into an ephemeral bare mirror from GitHub, then validated; a complete
plan.json is written atomically only after every required mirror is valid.
This deliberately does not read laptop mirrors or reuse local alternates.
"""

import argparse
import os
import subprocess
import sys

from ghb_common import mirror_path, save_json, valid_mirror
from plain_mirror import FLEET_REPOS

GIT_TIMEOUT_SECONDS = 1800


def _git(args):
    env = dict(os.environ, GIT_TERMINAL_PROMPT="0")
    try:
        result = subprocess.run(
            ["git", *args],
            capture_output=True,
            text=True,
            timeout=GIT_TIMEOUT_SECONDS,
            env=env,
        )
    except subprocess.TimeoutExpired as exc:
        raise RuntimeError(f"git {' '.join(args[:3])} timed out") from exc
    if result.returncode:
        detail = result.stderr.strip() or result.stdout.strip()
        raise RuntimeError(f"git {' '.join(args[:3])} failed: {detail[:400]}")


def run_all(root):
    """Fetch every required input; return nonzero without a partial plan."""
    os.makedirs(root, exist_ok=True)
    failures = []
    verified = []
    for repo in FLEET_REPOS:
        destination = mirror_path(root, repo)
        try:
            os.makedirs(os.path.dirname(destination), exist_ok=True)
            _git(
                [
                    "clone",
                    "--mirror",
                    "--",
                    f"https://github.com/{repo}.git",
                    destination,
                ]
            )
            if not valid_mirror(destination):
                raise RuntimeError("fresh GitHub mirror failed bare/HEAD validation")
        except (OSError, RuntimeError, ValueError) as exc:
            print(f"github-backup: hosted source {repo}: {exc}", file=sys.stderr)
            failures.append(repo)
            continue
        print(f"github-backup: hosted source {repo}: fetched and verified")
        verified.append(repo)

    if failures:
        print(
            "github-backup: hosted source failures: " + ", ".join(failures),
            file=sys.stderr,
        )
        return 1

    try:
        save_json(os.path.join(root, "plan.json"), [{"repo": repo} for repo in verified])
    except OSError as exc:
        print(f"github-backup: could not write hosted source plan: {exc}", file=sys.stderr)
        return 1
    return 0


def _parser():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--root", required=True, help="ephemeral hosted backup tree root")
    return parser


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    try:
        args = _parser().parse_args(argv)
    except SystemExit as exc:
        # argparse exits 2 on unaccounted input and 0 on --help.
        return int(exc.code or 0)
    root = os.path.abspath(os.path.expanduser(args.root))
    return run_all(root)


if __name__ == "__main__":
    raise SystemExit(main())
