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
   happens only once the Worker answers its health check and says it is
   built for this hostname; until then, and after `--edge off`, the
   hostname goes straight to this host.
7. The Worker is given the two tokens it needs to do the core itself,
   with this host connected or not (the repository automation token and
   the queue store token), and the switch that lets it use them. On
   unless `--edge-direct off` was asked for, which is remembered like
   `--edge`, turns the switch off and takes the two tokens away again.
   Nothing is uploaded on a run that changes nothing (unless the Worker
   has lost one of them, and then it gets them again). A run that was
   never told either way and cannot do it (a secret that is not readable
   here, Cloudflare refusing) says so and carries on: the rest of what
   this script keeps is not held up by it, and the next run tries again.
   The tokens go through Cloudflare's API with the token this script
   already holds, so nothing has to be installed here for it. The Worker
   says in /console/health whether the switch is on and whether each
   token is there.

The gate comes first and the routes last, so the origin is never
reachable without sign-in in front of it. If the console does not come
up with its sign-in settings, the run stops before the route.

Every secret is read by name through `fleet secrets get` and stays in
memory. Nothing printed is a secret.

Usage: expose.py [--check] [--edge on|off] [--edge-direct on|off]
                 [--host pm.security.cab] [--owner ADDRESS]
                 [--google-stdin] [--status]
  --owner      whose sign-in is let in. Needed the first time; after that
               the console's environment file remembers it. The address
               is not written in this script.
  --check      report what would change and exit 1 if anything would;
               with --edge, what that choice would change, keeping nothing.
  --google-stdin
               turn Google sign-in on with an OAuth client read from
               standard input: the client ID on the first line, the
               client secret on the second. They go to Cloudflare and
               nowhere else: not to a file, a log or the output.
  --status     print which sign-in methods are on, as JSON, and change
               nothing.
  --edge off   take the hostname off the edge Worker; remembered by later
               runs (the timer's included) until --edge on.
  --edge-direct on|off
               whether the edge Worker does the core itself: on uploads
               the GitHub and queue tokens and turns the switch on; off
               turns it off and takes the tokens away. Remembered by
               later runs. On when never said.
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
# Bitwarden names for the two tokens the edge Worker does the core
# with: the repository automation token (the same name the
# queue-transport-alert deploy uses), and the queue store token, whose
# name the console's environment file may override.
GITHUB_TOKEN_SECRET = "github.developing-today-code-automation"
QUEUE_TOKEN_DEFAULT = "turso.fleet-queue-rw-token"


@contextlib.contextmanager
def one_at_a_time(state: pathlib.Path):
    """Hold this for a run that may change something: a second run waits
    for the first. The timer's run and one started from the app would
    otherwise each read what is so, then both act on it (one deleting a
    token the other has just uploaded)."""
    state.mkdir(parents=True, exist_ok=True, mode=0o700)
    with open(state / "expose.lock", "w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        try:
            yield
        finally:
            fcntl.flock(lock, fcntl.LOCK_UN)


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


# Cloudflare answers 403 to a client that calls itself Python-urllib. The
# name sent is this program's and nothing more: no address, no contact.
USER_AGENT = "pm-console-expose"
# A health check that fails once is asked again before the hostname is
# taken off the edge over it.
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


def edge_choice(state: pathlib.Path, choice: str | None, keep: bool) -> bool:
    """Whether the hostname should go through the edge Worker. A choice
    given now decides, and with `keep` it is written to a file, so a
    timer's run keeps what was last asked for. With no choice given the
    file decides, and with no file the answer is yes."""
    marker = state / "edge"
    if choice is not None:
        if keep:
            state.mkdir(parents=True, exist_ok=True, mode=0o700)
            marker.write_text(choice + "\n")
        return choice != "off"
    return not marker.exists() or marker.read_text().strip() != "off"


def direct_choice(state: pathlib.Path, choice: str | None, keep: bool) -> bool:
    """Whether the edge Worker does the core itself. A choice given now
    decides, and with `keep` it is written to a file, so a timer's run
    keeps what was last asked for. With no choice given the file decides,
    and with no file the answer is yes: Cloudflare carrying the core is
    how this is set up, and `off` is the choice that has to be made."""
    marker = state / "edge-direct"
    if choice is not None:
        if keep:
            state.mkdir(parents=True, exist_ok=True, mode=0o700)
            marker.write_text(choice + "\n")
        return choice != "off"
    return not marker.exists() or marker.read_text().strip() != "off"


def ensure_edge_direct(
    cf: Cloudflare,
    account: str,
    env: pathlib.Path,
    choice: str | None,
    check: bool,
) -> None:
    """Upload the two tokens the edge Worker acts with, and its switch,
    or turn the switch off and take the tokens away again.

    A secret is read by name through `fleet secrets get` and sent to
    Cloudflare's API as a Worker secret; it is never printed, written to
    a file or put on a command line.

    The choice is remembered only once the Worker has it. Turning it on
    either gets all three there or leaves none: an attempt that fails
    part-way takes away what it uploaded. Turning it off puts the opt-in
    off first, so the Worker stops at once, and then removes the tokens;
    if that stops part-way the mode is still off and a later run
    finishes. A run that changes nothing uploads nothing, with one
    exception: a mode that is on gets back whatever the Worker has lost.

    On is what a run does when it was never told either way. Such a run
    that cannot do it says why and returns: it was not asked for this,
    so it does not fail over it, and the next run tries again. A run
    that was asked (`--edge-direct on`), or whose mode is already on,
    fails as before."""
    state = env.parent
    # Read what is remembered before the choice overwrites it. Nothing is
    # written here: the marker moves only once the Worker has the change.
    marker = state / "edge-direct"
    current = marker.exists() and marker.read_text().strip() == "on"
    wanted = direct_choice(state, choice, keep=False)
    # Straight to Cloudflare's API, with the token this script already
    # holds: nothing to install on this host, and the value travels in
    # the request body, never on a command line.
    secrets_path = f"/accounts/{account}/workers/scripts/{WORKER}/secrets"
    all_three = ("GITHUB_TOKEN", "QUEUE_TOKEN", "EDGE_DIRECT")

    def put(name: str, value: str) -> None:
        cf.call(
            "PUT", secrets_path, {"name": name, "text": value, "type": "secret_text"}
        )

    def held() -> set[str]:
        return {s.get("name") for s in cf.get(secrets_path)}

    def remember(mode: str) -> None:
        state.mkdir(parents=True, exist_ok=True, mode=0o700)
        marker.write_text(mode + "\n")

    def take_tokens_away() -> None:
        """Off means the Worker holds nothing it could act with."""
        names = held()
        for name in ("GITHUB_TOKEN", "QUEUE_TOKEN"):
            if name in names:
                cf.change(
                    f"edge direct: take {name} away from the Worker",
                    "DELETE",
                    f"{secrets_path}/{name}",
                    None,
                )

    def upload() -> None:
        github = secret(GITHUB_TOKEN_SECRET)
        queue_name = (
            env_values(env).get("PM_CONSOLE_QUEUE_TOKEN_NAME") or QUEUE_TOKEN_DEFAULT
        )
        queue = secret(queue_name)
        if not github:
            raise SystemExit(f"secret {GITHUB_TOKEN_SECRET} is not readable here")
        if not queue:
            raise SystemExit(f"secret {queue_name} is not readable here")
        try:
            put("GITHUB_TOKEN", github)
            put("QUEUE_TOKEN", queue)
            put("EDGE_DIRECT", "on")
        except SystemExit:
            # Part-way is not on. What did get there is taken away again,
            # so a failed attempt leaves the Worker holding nothing.
            if not current:
                try:
                    take_tokens_away()
                except SystemExit:
                    pass
            raise

    if wanted and current:
        # On already. A timer's run uploads nothing, unless the Worker
        # has lost one of the three, and then it gets them again.
        try:
            lost = sorted(set(all_three) - held())
        except SystemExit as error:
            print(f"note: could not see what the Worker holds: {error}")
            return
        if lost:
            cf.changes.append(
                f"edge direct: give the Worker back what it has lost ({', '.join(lost)})"
            )
            if not check:
                upload()
        return
    if wanted and choice is None:
        # Never told either way: on is how this is set up. Whether it can
        # be done is found out first, so that a run that cannot says
        # nothing will change, and does not fail over what it was not
        # asked for.
        queue_name = (
            env_values(env).get("PM_CONSOLE_QUEUE_TOKEN_NAME") or QUEUE_TOKEN_DEFAULT
        )
        missing = [
            name for name in (GITHUB_TOKEN_SECRET, queue_name) if not secret(name)
        ]
        if missing:
            print(
                "note: the edge Worker was not given its tokens: "
                f"{' and '.join(missing)} not readable here; the next run tries again"
            )
            return
        before = len(cf.changes)
        cf.changes.append(
            "edge direct: upload the GitHub and queue tokens and turn direct actions on"
        )
        if check:
            return
        try:
            upload()
        except SystemExit as error:
            # What got there was taken away again (upload does that), so
            # nothing has changed, and nothing is listed as changed.
            del cf.changes[before:]
            print(
                f"note: the edge Worker was not given its tokens: {error}; "
                "the next run tries again"
            )
            return
        remember("on")
        return
    if wanted:
        cf.changes.append(
            "edge direct: upload the GitHub and queue tokens and turn direct actions on"
        )
        if check:
            return
        upload()
        # The Worker has all three now, so the choice is remembered; a
        # failure above raises before this line and leaves no marker.
        remember("on")
        return
    if current:
        cf.changes.append("edge direct: turn direct actions off")
        if check:
            return
        # The opt-in goes first, so the Worker stops acting at once, and
        # the mode is off from here whatever happens to the tidying.
        put("EDGE_DIRECT", "off")
        remember("off")
        take_tokens_away()
        return
    if choice == "off":
        # Asked for off while already off, or after an attempt to turn it
        # on that never finished: make sure nothing was left behind.
        take_tokens_away()
        return
    if marker.exists():
        # Off, after having been on, and nobody asking: a token left by a
        # run that stopped part-way is taken away now. This is tidying;
        # it never fails a run that changes nothing else.
        try:
            take_tokens_away()
        except SystemExit as error:
            print(f"note: could not look for tokens left on the Worker: {error}")


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


def read_google(stream) -> tuple[str, str]:
    """A Google OAuth client from standard input: the client ID on the
    first line and the client secret on the second. Refused, before
    anything is asked of Cloudflare, unless it looks like one. Neither
    value is ever printed, in a refusal or otherwise."""
    lines = [line.strip() for line in stream.read().splitlines() if line.strip()]
    if len(lines) != 2:
        raise SystemExit(
            "--google-stdin: expected two lines, the client ID and then the client secret"
        )
    client_id, client_secret = lines
    if not re.fullmatch(r"[0-9]+-[0-9a-z]+\.apps\.googleusercontent\.com", client_id):
        raise SystemExit(
            "--google-stdin: the first line is not a Google OAuth client ID "
            "(one ends in .apps.googleusercontent.com)"
        )
    if not re.fullmatch(r"[\x21-\x7e]{8,200}", client_secret):
        raise SystemExit("--google-stdin: the second line is not a client secret")
    return client_id, client_secret


def owner_address(given: str | None, env: pathlib.Path) -> str:
    """Whose sign-in is let in. It is named with --owner the first time;
    after that the console's own environment file remembers it, so the
    timer's runs need not be told. It is not written in this script."""
    remembered = env_values(env).get("PM_CONSOLE_OWNER_EMAILS", "").split(",")[0]
    owner = (given or remembered).strip().lower()
    if not re.fullmatch(r"[^@\s,]+@[^@\s,]+\.[^@\s,]+", owner):
        raise SystemExit(
            f"say whose sign-in is let in: --owner ADDRESS (after the first run, {env} remembers it)"
        )
    return owner


def ensure_idps(
    cf: Cloudflare, account: str, google: tuple[str, str] | None = None
) -> list[str]:
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
    # A client handed in now is used; otherwise the two secrets, if they exist.
    client_id, client_secret = google or (secret(GOOGLE_ID), secret(GOOGLE_SECRET))
    if "google" in by_type:
        ids.append(by_type["google"]["id"])
        if google:
            # Already on, and a client was handed in: it is the one to
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
    # A run that only looks needs no turn. One that may change something
    # waits for any other such run to finish first.
    if {"--status", "--check"} & set(sys.argv[1:]):
        return configure()
    with one_at_a_time(pathlib.Path.home() / ".local/state/pm-console"):
        return configure()


def configure() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--host", default="pm.security.cab")
    parser.add_argument(
        "--owner",
        help="whose sign-in is let in; needed the first time, remembered after",
    )
    parser.add_argument(
        "--google-stdin",
        action="store_true",
        help="turn Google sign-in on with the OAuth client ID and secret read "
        "from standard input, one a line",
    )
    parser.add_argument(
        "--status",
        action="store_true",
        help="print which sign-in methods are on, as JSON, and change nothing",
    )
    parser.add_argument(
        "--edge",
        choices=["on", "off"],
        help="send the hostname through the edge Worker, or straight to this host; "
        "the choice is kept for later runs (default: on, once the Worker answers)",
    )
    parser.add_argument(
        "--edge-direct",
        choices=["on", "off"],
        help="whether the edge Worker does the core itself: on uploads its "
        "GitHub and queue tokens and turns the switch on, off takes them "
        "away; the choice is kept for later runs (default: on)",
    )
    args = parser.parse_args()
    env = pathlib.Path.home() / ".local/state/pm-console/.env"
    # Refused here, before anything is asked of Cloudflare.
    google = read_google(sys.stdin) if args.google_stdin else None
    owner = None if args.status else owner_address(args.owner, env)

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
    if args.status:
        kinds = {
            i["type"] for i in cf.get(f"/accounts/{account}/access/identity_providers")
        }
        print(
            json.dumps(
                {
                    "google": "google" in kinds,
                    "pin": "onetimepin" in kinds,
                    "team": team,
                }
            )
        )
        return 0

    # Look before changing anything: a hostname that already points
    # somewhere else stops the run with nothing written.
    tunnel, tunnel_config = find_tunnel(cf, account)
    zone, dns_needed = dns_plan(cf, args.host, tunnel)
    console = console_origin(env)
    # Through the edge only if that is wanted and the Worker is really
    # there; otherwise straight to this host, as before the edge existed.
    # --check previews a choice given with it, and keeps nothing.
    edge = edge_choice(env.parent, args.edge, keep=not args.check)
    if edge and not worker_ready(cf, account, args.host):
        edge = False
        print(
            f"note: the edge Worker {WORKER} is not deployed, does not answer, or is "
            f"not built for {args.host}; the hostname goes straight to this host"
        )

    # Order matters. The sign-in gate is put in place first and the route
    # last, so the origin is never reachable without the gate in front.
    idps = ensure_idps(cf, account, google)
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
        [{"email": {"email": owner}}],
        idps,
        APP_SETTINGS,
    )
    if aud:
        ensure_console(
            env,
            {
                "PM_CONSOLE_ACCESS_TEAM_DOMAIN": team,
                "PM_CONSOLE_ACCESS_AUD": aud,
                "PM_CONSOLE_OWNER_EMAILS": owner,
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
        # The switch is independent of the route: it only decides whether
        # the Worker already deployed does the core itself.
        ensure_edge_direct(cf, account, env, args.edge_direct, args.check)
    else:
        raise SystemExit(
            "the Access application has no audience tag; not routing the hostname"
        )

    for change in cf.changes:
        print(("would change: " if args.check else "changed: ") + change)
    if not cf.changes:
        print(
            f"https://{args.host}/ is set up: dashboard at /, console at /console/, owner {owner}"
        )
    return 1 if args.check and cf.changes else 0


if __name__ == "__main__":
    sys.exit(main())
