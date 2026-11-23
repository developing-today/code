#!/usr/bin/env python3
"""Coverage for the deploy slice-4 sandbox path (issue #5770, M3).

Oracle: design doc/2026-11-04_deployment-architecture-services-layer.md
§4.4 — a sandbox target's generation lives in the home profile, its
rendered units are stored with the generation, the sandbox manifest
lists the activated targets, and the generic restorer links each
current generation's units into the unit dir, daemon-reloads, and
enables the target's enable unit (timer when one was rendered).
§4.6 as in slices 2-3: rollback repoints at the previous distinct
generation, never rebuilds, and refuses with one generation.

The `timer` field's validation oracle is the renderer's contract:
exactly {onCalendar, persistent, randomizedDelaySec}, a single-line
calendar spec (it is rendered verbatim into a unit file), a real
bool, a non-negative int.

All external effects are faked: run_cmd records its argv instead of
touching systemctl/nix-env. Filesystem effects (profile link,
generation dirs, rendered units, manifest, generation log, unit
links) are real, in temp dirs. restore.py is exercised the same way.

Usage: test_deploy_slice4.py [path-to-deploy.py] [path-to-restore.py]
"""

import importlib.util
import os
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
DEPLOY_PATH = sys.argv[1] if len(sys.argv) > 1 else os.path.join(HERE, "deploy.py")
RESTORE_PATH = sys.argv[2] if len(sys.argv) > 2 else os.path.join(HERE, "restore.py")


def load_module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


deploy = load_module("deploy", DEPLOY_PATH)
restore = load_module("deploy_restore", RESTORE_PATH)

TIMER_DEFN = {
    "package": "/placeholder/artifact",
    "entryPoint": "bin/quota-recorder",
    "args": [],
    "environment": {"HOME": "/home/hatch"},
    "secrets": ["turso.fleet-platform-rw-token"],
    "user": "root",
    "ports": [],
    "restart": "no",
    "healthCheck": None,
    "targets": ["sandbox"],
    "class": "sandbox",
    "timer": {
        "onCalendar": "*:0/5",
        "persistent": True,
        "randomizedDelaySec": 20,
    },
}

DAEMON_DEFN = {
    "package": "/placeholder/artifact",
    "entryPoint": "bin/svc",
    "args": ["serve"],
    "environment": {},
    "secrets": [],
    "user": "root",
    "ports": [3130],
    "restart": "on-failure",
    "healthCheck": None,
    "targets": ["sandbox"],
    "class": "sandbox",
}


class FakeEffects:
    """Records run_cmd argv; can be told to fail the next N enables."""

    def __init__(self, fail_enables=0):
        self.commands = []
        self.fail_enables = fail_enables
        self._real_run_cmd = deploy.run_cmd

    def __enter__(self):
        deploy.run_cmd = self._run
        return self

    def __exit__(self, *exc):
        deploy.run_cmd = self._real_run_cmd

    def _run(self, argv):
        self.commands.append(list(argv))
        if "enable" in argv and self.fail_enables > 0:
            self.fail_enables -= 1
            return 1, "", "enable failed (fake)"
        return 0, "", ""

    def systemctl_steps(self):
        return [c[1] for c in self.commands if c[0] == "systemctl"]


class TimerValidationTests(unittest.TestCase):
    def registry(self, defn):
        return {"quota-recorder": defn}

    def test_valid_timer_definition_validates(self):
        self.assertEqual(deploy.validate(self.registry(TIMER_DEFN)), [])

    def test_definition_without_timer_still_validates(self):
        defn = {k: v for k, v in TIMER_DEFN.items() if k != "timer"}
        self.assertEqual(deploy.validate(self.registry(defn)), [])

    def test_timer_must_be_an_object(self):
        errors = deploy.validate(self.registry(dict(TIMER_DEFN, timer="*:0/5")))
        self.assertTrue(any("timer must be an object" in e for e in errors), errors)

    def test_timer_missing_and_unknown_fields(self):
        bad = dict(
            TIMER_DEFN,
            timer={"onCalendar": "*:0/5", "weekly": True},
        )
        errors = deploy.validate(self.registry(bad))
        self.assertTrue(any("timer missing field(s)" in e for e in errors), errors)
        self.assertTrue(any("timer unknown field(s)" in e for e in errors), errors)

    def test_timer_calendar_must_be_single_line(self):
        bad = dict(
            TIMER_DEFN,
            timer={
                "onCalendar": "*:0/5\nOnBootSec=1",
                "persistent": True,
                "randomizedDelaySec": 0,
            },
        )
        errors = deploy.validate(self.registry(bad))
        self.assertTrue(any("timer.onCalendar" in e for e in errors), errors)

    def test_timer_field_types(self):
        bad = dict(
            TIMER_DEFN,
            timer={
                "onCalendar": "*:0/5",
                "persistent": "yes",
                "randomizedDelaySec": True,
            },
        )
        errors = deploy.validate(self.registry(bad))
        self.assertTrue(any("timer.persistent" in e for e in errors), errors)
        self.assertTrue(any("timer.randomizedDelaySec" in e for e in errors), errors)
        bad2 = dict(
            TIMER_DEFN,
            timer={
                "onCalendar": "*:0/5",
                "persistent": True,
                "randomizedDelaySec": -1,
            },
        )
        errors = deploy.validate(self.registry(bad2))
        self.assertTrue(any("timer.randomizedDelaySec" in e for e in errors), errors)


class RenderTests(unittest.TestCase):
    def test_timer_target_renders_oneshot_plus_timer(self):
        units = deploy.render_sandbox_units("quota-recorder", TIMER_DEFN, "/base")
        self.assertEqual(
            sorted(units), ["quota-recorder.service", "quota-recorder.timer"]
        )
        service = units["quota-recorder.service"]
        self.assertIn("Type=oneshot", service)
        self.assertIn(
            "ExecStart=/base/quota-recorder/current/bin/quota-recorder", service
        )
        self.assertIn("Environment=HOME=/home/hatch", service)
        self.assertIn("User=root", service)
        self.assertNotIn("[Install]", service)
        timer = units["quota-recorder.timer"]
        self.assertIn("OnCalendar=*:0/5", timer)
        self.assertIn("Persistent=true", timer)
        self.assertIn("RandomizedDelaySec=20", timer)
        self.assertIn("Unit=quota-recorder.service", timer)
        self.assertIn("WantedBy=timers.target", timer)

    def test_daemon_target_renders_installable_service(self):
        units = deploy.render_sandbox_units("svc", DAEMON_DEFN, "/base")
        self.assertEqual(sorted(units), ["svc.service"])
        service = units["svc.service"]
        self.assertIn("Restart=on-failure", service)
        self.assertIn("WantedBy=multi-user.target", service)
        self.assertNotIn("Type=oneshot", service)


class SandboxCase(unittest.TestCase):
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
        return dict(TIMER_DEFN, package=artifact)

    def entries(self):
        return deploy.read_generations("quota-recorder", self.state)

    def do_deploy(self, artifact):
        return deploy.execute_deploy_sandbox(
            "quota-recorder",
            self.defn(artifact),
            self.entries(),
            base=self.base,
            state=self.state,
            units_dir=self.units,
            actor="test",
        )

    def current_artifact(self):
        link = deploy.profile_link("quota-recorder", self.base)
        return os.path.realpath(os.path.join(link, "current"))


class PlanTests(SandboxCase):
    def test_sandbox_deploy_plan_names_manifest_and_restore(self):
        steps = deploy.plan_deploy(
            "quota-recorder", self.defn(self.art1), [], base=self.base
        )
        text = "\n".join(steps)
        self.assertIn("sandbox manifest", text)
        self.assertIn("enable --now quota-recorder.timer", text)
        self.assertIn("restore step", text)

    def test_rollback_plan_refuses_a_single_generation(self):
        with FakeEffects():
            self.assertEqual(self.do_deploy(self.art1)[0], 0)
        _steps, error = deploy.plan_rollback(
            "quota-recorder", self.defn(self.art1), self.entries(), base=self.base
        )
        self.assertIn("only one generation recorded", error)


class ActivationTests(SandboxCase):
    def test_deploy_success_links_units_and_records_manifest(self):
        with FakeEffects() as fx:
            code, lines = self.do_deploy(self.art1)
        self.assertEqual(code, 0, lines)
        self.assertEqual(self.current_artifact(), os.path.realpath(self.art1))
        # units rendered into the generation and linked into the unit dir
        for filename in ("quota-recorder.service", "quota-recorder.timer"):
            dest = os.path.join(self.units, filename)
            self.assertTrue(os.path.islink(dest), filename)
            self.assertTrue(os.path.exists(dest), filename)
        # manifest records the profile LINK, not a generation dir
        targets = deploy.read_sandbox_manifest(self.state)
        entry = targets["quota-recorder"]
        self.assertEqual(
            entry["profile"], deploy.profile_link("quota-recorder", self.base)
        )
        self.assertEqual(
            entry["units"], ["quota-recorder.service", "quota-recorder.timer"]
        )
        # systemctl order: daemon-reload, enable --now of the timer,
        # then the is-active verification
        self.assertEqual(fx.systemctl_steps(), ["daemon-reload", "enable", "is-active"])
        enable_argv = [c for c in fx.commands if "enable" in c][0]
        self.assertIn("quota-recorder.timer", enable_argv)
        current, previous = deploy.current_previous(self.entries())
        self.assertEqual(current["generation"], 1)
        self.assertIsNone(previous)

    def test_second_deploy_then_rollback_repoints_and_relinks(self):
        with FakeEffects():
            self.assertEqual(self.do_deploy(self.art1)[0], 0)
            self.assertEqual(self.do_deploy(self.art2)[0], 0)
        self.assertEqual(self.current_artifact(), os.path.realpath(self.art2))
        with FakeEffects() as fx:
            code, lines = deploy.execute_rollback_sandbox(
                "quota-recorder",
                self.defn(self.art2),
                self.entries(),
                base=self.base,
                state=self.state,
                units_dir=self.units,
                actor="test",
            )
        self.assertEqual(code, 0, lines)
        self.assertEqual(self.current_artifact(), os.path.realpath(self.art1))
        # the unit links resolve into generation 1 again
        dest = os.path.join(self.units, "quota-recorder.service")
        self.assertIn(
            os.path.join(".generations", "quota-recorder", "1"),
            os.path.realpath(dest),
        )
        current, previous = deploy.current_previous(self.entries())
        self.assertEqual(current["generation"], 1)
        self.assertEqual(previous["generation"], 2)
        self.assertIn("enable", fx.systemctl_steps())

    def test_rollback_with_one_generation_refuses(self):
        with FakeEffects():
            self.assertEqual(self.do_deploy(self.art1)[0], 0)
        with FakeEffects():
            code, lines = deploy.execute_rollback_sandbox(
                "quota-recorder",
                self.defn(self.art1),
                self.entries(),
                base=self.base,
                state=self.state,
                units_dir=self.units,
                actor="test",
            )
        self.assertEqual(code, 1)
        self.assertIn("only one generation recorded", lines[0])

    def test_failed_deploy_flips_back_to_previous_generation(self):
        with FakeEffects():
            self.assertEqual(self.do_deploy(self.art1)[0], 0)
        with FakeEffects(fail_enables=1) as fx:
            code, lines = self.do_deploy(self.art2)
        self.assertEqual(code, 1)
        self.assertIn("flipped back to generation 1", lines[-1])
        self.assertEqual(self.current_artifact(), os.path.realpath(self.art1))
        current, _prev = deploy.current_previous(self.entries())
        self.assertEqual(current["generation"], 1)
        self.assertEqual(self.entries()[-1]["action"], "deploy_failed")
        # flip-back re-enabled the timer after the failed enable
        self.assertEqual(fx.systemctl_steps().count("enable"), 2)

    def test_first_deploy_failure_removes_units(self):
        with FakeEffects(fail_enables=1):
            code, lines = self.do_deploy(self.art1)
        self.assertEqual(code, 1)
        self.assertIn("units removed (no previous generation)", lines[-1])
        self.assertFalse(
            os.path.exists(os.path.join(self.units, "quota-recorder.timer"))
        )
        self.assertFalse(
            os.path.islink(deploy.profile_link("quota-recorder", self.base))
        )
        current, _prev = deploy.current_previous(self.entries())
        self.assertIsNone(current)


class RestoreTests(SandboxCase):
    def test_restore_relinks_after_units_wiped(self):
        with FakeEffects():
            self.assertEqual(self.do_deploy(self.art1)[0], 0)
        # simulate a recycle: /etc copies gone
        for filename in os.listdir(self.units):
            os.unlink(os.path.join(self.units, filename))
        with FakeEffects() as fx:
            code, lines = deploy.restore_sandbox(
                base=self.base, state=self.state, units_dir=self.units
            )
        self.assertEqual(code, 0, lines)
        self.assertTrue(
            os.path.islink(os.path.join(self.units, "quota-recorder.timer"))
        )
        self.assertIn("enable", fx.systemctl_steps())

    def test_restore_empty_manifest_is_a_noop(self):
        code, lines = deploy.restore_sandbox(
            base=self.base, state=self.state, units_dir=self.units
        )
        self.assertEqual(code, 0)
        self.assertIn("nothing to do", lines[0])

    def test_restore_reports_a_missing_generation_and_continues(self):
        with FakeEffects():
            self.assertEqual(self.do_deploy(self.art1)[0], 0)
        targets = deploy.read_sandbox_manifest(self.state)
        targets["ghost"] = {
            "profile": os.path.join(self.base, "ghost"),
            "units": ["ghost.service"],
        }
        import json

        with open(
            deploy.sandbox_manifest_path(self.state), "w", encoding="utf-8"
        ) as fh:
            json.dump({"targets": targets}, fh)
        with FakeEffects():
            code, lines = deploy.restore_sandbox(
                base=self.base, state=self.state, units_dir=self.units
            )
        self.assertEqual(code, 1)
        self.assertTrue(any("ghost: FAILED" in line for line in lines), lines)
        self.assertTrue(any("quota-recorder: linked" in line for line in lines), lines)

    def test_restore_verb_via_main(self):
        with FakeEffects():
            self.assertEqual(self.do_deploy(self.art1)[0], 0)
        for filename in os.listdir(self.units):
            os.unlink(os.path.join(self.units, filename))
        with FakeEffects():
            code = deploy.main(
                [
                    "--state-dir",
                    self.state,
                    "--unit-dir",
                    self.units,
                    "restore",
                ]
            )
        self.assertEqual(code, 0)
        self.assertTrue(
            os.path.islink(os.path.join(self.units, "quota-recorder.timer"))
        )


class RestoreScriptTests(SandboxCase):
    def test_parse_args_is_strict(self):
        options, error = restore.parse_args(["--state-dir", "/x"])
        self.assertIsNone(error)
        self.assertEqual(options["--state-dir"], "/x")
        for argv in (
            ["--bogus"],
            ["positional"],
            ["--state-dir"],
            ["--state-dir", "/x", "--state-dir", "/y"],
            ["--target"],
        ):
            _options, error = restore.parse_args(argv)
            self.assertIsNotNone(error, argv)

    def test_main_restores_from_manifest(self):
        with FakeEffects():
            self.assertEqual(self.do_deploy(self.art1)[0], 0)
        for filename in os.listdir(self.units):
            os.unlink(os.path.join(self.units, filename))
        commands = []
        real_run_cmd = restore.run_cmd
        restore.run_cmd = lambda argv: (commands.append(list(argv)), (0, "", ""))[1]
        try:
            code = restore.main(["--state-dir", self.state, "--unit-dir", self.units])
        finally:
            restore.run_cmd = real_run_cmd
        self.assertEqual(code, 0)
        self.assertTrue(
            os.path.islink(os.path.join(self.units, "quota-recorder.timer"))
        )
        steps = [c[1] for c in commands if c[0] == "systemctl"]
        self.assertEqual(steps, ["daemon-reload", "enable", "is-active"])

    def test_main_without_manifest_is_a_noop(self):
        code = restore.main(["--state-dir", self.state, "--unit-dir", self.units])
        self.assertEqual(code, 0)


if __name__ == "__main__":
    unittest.main(argv=[sys.argv[0]])
