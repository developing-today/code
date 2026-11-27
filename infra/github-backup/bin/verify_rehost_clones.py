#!/usr/bin/env python3
"""Clone every live rehost endpoint and compare its full ref set to source.

Command line: --root PATH. The complete command line is parsed strictly
before creating a temporary directory or starting a subprocess (#3352).
The oracle is exact equality of the live clone's HEAD symref, HEAD oid, and
heads/tags (including peeled tag targets) with the reconstructed GitHub source
mirror. This verifies the Worker and R2 data path through real HTTPS clones.
"""

import argparse
import os
import subprocess
import sys
import tempfile

import rehost_publish
from ghb_common import mirror_path, valid_mirror
from plain_mirror import FLEET_REPOS

BASE_URL = "https://git.security.cab"
CLONE_TIMEOUT_SECONDS = 3600


def _run_git(args):
    env = dict(os.environ, GIT_TERMINAL_PROMPT="0")
    try:
        result = subprocess.run(
            ["git", *args],
            capture_output=True,
            text=True,
            timeout=CLONE_TIMEOUT_SECONDS,
            stdin=subprocess.DEVNULL,
            env=env,
        )
    except subprocess.TimeoutExpired as exc:
        raise RuntimeError(f"git {' '.join(args[:3])} timed out") from exc
    if result.returncode:
        detail = result.stderr.strip() or result.stdout.strip()
        raise RuntimeError(f"git {' '.join(args[:3])} failed: {detail[:400]}")
    return result.stdout.strip()


def verify_repo(repo, root, staging):
    """Return the verified source HEAD after a complete live clone check."""
    mirror = mirror_path(root, repo)
    if not valid_mirror(mirror):
        raise ValueError(f"source mirror is invalid: {mirror}")
    expected = rehost_publish.collect_refs(mirror)
    owner, name = repo.split("/", 1)
    with tempfile.TemporaryDirectory(prefix=f"{owner}-{name}.", dir=staging) as work:
        clone = os.path.join(work, "repo.git")
        _run_git(["clone", "--bare", f"{BASE_URL}/{repo}", clone])
        actual = rehost_publish.collect_refs(clone)
    if actual != expected:
        raise RuntimeError(
            f"live clone refs differ from source mirror for {repo} "
            f"(source head={expected[1]}, live head={actual[1]})"
        )
    return expected[1], len(expected[2])


def run_all(root):
    failures = []
    with tempfile.TemporaryDirectory(prefix="github-backup-rehost-verify-") as staging:
        for repo in FLEET_REPOS:
            try:
                head, refs = verify_repo(repo, root, staging)
            except (OSError, RuntimeError, ValueError) as exc:
                print(
                    f"github-backup: rehost clone {repo}: FAILED: {exc}",
                    file=sys.stderr,
                )
                failures.append(repo)
                continue
            print(
                f"github-backup: rehost clone {repo}: verified HEAD {head} "
                f"({refs} heads/tags)"
            )
    if failures:
        print(
            "github-backup: rehost clone failures: " + ", ".join(failures),
            file=sys.stderr,
        )
        return 1
    return 0


def _parser():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--root", required=True, help="hosted backup tree root")
    return parser


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    try:
        args = _parser().parse_args(argv)
    except SystemExit as exc:
        return int(exc.code or 0)
    root = os.path.abspath(os.path.expanduser(args.root))
    return run_all(root)


if __name__ == "__main__":
    raise SystemExit(main())
