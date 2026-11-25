#!/usr/bin/env python3
"""Offline identity gate for kaneo-cutover.sh; no network or file writes.

Argument contract: exactly SOURCE_FILE TARGET_FILE TARGET_HOST, validated
before reading files. Deliberately supports only explicit single-host URIs,
not general libpq conninfo. Diagnostics never include DSNs or credentials.
"""

import ipaddress
import re
import sys
from pathlib import Path
from urllib.parse import parse_qsl, unquote, urlsplit


def decode(value):
    if re.search(r"%(?![0-9a-fA-F]{2})", value):
        raise ValueError("invalid percent encoding")
    result = unquote(value, errors="strict")
    if "%" in result or any(ord(c) < 33 or ord(c) > 126 for c in result):
        raise ValueError("unsupported encoding or whitespace")
    return result


def host_identity(value):
    value = decode(value).lower().rstrip(".")
    if not value or "muse-dev2" in value:
        raise ValueError("retired or absent host")
    if any(value == suffix or value.endswith("." + suffix)
           for suffix in ("supabase.com", "supabase.co")):
        raise ValueError("Supabase endpoints are not cluster-postgres targets")
    try:
        return str(ipaddress.ip_address(value))
    except ValueError:
        if len(value) > 253 or not all(
            re.fullmatch(r"[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?", label)
            for label in value.split(".")
        ):
            raise ValueError("unsupported host (require one TCP hostname or IP)") from None
        return value


def identity(filename):
    text = Path(filename).read_text().rstrip("\n")
    decoded = decode(text)
    if "muse-dev2" in decoded.lower():
        raise ValueError("retired muse-dev2 identifier")
    uri = urlsplit(text)
    if uri.scheme.lower() not in ("postgres", "postgresql") or not uri.netloc:
        raise ValueError("require a postgres/postgresql URI with explicit host")
    if uri.fragment or "#" in text or "\\" in text:
        raise ValueError("unsupported URI fragment or escape")
    # Only connection options that cannot redirect the endpoint are supported.
    pairs = parse_qsl(uri.query, keep_blank_values=True, strict_parsing=True)
    allowed = {"sslmode", "connect_timeout", "application_name"}
    if any(key not in allowed or not val for key, val in pairs) or len({k for k, _ in pairs}) != len(pairs):
        raise ValueError("unsupported or duplicate URI query option")
    host = host_identity(uri.hostname or "")
    port = uri.port if uri.port is not None else 5432
    if not 1 <= port <= 65535:
        raise ValueError("invalid TCP port")
    database = decode(uri.path.removeprefix("/"))
    if not database or "/" in database:
        raise ValueError("require one explicit database name")
    database_lower = database.lower()
    if "muse-dev2" in database_lower or any(
        database_lower == suffix or database_lower.endswith("." + suffix)
        for suffix in ("supabase.com", "supabase.co")
    ):
        raise ValueError("retired database identifier")
    # No authority tricks (multiple @, comma-separated ports/hosts, empty port).
    authority = uri.netloc.rsplit("@", 1)[-1]
    if uri.netloc.count("@") > 1 or "," in authority or authority.endswith(":"):
        raise ValueError("ambiguous authority")
    return host, port, database


def main():
    if len(sys.argv) != 4 or any(not arg or arg.startswith("--") for arg in sys.argv[1:]):
        print("kaneo target preflight: require SOURCE_FILE TARGET_FILE TARGET_HOST", file=sys.stderr)
        return 2
    try:
        source = identity(sys.argv[1])
        target = identity(sys.argv[2])
        if target[0] != host_identity(sys.argv[3]):
            raise ValueError("target URI host differs from --target-host")
        if source == target:
            raise ValueError("source and target identify the same host/port/database")
    except (ValueError, OSError, UnicodeError):
        print("kaneo target preflight: rejected target identity or unsupported DSN; require distinct explicit TCP postgres URIs and matching cluster target host", file=sys.stderr)
        return 2
    print("kaneo-cutover: PREFLIGHT OK — distinct source and declared cluster-postgres target")
    return 0


if __name__ == "__main__":
    sys.exit(main())
