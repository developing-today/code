#!/usr/bin/env python3
"""Reference codec for the proposed id:// v2 graph shapes (D1 slice 1, #13886).

Stdlib only. Run with:  python3 -I graph_uri_reference.py self-test

This extends, and does not replace, the V1 codec in
doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/uri_codec_reference.py.
V1 URIs keep their meaning; this file only parses the `v2` version segment.

Grammar (canonical form only; every other spelling is rejected):

  uri       = "id://" alias "/" ns "/v2/" shape [ "?" hints ] [ "#" fragment ]
  shape     = "obj/" cid [ "/bytes/" range ]
            | "head/" cid [ "/at/" cid ]
  cid       = "raw-blake3-256." 64 lowercase hex
  range     = from "-" [ to ]            ; decimal, no leading zeros, to > from
  hints     = via *( "&via=" via-value ) ; only the `via` key, order kept
  via-value = kind ":" pct-value         ; kind in broker | pkarr | bundle | https

Identity is the path. Query hints and the fragment never change what is named
and are removed by `identity()`.
"""

from __future__ import annotations

import hashlib
import random
import re
import sys
from dataclasses import dataclass, field

_ALIAS = re.compile(r"[a-z0-9](?:[a-z0-9._-]{0,62})\Z", re.ASCII)
_PROFILE = "raw-blake3-256"
_DIGEST = re.compile(r"[0-9a-f]{64}\Z", re.ASCII)
_DEC = re.compile(r"(?:0|[1-9][0-9]*)\Z", re.ASCII)
_U64_MAX = (1 << 64) - 1
_UNRESERVED = frozenset(
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~"
)
_VIA_KINDS = ("broker", "pkarr", "bundle", "https")
_MAX_VIA = 8
_MAX_LEN = 8192
PORTABLE_ALIAS = "any"
PORTABLE_NAMESPACE = "graph"
QR_ALNUM = frozenset("0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ $%*+-./:")

# Pinned after the first run; any codec change that moves it is a format change.
CORPUS_SHA256 = "fec7089f0f07356844a9d95f60e99fc2271c7f23dc304e908a35e953c228b6db"


class UriError(ValueError):
    """A stable, machine-readable rejection code."""


@dataclass(frozen=True)
class ObjRef:
    resolver_hint: str
    namespace: str
    digest: bytes
    byte_range: tuple[int, int | None] | None = None
    via: tuple[str, ...] = field(default=())
    fragment: str | None = None


@dataclass(frozen=True)
class HeadRef:
    resolver_hint: str
    namespace: str
    root: bytes
    expected_head: bytes | None = None
    via: tuple[str, ...] = field(default=())
    fragment: str | None = None


def _pct_encode(data: bytes) -> str:
    return "".join(chr(b) if b in _UNRESERVED else f"%{b:02X}" for b in data)


def _pct_decode(text: str) -> bytes:
    out = bytearray()
    i = 0
    while i < len(text):
        ch = text[i]
        if ch == "%":
            pair = text[i + 1 : i + 3]
            if len(pair) != 2 or not re.fullmatch(r"[0-9A-F]{2}", pair):
                raise UriError("invalid_percent_escape")
            value = int(pair, 16)
            if value in _UNRESERVED:
                raise UriError("noncanonical_percent_escape")
            out.append(value)
            i += 3
        else:
            if ord(ch) not in _UNRESERVED:
                raise UriError("unescaped_reserved_octet")
            out.append(ord(ch))
            i += 1
    return bytes(out)


def _check_alias(alias: str) -> None:
    if not _ALIAS.fullmatch(alias):
        raise UriError("invalid_resolver_hint")


def _cid_text(digest: bytes) -> str:
    if len(digest) != 32:
        raise UriError("invalid_digest_length")
    return f"{_PROFILE}.{digest.hex()}"


def _cid_parse(token: str) -> bytes:
    profile, dot, digest = token.partition(".")
    if not dot:
        raise UriError("invalid_content_token")
    if profile != _PROFILE:
        raise UriError("unsupported_hash_profile")
    if not _DIGEST.fullmatch(digest):
        raise UriError("invalid_digest_text")
    return bytes.fromhex(digest)


def _ns_text(namespace: str) -> str:
    raw = namespace.encode("utf-8")
    if not raw or namespace in (".", ".."):
        raise UriError("invalid_namespace_component")
    return _pct_encode(raw)


def _check_portable(alias: str, namespace: str) -> None:
    # `any` is reserved: it is valid only with the `graph` namespace, and the
    # `graph` namespace under `any` carries only self-certifying shapes, which
    # is every v2 shape in slice 1.
    if alias == PORTABLE_ALIAS and namespace != PORTABLE_NAMESPACE:
        raise UriError("portable_hint_requires_graph_namespace")


def _hints(via: tuple[str, ...], fragment: str | None) -> str:
    if len(via) > _MAX_VIA:
        raise UriError("too_many_via_hints")
    out = ""
    if via:
        parts = []
        for v in via:
            kind, colon, value = v.partition(":")
            if not colon or kind not in _VIA_KINDS or not value:
                raise UriError("invalid_via_hint")
            parts.append(f"via={kind}:{_pct_encode(value.encode('utf-8'))}")
        out += "?" + "&".join(parts)
    if fragment is not None:
        if fragment == "":
            raise UriError("empty_fragment")
        out += "#" + _pct_encode(fragment.encode("utf-8"))
    return out


def format_uri(ref: ObjRef | HeadRef) -> str:
    _check_alias(ref.resolver_hint)
    _check_portable(ref.resolver_hint, ref.namespace)
    head = f"id://{ref.resolver_hint}/{_ns_text(ref.namespace)}/v2/"
    if isinstance(ref, ObjRef):
        body = "obj/" + _cid_text(ref.digest)
        if ref.byte_range is not None:
            start, end = ref.byte_range
            if not 0 <= start <= _U64_MAX:
                raise UriError("invalid_byte_range")
            if end is not None and not start < end <= _U64_MAX:
                raise UriError("invalid_byte_range")
            body += f"/bytes/{start}-" + ("" if end is None else str(end))
    else:
        body = "head/" + _cid_text(ref.root)
        if ref.expected_head is not None:
            body += "/at/" + _cid_text(ref.expected_head)
    uri = head + body + _hints(ref.via, ref.fragment)
    if len(uri) > _MAX_LEN:
        raise UriError("uri_too_long")
    return uri


def _parse_u64(text: str) -> int:
    if not _DEC.fullmatch(text):
        raise UriError("noncanonical_decimal")
    if len(text) > 20:  # 2^64-1 has 20 digits; also keeps int() off huge strings
        raise UriError("decimal_out_of_range")
    value = int(text)
    if value > _U64_MAX:
        raise UriError("decimal_out_of_range")
    return value


def parse(uri: str) -> ObjRef | HeadRef:
    if len(uri) > _MAX_LEN:
        raise UriError("uri_too_long")
    if any(ord(c) < 0x21 or ord(c) > 0x7E for c in uri):
        raise UriError("raw_control_space_or_nonascii")
    if not uri.startswith("id://"):
        raise UriError("invalid_scheme_or_missing_authority")
    rest = uri[5:]
    rest, hash_mark, frag_text = rest.partition("#")
    rest, q_mark, query = rest.partition("?")
    alias, slash, path = rest.partition("/")
    if not slash:
        raise UriError("empty_or_missing_path_component")
    _check_alias(alias)
    segs = path.split("/")
    if len(segs) < 4 or any(s == "" for s in segs):
        raise UriError("empty_or_missing_path_component")
    raw_ns = _pct_decode(segs[0])
    try:
        namespace = raw_ns.decode("utf-8", "strict")
    except UnicodeDecodeError as exc:
        raise UriError("invalid_namespace_utf8") from exc
    if namespace in (".", ".."):
        raise UriError("invalid_namespace_component")
    if segs[1] != "v2":
        raise UriError("unsupported_critical_version")
    _check_portable(alias, namespace)
    via: list[str] = []
    if q_mark:
        if query == "":
            raise UriError("empty_query")
        for pair in query.split("&"):
            key, eq, value = pair.partition("=")
            if key != "via" or not eq:
                raise UriError("undeclared_query_parameter")
            kind, colon, enc = value.partition(":")
            if not colon or kind not in _VIA_KINDS or not enc:
                raise UriError("invalid_via_hint")
            try:
                decoded = _pct_decode(enc).decode("utf-8", "strict")
            except UnicodeDecodeError as exc:
                raise UriError("invalid_via_hint") from exc
            via.append(f"{kind}:{decoded}")
        if len(via) > _MAX_VIA:
            raise UriError("too_many_via_hints")
    fragment = None
    if hash_mark:
        if frag_text == "":
            raise UriError("empty_fragment")
        fragment = _pct_decode(frag_text).decode("utf-8", "replace")
    tail = segs[2:]
    if tail[0] == "obj" and len(tail) in (2, 4):
        digest = _cid_parse(tail[1])
        rng = None
        if len(tail) == 4:
            if tail[2] != "bytes":
                raise UriError("unknown_critical_shape")
            start_t, dash, end_t = tail[3].partition("-")
            if not dash:
                raise UriError("invalid_byte_range")
            start = _parse_u64(start_t)
            end = None if end_t == "" else _parse_u64(end_t)
            if end is not None and end <= start:
                raise UriError("invalid_byte_range")
            rng = (start, end)
        ref: ObjRef | HeadRef = ObjRef(
            alias, namespace, digest, rng, tuple(via), fragment
        )
    elif tail[0] == "head" and len(tail) in (2, 4):
        root = _cid_parse(tail[1])
        expected = None
        if len(tail) == 4:
            if tail[2] != "at":
                raise UriError("unknown_critical_shape")
            expected = _cid_parse(tail[3])
        ref = HeadRef(alias, namespace, root, expected, tuple(via), fragment)
    elif tail[0] == "set":
        raise UriError("reserved_shape_not_in_slice_1")
    else:
        raise UriError("unknown_critical_shape")
    # Round trip: only the canonical spelling is accepted.
    if format_uri(ref) != uri:
        raise UriError("noncanonical_spelling")
    return ref


def identity(uri: str) -> str:
    """The canonical identity form: same object, hints and fragment removed."""
    ref = parse(uri)
    if isinstance(ref, ObjRef):
        bare = ObjRef(ref.resolver_hint, ref.namespace, ref.digest, ref.byte_range)
    else:
        bare = HeadRef(ref.resolver_hint, ref.namespace, ref.root, ref.expected_head)
    return format_uri(bare)


def to_qr(uri: str) -> str:
    """QR alphanumeric transport form; portable identity form only."""
    ident = identity(uri)
    ref = parse(ident)
    if ref.resolver_hint != PORTABLE_ALIAS:
        raise UriError("qr_form_requires_portable_hint")
    up = ident.upper()
    assert all(c in QR_ALNUM for c in up)
    return up


def from_qr(text: str) -> ObjRef | HeadRef:
    if text != text.upper() or any(c not in QR_ALNUM for c in text):
        raise UriError("not_qr_alphanumeric_form")
    ref = parse(text.lower())
    if ref.resolver_hint != PORTABLE_ALIAS or ref.via or ref.fragment is not None:
        raise UriError("qr_form_requires_portable_identity")
    return ref


def resolve_context(uri_hint: str, selected_local_resolver: str) -> str:
    """V1 rule (A-9117.1) plus the proposed portable exception."""
    _check_alias(uri_hint)
    _check_alias(selected_local_resolver)
    if uri_hint == PORTABLE_ALIAS:
        return selected_local_resolver
    if uri_hint != selected_local_resolver:
        raise UriError("resolver_context_mismatch")
    return selected_local_resolver


_QR_ALNUM_CAP_M = [20, 38, 61, 90, 122, 154, 178, 221, 262, 311]


def qr_version_alnum_m(n: int) -> int | None:
    for version, cap in enumerate(_QR_ALNUM_CAP_M, start=1):
        if n <= cap:
            return version
    return None


def self_test() -> int:
    empty = hashlib.blake2s(b"").digest()  # stand-in 32-byte digest, not blake3
    d1 = bytes(range(32))
    golden = {
        "obj_portable": (
            ObjRef("any", "graph", d1),
            "id://any/graph/v2/obj/raw-blake3-256."
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
        ),
        "obj_slice_open": (
            ObjRef("local", "main", d1, (100, None)),
            "id://local/main/v2/obj/raw-blake3-256."
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f/bytes/100-",
        ),
        "head_at_hinted": (
            HeadRef(
                "any", "graph", d1, empty, ("pkarr:yb4ndy", "https:relay.example/x y")
            ),
            "id://any/graph/v2/head/raw-blake3-256."
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f/at/raw-blake3-256."
            + empty.hex()
            + "?via=pkarr:yb4ndy&via=https:relay.example%2Fx%20y",
        ),
    }
    for name, (ref, text) in golden.items():
        got = format_uri(ref)
        if got != text:
            print(f"FAIL golden {name}: {got}", file=sys.stderr)
            return 1
        if parse(text) != ref:
            print(f"FAIL golden parse {name}", file=sys.stderr)
            return 1
    hinted = golden["head_at_hinted"][1] + "#comment-3"
    if identity(hinted) != golden["head_at_hinted"][1].split("?")[0]:
        print("FAIL identity strips hints and fragment", file=sys.stderr)
        return 1
    qr = to_qr(golden["obj_portable"][1])
    if from_qr(qr) != golden["obj_portable"][0]:
        print("FAIL qr round trip", file=sys.stderr)
        return 1

    rejects = {
        "id://local/main/v1/content/raw-blake3-256."
        + "00" * 32: "unsupported_critical_version",
        "id://any/main/v2/obj/raw-blake3-256."
        + "00" * 32: "portable_hint_requires_graph_namespace",
        "id://any/graph/v2/obj/raw-blake3-256." + "AB" * 32: "invalid_digest_text",
        "id://any/graph/v2/obj/william3-256." + "00" * 32: "unsupported_hash_profile",
        "id://any/graph/v2/obj/raw-blake3-256."
        + "00" * 32
        + "/bytes/05-9": "noncanonical_decimal",
        "id://any/graph/v2/obj/raw-blake3-256."
        + "00" * 32
        + "/bytes/9-9": "invalid_byte_range",
        "id://any/graph/v2/obj/raw-blake3-256."
        + "00" * 32
        + "?cap=abc": "undeclared_query_parameter",
        "id://any/graph/v2/obj/raw-blake3-256."
        + "00" * 32
        + "?via=ftp:x": "invalid_via_hint",
        "id://any/graph/v2/obj/raw-blake3-256." + "00" * 32 + "?": "empty_query",
        "id://any/graph/v2/set/abc": "reserved_shape_not_in_slice_1",
        "id://any/graph/v2/obj/raw-blake3-256."
        + "00" * 32
        + "?via=https:a%2fb": "invalid_percent_escape",
        "id://any/graph/v2/obj/raw-blake3-256."
        + "00" * 32
        + "?via=https:%61": "noncanonical_percent_escape",
        "id://any/gr%61ph/v2/obj/raw-blake3-256."
        + "00" * 32: "noncanonical_percent_escape",
        "id://Any/graph/v2/obj/raw-blake3-256." + "00" * 32: "invalid_resolver_hint",
    }
    for text, code in rejects.items():
        try:
            parse(text)
        except UriError as exc:
            if str(exc) != code:
                print(f"FAIL reject {text!r}: {exc} != {code}", file=sys.stderr)
                return 1
        else:
            print(f"FAIL accepted {text!r}", file=sys.stderr)
            return 1
    try:
        resolve_context("home", "work")
    except UriError as exc:
        assert str(exc) == "resolver_context_mismatch"
    else:
        print("FAIL resolver mismatch accepted", file=sys.stderr)
        return 1
    assert resolve_context("any", "work") == "work"

    rng = random.Random(13886)
    corpus = hashlib.sha256()
    for _ in range(512):
        alias = rng.choice(["any", "local", "home.lan", "r-2"])
        ns = "graph" if alias == "any" else rng.choice(["graph", "main", "photos 2026"])
        via = tuple(
            rng.choice(_VIA_KINDS) + ":" + rng.choice(["z", "relay/a b", "ü"])
            for _ in range(rng.randrange(3))
        )
        if rng.random() < 0.5:
            start = rng.randrange(1 << 20)
            rngv = rng.choice(
                [None, (start, None), (start, start + 1 + rng.randrange(99))]
            )
            ref: ObjRef | HeadRef = ObjRef(alias, ns, rng.randbytes(32), rngv, via)
        else:
            ref = HeadRef(
                alias, ns, rng.randbytes(32), rng.choice([None, rng.randbytes(32)]), via
            )
        text = format_uri(ref)
        if parse(text) != ref:
            print(f"FAIL round trip {text}", file=sys.stderr)
            return 1
        corpus.update(text.encode() + b"\n")
    digest = corpus.hexdigest()
    if CORPUS_SHA256 != "PIN_ME" and digest != CORPUS_SHA256:
        print(f"FAIL corpus {digest}", file=sys.stderr)
        return 1

    portable = golden["obj_portable"][1]
    head_at = golden["head_at_hinted"][1].split("?")[0]
    print(
        "ok: 3 golden, 14 rejects, 512 round trips, corpus sha256 "
        f"{digest}; portable obj {len(portable)} chars (QR alnum v{qr_version_alnum_m(len(portable))}-M), "
        f"portable head/at {len(head_at)} chars (QR alnum v{qr_version_alnum_m(len(head_at))}-M)"
    )
    return 0


def main(argv: list[str]) -> int:
    if len(argv) != 2 or argv[1] != "self-test":
        print("usage: graph_uri_reference.py self-test", file=sys.stderr)
        return 2
    return self_test()


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
