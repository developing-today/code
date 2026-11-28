#!/usr/bin/env python3
"""Model tests for the laptop script's tunnel-ingress ownership boundary."""

import copy
import pathlib
import sys
import unittest
from unittest import mock

sys.path.insert(0, str(pathlib.Path(__file__).parent))
import expose


class Account(expose.Cloudflare):
    def __init__(self, check=False):
        super().__init__("token", check)
        self.ingress = [
            {"hostname": "kaneo.security.cab", "service": "http://localhost:5173"},
            {"service": "http_status:404"},
        ]
        self.calls = []

    def call(self, method, path, body=None):
        self.calls.append((method, path, body))
        if path == "/accounts?per_page=5":
            result = [{"id": "account"}]
        elif path.endswith("/cfd_tunnel?name=amd-6-pm-stack&is_deleted=false"):
            result = [{"id": "tunnel"}]
        elif path.endswith("/configurations"):
            if method == "PUT":
                self.ingress = copy.deepcopy(body["config"]["ingress"])
            result = {"config": {"ingress": copy.deepcopy(self.ingress)}}
        else:
            raise AssertionError(f"unexpected Cloudflare call: {method} {path}")
        return {"success": True, "result": result}


class ExposeTunnelBoundary(unittest.TestCase):
    def test_only_tunnel_ingress_is_written_and_unrelated_rules_are_preserved(self):
        account = Account()
        expose.configure(account)
        writes = [
            (method, path) for method, path, _ in account.calls if method != "GET"
        ]
        self.assertEqual(
            writes, [("PUT", "/accounts/account/cfd_tunnel/tunnel/configurations")]
        )
        self.assertEqual(
            account.ingress[0],
            {"hostname": "pm.security.cab", "service": expose.CONSOLE},
        )
        self.assertEqual(account.ingress[1]["hostname"], "kaneo.security.cab")
        self.assertEqual(account.ingress[-1], {"service": "http_status:404"})
        self.assertFalse(
            any(
                "access" in path or "dns" in path or "workers" in path
                for _, path, _ in account.calls
            )
        )

    def test_current_tunnel_config_is_idempotent(self):
        account = Account()
        account.ingress.insert(
            0, {"hostname": "pm.security.cab", "service": expose.CONSOLE}
        )
        expose.configure(account)
        self.assertFalse(any(method != "GET" for method, _, _ in account.calls))

    def test_check_reports_tunnel_drift_without_writing(self):
        account = Account(check=True)
        before = copy.deepcopy(account.ingress)
        expose.configure(account)
        self.assertEqual(account.ingress, before)
        self.assertEqual(len(account.changes), 1)
        self.assertTrue(all(method == "GET" for method, _, _ in account.calls))

    def test_desired_ingress_keeps_unrelated_host_rules_and_final_fallback(self):
        account = Account()
        account.ingress = [
            {"hostname": "pm.security.cab", "service": "http://localhost:1"},
            {"hostname": "other.security.cab", "service": "http://localhost:2"},
        ]
        expose.configure(account)
        self.assertEqual(
            account.ingress[0],
            {"hostname": "pm.security.cab", "service": expose.CONSOLE},
        )
        self.assertEqual(
            account.ingress[1],
            {"hostname": "other.security.cab", "service": "http://localhost:2"},
        )
        self.assertEqual(account.ingress[-1], {"service": "http_status:404"})

    def test_script_rejects_old_account_management_flags(self):
        with mock.patch.object(sys, "argv", ["expose.py", "--edge", "off"]):
            with self.assertRaisesRegex(SystemExit, "usage: expose.py"):
                expose.main()


if __name__ == "__main__":
    unittest.main()
