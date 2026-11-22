#!/usr/bin/env python3
"""Create and locally verify one cold git bundle from a github-backup mirror.

Command line: --repo OWNER/REPO [--root PATH]. Arguments are parsed strictly
before any subprocess or filesystem write. This local drill does not upload to
R2; the bundle is published below <root>/cold-bundles only after verification.
"""

import argparse
import os
import re
import subprocess
import sys
import tempfile

from ghb_common import RUN_TIMEOUT_SECONDS, backup_root, load_plan, mirror_path, valid_mirror


REPO_PART = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_.-]*$")


def _run_git(args):
    try:
        result = subprocess.run(
            ["git", *args],
            capture_output=True,
            text=True,
            timeout=RUN_TIMEOUT_SECONDS,
        )
    except subprocess.TimeoutExpired as exc:
        raise RuntimeError(f"git {' '.join(args)} timed out") from exc
    if result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip()
        raise RuntimeError(f"git {' '.join(args)} failed: {detail[:400]}")
    return result.stdout.strip()


def create_bundle(mirror, bundle_path):
    """Write a bundle only if Git can verify and clone it at the same HEAD."""
    if not valid_mirror(mirror):
        raise ValueError(f"not a valid mirror: {mirror}")

    bundle_path = os.path.abspath(bundle_path)
    parent = os.path.dirname(bundle_path)
    os.makedirs(parent, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".bundle-stage-", dir=parent) as stage_dir:
        staged_bundle = os.path.join(stage_dir, "backup.bundle")
        expected_head = _run_git([f"--git-dir={mirror}", "rev-parse", "HEAD"])
        _run_git([f"--git-dir={mirror}", "bundle", "create", staged_bundle, "--all"])
        verifier_path = os.path.join(stage_dir, "empty.git")
        _run_git(["init", "--bare", verifier_path])
        _run_git([f"--git-dir={verifier_path}", "bundle", "verify", staged_bundle])
        clone_path = os.path.join(stage_dir, "clone.git")
        _run_git(["clone", "--bare", staged_bundle, clone_path])
        restored_head = _run_git([f"--git-dir={clone_path}", "rev-parse", "HEAD"])
        if restored_head != expected_head:
            raise RuntimeError(
                f"bundle clone HEAD {restored_head} does not match mirror HEAD {expected_head}"
            )
        os.replace(staged_bundle, bundle_path)
        return {"status": "ok", "head": expected_head, "bundle": bundle_path}


def _parser():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--repo", required=True, help="planned repository as OWNER/REPO")
    parser.add_argument("--root", help="backup tree root (defaults to configured backup_root)")
    return parser


def main(argv=None):
    args = _parser().parse_args(argv)
    parts = args.repo.split("/")
    if len(parts) != 2 or not all(REPO_PART.fullmatch(part) for part in parts):
        print("github-backup: --repo must be OWNER/REPO using safe path components", file=sys.stderr)
        return 2

    root = os.path.abspath(os.path.expanduser(args.root)) if args.root else backup_root()
    plan, problems = load_plan(root)
    if problems:
        for problem in problems:
            print(f"github-backup: {problem}", file=sys.stderr)
    entry = next((item for item in plan if item["repo"] == args.repo), None)
    if entry is None:
        print(f"github-backup: {args.repo} is not in the backup plan", file=sys.stderr)
        return 1

    mirror = mirror_path(root, args.repo)
    bundle = os.path.join(root, "cold-bundles", *parts[:-1], parts[-1] + ".bundle")
    try:
        result = create_bundle(mirror, bundle)
    except (OSError, RuntimeError, ValueError) as exc:
        print(f"github-backup: {args.repo}: {exc}", file=sys.stderr)
        return 1
    print(f"github-backup: {args.repo}: verified bundle at {result['bundle']} ({result['head']})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
