#!/usr/bin/env python3
"""Put the PM stack on a public hostname behind sign-in, or check that it is.

One hostname carries the dashboard (pm-aggregator) and the console
(pm-console). All of it is routed to the console, which serves /console
and passes every other path to the dashboard only for a verified owner.
The dashboard has no sign-in of its own, so sign-in is checked twice: by
Cloudflare Access at the edge, and by the console at the origin.

This script makes each of these true, changing only what is not already
so, and prints what it did:

1. A Cloudflare Access application covers the hostname, with one policy:
   the owner's address. Sign-in methods are the one-time PIN, plus Google
   when the secrets `google.access-sso.client-id` and
   `google.access-sso.client-secret` exist.
2. Five paths bypass sign-in at the edge. Three are files a phone
   fetches before anyone has signed in: the asset-links file, the
   install manifest and its icons. The fourth, /console/device, is where
   the Android app's home-screen widget presents a token of its own to
   read a short summary. The fifth, /console/edge, is where this host
   reports to the edge Worker, signed with the host's own key. The
   console serves exactly those files to an unsigned caller, checks that
   token and that signature itself, and answers nothing else.
3. The console's environment file names the Access issuer and the owner,
   and the console is running with it.
4. The tunnel routes the hostname to the console.
5. DNS points the hostname at the tunnel.
6. The hostname's traffic goes through the edge Worker (pm-console-edge,
   which serves the app from Cloudflare and reaches this host through
   the same tunnel), and the console is told where to report to it. This
   happens only once the Worker answers its health check; until then,
   and after `--edge off`, the hostname goes straight to this host.

The gate comes first and the routes last, so the origin is never
reachable without sign-in in front of it. If the console does not come
up with its sign-in settings, the run stops before the route.

Every secret is read by name through `fleet secrets get` and stays in
memory. Nothing printed is a secret.

Usage: expose.py [--check] [--edge on|off] [--host pm.security.cab] [--owner EMAIL]
  --check      report what would change and exit 1 if anything would.
  --edge off   take the hostname off the edge Worker; remembered by later
               runs (the timer's included) until --edge on.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import subprocess
import sys
import time
import urllib.error
import urllib.request

API = "https://api.cloudflare.com/client/v4"
TUNNEL = "amd-6-pm-stack"
CONSOLE = "http://localhost:4872"
UNIT = "pm-console.service"
# The sign-in cookie is not sent on another site's requests, and page
# scripts cannot read it. A sign-in lasts a month, so signing in is a
# rare event and not a daily one.
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
# The Worker that serves the app from the edge (pm-stack/console/edge).
# It is deployed from the repository; this script only decides whether
# the hostname's traffic goes through it.
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


def fetch_json(url: str) -> dict | None:
    """One GET of a JSON document, or None if it cannot be had."""
    try:
        with urllib.request.urlopen(url, timeout=10) as res:
            out = json.load(res)
            return out if isinstance(out, dict) else None
    except (OSError, ValueError):
        return None


def edge_choice(state: pathlib.Path, choice: str | None) -> bool:
    """Whether the hostname should go through the edge Worker. The choice
    is kept in a file, so a timer's run keeps what was last asked for;
    with no file the answer is yes."""
    marker = state / "edge"
    if choice is not None:
        state.mkdir(parents=True, exist_ok=True, mode=0o700)
        marker.write_text(choice + "\n")
    return not marker.exists() or marker.read_text().strip() != "off"


def worker_ready(cf: Cloudflare, account: str) -> bool:
    """True when the edge Worker is deployed and answers its health check
    as the edge. Until then the hostname keeps going straight to the
    host, so there is never a route to a Worker that is not there."""
    subdomain = cf.get(f"/accounts/{account}/workers/subdomain").get("subdomain")
    if not subdomain:
        return False
    health = fetch_json(f"https://{WORKER}.{subdomain}.workers.dev/console/health")
    return bool(health and health.get("ok") is True and health.get("edge") is True)


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


def env_values(path: pathlib.Path) -> dict[str, str]:
    """Read a systemd EnvironmentFile: KEY=value lines, comments, and one
    optional pair of quotes around a value."""
    values: dict[str, str] = {}
    if not path.exists():
        return values
    for raw in path.read_text().splitlines():
        line = raw.strip()
        if not line or line[0] in "#;" or "=" not in line:
            continue
        key, value = line.split("=", 1)
        value = value.strip()
        if len(value) >= 2 and value[0] == value[-1] and value[0] in "\"'":
            value = value[1:-1]
        values[key.strip()] = value
    return values


def console_origin(env: pathlib.Path) -> str:
    """Where the console listens, from its own environment file, so the
    tunnel never routes to a port the console is not on."""
    port = env_values(env).get("PM_CONSOLE_PORT") or CONSOLE.rsplit(":", 1)[1]
    if not port.isdigit() or not 0 < int(port) < 65536:
        raise SystemExit(f"{env}: PM_CONSOLE_PORT is not a port: {port!r}")
    return f"http://localhost:{port}"


def console_up(origin: str, seconds: int = 20) -> bool:
    """True once the console answers its health check."""
    for _ in range(seconds):
        try:
            with urllib.request.urlopen(f"{origin}/console/health", timeout=3) as res:
                if res.status == 200:
                    return True
        except OSError:
            pass
        time.sleep(1)
    return False


def ensure_console(
    path: pathlib.Path,
    wanted: dict[str, str | None],
    origin: str,
    check: bool,
    changes: list[str],
) -> None:
    """The console runs with the sign-in settings. A value of None means
    the setting must be absent. Raises, so nothing is routed, if the
    console cannot be brought up with them."""
    have = env_values(path)
    stale = [k for k, v in wanted.items() if have.get(k) != v]
    if stale:
        changes.append(f"console environment: {', '.join(stale)}")
    running = (
        subprocess.run(
            ["systemctl", "--user", "is-active", "--quiet", UNIT], check=False
        ).returncode
        == 0
    )
    if not stale and not running:
        changes.append(f"console service: {UNIT} is not running, start it")
    if check or (not stale and running):
        return
    if stale:
        lines = path.read_text().splitlines() if path.exists() else []
        kept = [line for line in lines if line.split("=", 1)[0].strip() not in wanted]
        path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        path.write_text(
            "\n".join(kept + [f"{k}={v}" for k, v in wanted.items() if v is not None])
            + "\n"
        )
        path.chmod(0o600)
    done = subprocess.run(
        ["systemctl", "--user", "restart", UNIT],
        check=False,
        capture_output=True,
        text=True,
    )
    if done.returncode != 0:
        raise SystemExit(
            f"{UNIT} did not restart ({(done.stderr or '').strip()[:200]}); the hostname is not routed"
        )
    if not console_up(origin):
        raise SystemExit(
            f"{UNIT} restarted but does not answer at {origin}/console/health; the hostname is not routed"
        )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--host", default="pm.security.cab")
    parser.add_argument("--owner", default="drewrypope@gmail.com")
    parser.add_argument(
        "--edge",
        choices=["on", "off"],
        help="send the hostname through the edge Worker, or straight to this host; "
        "the choice is kept for later runs (default: on, once the Worker answers)",
    )
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

    # Look before changing anything: a hostname that already points
    # somewhere else stops the run with nothing written.
    tunnel, tunnel_config = find_tunnel(cf, account)
    zone, dns_needed = dns_plan(cf, args.host, tunnel)
    env = pathlib.Path.home() / ".local/state/pm-console/.env"
    console = console_origin(env)
    # Through the edge only if that is wanted and the Worker is really
    # there; otherwise straight to this host, as before the edge existed.
    edge = edge_choice(env.parent, None if args.check else args.edge)
    if edge and not worker_ready(cf, account):
        edge = False
        print(
            f"note: the edge Worker {WORKER} is not deployed or does not answer; "
            "the hostname goes straight to this host"
        )

    # Order matters. The sign-in gate is put in place first and the route
    # last, so the origin is never reachable without the gate in front.
    idps = ensure_idps(cf, account)
    apps = cf.get(f"/accounts/{account}/access/apps?per_page=200")
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
        APP_SETTINGS,
    )
    if aud:
        ensure_console(
            env,
            {
                "PM_CONSOLE_ACCESS_TEAM_DOMAIN": team,
                "PM_CONSOLE_ACCESS_AUD": aud,
                "PM_CONSOLE_OWNER_EMAILS": args.owner,
                # Where this host reports to the edge; absent, it reports
                # to nobody.
                "PM_CONSOLE_EDGE_URL": f"https://{args.host}" if edge else None,
            },
            console,
            args.check,
            cf.changes,
        )
    if aud or args.check:
        ensure_routes(cf, account, args.host, tunnel, tunnel_config, console)
        ensure_dns(cf, args.host, tunnel, zone, dns_needed)
        # Last: by now the host is up, knows where the edge is, and is
        # reachable behind the hostname the Worker will sit on.
        ensure_edge_route(cf, zone, args.host, edge)
    else:
        raise SystemExit(
            "the Access application has no audience tag; not routing the hostname"
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
