#!/usr/bin/env python3
"""Small stdlib reference for the settled internal id:// V1 syntax.

This is evidence for design/test vectors, not production code. CLI contract:
exactly one argument, `self-test`; all other invocations are usage errors.
The self-test is deterministic and has no file, network, or environment side effects.

Run: python3 -I uri_codec_reference.py self-test
"""

from __future__ import annotations

from dataclasses import dataclass
import random
import re
import sys
from urllib.parse import urlsplit


class UriError(ValueError):
    """Input is not a valid URI under this deliberately narrow profile."""


_ALIAS = re.compile(r"[a-z0-9](?:[a-z0-9._-]{0,62})\Z", re.ASCII)
_PROFILE = "raw-blake3-256"
_HEX = frozenset("0123456789abcdefABCDEF")
_UNRESERVED = frozenset(
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~"
)
_DIGEST = re.compile(r"[0-9a-f]{64}\Z", re.ASCII)


@dataclass(frozen=True)
class ArrayId:
    resolver_hint: str
    namespace: str
    slots: tuple[bytes | None, ...]


@dataclass(frozen=True)
class ContentId:
    resolver_hint: str
    namespace: str
    profile: str
    digest: bytes


def _check_alias(alias: str) -> None:
    if not _ALIAS.fullmatch(alias):
        raise UriError("invalid_resolver_alias")


def _pct_encode(data: bytes) -> str:
    return "".join(chr(b) if chr(b) in _UNRESERVED else f"%{b:02X}" for b in data)


def _pct_decode(component: str) -> bytes:
    """Decode one URI path component, after path splitting, exactly once."""
    out = bytearray()
    i = 0
    while i < len(component):
        ch = component[i]
        if ch == "%":
            if i + 2 >= len(component) or component[i + 1] not in _HEX or component[i + 2] not in _HEX:
                raise UriError("malformed_percent_escape")
            out.append(int(component[i + 1 : i + 3], 16))
            i += 3
            continue
        if ord(ch) > 0x7F or ch not in _UNRESERVED:
            raise UriError("raw_non_unreserved_component_byte")
        out.append(ord(ch))
        i += 1
    return bytes(out)


def _decode_octets(component: str) -> bytes:
    """Keep unreserved octets readable; escape all other octets exactly once."""
    decoded = _pct_decode(component)
    if _pct_encode(decoded) != component:
        raise UriError("noncanonical_byte_slot")
    return decoded


def _namespace_bytes(namespace: str) -> bytes:
    try:
        encoded = namespace.encode("utf-8", "strict")
    except UnicodeEncodeError as exc:
        raise UriError("invalid_namespace_unicode") from exc
    if not encoded:
        raise UriError("empty_namespace")
    if namespace in (".", ".."):
        # Prevent generic path processors from treating a data namespace as
        # a dot-segment. Other embedded dots are ordinary UTF-8 data.
        raise UriError("dot_segment_namespace")
    return encoded


def _namespace_text(segment: str) -> str:
    raw = _pct_decode(segment)
    try:
        value = raw.decode("utf-8", "strict")
    except UnicodeDecodeError as exc:
        raise UriError("invalid_namespace_utf8") from exc
    if not raw or value in (".", ".."):
        raise UriError("invalid_namespace_component")
    return value


def _head(alias: str, namespace: str) -> str:
    _check_alias(alias)
    ns = _pct_encode(_namespace_bytes(namespace))
    return f"id://{alias}/{ns}/v1"


def format_array(value: ArrayId) -> str:
    slots = []
    for slot in value.slots:
        if slot is None:
            slots.append("~")
        else:
            slots.append("b." + _pct_encode(slot))
    suffix = "/array" + ("/" + "/".join(slots) if slots else "")
    return _head(value.resolver_hint, value.namespace) + suffix


def format_content(value: ContentId) -> str:
    if value.profile != _PROFILE:
        raise UriError("unsupported_hash_profile")
    if len(value.digest) != 32:
        raise UriError("invalid_digest_length")
    digest_text = value.digest.hex()
    return f"{_head(value.resolver_hint, value.namespace)}/content/{value.profile}.{digest_text}"


def _split(uri: str) -> tuple[str, str, list[str]]:
    if any(ord(ch) < 0x21 or ord(ch) > 0x7E for ch in uri):
        raise UriError("raw_control_space_or_nonascii")
    try:
        parsed = urlsplit(uri, allow_fragments=True)
    except ValueError as exc:
        raise UriError("uri_parse_error") from exc
    if parsed.scheme != "id" or not parsed.netloc:
        raise UriError("invalid_scheme_or_missing_authority")
    if "?" in uri or "#" in uri:
        raise UriError("query_or_fragment_not_in_v1")
    alias = parsed.netloc
    _check_alias(alias)
    if not parsed.path.startswith("/"):
        raise UriError("path_must_be_absolute")
    # Split while escapes are still encoded. Never decode and then split.
    segments = parsed.path[1:].split("/")
    if len(segments) < 3 or any(s == "" for s in segments):
        raise UriError("empty_or_missing_path_component")
    namespace = _namespace_text(segments[0])
    if segments[1] != "v1":
        raise UriError("unsupported_critical_version")
    return alias, namespace, segments[2:]


def parse(uri: str) -> ArrayId | ContentId:
    alias, namespace, tail = _split(uri)
    if tail[0] == "array":
        slots: list[bytes | None] = []
        for token in tail[1:]:
            if token == "~":
                slots.append(None)
            elif token.startswith("b."):
                slots.append(_decode_octets(token[2:]))
            else:
                raise UriError("invalid_array_slot_token")
        return ArrayId(alias, namespace, tuple(slots))
    if tail[0] == "content" and len(tail) == 2:
        try:
            profile, digest_text = tail[1].split(".", 1)
        except ValueError as exc:
            raise UriError("invalid_content_token") from exc
        if profile != _PROFILE:
            raise UriError("unsupported_hash_profile")
        if not _DIGEST.fullmatch(digest_text):
            raise UriError("invalid_digest_text")
        return ContentId(alias, namespace, profile, bytes.fromhex(digest_text))
    raise UriError("unknown_critical_shape")


def self_test() -> int:
    # Required human-readable edge vectors.
    vectors = {
        "id://local/main/v1/array": ArrayId("local", "main", ()),
        "id://local/main/v1/array/~": ArrayId("local", "main", (None,)),
        "id://local/main/v1/array/b.": ArrayId("local", "main", (b"",)),
        "id://local/main/v1/array/b.%2F": ArrayId("local", "main", (b"/",)),
        "id://local/wiki/v1/array/b.us%20president.md": ArrayId("local", "wiki", (b"us president.md",)),
        "id://local/%E9%9B%AA%2F%C3%A9/v1/array": ArrayId("local", "雪/é", ()),
    }
    for uri, expected in vectors.items():
        got = parse(uri)
        assert got == expected, (uri, got, expected)
        assert format_array(got) == uri

    empty_blake3 = bytes.fromhex(
        "af1349b9f5f9a1a6a0404dea36dcc949"
        "9bcb25c9adc112b7cc9a93cae41f3262"
    )
    content_uri = (
        "id://local/main/v1/content/raw-blake3-256."
        "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
    )
    content_golden = ContentId("local", "main", _PROFILE, empty_blake3)
    assert parse(content_uri) == content_golden
    assert format_content(content_golden) == content_uri

    bad = [
        "id:/local/main/v1/array",  # no authority
        "id://Local/main/v1/array",  # aliases are lowercase
        "id://local/main/v2/array",  # unknown critical version
        "id://local/main/v1/array/%2F",  # no typed slot marker
        "id://local/main/v1/array/b.%",  # malformed escape
        "id://local/main/v1/array/b.%2F/",  # trailing empty component
        "id://local/main/v1/array/b.%61",  # unreserved octets must stay readable
        "id://local/main/v1/array?",  # empty query still introduces a delimiter
        "id://local/main/v1/array#",  # empty fragment still introduces a delimiter
        "id://local/%FF/v1/array",  # namespace is UTF-8 text
        "id://local/main/v1/array?x=1",
        "id://local/main/v1/array#x",
        "id://local/../v1/array",
        "id://local/main/v1/content/unknown.0000000000000000000000000000000000000000000000000000000000000000",
        "id://local/main/v1/content/raw-blake3-256.000000000000000000000000000000000000000000000000000000000000000",
        "id://local/main/v1/content/raw-blake3-256.000000000000000000000000000000000000000000000000000000000000000G",
        "id://local/main/v1/content/raw-blake3-256.000000000000000000000000000000000000000000000000000000000000000A",
    ]
    for uri in bad:
        try:
            parse(uri)
        except UriError:
            pass
        else:
            raise AssertionError(f"accepted malformed URI: {uri!r}")

    rng = random.Random(0x9117_2026)
    alphabet = [None, b"", b"/", b"%", b"~", bytes(range(256)), "雪/é".encode()]
    for _ in range(10_000):
        namespace = rng.choice(["ns", "雪/é", "café%path"]) + "".join(
            rng.choice("abcXYZ012-_") for _ in range(rng.randrange(0, 12))
        )
        count = rng.randrange(0, 10)
        slots = tuple(
            rng.choice(alphabet) if rng.randrange(0, 3) else rng.randbytes(rng.randrange(0, 17))
            for _ in range(count)
        )
        address = ArrayId("local", namespace, slots)
        uri = format_array(address)
        reparsed = parse(uri)
        assert reparsed == address, (address, uri, reparsed)
        assert format_array(reparsed) == uri

    # Content syntax has its own deterministic generator and typed formatter.
    for _ in range(10_000):
        content = ContentId("local", "main", _PROFILE, rng.randbytes(32))
        uri = format_content(content)
        reparsed = parse(uri)
        assert reparsed == content, (content, uri, reparsed)
        assert format_content(reparsed) == uri

    for invalid_content in (
        ContentId("local", "main", _PROFILE, b"short"),
        ContentId("local", "main", "unknown-profile", bytes(32)),
    ):
        try:
            format_content(invalid_content)
        except UriError:
            pass
        else:
            raise AssertionError(f"formatted invalid content ID: {invalid_content!r}")

    # Explicit resolver context: URI alias must match selected local resolver.
    uri = format_array(ArrayId("local", "main", (None, b"")))
    parsed = parse(uri)
    assert parsed.resolver_hint == "local"
    try:
        resolve_context(parsed.resolver_hint, "other")
    except UriError as exc:
        assert str(exc) == "resolver_context_mismatch"
    else:
        raise AssertionError("mismatched resolver context accepted")

    print(
        f"PASS: {len(vectors)} array vectors + 1 content golden, {len(bad)} malformed cases, "
        "10000 deterministic array and 10000 content round trips"
    )
    return 0


def resolve_context(uri_hint: str, selected_local_resolver: str) -> str:
    _check_alias(uri_hint)
    _check_alias(selected_local_resolver)
    if uri_hint != selected_local_resolver:
        raise UriError("resolver_context_mismatch")
    return selected_local_resolver


def main(argv: list[str]) -> int:
    # Strict argument contract is checked before any operation.
    if len(argv) != 2 or argv[1] != "self-test":
        print("usage: uri_codec_reference.py self-test", file=sys.stderr)
        return 2
    return self_test()


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
