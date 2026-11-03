"""Exercise the scheduled auto-format workflow using a real git history."""

import os
from pathlib import Path
import subprocess
import tempfile
import unittest


WORKFLOW = Path(__file__).resolve().parents[2] / ".github/workflows/auto-format.yml"


class AutoFormatWorkflowTest(unittest.TestCase):
    def test_deleted_final_path_does_not_abort_scheduled_filter(self):
        # Execute the workflow's real run block, not a copy of its filter.
        workflow = WORKFLOW.read_text()
        block = workflow.split(
            "      - name: Format changed files with treefmt and commit any fixes\n", 1
        )[1]
        script = block.split("        run: |\n", 1)[1]
        script = "\n".join(
            line[10:] for line in script.splitlines() if line.startswith("          ")
        )

        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            repo = root / "repo"
            repo.mkdir()
            subprocess.run(["git", "init", "-q", str(repo)], check=True)

            def git(*args):
                subprocess.run(
                    ["git", *args], cwd=repo, check=True, capture_output=True
                )

            git("config", "user.name", "Test")
            git("config", "user.email", "test@example.com")
            (repo / "a-existing").write_text("keep\n")
            (repo / "z-deleted").write_text("delete\n")
            git("add", ".")
            git("commit", "-qm", "initial")
            (repo / "z-deleted").unlink()
            git("add", "-u")
            git("commit", "-qm", "delete final path")

            bindir = root / "bin"
            bindir.mkdir()
            nix = bindir / "nix"
            nix.write_text('#!/bin/sh\nprintf "%s\\n" "$@" > "$NIX_ARGS"\n')
            nix.chmod(0o755)
            args = root / "nix-args"
            env = {
                **os.environ,
                "PATH": f"{bindir}:{os.environ['PATH']}",
                "GITHUB_EVENT_NAME": "schedule",
                "NIX_ARGS": str(args),
            }
            result = subprocess.run(
                ["bash", "-c", script],
                cwd=repo,
                env=env,
                text=True,
                capture_output=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                args.read_text().splitlines(), ["run", ".#format", "--", "a-existing"]
            )


    def _run_scheduled_pass(self, files):
        # Run the workflow's real run block in schedule mode against a
        # repo whose history contains exactly the given files.
        workflow = WORKFLOW.read_text()
        block = workflow.split("      - name: Format changed files with treefmt and commit any fixes\n", 1)[1]
        script = block.split("        run: |\n", 1)[1]
        script = "\n".join(line[10:] for line in script.splitlines() if line.startswith("          "))

        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            repo = root / "repo"
            repo.mkdir()
            subprocess.run(["git", "init", "-q", str(repo)], check=True)
            def git(*args):
                subprocess.run(["git", *args], cwd=repo, check=True, capture_output=True)

            git("config", "user.name", "Test")
            git("config", "user.email", "test@example.com")
            for name, content in files.items():
                path = repo / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(content)
            git("add", ".")
            git("commit", "-qm", "initial")

            bindir = root / "bin"
            bindir.mkdir()
            nix = bindir / "nix"
            nix.write_text('#!/bin/sh\nprintf "%s\\n" "$@" > "$NIX_ARGS"\n')
            nix.chmod(0o755)
            args = root / "nix-args"
            env = {**os.environ, "PATH": f"{bindir}:{os.environ['PATH']}",
                   "GITHUB_EVENT_NAME": "schedule", "NIX_ARGS": str(args)}
            result = subprocess.run(["bash", "-c", script], cwd=repo, env=env, text=True, capture_output=True)
            nix_args = args.read_text().splitlines() if args.exists() else None
            return result, nix_args

    def test_workflow_files_do_not_block_unrelated_formatting(self):
        # Mixed change: .yml and .yaml workflow files are skipped and
        # logged; the unrelated file is still formatted (#3118).
        result, nix_args = self._run_scheduled_pass({
            "notes.txt": "keep\n",
            ".github/workflows/ci.yml": "name: ci\n",
            ".github/workflows/check.yaml": "name: check\n",
        })
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(nix_args, ["run", ".#format", "--", "notes.txt"])
        self.assertIn("Skipping workflow file(s)", result.stdout)
        self.assertIn(".github/workflows/ci.yml", result.stdout)
        self.assertIn(".github/workflows/check.yaml", result.stdout)

    def test_workflow_only_change_exits_cleanly_without_formatting(self):
        # Workflow-only change: nothing the bot may push, clean exit,
        # and the formatter is never invoked (#3118).
        result, nix_args = self._run_scheduled_pass({
            ".github/workflows/ci.yml": "name: ci\n",
            ".github/workflows/check.yaml": "name: check\n",
        })
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIsNone(nix_args)
        self.assertIn("Only workflow files changed", result.stdout)


if __name__ == "__main__":
    unittest.main()
