#!/usr/bin/env python3
"""Put the PM stack on a public hostname behind sign-in, or check that it is.

One hostname carries the dashboard (pm-aggregator) and the console
(pm-console). This script makes each of these true, changing only what
is not already so, and prints what it did:

1. The tunnel routes the hostname: /console and the Android asset-links
   file to the console, every other path to the dashboard.
2. DNS points the hostname at the tunnel.
3. A Cloudflare Access application covers the hostname, with one policy:
   the owner's address. Sign-in methods are the one-time PIN, plus Google
   when the secrets `google.access-sso.client-id` and
   `google.access-sso.client-secret` exist.
4. Three public paths bypass sign-in, because a phone fetches them before
   anyone has signed in: the asset-links file, the install manifest and
   its icons. None of them says anything about the fleet.
5. The console's environment file names the Access issuer and the owner.

Every secret is read by name through `fleet secrets get` and stays in
memory. Nothing printed is a secret.

Usage: expose.py [--check] [--host pm.security.cab] [--owner EMAIL]
  --check   report what would change and exit 1 if anything would.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import subprocess
import sys
import urllib.error
import urllib.request

API = "https://api.cloudflare.com/client/v4"
TUNNEL = "amd-6-pm-stack"
CONSOLE = "http://localhost:4872"
DASHBOARD = "http://localhost:4870"
CONSOLE_PATHS = r"^/(console($|/)|\.well-known/assetlinks\.json$)"
PUBLIC_PATHS = ("/.well-known/assetlinks.json", "/manifest.webmanifest", "/icons")
GOOGLE_ID = "google.access-sso.client-id"
GOOGLE_SECRET = "google.access-sso.client-secret"


def secret(name: str) -> str:
    return subprocess.run(
        ["fleet", "secrets", "get", name], capture_output=True, text=True, check=False
    ).stdout.strip()


class Cloudflare:
    def __init__(self, token: str, check: bool) -> None:
        self.token = token
        self.check = check
        self.changes: list[str] = []

    def call(self, method: str, path: str, body: object | None = None) -> dict:
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
            with urllib.request.urlopen(req, timeout=30) as res:
                out = json.load(res)
        except urllib.error.HTTPError as error:
            out = json.load(error)
        if not out.get("success"):
            raise SystemExit(
                f"cloudflare {method} {path.split('?')[0]}: {json.dumps(out.get('errors'))[:300]}"
            )
        return out

    def get(self, path: str) -> list | dict:
        return self.call("GET", path)["result"]

    def change(self, what: str, method: str, path: str, body: object) -> dict | None:
        """Record a change and, unless checking, make it."""
        self.changes.append(what)
        if self.check:
            return None
        return self.call(method, path, body)["result"]


def ensure_routes(cf: Cloudflare, account: str, host: str) -> str:
    tunnels = cf.get(f"/accounts/{account}/cfd_tunnel?name={TUNNEL}&is_deleted=false")
    if len(tunnels) != 1:
        raise SystemExit(f"expected one tunnel named {TUNNEL}, found {len(tunnels)}")
    tunnel = tunnels[0]["id"]
    config = cf.get(f"/accounts/{account}/cfd_tunnel/{tunnel}/configurations")["config"]
    ingress = config.get("ingress") or []
    wanted = [
        {"hostname": host, "path": CONSOLE_PATHS, "service": CONSOLE},
        {"hostname": host, "service": DASHBOARD},
    ]
    mine = [
        {k: r[k] for k in ("hostname", "path", "service") if k in r}
        for r in ingress
        if r.get("hostname") == host
    ]
    if mine != wanted:
        others = [r for r in ingress if r.get("hostname") not in (host, None)]
        catch_all = [r for r in ingress if r.get("hostname") is None] or [
            {"service": "http_status:404"}
        ]
        # A rule with a path must come before the same hostname's rule without one.
        config["ingress"] = others + wanted + catch_all
        cf.change(
            f"tunnel {TUNNEL}: route {host} (/console -> console, the rest -> dashboard)",
            "PUT",
            f"/accounts/{account}/cfd_tunnel/{tunnel}/configurations",
            {"config": config},
        )
    return tunnel


def ensure_dns(cf: Cloudflare, host: str, tunnel: str) -> None:
    zone_name = ".".join(host.split(".")[-2:])
    zones = cf.get(f"/zones?name={zone_name}")
    if len(zones) != 1:
        raise SystemExit(f"zone {zone_name} not found")
    zone = zones[0]["id"]
    target = f"{tunnel}.cfargotunnel.com"
    records = cf.get(f"/zones/{zone}/dns_records?name={host}")
    if not records:
        cf.change(
            f"dns: {host} -> tunnel",
            "POST",
            f"/zones/{zone}/dns_records",
            {
                "type": "CNAME",
                "name": host,
                "content": target,
                "proxied": True,
                "comment": f"pm stack via tunnel {TUNNEL}",
            },
        )
    elif (
        len(records) != 1
        or records[0]["type"] != "CNAME"
        or records[0]["content"] != target
        or not records[0]["proxied"]
    ):
        raise SystemExit(
            f"dns: {host} already has records that are not this tunnel; not touching them"
        )


def ensure_idps(cf: Cloudflare, account: str) -> list[str]:
    idps = cf.get(f"/accounts/{account}/access/identity_providers")
    by_type = {i["type"]: i for i in idps}
    ids = []
    if "onetimepin" not in by_type:
        made = cf.change(
            "access: enable the one-time PIN sign-in",
            "POST",
            f"/accounts/{account}/access/identity_providers",
            {"name": "One-time PIN", "type": "onetimepin", "config": {}},
        )
        if made:
            ids.append(made["id"])
    else:
        ids.append(by_type["onetimepin"]["id"])
    client_id, client_secret = secret(GOOGLE_ID), secret(GOOGLE_SECRET)
    if "google" in by_type:
        ids.append(by_type["google"]["id"])
    elif client_id and client_secret:
        made = cf.change(
            "access: add Google sign-in",
            "POST",
            f"/accounts/{account}/access/identity_providers",
            {
                "name": "Google",
                "type": "google",
                "config": {"client_id": client_id, "client_secret": client_secret},
            },
        )
        if made:
            ids.append(made["id"])
    else:
        print(
            f"note: Google sign-in is not on yet: secrets {GOOGLE_ID} and {GOOGLE_SECRET} do not exist"
        )
    return ids


def ensure_app(
    cf: Cloudflare,
    account: str,
    apps: list,
    domain: str,
    name: str,
    decision: str,
    include: list,
    idps: list[str] | None,
) -> str | None:
    app = next(
        (
            a
            for a in apps
            if a.get("domain") == domain and a.get("type") == "self_hosted"
        ),
        None,
    )
    if app is None:
        body: dict = {
            "name": name,
            "domain": domain,
            "type": "self_hosted",
            "session_duration": "24h",
            "app_launcher_visible": decision == "allow",
        }
        if idps:
            body["allowed_idps"] = idps
        app = cf.change(
            f"access: application for {domain}",
            "POST",
            f"/accounts/{account}/access/apps",
            body,
        )
        if app is None:
            return None
    elif idps and sorted(app.get("allowed_idps") or []) != sorted(idps):
        app["allowed_idps"] = idps
        cf.change(
            f"access: sign-in methods for {domain}",
            "PUT",
            f"/accounts/{account}/access/apps/{app['id']}",
            app,
        )
    policies = cf.get(f"/accounts/{account}/access/apps/{app['id']}/policies")
    if not any(
        p.get("decision") == decision and p.get("include") == include for p in policies
    ):
        cf.change(
            f"access: {decision} policy on {domain}",
            "POST",
            f"/accounts/{account}/access/apps/{app['id']}/policies",
            {
                "name": f"{decision}: {name}",
                "decision": decision,
                "include": include,
                "precedence": len(policies) + 1,
            },
        )
    extra = [
        p
        for p in policies
        if not (p.get("decision") == decision and p.get("include") == include)
    ]
    if extra:
        raise SystemExit(
            f"access: {domain} carries {len(extra)} policy(ies) this script did not write; review them before going on"
        )
    return app.get("aud")


def ensure_env(
    path: pathlib.Path, wanted: dict[str, str], check: bool, changes: list[str]
) -> None:
    lines = path.read_text().splitlines() if path.exists() else []
    have = dict(
        line.split("=", 1) for line in lines if "=" in line and not line.startswith("#")
    )
    if all(have.get(k) == v for k, v in wanted.items()):
        return
    changes.append(
        f"console environment: {', '.join(k for k, v in wanted.items() if have.get(k) != v)}"
    )
    if check:
        return
    kept = [line for line in lines if line.split("=", 1)[0] not in wanted]
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    path.write_text("\n".join(kept + [f"{k}={v}" for k, v in wanted.items()]) + "\n")
    path.chmod(0o600)
    subprocess.run(
        ["systemctl", "--user", "restart", "pm-console.service"], check=False
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--host", default="pm.security.cab")
    parser.add_argument("--owner", default="drewrypope@gmail.com")
    args = parser.parse_args()

    token = secret("cloudflare.api-token-account-wide")
    if not token:
        raise SystemExit(
            "secret cloudflare.api-token-account-wide is not readable here"
        )
    cf = Cloudflare(token, args.check)
    accounts = cf.get("/accounts?per_page=5")
    if len(accounts) != 1:
        raise SystemExit(f"expected one account, found {len(accounts)}")
    account = accounts[0]["id"]
    team = cf.get(f"/accounts/{account}/access/organizations")["auth_domain"]

    tunnel = ensure_routes(cf, account, args.host)
    ensure_dns(cf, args.host, tunnel)
    idps = ensure_idps(cf, account)
    apps = cf.get(f"/accounts/{account}/access/apps?per_page=200")
    # The public paths first: until the main application exists nothing
    # routes here, and once it does these must already be open.
    for path in PUBLIC_PATHS:
        ensure_app(
            cf,
            account,
            apps,
            args.host + path,
            f"PM stack public file {path}",
            "bypass",
            [{"everyone": {}}],
            None,
        )
    aud = ensure_app(
        cf,
        account,
        apps,
        args.host,
        "PM stack",
        "allow",
        [{"email": {"email": args.owner}}],
        idps,
    )
    if aud:
        env = pathlib.Path.home() / ".local/state/pm-console/.env"
        ensure_env(
            env,
            {
                "PM_CONSOLE_ACCESS_TEAM_DOMAIN": team,
                "PM_CONSOLE_ACCESS_AUD": aud,
                "PM_CONSOLE_OWNER_EMAILS": args.owner,
            },
            args.check,
            cf.changes,
        )

    for change in cf.changes:
        print(("would change: " if args.check else "changed: ") + change)
    if not cf.changes:
        print(
            f"https://{args.host}/ is set up: dashboard at /, console at /console/, owner {args.owner}"
        )
    return 1 if args.check and cf.changes else 0


if __name__ == "__main__":
    sys.exit(main())
