#!/usr/bin/env python3
"""Synchronize one planned local Git mirror to its plain mirror destination.

Command line: --repo OWNER/REPO [--root PATH]. The destination is the
pre-created `developing-today/<repo>-mirror` repository; this command never
creates or deletes a remote repository. The destination's refs are made an
exact mirror of the planned local source and are checked after the push.
"""

import argparse
import os
import re
import subprocess
import sys
from urllib.parse import urlparse

from ghb_common import backup_root, load_plan, mirror_path, valid_mirror


FLEET_REPOS = (
    "developing-today/code",
    "developing-today/fleet",
    "developing-today-forks/openchamber",
    "developing-today-forks/opencode",
)
REPO_PART = re.compile(r"^[A-Za-z0-9_.-]+$")
GIT_TIMEOUT_SECONDS = 1800


def _git(args, *, timeout=GIT_TIMEOUT_SECONDS):
    try:
        result = subprocess.run(
            ["git", *args], capture_output=True, text=True, timeout=timeout
        )
    except subprocess.TimeoutExpired as exc:
        raise RuntimeError(f"git {' '.join(args[:3])} timed out") from exc
    if result.returncode:
        detail = result.stderr.strip() or result.stdout.strip()
        raise RuntimeError(f"git {' '.join(args[:3])} failed: {detail[:400]}")
    return result.stdout.strip()


def _refs(source, *, remote=False):
    if remote:
        output = _git(["ls-remote", "--refs", source])
        refs = output.splitlines()
        return {
            fields[1]: fields[0]
            for line in refs
            if len(fields := line.split()) == 2
            and fields[1].startswith(("refs/heads/", "refs/tags/"))
        }
    output = _git(
        [
            "--git-dir",
            source,
            "for-each-ref",
            "--format=%(objectname) %(refname)",
            "refs/heads",
            "refs/tags",
        ]
    )
    return {
        fields[1]: fields[0]
        for line in output.splitlines()
        if len(fields := line.split()) == 2
    }


def _github_repository(destination):
    parsed = urlparse(destination)
    if parsed.scheme != "https" or parsed.netloc != "github.com":
        return None
    parts = parsed.path.removesuffix(".git").strip("/").split("/")
    if len(parts) != 2 or not all(REPO_PART.fullmatch(part) for part in parts):
        return None
    return "/".join(parts)


def _gh(args):
    try:
        result = subprocess.run(
            ["gh", "api", *args], capture_output=True, text=True, timeout=60
        )
    except subprocess.TimeoutExpired as exc:
        raise RuntimeError("gh api timed out while preparing mirror push") from exc
    if result.returncode:
        detail = result.stderr.strip() or result.stdout.strip()
        raise RuntimeError(f"gh api failed while preparing mirror push: {detail[:400]}")
    return result.stdout.strip()


def sync_mirror(source, destination):
    """Push all source refs and verify the destination has the identical set."""
    if not valid_mirror(source):
        raise ValueError(f"source is not a valid mirror: {source}")
    expected = _refs(source)
    if not expected:
        raise ValueError(f"source mirror has no refs: {source}")
    # GitHub reserves refs/pull/* and rejects pushes to them. The durable Git
    # history contract for this plain mirror is the complete branch/tag set.
    # Push replacement refs before changing GitHub metadata or pruning: GitHub
    # refuses deletion of a repository's current default branch.
    repository = _github_repository(destination)
    default_branch = None
    if repository:
        default_branch = _gh([f"repos/{repository}", "--jq", ".default_branch"])
    if default_branch and f"refs/heads/{default_branch}" not in expected:
        source_default = _git(["--git-dir", source, "symbolic-ref", "HEAD"])
        if not source_default.startswith("refs/heads/") or source_default not in expected:
            raise RuntimeError(
                "source default branch is unavailable; refusing to prune the "
                "destination default branch"
            )
        # Without --prune this first push transfers the replacement branch but
        # leaves the old default branch available until GitHub accepts the
        # metadata change.
        _git(
            [
                "--git-dir",
                source,
                "push",
                destination,
                "+refs/heads/*:refs/heads/*",
                "+refs/tags/*:refs/tags/*",
            ]
        )
        _gh(
            [
                f"repos/{repository}",
                "--method",
                "PATCH",
                "-f",
                f"default_branch={source_default.removeprefix('refs/heads/')}",
            ]
        )
    _git(
        [
            "--git-dir",
            source,
            "push",
            "--prune",
            destination,
            "+refs/heads/*:refs/heads/*",
            "+refs/tags/*:refs/tags/*",
        ]
    )
    actual = _refs(destination, remote=True)
    if actual != expected:
        missing = sorted(set(expected) - set(actual))
        extra = sorted(set(actual) - set(expected))
        changed = sorted(
            ref for ref in set(actual) & set(expected) if actual[ref] != expected[ref]
        )
        raise RuntimeError(
            "destination refs differ after mirror push "
            f"(missing={missing[:5]}, extra={extra[:5]}, changed={changed[:5]})"
        )
    return {"status": "verified", "refs": len(expected)}


def destination_repo(repo):
    parts = repo.split("/")
    if len(parts) != 2 or not all(REPO_PART.fullmatch(part) for part in parts):
        raise ValueError("repo must be OWNER/REPO using safe path components")
    if repo not in FLEET_REPOS:
        raise ValueError(f"{repo} is not one of the four fleet repositories")
    return f"https://github.com/developing-today/{parts[1]}-mirror.git"


def mirror_repo(repo, root):
    destination = destination_repo(repo)
    plan, problems = load_plan(root)
    if problems or not plan:
        raise ValueError("backup plan is invalid or empty: " + "; ".join(problems))
    if not any(item["repo"] == repo for item in plan):
        raise ValueError(f"{repo} is not in the backup plan")
    return sync_mirror(mirror_path(root, repo), destination)


def _parser():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--repo", required=True, help="planned repository as OWNER/REPO")
    parser.add_argument("--root", help="backup tree root (defaults to configured backup_root)")
    return parser


def main(argv=None):
    args = _parser().parse_args(argv)
    root = os.path.abspath(os.path.expanduser(args.root)) if args.root else backup_root()
    try:
        result = mirror_repo(args.repo, root)
    except (OSError, RuntimeError, ValueError) as exc:
        print(f"github-backup: {args.repo}: {exc}", file=sys.stderr)
        return 1
    print(f"github-backup: {args.repo}: verified plain mirror ({result['refs']} refs)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
