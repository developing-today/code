#!/usr/bin/env python3
"""Reference rules for carrying id:// v2 URIs and their hints (D1 slice 4, #13886).

Stdlib only. Run with:  python3 -I carriers_reference.py self-test

Three parts:

  parse(uri)              slice 1 and 2 parsing, with hint kinds made extensible:
                          an unknown well-formed kind is kept and skipped, and the
                          four known kinds get value rules
  follow(hint, reader)    what a resolver does with one hint without asking the reader
  carry(uri, carrier)     the string a carrier holds, or a rejection when it cannot

Hints stay outside identity, unsigned and droppable, exactly as slice 1 says.
"""

from __future__ import annotations

import sys

sys.dont_write_bytecode = True

import importlib.util
import random
import re
from dataclasses import dataclass, replace
from pathlib import Path

HERE = Path(__file__).resolve().parent


def _load(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


d = _load("display_reference", HERE / "display_reference.py")
s = d.s
g = s.g
UriError = g.UriError

KNOWN_KINDS = ("broker", "pkarr", "bundle", "https")
_KIND = re.compile(r"[a-z](?:[a-z0-9-]{0,14}[a-z0-9])?\Z", re.ASCII)
_ZBASE32 = re.compile(r"[ybndrfg8ejkmcpqxot1uwisza345h769]{52}\Z", re.ASCII)
_LABEL = r"[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?"
_HTTPS = re.compile(
    rf"(?P<host>{_LABEL}(?:\.{_LABEL})+)(?::(?P<port>[1-9][0-9]{{0,4}}))?"
    r"(?P<path>/[A-Za-z0-9._~!$&'()*+,;=:@/%-]*)?\Z",
    re.ASCII,
)


def check_hint(kind: str, value: str) -> None:
    """Value rules for the known kinds; unknown kinds only need a sane token."""
    if not _KIND.fullmatch(kind):
        raise UriError("invalid_via_hint")
    if value == "":
        raise UriError("invalid_via_hint")
    if kind == "broker" and not g._ALIAS.fullmatch(value):
        raise UriError("invalid_via_value")
    if kind == "pkarr" and not _ZBASE32.fullmatch(value):
        raise UriError("invalid_via_value")
    if kind == "bundle":
        try:
            g._cid_parse(value)
        except UriError:
            raise UriError("invalid_via_value") from None
    if kind == "https":
        match = _HTTPS.fullmatch(value)
        if not match or (match["port"] and int(match["port"]) > 65535):
            raise UriError("invalid_via_value")


def _hints(via: tuple[str, ...], fragment: str | None) -> str:
    if len(via) > g._MAX_VIA:
        raise UriError("too_many_via_hints")
    out = []
    for hint in via:
        kind, colon, value = hint.partition(":")
        if not colon:
            raise UriError("invalid_via_hint")
        check_hint(kind, value)
        out.append(f"via={kind}:{g._pct_encode(value.encode('utf-8'))}")
    text = "?" + "&".join(out) if out else ""
    if fragment is not None:
        if fragment == "":
            raise UriError("empty_fragment")
        text += "#" + g._pct_encode(fragment.encode("utf-8"))
    return text


def format_uri(ref) -> str:
    bare = replace(ref, via=(), fragment=None)
    base = s.format_set(bare) if isinstance(bare, s.SetRef) else g.format_uri(bare)
    uri = base + _hints(ref.via, ref.fragment)
    if len(uri) > g._MAX_LEN:
        raise UriError("uri_too_long")
    return uri


def parse(uri: str):
    """Parse with extensible hint kinds. The path and fragment rules are unchanged."""
    if len(uri) > g._MAX_LEN:
        raise UriError("uri_too_long")
    if any(ord(c) < 0x21 or ord(c) > 0x7E for c in uri):
        raise UriError("raw_control_space_or_nonascii")
    rest, hash_mark, frag_text = uri.partition("#")
    path, q_mark, query = rest.partition("?")
    # Check the path (and fragment) with the slice 1/2 codec, hints removed.
    ref = s.parse(path + (hash_mark + frag_text if hash_mark else ""))
    via: list[str] = []
    if q_mark:
        if query == "":
            raise UriError("empty_query")
        for pair in query.split("&"):
            key, eq, value = pair.partition("=")
            if key != "via" or not eq:
                raise UriError("undeclared_query_parameter")
            kind, colon, enc = value.partition(":")
            if not colon:
                raise UriError("invalid_via_hint")
            try:
                decoded = g._pct_decode(enc).decode("utf-8", "strict")
            except UnicodeDecodeError:
                raise UriError("invalid_via_hint") from None
            via.append(f"{kind}:{decoded}")
    ref = replace(ref, via=tuple(via))
    if format_uri(ref) != uri:
        raise UriError("noncanonical_spelling")
    return ref


def identity(uri: str) -> str:
    return format_uri(replace(parse(uri), via=(), fragment=None))


# ------------------------------------------------------------- following


@dataclass(frozen=True)
class Reader:
    """What one reader has set up. Nothing here comes from a URI."""

    bundles_held: frozenset[str] = frozenset()  # cid tokens of bundles on disk
    brokers: frozenset[str] = frozenset()  # broker names the reader uses
    pkarr_resolver: bool = False  # the reader runs or chose a pkarr resolver
    https_allow: frozenset[str] = frozenset()  # hosts the reader trusts with lookups


def follow(hint: str, reader: Reader) -> str:
    """'follow' (use it now), 'ask' (only on the reader's say-so) or 'skip'.

    Following a hint tells whoever runs it that someone wants this name.
    So a hint is followed without asking only when it reaches something the
    reader already uses; a stranger's door is never contacted on a link's word.
    """
    kind, _, value = hint.partition(":")
    if kind not in KNOWN_KINDS:
        return "skip"
    if kind == "bundle":
        # A bundle on disk costs nothing to look in; one not held cannot be fetched
        # from the hint alone, so there is nothing to ask about.
        return "follow" if value in reader.bundles_held else "skip"
    if kind == "broker":
        return "follow" if value in reader.brokers else "ask"
    if kind == "pkarr":
        return "follow" if reader.pkarr_resolver else "ask"
    host = _HTTPS.fullmatch(value)["host"]
    return "follow" if host in reader.https_allow else "ask"


def plan(uri: str, reader: Reader) -> list[tuple[str, str]]:
    """Hints in carrier order with what the resolver does with each."""
    return [(hint, follow(hint, reader)) for hint in parse(uri).via]


# -------------------------------------------------------------- carriers

QR_BYTE_CAP_M = [14, 26, 42, 62, 84, 106, 122, 152, 180, 213]
QR_MAX_VERSION = 10


@dataclass(frozen=True)
class Carrier:
    name: str
    brackets: bool  # written as <id://...> (slice 5)
    keep_hints: bool
    keep_fragment: bool
    portable_only: bool
    max_chars: int | None  # of the carried text, brackets included


CARRIERS = {
    c.name: c
    for c in (
        # IRC-style line, about 400 usable bytes (R1 §4).
        Carrier("chat-line", True, True, True, False, 400),
        # Body text; slice 5's extractor rejoins wrapped lines.
        Carrier("email-body", True, True, True, False, None),
        # Headers are copied by every relay and archive: identity only, one line
        # well under RFC 5322's 998-octet limit.
        Carrier("email-header", True, False, False, False, 900),
        # Printed text next to a QR code; anyone holding the paper can read it.
        Carrier("paper", True, False, False, True, None),
        Carrier("qr", False, False, False, True, None),
        # D4 bundle scope and index, D3 link targets, D2 embeds: signed or hashed
        # records that outlive any hint.
        Carrier("bundle-index", False, False, False, False, None),
        Carrier("link-object", False, False, True, False, None),
        # Clipboard and share sheet: exactly what the person copied.
        Carrier("clipboard", False, True, True, False, None),
    )
}


def qr_form(uri: str) -> tuple[str, str, int]:
    """(text, mode, version at level M) for a portable identity form."""
    ref = parse(uri)
    if (ref.resolver_hint, ref.namespace) != (g.PORTABLE_ALIAS, g.PORTABLE_NAMESPACE):
        raise UriError("carrier_requires_portable_form")
    upper = uri.upper()
    if all(c in g.QR_ALNUM for c in upper):
        version = g.qr_version_alnum_m(len(upper))
        mode, text = "alphanumeric", upper
    else:
        # `=` and `,` in set parameters are outside the QR alphanumeric set.
        version = next(
            (v for v, cap in enumerate(QR_BYTE_CAP_M, 1) if len(uri) <= cap), None
        )
        mode, text = "byte", uri
    if version is None or version > QR_MAX_VERSION:
        raise UriError("too_long_for_carrier")
    return text, mode, version


def carry(uri: str, carrier: str) -> str:
    """The text a carrier holds for this URI, dropping what the carrier must not keep."""
    c = CARRIERS[carrier]
    ref = parse(uri)
    if not c.keep_hints:
        ref = replace(ref, via=())
    if not c.keep_fragment:
        ref = replace(ref, fragment=None)
    if c.portable_only and (ref.resolver_hint, ref.namespace) != (
        g.PORTABLE_ALIAS,
        g.PORTABLE_NAMESPACE,
    ):
        raise UriError("carrier_requires_portable_form")
    if carrier == "qr":
        return qr_form(format_uri(ref))[0]
    text = format_uri(ref)
    if c.max_chars is not None:
        # Drop hints from the end until it fits; never cut into the name itself.
        while len(text) + (2 if c.brackets else 0) > c.max_chars and ref.via:
            ref = replace(ref, via=ref.via[:-1])
            text = format_uri(ref)
        if len(text) + (2 if c.brackets else 0) > c.max_chars:
            raise UriError("too_long_for_carrier")
    return f"<{text}>" if c.brackets else text


def fail(message: str) -> int:
    print(f"FAIL {message}", file=sys.stderr)
    return 1


PKARR = "yb4ndy" + "o" * 46
D1 = bytes(range(32))
CID1 = g._cid_text(D1)
OBJ = f"id://any/graph/v2/obj/{CID1}"
HINTED = (
    f"{OBJ}?via=broker:fleet-1&via=pkarr:{PKARR}"
    f"&via=bundle:{CID1}&via=https:door.example.org%2Fid"
)


def self_test() -> int:
    # 1. Extensible kinds and value rules.
    accept = [
        HINTED,
        OBJ + "?via=iroh-ticket:abc&via=broker:fleet-1",  # unknown kind kept
        OBJ + "?via=https:door.example.org%3A8443%2Fa-b",
    ]
    for uri in accept:
        if format_uri(parse(uri)) != uri:
            return fail(f"accept {uri}")
    if identity(HINTED) != OBJ:
        return fail("identity drops hints")
    rejects = {
        OBJ + "?via=Ftp:x": "invalid_via_hint",
        OBJ + "?via=new-:x": "invalid_via_hint",
        OBJ + "?via=iroh-ticket:": "invalid_via_hint",
        OBJ + "?via=broker:Fleet": "invalid_via_value",
        OBJ + "?via=pkarr:short": "invalid_via_value",
        OBJ + "?via=bundle:raw-blake3-256.00": "invalid_via_value",
        OBJ + "?via=https:localhost": "invalid_via_value",
        OBJ + "?via=https:user%40door.example.org": "invalid_via_value",
        OBJ + "?via=https:door.example.org%3A99999": "invalid_via_value",
        OBJ + "?via=https:door.example.org%3Fq%3D1": "invalid_via_value",
        OBJ + "?" + "&".join(["via=broker:b"] * 9): "too_many_via_hints",
        OBJ + "?via=broker:b&cap=x": "undeclared_query_parameter",
        OBJ + "?via=https:door.example.org%2F%7E": "noncanonical_percent_escape",
    }
    for uri, code in rejects.items():
        try:
            parse(uri)
        except UriError as exc:
            if str(exc) != code:
                return fail(f"reject {uri}: {exc} != {code}")
        else:
            return fail(f"accepted {uri}")

    # 2. Following hints without asking.
    stranger = Reader()
    set_up = Reader(
        bundles_held=frozenset({CID1}),
        brokers=frozenset({"fleet-1"}),
        pkarr_resolver=True,
        https_allow=frozenset({"door.example.org"}),
    )
    want = {
        "stranger": ["ask", "ask", "skip", "ask"],
        "set_up": ["follow", "follow", "follow", "follow"],
    }
    for name, reader in (("stranger", stranger), ("set_up", set_up)):
        got = [action for _, action in plan(HINTED, reader)]
        if got != want[name]:
            return fail(f"follow {name}: {got}")
    odd = plan(OBJ + "?via=iroh-ticket:abc", set_up)
    if odd != [("iroh-ticket:abc", "skip")]:
        return fail("unknown kind is skipped")
    # A look-alike host on the allow list's parent domain is still a stranger.
    if follow("https:door.example.org.evil.example", set_up) != "ask":
        return fail("allow list matches whole hosts only")

    # 3. One worked URI per carrier.
    scope = f"id://any/graph/v2/set/head/{CID1}/verbs=nest,reply/dir=in/count=4096"
    local = f"id://home.lan/photos%202026/v2/obj/{CID1}#n-3"
    worked = {
        "chat-line": (HINTED + "#n-3", f"<{HINTED}#n-3>"),
        "email-body": (HINTED, f"<{HINTED}>"),
        "email-header": (HINTED + "#n-3", f"<{OBJ}>"),
        "paper": (HINTED, f"<{OBJ}>"),
        "qr": (HINTED + "#n-3", OBJ.upper()),
        "bundle-index": (scope + "?via=broker:fleet-1", scope),
        "link-object": (HINTED + "#n-3", OBJ + "#n-3"),
        "clipboard": (local + "", local),
    }
    lengths = {}
    for carrier, (uri, expect) in worked.items():
        got = carry(uri, carrier)
        if got != expect:
            return fail(f"carry {carrier}: {got}")
        lengths[carrier] = len(got)
        back = (
            d.extract(got)
            if got.startswith("<")
            else [got.lower() if carrier == "qr" else got]
        )
        if [identity(u) for u in back] != [identity(uri)]:
            return fail(f"{carrier} does not read back to the same name")
    for carrier in ("paper", "qr"):
        try:
            carry(local, carrier)
        except UriError as exc:
            if str(exc) != "carrier_requires_portable_form":
                return fail(f"{carrier} local: {exc}")
        else:
            return fail(f"{carrier} accepted a local form")

    # Chat drops hints from the end to fit a line, never the name.
    long_hint = format_uri(
        replace(
            parse(OBJ),
            via=tuple(f"https:door{i}.example.org/{'p' * 40}" for i in range(8)),
        )
    )
    fitted = carry(long_hint, "chat-line")
    if len(fitted) > 400 or identity(d.extract(fitted)[0]) != OBJ:
        return fail("chat fit")
    kept = len(parse(d.extract(fitted)[0]).via)

    # QR modes and versions for the three shapes.
    qr_rows = {
        "obj": qr_form(OBJ),
        "head/at": qr_form(f"id://any/graph/v2/head/{CID1}/at/{CID1}"),
        "set scope": qr_form(scope),
    }
    expect_qr = {
        "obj": ("alphanumeric", 5),
        "head/at": ("alphanumeric", 8),
        "set scope": ("byte", 8),
    }
    for name, (_, mode, version) in qr_rows.items():
        if (mode, version) != expect_qr[name]:
            return fail(f"qr {name}: {mode} v{version}")
    big = f"id://any/graph/v2/set/obj/{CID1}/authors=" + ",".join(
        f"ed25519.{i:064x}" for i in range(1, 4)
    )
    try:
        qr_form(big)
    except UriError as exc:
        if str(exc) != "too_long_for_carrier":
            return fail(f"qr big: {exc}")
    else:
        return fail("qr accepted a URI past version 10")

    # 4. Random round trips with mixed known and unknown kinds.
    rng = random.Random(13886_4)
    for _ in range(256):
        hints = tuple(
            rng.choice(
                [
                    "broker:fleet-1",
                    f"pkarr:{PKARR}",
                    f"bundle:{CID1}",
                    "https:door.example.org/a-b",
                    "x-new:anything at all/é",
                ]
            )
            for _ in range(rng.randrange(4))
        )
        ref = replace(parse(rng.choice([OBJ, scope, local])), via=hints)
        text = format_uri(ref)
        if parse(text) != ref:
            return fail(f"round trip {text}")

    print(
        f"ok: {len(accept)} accepted, {len(rejects)} rejects, follow policy for 2 readers, "
        f"{len(worked)} carriers ({', '.join(f'{k} {v}' for k, v in lengths.items())} chars), "
        f"chat kept {kept} of 8 long hints, QR obj v5 / head-at v8 alphanumeric, "
        "set scope v8 byte, 256 round trips"
    )
    return 0


def main(argv: list[str]) -> int:
    if len(argv) != 2 or argv[1] != "self-test":
        print("usage: carriers_reference.py self-test", file=sys.stderr)
        return 2
    return self_test()


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
