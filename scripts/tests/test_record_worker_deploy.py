"""Simulation + contract tests for .github/scripts/record-worker-deploy.sh (#15041).

Oracle: the record job of cf-worker-versions-release.yml must never lose
a deploy record to the parallel-release push race. These tests build a
local bare remote holding a fixture workers.json and run the real script
in disposable clones:

- a clone left stale by a competing record push (the exact 2026-11-25
  CF-1 failure: "cannot lock ref 'refs/heads/main'") must recover by
  fetching/rebasing and retrying, and both records must land;
- six clones recording in parallel (one per in-scope Worker) must leave
  workers.json holding a record for every Worker, no manual re-dispatch;
- a remote that permanently rejects pushes must make the script exit
  non-zero with the failure visible, after bounded attempts;
- the #3352 argument contract: any argument, a missing WORKER, or a
  non-SHA DEPLOYED_COMMIT is a usage error (exit 2) before any git or
  file mutation.

Stdlib + git + bash only; runs offline in seconds.
"""

import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / ".github" / "scripts" / "record-worker-deploy.sh"
WORKERS = [
    "github-ci",
    "fleet-hosted",
    "cold-store-rehydrator",
    "fleet-git-rehost",
    "lab-ops-hub",
    "cf-container-hello",
]
REL_WORKERS_JSON = "pm-stack/platform/workers.json"


def git(cwd, *args, check=True):
    result = subprocess.run(
        ["git", *args], cwd=cwd, capture_output=True, text=True
    )
    if check and result.returncode != 0:
        raise AssertionError(
            f"git {' '.join(args)} failed in {cwd}: {result.stderr}"
        )
    return result


def fixture_workers_json():
    return {
        "about": "test fixture for record-worker-deploy (#15041)",
        "checked": "fixture",
        "workers": [
            {
                "name": name,
                "deploy": "hand",
                "workflow": "",
                "lastDeployedCommit": None,
            }
            for name in WORKERS
        ],
    }


class RecordWorkerDeployTest(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="record-worker-deploy-"))
        self.addCleanup(self._cleanup)
        self.remote = self.tmp / "remote.git"
        git(self.tmp, "init", "--bare", "-b", "main", str(self.remote))
        seed = self.tmp / "seed"
        git(self.tmp, "init", "-b", "main", str(seed))
        git(seed, "config", "user.name", "Fixture")
        git(seed, "config", "user.email", "fixture@example.invalid")
        target = seed / REL_WORKERS_JSON
        target.parent.mkdir(parents=True)
        target.write_text(json.dumps(fixture_workers_json(), indent=2) + "\n")
        git(seed, "add", REL_WORKERS_JSON)
        git(seed, "commit", "-q", "-m", "fixture workers.json")
        git(seed, "remote", "add", "origin", str(self.remote))
        git(seed, "push", "-q", "origin", "HEAD:main")

    def _cleanup(self):
        import shutil

        shutil.rmtree(self.tmp, ignore_errors=True)

    def clone(self, name):
        dest = self.tmp / name
        git(self.tmp, "clone", "-q", str(self.remote), str(dest))
        git(dest, "config", "user.name", "Record Test")
        git(dest, "config", "user.email", "record@example.invalid")
        return dest

    def run_script(self, cwd, worker, commit, extra_env=None, args=()):
        env = dict(os.environ)
        env.update(
            {
                "WORKER": worker,
                "DEPLOYED_COMMIT": commit,
                "RETRY_BASE_SECONDS": "0",
            }
        )
        if extra_env:
            env.update(extra_env)
        return subprocess.run(
            ["bash", str(SCRIPT), *args],
            cwd=cwd,
            env=env,
            capture_output=True,
            text=True,
        )

    def remote_workers(self):
        shown = git(
            self.tmp,
            f"--git-dir={self.remote}",
            "show",
            f"main:{REL_WORKERS_JSON}",
        )
        data = json.loads(shown.stdout)
        return {entry["name"]: entry for entry in data["workers"]}

    def assert_recorded(self, workers, name, commit):
        entry = workers[name]
        self.assertEqual(entry["deploy"], "actions", name)
        self.assertEqual(
            entry["workflow"], "cf-worker-versions-release.yml", name
        )
        self.assertEqual(entry["lastDeployedCommit"], commit, name)

    def test_stale_clone_recovers_via_retry(self):
        """The CF-1 race, deterministically: clone B goes stale when A's
        record push lands first; B's script must retry onto current main
        and both records must survive."""
        commit_a = "a" * 40
        commit_b = "b" * 40
        clone_a = self.clone("a")
        clone_b = self.clone("b")  # cloned before A pushes: stale base

        result_a = self.run_script(clone_a, "github-ci", commit_a)
        self.assertEqual(result_a.returncode, 0, result_a.stderr)

        result_b = self.run_script(clone_b, "fleet-hosted", commit_b)
        self.assertEqual(result_b.returncode, 0, result_b.stderr)
        self.assertIn("rejected; retrying", result_b.stderr)

        workers = self.remote_workers()
        self.assert_recorded(workers, "github-ci", commit_a)
        self.assert_recorded(workers, "fleet-hosted", commit_b)

    def test_parallel_records_all_land(self):
        """Simulated parallel release of all six in-scope Workers: every
        record lands with no manual re-dispatch."""
        commits = {name: ("%x" % (i + 1)) * 40 for i, name in enumerate(WORKERS)}
        procs = {}
        for name in WORKERS:
            clone = self.clone(f"par-{name}")
            env = dict(os.environ)
            env.update(
                {
                    "WORKER": name,
                    "DEPLOYED_COMMIT": commits[name],
                    "RETRY_BASE_SECONDS": "0",
                    "MAX_ATTEMPTS": "12",
                }
            )
            procs[name] = subprocess.Popen(
                ["bash", str(SCRIPT)],
                cwd=clone,
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
        for name, proc in procs.items():
            _, stderr = proc.communicate()
            self.assertEqual(proc.returncode, 0, f"{name}: {stderr}")

        workers = self.remote_workers()
        for name in WORKERS:
            self.assert_recorded(workers, name, commits[name])

    def test_permanent_push_failure_surfaces(self):
        """A remote whose pre-receive hook always rejects: bounded
        attempts, non-zero exit, the push error visible on stderr."""
        # Seed the blocked remote by copying the good remote (a push
        # into it would be rejected by the very hook under test), then
        # install a pre-receive hook that rejects every push.
        import shutil

        blocked = self.tmp / "blocked.git"
        shutil.copytree(self.remote, blocked)
        hook = blocked / "hooks" / "pre-receive"
        hook.write_text("#!/bin/sh\necho 'pushes are disabled in this fixture' >&2\nexit 1\n")
        hook.chmod(0o755)

        clone = self.tmp / "blocked-clone"
        git(self.tmp, "clone", "-q", str(blocked), str(clone))
        git(clone, "config", "user.name", "Record Test")
        git(clone, "config", "user.email", "record@example.invalid")

        result = self.run_script(
            clone, "github-ci", "c" * 40, extra_env={"MAX_ATTEMPTS": "3"}
        )
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("FAILED to record", result.stderr)
        self.assertIn("pushes are disabled in this fixture", result.stderr)

    def test_idempotent_when_already_recorded(self):
        commit = "d" * 40
        first = self.run_script(self.clone("idem-1"), "github-ci", commit)
        self.assertEqual(first.returncode, 0, first.stderr)
        second = self.run_script(self.clone("idem-2"), "github-ci", commit)
        self.assertEqual(second.returncode, 0, second.stderr)
        self.assertIn("already records", second.stdout)
        self.assert_recorded(self.remote_workers(), "github-ci", commit)

    def test_usage_contract_rejects_before_any_mutation(self):
        clone = self.clone("usage")
        head_before = git(clone, "rev-parse", "HEAD").stdout.strip()

        with_arg = self.run_script(clone, "github-ci", "e" * 40, args=("--bogus",))
        self.assertEqual(with_arg.returncode, 2, with_arg.stderr)

        env = dict(os.environ)
        env.pop("WORKER", None)
        env["DEPLOYED_COMMIT"] = "e" * 40
        missing_worker = subprocess.run(
            ["bash", str(SCRIPT)],
            cwd=clone,
            env=env,
            capture_output=True,
            text=True,
        )
        self.assertEqual(missing_worker.returncode, 2, missing_worker.stderr)

        bad_sha = self.run_script(clone, "github-ci", "not-a-sha")
        self.assertEqual(bad_sha.returncode, 2, bad_sha.stderr)

        # Nothing moved: no commit, clean tree.
        self.assertEqual(git(clone, "rev-parse", "HEAD").stdout.strip(), head_before)
        self.assertEqual(git(clone, "status", "--porcelain").stdout, "")


if __name__ == "__main__":
    unittest.main()
