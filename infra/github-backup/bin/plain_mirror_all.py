#!/usr/bin/env python3
"""Synchronize and verify every planned fleet plain Git mirror.

Command line: no arguments. The backup root is resolved from the normal
github-backup configuration. Every repository is attempted; any failure makes
the run fail after reporting the remaining results.
"""

import sys

from ghb_common import backup_root
from plain_mirror import FLEET_REPOS, mirror_repo


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    if argv:
        print(f"plain_mirror_all.py: unexpected argument: {argv[0]!r}", file=sys.stderr)
        print("Usage: plain_mirror_all.py (no arguments)", file=sys.stderr)
        return 2

    failures = []
    for repo in FLEET_REPOS:
        try:
            result = mirror_repo(repo, backup_root())
            print(f"github-backup: {repo}: verified plain mirror ({result['refs']} refs)")
        except (OSError, RuntimeError, ValueError) as exc:
            print(f"github-backup: {repo}: {exc}", file=sys.stderr)
            failures.append(repo)

    if failures:
        print("github-backup: plain mirror failures: " + ", ".join(failures), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
