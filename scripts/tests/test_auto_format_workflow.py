"""Exercise the scheduled auto-format workflow using a real git history."""

import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile
import time
import unittest


WORKFLOW = Path(__file__).resolve().parents[2] / ".github/workflows/auto-format.yml"
TEST_COMMIT_DATE = time.strftime("%Y-%m-%dT%H:00:00Z", time.gmtime())


class AutoFormatWorkflowTest(unittest.TestCase):
    def _run_scheduled_pass(
        self,
        files,
        last_success_sha=None,
        base_files=None,
        formatter_edits=None,
        formatter_script=None,
        race_files=None,
    ):
        # Run the workflow's real run block against a repo whose history
        # contains exactly the given files. `gh` and `nix` are stubbed so
        # record_success is observable and no network or Nix is touched.
        # `base_files` are committed first; pass last_success_sha="base"
        # to diff against that commit, so `files` are the only changed
        # (listed) files. `formatter_edits` maps path -> content the nix
        # stub writes when invoked, simulating changes treefmt makes to
        # files that were not listed (e.g. rustfmt mod children).
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

            def git(*args, **kwargs):
                subprocess.run(
                    ["git", *args], cwd=repo, check=True, capture_output=True, **kwargs
                )

            subprocess.run(["git", "init", "-q", str(repo)], check=True)
            git("config", "user.name", "Test")
            git("config", "user.email", "test@example.com")

            def commit_files(contents, message):
                for name, content in contents.items():
                    path = repo / name
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_text(content)
                git("add", ".")
                git(
                    "commit",
                    "-qm",
                    message,
                    env={
                        **os.environ,
                        "GIT_AUTHOR_DATE": TEST_COMMIT_DATE,
                        "GIT_COMMITTER_DATE": TEST_COMMIT_DATE,
                    },
                )
                return subprocess.run(
                    ["git", "rev-parse", "HEAD"],
                    cwd=repo,
                    check=True,
                    capture_output=True,
                    text=True,
                ).stdout.strip()

            base_head = None
            if base_files:
                base_head = commit_files(base_files, "base")
            head = commit_files(files, "initial")

            # A bare origin so the write pass can really push, and the
            # test can inspect the pushed commit afterwards.
            origin = root / "origin.git"
            subprocess.run(["git", "init", "-q", "--bare", str(origin)], check=True)
            git("remote", "add", "origin", str(origin))
            git("push", "-q", "origin", "HEAD:main")

            bindir = root / "bin"
            bindir.mkdir()
            nix = bindir / "nix"
            nix_lines = [
                "#!/bin/sh\n",
                'printf "%s\\n" "$@" >> "$NIX_ARGS"\n',
            ]
            for name, content in (formatter_edits or {}).items():
                parent = str(Path(name).parent)
                if parent != ".":
                    nix_lines.append(f"mkdir -p {shlex.quote(parent)}\n")
                nix_lines.append(
                    f"printf '%s' {shlex.quote(content)} > {shlex.quote(name)}\n"
                )
            if formatter_script:
                nix_lines.append(formatter_script)
                if not formatter_script.endswith("\n"):
                    nix_lines.append("\n")
            nix.write_text("".join(nix_lines))
            nix.chmod(0o755)
            args = root / "nix-args"
            recorded = root / "recorded-sha"
            # The workflow updates the variable with the HEAD value; the
            # stub records whatever value it was asked to store.
            gh = bindir / "gh"
            gh.write_text(
                "#!/bin/sh\n"
                "while [ $# -gt 1 ]; do\n"
                '  if [ "$1" = "-f" ]; then\n'
                '    case "$2" in\n'
                '      value=*) printf "%s\\n" "${2#value=}" > "$GH_RECORD_FILE"; exit 0;;\n'
                "    esac\n"
                "  fi\n"
                "  shift\n"
                "done\n"
                "exit 0\n"
            )
            gh.chmod(0o755)
            race_env = {}
            if race_files:
                real_git = shutil.which("git")
                self.assertIsNotNone(real_git)
                race_repo = root / "race-repo"
                marker = root / "race-injected"
                git_lines = [
                    "#!/bin/sh\n",
                    'if [ "$1" = "push" ] && [ ! -e "$GIT_RACE_MARKER" ]; then\n',
                    '  : > "$GIT_RACE_MARKER"\n',
                    f'  {shlex.quote(real_git)} clone -q -b main "$GIT_RACE_ORIGIN" "$GIT_RACE_REPO"\n',
                    f'  {shlex.quote(real_git)} -C "$GIT_RACE_REPO" config user.name "Concurrent writer"\n',
                    f'  {shlex.quote(real_git)} -C "$GIT_RACE_REPO" config user.email concurrent@example.com\n',
                ]
                for name, content in race_files.items():
                    parent = str(Path(name).parent)
                    if parent != ".":
                        git_lines.append(
                            f'  mkdir -p "$GIT_RACE_REPO/{shlex.quote(parent)}"\n'
                        )
                    git_lines.append(
                        f"  printf '%s' {shlex.quote(content)} > \"$GIT_RACE_REPO/{shlex.quote(name)}\"\n"
                    )
                git_lines.extend(
                    [
                        f'  {shlex.quote(real_git)} -C "$GIT_RACE_REPO" add -A\n',
                        f"  {shlex.quote(real_git)} -C \"$GIT_RACE_REPO\" commit -qm 'concurrent main update'\n",
                        f'  {shlex.quote(real_git)} -C "$GIT_RACE_REPO" push -q origin main\n',
                        "fi\n",
                        f'exec {shlex.quote(real_git)} "$@"\n',
                    ]
                )
                git_wrapper = bindir / "git"
                git_wrapper.write_text("".join(git_lines))
                git_wrapper.chmod(0o755)
                race_env = {
                    "GIT_RACE_ORIGIN": str(origin),
                    "GIT_RACE_REPO": str(race_repo),
                    "GIT_RACE_MARKER": str(marker),
                }
            env = {
                **os.environ,
                "PATH": f"{bindir}:{os.environ['PATH']}",
                "GITHUB_EVENT_NAME": "schedule",
                "GITHUB_REPOSITORY": "developing-today/code",
                "REPO_AUTOMATION_TOKEN": "test-token",
                "GH_TOKEN": "test-token",
                "NIX_ARGS": str(args),
                "GH_RECORD_FILE": str(recorded),
                "GH_RECORD_VALUE": head,
                **race_env,
            }
            if last_success_sha == "base":
                last_success_sha = base_head
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
            pushed_head = subprocess.run(
                ["git", "--git-dir", str(origin), "rev-parse", "main"],
                check=True,
                capture_output=True,
                text=True,
            ).stdout.strip()
            pushed_tree = {}
            names = subprocess.run(
                [
                    "git",
                    "--git-dir",
                    str(origin),
                    "ls-tree",
                    "-r",
                    "--name-only",
                    "main",
                ],
                check=True,
                capture_output=True,
                text=True,
            ).stdout.splitlines()
            for name in names:
                pushed_tree[name] = subprocess.run(
                    ["git", "--git-dir", str(origin), "show", f"main:{name}"],
                    check=True,
                    capture_output=True,
                    text=True,
                ).stdout
            return result, nix_args, recorded_sha, head, pushed_tree, pushed_head

    def test_deleted_final_path_does_not_abort_scheduled_filter(self):
        result, nix_args, _, _, _, _ = self._run_scheduled_pass(
            {"a-existing": "keep\n", "z-deleted": "delete\n"}
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("a-existing", result.stdout)

    def test_workflow_files_are_formatted_like_everything_else(self):
        # Under the REPO_AUTOMATION_TOKEN PAT arrangement the bot can
        # push workflow files, so nothing is excluded from the write
        # pass (#3118, superseding the old GITHUB_TOKEN skip behavior).
        result, nix_args, _, _, _, _ = self._run_scheduled_pass(
            {
                "notes.txt": "keep\n",
                ".github/workflows/ci.yml": "name: ci\n",
                ".github/workflows/check.yaml": "name: check\n",
            }
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            nix_args,
            [
                "run",
                ".#format",
                "--",
                ".github/workflows/check.yaml",
                ".github/workflows/ci.yml",
                "notes.txt",
            ],
        )

    def test_workflow_only_change_records_success_after_formatting(self):
        # Workflow-only change under the PAT arrangement: the formatter
        # runs on the workflow files; with no resulting changes the
        # pass records success so the next run can take the fast path.
        result, nix_args, recorded_sha, head, _, _ = self._run_scheduled_pass(
            {
                ".github/workflows/ci.yml": "name: ci\n",
                ".github/workflows/check.yaml": "name: check\n",
            }
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            nix_args,
            [
                "run",
                ".#format",
                "--",
                ".github/workflows/check.yaml",
                ".github/workflows/ci.yml",
            ],
        )
        self.assertIn("treefmt made no changes.", result.stdout)
        self.assertEqual(recorded_sha, head)
        self.assertIn("Recorded AUTO_FORMAT_LAST_SUCCESS_SHA", result.stdout)

    def test_unchanged_head_takes_fast_path_after_workflow_only_pass(self):
        # Once a workflow-only pass records HEAD, a scheduled run whose
        # HEAD still equals it exits before Nix is installed or invoked.
        files = {".github/workflows/ci.yml": "name: ci\n"}
        result, _, recorded_sha, head, _, _ = self._run_scheduled_pass(files)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(recorded_sha, head)

        result, nix_args, _, _, _, _ = self._run_scheduled_pass(
            files, last_success_sha=recorded_sha
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIsNone(nix_args)
        self.assertIn(
            "main is unchanged since the last successful format pass",
            result.stdout,
        )

    def test_companion_change_is_committed_and_pushed(self):
        # Regression test for #5014: the listed file (parent.rs) is
        # already formatted, but the formatter rewrites an unlisted
        # tracked companion (child.rs, an out-of-line mod child, as
        # rustfmt does). The run must exit 0 and the companion change
        # must be in the pushed commit — not stranded, and not failing
        # the commit with "no changes added to commit".
        result, nix_args, recorded_sha, head, pushed_tree, pushed_head = (
            self._run_scheduled_pass(
                {"parent.rs": "mod child;\n\npub fn parent() {}\n"},
                last_success_sha="base",
                base_files={"child.rs": "pub fn child( ){}\n"},
                formatter_edits={"child.rs": "pub fn child() {}\n"},
            )
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(nix_args, ["run", ".#format", "--", "parent.rs"])
        self.assertNotEqual(pushed_head, head)
        self.assertEqual(pushed_tree["child.rs"], "pub fn child() {}\n")
        self.assertEqual(pushed_tree["parent.rs"], "mod child;\n\npub fn parent() {}\n")
        self.assertEqual(recorded_sha, pushed_head)

    def test_push_race_restarts_formatting_from_new_main(self):
        readme = "pm-stack/aggregator/README.md"
        result, nix_args, recorded_sha, _, pushed_tree, pushed_head = (
            self._run_scheduled_pass(
                {readme: "Before race   \n"},
                last_success_sha="base",
                base_files={readme: "Base\n"},
                race_files={readme: "Concurrent main edit   \n"},
                formatter_script=(
                    'for file in "$@"; do\n'
                    '  if [ "$file" = "pm-stack/aggregator/README.md" ]; then\n'
                    '    sed -i "s/[[:blank:]]*$//" "$file"\n'
                    "  fi\n"
                    "done\n"
                ),
            )
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(nix_args, ["run", ".#format", "--", readme] * 2)
        self.assertEqual(pushed_tree[readme], "Concurrent main edit\n")
        self.assertEqual(recorded_sha, pushed_head)
        self.assertIn("main advanced during the format pass", result.stdout)


if __name__ == "__main__":
    unittest.main()
