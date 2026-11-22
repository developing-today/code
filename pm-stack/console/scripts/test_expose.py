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
import io
import json
import pathlib
import re
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(pathlib.Path(__file__).parent))
import expose

HOST = "pm.security.cab"
OWNER = "owner@example.com"
GOOGLE_CLIENT = "123456789012-abc123def456.apps.googleusercontent.com"
GOOGLE_VALUE = "not-a-real-value-0123456789"


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
        # Workers routes on the zone, and whether the edge Worker answers.
        self.routes = []
        self.worker_up = True
        # The hostname the Worker says it is built for, and how many of
        # its next health checks get no answer.
        self.worker_host = HOST
        self.health_misses = 0
        # What the Worker holds as secrets, and how many of the next
        # uploads Cloudflare refuses.
        self.worker_secrets = {}
        self.secret_failures = 0

    def call(self, method, path, body=None):
        path, _, query = path.partition("?")
        self.asked = getattr(self, "asked", 0) + 1  # every request, reads too
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
        elif "/access/identity_providers/" in path and method == "PUT":
            idp = next(i for i in self.idps if i["id"] == path.split("/")[-1])
            idp.update(body)
            result = idp
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
        elif path.endswith(f"/workers/scripts/{expose.WORKER}/secrets"):
            # The Worker's secrets: names can be listed, values never read back.
            if method == "PUT":
                if self.secret_failures > 0:
                    self.secret_failures -= 1
                    raise SystemExit("cloudflare PUT secrets: refused")
                self.worker_secrets[body["name"]] = body["text"]
            result = [
                {"name": name, "type": "secret_text"} for name in self.worker_secrets
            ]
        elif (
            f"/workers/scripts/{expose.WORKER}/secrets/" in path and method == "DELETE"
        ):
            self.worker_secrets.pop(path.split("/")[-1], None)
            result = None
        elif path.endswith("/workers/subdomain"):
            result = {"subdomain": "acct"}
        elif "/workers/routes" in path:
            if method == "POST":
                self.routes.append({"id": f"route-{len(self.routes)}", **body})
            elif method == "DELETE":
                self.routes = [r for r in self.routes if r["id"] != path.split("/")[-1]]
            result = self.routes
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


def run(
    account,
    home,
    *args,
    machine=None,
    owner=OWNER,
    stdin=None,
    secrets=None,
):
    """One run of the script. What it printed is left in account.printed."""
    argv = ["expose.py", *(["--owner", owner] if owner else []), *args]
    machine = machine or Machine()

    def secret(name):
        if secrets is not None and name in secrets:
            return secrets[name]
        return "token" if name.startswith("cloudflare.") else ""

    def health(url):
        if account.health_misses > 0:
            account.health_misses -= 1
            return None
        if (
            not account.worker_up
            or url != f"https://{expose.WORKER}.acct.workers.dev/console/health"
        ):
            return None
        return {"ok": True, "edge": True, "host": account.worker_host}

    with (
        mock.patch.object(expose, "Cloudflare", lambda token, check: account),
        mock.patch.object(expose, "secret", secret),
        mock.patch.object(expose.pathlib.Path, "home", lambda: pathlib.Path(home)),
        mock.patch.object(expose.subprocess, "run", machine.systemctl),
        mock.patch.object(expose, "console_up", machine.up),
        mock.patch.object(expose, "fetch_json", health),
        mock.patch.object(expose.time, "sleep", lambda seconds: None),
        mock.patch.object(sys, "argv", argv),
        mock.patch.object(sys, "stdin", io.StringIO(stdin or "")),
        mock.patch("builtins.print") as printed,
    ):
        account.check = "--check" in args
        account.changes = []
        try:
            return expose.main()
        finally:
            account.printed = [str(c.args[0]) for c in printed.call_args_list]


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
                    "include": [{"email": {"email": OWNER}}],
                    "precedence": 1,
                }
            ],
        )
        self.assertEqual(main["allowed_idps"], ["idp-onetimepin"])
        self.assertEqual(main["same_site_cookie_attribute"], "lax")
        self.assertEqual(main["session_duration"], "720h")
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
        self.assertIn(f"PM_CONSOLE_OWNER_EMAILS={OWNER}", env)

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
        # One per public path, then the application, the console's
        # settings, the route and the DNS record.
        self.assertEqual(
            expose.PUBLIC_PATHS,
            (
                "/.well-known/assetlinks.json",
                "/manifest.webmanifest",
                "/icons",
                "/console/device",
                "/console/edge",
            ),
        )
        # ... and the hostname's route through the edge Worker.
        self.assertEqual(len(account.changes), len(expose.PUBLIC_PATHS) + 5)
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
        owner = [{"email": {"email": OWNER}}]
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

    def env(self):
        return expose.env_values(
            pathlib.Path(self.home) / ".local/state/pm-console/.env"
        )

    def test_the_hostname_goes_through_the_edge_last_and_only_once_it_answers(self):
        # The Worker is not deployed yet: nothing about the edge is set up.
        account = Account()
        account.worker_up = False
        self.assertEqual(run(account, self.home), 0)
        self.assertEqual(account.routes, [])
        self.assertNotIn("PM_CONSOLE_EDGE_URL", self.env())
        self.assertEqual(run(account, self.home), 0)
        self.assertEqual(account.changes, [])
        # It answers: the host is told where to report, then the route goes in, last of all.
        account.worker_up = True
        account.order.clear()
        machine = Machine()
        self.assertEqual(run(account, self.home, machine=machine), 0)
        self.assertEqual(
            account.routes,
            [{"id": "route-0", "pattern": f"{HOST}/*", "script": expose.WORKER}],
        )
        self.assertEqual(self.env()["PM_CONSOLE_EDGE_URL"], f"https://{HOST}")
        self.assertIn("restart", machine.commands)
        self.assertEqual(account.order, ["POST /zones/zone/workers/routes"])
        self.assertEqual(
            account.changes,
            [
                "console environment: PM_CONSOLE_EDGE_URL",
                f"workers: route {HOST}/* through {expose.WORKER}",
            ],
        )
        self.assertEqual(run(account, self.home), 0)
        self.assertEqual(account.changes, [])

    def test_a_console_that_does_not_come_up_is_never_put_behind_the_edge(self):
        account = Account()
        with self.assertRaises(SystemExit):
            run(account, self.home, machine=Machine(comes_up=False))
        self.assertEqual(account.routes, [])

    def test_edge_off_is_remembered_and_edge_on_brings_it_back(self):
        account = Account()
        run(account, self.home)
        self.assertEqual(len(account.routes), 1)
        self.assertEqual(run(account, self.home, "--edge", "off"), 0)
        self.assertEqual(account.routes, [])
        self.assertNotIn("PM_CONSOLE_EDGE_URL", self.env())
        # The sign-in settings are still there: only the edge's went.
        self.assertEqual(self.env()["PM_CONSOLE_OWNER_EMAILS"], OWNER)
        # A later run with no flag (the timer's) keeps it off.
        self.assertEqual(run(account, self.home), 0)
        self.assertEqual(account.changes, [])
        self.assertEqual(account.routes, [])
        self.assertEqual(run(account, self.home, "--edge", "on"), 0)
        self.assertEqual(
            [(r["pattern"], r["script"]) for r in account.routes],
            [(f"{HOST}/*", expose.WORKER)],
        )
        self.assertEqual(self.env()["PM_CONSOLE_EDGE_URL"], f"https://{HOST}")

    def test_direct_mode_gives_the_worker_its_tokens_and_takes_them_away_again(self):
        account = Account()
        run(account, self.home)
        marker = pathlib.Path(self.home) / ".local/state/pm-console/edge-direct"
        path = f"/accounts/acc/workers/scripts/{expose.WORKER}/secrets"
        # Off by default: an ordinary run uploads nothing.
        self.assertEqual(account.worker_secrets, {})
        # Asked what turning it on would change: said, and nothing done.
        account.order.clear()
        self.assertEqual(run(account, self.home, "--check", "--edge-direct", "on"), 1)
        self.assertEqual(len(account.changes), 1)
        self.assertEqual(account.order, [])
        self.assertFalse(marker.exists())
        # The tokens cannot be read here: the run fails, and nothing is
        # remembered, so a later run still tries to turn direct mode on.
        with self.assertRaisesRegex(SystemExit, "is not readable here"):
            run(account, self.home, "--edge-direct", "on")
        self.assertFalse(marker.exists())
        self.assertEqual(account.worker_secrets, {})
        secrets = {
            expose.GITHUB_TOKEN_SECRET: "gh-test-value",
            expose.QUEUE_TOKEN_DEFAULT: "queue-test-value",
        }
        # Cloudflare refuses part-way: still nothing remembered.
        account.secret_failures = 1
        with self.assertRaisesRegex(SystemExit, "refused"):
            run(account, self.home, "--edge-direct", "on", secrets=secrets)
        self.assertFalse(marker.exists())
        # Readable and accepted: the two tokens and the opt-in are the
        # Worker's, through Cloudflare's API and nothing installed here.
        account.order.clear()
        self.assertEqual(
            run(account, self.home, "--edge-direct", "on", secrets=secrets), 0
        )
        self.assertEqual(
            account.worker_secrets,
            {
                "GITHUB_TOKEN": "gh-test-value",
                "QUEUE_TOKEN": "queue-test-value",
                "EDGE_DIRECT": "on",
            },
        )
        self.assertEqual(account.order, [f"PUT {path}"] * 3)
        self.assertEqual(marker.read_text().strip(), "on")
        # Neither value is in anything it printed, said it changed, or wrote.
        said = "\n".join(account.printed + account.changes)
        for value in secrets.values():
            self.assertNotIn(value, said)
            for kept in pathlib.Path(self.home).rglob("*"):
                if kept.is_file():
                    self.assertNotIn(value, kept.read_text())
        # A later run with the same choice, or with none (the timer's),
        # uploads nothing again.
        for again in (("--edge-direct", "on"), ()):
            account.order.clear()
            self.assertEqual(run(account, self.home, *again, secrets=secrets), 0)
            self.assertEqual(account.changes, [])
            self.assertEqual(account.order, [])
        # Off: the opt-in first, then the two tokens are taken away.
        account.order.clear()
        self.assertEqual(run(account, self.home, "--edge-direct", "off"), 0)
        self.assertEqual(account.worker_secrets, {"EDGE_DIRECT": "off"})
        self.assertEqual(
            account.order,
            [
                f"PUT {path}",
                f"DELETE {path}/GITHUB_TOKEN",
                f"DELETE {path}/QUEUE_TOKEN",
            ],
        )
        self.assertEqual(marker.read_text().strip(), "off")

    def test_another_workers_route_on_the_hostname_is_left_alone(self):
        for foreign in (
            {"id": "x", "pattern": f"{HOST}/*", "script": "someone-elses"},
            {"id": "y", "pattern": f"{HOST}/api/*", "script": expose.WORKER},
        ):
            account = Account()
            account.routes = [dict(foreign)]
            with self.assertRaises(SystemExit) as stopped:
                run(account, self.home)
            self.assertIn("not touching it", str(stopped.exception))
            self.assertEqual(account.routes, [foreign])
            self.assertFalse(any("workers/routes" in c for c in account.order))
        # A route for a different hostname in the same zone is nobody's business here.
        account = Account()
        account.routes = [
            {"id": "z", "pattern": "bridge.security.cab/*", "script": "message-bridge"}
        ]
        self.assertEqual(run(account, self.home), 0)
        self.assertEqual(len(account.routes), 2)

    def test_check_previews_an_edge_choice_and_keeps_nothing(self):
        account = Account()
        run(account, self.home)
        # Asked what turning the edge off would change: both things, and exit 1.
        self.assertEqual(run(account, self.home, "--check", "--edge", "off"), 1)
        self.assertEqual(
            account.changes,
            [
                "console environment: PM_CONSOLE_EDGE_URL",
                f"workers: take {HOST}/* off {expose.WORKER}",
            ],
        )
        # Nothing was done and nothing was remembered.
        self.assertFalse(
            (pathlib.Path(self.home) / ".local/state/pm-console/edge").exists()
        )
        self.assertEqual(len(account.routes), 1)
        self.assertEqual(self.env()["PM_CONSOLE_EDGE_URL"], f"https://{HOST}")
        self.assertEqual(run(account, self.home, "--check"), 0)
        # And the other way: off for real, then asked what on would change.
        self.assertEqual(run(account, self.home, "--edge", "off"), 0)
        self.assertEqual(run(account, self.home, "--check", "--edge", "on"), 1)
        self.assertEqual(len(account.changes), 2)
        self.assertEqual(account.routes, [])
        self.assertEqual(run(account, self.home, "--check"), 0)

    def test_a_worker_built_for_another_hostname_is_not_routed(self):
        account = Account()
        account.worker_host = "pm.other.example"
        self.assertEqual(run(account, self.home), 0)
        self.assertEqual(account.routes, [])
        self.assertNotIn("PM_CONSOLE_EDGE_URL", self.env())
        # A Worker from before it said which hostname it is built for.
        with mock.patch.object(expose, "HEALTH_TRIES", 1):
            account.worker_host = None
            self.assertEqual(run(account, self.home), 0)
        self.assertEqual(account.routes, [])

    def test_one_unanswered_health_check_does_not_take_the_hostname_off_the_edge(self):
        account = Account()
        run(account, self.home)
        self.assertEqual(len(account.routes), 1)
        # Two checks in a row get no answer; the third does.
        account.health_misses = expose.HEALTH_TRIES - 1
        self.assertEqual(run(account, self.home), 0)
        self.assertEqual(account.changes, [])
        self.assertEqual(len(account.routes), 1)
        # None of them does: the hostname goes back to this host, which is
        # what keeps it reachable when the Worker is what is broken.
        account.health_misses = expose.HEALTH_TRIES
        self.assertEqual(run(account, self.home), 0)
        self.assertEqual(
            account.changes,
            [
                "console environment: PM_CONSOLE_EDGE_URL",
                f"workers: take {HOST}/* off {expose.WORKER}",
            ],
        )
        self.assertEqual(account.routes, [])
        # And returns by itself once the Worker answers again.
        self.assertEqual(run(account, self.home), 0)
        self.assertEqual(len(account.routes), 1)

    def test_the_owner_is_named_once_and_remembered_and_never_written_in_the_script(
        self,
    ):
        # Never told, on a machine that has not been set up: refused
        # before anything is read or changed.
        account = Account()
        with self.assertRaisesRegex(SystemExit, "say whose sign-in is let in"):
            run(account, self.home, owner=None)
        with self.assertRaisesRegex(SystemExit, "say whose sign-in"):
            run(account, self.home, owner="not an address")
        self.assertEqual(getattr(account, "asked", 0), 0)
        # Told once: the console's environment file remembers, and a run
        # that is not told (the timer's) uses that and changes nothing.
        self.assertEqual(run(account, self.home, owner="Owner@Example.com"), 0)
        self.assertEqual(self.env()["PM_CONSOLE_OWNER_EMAILS"], OWNER)
        self.assertEqual(run(account, self.home, owner=None), 0)
        self.assertEqual(account.changes, [])
        main = next(a for a in account.apps if a["domain"] == HOST)
        self.assertEqual(
            account.policies[main["id"]][0]["include"], [{"email": {"email": OWNER}}]
        )
        # No address is written in the script itself.
        source = pathlib.Path(expose.__file__).read_text()
        self.assertIsNone(
            re.search(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+\.[a-z]{2,}", source)
        )

    def test_google_sign_in_is_turned_on_from_a_client_handed_in(self):
        account = Account()
        run(account, self.home)
        main = next(a for a in account.apps if a["domain"] == HOST)
        self.assertEqual(main["allowed_idps"], ["idp-onetimepin"])
        # Not a client: refused before Cloudflare is asked anything.
        for bad in (
            "",
            f"{GOOGLE_CLIENT}\n",
            f"not-a-client-id\n{GOOGLE_VALUE}\n",
            f"{GOOGLE_CLIENT}\nhas a space in it\n",
            f"{GOOGLE_CLIENT}\n{GOOGLE_VALUE}\nthird line\n",
        ):
            account.asked = 0
            with self.assertRaisesRegex(SystemExit, "--google-stdin") as refused:
                run(account, self.home, "--google-stdin", stdin=bad)
            self.assertEqual(account.asked, 0)
            self.assertNotIn(GOOGLE_VALUE, str(refused.exception))
            self.assertNotIn(GOOGLE_VALUE, "\n".join(account.printed))
        # A client: Google is added and the application lets both methods in.
        self.assertEqual(
            run(
                account,
                self.home,
                "--google-stdin",
                stdin=f"  {GOOGLE_CLIENT}\n{GOOGLE_VALUE}\n",
            ),
            0,
        )
        google = next(i for i in account.idps if i["type"] == "google")
        self.assertEqual(
            google["config"],
            {"client_id": GOOGLE_CLIENT, "client_secret": GOOGLE_VALUE},
        )
        self.assertEqual(main["allowed_idps"], ["idp-onetimepin", "idp-google"])
        self.assertEqual(account.changes[0], "access: add Google sign-in")
        # Nothing it printed, and nothing it wrote, holds either value.
        self.assertNotIn(GOOGLE_VALUE, "\n".join(account.printed))
        self.assertNotIn(GOOGLE_CLIENT, "\n".join(account.printed))
        for kept in pathlib.Path(self.home).rglob("*"):
            if kept.is_file():
                self.assertNotIn(GOOGLE_VALUE, kept.read_text())
        # The timer's run, with nothing handed in, keeps it as it is.
        self.assertEqual(run(account, self.home), 0)
        self.assertEqual(account.changes, [])
        # Handed a client again (a corrected one): that is the one used.
        other = GOOGLE_VALUE[::-1]
        self.assertEqual(
            run(
                account,
                self.home,
                "--google-stdin",
                stdin=f"{GOOGLE_CLIENT}\n{other}\n",
            ),
            0,
        )
        self.assertEqual(
            account.changes, ["access: give Google sign-in the client just supplied"]
        )
        self.assertEqual(google["config"]["client_secret"], other)
        self.assertEqual(len([i for i in account.idps if i["type"] == "google"]), 1)

    def test_status_says_which_sign_in_methods_are_on_and_changes_nothing(self):
        account = Account()
        # Needs no owner, and works before anything is set up.
        self.assertEqual(run(account, self.home, "--status", owner=None), 0)
        self.assertEqual(
            json.loads(account.printed[-1]),
            {"google": False, "pin": False, "team": "team.cloudflareaccess.com"},
        )
        self.assertEqual(account.order, [])
        self.assertFalse((pathlib.Path(self.home) / ".local").exists())
        run(
            account,
            self.home,
            "--google-stdin",
            stdin=f"{GOOGLE_CLIENT}\n{GOOGLE_VALUE}\n",
        )
        account.order.clear()
        self.assertEqual(run(account, self.home, "--status", owner=None), 0)
        self.assertEqual(
            json.loads(account.printed[-1]),
            {"google": True, "pin": True, "team": "team.cloudflareaccess.com"},
        )
        self.assertEqual(account.order, [])

    def test_the_health_check_names_itself(self):
        # Found live: the edge's health check answered 403 to Python's
        # default client name, so the Worker looked undeployed.
        seen = []

        class Answer:
            def __enter__(self):
                return self

            def __exit__(self, *_):
                return False

            def read(self, *_):
                return b'{"ok": true, "edge": true}'

        def urlopen(request, timeout):
            seen.append((request.full_url, request.get_header("User-agent"), timeout))
            return Answer()

        with mock.patch.object(expose.urllib.request, "urlopen", urlopen):
            self.assertEqual(
                expose.fetch_json("https://w.example/console/health"),
                {"ok": True, "edge": True},
            )
        self.assertEqual(len(seen), 1)
        self.assertEqual(seen[0][0], "https://w.example/console/health")
        # The program's name and nothing more: no address, no contact.
        self.assertEqual(seen[0][1], "pm-console-expose")
        # Anything that is not a JSON object, or cannot be had, is "no answer".
        with mock.patch.object(
            expose.urllib.request, "urlopen", mock.Mock(side_effect=OSError("403"))
        ):
            self.assertIsNone(expose.fetch_json("https://w.example/x"))


if __name__ == "__main__":
    unittest.main()
