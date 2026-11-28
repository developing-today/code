#!/usr/bin/env python3
"""Keep this laptop's named Cloudflare tunnel pointed at its local console.

Access, DNS, Worker routes and Worker settings are applied only by the
repository workflow in infra/pm-cloudflare. This script owns tunnel ingress.
"""

from __future__ import annotations

import json
import pathlib
import subprocess
import sys
import urllib.error
import urllib.request

API = "https://api.cloudflare.com/client/v4"
TUNNEL = "amd-6-pm-stack"
CONSOLE = "http://localhost:4872"


class Cloudflare:
    def __init__(self, token: str, check: bool):
        self.token = token
        self.check = check
        self.changes: list[str] = []

    def call(self, method: str, path: str, body=None):
        req = urllib.request.Request(
            API + path,
            method=method,
            data=None if body is None else json.dumps(body).encode(),
            headers={
                "Authorization": f"Bearer {self.token}",
                "Content-Type": "application/json",
            },
        )
        try:
            with urllib.request.urlopen(req, timeout=30) as response:
                result = json.load(response)
        except urllib.error.HTTPError as error:
            result = json.load(error)
        if not result.get("success"):
            raise SystemExit(
                f"cloudflare {method} {path.split('?')[0]}: {json.dumps(result.get('errors'))[:300]}"
            )
        return result

    def get(self, path: str):
        return self.call("GET", path)["result"]

    def change(self, what: str, method: str, path: str, body):
        self.changes.append(what)
        if self.check:
            return None
        return self.call(method, path, body)["result"]


def configure(cf: Cloudflare, host: str = "pm.security.cab", console: str = CONSOLE):
    accounts = cf.get("/accounts?per_page=5")
    if len(accounts) != 1:
        raise SystemExit(f"expected one account, found {len(accounts)}")
    account = accounts[0]["id"]
    tunnels = cf.get(f"/accounts/{account}/cfd_tunnel?name={TUNNEL}&is_deleted=false")
    if len(tunnels) != 1:
        raise SystemExit(f"expected one tunnel named {TUNNEL}, found {len(tunnels)}")
    tunnel = tunnels[0]["id"]
    configuration = cf.get(f"/accounts/{account}/cfd_tunnel/{tunnel}/configurations")[
        "config"
    ]
    ingress = configuration.get("ingress") or []
    wanted = {"hostname": host, "service": console}
    plain = lambda rule: {
        key: rule[key] for key in ("hostname", "path", "service") if key in rule
    }
    if (
        ingress
        and plain(ingress[0]) == wanted
        and all(rule.get("hostname") != host for rule in ingress[1:])
    ):
        return
    rest = [rule for rule in ingress if rule.get("hostname") != host]
    if not rest or rest[-1].get("hostname") or rest[-1].get("path"):
        rest.append({"service": "http_status:404"})
    configuration["ingress"] = [wanted] + rest
    cf.change(
        f"tunnel {TUNNEL}: route {host} to the console",
        "PUT",
        f"/accounts/{account}/cfd_tunnel/{tunnel}/configurations",
        {"config": configuration},
    )


def main() -> int:
    check = sys.argv[1:] in (["--check"], [])
    if sys.argv[1:] not in ([], ["--check"]):
        raise SystemExit("usage: expose.py [--check]")
    token = subprocess.run(
        ["fleet", "secrets", "get", "cloudflare.api-token-account-wide"],
        capture_output=True,
        text=True,
        check=False,
    ).stdout.strip()
    if not token:
        raise SystemExit(
            "secret cloudflare.api-token-account-wide is not readable here"
        )
    cf = Cloudflare(token, check)
    configure(cf)
    for change in cf.changes:
        print(("would change: " if check else "changed: ") + change)
    if not cf.changes:
        print(f"tunnel {TUNNEL}: ingress is set up for the local console")
    return 1 if check and cf.changes else 0


if __name__ == "__main__":
    sys.exit(main())
