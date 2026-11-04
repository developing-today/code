"""Exercise the scheduled auto-format workflow using a real git history."""

import os
from pathlib import Path
import subprocess
import tempfile
import unittest


WORKFLOW = Path(__file__).resolve().parents[2] / ".github/workflows/auto-format.yml"


class AutoFormatWorkflowTest(unittest.TestCase):
    def _run_scheduled_pass(self, files, last_success_sha=None):
        # Run the workflow's real run block against a repo whose history
        # contains exactly the given files. `gh` and `nix` are stubbed so
        # record_success is observable and no network or Nix is touched.
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
            for name, content in files.items():
                path = repo / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(content)
            git("add", ".")
            git("commit", "-qm", "initial")
            head = subprocess.run(
                ["git", "rev-parse", "HEAD"],
                cwd=repo,
                check=True,
                capture_output=True,
                text=True,
            ).stdout.strip()

            bindir = root / "bin"
            bindir.mkdir()
            nix = bindir / "nix"
            nix.write_text('#!/bin/sh\nprintf "%s\\n" "$@" > "$NIX_ARGS"\n')
            nix.chmod(0o755)
            args = root / "nix-args"
            recorded = root / "recorded-sha"
            # The workflow updates the variable with the HEAD value; the
            # stub records the value it was asked to store.
            gh = bindir / "gh"
            gh.write_text(
                "#!/bin/sh\n"
                "while [ $# -gt 1 ]; do\n"
                '  if [ "$1" = "-f" ] && [ "$2" = "value=$GH_RECORD_VALUE" ]; then\n'
                '    printf "%s\\n" "$GH_RECORD_VALUE" > "$GH_RECORD_FILE"\n'
                "    exit 0\n"
                "  fi\n"
                "  shift\n"
                "done\n"
                "exit 0\n"
            )
            gh.chmod(0o755)
            env = {
                **os.environ,
                "PATH": f"{bindir}:{os.environ['PATH']}",
                "GITHUB_EVENT_NAME": "schedule",
                "GITHUB_REPOSITORY": "developing-today/code",
                "NIX_ARGS": str(args),
                "GH_RECORD_FILE": str(recorded),
                "GH_RECORD_VALUE": head,
            }
            if last_success_sha is not None:
                env["LAST_SUCCESS_SHA"] = last_success_sha
            result = subprocess.run(
                ["bash", "-c", script],
                cwd=repo,
                env=env,
                text=True,
                capture_output=True,
            )
            nix_args = args.read_text().splitlines() if args.exists() else None
            recorded_sha = recorded.read_text().strip() if recorded.exists() else None
            return result, nix_args, recorded_sha, head

    def test_deleted_final_path_does_not_abort_scheduled_filter(self):
        result, nix_args, _, _ = self._run_scheduled_pass(
            {"a-existing": "keep\n", "z-deleted": "delete\n"}
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("a-existing", result.stdout)

    def test_workflow_files_do_not_block_unrelated_formatting(self):
        # Mixed change: .yml and .yaml workflow files are skipped and
        # logged; the unrelated file is still formatted (#3118).
        result, nix_args, _, _ = self._run_scheduled_pass(
            {
                "notes.txt": "keep\n",
                ".github/workflows/ci.yml": "name: ci\n",
                ".github/workflows/check.yaml": "name: check\n",
            }
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(nix_args, ["run", ".#format", "--", "notes.txt"])
        self.assertIn("Skipping workflow file(s)", result.stdout)
        self.assertIn(".github/workflows/ci.yml", result.stdout)
        self.assertIn(".github/workflows/check.yaml", result.stdout)

    def test_workflow_only_change_records_success_without_formatting(self):
        # Workflow-only change: nothing the bot may push, clean exit,
        # and the formatter is never invoked (#3118). The pass must
        # record success so the next run can take the fast path.
        result, nix_args, recorded_sha, head = self._run_scheduled_pass(
            {
                ".github/workflows/ci.yml": "name: ci\n",
                ".github/workflows/check.yaml": "name: check\n",
            }
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIsNone(nix_args)
        self.assertIn("Only workflow files changed", result.stdout)
        self.assertEqual(recorded_sha, head)
        self.assertIn("Recorded AUTO_FORMAT_LAST_SUCCESS_SHA", result.stdout)

    def test_unchanged_head_takes_fast_path_after_workflow_only_pass(self):
        # Once the workflow-only pass records HEAD, a scheduled run whose
        # HEAD still equals it exits before Nix is installed or invoked.
        files = {".github/workflows/ci.yml": "name: ci\n"}
        result, _, recorded_sha, head = self._run_scheduled_pass(files)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(recorded_sha, head)

        result, nix_args, _, _ = self._run_scheduled_pass(
            files, last_success_sha=recorded_sha
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIsNone(nix_args)
        self.assertIn(
            "main is unchanged since the last successful format pass",
            result.stdout,
        )


if __name__ == "__main__":
    unittest.main()
