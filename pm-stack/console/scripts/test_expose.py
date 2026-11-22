#!/usr/bin/env python3
"""expose.py against a model of the Cloudflare account.

The model holds what the real account holds (sign-in methods, Access
applications and policies, tunnel routes, zones and DNS records) and
applies each change the script makes; a second model stands in for the
machine's console service. Checked: the sign-in gate exists before the
route does, a second run changes nothing, --check changes nothing,
records or policies the script did not write are left alone, a console
that does not come up stops the run before the route, the host's rules
sit ahead of any wildcard, and the DNS record goes in the zone that
really holds the name.
"""

import copy
import pathlib
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(pathlib.Path(__file__).parent))
import expose

HOST = "pm.security.cab"


class Account(expose.Cloudflare):
    def __init__(self, check=False):
        super().__init__("token", check)
        self.idps = [{"id": "idp-cf", "type": "cloudflare", "name": ""}]
        self.apps = []
        self.policies = {}
        self.ingress = [
            {"hostname": "kaneo.security.cab", "service": "http://localhost:5173"},
            {"service": "http_status:404"},
        ]
        self.dns = []
        self.zones = {"security.cab": "zone"}
        self.order = []

    def call(self, method, path, body=None):
        path, _, query = path.partition("?")
        if method != "GET":
            self.order.append(f"{method} {path}")
        if path == "/accounts":
            result = [{"id": "acc"}]
        elif path.endswith("/access/organizations"):
            result = {"auth_domain": "team.cloudflareaccess.com"}
        elif path.endswith("/access/identity_providers"):
            if method == "POST":
                self.idps.append({"id": f"idp-{body['type']}", **body})
            result = self.idps[-1] if method == "POST" else self.idps
        elif "/access/apps/" in path and method == "PUT":
            app = next(a for a in self.apps if a["id"] == path.split("/")[-1])
            app.update(body)
            result = app
        elif path.endswith("/access/apps"):
            if method == "POST":
                self.apps.append(
                    {
                        "id": f"app-{len(self.apps)}",
                        "aud": f"aud-{len(self.apps)}",
                        **body,
                    }
                )
                self.policies[self.apps[-1]["id"]] = []
            result = self.apps[-1] if method == "POST" else self.apps
        elif path.endswith("/policies"):
            app = path.split("/")[-2]
            if method == "POST":
                self.policies[app].append(body)
            result = self.policies[app]
        elif path.endswith("/cfd_tunnel"):
            result = [{"id": "tun", "name": expose.TUNNEL}]
        elif path.endswith("/configurations"):
            if method == "PUT":
                self.ingress = body["config"]["ingress"]
            result = {
                "config": {
                    "ingress": copy.deepcopy(self.ingress),
                    "warp-routing": {"enabled": False},
                }
            }
        elif path == "/zones":
            name = query.partition("name=")[2]
            result = [{"id": self.zones[name]}] if name in self.zones else []
        elif path.endswith("/dns_records"):
            if method == "POST":
                self.dns.append({**body, "zone": path.split("/")[2]})
            name = query.partition("name=")[2]
            result = [r for r in self.dns if r["name"] == name]
        else:
            raise AssertionError(f"unexpected call {method} {path}")
        return {"success": True, "result": result}


class Machine:
    """The console's user unit: systemctl and the health check."""

    def __init__(self, running=True, restarts=True, comes_up=True):
        self.running = running
        self.restarts = restarts
        self.comes_up = comes_up
        self.commands = []

    def systemctl(self, argv, **_):
        self.commands.append(argv[2])
        code = 0
        if argv[2] == "is-active":
            code = 0 if self.running else 3
        elif argv[2] == "restart":
            self.running = self.restarts and self.comes_up
            code = 0 if self.restarts else 1
        return mock.Mock(returncode=code, stdout="", stderr="unit failed")

    def up(self, origin, seconds=20):
        return self.running


def run(account, home, *args, machine=None):
    argv = ["expose.py", *args]
    machine = machine or Machine()
    with (
        mock.patch.object(expose, "Cloudflare", lambda token, check: account),
        mock.patch.object(
            expose,
            "secret",
            lambda name: "token" if name.startswith("cloudflare.") else "",
        ),
        mock.patch.object(expose.pathlib.Path, "home", lambda: pathlib.Path(home)),
        mock.patch.object(expose.subprocess, "run", machine.systemctl),
        mock.patch.object(expose, "console_up", machine.up),
        mock.patch.object(sys, "argv", argv),
        mock.patch("builtins.print"),
    ):
        account.check = "--check" in args
        account.changes = []
        return expose.main()


class Expose(unittest.TestCase):
    def setUp(self):
        self.home = tempfile.mkdtemp()

    def test_the_gate_exists_before_the_route(self):
        account = Account()
        self.assertEqual(run(account, self.home), 0)
        route = account.order.index("PUT /accounts/acc/cfd_tunnel/tun/configurations")
        dns = account.order.index("POST /zones/zone/dns_records")
        gate = max(i for i, c in enumerate(account.order) if "/access/" in c)
        self.assertLess(gate, route)
        self.assertLess(route, dns)
        main = next(a for a in account.apps if a["domain"] == HOST)
        self.assertEqual(
            account.policies[main["id"]],
            [
                {
                    "name": "allow: PM stack",
                    "decision": "allow",
                    "include": [{"email": {"email": "drewrypope@gmail.com"}}],
                    "precedence": 1,
                }
            ],
        )
        self.assertEqual(main["allowed_idps"], ["idp-onetimepin"])
        self.assertEqual(main["same_site_cookie_attribute"], "lax")
        self.assertIs(main["http_only_cookie_attribute"], True)
        for path in expose.PUBLIC_PATHS:
            app = next(a for a in account.apps if a["domain"] == HOST + path)
            self.assertEqual(
                [p["decision"] for p in account.policies[app["id"]]], ["bypass"]
            )
        # The whole host goes to the console: no path of it is routed past the gate.
        self.assertEqual(
            [(r.get("hostname"), r.get("path"), r["service"]) for r in account.ingress],
            [
                (HOST, None, expose.CONSOLE),
                ("kaneo.security.cab", None, "http://localhost:5173"),
                (None, None, "http_status:404"),
            ],
        )
        self.assertEqual(
            account.dns,
            [
                {
                    "type": "CNAME",
                    "name": HOST,
                    "content": "tun.cfargotunnel.com",
                    "proxied": True,
                    "comment": f"pm stack via tunnel {expose.TUNNEL}",
                    "zone": "zone",
                }
            ],
        )
        env = (pathlib.Path(self.home) / ".local/state/pm-console/.env").read_text()
        self.assertIn("PM_CONSOLE_ACCESS_AUD=" + main["aud"], env)
        self.assertIn("PM_CONSOLE_OWNER_EMAILS=drewrypope@gmail.com", env)

    def test_a_second_run_changes_nothing(self):
        account = Account()
        run(account, self.home)
        before = (
            copy.deepcopy(account.apps),
            copy.deepcopy(account.policies),
            copy.deepcopy(account.ingress),
            copy.deepcopy(account.dns),
            len(account.order),
        )
        self.assertEqual(run(account, self.home), 0)
        self.assertEqual(account.changes, [])
        self.assertEqual(
            (
                account.apps,
                account.policies,
                account.ingress,
                account.dns,
                len(account.order),
            ),
            before,
        )
        self.assertEqual(run(account, self.home, "--check"), 0)

    def test_check_reports_and_changes_nothing(self):
        account = Account()
        self.assertEqual(run(account, self.home, "--check"), 1)
        self.assertEqual(account.order, [])
        self.assertEqual(len(account.changes), 7)
        self.assertFalse(
            (pathlib.Path(self.home) / ".local/state/pm-console/.env").exists()
        )

    def test_foreign_records_and_policies_are_left_alone(self):
        account = Account()
        account.dns = [
            {"type": "A", "name": HOST, "content": "203.0.113.5", "proxied": False}
        ]
        with self.assertRaisesRegex(SystemExit, "not this tunnel"):
            run(account, self.home)
        self.assertEqual(len(account.dns), 1)
        self.assertEqual(account.order, [])  # refused before anything was written

        account = Account()
        account.apps = [
            {
                "id": "app-x",
                "aud": "aud-x",
                "domain": HOST,
                "type": "self_hosted",
                "allowed_idps": ["idp-onetimepin"],
            }
        ]
        account.policies = {
            "app-x": [{"decision": "allow", "include": [{"everyone": {}}]}]
        }
        with self.assertRaisesRegex(SystemExit, "did not write"):
            run(account, self.home)
        # Nothing was routed, and no policy added, while the gate was in doubt.
        self.assertNotIn(HOST, [r.get("hostname") for r in account.ingress])
        self.assertEqual(len(account.policies["app-x"]), 1)

        # The owner's own rule with a condition added is not this script's rule.
        owner = [{"email": {"email": "drewrypope@gmail.com"}}]
        for extra in (
            {"exclude": owner},
            {"require": [{"geo": {"country_code": "XX"}}]},
        ):
            account = Account()
            run(account, self.home)
            main = next(a for a in account.apps if a["domain"] == HOST)
            account.policies[main["id"]][0].update(extra)
            with self.assertRaisesRegex(SystemExit, "did not write"):
                run(account, self.home)
        # Cookie settings changed by hand are put back.
        account = Account()
        run(account, self.home)
        main = next(a for a in account.apps if a["domain"] == HOST)
        main["same_site_cookie_attribute"] = "none"
        self.assertEqual(run(account, self.home), 0)
        self.assertEqual(len(account.changes), 1)
        self.assertEqual(main["same_site_cookie_attribute"], "lax")
        self.assertEqual(run(account, self.home), 0)
        self.assertEqual(account.changes, [])
        # Empty lists, as the API returns them, are still this script's rule.
        account = Account()
        run(account, self.home)
        main = next(a for a in account.apps if a["domain"] == HOST)
        account.policies[main["id"]][0].update({"exclude": [], "require": []})
        self.assertEqual(run(account, self.home), 0)
        self.assertEqual(account.changes, [])

    def test_a_console_that_does_not_come_up_stops_the_run_before_the_route(self):
        for machine, why in (
            (Machine(restarts=False), "did not restart"),
            (Machine(comes_up=False), "does not answer"),
        ):
            account = Account()
            with self.assertRaisesRegex(SystemExit, why):
                run(account, self.home, machine=machine)
            self.assertNotIn(HOST, [r.get("hostname") for r in account.ingress])
            self.assertEqual(account.dns, [])
            pathlib.Path(self.home, ".local/state/pm-console/.env").unlink()

        # Settings already right but the service down: it is started, and
        # --check says so instead of calling the setup complete.
        account = Account()
        run(account, self.home)
        down = Machine(running=False)
        self.assertEqual(run(account, self.home, "--check", machine=down), 1)
        self.assertEqual(down.commands, ["is-active"])
        self.assertEqual(run(account, self.home, machine=down), 0)
        self.assertEqual(down.commands, ["is-active", "is-active", "restart"])
        self.assertTrue(down.running)
        up = Machine()
        self.assertEqual(run(account, self.home, machine=up), 0)
        self.assertEqual(up.commands, ["is-active"])

    def test_the_route_follows_the_console_port_and_sits_ahead_of_wildcards(self):
        env = pathlib.Path(self.home, ".local/state/pm-console/.env")
        env.parent.mkdir(parents=True)
        env.write_text('# settings\nPM_CONSOLE_PORT="4999"\nOTHER=kept\n')
        account = Account()
        wildcard = {"hostname": "*.security.cab", "service": "http://localhost:1"}
        account.ingress.insert(0, wildcard)
        self.assertEqual(run(account, self.home), 0)
        self.assertEqual(
            [(r.get("hostname"), r["service"]) for r in account.ingress],
            [
                (HOST, "http://localhost:4999"),
                ("*.security.cab", "http://localhost:1"),
                ("kaneo.security.cab", "http://localhost:5173"),
                (None, "http_status:404"),
            ],
        )
        self.assertIn("OTHER=kept", env.read_text())
        self.assertIn('PM_CONSOLE_PORT="4999"', env.read_text())
        # The host's rules behind a wildcard are moved ahead of it, once.
        account.ingress = [wildcard] + [
            r for r in account.ingress if r is not wildcard and r != wildcard
        ]
        self.assertEqual(run(account, self.home), 0)
        self.assertEqual(account.ingress[0]["hostname"], HOST)
        self.assertEqual(len(account.changes), 1)
        self.assertEqual(run(account, self.home), 0)
        self.assertEqual(account.changes, [])
        # Rules from an earlier layout that sent part of the host straight to
        # the dashboard are replaced: nothing on the host skips the console.
        account.ingress = [
            {"hostname": HOST, "path": "^/console", "service": "http://localhost:4999"},
            {"hostname": HOST, "service": "http://localhost:4870"},
            {"service": "http_status:404"},
        ]
        self.assertEqual(run(account, self.home), 0)
        self.assertEqual(
            [(r.get("hostname"), r.get("path"), r["service"]) for r in account.ingress],
            [(HOST, None, "http://localhost:4999"), (None, None, "http_status:404")],
        )
        # A tunnel whose last rule is not a catch-all gets one.
        account.ingress = [{"hostname": "kaneo.security.cab", "service": "x"}]
        run(account, self.home)
        self.assertEqual(account.ingress[-1], {"service": "http_status:404"})

        env.write_text("PM_CONSOLE_PORT=console\n")
        account = Account()
        with self.assertRaisesRegex(SystemExit, "PM_CONSOLE_PORT"):
            run(account, self.home)
        self.assertEqual(account.order, [])  # refused before anything was written

    def test_the_record_goes_in_the_zone_that_holds_the_name(self):
        host = "pm.dev.example.co.uk"
        account = Account()
        account.zones = {"example.co.uk": "outer", "dev.example.co.uk": "inner"}
        self.assertEqual(run(account, self.home, "--host", host), 0)
        self.assertEqual(
            [(r["name"], r["zone"]) for r in account.dns], [(host, "inner")]
        )
        account = Account()
        account.zones = {"example.co.uk": "outer"}
        self.assertEqual(run(account, self.home, "--host", host), 0)
        self.assertEqual([r["zone"] for r in account.dns], ["outer"])
        account = Account()
        account.zones = {"co.uk.example": "wrong"}
        with self.assertRaisesRegex(SystemExit, "no zone"):
            run(account, self.home, "--host", host)
        self.assertEqual(account.order, [])


if __name__ == "__main__":
    unittest.main()
