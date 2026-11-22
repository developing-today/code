#!/usr/bin/env python3
"""Create, locally verify, upload, and read-back-check one cold bundle.

Command line: --repo OWNER/REPO --bucket NAME [--root PATH]. The rclone
remote `r2bak` must already be configured through its environment variables;
this command never reads or prints credentials. The bundle is uploaded only
after bundle_cold's empty-repository verify and clone-HEAD comparison pass.
"""

import argparse
import hashlib
import os
import re
import subprocess
import sys
import tempfile

import bundle_cold
from ghb_common import RUN_TIMEOUT_SECONDS, backup_root, load_plan, mirror_path


BUCKET_PART = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_-]*$")


def _run_rclone(args):
    try:
        result = subprocess.run(
            ["rclone", *args], capture_output=True, text=True, timeout=RUN_TIMEOUT_SECONDS
        )
    except subprocess.TimeoutExpired as exc:
        raise RuntimeError(f"rclone {' '.join(args)} timed out") from exc
    if result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip()
        raise RuntimeError(f"rclone {' '.join(args)} failed: {detail[:400]}")


def upload_repo(repo, root, bucket):
    """Publish a locally verified bundle and compare the remote bytes."""
    parts = repo.split("/")
    if len(parts) != 2 or not all(bundle_cold.REPO_PART.fullmatch(part) for part in parts):
        raise ValueError("repo must be OWNER/REPO using safe path components")
    if not BUCKET_PART.fullmatch(bucket):
        raise ValueError("bucket must be a safe R2 bucket name")

    plan, _problems = load_plan(root)
    if not any(item["repo"] == repo for item in plan):
        raise ValueError(f"{repo} is not in the backup plan")

    mirror = mirror_path(root, repo)
    bundle_dir = os.path.join(root, "cold-bundles", *parts[:-1])
    os.makedirs(bundle_dir, exist_ok=True)
    fd, bundle = tempfile.mkstemp(prefix=parts[-1] + ".", suffix=".bundle", dir=bundle_dir)
    os.close(fd)
    os.unlink(bundle)
    readback = None
    try:
        result = bundle_cold.create_bundle(mirror, bundle)
        digest = hashlib.sha256()
        with open(bundle, "rb") as artifact:
            for chunk in iter(lambda: artifact.read(1024 * 1024), b""):
                digest.update(chunk)
        bundle_sha256 = digest.hexdigest()
        # Content-addressing avoids replacing an earlier verified backup when
        # the same repository advances; a repeated identical artifact is safe.
        remote = (
            f"r2bak:{bucket}/github-backup/cold-bundles/{parts[0]}/{parts[1]}/"
            f"{result['head']}-{bundle_sha256}.bundle"
        )
        _run_rclone(["copyto", bundle, remote])
        fd, readback = tempfile.mkstemp(prefix=".r2-readback-", dir=bundle_dir)
        os.close(fd)
        _run_rclone(["copyto", remote, readback])
        downloaded = hashlib.sha256()
        with open(readback, "rb") as artifact:
            for chunk in iter(lambda: artifact.read(1024 * 1024), b""):
                downloaded.update(chunk)
        if downloaded.hexdigest() != bundle_sha256:
            raise RuntimeError(f"downloaded R2 bundle SHA-256 does not match: {remote}")
        return {
            "status": result["status"],
            "head": result["head"],
            "remote": remote,
            "sha256": bundle_sha256,
        }
    finally:
        # Remote verified objects are the retained backups; mirrors remain the
        # source for rebuilding a local staged bundle on the next run.
        for artifact in (readback, bundle):
            if artifact is not None:
                try:
                    os.unlink(artifact)
                except FileNotFoundError:
                    pass


def _parser():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--repo", required=True, help="planned repository as OWNER/REPO")
    parser.add_argument("--bucket", required=True, help="existing R2 bucket")
    parser.add_argument("--root", help="backup tree root (defaults to configured backup_root)")
    return parser


def main(argv=None):
    args = _parser().parse_args(argv)
    root = os.path.abspath(os.path.expanduser(args.root)) if args.root else backup_root()
    try:
        result = upload_repo(args.repo, root, args.bucket)
    except (OSError, RuntimeError, ValueError) as exc:
        print(f"github-backup: {args.repo}: {exc}", file=sys.stderr)
        return 1
    print(
        f"github-backup: {args.repo}: verified R2 bundle at {result['remote']} "
        f"({result['head']})"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
