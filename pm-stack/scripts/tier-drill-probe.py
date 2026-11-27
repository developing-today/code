#!/usr/bin/env python3
"""Read-only HTTP observations for CF-12 (#14952), NOT an outage controller.

Argument contract: --base-url ORIGIN --path PATH (repeatable), optional
--timeout SECONDS (0 < value <= 120); no positionals/abbreviations. Validate
all arguments before network access. GET only; stdout JSON, no file writes.
An HTTP success is not proof of surface correctness or an executed drill.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import math
import time
from urllib.error import HTTPError, URLError
from urllib.parse import urlsplit
from urllib.request import build_opener, Request


def timestamp():
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def arguments():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--base-url", required=True)
    parser.add_argument("--path", action="append", required=True)
    parser.add_argument("--timeout", type=float, default=10)
    args = parser.parse_args()
    base = urlsplit(args.base_url)
    try:
        valid_port = base.port
    except ValueError:
        parser.error("--base-url has an invalid port")
    if (base.scheme not in ("http", "https") or not base.hostname
            or base.username is not None or base.password is not None
            or base.path not in ("", "/") or base.query or base.fragment):
        parser.error("--base-url must be an http(s) origin without credentials, path, query or fragment")
    if not math.isfinite(args.timeout) or not 0 < args.timeout <= 120:
        parser.error("--timeout must be finite and between 0 (exclusive) and 120 seconds")
    for path in args.path:
        if (not path.startswith("/") or path.startswith("//") or "#" in path
                or "\\" in path or any(ord(c) <= 32 or ord(c) == 127 for c in path)):
            parser.error("--path must be an origin-relative path without whitespace, backslashes or fragments")
    args.base_url = args.base_url.rstrip("/")
    return args


def probe(opener, url, timeout):
    row = {"url": url, "startedAt": timestamp(), "status": None,
           "httpSuccess": False, "error": None}
    start = time.monotonic()
    try:
        try:
            response = opener.open(Request(url, method="GET"), timeout=timeout)
        except HTTPError as error:
            response = error
        with response:
            # Bound memory; the digest explicitly describes only the sampled bytes.
            body = response.read(65537)
            row.update(status=response.code, httpSuccess=200 <= response.code < 300,
                       location=response.headers.get("Location"),
                       contentType=response.headers.get("Content-Type"),
                       bodyBytes=min(len(body), 65536), bodyTruncated=len(body) > 65536,
                       bodySha256=hashlib.sha256(body[:65536]).hexdigest())
    except (OSError, URLError, ValueError) as error:
        row["error"] = type(error).__name__
    row.update(finishedAt=timestamp(), elapsedMs=round((time.monotonic() - start) * 1000))
    return row


def main():
    args = arguments()
    opener = build_opener()
    record = {"schemaVersion": 1, "kind": "tier-probe-observations", "provesDrill": False,
              "startedAt": timestamp(), "baseUrl": args.base_url}
    record["observations"] = [probe(opener, args.base_url + path, args.timeout) for path in args.path]
    record["finishedAt"] = timestamp()
    print(json.dumps(record, indent=2))
    return 0 if all(row["httpSuccess"] for row in record["observations"]) else 1


if __name__ == "__main__":
    raise SystemExit(main())
