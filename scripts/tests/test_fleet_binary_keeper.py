#!/usr/bin/env python3
"""Contract tests for scripts/muse/fleet-binary-keeper.sh (#14999).

Oracle: the .sha256 asset published with the newest fleet-static-*
release is the ground truth for what the installed fleet binary must
hash to. The keeper must leave the target byte-identical on a match,
replace it with the verified asset on drift, and never touch it when
the download fails verification or no fleet-static release exists.

A fake `gh` on PATH serves canned releases and assets from a fixture
directory, applying the keeper's documented selection (newest
fleet-static-* tag by published_at; other release families ignored)
and recording every invocation, so the tests can also assert what the
keeper did *not* ask for. Argument-contract cases (#3352) assert usage
errors land before gh is ever invoked.
"""

import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
KEEPER = ROOT / "scripts" / "muse" / "fleet-binary-keeper.sh"
ASSET = "fleet-x86_64-linux-musl"

FAKE_GH = r'''
import json, os, shutil, sys
from pathlib import Path

home = Path(os.environ["FAKE_GH_HOME"])
with (home / "calls.log").open("a") as fh:
    fh.write(" ".join(sys.argv[1:]) + "\n")

args = sys.argv[1:]
if args[0] == "api":
    jq = args[args.index("--jq") + 1] if "--jq" in args else ""
    assert "fleet-static-" in jq and "published_at" in jq, \
        "keeper's release selection must filter fleet-static-* by published_at"
    releases = json.loads((home / "releases.json").read_text())
    fleet = [r for r in releases if r["tag_name"].startswith("fleet-static-")]
    fleet.sort(key=lambda r: r["published_at"])
    print(fleet[-1]["tag_name"] if fleet else "")
    sys.exit(0)
if args[0] == "release" and args[1] == "download":
    tag = args[2]
    pattern = args[args.index("-p") + 1]
    out = args[args.index("-O") + 1]
    src = home / tag / pattern
    if not src.exists():
        print(f"fake gh: no asset {pattern} for {tag}", file=sys.stderr)
        sys.exit(1)
    if out == "-":
        sys.stdout.buffer.write(src.read_bytes())
    else:
        shutil.copyfile(src, out)
    sys.exit(0)
print(f"fake gh: unhandled argv {args}", file=sys.stderr)
sys.exit(1)
'''


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


class KeeperTest(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="fleet-binary-keeper-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.gh_home = self.tmp / "gh"
        self.gh_home.mkdir()
        self.bin_dir = self.tmp / "bin"
        self.bin_dir.mkdir()
        fake = self.bin_dir / "gh"
        fake.write_text("#!/usr/bin/env python3\n" + FAKE_GH)
        fake.chmod(0o755)
        self.target = self.tmp / "tools" / "fleet"
        self.target.parent.mkdir(parents=True)
        self.log_file = self.tmp / "state" / "keeper.log"

    # -- fixture helpers -------------------------------------------------
    def serve_release(self, tag, published_at, asset_bytes, *, sum_bytes=None):
        """Register one release; its assets land under gh_home/<tag>/."""
        rel_dir = self.gh_home / tag
        rel_dir.mkdir(parents=True, exist_ok=True)
        (rel_dir / ASSET).write_bytes(asset_bytes)
        digest = sha256(asset_bytes) if sum_bytes is None else sum_bytes
        (rel_dir / f"{ASSET}.sha256").write_text(f"{digest}  {ASSET}\n")
        releases = []
        index = self.gh_home / "releases.json"
        if index.exists():
            releases = json.loads(index.read_text())
        releases.append({"tag_name": tag, "published_at": published_at})
        index.write_text(json.dumps(releases))

    def serve_decoys(self):
        """A newer non-fleet release and an older fleet-static release,
        both of which the keeper must ignore in favour of the newest
        fleet-static one served by each test."""
        self.serve_release("pm-console-android-v999", "2026-11-25T09:00:00Z", b"android")
        self.serve_release("fleet-static-aaaaaaa", "2026-11-20T05:41:00Z", b"stale fleet")

    def run_keeper(self, *extra, target_bytes=None):
        if target_bytes is not None:
            self.target.write_bytes(target_bytes)
        env = dict(os.environ)
        env["PATH"] = f"{self.bin_dir}:/usr/bin:/bin"
        env["FAKE_GH_HOME"] = str(self.gh_home)
        return subprocess.run(
            ["bash", str(KEEPER), "--target", str(self.target),
             "--log-file", str(self.log_file), *extra],
            capture_output=True, text=True, env=env, timeout=60,
        )

    def calls(self):
        log = self.gh_home / "calls.log"
        return log.read_text() if log.exists() else ""

    def keeper_log(self):
        return self.log_file.read_text() if self.log_file.exists() else ""

    # -- cases ------------------------------------------------------------
    def test_match_leaves_target_and_skips_binary_download(self):
        self.serve_decoys()
        self.serve_release("fleet-static-bbbbbbb", "2026-11-25T05:41:00Z", b"current fleet")
        result = self.run_keeper(target_bytes=b"current fleet")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.target.read_bytes(), b"current fleet")
        self.assertIn("match tag=fleet-static-bbbbbbb", self.keeper_log())
        # Only the .sha256 was fetched; the binary asset was not.
        self.assertIn(f"-p {ASSET}.sha256", self.calls())
        self.assertNotIn(f"-p {ASSET} ", self.calls() + " ")
        self.assertNotIn(f"-p {ASSET}\n", self.calls())

    def test_drift_restores_verified_asset(self):
        self.serve_decoys()
        self.serve_release("fleet-static-bbbbbbb", "2026-11-25T05:41:00Z", b"current fleet")
        result = self.run_keeper(target_bytes=b"stale fleet")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.target.read_bytes(), b"current fleet")
        self.assertTrue(os.access(self.target, os.X_OK))
        self.assertIn("restored tag=fleet-static-bbbbbbb", self.keeper_log())

    def test_missing_target_is_installed(self):
        self.serve_release("fleet-static-bbbbbbb", "2026-11-25T05:41:00Z", b"current fleet")
        result = self.run_keeper()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.target.read_bytes(), b"current fleet")
        self.assertIn("restored tag=fleet-static-bbbbbbb", self.keeper_log())

    def test_corrupt_download_never_touches_target(self):
        self.serve_release(
            "fleet-static-bbbbbbb", "2026-11-25T05:41:00Z", b"tampered bytes",
            sum_bytes=sha256(b"honest bytes"),
        )
        result = self.run_keeper(target_bytes=b"installed fleet")
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertEqual(self.target.read_bytes(), b"installed fleet")
        self.assertIn("failed sha256 verification", self.keeper_log())

    def test_check_only_reports_drift_without_restoring(self):
        self.serve_release("fleet-static-bbbbbbb", "2026-11-25T05:41:00Z", b"current fleet")
        result = self.run_keeper("--check-only", target_bytes=b"stale fleet")
        self.assertEqual(result.returncode, 3, result.stderr)
        self.assertEqual(self.target.read_bytes(), b"stale fleet")
        self.assertIn("drift tag=fleet-static-bbbbbbb", self.keeper_log())

    def test_no_fleet_static_release_is_an_error(self):
        self.serve_release("pm-console-android-v999", "2026-11-25T09:00:00Z", b"android")
        result = self.run_keeper(target_bytes=b"installed fleet")
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertEqual(self.target.read_bytes(), b"installed fleet")
        self.assertIn("no fleet-static-* release found", self.keeper_log())

    def test_unknown_flag_is_a_usage_error_before_any_gh_call(self):
        result = self.run_keeper("--bogus")
        self.assertEqual(result.returncode, 2)
        self.assertIn("unknown flag --bogus", result.stderr)
        self.assertEqual(self.calls(), "")

    def test_duplicate_option_is_a_usage_error(self):
        result = self.run_keeper("--repo", "a/b", "--repo=c/d")
        self.assertEqual(result.returncode, 2)
        self.assertIn("duplicate --repo", result.stderr)
        self.assertEqual(self.calls(), "")

    def test_valueless_option_is_a_usage_error(self):
        result = self.run_keeper("--target=")
        self.assertEqual(result.returncode, 2)
        self.assertIn("--target requires a non-empty value", result.stderr)
        self.assertEqual(self.calls(), "")


if __name__ == "__main__":
    unittest.main()
