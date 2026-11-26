#!/usr/bin/env python3
"""Synchronize and verify every planned fleet plain Git mirror.

Command line: [--root PATH]. The whole command line is declared and
parsed strictly before any sync starts (#3352 operator-script argument
contract); anything unaccounted for is a usage error. The backup root
defaults to the normal github-backup configuration; a hosted runner
passes --root to point the driver at its ephemeral storage instead
(#15000: the #14964 design requires this adapter, because the hosted
publisher cannot resolve a root from laptop configuration). Every
repository is attempted; any failure makes the run fail after
reporting the remaining results.
"""

import argparse
import os
import sys

from ghb_common import backup_root
from plain_mirror import FLEET_REPOS, mirror_repo


def run_all(root):
    failures = []
    for repo in FLEET_REPOS:
        try:
            result = mirror_repo(repo, root)
            print(
                f"github-backup: {repo}: verified plain mirror ({result['refs']} refs)"
            )
        except (OSError, RuntimeError, ValueError) as exc:
            print(f"github-backup: {repo}: {exc}", file=sys.stderr)
            failures.append(repo)

    if failures:
        print(
            "github-backup: plain mirror failures: " + ", ".join(failures),
            file=sys.stderr,
        )
        return 1
    return 0


def _parser():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument(
        "--root", help="backup tree root (defaults to configured backup_root)"
    )
    return parser


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    try:
        args = _parser().parse_args(argv)
    except SystemExit as exc:
        # argparse exits 2 on an unaccounted argument and 0 on --help;
        # preserve that contract without raising through callers/tests.
        return int(exc.code or 0)
    root = (
        os.path.abspath(os.path.expanduser(args.root)) if args.root else backup_root()
    )
    return run_all(root)


if __name__ == "__main__":
    raise SystemExit(main())
