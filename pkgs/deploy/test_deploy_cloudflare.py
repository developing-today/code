#!/usr/bin/env python3
"""Coverage for the Cloudflare lifecycle slice (issue #5767).

Oracle: #5767's remaining acceptance, as queued — the generic deploy
CLI reports the version/deployment Cloudflare is actually serving
for cf-container-hello (not a version inferred from the local
generation log), deploys the flake-built package and appends the
provider version/deployment ids to the generation log only after
success, and rolls back by pointing the deployment at the previously
recorded version — no rebuild, no re-upload — refusing before any
mutation when that version lacks a required secret binding.

Every provider effect is faked: FakeProvider stands in for the
Cloudflare boundary, so no test contacts Cloudflare or deploys live.
The generation log's filesystem effects are real, in temp dirs.

Usage: test_deploy_cloudflare.py [path-to-deploy.py]
"""

import contextlib
import importlib.util
import io
import json
import os
import sys
import tempfile
import unittest

DEPLOY_PATH = (
    sys.argv[1]
    if len(sys.argv) > 1
    else os.path.join(os.path.dirname(os.path.abspath(__file__)), "deploy.py")
)
spec = importlib.util.spec_from_file_location("deploy", DEPLOY_PATH)
deploy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(deploy)

NAME = "cf-container-hello"
DEFN = {
    "package": "/placeholder/cf-container-hello-deploy",
    "entryPoint": "bin/cf-container-hello-deploy",
    "args": [],
    "environment": {},
    "secrets": [],
    "user": "user",
    "ports": [],
    "restart": "no",
    "healthCheck": None,
    "targets": ["cloudflare"],
    "class": "cloudflare",
}
SECRET_DEFN = dict(DEFN, secrets=["ACCESS_TOKEN"])


class FakeProvider:
    """A scripted Cloudflare boundary. Records every call in order;
    `serving` is whatever the test says Cloudflare reports, fully
    independent of the local generation log."""

    def __init__(
        self,
        serving=None,
        published=None,
        bindings=None,
        fail_on=(),
    ):
        self.calls = []
        self._serving = serving or {
            "version_id": "v-live",
            "deployment_id": "dep-live",
        }
        self._published = published or {
            "version_id": "v-new",
            "deployment_id": "dep-new",
        }
        self._bindings = bindings if bindings is not None else ["ACCESS_TOKEN"]
        self._fail_on = set(fail_on)

    def _step(self, verb, *args):
        self.calls.append((verb, *args))
        if verb in self._fail_on:
            raise deploy.CloudflareError(f"fake provider: {verb} failed")

    def serving(self):
        self._step("serving")
        return dict(self._serving)

    def publish(self):
        self._step("publish")
        return dict(self._published)

    def version_secret_bindings(self, version_id):
        self._step("version_secret_bindings", version_id)
        return list(self._bindings)

    def activate(self, version_id):
        self._step("activate", version_id)
        self._serving = {
            "version_id": version_id,
            "deployment_id": "dep-rollback",
        }
        return {"deployment_id": "dep-rollback"}


def record_cf(state, generation, version_id, deployment_id, action="deploy"):
    return deploy.record_generation(
        NAME,
        artifact=DEFN["package"],
        source_rev=f"rev{generation}",
        actor="tester",
        action=action,
        generation=generation,
        result="activated" if action == "deploy" else "rolled-back",
        explicit_state=state,
        extra={"cf_version_id": version_id, "cf_deployment_id": deployment_id},
    )


class StatusTests(unittest.TestCase):
    def test_status_reports_serving_version_from_provider_not_log(self):
        # The oracle: the log is empty, yet status still names the
        # serving version — because it asked Cloudflare.
        with tempfile.TemporaryDirectory() as state:
            provider = FakeProvider()
            out = deploy.format_cloudflare_status(NAME, DEFN, [], provider)
            self.assertIn("serving: version v-live (deployment dep-live)", out)
            self.assertIn("recorded current: none recorded", out)
            self.assertEqual(provider.calls, [("serving",)])

    def test_status_prefers_provider_over_stale_log(self):
        with tempfile.TemporaryDirectory() as state:
            record_cf(state, 1, "v-old", "dep-old")
            entries = deploy.read_generations(NAME, state)
            out = deploy.format_cloudflare_status(
                NAME, DEFN, entries, FakeProvider()
            )
            self.assertIn("serving: version v-live", out)
            self.assertIn("recorded current: generation 1", out)
            self.assertIn("cloudflare version v-old", out)

    def test_status_provider_failure_is_reported_not_faked(self):
        provider = FakeProvider(fail_on={"serving"})
        out = deploy.format_cloudflare_status(NAME, DEFN, [], provider)
        self.assertIn("serving: unavailable", out)
        self.assertNotIn("v-live", out)


class DeployTests(unittest.TestCase):
    def test_deploy_publishes_then_records_provider_ids(self):
        with tempfile.TemporaryDirectory() as state:
            provider = FakeProvider()
            code, lines = deploy.execute_deploy_cloudflare(
                NAME, DEFN, [], state=state, actor="tester", provider=provider
            )
            self.assertEqual(code, 0)
            self.assertEqual(provider.calls, [("publish",)])
            entries = deploy.read_generations(NAME, state)
            self.assertEqual(len(entries), 1)
            entry = entries[0]
            self.assertEqual(entry["action"], "deploy")
            self.assertEqual(entry["result"], "activated")
            self.assertEqual(entry["cf_version_id"], "v-new")
            self.assertEqual(entry["cf_deployment_id"], "dep-new")
            self.assertIn("cloudflare version v-new", lines[0])

    def test_failed_publish_records_no_activation(self):
        with tempfile.TemporaryDirectory() as state:
            provider = FakeProvider(fail_on={"publish"})
            code, _lines = deploy.execute_deploy_cloudflare(
                NAME, DEFN, [], state=state, actor="tester", provider=provider
            )
            self.assertEqual(code, 1)
            entries = deploy.read_generations(NAME, state)
            self.assertEqual(len(entries), 1)
            self.assertEqual(entries[0]["action"], "deploy_failed")
            self.assertNotIn("cf_version_id", entries[0])
            self.assertEqual(deploy.current_previous(entries), (None, None))


class RollbackTests(unittest.TestCase):
    def _two_generations(self, state):
        record_cf(state, 1, "v-one", "dep-one")
        record_cf(state, 2, "v-two", "dep-two")
        return deploy.read_generations(NAME, state)

    def test_rollback_repoints_previous_version_without_rebuild(self):
        with tempfile.TemporaryDirectory() as state:
            entries = self._two_generations(state)
            provider = FakeProvider(
                serving={"version_id": "v-two", "deployment_id": "dep-two"}
            )
            code, lines = deploy.execute_rollback_cloudflare(
                NAME,
                SECRET_DEFN,
                entries,
                state=state,
                actor="tester",
                provider=provider,
            )
            self.assertEqual(code, 0)
            # Bindings verified before the mutation; publish (the
            # rebuild/re-upload verb) is never called.
            self.assertEqual(
                provider.calls,
                [
                    ("version_secret_bindings", "v-one"),
                    ("activate", "v-one"),
                    ("serving",),
                ],
            )
            entries = deploy.read_generations(NAME, state)
            last = entries[-1]
            self.assertEqual(last["action"], "rollback")
            self.assertEqual(last["generation"], 1)
            self.assertEqual(last["cf_version_id"], "v-one")
            self.assertEqual(last["cf_deployment_id"], "dep-rollback")
            current, _previous = deploy.current_previous(entries)
            self.assertEqual(current["generation"], 1)
            self.assertIn("rolled back to generation 1", lines[0])

    def test_rollback_refuses_before_mutation_when_binding_missing(self):
        with tempfile.TemporaryDirectory() as state:
            entries = self._two_generations(state)
            provider = FakeProvider(bindings=[])
            code, lines = deploy.execute_rollback_cloudflare(
                NAME,
                SECRET_DEFN,
                entries,
                state=state,
                actor="tester",
                provider=provider,
            )
            self.assertEqual(code, 1)
            self.assertIn("REFUSED", lines[0])
            self.assertIn("ACCESS_TOKEN", lines[0])
            # The only provider contact was the read-only check.
            self.assertEqual(
                provider.calls, [("version_secret_bindings", "v-one")]
            )
            self.assertEqual(len(deploy.read_generations(NAME, state)), 2)

    def test_rollback_refuses_when_bindings_unverifiable(self):
        with tempfile.TemporaryDirectory() as state:
            entries = self._two_generations(state)
            provider = FakeProvider(fail_on={"version_secret_bindings"})
            code, lines = deploy.execute_rollback_cloudflare(
                NAME,
                SECRET_DEFN,
                entries,
                state=state,
                actor="tester",
                provider=provider,
            )
            self.assertEqual(code, 1)
            self.assertIn("REFUSED", lines[0])
            self.assertNotIn(("activate", "v-one"), provider.calls)
            self.assertEqual(len(deploy.read_generations(NAME, state)), 2)

    def test_rollback_refuses_generation_without_recorded_version(self):
        with tempfile.TemporaryDirectory() as state:
            deploy.record_generation(
                NAME,
                artifact=DEFN["package"],
                source_rev="rev1",
                actor="tester",
                generation=1,
                explicit_state=state,
            )
            deploy.record_generation(
                NAME,
                artifact=DEFN["package"],
                source_rev="rev2",
                actor="tester",
                generation=2,
                explicit_state=state,
            )
            entries = deploy.read_generations(NAME, state)
            provider = FakeProvider()
            code, lines = deploy.execute_rollback_cloudflare(
                NAME, DEFN, entries, state=state, actor="tester", provider=provider
            )
            self.assertEqual(code, 1)
            self.assertIn("records no Cloudflare version id", lines[0])
            self.assertEqual(provider.calls, [])


class PlanTests(unittest.TestCase):
    def test_deploy_plan_is_cloudflare_shaped(self):
        steps = deploy.plan_deploy(NAME, DEFN, [])
        self.assertIsNotNone(steps)
        text = "\n".join(steps)
        self.assertIn("publish the flake-built package", text)
        self.assertIn("version id and deployment id", text)
        self.assertNotIn("nix-env", text)
        self.assertNotIn("systemctl", text)

    def test_rollback_plan_verifies_bindings_and_never_rebuilds(self):
        with tempfile.TemporaryDirectory() as state:
            record_cf(state, 1, "v-one", "dep-one")
            record_cf(state, 2, "v-two", "dep-two")
            steps, error = deploy.plan_rollback(
                NAME, SECRET_DEFN, deploy.read_generations(NAME, state)
            )
            self.assertIsNone(error)
            text = "\n".join(steps)
            self.assertLess(
                text.index("verify version v-one"), text.index("create a deployment")
            )
            self.assertIn("no rebuild, no re-upload", text)


class ProviderBoundaryTests(unittest.TestCase):
    def test_app_provider_parses_last_json_line(self):
        real_run_cmd = deploy.run_cmd
        seen = []

        def fake_run_cmd(argv):
            seen.append(argv)
            return 0, 'progress line\n{"ok": true, "version_id": "v9", "deployment_id": "d9"}\n', ""

        deploy.run_cmd = fake_run_cmd
        try:
            provider = deploy.DeployAppCloudflareProvider(NAME, DEFN)
            self.assertEqual(
                provider.serving(), {"version_id": "v9", "deployment_id": "d9"}
            )
            self.assertEqual(
                seen,
                [
                    [
                        "/placeholder/cf-container-hello-deploy/bin/cf-container-hello-deploy",
                        "status",
                    ]
                ],
            )
        finally:
            deploy.run_cmd = real_run_cmd

    def test_app_provider_surfaces_app_failure(self):
        real_run_cmd = deploy.run_cmd
        deploy.run_cmd = lambda argv: (0, '{"ok": false, "error": "nope"}\n', "")
        try:
            provider = deploy.DeployAppCloudflareProvider(NAME, DEFN)
            with self.assertRaises(deploy.CloudflareError) as ctx:
                provider.publish()
            self.assertIn("nope", str(ctx.exception))
        finally:
            deploy.run_cmd = real_run_cmd

    def test_app_provider_nonzero_exit_is_a_failed_step(self):
        real_run_cmd = deploy.run_cmd
        deploy.run_cmd = lambda argv: (3, "", "app exploded")
        try:
            provider = deploy.DeployAppCloudflareProvider(NAME, DEFN)
            with self.assertRaises(deploy.CloudflareError) as ctx:
                provider.activate("v1")
            self.assertIn("exit 3", str(ctx.exception))
        finally:
            deploy.run_cmd = real_run_cmd


class MainDispatchTests(unittest.TestCase):
    """The CLI surface itself: dry runs plan without touching the
    provider, and status answers through the (faked) provider."""

    def setUp(self):
        self._real_provider = deploy.cloudflare_provider
        self.provider = FakeProvider()
        deploy.cloudflare_provider = lambda name, defn: self.provider

    def tearDown(self):
        deploy.cloudflare_provider = self._real_provider

    def _registry(self, tmp):
        path = os.path.join(tmp, "registry.json")
        with open(path, "w", encoding="utf-8") as fh:
            json.dump({NAME: DEFN}, fh)
        return path

    def _main(self, argv):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = deploy.main(argv)
        return code, out.getvalue()

    def test_main_deploy_dry_run_plans_without_provider_contact(self):
        with tempfile.TemporaryDirectory() as tmp:
            registry = self._registry(tmp)
            code, out = self._main(
                [
                    "--registry",
                    registry,
                    "--state-dir",
                    os.path.join(tmp, "state"),
                    NAME,
                    "--dry-run",
                ]
            )
            self.assertEqual(code, 0)
            self.assertIn("no changes made", out)
            self.assertIn("publish the flake-built package", out)
            self.assertEqual(self.provider.calls, [])

    def test_main_rollback_dry_run_plans_without_provider_contact(self):
        with tempfile.TemporaryDirectory() as tmp:
            registry = self._registry(tmp)
            state = os.path.join(tmp, "state")
            record_cf(state, 1, "v-one", "dep-one")
            record_cf(state, 2, "v-two", "dep-two")
            code, out = self._main(
                [
                    "--registry",
                    registry,
                    "--state-dir",
                    state,
                    NAME,
                    "rollback",
                    "--dry-run",
                ]
            )
            self.assertEqual(code, 0)
            self.assertIn("no rebuild, no re-upload", out)
            self.assertEqual(self.provider.calls, [])

    def test_main_status_reports_provider_serving_version(self):
        with tempfile.TemporaryDirectory() as tmp:
            registry = self._registry(tmp)
            code, out = self._main(
                [
                    "--registry",
                    registry,
                    "--state-dir",
                    os.path.join(tmp, "state"),
                    NAME,
                    "status",
                ]
            )
            self.assertEqual(code, 0)
            self.assertIn("serving: version v-live (deployment dep-live)", out)
            self.assertEqual(self.provider.calls, [("serving",)])


if __name__ == "__main__":
    unittest.main(argv=[sys.argv[0]])
