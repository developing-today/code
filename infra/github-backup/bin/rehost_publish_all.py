#!/usr/bin/env python3
"""Publish every fleet repo's mirror to the Cloudflare rehost bucket.

Command line: [--bucket NAME] [--root PATH]. Arguments are parsed
before reading the plan or starting a publish (#3352). A failing
repository does not prevent later repositories from being attempted;
the run exits non-zero if any fail (same contract as bundle_r2_all).
"""

import argparse
import os
import sys

import rehost_publish
from ghb_common import backup_root, load_plan
from plain_mirror import FLEET_REPOS


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
            doc = rehost_publish.publish_repo(repo, root, bucket)
        except (OSError, RuntimeError, ValueError) as exc:
            print(f"github-backup: {repo}: FAILED: {exc}", file=sys.stderr)
            failed = True
            continue
        print(f"github-backup: {repo}: verified rehost publish ({doc['head']})")
    return 1 if failed else 0


def _parser():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument(
        "--bucket", default=rehost_publish.DEFAULT_BUCKET, help="existing R2 bucket"
    )
    parser.add_argument(
        "--root", help="backup tree root (defaults to configured backup_root)"
    )
    return parser


def main(argv=None):
    args = _parser().parse_args(argv)
    root = (
        os.path.abspath(os.path.expanduser(args.root)) if args.root else backup_root()
    )
    return run_all(root, args.bucket)


if __name__ == "__main__":
    raise SystemExit(main())
