#!/usr/bin/env python3
"""Contract tests for update-nixpkgs-inputs (both copies) — #2783, #603.

The repo carries two copies of this tool: the fleet copy at scripts/
and the pkgs/id copy at pkgs/id/scripts/, which the pkgs/id just
recipes run. #598 fixed a silent-failure bug (failed updates exited 0;
the wrapper re-ran the whole update after a genuine failure) in the
root copy only, and the fix silently never reached pkgs/id — the
drift this file exists to prevent:

- The two .py copies must stay byte-identical, and so must the two
  .sh wrappers. A fix that lands in one copy and not the other fails
  here instead of shipping silently.
- The #598 exit-code contract is pinned for BOTH .py copies: exit 1
  after the summaries when any input update failed, 0 when all
  succeed / are already up-to-date.
- The wrapper contract is pinned for BOTH .sh copies: fall back to
  python3 only when the script cannot be executed at all (126/127),
  propagate the exit status, and never re-run after a genuine failure
  (the update body executes exactly once).

Run: python3 -m unittest discover -s scripts/tests -p 'test_update_nixpkgs_inputs.py'
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
COPIES = {
    "root": ROOT / "scripts",
    "pkgs/id": ROOT / "pkgs" / "id" / "scripts",
}

FLAKE_LOCK = {
    "nodes": {
        "root": {"inputs": {"nixpkgs": "nixpkgs"}},
        "nixpkgs": {
            "original": {"type": "github", "owner": "NixOS", "repo": "nixpkgs"},
            "locked": {
                "type": "github",
                "owner": "NixOS",
                "repo": "nixpkgs",
                "rev": "0123456789abcdef0123456789abcdef01234567",
            },
        },
    },
    "root": "root",
    "version": 7,
}


def _write_executable(path: Path, content: str) -> None:
    path.write_text(content)
    path.chmod(0o755)


class TestCopiesCannotDrift(unittest.TestCase):
    def test_py_copies_byte_identical(self):
        root = (COPIES["root"] / "update-nixpkgs-inputs.py").read_bytes()
        pkgs_id = (COPIES["pkgs/id"] / "update-nixpkgs-inputs.py").read_bytes()
        self.assertEqual(
            root,
            pkgs_id,
            "scripts/ and pkgs/id/scripts/ update-nixpkgs-inputs.py diverged — "
            "port the change to both copies (see #2783)",
        )

    def test_sh_copies_byte_identical(self):
        root = (COPIES["root"] / "update-nixpkgs-inputs.sh").read_bytes()
        pkgs_id = (COPIES["pkgs/id"] / "update-nixpkgs-inputs.sh").read_bytes()
        self.assertEqual(
            root,
            pkgs_id,
            "scripts/ and pkgs/id/scripts/ update-nixpkgs-inputs.sh diverged — "
            "port the change to both copies (see #2783)",
        )


class ScriptTree:
    """One copy of the tool staged as <tmp>/scripts/ + <tmp>/flake.lock,

    with a stub `nix` (and a stub `uv`, so the .py shebang executes the
    body under python3) on PATH. The stub nix appends one line to a
    counter file per invocation and exits with the configured status.
    """

    def __init__(self, src_dir: Path, nix_status: int):
        self._tmp = tempfile.TemporaryDirectory()
        base = Path(self._tmp.name)
        self.repo_dir = base / "repo"
        scripts = self.repo_dir / "scripts"
        scripts.mkdir(parents=True)
        shutil.copy2(src_dir / "update-nixpkgs-inputs.py", scripts)
        shutil.copy2(src_dir / "update-nixpkgs-inputs.sh", scripts)
        (self.repo_dir / "flake.lock").write_text(json.dumps(FLAKE_LOCK))
        self.bin = base / "bin"
        self.bin.mkdir()
        self.counter = base / "nix-calls"
        _write_executable(
            self.bin / "nix",
            "#!/bin/sh\n"
            f'echo call >> "{self.counter}"\n'
            f"exit {nix_status}\n",
        )
        # The .py shebang is `env -S uv run --script`; this stub makes
        # the wrapper's direct-execution attempt run the body under
        # python3, so wrapper behaviour is testable without uv.
        _write_executable(
            self.bin / "uv",
            "#!/bin/sh\n"
            '[ "$1" = run ] && shift\n'
            '[ "$1" = --script ] && shift\n'
            'exec python3 "$@"\n',
        )
        self.env = dict(os.environ, PATH=f"{self.bin}:{os.environ['PATH']}")

    def cleanup(self):
        self._tmp.cleanup()

    def nix_calls(self) -> int:
        if not self.counter.exists():
            return 0
        return len(self.counter.read_text().splitlines())

    def run_py(self) -> subprocess.CompletedProcess:
        return subprocess.run(
            [sys.executable, str(self.repo_dir / "scripts" / "update-nixpkgs-inputs.py")],
            env=self.env,
            capture_output=True,
            text=True,
            timeout=60,
        )

    def run_sh(self) -> subprocess.CompletedProcess:
        return subprocess.run(
            ["bash", str(self.repo_dir / "scripts" / "update-nixpkgs-inputs.sh")],
            env=self.env,
            capture_output=True,
            text=True,
            timeout=60,
        )


class TestExitCodeContract(unittest.TestCase):
    def _check_copy(self, label: str, src_dir: Path):
        with self.subTest(copy=label, case="failed update exits 1"):
            tree = ScriptTree(src_dir, nix_status=1)
            try:
                r = tree.run_py()
                self.assertIn("Failed:  1", r.stdout)
                self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
                self.assertEqual(tree.nix_calls(), 1)
            finally:
                tree.cleanup()
        with self.subTest(copy=label, case="up-to-date exits 0"):
            tree = ScriptTree(src_dir, nix_status=0)
            try:
                r = tree.run_py()
                self.assertIn("Failed:  0", r.stdout)
                self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
            finally:
                tree.cleanup()

    def test_root_copy(self):
        self._check_copy("root", COPIES["root"])

    def test_pkgs_id_copy(self):
        self._check_copy("pkgs/id", COPIES["pkgs/id"])


class TestWrapperContract(unittest.TestCase):
    def _check_copy(self, label: str, src_dir: Path):
        with self.subTest(copy=label, case="genuine failure runs once, exits 1"):
            tree = ScriptTree(src_dir, nix_status=1)
            try:
                r = tree.run_sh()
                self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
                self.assertEqual(
                    tree.nix_calls(),
                    1,
                    "wrapper re-ran the update after a genuine failure",
                )
            finally:
                tree.cleanup()
        with self.subTest(copy=label, case="success exits 0"):
            tree = ScriptTree(src_dir, nix_status=0)
            try:
                r = tree.run_sh()
                self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
                self.assertEqual(tree.nix_calls(), 1)
            finally:
                tree.cleanup()
        with self.subTest(copy=label, case="cannot-execute falls back to python3"):
            tree = ScriptTree(src_dir, nix_status=1)
            try:
                # No exec bit: direct execution fails 126, the wrapper
                # must fall back to python3 exactly once and still
                # propagate the genuine failure status.
                (tree.repo_dir / "scripts" / "update-nixpkgs-inputs.py").chmod(0o644)
                r = tree.run_sh()
                self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
                self.assertEqual(tree.nix_calls(), 1)
            finally:
                tree.cleanup()

    def test_root_copy(self):
        self._check_copy("root", COPIES["root"])

    def test_pkgs_id_copy(self):
        self._check_copy("pkgs/id", COPIES["pkgs/id"])


if __name__ == "__main__":
    unittest.main()
