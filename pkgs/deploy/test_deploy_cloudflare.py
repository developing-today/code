#!/usr/bin/env python3
"""Coverage for the Cloudflare lifecycle slice (issue #5767, slice 4).

Oracle: the slice criterion for cf-container-hello through the generic
deploy CLI.

- status reports the version/deployment currently serving from
  Cloudflare (the provider), never an inference from the local log.
- deploy publishes the flake-built package and appends the provider
  version/deployment ids to the generation log only after success.
- rollback re-activates the previously recorded version without
  rebuilding/re-uploading and refuses before mutation when the
  version lacks the required bindings.
- dry-run plans without contacting Cloudflare or mutating state.

Stdlib-only, deterministic. The provider boundary (cf_*) is replaced
with fakes; no live deploy happens here. CLI dry-run cases run
deploy.py as a subprocess against a fixture registry in a temp state
dir (dry-run never calls the provider, so no fake is needed there).

Usage: test_deploy_cloudflare.py [path-to-deploy.py]
"""

import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import unittest

DEPLOY_PATH = (
    sys.argv[1]
    if len(sys.argv) > 1 and os.path.isfile(sys.argv[1])
    else os.path.join(os.path.dirname(os.path.abspath(__file__)), "deploy.py")
)

spec = importlib.util.spec_from_file_location("deploy", DEPLOY_PATH)
deploy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(deploy)

DEFN = {
    "package": "/nix/store/cf-container-hello-1.0",
    "entryPoint": "bin/cf-container-hello-deploy",
    "args": [],
    "environment": {},
    "secrets": [],
    "user": "nobody",
    "ports": [],
    "restart": "on-failure",
    "healthCheck": None,
    "targets": ["cloudflare"],
    "class": "cloudflare",
}

NAME = "cf-container-hello"
REQUIRED = list(deploy.REQUIRED_CF_CONTAINER_HELLO_BINDINGS)


def record(state, gen, **overrides):
    entry = {
        "artifact": "/nix/store/cf-container-hello-1.0",
        "source_rev": f"rev{gen}",
        "actor": "tester",
        "action": "deploy",
        "generation": gen,
        "result": "activated",
        "at": f"2026-11-08T00:0{gen}:00Z",
    }
    entry.update(overrides)
    return deploy.record_generation(
        NAME,
        artifact=entry.pop("artifact"),
        source_rev=entry.pop("source_rev"),
        actor=entry.pop("actor"),
        action=entry.pop("action"),
        generation=entry.pop("generation"),
        result=entry.pop("result"),
        at=entry.pop("at"),
        explicit_state=state,
        **{k: v for k, v in entry.items() if k.startswith("cloudflare_")},
    )


class PlanTests(unittest.TestCase):
    def test_deploy_plan_publishes_and_records_ids(self):
        steps = deploy.plan_deploy(NAME, DEFN, [])
        text = "\n".join(steps)
        self.assertIn("flake-built", text)
        self.assertIn("publish", text)
        self.assertIn("required bindings", text)
        self.assertIn("only after success", text)

    def test_deploy_plan_is_pure_no_provider_contact(self):
        # The plan is a pure function of (name, defn, entries): building
        # it must not touch the provider boundary.
        orig = deploy.cf_get_serving

        def boom(*a, **k):
            raise AssertionError("plan contacted Cloudflare")

        deploy.cf_get_serving = boom
        try:
            steps = deploy.plan_deploy(NAME, DEFN, [])
            self.assertTrue(steps)
        finally:
            deploy.cf_get_serving = orig

    def test_other_cloudflare_names_still_refuse(self):
        # Slice-2 oracle stays green: only the lifecycle set has a
        # Cloudflare path; any other cloudflare/container target still
        # reports the different-slice refusal.
        other = dict(DEFN)
        self.assertIsNone(deploy.plan_deploy("other-worker", other, []))
        steps, error = deploy.plan_rollback("other-worker", other, [])
        self.assertIsNone(steps)
        self.assertIn("not an activation-class", error)

    def test_rollback_plan_needs_recorded_version(self):
        with tempfile.TemporaryDirectory() as state:
            record(state, 1)  # no version recorded
            record(state, 2, cloudflare_version="v2", cloudflare_deployment="d2")
            _steps, error = deploy.plan_rollback(
                NAME, DEFN, deploy.read_generations(NAME, state)
            )
            self.assertIsNone(_steps)
            self.assertIn("lacks a recorded Cloudflare version", error)

    def test_rollback_plan_is_no_rebuild(self):
        with tempfile.TemporaryDirectory() as state:
            record(state, 1, cloudflare_version="v1", cloudflare_deployment="d1")
            record(state, 2, cloudflare_version="v2", cloudflare_deployment="d2")
            steps, error = deploy.plan_rollback(
                NAME, DEFN, deploy.read_generations(NAME, state)
            )
            self.assertIsNone(error)
            text = "\n".join(steps)
            self.assertIn("v1", text)
            self.assertIn("no rebuild, no re-upload", text)
            self.assertIn("refuse before mutation", text)


class StatusTests(unittest.TestCase):
    def test_status_reports_serving_not_log_inference(self):
        # The log says v1; the provider serves v9: status must report
        # the provider's v9, never infer v1 from the log.
        with tempfile.TemporaryDirectory() as state:
            record(state, 1, cloudflare_version="v1", cloudflare_deployment="d1")
            entries = deploy.read_generations(NAME, state)
            out = deploy.format_cloudflare_status(
                NAME,
                DEFN,
                entries,
                serving={"version_id": "v9", "deployment_id": "d9"},
            )
            self.assertIn("serving: cloudflare version v9 deployment d9", out)
            self.assertIn("current: generation 1", out)
            self.assertIn("cloudflare version v1 deployment d1", out)

    def test_status_reports_provider_failure_without_crash(self):
        out = deploy.format_cloudflare_status(
            NAME, DEFN, [], serving_error="boom"
        )
        self.assertIn("Cloudflare query failed: boom", out)

    def test_status_dry_run_does_not_query_provider(self):
        orig = deploy.cf_get_serving

        def boom(name, defn=None):
            raise AssertionError("dry-run contacted Cloudflare")

        deploy.cf_get_serving = boom
        try:
            registry = {NAME: DEFN}
            reg_path = None
            with tempfile.TemporaryDirectory() as state:
                reg_path = os.path.join(state, "registry.json")
                with open(reg_path, "w", encoding="utf-8") as fh:
                    json.dump(registry, fh)
                env = dict(os.environ, DEPLOY_STATE_DIR=state)
                proc = subprocess.run(
                    [sys.executable, DEPLOY_PATH, "--registry", reg_path,
                     NAME, "status", "--dry-run"],
                    capture_output=True,
                    text=True,
                    env=env,
                    timeout=30,
                )
            self.assertEqual(proc.returncode, 0, proc.stderr)
            self.assertIn("not queried (--dry-run)", proc.stdout)
        finally:
            deploy.cf_get_serving = orig


class ExecuteTests(unittest.TestCase):
    def patch(self, **fakes):
        orig = {k: getattr(deploy, k) for k in fakes}
        for k, v in fakes.items():
            setattr(deploy, k, v)
        return orig

    def restore(self, orig):
        for k, v in orig.items():
            setattr(deploy, k, v)

    def test_deploy_records_ids_only_after_success(self):
        with tempfile.TemporaryDirectory() as state:
            calls = []

            def fake_publish(name, defn):
                calls.append("publish")
                return {"version_id": "v3", "deployment_id": "d3"}

            def fake_verify(name, defn, version_id):
                calls.append(("verify", version_id))

            orig = self.patch(cf_publish=fake_publish,
                              cf_verify_bindings=fake_verify)
            try:
                code, _lines = deploy.execute_deploy_cloudflare(
                    NAME, DEFN, [], state=state, actor="tester"
                )
            finally:
                self.restore(orig)
            self.assertEqual(code, 0)
            self.assertEqual(calls, ["publish", ("verify", "v3")])
            entries = deploy.read_generations(NAME, state)
            self.assertEqual(len(entries), 1)
            self.assertEqual(entries[0]["cloudflare_version"], "v3")
            self.assertEqual(entries[0]["cloudflare_deployment"], "d3")
            self.assertEqual(entries[0]["action"], "deploy")

    def test_deploy_failure_records_failed_without_ids(self):
        with tempfile.TemporaryDirectory() as state:
            def fake_publish(name, defn):
                raise deploy.CloudflareError("publish exploded")

            orig = self.patch(cf_publish=fake_publish)
            try:
                code, lines = deploy.execute_deploy_cloudflare(
                    NAME, DEFN, [], state=state, actor="tester"
                )
            finally:
                self.restore(orig)
            self.assertEqual(code, 1)
            self.assertIn("FAILED", lines[0])
            entries = deploy.read_generations(NAME, state)
            self.assertEqual(len(entries), 1)
            self.assertEqual(entries[0]["action"], "deploy_failed")
            self.assertNotIn("cloudflare_version", entries[0])

    def test_deploy_missing_bindings_is_failure(self):
        with tempfile.TemporaryDirectory() as state:
            orig = self.patch(
                cf_publish=lambda n, d: {"version_id": "v4",
                                         "deployment_id": "d4"},
                cf_verify_bindings=self._missing_bindings,
            )
            try:
                code, lines = deploy.execute_deploy_cloudflare(
                    NAME, DEFN, [], state=state, actor="tester"
                )
            finally:
                self.restore(orig)
            self.assertEqual(code, 1)
            self.assertIn("lacks required bindings", lines[0])
            entries = deploy.read_generations(NAME, state)
            self.assertEqual(entries[0]["action"], "deploy_failed")

    def _missing_bindings(self, name, defn, version_id):
        raise deploy.CloudflareError(
            f"{name}: version {version_id} lacks required bindings: "
            + REQUIRED[0]
        )

    def test_rollback_reactivates_without_rebuild(self):
        with tempfile.TemporaryDirectory() as state:
            record(state, 1, cloudflare_version="v1", cloudflare_deployment="d1")
            record(state, 2, cloudflare_version="v2", cloudflare_deployment="d2")
            entries = deploy.read_generations(NAME, state)
            calls = []

            def fake_verify(name, defn, version_id):
                calls.append(("verify", version_id))

            def fake_activate(name, defn, version_id):
                calls.append(("activate", version_id))
                self.assertEqual(version_id, "v1")
                return {"version_id": "v1", "deployment_id": "d9"}

            orig = self.patch(cf_verify_bindings=fake_verify,
                              cf_activate_version=fake_activate)
            try:
                code, lines = deploy.execute_rollback_cloudflare(
                    NAME, DEFN, entries, state=state, actor="tester"
                )
            finally:
                self.restore(orig)
            self.assertEqual(code, 0)
            self.assertEqual(calls, [("verify", "v1"), ("activate", "v1")])
            self.assertIn("no rebuild, no re-upload", lines[0])
            current, _prev = deploy.current_previous(
                deploy.read_generations(NAME, state)
            )
            self.assertEqual(current["generation"], 1)

    def test_rollback_refuses_before_mutation_on_bindings(self):
        with tempfile.TemporaryDirectory() as state:
            record(state, 1, cloudflare_version="v1", cloudflare_deployment="d1")
            record(state, 2, cloudflare_version="v2", cloudflare_deployment="d2")
            entries = deploy.read_generations(NAME, state)
            activated = []

            def fake_activate(name, defn, version_id):
                activated.append(version_id)
                return {"version_id": version_id, "deployment_id": "dx"}

            orig = self.patch(
                cf_verify_bindings=self._missing_bindings,
                cf_activate_version=fake_activate,
            )
            try:
                code, lines = deploy.execute_rollback_cloudflare(
                    NAME, DEFN, entries, state=state, actor="tester"
                )
            finally:
                self.restore(orig)
            self.assertEqual(code, 1)
            self.assertIn("REFUSED", lines[0])
            # Refused before mutation: the provider was never asked to
            # re-activate.
            self.assertEqual(activated, [])


class CliDryRunTests(unittest.TestCase):
    def run_cli(self, state, reg_path, *argv):
        env = dict(os.environ, DEPLOY_STATE_DIR=state)
        return subprocess.run(
            [sys.executable, DEPLOY_PATH, "--registry", reg_path, *argv],
            capture_output=True,
            text=True,
            env=env,
            timeout=30,
        )

    def test_dry_run_plans_without_side_effects(self):
        with tempfile.TemporaryDirectory() as state:
            reg_path = os.path.join(state, "registry.json")
            with open(reg_path, "w", encoding="utf-8") as fh:
                json.dump({NAME: DEFN}, fh)
            proc = self.run_cli(state, reg_path, NAME, "--dry-run")
            self.assertEqual(proc.returncode, 0, proc.stderr)
            self.assertIn("publish", proc.stdout)
            self.assertIn("no changes made", proc.stdout)
            self.assertFalse(
                os.path.exists(os.path.join(state, "generations")),
                "dry-run must not create generation state",
            )

    def test_rollback_dry_run_plans_without_side_effects(self):
        with tempfile.TemporaryDirectory() as state:
            record(state, 1, cloudflare_version="v1", cloudflare_deployment="d1")
            record(state, 2, cloudflare_version="v2", cloudflare_deployment="d2")
            reg_path = os.path.join(state, "registry.json")
            with open(reg_path, "w", encoding="utf-8") as fh:
                json.dump({NAME: DEFN}, fh)
            before = list(deploy.read_generations(NAME, state))
            proc = self.run_cli(state, reg_path, NAME, "rollback", "--dry-run")
            self.assertEqual(proc.returncode, 0, proc.stderr)
            self.assertIn("no rebuild", proc.stdout)
            after = deploy.read_generations(NAME, state)
            self.assertEqual(before, after)


if __name__ == "__main__":
    unittest.main(argv=[sys.argv[0]])
