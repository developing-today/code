#!/usr/bin/env python3
"""Coverage for the deploy slice-2 invariants (issue #5766).

Oracle: doc/2026-11-04_deployment-architecture-services-layer.md §4.1,
§4.2 and §4.6, plus the issue text. The invariants under test:

- A rendered systemd unit (or launchd plist) points at the target's
  per-service nix profile `current` link, never at a bare package
  path, so a generation flip is a profile repoint and the rendered
  file is identical for every generation of the same definition.
- The generation log is append-only; current/previous generations are
  the fold of its activation events, and rollback selects the
  previous DISTINCT generation — repointing, never rebuilding — and
  is refused when fewer than two distinct generations were current.
- `deploy <target> --dry-run` prints the §4.1 plan in order (build,
  profile install, unit write, reload, restart only that unit,
  health check, flip-back, log append) and creates no state.
- Status reports the folded current/previous generations and the
  health verdict for the target's healthCheck URL (or that no check
  is defined).

Stdlib-only, deterministic. CLI cases run deploy.py as a subprocess
against a fixture registry in a temp state dir.

Usage: test_deploy_slice2.py [path-to-deploy.py]
"""

import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import unittest

DEPLOY_PATH = sys.argv[1] if len(sys.argv) > 1 else os.path.join(
    os.path.dirname(os.path.abspath(__file__)), "deploy.py"
)

spec = importlib.util.spec_from_file_location("deploy", DEPLOY_PATH)
deploy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(deploy)

DEFN = {
    "package": "/nix/store/aaa-svc-1.0",
    "entryPoint": "bin/svc",
    "args": ["--port", "8080", "--greeting", "hello world"],
    "environment": {"MODE": "prod", "PCT": "100%"},
    "secrets": ["svc.api-key"],
    "user": "svcuser",
    "ports": [8080],
    "restart": "on-failure",
    "healthCheck": "http://127.0.0.1:8080/health",
    "targets": ["laptop"],
    "class": "systemd",
}
BASE = "/profiles"


def record(state, gen, artifact, action="deploy", result="activated"):
    return deploy.record_generation(
        "svc",
        artifact=artifact,
        source_rev=f"rev{gen}",
        actor="tester",
        action=action,
        generation=gen,
        result=result,
        at=f"2026-11-08T00:0{gen}:00Z",
        explicit_state=state,
    )


class RenderTests(unittest.TestCase):
    def test_unit_points_at_profile_current_link(self):
        unit = deploy.render_unit("svc", DEFN, base=BASE)
        self.assertIn(
            "ExecStart=/profiles/svc/current/bin/svc --port 8080 "
            '--greeting "hello world"',
            unit,
        )
        self.assertNotIn("/nix/store/aaa-svc-1.0/bin/svc", unit)
        self.assertIn("User=svcuser", unit)
        self.assertIn("Restart=on-failure", unit)

    def test_unit_environment_quoting_escapes_percent(self):
        unit = deploy.render_unit("svc", DEFN, base=BASE)
        self.assertIn('Environment="PCT=100%%"', unit)
        self.assertIn("Environment=MODE=prod", unit)
        self.assertIn("# secrets resolved by name at activation time: svc.api-key", unit)

    def test_unit_render_is_generation_independent(self):
        # The flip-is-a-repoint invariant: the rendered unit cannot
        # depend on which generation is current.
        self.assertEqual(
            deploy.render_unit("svc", DEFN, base=BASE),
            deploy.render_unit("svc", dict(DEFN, package="/nix/store/zzz-svc-2.0"), base=BASE),
        )

    def test_plist_stub_points_at_profile_current_link(self):
        plist = deploy.render_plist("svc", DEFN, base=BASE)
        self.assertIn("<string>com.developing-today.deploy.svc</string>", plist)
        self.assertIn("<string>/profiles/svc/current/bin/svc</string>", plist)
        self.assertIn("<true/>", plist)  # KeepAlive for restart != "no"
        no_restart = deploy.render_plist("svc", dict(DEFN, restart="no"), base=BASE)
        self.assertIn("<false/>", no_restart)


class GenerationLogTests(unittest.TestCase):
    def test_round_trip_and_fold(self):
        with tempfile.TemporaryDirectory() as state:
            record(state, 1, "/nix/store/aaa-svc-1.0")
            record(state, 2, "/nix/store/bbb-svc-1.1")
            entries = deploy.read_generations("svc", state)
            self.assertEqual([e["generation"] for e in entries], [1, 2])
            current, previous = deploy.current_previous(entries)
            self.assertEqual(current["generation"], 2)
            self.assertEqual(previous["generation"], 1)
            self.assertEqual(deploy.next_generation_id(entries), 3)

    def test_rollback_fold_makes_previous_current(self):
        with tempfile.TemporaryDirectory() as state:
            record(state, 1, "/nix/store/aaa-svc-1.0")
            record(state, 2, "/nix/store/bbb-svc-1.1")
            record(state, 1, "/nix/store/aaa-svc-1.0", action="rollback", result="rolled-back")
            current, previous = deploy.current_previous(
                deploy.read_generations("svc", state)
            )
            self.assertEqual(current["generation"], 1)
            self.assertEqual(previous["generation"], 2)

    def test_empty_log_has_no_current(self):
        with tempfile.TemporaryDirectory() as state:
            self.assertEqual(deploy.read_generations("svc", state), [])
            self.assertEqual(deploy.current_previous([]), (None, None))

    def test_malformed_log_line_is_named(self):
        with tempfile.TemporaryDirectory() as state:
            record(state, 1, "/nix/store/aaa-svc-1.0")
            with open(deploy.generations_path("svc", state), "a", encoding="utf-8") as fh:
                fh.write("not json\n")
            with self.assertRaises(ValueError) as ctx:
                deploy.read_generations("svc", state)
            self.assertIn("line 2", str(ctx.exception))


class PlanTests(unittest.TestCase):
    def test_deploy_plan_order_and_flip_back(self):
        with tempfile.TemporaryDirectory() as state:
            record(state, 1, "/nix/store/aaa-svc-1.0")
            entries = deploy.read_generations("svc", state)
            steps = deploy.plan_deploy("svc", DEFN, entries, base=BASE)
            text = "\n".join(steps)
            order = [
                text.index("build the service package"),
                text.index("install generation 2 into the profile"),
                text.index("write the unit svc.service"),
                text.index("daemon-reload"),
                text.index("restart only svc.service"),
                text.index("health check"),
                text.index("flip the profile back to generation 1"),
                text.index("append generation 2 to the generation log"),
            ]
            self.assertEqual(order, sorted(order))
            self.assertIn("nix-env --profile /profiles/svc --set", text)

    def test_deploy_plan_refuses_other_classes(self):
        cf = dict(DEFN, **{"class": "cloudflare"})
        self.assertIsNone(deploy.plan_deploy("svc", cf, [], base=BASE))

    def test_rollback_plan_selects_previous_without_rebuild(self):
        with tempfile.TemporaryDirectory() as state:
            record(state, 1, "/nix/store/aaa-svc-1.0")
            record(state, 2, "/nix/store/bbb-svc-1.1")
            steps, error = deploy.plan_rollback(
                "svc", DEFN, deploy.read_generations("svc", state), base=BASE
            )
            self.assertIsNone(error)
            text = "\n".join(steps)
            self.assertIn("repoint the profile /profiles/svc at generation 1", text)
            self.assertIn("no rebuild", text)
            self.assertIn("restart only svc.service", text)

    def test_rollback_refused_with_one_generation(self):
        with tempfile.TemporaryDirectory() as state:
            record(state, 1, "/nix/store/aaa-svc-1.0")
            steps, error = deploy.plan_rollback(
                "svc", DEFN, deploy.read_generations("svc", state), base=BASE
            )
            self.assertIsNone(steps)
            self.assertIn("only one generation", error)

    def test_rollback_refused_with_no_generations(self):
        steps, error = deploy.plan_rollback("svc", DEFN, [], base=BASE)
        self.assertIsNone(steps)
        self.assertIn("no generations recorded", error)


class StatusTests(unittest.TestCase):
    def test_status_reports_fold_and_unprobed_health(self):
        with tempfile.TemporaryDirectory() as state:
            record(state, 1, "/nix/store/aaa-svc-1.0")
            record(state, 2, "/nix/store/bbb-svc-1.1")
            out = deploy.format_status(
                "svc", DEFN, deploy.read_generations("svc", state)
            )
            self.assertIn("current: generation 2 (source rev rev2", out)
            self.assertIn("previous: generation 1 (source rev rev1", out)
            self.assertIn("(not probed)", out)

    def test_status_no_healthcheck_defined(self):
        out = deploy.format_status("svc", dict(DEFN, healthCheck=None), [])
        self.assertIn("current: none recorded", out)
        self.assertIn("health: not checked (no healthCheck defined)", out)

    def test_status_health_verdicts(self):
        out = deploy.format_status("svc", DEFN, [], health=(True, "HTTP 200"))
        self.assertIn("-> ok (HTTP 200)", out)
        out = deploy.format_status("svc", DEFN, [], health=(False, "HTTP 500"))
        self.assertIn("-> FAILED (HTTP 500)", out)


class CliTests(unittest.TestCase):
    def run_cli(self, state, *argv):
        registry = {"svc": DEFN}
        reg_path = os.path.join(state, "registry.json")
        with open(reg_path, "w", encoding="utf-8") as fh:
            json.dump(registry, fh)
        env = dict(os.environ, DEPLOY_STATE_DIR=state)
        return subprocess.run(
            [sys.executable, DEPLOY_PATH, "--registry", reg_path, *argv],
            capture_output=True, text=True, env=env, timeout=30,
        )

    def test_dry_run_prints_plan_and_creates_no_state(self):
        with tempfile.TemporaryDirectory() as state:
            proc = self.run_cli(state, "svc", "--dry-run")
            self.assertEqual(proc.returncode, 0, proc.stderr)
            self.assertIn("no changes made", proc.stdout)
            self.assertIn("install generation 1 into the profile", proc.stdout)
            self.assertFalse(os.path.exists(os.path.join(state, "generations")))

    def test_real_deploy_is_refused_in_this_slice(self):
        with tempfile.TemporaryDirectory() as state:
            proc = self.run_cli(state, "svc")
            self.assertEqual(proc.returncode, 2)
            self.assertIn("next slice of issue #5766", proc.stderr)
            self.assertFalse(os.path.exists(os.path.join(state, "generations")))

    def test_cli_rollback_dry_run_refused_with_one_generation(self):
        with tempfile.TemporaryDirectory() as state:
            record(state, 1, "/nix/store/aaa-svc-1.0")
            proc = self.run_cli(state, "svc", "rollback", "--dry-run")
            self.assertEqual(proc.returncode, 1)
            self.assertIn("only one generation", proc.stderr)

    def test_cli_status_reads_seeded_log(self):
        with tempfile.TemporaryDirectory() as state:
            record(state, 1, "/nix/store/aaa-svc-1.0")
            record(state, 2, "/nix/store/bbb-svc-1.1")
            # healthCheck points at a closed port: status still exits 0
            # and reports the FAILED verdict — status is the audit surface.
            proc = self.run_cli(state, "svc", "status")
            self.assertEqual(proc.returncode, 0, proc.stderr)
            self.assertIn("current: generation 2", proc.stdout)
            self.assertIn("health: http://127.0.0.1:8080/health -> FAILED", proc.stdout)

    def test_cli_unknown_target(self):
        with tempfile.TemporaryDirectory() as state:
            proc = self.run_cli(state, "nope", "status")
            self.assertEqual(proc.returncode, 1)
            self.assertIn("unknown target", proc.stderr)


if __name__ == "__main__":
    unittest.main(argv=[sys.argv[0]])
