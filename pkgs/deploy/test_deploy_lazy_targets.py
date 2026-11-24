#!/usr/bin/env python3
"""Coverage for building a deploy target's package on demand (code#12738).

Oracle: issue #12738 ask 2. The registry names each target's store path
without depending on it, so building the CLI realizes no target package;
a real deploy builds its own target's package from the derivation that
$DEPLOY_TARGET_DRVS records, and nothing else.

run_cmd and _which are faked (no nix-store runs); the filesystem effects
(profile link, generation dirs, the drv map) are real, in temp dirs.

Usage: test_deploy_lazy_targets.py [path-to-deploy.py]
"""

import importlib.util
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

ARTIFACT = "/nix/store/00000000000000000000000000000000-svc-12738-missing"
DRV = "/nix/store/11111111111111111111111111111111-svc-12738.drv"


class LazyTargetTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.base = os.path.join(self.tmp.name, "profiles")
        self.calls = []
        self.produce = None
        self._real_run_cmd = deploy.run_cmd
        self._real_which = deploy._which
        self._real_env = os.environ.get("DEPLOY_TARGET_DRVS")

        def run(argv):
            self.calls.append(list(argv))
            if argv[:2] == ["nix-store", "--realise"] and self.produce:
                self.produce()
            return 0, "", ""

        deploy.run_cmd = run
        deploy._which = lambda tool: tool == "nix-store"
        self.drvs_path = os.path.join(self.tmp.name, "drvs.json")
        os.environ["DEPLOY_TARGET_DRVS"] = self.drvs_path

    def tearDown(self):
        deploy.run_cmd = self._real_run_cmd
        deploy._which = self._real_which
        if self._real_env is None:
            os.environ.pop("DEPLOY_TARGET_DRVS", None)
        else:
            os.environ["DEPLOY_TARGET_DRVS"] = self._real_env
        self.tmp.cleanup()

    def write_drvs(self, drvs):
        with open(self.drvs_path, "w", encoding="utf-8") as fh:
            json.dump(drvs, fh)

    def realise_calls(self):
        return [c for c in self.calls if c[:2] == ["nix-store", "--realise"]]

    def test_missing_artifact_is_built_from_its_recorded_drv(self):
        self.write_drvs({"svc": DRV, "other": DRV.replace("svc", "other")})
        with self.assertRaises(deploy.ActivationError) as ctx:
            deploy.install_generation("svc", ARTIFACT, 1, self.base)
        # The fake build produces nothing, so the deploy fails loudly
        # instead of linking a dangling profile.
        self.assertIn("did not produce", str(ctx.exception))
        self.assertEqual(self.realise_calls(), [["nix-store", "--realise", DRV]])

    def test_built_artifact_proceeds_to_the_profile(self):
        artifact = os.path.join(self.tmp.name, "svc-out")
        self.write_drvs({"svc": DRV})

        def produce():
            os.makedirs(artifact, exist_ok=True)

        self.produce = produce
        # ensure_artifact only acts on store paths; exercise it directly
        # with a store-shaped path whose existence check is redirected.
        real_exists = deploy.os.path.exists

        def exists(path):
            if path == ARTIFACT:
                return real_exists(artifact)
            return real_exists(path)

        deploy.os.path.exists = exists
        try:
            deploy.ensure_artifact("svc", ARTIFACT)
        finally:
            deploy.os.path.exists = real_exists
        self.assertEqual(self.realise_calls(), [["nix-store", "--realise", DRV]])

    def test_present_or_unmapped_artifacts_build_nothing(self):
        self.write_drvs({"svc": DRV})
        present = os.path.join(self.tmp.name, "present")
        os.makedirs(present)
        deploy.ensure_artifact("svc", present)  # not a store path
        deploy.ensure_artifact("unmapped", ARTIFACT)  # no recorded drv
        os.environ.pop("DEPLOY_TARGET_DRVS")
        deploy.ensure_artifact("svc", ARTIFACT)  # no map configured
        self.assertEqual(self.realise_calls(), [])

    def test_malformed_drv_maps_are_ignored(self):
        for drvs in ("not json", '["svc"]', '{"svc": "/tmp/evil.drv"}', '{"svc": 3}'):
            with open(self.drvs_path, "w", encoding="utf-8") as fh:
                fh.write(drvs)
            self.assertIsNone(deploy._target_drv("svc"), drvs)
        deploy.ensure_artifact("svc", ARTIFACT)
        self.assertEqual(self.realise_calls(), [])


if __name__ == "__main__":
    unittest.main(argv=[sys.argv[0]], verbosity=2)
