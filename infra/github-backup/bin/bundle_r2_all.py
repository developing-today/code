#!/usr/bin/env python3
"""Upload and verify one R2 bundle for every planned GitHub mirror.

Command line: [--bucket NAME] [--root PATH]. Arguments are parsed before
reading the plan or starting an upload. A failing repository does not prevent
later repositories from being attempted; the run exits non-zero if any fail.
"""

import argparse
import os
import sys

import bundle_r2
from ghb_common import backup_root, load_plan


DEFAULT_BUCKET = "supabase-backups"
FLEET_REPOS = (
    "developing-today/code",
    "developing-today/fleet",
    "developing-today-forks/openchamber",
    "developing-today-forks/opencode",
)


def run_all(root, bucket):
    plan, problems = load_plan(root)
    if problems or not plan:
        for problem in problems:
            print(f"github-backup: {problem}", file=sys.stderr)
        if not plan and not problems:
            print("github-backup: backup plan is empty", file=sys.stderr)
        return 1

    planned = {item["repo"] for item in plan}
    missing = [repo for repo in FLEET_REPOS if repo not in planned]
    if missing:
        print(
            "github-backup: fleet repos missing from backup plan: "
            + ", ".join(missing),
            file=sys.stderr,
        )
        return 1

    failed = False
    for repo in FLEET_REPOS:
        try:
            result = bundle_r2.upload_repo(repo, root, bucket)
        except (OSError, RuntimeError, ValueError) as exc:
            print(f"github-backup: {repo}: FAILED: {exc}", file=sys.stderr)
            failed = True
            continue
        print(f"github-backup: {repo}: verified R2 bundle ({result['head']})")
    return 1 if failed else 0


def _parser():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--bucket", default=DEFAULT_BUCKET, help="existing R2 bucket")
    parser.add_argument("--root", help="backup tree root (defaults to configured backup_root)")
    return parser


def main(argv=None):
    args = _parser().parse_args(argv)
    root = os.path.abspath(os.path.expanduser(args.root)) if args.root else backup_root()
    return run_all(root, args.bucket)


if __name__ == "__main__":
    raise SystemExit(main())
