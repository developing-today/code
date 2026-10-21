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
    host = (u.hostname or "").lower()
    if not host:
        raise ValueError("url has no host")
    if host == "localhost" or host.endswith(_BLOCKED_SUFFIXES):
        raise ValueError(f"refusing to crawl local host {host}")
    try:
        ip = ipaddress.ip_address(host)
    except ValueError:
        return raw  # a public DNS name; literal-IP policy does not apply
    if not ip.is_global:
        raise ValueError(f"refusing to crawl non-public address {host}")
    return raw


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
