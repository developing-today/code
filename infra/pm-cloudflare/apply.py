#!/usr/bin/env python3
"""Apply the PM stack's Cloudflare edge configuration from the repository.

Reads state.json (this directory): the hostname, its tunnel route, the
Access application and policy, DNS, the edge Worker route, and the
Worker's secrets (names only; values come from the environment, named
by the state). Applies each through Cloudflare's API using the
workflow-owned cloudflare.py module, in gate-first order, recording
what changed. The laptop's expose.py separately manages only its named
tunnel's ingress to the local console.

This changes nothing on the laptop: no console environment file, no
systemd unit, and no remembered edge/edge-direct markers. The workflow
is the sole applier of Access, DNS, Worker routes and Worker settings.

Values never come from this directory: the owner address, the
Cloudflare token, the Google OAuth client and the Worker secret values
are read from the environment (in Actions, from the repository's
secrets). Nothing printed is a secret.

Usage: apply.py [--check|--apply] [--state PATH]
  --check (default)  report what would change and exit 1 if anything would;
                     changes nothing.
  --apply            make the changes.
"""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import sys

import cloudflare as expose


def read_state(path: pathlib.Path) -> dict:
    """The declared configuration. Refuses anything it does not know:
    an unknown key is a typo that would silently not apply."""
    try:
        state = json.loads(path.read_text())
    except (OSError, ValueError) as error:
        raise SystemExit(f"{path}: not readable as JSON: {error}")
    if not isinstance(state, dict):
        raise SystemExit(f"{path}: the state is a JSON object")
    known = {"about", "host", "tunnel", "console_port", "access", "dns", "worker"}
    unknown = sorted(set(state) - known)
    if unknown:
        raise SystemExit(f"{path}: unknown keys: {', '.join(unknown)}")
    for key in ("host", "tunnel"):
        if not isinstance(state.get(key), str) or not state[key]:
            raise SystemExit(f"{path}: {key} names a non-empty string")
    port = state.get("console_port")
    if not isinstance(port, int) or not 0 < port < 65536:
        raise SystemExit(f"{path}: console_port is a port number")
    access = state.get("access")
    if not isinstance(access, dict):
        raise SystemExit(f"{path}: access is an object")
    aknown = {
        "about",
        "session_duration",
        "cookie",
        "owner_email_env",
        "google_client_id_env",
        "google_client_secret_env",
        "public_paths",
    }
    aunknown = sorted(set(access) - aknown)
    if aunknown:
        raise SystemExit(f"{path}: access has unknown keys: {', '.join(aunknown)}")
    for key in ("owner_email_env", "google_client_id_env", "google_client_secret_env"):
        if not isinstance(access.get(key), str) or not access[key]:
            raise SystemExit(f"{path}: access.{key} names the environment variable")
    if not isinstance(access.get("public_paths"), list) or not access["public_paths"]:
        raise SystemExit(
            f"{path}: access.public_paths lists the paths that bypass sign-in"
        )
    worker = state.get("worker")
    if not isinstance(worker, dict):
        raise SystemExit(f"{path}: worker is an object")
    wknown = {
        "about",
        "name",
        "route",
        "direct",
        "github_token_env",
        "queue_token_env",
        "bridge_token_env",
        "broker_access_id_env",
        "broker_access_secret_env",
    }
    wunknown = sorted(set(worker) - wknown)
    if wunknown:
        raise SystemExit(f"{path}: worker has unknown keys: {', '.join(wunknown)}")
    if not isinstance(worker.get("name"), str) or not worker["name"]:
        raise SystemExit(f"{path}: worker.name names the Worker")
    if not isinstance(worker.get("route"), bool):
        raise SystemExit(f"{path}: worker.route is true or false")
    if worker.get("direct") not in ("on", "off"):
        raise SystemExit(f"{path}: worker.direct is on or off")
    for key in ("github_token_env", "queue_token_env", "bridge_token_env"):
        if not isinstance(worker.get(key), str) or not worker[key]:
            raise SystemExit(f"{path}: worker.{key} names the environment variable")
    # The queue transport watch's Access pair (#16242) is optional in
    # the state, like the Google pair: both names or neither.
    broker_keys = ("broker_access_id_env", "broker_access_secret_env")
    if bool(worker.get(broker_keys[0])) != bool(worker.get(broker_keys[1])):
        raise SystemExit(
            f"{path}: worker names both of the broker Access pair or neither"
        )
    for key in broker_keys:
        if key in worker and (not isinstance(worker.get(key), str) or not worker[key]):
            raise SystemExit(f"{path}: worker.{key} names the environment variable")
    return state


def env(name: str) -> str:
    return os.environ.get(name, "")


def ensure_worker_direct(
    cf: expose.Cloudflare,
    account: str,
    worker: dict,
    check: bool,
    refresh_tokens: bool = False,
) -> None:
    """The Worker's secrets, as declared: present when direct is on,
    gone when it is off. Values are never readable back, so presence
    is enforced, never content; a run that changes nothing uploads
    nothing. Unlike expose.py this keeps no marker file: the state
    file is the memory."""
    name = worker["name"]
    path = f"/accounts/{account}/workers/scripts/{name}/secrets"
    held = {s.get("name") for s in cf.get(path)}
    names = ("GITHUB_TOKEN", "QUEUE_TOKEN")
    # Optional riders: secrets that ride along when the environment
    # holds them — the bridge token, and the queue transport watch's
    # Cloudflare Access pair (#16242), with which the Worker probes
    # the queue broker itself. A rider the environment does not hold
    # is left alone in silence, as expose.py leaves a vault without
    # it; none is one of the three the mode stands on.
    riders: dict[str, str] = {}
    bridge = env(worker["bridge_token_env"])
    if bridge:
        riders["BRIDGE_TOKEN"] = bridge
    broker_id = env(worker.get("broker_access_id_env") or "")
    broker_secret = env(worker.get("broker_access_secret_env") or "")
    if broker_id and broker_secret:
        riders["QUEUE_BROKER_ACCESS_ID"] = broker_id
        riders["QUEUE_BROKER_ACCESS_SECRET"] = broker_secret
    riders_missing = {k: v for k, v in riders.items() if k not in held}
    if worker["direct"] == "off":
        # Removing the opt-in first disables actions before credentials are
        # removed, and presence alone can then reconcile a later re-enable.
        for secret in (
            "EDGE_DIRECT",
            *names,
            "BRIDGE_TOKEN",
            "QUEUE_BROKER_ACCESS_ID",
            "QUEUE_BROKER_ACCESS_SECRET",
        ):
            if secret in held:
                cf.change(
                    f"edge direct: take {secret} away from the Worker",
                    "DELETE",
                    f"{path}/{secret}",
                    None,
                )
        return
    if refresh_tokens:
        cf.changes.append("edge direct: refresh the Worker's repository tokens")
        if check:
            return
        values = {
            "GITHUB_TOKEN": env(worker["github_token_env"]),
            "QUEUE_TOKEN": env(worker["queue_token_env"]),
            "EDGE_DIRECT": "on",
        }
        for secret, value in values.items():
            cf.call("PUT", path, {"name": secret, "text": value, "type": "secret_text"})
        for secret, value in riders.items():
            cf.call(
                "PUT",
                path,
                {"name": secret, "text": value, "type": "secret_text"},
            )
        return
    lost = sorted(set([*names, "EDGE_DIRECT"]) - held)
    if lost:
        if set(lost) == {*names, "EDGE_DIRECT"}:
            # Nothing there: the same change expose.py reports on a first run.
            cf.changes.append(
                "edge direct: upload the GitHub and queue tokens and turn direct actions on"
            )
        else:
            cf.changes.append(
                f"edge direct: give the Worker back what it has lost ({', '.join(lost)})"
            )
        if check:
            return
        values = {
            "GITHUB_TOKEN": env(worker["github_token_env"]),
            "QUEUE_TOKEN": env(worker["queue_token_env"]),
            "EDGE_DIRECT": "on",
        }
        for secret in lost:
            cf.call(
                "PUT",
                path,
                {"name": secret, "text": values[secret], "type": "secret_text"},
            )
        # Riders ride along with the upload, saying nothing of their
        # own, as in expose.py.
        for secret, value in riders_missing.items():
            cf.call(
                "PUT",
                path,
                {"name": secret, "text": value, "type": "secret_text"},
            )
        return
    if riders_missing:
        if set(riders_missing) == {"BRIDGE_TOKEN"}:
            cf.changes.append("edge direct: give the Worker its bridge token")
        else:
            cf.changes.append(
                f"edge direct: give the Worker what it is missing ({', '.join(sorted(riders_missing))})"
            )
        if check:
            return
        for secret, value in riders_missing.items():
            cf.call(
                "PUT",
                path,
                {"name": secret, "text": value, "type": "secret_text"},
            )


def apply(state: dict, check: bool, refresh_tokens: bool = False) -> list[str]:
    token = env("PMCF_CLOUDFLARE_TOKEN")
    if not token:
        raise SystemExit("secret PMCF_CLOUDFLARE_TOKEN is not readable here")
    owner = env(state["access"]["owner_email_env"])
    if not owner:
        raise SystemExit(
            f"secret {state['access']['owner_email_env']} (the owner address) is not readable here"
        )
    # Every secret this run will need is checked here, before a single
    # call is made: a run that cannot finish changes nothing at all.
    worker = state["worker"]
    if worker["direct"] == "on":
        for key in ("github_token_env", "queue_token_env"):
            if not env(worker[key]):
                raise SystemExit(
                    f"secret {worker[key]} (a Worker token) is not readable here"
                )
    google_id = env(state["access"]["google_client_id_env"])
    google_secret = env(state["access"]["google_client_secret_env"])
    if bool(google_id) != bool(google_secret):
        raise SystemExit("Google sign-in needs both client credentials or neither")
    # The queue transport watch's Access pair (#16242), the same
    # both-or-neither rule, checked before anything is asked.
    broker_id = env(worker.get("broker_access_id_env") or "")
    broker_secret = env(worker.get("broker_access_secret_env") or "")
    if bool(broker_id) != bool(broker_secret):
        raise SystemExit(
            "the queue broker Access pair needs both its client id and its client secret or neither"
        )
    # An empty pair preserves any existing Google provider. Passing a tuple
    # prevents expose.py from trying the laptop's secret store in Actions.
    google = (google_id, google_secret)
    cf = expose.Cloudflare(token, check)
    accounts = cf.get("/accounts?per_page=5")
    if len(accounts) != 1:
        raise SystemExit(f"expected one account, found {len(accounts)}")
    account = accounts[0]["id"]
    host = state["host"]
    if state["tunnel"] != expose.TUNNEL:
        raise SystemExit(
            f"state tunnel is {state['tunnel']}; expose.py manages {expose.TUNNEL}: renaming the tunnel is its own change"
        )
    if state["worker"]["name"] != expose.WORKER:
        raise SystemExit(
            f"state Worker is {state['worker']['name']}; expose.py routes {expose.WORKER}: moving the hostname is its own change"
        )

    # Order matters, as in expose.py: the sign-in gate is put in place
    # first and the route last, so the origin is never reachable
    # without the gate in front of it.
    tunnel, tunnel_config = expose.find_tunnel(cf, account)
    zone, dns_needed = expose.dns_plan(cf, host, tunnel)
    console = f"http://localhost:{state['console_port']}"
    idps = expose.ensure_idps(cf, account, google)
    apps = cf.get(f"/accounts/{account}/access/apps?per_page=200")
    for bypass in state["access"]["public_paths"]:
        expose.ensure_app(
            cf,
            account,
            apps,
            host + bypass,
            f"PM stack public file {bypass}",
            "bypass",
            [{"everyone": {}}],
            None,
        )
    expose.ensure_app(
        cf,
        account,
        apps,
        host,
        "PM stack",
        "allow",
        [{"email": {"email": owner}}],
        idps,
        expose.APP_SETTINGS,
    )
    expose.ensure_routes(cf, account, host, tunnel, tunnel_config, console)
    expose.ensure_dns(cf, host, tunnel, zone, dns_needed)
    # Last: the hostname through the Worker only once it answers built
    # for it; otherwise straight to the host, as expose.py decides.
    edge = state["worker"]["route"] and expose.worker_ready(cf, account, host)
    if state["worker"]["route"] and not edge:
        print(
            f"note: the edge Worker {state['worker']['name']} is not deployed, does not answer, or is not built for {host}; the hostname goes straight to this host"
        )
    expose.ensure_edge_route(cf, zone, host, edge)
    ensure_worker_direct(cf, account, state["worker"], check, refresh_tokens)
    return cf.changes


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument(
        "--check",
        action="store_true",
        help="report what would change and exit 1 if anything would; changes nothing",
    )
    parser.add_argument("--apply", action="store_true", help="make the changes")
    parser.add_argument(
        "--refresh-tokens",
        action="store_true",
        help="upload current repository token values even when names already exist",
    )
    parser.add_argument(
        "--state", default=str(pathlib.Path(__file__).parent / "state.json")
    )
    args = parser.parse_args(argv)
    if args.check and args.apply:
        raise SystemExit("choose one of --check and --apply")
    state = read_state(pathlib.Path(args.state))
    if args.refresh_tokens and not args.apply:
        raise SystemExit("--refresh-tokens requires --apply")
    if args.refresh_tokens and state["worker"]["direct"] != "on":
        raise SystemExit("--refresh-tokens requires worker.direct to be on")
    changes = apply(state, check=not args.apply, refresh_tokens=args.refresh_tokens)
    for change in changes:
        print(("would change: " if not args.apply else "changed: ") + change)
    if not changes:
        print(
            f"{state['host']} is as declared: route, sign-in gate and Worker settings match the repository state"
        )
    return 1 if not args.apply and changes else 0


if __name__ == "__main__":
    sys.exit(main())
