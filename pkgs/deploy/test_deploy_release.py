#!/usr/bin/env python3
"""Coverage for the deploy release slice (issue #13306).

Oracle: doc/2026-11-23T13-09-25Z_design_release-workflow-per-app-tags
(the release-workflow design record, #11177) — §2 preconditions and
order, §3 attestation judging (the rules .github/scripts/release-gate.sh
implements), §4 the attestation block and the ledger, §5 the repo tag,
§6 the deploy-time refusal.

git effects are REAL against a fixture repository and bare remote in a
temp dir; gh effects are faked by a run_cmd wrapper that delegates git
to the real runner and answers gh api endpoints from fixtures. The
artifact version probe is replaced like probe_health is in the
slice-3 tests.

Usage: test_deploy_release.py [path-to-deploy.py]
"""

import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import unittest
from datetime import datetime, timezone

DEPLOY_PATH = (
    sys.argv[1]
    if len(sys.argv) > 1
    else os.path.join(os.path.dirname(os.path.abspath(__file__)), "deploy.py")
)
spec = importlib.util.spec_from_file_location("deploy", DEPLOY_PATH)
deploy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(deploy)

BASE_DEFN = {
    "package": "/placeholder/pkg",
    "entryPoint": "bin/svc",
    "args": [],
    "environment": {},
    "secrets": [],
    "user": "user",
    "ports": [8080],
    "restart": "on-failure",
    "healthCheck": None,
    "targets": ["amd"],
    "class": "systemd",
}


def defn(**overrides):
    merged = dict(BASE_DEFN)
    merged.update(overrides)
    return merged


# The fleet app is deployed as two services running bin/fleet; mcpx is
# a definition named after its app with no evidence workflow.
REGISTRY = {
    "queue-broker": defn(entryPoint="bin/fleet", evidenceWorkflow="fleet-rust-tests"),
    "secrets-broker": defn(entryPoint="bin/fleet", evidenceWorkflow="fleet-rust-tests"),
    "mcpx": defn(entryPoint="bin/mcpx"),
}


def git(repo, *args):
    proc = subprocess.run(["git", "-C", repo, *args], capture_output=True, text=True)
    if proc.returncode != 0:
        raise AssertionError(f"git {' '.join(args)} failed: {proc.stderr}")
    return proc.stdout.strip()


class FakeGh:
    """Answers the gh api endpoints deploy.py reads, from fixtures."""

    def __init__(self, parent_sha, head_sha):
        self.workflows = [
            {"id": i + 1, "name": name}
            for i, name in enumerate(deploy.WATCHED_WORKFLOWS)
        ] + [{"id": 10, "name": "fleet-rust-tests"}]
        self.runs = {}
        for wf in self.workflows:
            sha = head_sha if wf["name"] == "fleet-rust-tests" else parent_sha
            self.runs[wf["id"]] = [
                {
                    "id": 100 + wf["id"],
                    "head_sha": sha,
                    "status": "completed",
                    "conclusion": "success",
                    "created_at": "2026-11-23T00:00:00Z",
                }
            ]
        self.tag_refs = {}  # tag name -> tag object sha
        self.tag_objects = {}  # tag object sha -> {"message", "commit"}

    def serve_tag(self, name, message, commit):
        obj_sha = "a" * 40 if not self.tag_objects else "b" * 40
        self.tag_refs[name] = obj_sha
        self.tag_objects[obj_sha] = {"message": message, "commit": commit}

    def __call__(self, argv):
        if argv[0] == "gh":
            endpoint = argv[2].split("/", 3)[3]  # after repos/<slug>/
            if endpoint.startswith("actions/workflows?"):
                return 0, json.dumps({"workflows": self.workflows}), ""
            if endpoint.startswith("actions/workflows/"):
                wid = int(endpoint.split("/")[2])
                return (
                    0,
                    json.dumps({"workflow_runs": self.runs.get(wid, [])}),
                    "",
                )
            if endpoint.startswith("git/ref/tags/"):
                name = endpoint[len("git/ref/tags/") :]
                if name not in self.tag_refs:
                    return 1, "", "gh: Not Found (HTTP 404)"
                return (
                    0,
                    json.dumps({"object": {"sha": self.tag_refs[name], "type": "tag"}}),
                    "",
                )
            if endpoint.startswith("git/tags/"):
                obj = self.tag_objects.get(endpoint[len("git/tags/") :])
                if obj is None:
                    return 1, "", "gh: Not Found (HTTP 404)"
                return (
                    0,
                    json.dumps(
                        {
                            "message": obj["message"],
                            "object": {"sha": obj["commit"], "type": "commit"},
                        }
                    ),
                    "",
                )
            return 1, "", f"unexpected gh endpoint {endpoint}"
        if argv[0] == "git":
            return self.real_run_cmd(argv)
        return 0, "", ""


class ReleaseFixture(unittest.TestCase):
    def setUp(self):
        self._real_run_cmd = deploy.run_cmd
        self.tmp = tempfile.mkdtemp(prefix="deploy-release-test-")
        self.remote = os.path.join(self.tmp, "remote.git")
        self.work = os.path.join(self.tmp, "work")
        self.state = os.path.join(self.tmp, "state")
        subprocess.run(
            ["git", "init", "--bare", self.remote],
            capture_output=True,
            check=True,
        )
        subprocess.run(["git", "init", self.work], capture_output=True, check=True)
        git(self.work, "symbolic-ref", "HEAD", "refs/heads/main")
        git(self.work, "config", "user.email", "test@example.invalid")
        git(self.work, "config", "user.name", "deploy test")
        git(self.work, "remote", "add", "origin", self.remote)
        with open(os.path.join(self.work, "a.txt"), "w") as fh:
            fh.write("one\n")
        git(self.work, "add", "a.txt")
        git(self.work, "commit", "-qm", "one")
        self.parent = git(self.work, "rev-parse", "HEAD")
        with open(os.path.join(self.work, "b.txt"), "w") as fh:
            fh.write("two\n")
        git(self.work, "add", "b.txt")
        git(self.work, "commit", "-qm", "two")
        self.head = git(self.work, "rev-parse", "HEAD")
        git(self.work, "push", "origin", "main")
        self.gh = FakeGh(self.parent, self.head)
        self.gh.real_run_cmd = self._real_run_cmd
        deploy.run_cmd = self.gh
        self._old_env = {
            k: os.environ.get(k) for k in ("DEPLOY_GH_REPO", "DEPLOY_SOURCE_REV")
        }
        os.environ["DEPLOY_GH_REPO"] = "owner/repo"

    def tearDown(self):
        deploy.run_cmd = self._real_run_cmd
        for key, value in self._old_env.items():
            if value is None:
                os.environ.pop(key, None)
            else:
                os.environ[key] = value

    def remote_tag_names(self):
        out = git(self.work, "ls-remote", "--tags", "origin")
        names = []
        for line in out.splitlines():
            if not line:
                continue
            ref = line.split("\t")[1]
            name = ref[len("refs/tags/") :]
            if not name.endswith("^{}"):
                names.append(name)
        return sorted(names)

    def release(self, app="fleet", **kwargs):
        kwargs.setdefault("repo", self.work)
        kwargs.setdefault("state", self.state)
        kwargs.setdefault("actor", "test-actor")
        return deploy.execute_release(app, REGISTRY, **kwargs)

    def today(self):
        return datetime.now(timezone.utc).strftime("%Y%m%d")


class TestReleaseSuccess(ReleaseFixture):
    def test_release_against_fixture_remote(self):
        code, lines = self.release()
        self.assertEqual(code, 0, lines)
        repo_tag = f"repo/v{self.today()}.1.0"
        self.assertEqual(self.remote_tag_names(), ["fleet/v0.1.0", repo_tag])
        # Both tags point at the head commit.
        self.assertEqual(
            git(self.work, "rev-parse", "fleet/v0.1.0^{commit}"), self.head
        )
        # The attestation block parses and verifies (the §4 record).
        message = git(
            self.work,
            "for-each-ref",
            "refs/tags/fleet/v0.1.0",
            "--format=%(contents)",
        )
        parsed = deploy.verify_attestation(message, "fleet", "0.1.0", self.head)
        self.assertIsNotNone(parsed)
        judged = {wf for wf, _id, _c, _s in parsed["runs"]}
        self.assertEqual(judged, set(deploy.WATCHED_WORKFLOWS) | {"fleet-rust-tests"})
        # The ledger round-trips through the status report: the tag
        # confirms the record, no disagreement.
        entries = deploy.read_releases(self.state)
        self.assertEqual(len(entries), 1)
        self.assertEqual(entries[0]["tag"], "fleet/v0.1.0")
        self.assertEqual(entries[0]["repo_tag"], repo_tag)
        report = deploy.format_releases(
            entries, deploy.local_tag_attestations(self.work)
        )
        text = "\n".join(report)
        self.assertIn("fleet/v0.1.0: app fleet version 0.1.0", text)
        self.assertIn("attested", text)
        self.assertNotIn("DISAGREEMENT", text)

    def test_second_and_third_releases_follow_the_discipline(self):
        code, lines = self.release()
        self.assertEqual(code, 0, lines)
        code, lines = self.release(bump="patch")
        self.assertEqual(code, 0, lines)
        code, lines = self.release()
        self.assertEqual(code, 0, lines)
        self.assertEqual(
            self.remote_tag_names(),
            [
                "fleet/v0.1.0",
                "fleet/v0.1.1",
                "fleet/v0.2.0",
                f"repo/v{self.today()}.1.0",
                f"repo/v{self.today()}.1.1",
                f"repo/v{self.today()}.2.0",
            ],
        )

    def test_dry_run_cuts_nothing(self):
        code, lines = self.release(dry_run=True)
        self.assertEqual(code, 0, lines)
        self.assertIn("would cut fleet/v0.1.0", "\n".join(lines))
        self.assertEqual(self.remote_tag_names(), [])
        self.assertEqual(deploy.read_releases(self.state), [])


class TestReleaseRefusals(ReleaseFixture):
    def test_red_gate_refuses(self):
        self.gh.runs[2][0]["conclusion"] = "failure"  # workflow "id"
        code, lines = self.release()
        self.assertEqual(code, 1)
        self.assertIn("REFUSED", "\n".join(lines))
        self.assertIn("attestation gate is not green", "\n".join(lines))
        self.assertEqual(self.remote_tag_names(), [])
        self.assertEqual(deploy.read_releases(self.state), [])

    def test_missing_gate_signal_refuses(self):
        self.gh.runs[1][0]["head_sha"] = "f" * 40  # not in the history
        code, lines = self.release()
        self.assertEqual(code, 1)
        self.assertIn("no completed run on main at or before", "\n".join(lines))
        self.assertEqual(self.remote_tag_names(), [])

    def test_commit_off_main_refuses(self):
        git(self.work, "checkout", "-qb", "side")
        with open(os.path.join(self.work, "c.txt"), "w") as fh:
            fh.write("three\n")
        git(self.work, "add", "c.txt")
        git(self.work, "commit", "-qm", "three")
        side = git(self.work, "rev-parse", "HEAD")
        code, lines = self.release(commit_arg=side)
        self.assertEqual(code, 1)
        self.assertIn("not reachable from origin/main", "\n".join(lines))
        self.assertEqual(self.remote_tag_names(), [])

    def test_version_discipline_refusals(self):
        # First tag must be 0.1.0.
        code, lines = self.release(app="mcpx", requested_version="0.2.0")
        self.assertEqual(code, 1)
        self.assertIn("the first tag for an app is 0.1.0", "\n".join(lines))
        # Major bumps are refused outright.
        code, lines = self.release(app="mcpx", requested_version="1.0.0")
        self.assertEqual(code, 1)
        self.assertIn("leaves 0.x", "\n".join(lines))
        code, lines = self.release()
        self.assertEqual(code, 0, lines)
        # Skipping a version is refused.
        code, lines = self.release(requested_version="0.3.0")
        self.assertEqual(code, 1)
        self.assertIn("is not the next minor", "\n".join(lines))
        # Re-cutting the same version at the same commit is refused.
        code, lines = self.release(requested_version="0.1.0")
        self.assertEqual(code, 1)
        self.assertIn("already released", "\n".join(lines))
        self.assertEqual(
            self.remote_tag_names(), ["fleet/v0.1.0", f"repo/v{self.today()}.1.0"]
        )

    def test_unknown_app_refuses(self):
        code, lines = self.release(app="nope")
        self.assertEqual(code, 1)
        self.assertIn("not in the deploy registry", "\n".join(lines))


class TestDeployTimeRefusal(ReleaseFixture):
    def setUp(self):
        super().setUp()
        self.defn = defn(entryPoint="bin/fleet")
        self._real_probe = deploy.probe_artifact_version
        deploy.probe_artifact_version = lambda defn: "0.1.0"
        os.environ["DEPLOY_SOURCE_REV"] = self.head

    def tearDown(self):
        deploy.probe_artifact_version = self._real_probe
        super().tearDown()

    def deploy_it(self):
        return deploy.execute_deploy(
            "queue-broker",
            self.defn,
            [],
            base=os.path.join(self.tmp, "profiles"),
            state=self.state,
            scope="user",
            units=os.path.join(self.tmp, "units"),
            actor="test-actor",
        )

    def test_unattested_release_claim_is_refused(self):
        code, lines = self.deploy_it()
        self.assertEqual(code, 1)
        self.assertIn("REFUSED", "\n".join(lines))
        self.assertIn("deploy refused (design §6)", "\n".join(lines))
        # Nothing was installed, and the refusal is on the record.
        self.assertFalse(
            os.path.exists(os.path.join(self.tmp, "profiles", "queue-broker"))
        )
        entries = deploy.read_generations("queue-broker", self.state)
        self.assertEqual(entries[-1]["action"], "deploy_failed")
        self.assertIn("refused", entries[-1]["result"])

    def test_attested_release_claim_deploys(self):
        block = deploy.compose_attestation(
            "fleet",
            "0.1.0",
            self.head,
            "fleet-rust-tests",
            [
                {
                    "workflow": "nix ci",
                    "run_id": 101,
                    "head_sha": self.parent,
                    "conclusion": "success",
                }
            ],
            "test-actor",
            "2026-11-23T00:00:00Z",
        )
        self.gh.serve_tag("fleet/v0.1.0", f"fleet v0.1.0\n\n{block}\n", self.head)
        code, lines = self.deploy_it()
        self.assertEqual(code, 0, lines)

    def test_prerelease_claim_is_not_a_release_claim(self):
        deploy.probe_artifact_version = lambda defn: "0.1.0-dev.3+gabc"
        self.assertIsNone(deploy.release_claim_refusal("queue-broker", self.defn))
        deploy.probe_artifact_version = lambda defn: None
        self.assertIsNone(deploy.release_claim_refusal("queue-broker", self.defn))


class TestAttestationAndNumbering(unittest.TestCase):
    def test_parse_roundtrip_and_tampering(self):
        runs = [
            {
                "workflow": "mcpx nix build",
                "run_id": 42,
                "head_sha": "b" * 40,
                "conclusion": "success",
            }
        ]
        block = deploy.compose_attestation(
            "fleet", "0.1.0", "c" * 40, None, runs, "actor", "2026-11-23T00:00:00Z"
        )
        message = f"fleet v0.1.0\n\n{block}\n"
        parsed = deploy.verify_attestation(message, "fleet", "0.1.0", "c" * 40)
        self.assertIsNotNone(parsed)
        self.assertEqual(parsed["runs"][0][0], "mcpx nix build")
        self.assertIsNone(
            deploy.verify_attestation(message, "fleet", "0.1.0", "d" * 40)
        )
        self.assertIsNone(deploy.parse_attestation("fleet v0.1.0\n\nno block\n"))
        self.assertIsNone(deploy.parse_attestation(""))

    def test_plan_release_version(self):
        self.assertEqual(deploy.plan_release_version([], None, "minor"), (0, 1, 0))
        self.assertEqual(
            deploy.plan_release_version([(0, 1, 0)], None, "patch"), (0, 1, 1)
        )
        self.assertEqual(
            deploy.plan_release_version([(0, 1, 1)], (0, 2, 0), "minor"), (0, 2, 0)
        )
        with self.assertRaises(deploy.ReleaseError):
            deploy.plan_release_version([(0, 1, 0)], (0, 3, 0), "minor")
        with self.assertRaises(deploy.ReleaseError):
            deploy.plan_release_version([], (1, 0, 0), "minor")

    def test_next_repo_tag(self):
        self.assertEqual(
            deploy.next_repo_tag([], "20261123", True), "repo/v20261123.1.0"
        )
        self.assertEqual(
            deploy.next_repo_tag(["repo/v20261123.1.0"], "20261123", False),
            "repo/v20261123.1.1",
        )
        self.assertEqual(
            deploy.next_repo_tag(["repo/v20261123.1.1"], "20261123", True),
            "repo/v20261123.2.0",
        )
        self.assertEqual(
            deploy.next_repo_tag(["repo/v20261122.2.3"], "20261123", False),
            "repo/v20261123.0.1",
        )

    def test_evidence_resolution(self):
        self.assertEqual(
            deploy.app_evidence_workflow(REGISTRY, "fleet"), "fleet-rust-tests"
        )
        self.assertIsNone(deploy.app_evidence_workflow(REGISTRY, "mcpx"))
        conflicting = {
            "a": defn(entryPoint="bin/x", evidenceWorkflow="one"),
            "b": defn(entryPoint="bin/x", evidenceWorkflow="two"),
        }
        with self.assertRaises(deploy.ReleaseError):
            deploy.app_evidence_workflow(conflicting, "x")

    def test_registry_validation_of_evidence_workflow(self):
        good = {"svc": defn(evidenceWorkflow="fleet-rust-tests")}
        self.assertEqual(deploy.validate(good), [])
        bad = {"svc": defn(evidenceWorkflow=7)}
        self.assertTrue(any("evidenceWorkflow" in e for e in deploy.validate(bad)))

    def test_ledger_disagreement_tag_wins(self):
        entries = [
            {
                "app": "fleet",
                "version": "0.1.0",
                "commit": "e" * 40,
                "tag": "fleet/v0.1.0",
            },
            {
                "app": "mcpx",
                "version": "0.2.0",
                "commit": "e" * 40,
                "tag": "mcpx/v0.2.0",
            },
        ]
        block = deploy.compose_attestation(
            "fleet",
            "0.1.0",
            "c" * 40,
            None,
            [
                {
                    "workflow": "id",
                    "run_id": 7,
                    "head_sha": "b" * 40,
                    "conclusion": "success",
                }
            ],
            "actor",
            "2026-11-23T00:00:00Z",
        )
        attestations = {"fleet/v0.1.0": deploy.parse_attestation(block)}
        text = "\n".join(deploy.format_releases(entries, attestations))
        # The tag's commit wins over the ledger's.
        self.assertIn("DISAGREEMENT", text)
        self.assertIn("the tag wins", text)
        # A ledger record no tag confirms is not a release.
        self.assertIn("mcpx/v0.2.0", text)
        self.assertIn("not a release", text)


if __name__ == "__main__":
    unittest.main(argv=[sys.argv[0]] + sys.argv[2:])
