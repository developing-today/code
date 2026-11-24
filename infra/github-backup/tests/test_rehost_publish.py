#!/usr/bin/env python3
"""Hermetic tests for rehost_publish.py (#8584 Cloudflare rehost).

Oracle: after a publish against a fake rclone backed by a local
directory, the bucket holds a refs.json whose head equals the mirror
HEAD and whose refs carry the peeled tag target, plus the pack it
names; the pack, loaded into a scratch repo, contains the head commit
(git cat-file) — i.e. the two objects the Worker serves are mutually
consistent. A second publish after a new commit prunes the first
pack only once a third generation exists (current + previous kept).

Run: python3 infra/github-backup/tests/test_rehost_publish.py
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stderr
from io import StringIO
from unittest.mock import patch

BIN = os.path.abspath(
    os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "bin")
)
sys.path.insert(0, BIN)

import rehost_publish

REAL_RUN = rehost_publish._run


def git(*args, cwd=None):
    return subprocess.run(
        ["git", *args], cwd=cwd, capture_output=True, text=True, check=True
    )


def make_mirror(root, repo="owner/repo"):
    src = os.path.join(root, "src")
    os.makedirs(src)
    git("init", "-b", "main", cwd=src)
    git("config", "user.email", "t@example.invalid", cwd=src)
    git("config", "user.name", "t", cwd=src)
    with open(os.path.join(src, "f.txt"), "w") as f:
        f.write("hello\n")
    git("add", "f.txt", cwd=src)
    git("commit", "-m", "one", cwd=src)
    git("tag", "-a", "-m", "version one", "v1")
    git("tag", "light")
    mirror = os.path.join(root, repo + ".git")
    os.makedirs(os.path.dirname(mirror))
    git("clone", "--mirror", src, mirror)
    plan = os.path.join(root, "plan.json")
    with open(plan, "w") as f:
        json.dump([{"repo": repo}], f)
    return src, mirror


class FakeRclone:
    """The rclone surface rehost_publish uses, backed by a directory.

    copyto maps r2bak:<bucket>/<key> to <bucket_dir>/<key>; lsf lists
    one prefix; deletefile removes one object. A copyto of a missing
    object fails the way rclone fails (non-zero exit -> RuntimeError).
    """

    def __init__(self, bucket_dir):
        self.bucket_dir = bucket_dir

    def _path(self, remote):
        assert remote.startswith("r2bak:"), remote
        _bucket, key = remote[len("r2bak:") :].split("/", 1)
        return os.path.join(self.bucket_dir, key)

    def __call__(self, argv):
        assert argv[0] == "rclone", argv
        op = argv[1]
        if op == "copyto":
            src, dst = argv[2], argv[3]
            if src.startswith("r2bak:"):
                src = self._path(src)
            if dst.startswith("r2bak:"):
                dst = self._path(dst)
            if not os.path.isfile(src):
                raise RuntimeError(f"rclone copyto failed: {src} not found")
            os.makedirs(os.path.dirname(dst), exist_ok=True)
            shutil.copyfile(src, dst)
            return ""
        if op == "lsf":
            base = self._path(argv[2])
            if not os.path.isdir(base):
                return ""
            return "\n".join(sorted(os.listdir(base)))
        if op == "deletefile":
            os.unlink(self._path(argv[2]))
            return ""
        raise AssertionError(f"unexpected rclone op {op}")


class RehostPublishTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = self._tmp.name
        self.bucket_dir = os.path.join(self.root, "bucket")
        os.makedirs(self.bucket_dir)
        self.fake = FakeRclone(self.bucket_dir)
        patcher = patch.object(rehost_publish, "_run", self._dispatch)
        patcher.start()
        self.addCleanup(patcher.stop)

    def _dispatch(self, argv, timeout=None):
        if argv[0] == "rclone":
            return self.fake(argv)
        return REAL_RUN(argv, timeout=timeout)

    def test_parser_rejects_abbreviated_options(self):
        with redirect_stderr(StringIO()):
            with self.assertRaises(SystemExit) as raised:
                rehost_publish._parser().parse_args(
                    ["--rep", "owner/repo", "--buc", "x"]
                )
        self.assertEqual(raised.exception.code, 2)

    def test_publish_serves_consistent_refs_and_pack(self):
        _src, mirror = make_mirror(self.root)
        doc = rehost_publish.publish_repo("owner/repo", self.root, "test-bucket")

        head = git("--git-dir", mirror, "rev-parse", "HEAD").stdout.strip()
        self.assertEqual(doc["head"], head)
        self.assertEqual(doc["head_ref"], "refs/heads/main")
        refs = {r["name"]: r for r in doc["refs"]}
        self.assertEqual(refs["refs/heads/main"]["oid"], head)
        tag_commit = git(
            "--git-dir", mirror, "rev-parse", "v1^{commit}"
        ).stdout.strip()
        self.assertEqual(refs["refs/tags/v1"]["peeled"], tag_commit)
        self.assertIsNone(refs["refs/tags/light"]["peeled"])

        # The bucket holds exactly the doc the Worker will read, and
        # the pack it names.
        with open(
            os.path.join(self.bucket_dir, "owner/repo/refs.json")
        ) as f:
            stored = json.load(f)
        self.assertEqual(stored, doc)
        pack_path = os.path.join(self.bucket_dir, "owner/repo", doc["pack"])
        self.assertTrue(os.path.isfile(pack_path))

        # Oracle: the pack alone reconstructs the head commit.
        scratch = os.path.join(self.root, "scratch.git")
        git("init", "--bare", scratch)
        git("--git-dir", scratch, "index-pack", pack_path)
        kind = git(
            "--git-dir", scratch, "cat-file", "-t", head
        ).stdout.strip()
        self.assertEqual(kind, "commit")

    def test_unplanned_repo_and_skeleton_mirror_refused(self):
        _src, _mirror = make_mirror(self.root)
        with self.assertRaisesRegex(ValueError, "not in the backup plan"):
            rehost_publish.publish_repo("owner/other", self.root, "test-bucket")
        skeleton = os.path.join(self.root, "owner", "empty.git")
        git("init", "--bare", skeleton)
        with open(os.path.join(self.root, "plan.json"), "w") as f:
            json.dump([{"repo": "owner/repo"}, {"repo": "owner/empty"}], f)
        with self.assertRaisesRegex(ValueError, "not a valid mirror"):
            rehost_publish.publish_repo("owner/empty", self.root, "test-bucket")

    def test_prune_keeps_current_and_previous_pack(self):
        src, _mirror = make_mirror(self.root)
        first = rehost_publish.publish_repo("owner/repo", self.root, "test-bucket")
        # Advance the source and republish twice; each new commit
        # changes the pack hash.
        docs = [first]
        mirror = os.path.join(self.root, "owner/repo.git")
        for i in range(2):
            with open(os.path.join(src, "f.txt"), "a") as f:
                f.write(f"more {i}\n")
            git("add", "f.txt", cwd=src)
            git("commit", "-m", f"c{i}", cwd=src)
            git("--git-dir", mirror, "fetch", "origin")
            docs.append(
                rehost_publish.publish_repo("owner/repo", self.root, "test-bucket")
            )
        packs = sorted(
            os.listdir(os.path.join(self.bucket_dir, "owner/repo/packs"))
        )
        # Three generations published; only the newest two packs stay.
        expected = sorted(
            os.path.basename(d["pack"]) for d in docs[1:]
        )
        self.assertEqual(packs, expected)
        self.assertNotIn(os.path.basename(first["pack"]), packs)


if __name__ == "__main__":
    unittest.main()
