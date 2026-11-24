#!/usr/bin/env python3
"""Contract tests for check-nix-dep-gate.sh (#14195).

The gate enforces the companion-change rule: a diff touching a
derivation's dependency inputs (lockfiles, manifests) must also touch
the file pinning the Nix hash, or the gate fails naming the derivation.

Tests build throwaway git repos (branch `base`, change on `head`) so
they never depend on the real tree's git state.
"""

import os
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "check-nix-dep-gate.sh"


def run_gate(args, env=None):
    return subprocess.run(
        ["bash", str(SCRIPT), *args],
        text=True,
        capture_output=True,
        check=False,
        env=env,
    )


def make_repo(files):
    """Create a git repo with `files` committed on main, return (tmp, repo).

    Caller must commit changes on top to form the diff head.
    """
    tmp = tempfile.TemporaryDirectory(prefix="nix-dep-gate-")
    repo = Path(tmp.name)
    subprocess.run(["git", "init", "-q", "-b", "main", str(repo)], check=True)
    subprocess.run(
        ["git", "-C", str(repo), "config", "user.email", "gate@test"],
        check=True,
    )
    subprocess.run(
        ["git", "-C", str(repo), "config", "user.name", "gate"], check=True
    )
    for rel, content in files.items():
        p = repo / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(content)
        subprocess.run(["git", "-C", str(repo), "add", rel], check=True)
    subprocess.run(
        ["git", "-C", str(repo), "commit", "-qm", "base"], check=True
    )
    # Mark the base commit so later commits on main form a diff against it.
    subprocess.run(["git", "-C", str(repo), "branch", "base"], check=True)
    return tmp, repo


def commit_all(repo, message):
    subprocess.run(["git", "-C", str(repo), "add", "-A"], check=True)
    subprocess.run(["git", "-C", str(repo), "commit", "-qm", message], check=True)


BASE_FILES = {
    "pkgs/mcpx/go.mod": "module mcpx\n",
    "pkgs/mcpx/go.sum": "sums\n",
    "pkgs/mcpx/package.nix": '{ vendorHash = "sha256-old"; }\n',
    "pkgs/grok-cli/package.json": '{"name":"grok"}\n',
    "pkgs/grok-cli/package-lock.json": '{"lockfileVersion":3}\n',
    "pkgs/grok-cli/default.nix": '{ npmDepsHash = "sha256-old"; }\n',
    "pkgs/id/Cargo.toml": '[package]\n',
    "pkgs/id/Cargo.lock": "lock\n",
    "pkgs/id/default.nix": '{ hash = "sha256-old"; }\n',
    "pkgs/id/flake.nix": '{ inputs = {}; }\n',
    "pkgs/id/flake.lock": '{ nodes = {}; }\n',
    "pkgs/cold-store-reaper/worker/package.json": '{"name":"w"}\n',
    "pkgs/cold-store-reaper/worker/package-lock.json": '{"lockfileVersion":3}\n',
    "pkgs/cold-store-reaper/worker/package.nix": '{ npmDepsHash = "sha256-old"; }\n',
}


class TestCheckNixDepGate(unittest.TestCase):
    def test_clean_tree_passes(self):
        tmp, repo = make_repo(BASE_FILES)
        try:
            (repo / "README.md").write_text("docs\n")
            commit_all(repo, "docs only")
            result = run_gate(["--repo", str(repo), "--base", "base"])
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("Nix dependency gate OK", result.stdout)
        finally:
            tmp.cleanup()

    def test_stale_go_bump_fails(self):
        """go.mod without package.nix: the mcpx vendorHash instance (#14051)."""
        tmp, repo = make_repo(BASE_FILES)
        try:
            (repo / "pkgs/mcpx/go.mod").write_text("module mcpx\n// bump\n")
            commit_all(repo, "bump go dep")
            result = run_gate(["--repo", str(repo), "--base", "base"])
            self.assertEqual(result.returncode, 1, result.stdout)
            self.assertIn("pkgs/mcpx", result.stderr)
            self.assertIn("vendorHash", result.stderr)
            self.assertIn("14195", result.stderr)
        finally:
            tmp.cleanup()

    def test_refreshed_go_bump_passes(self):
        tmp, repo = make_repo(BASE_FILES)
        try:
            (repo / "pkgs/mcpx/go.mod").write_text("module mcpx\n// bump\n")
            (repo / "pkgs/mcpx/package.nix").write_text(
                '{ vendorHash = "sha256-new"; }\n'
            )
            commit_all(repo, "bump go dep + refresh hash")
            result = run_gate(["--repo", str(repo), "--base", "base"])
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("pkgs/mcpx/package.nix", result.stdout)
        finally:
            tmp.cleanup()

    def test_stale_npm_bump_fails(self):
        """package-lock.json without default.nix: the grok-cli instance."""
        tmp, repo = make_repo(BASE_FILES)
        try:
            (repo / "pkgs/grok-cli/package-lock.json").write_text(
                '{"lockfileVersion":3,"bump":true}\n'
            )
            commit_all(repo, "bump npm dep")
            result = run_gate(["--repo", str(repo), "--base", "base"])
            self.assertEqual(result.returncode, 1, result.stdout)
            self.assertIn("pkgs/grok-cli", result.stderr)
            self.assertIn("npmDepsHash", result.stderr)
        finally:
            tmp.cleanup()

    def test_stale_cargo_bump_fails(self):
        tmp, repo = make_repo(BASE_FILES)
        try:
            (repo / "pkgs/id/Cargo.lock").write_text("lock bumped\n")
            commit_all(repo, "bump cargo dep")
            result = run_gate(["--repo", str(repo), "--base", "base"])
            self.assertEqual(result.returncode, 1, result.stdout)
            self.assertIn("pkgs/id", result.stderr)
        finally:
            tmp.cleanup()

    def test_standalone_flake_lock_gate(self):
        """flake.nix without flake.lock fails; the pair passes."""
        tmp, repo = make_repo(BASE_FILES)
        try:
            (repo / "pkgs/id/flake.nix").write_text('{ inputs = { x = {}; }; }\n')
            commit_all(repo, "bump flake input")
            result = run_gate(["--repo", str(repo), "--base", "base"])
            self.assertEqual(result.returncode, 1, result.stdout)
            self.assertIn("flake.lock", result.stderr)

            (repo / "pkgs/id/flake.lock").write_text('{ nodes = { x = {}; }; }\n')
            commit_all(repo, "re-lock")
            result = run_gate(["--repo", str(repo), "--base", "base"])
            self.assertEqual(result.returncode, 0, result.stderr)
        finally:
            tmp.cleanup()

    def test_nested_lockfile_covered(self):
        """Nested lockfiles (pkgs/cold-store-reaper/worker) are gated."""
        tmp, repo = make_repo(BASE_FILES)
        try:
            (repo / "pkgs/cold-store-reaper/worker/package-lock.json").write_text(
                '{"lockfileVersion":3,"bump":true}\n'
            )
            commit_all(repo, "bump nested npm dep")
            result = run_gate(["--repo", str(repo), "--base", "base"])
            self.assertEqual(result.returncode, 1, result.stdout)
            self.assertIn("pkgs/cold-store-reaper/worker", result.stderr)
        finally:
            tmp.cleanup()

    def test_unknown_option_fails_closed(self):
        result = run_gate(["--bogus"])
        self.assertEqual(result.returncode, 2)
        self.assertIn("Unknown option", result.stderr)

    def test_valueless_option_fails_closed(self):
        result = run_gate(["--base"])
        self.assertEqual(result.returncode, 2)

    def test_positional_refused(self):
        result = run_gate(["main"])
        self.assertEqual(result.returncode, 2)
        self.assertIn("Unexpected positional", result.stderr)

    def test_uncomputable_diff_fails_closed(self):
        tmp, repo = make_repo(BASE_FILES)
        try:
            result = run_gate(
                ["--repo", str(repo), "--base", "no-such-ref", "--head", "HEAD"]
            )
            self.assertEqual(result.returncode, 2)
            self.assertIn("failing closed", result.stderr)
        finally:
            tmp.cleanup()

    def test_list_names_coverage(self):
        result = run_gate(["--list"])
        self.assertEqual(result.returncode, 0, result.stderr)
        for name in (
            "pkgs/mcpx",
            "pkgs/grok-cli",
            "pkgs/id",
            "pkgs/supergateway",
            "pkgs/cold-store-reaper/worker",
            "flake.lock",
        ):
            self.assertIn(name, result.stdout)


if __name__ == "__main__":
    unittest.main()
