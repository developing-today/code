#!/usr/bin/env python3
"""Lab Ops Hub crawler container.

Tiny stdlib-only HTTP service. POST /crawl {"url": "..."} -> fetches the page
with a real HTTP client, extracts title / link count / readable text, returns
JSON in the same shape the Worker's fallback produces. Runs only on demand
(Worker starts it via getContainer, sleepAfter=2m kills it when idle), so it
burns container minutes only while crawling.
"""

import glob
import ipaddress
import json
import re
import socket
import ssl
import urllib.request
from html.parser import HTMLParser
from http.server import BaseHTTPRequestHandler, HTTPServer
from urllib.parse import urlparse

# Crawl-target policy (#441): the Worker validates before dispatching,
# and this service re-validates independently (defence in depth — it is
# the component that actually fetches). Only public http(s) targets:
# no other schemes, no localhost-style names, no non-global IP literals.
# Redirect targets are held to the same policy.
_BLOCKED_SUFFIXES = (".localhost", ".local", ".internal", ".lan", ".home.arpa")


def validate_crawl_url(raw):
    u = urlparse(raw)
    if u.scheme not in ("http", "https"):
        raise ValueError("only http: and https: URLs may be crawled")
    # A port that is present must parse as an integer in range: urlparse
    # only raises for it lazily (when .port is read, deep inside the
    # fetcher), so read it here and refuse up front — the Worker's
    # WHATWG URL parser rejects these spellings outright (#502).
    try:
        u.port
    except ValueError:
        raise ValueError("url has an invalid port") from None
    host = (u.hostname or "").lower()
    # A single trailing dot is the DNS root dot (the FQDN spelling of the
    # same host: "localhost." is "localhost", "127.0.0.1." is 127.0.0.1);
    # strip it before the checks so the dotted spelling cannot evade
    # them, matching the Worker's validateCrawlUrl (#466).
    if host.endswith("."):
        host = host[:-1]
    if not host:
        raise ValueError("url has no host")
    # An empty label (a second trailing dot, an interior "..", or a
    # leading dot) is not a valid DNS name at all, and the one-dot strip
    # above would let "localhost.." slip past the blocked-host checks as
    # "localhost." — refuse it instead of certifying it as a public DNS
    # name, matching the Worker's validator (#511). IPv6 literals
    # contain ":" and are judged as addresses below, not by label
    # structure.
    if ":" not in host and any(label == "" for label in host.split(".")):
        raise ValueError(f"refusing to crawl host with an empty label {host}")
    # A raw % in the host is a percent-encoded spelling (or an IPv6 zone
    # id): WHATWG URL parsing percent-decodes the host before judging it
    # — %6cocalhost is localhost — and rejects the rest outright, while
    # urlparse hands the spelling through verbatim and the checks below
    # would mistake it for a public DNS name (or ipaddress would accept
    # the zone id). Refuse every % host, fail-closed (#505).
    if "%" in host:
        raise ValueError(f"refusing to crawl percent-encoded host {host}")
    # A space or control character in the host is not a DNS name: WHATWG
    # rejects it outright, urlparse hands it through verbatim, and
    # http.client only raises for it deep inside the fetcher — the same
    # lazy-failure shape #502 fixed for ports. Refuse up front (#507).
    if any(ord(c) <= 0x20 for c in host):
        raise ValueError(f"refusing to crawl host with whitespace {host!r}")
    if host == "localhost" or host.endswith(_BLOCKED_SUFFIXES):
        raise ValueError(f"refusing to crawl local host {host}")
    try:
        ip = ipaddress.ip_address(host)
    except ValueError:
        ip = _inet_aton_ipv4(host)
        if ip is None:
            # A host whose final label is numeric is an IP literal in
            # the URL grammar (WHATWG "ends in a number"), not a DNS
            # name: no numeric TLDs exist. If it reached here it is an
            # IPv4 spelling nothing accepts (256.1.1.1, 1.2.3.4.5,
            # 4294967296) or a name like example.123 / example.0x7f —
            # refuse it instead of reclassifying it as a public DNS
            # name, matching the Worker's validator (#502).
            last_label = host.rsplit(".", 1)[-1]
            # The hex alternative allows zero digits: WHATWG's
            # IPv4-number parser returns 0 for a bare 0x/0X prefix, so
            # `example.0x` and the single-label `0x` (0.0.0.0) also end
            # in a number — refuse them too (#504). A trailing non-hex
            # character (0xg, 0x7fg) still fails the full match and
            # stays a DNS name, as in WHATWG.
            if re.fullmatch(r"0[xX][0-9a-fA-F]*|[0-9]+", last_label):
                raise ValueError(
                    f"refusing to crawl host {host}: numeric final label "
                    "but not a valid IPv4 address"
                )
            # A bracketed host occupied the IP-literal position
            # ([v1.fe80::], [v7.a] and other IPvFuture forms) but is not
            # a valid IPv6 literal — urlparse strips the brackets, so
            # look at the netloc. Fail closed instead of reclassifying
            # it as a public DNS name, matching the Worker's validator
            # (#507).
            if u.netloc.rsplit("@", 1)[-1].startswith("["):
                raise ValueError(
                    f"refusing to crawl host {host}: bracketed literal "
                    "but not a valid IPv6 address"
                )
            return raw  # a public DNS name; literal-IP policy does not apply
    embedded = _embedded_ipv4(ip)
    if embedded is not None:
        # Translation/mapped prefixes are judged by the IPv4 they embed,
        # not by ipaddress's wholesale verdict on the prefix (#472).
        # is_global alone passes multicast and the 192.88.99.0/24 6to4
        # relay anycast range, so refuse those explicitly (#476).
        if (
            not embedded.is_global
            or embedded.is_multicast
            or embedded in _IPV4_6TO4_RELAY_ANYCAST
        ):
            raise ValueError(
                f"refusing to crawl address {host} embedding non-public IPv4 {embedded}"
            )
    elif isinstance(ip, ipaddress.IPv4Address) and (
        ip.is_multicast or ip in _IPV4_6TO4_RELAY_ANYCAST
    ):
        # ipaddress reports multicast (224.0.0.0/4) and 192.88.99.0/24 as
        # is_global; neither is a public unicast crawl target (#476).
        raise ValueError(f"refusing to crawl non-public address {host}")
    elif isinstance(ip, ipaddress.IPv6Address) and ip not in _GLOBAL_UNICAST_V6:
        # ipaddress.is_global is True for any IPv6 address outside its
        # narrow private list — multicast (ff00::/8), site-local
        # (fec0::/10), SRv6 SIDs (5f00::/16) and the reserved space
        # outside global unicast all pass it (#474). Global unicast
        # lives in 2000::/3; require that first, as the Worker's
        # validateCrawlUrl does, then let is_global judge within it
        # (it knows the 2001::/23 special-purpose block).
        raise ValueError(f"refusing to crawl non-public address {host}")
    elif not ip.is_global:
        raise ValueError(f"refusing to crawl non-public address {host}")
    return raw


_GLOBAL_UNICAST_V6 = ipaddress.IPv6Network("2000::/3")


_NAT64_WELL_KNOWN = ipaddress.IPv6Network("64:ff9b::/96")

_IPV4_6TO4_RELAY_ANYCAST = ipaddress.IPv4Network("192.88.99.0/24")


def _embedded_ipv4(ip):
    # Prefixes that embed an IPv4 address are judged by that address, the
    # same policy as plain IPv4 (#472). ipaddress folds the IPv4-mapped
    # case into is_global, but its verdicts on the translation prefixes
    # are wholesale: the whole NAT64 well-known prefix 64:ff9b::/96 is
    # "global" even when it embeds 127.0.0.1, and the whole 6to4 range
    # 2002::/16 (IPv6Address.sixtofour) is "private" even when it embeds
    # a public address.
    if not isinstance(ip, ipaddress.IPv6Address):
        return None
    if ip.ipv4_mapped is not None:
        return ip.ipv4_mapped
    if ip.sixtofour is not None:
        return ip.sixtofour
    if ip in _NAT64_WELL_KNOWN:
        return ipaddress.IPv4Address(int(ip) & 0xFFFFFFFF)
    return None


def _inet_aton_ipv4(host):
    # ipaddress only accepts canonical dotted-quad, but the resolver the
    # fetcher uses (getaddrinfo, inet_aton semantics) also interprets the
    # short (127.1), integer (2130706433), hex (0x7f.1) and octal
    # (0177.0.0.1) spellings as IPv4 (#468). Judge those spellings by the
    # address they resolve to instead of mistaking them for DNS names.
    # Only hosts made solely of numeric labels reach inet_aton, so
    # ordinary DNS names are never affected.
    if not re.fullmatch(r"[0-9a-fA-FxX.]+", host) or not any(c.isdigit() for c in host):
        return None
    try:
        return ipaddress.IPv4Address(socket.inet_aton(host))
    except (OSError, ValueError):
        return None


class _ValidatingRedirectHandler(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        validate_crawl_url(newurl)
        return super().redirect_request(req, fp, code, msg, headers, newurl)


_OPENER = None  # built after CTX below (the HTTPS handler carries it)

# The Nix image's CA bundle lives at a store path; do not rely on the
# container runtime propagating the image's SSL_CERT_FILE env var —
# pass the CA file to the TLS context explicitly.
_ca = glob.glob("/nix/store/*cacert*/etc/ssl/certs/ca-bundle.crt")
CTX = ssl.create_default_context(cafile=_ca[0]) if _ca else ssl.create_default_context()
_OPENER = urllib.request.build_opener(
    _ValidatingRedirectHandler, urllib.request.HTTPSHandler(context=CTX)
)


class Extractor(HTMLParser):
    def __init__(self):
        super().__init__()
        self.title = ""
        self._in_title = False
        self.links = 0
        self.text = []
        self._skip = 0

    def handle_starttag(self, tag, attrs):
        if tag == "title":
            self._in_title = True
        elif tag == "a":
            self.links += 1
        elif tag in ("script", "style"):
            self._skip += 1

    def handle_endtag(self, tag):
        if tag == "title":
            self._in_title = False
        elif tag in ("script", "style") and self._skip:
            self._skip -= 1

    def handle_data(self, data):
        if self._in_title:
            self.title += data
        elif not self._skip:
            self.text.append(data)


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):  # keep stdout quiet
        pass

    def _send(self, code, payload):
        body = json.dumps(payload).encode()
        self.send_response(code)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        self._send(200, {"ok": True, "service": "labops-crawler"})

    def do_POST(self):
        if self.path != "/crawl":
            return self._send(404, {"error": "POST /crawl only"})
        try:
            length = int(self.headers.get("content-length", "0"))
            url = json.loads(self.rfile.read(length))["url"]
            try:
                validate_crawl_url(url)
            except ValueError as e:
                return self._send(400, {"error": str(e)})
            req = urllib.request.Request(
                url, headers={"User-Agent": "LabOpsHub-Crawler/0.1 (container)"}
            )
            # TLS uses the pinned CA bundle via the opener's HTTPS handler.
            with _OPENER.open(req, timeout=20) as res:
                raw = res.read(2_000_000)
                status = res.status
            html = raw.decode("utf-8", errors="replace")
            ex = Extractor()
            ex.feed(html)
            text = re.sub(r"\s+", " ", " ".join(ex.text)).strip()
            self._send(
                200,
                {
                    "title": ex.title.strip(),
                    "excerpt": text[:500],
                    "links": ex.links,
                    "status_code": status,
                    "text": text[:20000],
                },
            )
        except Exception as e:  # noqa: BLE001 - surface any crawl failure as JSON
            self._send(502, {"error": str(e)})


if __name__ == "__main__":
    HTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
