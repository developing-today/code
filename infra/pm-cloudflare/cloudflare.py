"""Cloudflare API helpers for the repository-owned PM configuration applier.

The laptop script separately owns only its named tunnel ingress.
"""

from __future__ import annotations

import argparse

import contextlib

import fcntl

import json

import pathlib

import re

import subprocess

import sys

import time

import urllib.error

import urllib.request

API = "https://api.cloudflare.com/client/v4"

TUNNEL = "amd-6-pm-stack"

CONSOLE = "http://localhost:4872"

APP_SETTINGS = {
    "same_site_cookie_attribute": "lax",
    "http_only_cookie_attribute": True,
    "session_duration": "720h",
}

PUBLIC_PATHS = (
    "/.well-known/assetlinks.json",
    "/manifest.webmanifest",
    "/icons",
    "/console/device",
    "/console/edge",
)

WORKER = "pm-console-edge"

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


def find_tunnel(cf: Cloudflare, account: str) -> tuple[str, dict]:
    tunnels = cf.get(f"/accounts/{account}/cfd_tunnel?name={TUNNEL}&is_deleted=false")
    if len(tunnels) != 1:
        raise SystemExit(f"expected one tunnel named {TUNNEL}, found {len(tunnels)}")
    tunnel = tunnels[0]["id"]
    return tunnel, cf.get(f"/accounts/{account}/cfd_tunnel/{tunnel}/configurations")[
        "config"
    ]


def ensure_routes(
    cf: Cloudflare, account: str, host: str, tunnel: str, config: dict, console: str
) -> None:
    ingress = config.get("ingress") or []
    # One rule, no path: every path on the hostname goes to the console,
    # which is the only thing that decides what reaches the dashboard.
    wanted = [{"hostname": host, "service": console}]

    def plain(rule: dict) -> dict:
        return {k: rule[k] for k in ("hostname", "path", "service") if k in rule}

    # cloudflared takes the first rule that matches. This host's rule goes
    # ahead of every other rule, so a wildcard such as *.security.cab can
    # never take the host's traffic and send it somewhere with no gate.
    if [plain(r) for r in ingress[:1]] == wanted and all(
        r.get("hostname") != host for r in ingress[1:]
    ):
        return
    rest = [r for r in ingress if r.get("hostname") != host]
    if not rest or rest[-1].get("hostname") or rest[-1].get("path"):
        rest.append({"service": "http_status:404"})
    config["ingress"] = wanted + rest
    cf.change(
        f"tunnel {TUNNEL}: route {host} to the console",
        "PUT",
        f"/accounts/{account}/cfd_tunnel/{tunnel}/configurations",
        {"config": config},
    )


USER_AGENT = "pm-console-expose"

HEALTH_TRIES = 3

HEALTH_WAIT_SECONDS = 3


def fetch_json(url: str) -> dict | None:
    """One GET of a JSON document, or None if it cannot be had."""
    try:
        request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
        with urllib.request.urlopen(request, timeout=10) as res:
            out = json.load(res)
            return out if isinstance(out, dict) else None
    except (OSError, ValueError):
        return None


def worker_ready(cf: Cloudflare, account: str, host: str) -> bool:
    """True when the edge Worker is deployed, answers its health check as
    the edge, and is built for this hostname (it treats any other as
    having no host behind it). Until then the hostname keeps going
    straight to the host, so there is never a route to a Worker that is
    not there or would not serve it. One failed answer is not believed:
    it is asked again, so a passing fault does not take the hostname off
    the edge."""
    subdomain = cf.get(f"/accounts/{account}/workers/subdomain").get("subdomain")
    if not subdomain:
        return False
    for attempt in range(HEALTH_TRIES):
        if attempt:
            time.sleep(HEALTH_WAIT_SECONDS)
        health = fetch_json(f"https://{WORKER}.{subdomain}.workers.dev/console/health")
        if (
            health
            and health.get("ok") is True
            and health.get("edge") is True
            and health.get("host") == host
        ):
            return True
    return False


def ensure_edge_route(cf: Cloudflare, zone: str, host: str, wanted: bool) -> None:
    """The hostname's traffic goes through the edge Worker, or does not.
    A route on this hostname that names another Worker is not this
    script's, and stops the run."""
    pattern = f"{host}/*"
    routes = [
        r
        for r in cf.get(f"/zones/{zone}/workers/routes")
        if r.get("pattern", "").split("/")[0] == host
    ]
    foreign = [
        r for r in routes if r.get("pattern") != pattern or r.get("script") != WORKER
    ]
    if foreign:
        raise SystemExit(
            f"workers: {host} already has a route that is not {WORKER}'s; not touching it"
        )
    if wanted and not routes:
        cf.change(
            f"workers: route {pattern} through {WORKER}",
            "POST",
            f"/zones/{zone}/workers/routes",
            {"pattern": pattern, "script": WORKER},
        )
    if not wanted:
        for route in routes:
            cf.change(
                f"workers: take {pattern} off {WORKER}",
                "DELETE",
                f"/zones/{zone}/workers/routes/{route['id']}",
                None,
            )


def find_zone(cf: Cloudflare, host: str) -> str:
    """The zone that holds the host: the longest suffix of the name that
    is a zone in this account. The last two labels are not always it
    (example.co.uk, or a delegated dev.example.com)."""
    labels = host.split(".")
    for start in range(len(labels) - 1):
        name = ".".join(labels[start:])
        zones = cf.get(f"/zones?name={name}")
        if len(zones) == 1:
            return zones[0]["id"]
    raise SystemExit(f"dns: no zone in this account holds {host}")


def dns_plan(cf: Cloudflare, host: str, tunnel: str) -> tuple[str, bool]:
    """Read DNS for the host. Returns the zone and whether a record is
    needed; refuses if the host already points anywhere else."""
    zone = find_zone(cf, host)
    records = cf.get(f"/zones/{zone}/dns_records?name={host}")
    target = f"{tunnel}.cfargotunnel.com"
    if records and (
        len(records) != 1
        or records[0]["type"] != "CNAME"
        or records[0]["content"] != target
        or not records[0]["proxied"]
    ):
        raise SystemExit(
            f"dns: {host} already has records that are not this tunnel; not touching them"
        )
    return zone, not records


def ensure_dns(cf: Cloudflare, host: str, tunnel: str, zone: str, needed: bool) -> None:
    if needed:
        cf.change(
            f"dns: {host} -> tunnel",
            "POST",
            f"/zones/{zone}/dns_records",
            {
                "type": "CNAME",
                "name": host,
                "content": f"{tunnel}.cfargotunnel.com",
                "proxied": True,
                "comment": f"pm stack via tunnel {TUNNEL}",
            },
        )


def ensure_idps(
    cf: Cloudflare, account: str, google: tuple[str, str] | None = None
) -> list[str]:
    # In Actions the owner may omit Google credentials. Keep an existing
    # provider unchanged in that case; a half-pair is a configuration error.
    client_id, client_secret = google or (secret(GOOGLE_ID), secret(GOOGLE_SECRET))
    if bool(client_id) != bool(client_secret):
        raise SystemExit("Google sign-in needs both its client ID and client secret")
    has_google_client = bool(client_id and client_secret)
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
    if "google" in by_type:
        ids.append(by_type["google"]["id"])
        if has_google_client:
            # Already on, and a client pair is available: it is the one to
            # use from here on (a corrected or replaced client).
            cf.change(
                "access: give Google sign-in the client just supplied",
                "PUT",
                f"/accounts/{account}/access/identity_providers/{by_type['google']['id']}",
                {
                    "name": by_type["google"].get("name") or "Google",
                    "type": "google",
                    "config": {"client_id": client_id, "client_secret": client_secret},
                },
            )
    elif has_google_client:
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
    settings: dict | None = None,
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
        body.update(settings or {})
        app = cf.change(
            f"access: application for {domain}",
            "POST",
            f"/accounts/{account}/access/apps",
            body,
        )
        if app is None:
            return None
    else:
        stale = [k for k, v in (settings or {}).items() if app.get(k) != v]
        if idps and sorted(app.get("allowed_idps") or []) != sorted(idps):
            stale.append("allowed_idps")
        if stale:
            app.update(settings or {})
            if idps:
                app["allowed_idps"] = idps
            cf.change(
                f"access: settings for {domain} ({', '.join(sorted(stale))})",
                "PUT",
                f"/accounts/{account}/access/apps/{app['id']}",
                app,
            )
    policies = cf.get(f"/accounts/{account}/access/apps/{app['id']}/policies")
    # Exclude and require rules change who a policy lets in, so a policy
    # carrying either is not the one this script writes.
    mine = [
        p
        for p in policies
        if p.get("decision") == decision
        and p.get("include") == include
        and not p.get("exclude")
        and not p.get("require")
    ]
    if len(mine) != len(policies):
        raise SystemExit(
            f"access: {domain} carries {len(policies) - len(mine)} policy(ies) this script did not write; review them before going on"
        )
    if not mine:
        cf.change(
            f"access: {decision} policy on {domain}",
            "POST",
            f"/accounts/{account}/access/apps/{app['id']}/policies",
            {
                "name": f"{decision}: {name}",
                "decision": decision,
                "include": include,
                "precedence": 1,
            },
        )
    return app.get("aud")
