#!/usr/bin/env python3
"""Publish one planned mirror to the Cloudflare rehost bucket (#8584).

Command line: --repo OWNER/REPO [--bucket NAME] [--root PATH]. The
rclone remote `r2bak` must already be configured through its
environment variables; this command never reads or prints credentials.

What the rehost Worker serves is pre-generated here, per repo:

    <owner>/<repo>/packs/<pack-hash>.pack   one self-contained pack
                                            holding every object in
                                            the mirror (pack-objects
                                            --all --delta-base-offset)
    <owner>/<repo>/refs.json                head symref, head oid, the
                                            refs/heads + refs/tags set
                                            with peeled tag targets,
                                            and the pack's key

Publish order is the consistency rule: the pack is uploaded and
read-back-verified first, refs.json (the only object the Worker
trusts) is uploaded and read-back-verified last, so a reader never
sees refs pointing at a pack that is not there. After the new
refs.json verifies, packs from older generations are pruned except
the new one and the one the previous refs.json named (a clone that
started under the previous generation keeps its pack).
"""

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
from datetime import datetime, timezone

import bundle_cold
from ghb_common import backup_root, load_plan, mirror_path, valid_mirror

BUCKET_PART = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_-]*$")
PACK_NAME = re.compile(r"^[0-9a-f]{40,64}\.pack$")
DEFAULT_BUCKET = "fleet-git-rehost"
# Pack generation and transfers of the large fleet repos take far
# longer than the quick helper calls ghb_common.run bounds.
LONG_TIMEOUT_SECONDS = 3600


def _run(argv, timeout=LONG_TIMEOUT_SECONDS):
    try:
        # stdin is DEVNULL, never inherited: git pack-objects reads
        # object names from stdin when its revision arguments run out,
        # so an inherited stdin that never reaches EOF hangs the
        # publish forever (found live 2026-11-24: two proof runs
        # blocked at 0 CPU until killed).
        result = subprocess.run(
            argv,
            capture_output=True,
            text=True,
            timeout=timeout,
            stdin=subprocess.DEVNULL,
        )
    except subprocess.TimeoutExpired as exc:
        raise RuntimeError(f"{argv[0]} timed out") from exc
    if result.returncode:
        detail = result.stderr.strip() or result.stdout.strip()
        raise RuntimeError(f"{' '.join(argv[:3])} failed: {detail[:400]}")
    return result.stdout.strip()


def _git(mirror, *args, timeout=LONG_TIMEOUT_SECONDS):
    return _run(["git", "--git-dir", mirror, *args], timeout=timeout)


def _sha256_file(path):
    digest = hashlib.sha256()
    with open(path, "rb") as fh:
        for chunk in iter(lambda: fh.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def collect_refs(mirror):
    """(head_ref, refs) for the advertisement: heads + tags, peeled."""
    head_ref = _git(mirror, "symbolic-ref", "HEAD")
    listing = _git(
        mirror,
        "for-each-ref",
        "--format=%(refname) %(objectname) %(*objectname)",
        "refs/heads",
        "refs/tags",
    )
    refs = []
    for line in listing.splitlines():
        fields = line.split(" ")
        if len(fields) < 2:
            raise RuntimeError(f"unexpected for-each-ref line: {line!r}")
        refs.append(
            {
                "name": fields[0],
                "oid": fields[1],
                "peeled": fields[2] if len(fields) > 2 and fields[2] else None,
            }
        )
    if not refs:
        raise RuntimeError("mirror has no heads or tags to publish")
    head = next((r["oid"] for r in refs if r["name"] == head_ref), None)
    if head is None:
        raise RuntimeError(f"HEAD target {head_ref} is not among the published refs")
    return head_ref, head, refs


def build_pack(mirror, staging):
    """One self-contained pack of every object in the mirror."""
    base = os.path.join(staging, "pack")
    pack_hash = _git(mirror, "pack-objects", "--all", "--delta-base-offset", base)
    pack_path = os.path.join(staging, f"pack-{pack_hash}.pack")
    if not os.path.isfile(pack_path):
        raise RuntimeError(f"pack-objects did not produce {pack_path}")
    return pack_path, pack_hash


def _remote(bucket, key):
    return f"r2bak:{bucket}/{key}"


def _download(bucket, key, dest):
    _run(["rclone", "copyto", _remote(bucket, key), dest])


def _read_previous_doc(bucket, prefix, staging):
    """The refs.json currently published, or None when unpublished."""
    dest = os.path.join(staging, "previous-refs.json")
    try:
        _download(bucket, f"{prefix}/refs.json", dest)
    except RuntimeError:
        return None
    try:
        with open(dest, encoding="utf-8") as fh:
            doc = json.load(fh)
    except (OSError, ValueError):
        return None
    return doc if isinstance(doc, dict) else None


def _prune_packs(bucket, prefix, keep):
    listing = _run(["rclone", "lsf", _remote(bucket, f"{prefix}/packs/")])
    for name in listing.splitlines():
        name = name.strip()
        if name in keep or not PACK_NAME.fullmatch(name):
            continue
        _run(["rclone", "deletefile", _remote(bucket, f"{prefix}/packs/{name}")])


def publish_repo(repo, root, bucket):
    """Publish one mirror; returns the verified refs document."""
    parts = repo.split("/")
    if len(parts) != 2 or not all(
        bundle_cold.REPO_PART.fullmatch(part) for part in parts
    ):
        raise ValueError("repo must be OWNER/REPO using safe path components")
    if not BUCKET_PART.fullmatch(bucket):
        raise ValueError("bucket must be a safe R2 bucket name")
    plan, _problems = load_plan(root)
    if not any(item["repo"] == repo for item in plan):
        raise ValueError(f"{repo} is not in the backup plan")
    mirror = mirror_path(root, repo)
    if not valid_mirror(mirror):
        raise ValueError(f"{mirror} is not a valid mirror")

    prefix = f"{parts[0]}/{parts[1]}"
    staging_root = os.path.join(root, "rehost-staging", parts[0])
    os.makedirs(staging_root, exist_ok=True)
    staging = tempfile.mkdtemp(prefix=parts[1] + ".", dir=staging_root)
    try:
        previous = _read_previous_doc(bucket, prefix, staging)
        head_ref, head, refs = collect_refs(mirror)
        pack_path, pack_hash = build_pack(mirror, staging)
        pack_key = f"{prefix}/packs/{pack_hash}.pack"
        pack_sha256 = _sha256_file(pack_path)

        # Pack first, read back and compared byte for byte.
        _run(["rclone", "copyto", pack_path, _remote(bucket, pack_key)])
        readback = os.path.join(staging, "pack-readback.pack")
        _download(bucket, pack_key, readback)
        if _sha256_file(readback) != pack_sha256:
            raise RuntimeError(f"downloaded R2 pack SHA-256 does not match: {pack_key}")
        os.unlink(readback)

        doc = {
            "version": 1,
            "repo": repo,
            "head_ref": head_ref,
            "head": head,
            "pack": f"packs/{pack_hash}.pack",
            "pack_bytes": os.path.getsize(pack_path),
            "pack_sha256": pack_sha256,
            "generated_at": datetime.now(timezone.utc)
            .isoformat()
            .replace("+00:00", "Z"),
            "refs": refs,
        }
        doc_path = os.path.join(staging, "refs.json")
        with open(doc_path, "w", encoding="utf-8") as fh:
            json.dump(doc, fh, indent=2, sort_keys=True)
            fh.write("\n")
        # refs.json last: it is the commit point of the publish.
        _run(["rclone", "copyto", doc_path, _remote(bucket, f"{prefix}/refs.json")])
        doc_readback = os.path.join(staging, "refs-readback.json")
        _download(bucket, f"{prefix}/refs.json", doc_readback)
        with open(doc_path, "rb") as fh:
            local_bytes = fh.read()
        with open(doc_readback, "rb") as fh:
            if fh.read() != local_bytes:
                raise RuntimeError("downloaded R2 refs.json does not match the upload")

        keep = {f"{pack_hash}.pack"}
        if previous and isinstance(previous.get("pack"), str):
            keep.add(os.path.basename(previous["pack"]))
        _prune_packs(bucket, prefix, keep)
        return doc
    finally:
        for name in os.listdir(staging):
            try:
                os.unlink(os.path.join(staging, name))
            except FileNotFoundError:
                pass
        try:
            os.rmdir(staging)
        except OSError:
            pass


def _parser():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument(
        "--repo", required=True, help="planned repository as OWNER/REPO"
    )
    parser.add_argument("--bucket", default=DEFAULT_BUCKET, help="existing R2 bucket")
    parser.add_argument(
        "--root", help="backup tree root (defaults to configured backup_root)"
    )
    return parser


def main(argv=None):
    args = _parser().parse_args(argv)
    root = (
        os.path.abspath(os.path.expanduser(args.root)) if args.root else backup_root()
    )
    try:
        doc = publish_repo(args.repo, root, args.bucket)
    except (OSError, RuntimeError, ValueError) as exc:
        print(f"github-backup: {args.repo}: {exc}", file=sys.stderr)
        return 1
    print(
        f"github-backup: {args.repo}: verified rehost publish "
        f"({doc['head']}, pack {doc['pack']}, {len(doc['refs'])} refs)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
