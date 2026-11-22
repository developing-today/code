#!/usr/bin/env python3
"""expose.py against a model of the Cloudflare account.

The model holds what the real account holds (sign-in methods, Access
applications and policies, tunnel routes, DNS records) and applies each
change the script makes. Checked: the sign-in gate exists before the
route does, a second run changes nothing, --check changes nothing, and
records or policies the script did not write are left alone.
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
        self.order = []

    def call(self, method, path, body=None):
        path = path.partition("?")[0]
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
            result = [{"id": "zone"}]
        elif path.endswith("/dns_records"):
            if method == "POST":
                self.dns.append(body)
            result = [r for r in self.dns if r["name"] == HOST]
        else:
            raise AssertionError(f"unexpected call {method} {path}")
        return {"success": True, "result": result}


def run(account, home, *args):
    argv = ["expose.py", *args]
    with (
        mock.patch.object(expose, "Cloudflare", lambda token, check: account),
        mock.patch.object(
            expose,
            "secret",
            lambda name: "token" if name.startswith("cloudflare.") else "",
        ),
        mock.patch.object(expose.pathlib.Path, "home", lambda: pathlib.Path(home)),
        mock.patch.object(expose.subprocess, "run"),
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
        for path in expose.PUBLIC_PATHS:
            app = next(a for a in account.apps if a["domain"] == HOST + path)
            self.assertEqual(
                [p["decision"] for p in account.policies[app["id"]]], ["bypass"]
            )
        # /console before the catch-all for the host; other hosts and the final rule kept.
        self.assertEqual(
            [(r.get("hostname"), r.get("path"), r["service"]) for r in account.ingress],
            [
                ("kaneo.security.cab", None, "http://localhost:5173"),
                (HOST, expose.CONSOLE_PATHS, expose.CONSOLE),
                (HOST, None, expose.DASHBOARD),
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


if __name__ == "__main__":
    unittest.main()
