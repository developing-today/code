#!/usr/bin/env python3
"""DNS cold-start publication and resolution for fleet public doors (#14717).

Transport & pooling design §1.2 (doc/2026-11-23T21-10-36Z_design_fleet-
transport-pooling): an agent with nothing configured needs one rendezvous
that works from bare internet — DNS under our own zones publishing the
stable public doors only (commander-pattern endpoints). DNS carries
offers, never secrets, and is never a liveness source.

This module is both halves of that discipline:

- Publication (`publish_records`): derives TXT records from the #2179
  machine registry (doc/fleet-machine-registry.yaml, offers from #14714).
  An offer is publishable only when it is a public door: its endpoint
  carries an `https` URL on a public hostname. Tailnet addresses, SSH
  targets, iroh NodeIds, loopback and `.ts.net` names are never
  published. A record carries exactly the machine id, the offer name,
  its kind, the door URL and, when the endpoint declares one, its port.
  Credential names, secret class names and the health block are never
  copied; as defence in depth the serialized records are scanned for
  every credential/secret name in the registry and publication fails
  rather than emit one.
- Resolution (`resolve_candidates`): parses those TXT values back into
  cold-start candidates. A candidate is an offer to *try*, nothing more:
  it carries `liveness: False` by construction, and record keys that
  claim liveness or carry credential material (`token`, `secret`,
  `bearer`, `credential`, `password`, `health`, `last_seen`, ...) are
  rejected, as are non-https URLs and non-public hosts.

CLI (strict argument contract, #3352 style):
  fleet-dns-cold-start.py publish REGISTRY [--zone ZONE]
  fleet-dns-cold-start.py resolve RECORDS-FILE [--zone ZONE]
`resolve` reads one TXT value per line (a leading record name and TTL
are tolerated, as in `dig +short TXT` or a zone-file line).
Requires PyYAML.
"""

import argparse
import ipaddress
import re
import sys
from urllib.parse import urlparse

import yaml

DEFAULT_ZONE = "security.cab"
RECORD_PREFIX = "_doors"
RECORD_VERSION = "fleet-door1"
# Keys a DNS record must never carry: credential material by any name,
# and liveness claims (DNS is never a liveness source, §1.2).
FORBIDDEN_KEYS = {
    "credential", "credentials", "token", "secret", "bearer", "password",
    "key", "auth", "health", "last_seen", "status", "alive", "liveness",
}
PUBLIC_KIND_URL = re.compile(r"^https://[^/@?#\s]+(/[^?#\s]*)?$")


def _public_https_url(endpoint):
    """Return the endpoint's URL iff it is a public https door, else None."""
    url = endpoint.get("url") if isinstance(endpoint, dict) else None
    if not isinstance(url, str) or not PUBLIC_KIND_URL.match(url):
        return None
    host = urlparse(url).hostname or ""
    if host in ("localhost",) or host.endswith((".local", ".ts.net", ".internal")):
        return None
    try:
        ip = ipaddress.ip_address(host)
    except ValueError:
        return url  # a public DNS name
    # Only a globally routable address is a public door. Checking
    # is_private/is_loopback/is_link_local is not enough: the shared
    # CGNAT range 100.64.0.0/10 (the tailnet address range) is neither
    # private nor global on Python >= 3.13, so it slipped through as
    # "public" there. is_global excludes every non-public range —
    # private, loopback, link-local, shared, reserved — on all versions.
    return url if ip.is_global else None


def publish_records(doc, zone=DEFAULT_ZONE):
    """Derive the DNS TXT publication (list of (name, value)) from a registry.

    Raises ValueError if a would-be record contains any credential or
    secret name from the registry (fail closed, never publish it).
    """
    if not isinstance(doc, dict) or not isinstance(doc.get("machines"), list):
        raise ValueError("registry must be a mapping with a `machines` list")
    forbidden = set()
    for rec in doc["machines"]:
        if not isinstance(rec, dict):
            continue
        forbidden.update(rec.get("secrets") or [])
        for offer in rec.get("transports") or []:
            if isinstance(offer, dict):
                forbidden.update(offer.get("credentials") or [])
    records = []
    for rec in doc["machines"]:
        if not isinstance(rec, dict) or rec.get("state") != "active":
            continue
        for offer in rec.get("transports") or []:
            if not isinstance(offer, dict):
                continue
            url = _public_https_url(offer.get("endpoint") or {})
            if url is None:
                continue  # not a public door: never published
            parts = [
                f"v={RECORD_VERSION}",
                f"machine={rec['id']}",
                f"offer={offer['name']}",
                f"kind={offer['kind']}",
                f"url={url}",
            ]
            port = (offer.get("endpoint") or {}).get("port")
            if isinstance(port, int):
                parts.append(f"port={port}")
            value = "; ".join(parts)
            leaked = [name for name in forbidden if name and name in value]
            if leaked:
                raise ValueError(
                    f"refusing to publish {offer['name']!r}: record would "
                    f"carry credential/secret name(s) {sorted(leaked)}"
                )
            name = f"{RECORD_PREFIX}.{rec['id']}.fleet.{zone}"
            records.append((name, value))
    return sorted(records)


def format_zone(records, zone=DEFAULT_ZONE):
    lines = [
        "; Fleet public-door cold-start records (#14717, design §1.2).",
        "; GENERATED by scripts/fleet-dns-cold-start.py publish from",
        "; doc/fleet-machine-registry.yaml — do not hand-edit.",
        "; Offers only: no credentials, no secrets, no liveness.",
    ]
    for name, value in records:
        lines.append(f'{name}. 300 IN TXT "{value}"')
    return "\n".join(lines) + "\n"


def parse_record_value(value):
    """Parse one TXT value into a cold-start candidate, or raise ValueError."""
    fields = {}
    for part in value.strip().strip('"').split(";"):
        part = part.strip()
        if not part:
            continue
        if "=" not in part:
            raise ValueError(f"malformed record field {part!r}")
        key, val = part.split("=", 1)
        key = key.strip().lower()
        if key in FORBIDDEN_KEYS:
            raise ValueError(
                f"record carries forbidden key {key!r} (DNS carries offers, "
                "never secrets, and is never a liveness source)"
            )
        fields[key] = val.strip()
    if fields.get("v") != RECORD_VERSION:
        raise ValueError(f"unsupported record version {fields.get('v')!r}")
    for need in ("machine", "offer", "kind", "url"):
        if not fields.get(need):
            raise ValueError(f"record is missing {need!r}")
    if _public_https_url({"url": fields["url"]}) is None:
        raise ValueError(f"record url {fields['url']!r} is not a public https door")
    candidate = {
        "machine": fields["machine"],
        "offer": fields["offer"],
        "kind": fields["kind"],
        "url": fields["url"],
        "source": "dns",
        # By construction: a DNS record says a door was published, never
        # that it is alive. Liveness is a pool/watchdog fact (§1.2/§3).
        "liveness": False,
    }
    if "port" in fields:
        candidate["port"] = int(fields["port"])
    return candidate


def resolve_candidates(lines):
    """Resolve TXT lines (dig/zone-file shaped) to cold-start candidates."""
    candidates = []
    for line in lines:
        line = line.strip()
        if not line or line.startswith(";") or "fleet-door1" not in line:
            continue
        value = line[line.index("v=fleet-door1"):]
        candidates.append(parse_record_value(value.rstrip('"; ')))
    return sorted(candidates, key=lambda c: (c["machine"], c["offer"]))


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="cmd", required=True)
    pub = sub.add_parser("publish", help="registry -> DNS zone records")
    pub.add_argument("registry")
    pub.add_argument("--zone", default=DEFAULT_ZONE)
    res = sub.add_parser("resolve", help="TXT records file -> candidates")
    res.add_argument("records")
    res.add_argument("--zone", default=DEFAULT_ZONE)
    args = parser.parse_args(argv)
    try:
        if args.cmd == "publish":
            with open(args.registry, encoding="utf-8") as fh:
                doc = yaml.safe_load(fh)
            sys.stdout.write(format_zone(publish_records(doc, args.zone), args.zone))
        else:
            with open(args.records, encoding="utf-8") as fh:
                for cand in resolve_candidates(fh):
                    port = f" port={cand['port']}" if "port" in cand else ""
                    print(
                        f"candidate: machine={cand['machine']} offer={cand['offer']} "
                        f"kind={cand['kind']} url={cand['url']}{port} "
                        "source=dns liveness=false"
                    )
    except (OSError, ValueError, yaml.YAMLError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
