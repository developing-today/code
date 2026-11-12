#!/usr/bin/env python3
"""Coverage for the deploy slice-3 real activation path (issue #5766).

Oracle: design doc/2026-11-04_deployment-architecture-services-layer.md
§4.1 plan order (install generation into the profile, write the unit,
daemon-reload, restart only that unit, health check, flip back on any
failure; append to the generation log) and §4.6 (rollback repoints at
the previous distinct generation, never rebuilds).

All external effects are faked: run_cmd records its argv instead of
touching systemctl/nix-env, probe_health is scripted per test. The
filesystem effects (profile link, generation dirs, unit file,
generation log) are real, in temp dirs.

Usage: test_deploy_slice3.py [path-to-deploy.py]
"""

import importlib.util
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

DEFN = {
    "package": "/placeholder/artifact",
    "entryPoint": "bin/svc",
    "args": [],
    "environment": {},
    "secrets": [],
    "user": "user",
    "ports": [8080],
    "restart": "on-failure",
    "healthCheck": "http://127.0.0.1:8080/health",
    "targets": ["amd"],
    "class": "systemd",
}


class FakeEffects:
    def __init__(self, health_ok=True):
        self.commands = []
        self.health_ok = health_ok
        self._real_run_cmd = deploy.run_cmd
        self._real_probe = deploy.probe_health

    def __enter__(self):
        deploy.run_cmd = self._run
        deploy.probe_health = self._probe
        return self

    def __exit__(self, *exc):
        deploy.run_cmd = self._real_run_cmd
        deploy.probe_health = self._real_probe

    def _run(self, argv):
        self.commands.append(list(argv))
        return 0, "", ""

    def _probe(self, url, timeout=2.0):
        return (True, "HTTP 200") if self.health_ok else (False, "HTTP 500")

    def systemctl_steps(self):
        return [
            c[2] if c[1] == "--user" else c[1]
            for c in self.commands
            if c[0] == "systemctl"
        ]


class ActivationTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        root = self.tmp.name
        self.base = os.path.join(root, "profiles")
        self.state = os.path.join(root, "state")
        self.units = os.path.join(root, "units")
        self.art1 = os.path.join(root, "artifact1")
        self.art2 = os.path.join(root, "artifact2")
        for art in (self.art1, self.art2):
            os.makedirs(os.path.join(art, "bin"))

    def tearDown(self):
        self.tmp.cleanup()

    def defn(self, artifact):
        return dict(DEFN, package=artifact)

    def entries(self):
        return deploy.read_generations("svc", self.state)

    def current_artifact(self):
        link = deploy.profile_link("svc", self.base)
        return os.path.realpath(os.path.join(link, "current"))

    def do_deploy(self, artifact, fx):
        return deploy.execute_deploy(
            "svc",
            self.defn(artifact),
            self.entries(),
            base=self.base,
            state=self.state,
            scope="user",
            units=self.units,
            actor="test",
        )

    def test_deploy_success_follows_plan_order(self):
        with FakeEffects() as fx:
            code, lines = self.do_deploy(self.art1, fx)
        self.assertEqual(code, 0, lines)
        # profile install happened: link/current resolves to artifact
        self.assertEqual(self.current_artifact(), os.path.realpath(self.art1))
        # unit written, pointing through the profile current link
        with open(os.path.join(self.units, "svc.service")) as fh:
            unit = fh.read()
        self.assertIn("current/bin/svc", unit)
        # daemon-reload strictly before the single-unit restart
        steps = fx.systemctl_steps()
        self.assertEqual(steps, ["daemon-reload", "restart"])
        restart_argv = [c for c in fx.commands if "restart" in c][0]
        self.assertIn("svc.service", restart_argv)
        self.assertNotIn("other.service", restart_argv)
        # generation log folded: gen 1 current, no previous
        current, previous = deploy.current_previous(self.entries())
        self.assertEqual(current["generation"], 1)
        self.assertIsNone(previous)

    def test_store_path_artifact_registers_with_nix_env_first(self):
        store_art = "/nix/store/abc123-svc-1.0"
        with FakeEffects() as fx:
            code, _lines = self.do_deploy(store_art, fx)
        self.assertEqual(code, 0)
        nix = [c for c in fx.commands if c[0] == "nix-env"]
        self.assertEqual(len(nix), 1)
        self.assertIn("--set", nix[0])
        self.assertIn(store_art, nix[0])
        # nix registration precedes daemon-reload in the plan order
        self.assertLess(
            fx.commands.index(nix[0]),
            min(i for i, c in enumerate(fx.commands) if c[0] == "systemctl"),
        )

    def test_broken_generation_flips_back(self):
        with FakeEffects() as fx:
            self.assertEqual(self.do_deploy(self.art1, fx)[0], 0)
            self.assertEqual(self.do_deploy(self.art2, fx)[0], 0)
        # third deploy: same defn (artifact2 path) but health now fails
        with FakeEffects(health_ok=False) as fx:
            code, lines = self.do_deploy(self.art2, fx)
        self.assertEqual(code, 1)
        self.assertIn("flipped back to generation 2", lines[-1])
        # profile is back on generation 2's artifact, and the fold
        # still reports gen 2 current (the failure entry must not fold)
        self.assertEqual(self.current_artifact(), os.path.realpath(self.art2))
        current, _prev = deploy.current_previous(self.entries())
        self.assertEqual(current["generation"], 2)
        self.assertEqual(self.entries()[-1]["action"], "deploy_failed")
        # flip-back restarted the unit again after the failed restart
        self.assertEqual(fx.systemctl_steps().count("restart"), 2)

    def test_first_deploy_failure_stops_unit(self):
        with FakeEffects(health_ok=False) as fx:
            code, lines = self.do_deploy(self.art1, fx)
        self.assertEqual(code, 1)
        self.assertIn("stopped (no previous generation)", lines[-1])
        self.assertIn("stop", fx.systemctl_steps())
        current, _prev = deploy.current_previous(self.entries())
        self.assertIsNone(current)

    def test_flip_back_restart_failure_is_recorded_not_raised(self):
        # Regression (slice-3 laptop E2E): a broken unit restart-loops
        # into systemd's start limit and the flip-back restart itself
        # fails. The deploy must still exit 1 and append deploy_failed
        # — an unrecorded failure is the one outcome not allowed.
        with FakeEffects() as fx:
            self.assertEqual(self.do_deploy(self.art1, fx)[0], 0)
        restarts = {"n": 0}

        def flaky(argv):
            fx.commands.append(list(argv))
            if "restart" in argv:
                restarts["n"] += 1
                if restarts["n"] >= 2:  # the flip-back restart
                    return 1, "", "start limit hit"
            return 0, "", ""

        real_run = deploy.run_cmd
        real_probe = deploy.probe_health
        deploy.run_cmd = flaky
        deploy.probe_health = lambda url, timeout=2.0: (False, "HTTP 500")
        try:
            code, lines = deploy.execute_deploy(
                "svc",
                self.defn(self.art2),
                self.entries(),
                base=self.base,
                state=self.state,
                scope="user",
                units=self.units,
                actor="test",
            )
        finally:
            deploy.run_cmd = real_run
            deploy.probe_health = real_probe
        self.assertEqual(code, 1)
        self.assertIn("INCOMPLETE", lines[-1])
        self.assertEqual(self.entries()[-1]["action"], "deploy_failed")
        current, _prev = deploy.current_previous(self.entries())
        self.assertEqual(current["generation"], 1)
        # reset-failed is issued before the flip-back restart
        flat = [step for c in fx.commands for step in c]
        self.assertIn("reset-failed", flat)

    def test_rollback_repoints_and_restarts(self):
        with FakeEffects() as fx:
            self.assertEqual(self.do_deploy(self.art1, fx)[0], 0)
            self.assertEqual(self.do_deploy(self.art2, fx)[0], 0)
            code, lines = deploy.execute_rollback(
                "svc",
                self.defn(self.art2),
                self.entries(),
                base=self.base,
                state=self.state,
                scope="user",
                units=self.units,
                actor="test",
            )
        self.assertEqual(code, 0, lines)
        self.assertEqual(self.current_artifact(), os.path.realpath(self.art1))
        current, _prev = deploy.current_previous(self.entries())
        self.assertEqual(current["generation"], 1)
        self.assertEqual(self.entries()[-1]["action"], "rollback")

    def test_rollback_refused_with_one_generation(self):
        with FakeEffects() as fx:
            self.assertEqual(self.do_deploy(self.art1, fx)[0], 0)
            code, lines = deploy.execute_rollback(
                "svc",
                self.defn(self.art1),
                self.entries(),
                base=self.base,
                state=self.state,
                scope="user",
                units=self.units,
                actor="test",
            )
        self.assertEqual(code, 1)
        self.assertIn("only one generation", lines[0])


if __name__ == "__main__":
    unittest.main(argv=[sys.argv[0]])
