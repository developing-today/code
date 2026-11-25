#!/usr/bin/env python3
"""Reference writer and two readers for the ID sneakernet bundle format
(D4 slice 1, #13889). Python stdlib only.

Layout (all integers big-endian):

  bundle   = magic version manifest_len manifest_body sig_alg signature objects
  magic    = "IDBUNDLE"                    8 bytes
  version  = u16, 1
  manifest_len = u32, <= 1 MiB
  manifest_body = record*  ; strictly ascending tags, no duplicates
  record   = tag:u8 len:u32 value
  sig_alg  = u8, 0x01 = Ed25519
  signature = 64 bytes, Ed25519 over DOMAIN || manifest_body
  objects  = the object bytes of every contents entry, in index order,
             concatenated; nothing after the last one

Manifest tags (0x01-0x7f critical: unknown means reject; 0x80-0xff
non-critical: unknown is ignored, but it is still covered by the signature):

  0x01 format            "id-bundle/1"                       required
  0x02 creator_identity  UTF-8, the IdP-stable identity      required
  0x03 signing_key       key_alg:u8 (0x01 Ed25519) + 32 bytes required
  0x04 creator_binding   opaque, checked in slice 3          optional
  0x05 created_at        u64 unix seconds (a claim)          required
  0x06 expires_at        u64 unix seconds, > created_at      optional
  0x07 contents          count:u32 entry*                    required
  0x08 grants            reserved for slice 2                must be absent
  0x09 scope             ASCII D1 URI of the exported slice  optional
  0x81 renewal_note      UTF-8                               optional

  entry = alg_len:u8 alg digest_len:u8 digest size:u64 kind:u8
          alg "raw-blake3-256", digest 32 bytes; kind 1 content, 2 link,
          3 blob, 4 envelope (slice 4); entries strictly ascending by (alg, digest)

The bundle ID is BLAKE3-256 of manifest_body, written as the D1 portable
object URI id://any/graph/v2/obj/raw-blake3-256.<hex>.

Run with no arguments for the self-test. --write PATH writes the example
bundle. --cross-check also checks BLAKE3 and Ed25519 against the `blake3`
and `cryptography` packages when they are installed.
"""

from __future__ import annotations

import hashlib
import struct
import sys
from dataclasses import dataclass, field

MAGIC = b"IDBUNDLE"
VERSION = 1
FORMAT = b"id-bundle/1"
DOMAIN = b"id-bundle/1\x00manifest\x00"
MAX_MANIFEST = 1 << 20
MAX_ENTRIES = 4096
MAX_OBJECT = 16 << 20
DIGEST_ALG = b"raw-blake3-256"
KINDS = {1: "content", 2: "link", 3: "blob", 4: "envelope"}  # 4: slice 4
KNOWN_TAGS = {0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x81}
REQUIRED_TAGS = {0x01, 0x02, 0x03, 0x05, 0x07}
EXAMPLE_SHA256 = "95f95205826df1419619663572e0698eb2a3133485fa81cc275263bf031b7e93"

# ---------------------------------------------------------------- BLAKE3

_IV = (0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A,
       0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19)
_PERM = (2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8)
_START, _END, _PARENT, _ROOT = 1, 2, 4, 8
_M = 0xFFFFFFFF


def _g(s, a, b, c, d, x, y):
    s[a] = (s[a] + s[b] + x) & _M
    s[d] ^= s[a]; s[d] = ((s[d] >> 16) | (s[d] << 16)) & _M
    s[c] = (s[c] + s[d]) & _M
    s[b] ^= s[c]; s[b] = ((s[b] >> 12) | (s[b] << 20)) & _M
    s[a] = (s[a] + s[b] + y) & _M
    s[d] ^= s[a]; s[d] = ((s[d] >> 8) | (s[d] << 24)) & _M
    s[c] = (s[c] + s[d]) & _M
    s[b] ^= s[c]; s[b] = ((s[b] >> 7) | (s[b] << 25)) & _M


def _compress(cv, words, counter, block_len, flags):
    s = list(cv) + list(_IV[:4]) + [counter & _M, (counter >> 32) & _M, block_len, flags]
    m = list(words)
    for r in range(7):
        _g(s, 0, 4, 8, 12, m[0], m[1]); _g(s, 1, 5, 9, 13, m[2], m[3])
        _g(s, 2, 6, 10, 14, m[4], m[5]); _g(s, 3, 7, 11, 15, m[6], m[7])
        _g(s, 0, 5, 10, 15, m[8], m[9]); _g(s, 1, 6, 11, 12, m[10], m[11])
        _g(s, 2, 7, 8, 13, m[12], m[13]); _g(s, 3, 4, 9, 14, m[14], m[15])
        if r < 6:
            m = [m[i] for i in _PERM]
    return [s[i] ^ s[i + 8] for i in range(8)] + [s[i + 8] ^ cv[i] for i in range(8)]


def _words(block: bytes):
    return struct.unpack("<16I", block.ljust(64, b"\0"))


def _chunk(data: bytes, index: int):
    """Return the pending output (cv, words, counter, len, flags) of one chunk."""
    cv = _IV
    blocks = [data[i:i + 64] for i in range(0, len(data), 64)] or [b""]
    for n, block in enumerate(blocks):
        flags = (_START if n == 0 else 0) | (_END if n == len(blocks) - 1 else 0)
        if n == len(blocks) - 1:
            return (cv, _words(block), index, len(block), flags)
        cv = _compress(cv, _words(block), index, 64, flags)[:8]
    raise AssertionError


def _subtree(data: bytes, first_chunk: int):
    if len(data) <= 1024:
        return _chunk(data, first_chunk)
    chunks = (len(data) + 1023) // 1024
    left = 1 << ((chunks - 1).bit_length() - 1)
    lcv = _compress(*_subtree(data[:left * 1024], first_chunk))[:8]
    rcv = _compress(*_subtree(data[left * 1024:], first_chunk + left))[:8]
    return (_IV, tuple(lcv) + tuple(rcv), 0, 64, _PARENT)


def blake3_256(data: bytes) -> bytes:
    cv, words, counter, block_len, flags = _subtree(data, 0)
    out = _compress(cv, words, 0 if flags & _PARENT else counter, block_len, flags | _ROOT)
    return struct.pack("<8I", *out[:8])


# -------------------------------------------------------------- Ed25519 (RFC 8032)

_P = 2 ** 255 - 19
_Q = 2 ** 252 + 27742317777372353535851937790883648493
_D = -121665 * pow(121666, _P - 2, _P) % _P
_SQRT_M1 = pow(2, (_P - 1) // 4, _P)


def _inv(x):
    return pow(x, _P - 2, _P)


def _add(a, b):
    A = (a[1] - a[0]) * (b[1] - b[0]) % _P
    B = (a[1] + a[0]) * (b[1] + b[0]) % _P
    C = 2 * a[3] * b[3] * _D % _P
    D = 2 * a[2] * b[2] % _P
    E, F, G, H = B - A, D - C, D + C, B + A
    return (E * F % _P, G * H % _P, F * G % _P, E * H % _P)


def _mul(s, pt):
    q = (0, 1, 1, 0)
    while s:
        if s & 1:
            q = _add(q, pt)
        pt = _add(pt, pt)
        s >>= 1
    return q


def _eq(a, b):
    return (a[0] * b[2] - b[0] * a[2]) % _P == 0 and (a[1] * b[2] - b[1] * a[2]) % _P == 0


def _recover_x(y, sign):
    if y >= _P:
        return None
    x2 = (y * y - 1) * _inv(_D * y * y + 1) % _P
    if x2 == 0:
        return None if sign else 0
    x = pow(x2, (_P + 3) // 8, _P)
    if (x * x - x2) % _P:
        x = x * _SQRT_M1 % _P
    if (x * x - x2) % _P:
        return None
    return _P - x if (x & 1) != sign else x


_GY = 4 * _inv(5) % _P
_GX = _recover_x(_GY, 0)
_G = (_GX, _GY, 1, _GX * _GY % _P)


def _encode_point(pt) -> bytes:
    z = _inv(pt[2])
    x, y = pt[0] * z % _P, pt[1] * z % _P
    return (y | ((x & 1) << 255)).to_bytes(32, "little")


def _decode_point(raw: bytes):
    if len(raw) != 32:
        return None
    y = int.from_bytes(raw, "little")
    sign, y = y >> 255, y & ((1 << 255) - 1)
    x = _recover_x(y, sign)
    return None if x is None else (x, y, 1, x * y % _P)


def _h(data: bytes) -> int:
    return int.from_bytes(hashlib.sha512(data).digest(), "little") % _Q


def _expand(seed: bytes):
    h = hashlib.sha512(seed).digest()
    a = int.from_bytes(h[:32], "little")
    a = (a & ((1 << 254) - 8)) | (1 << 254)
    return a, h[32:]


def ed25519_public(seed: bytes) -> bytes:
    return _encode_point(_mul(_expand(seed)[0], _G))


def ed25519_sign(seed: bytes, msg: bytes) -> bytes:
    a, prefix = _expand(seed)
    pub = _encode_point(_mul(a, _G))
    r = _h(prefix + msg)
    big_r = _encode_point(_mul(r, _G))
    s = (r + _h(big_r + pub + msg) * a) % _Q
    return big_r + s.to_bytes(32, "little")


def ed25519_verify(pub: bytes, msg: bytes, sig: bytes) -> bool:
    if len(sig) != 64:
        return False
    a, r = _decode_point(pub), _decode_point(sig[:32])
    if a is None or r is None:
        return False
    s = int.from_bytes(sig[32:], "little")
    if s >= _Q:
        return False
    return _eq(_mul(s, _G), _add(r, _mul(_h(sig[:32] + pub + msg), a)))


# ---------------------------------------------------------------- model

class BundleError(ValueError):
    """Rejection; the message is the stable rejection code."""


@dataclass(frozen=True)
class Entry:
    digest: bytes
    size: int
    kind: int


@dataclass
class Manifest:
    creator_identity: str
    signing_key: bytes
    created_at: int
    entries: list[Entry]
    creator_binding: bytes | None = None
    expires_at: int | None = None
    scope: str | None = None
    renewal_note: str | None = None
    ignored_tags: list[int] = field(default_factory=list)


@dataclass
class Report:
    bundle_id: str
    creator_identity: str
    binding: str          # "unbound" or "present-unchecked" in slice 1
    objects: list[tuple[str, str, int]]  # (kind, digest hex, size)
    expiry: str           # "none", "valid", "expired" (status only in slice 1)
    ignored_tags: list[int]


def _u64(v: int) -> bytes:
    return struct.pack(">Q", v)


def encode_manifest(m: Manifest) -> bytes:
    recs = {
        0x01: FORMAT,
        0x02: m.creator_identity.encode("utf-8"),
        0x03: b"\x01" + m.signing_key,
        0x05: _u64(m.created_at),
    }
    if m.creator_binding is not None:
        recs[0x04] = m.creator_binding
    if m.expires_at is not None:
        recs[0x06] = _u64(m.expires_at)
    entries = sorted(m.entries, key=lambda e: e.digest)
    body = struct.pack(">I", len(entries))
    for e in entries:
        body += bytes([len(DIGEST_ALG)]) + DIGEST_ALG + bytes([len(e.digest)]) + e.digest
        body += _u64(e.size) + bytes([e.kind])
    recs[0x07] = body
    if m.scope is not None:
        recs[0x09] = m.scope.encode("ascii")
    if m.renewal_note is not None:
        recs[0x81] = m.renewal_note.encode("utf-8")
    return b"".join(bytes([t]) + struct.pack(">I", len(v)) + v for t, v in sorted(recs.items()))


def write_bundle(m: Manifest, seed: bytes, objects: dict[bytes, bytes]) -> bytes:
    body = encode_manifest(m)
    sig = ed25519_sign(seed, DOMAIN + body)
    order = sorted(e.digest for e in m.entries)
    return (MAGIC + struct.pack(">HI", VERSION, len(body)) + body + b"\x01" + sig
            + b"".join(objects[d] for d in order))


def bundle_id_uri(body: bytes) -> str:
    return "id://any/graph/v2/obj/raw-blake3-256." + blake3_256(body).hex()


# ------------------------------------------------------------- reader A
# Structured: parse into a Manifest, then verify.

def _take(buf: bytes, pos: int, n: int, code: str) -> tuple[bytes, int]:
    if n < 0 or pos + n > len(buf):
        raise BundleError(code)
    return buf[pos:pos + n], pos + n


def _parse_records(body: bytes) -> dict[int, bytes]:
    recs: dict[int, bytes] = {}
    pos, last = 0, -1
    while pos < len(body):
        head, pos = _take(body, pos, 5, "truncated_record")
        tag, n = head[0], struct.unpack(">I", head[1:])[0]
        value, pos = _take(body, pos, n, "truncated_record")
        if tag <= last:
            raise BundleError("records_not_ascending")
        last = tag
        if tag not in KNOWN_TAGS and tag < 0x80:
            raise BundleError("unknown_critical_tag")
        recs[tag] = value
    return recs


def _u64_field(v: bytes, code: str) -> int:
    if len(v) != 8:
        raise BundleError(code)
    return struct.unpack(">Q", v)[0]


def _utf8(v: bytes, code: str) -> str:
    try:
        text = v.decode("utf-8")
    except UnicodeDecodeError:
        raise BundleError(code) from None
    if not text or any(ord(c) < 0x20 for c in text):
        raise BundleError(code)
    return text


def _parse_entries(v: bytes) -> list[Entry]:
    head, pos = _take(v, 0, 4, "bad_contents")
    count = struct.unpack(">I", head)[0]
    if count == 0:
        raise BundleError("empty_contents")
    if count > MAX_ENTRIES:
        raise BundleError("too_many_entries")
    out: list[Entry] = []
    for _ in range(count):
        n, pos = _take(v, pos, 1, "bad_contents")
        alg, pos = _take(v, pos, n[0], "bad_contents")
        if alg != DIGEST_ALG:
            raise BundleError("unknown_digest_alg")
        n, pos = _take(v, pos, 1, "bad_contents")
        if n[0] != 32:
            raise BundleError("bad_digest_length")
        digest, pos = _take(v, pos, 32, "bad_contents")
        tail, pos = _take(v, pos, 9, "bad_contents")
        size, kind = struct.unpack(">Q", tail[:8])[0], tail[8]
        if kind not in KINDS:
            raise BundleError("unknown_kind")
        if size > MAX_OBJECT:
            raise BundleError("object_too_large")
        if out and digest <= out[-1].digest:
            raise BundleError("entries_not_ascending")
        out.append(Entry(digest, size, kind))
    if pos != len(v):
        raise BundleError("bad_contents")
    return out


def parse_manifest(body: bytes) -> Manifest:
    recs = _parse_records(body)
    if not REQUIRED_TAGS <= recs.keys():
        raise BundleError("missing_required_field")
    if recs[0x01] != FORMAT:
        raise BundleError("unknown_format")
    if 0x08 in recs:
        raise BundleError("grants_reserved_slice_2")
    key = recs[0x03]
    if len(key) != 33 or key[0] != 0x01:
        raise BundleError("bad_signing_key")
    m = Manifest(
        creator_identity=_utf8(recs[0x02], "bad_creator_identity"),
        signing_key=key[1:],
        created_at=_u64_field(recs[0x05], "bad_time"),
        entries=_parse_entries(recs[0x07]),
        creator_binding=recs.get(0x04),
        ignored_tags=[t for t in recs if t >= 0x80 and t not in KNOWN_TAGS],
    )
    if 0x06 in recs:
        m.expires_at = _u64_field(recs[0x06], "bad_time")
        if m.expires_at <= m.created_at:
            raise BundleError("expiry_before_creation")
    if 0x09 in recs:
        try:
            m.scope = recs[0x09].decode("ascii")
        except UnicodeDecodeError:
            raise BundleError("bad_scope") from None
        if not m.scope.startswith("id://"):
            raise BundleError("bad_scope")
    if 0x81 in recs:
        m.renewal_note = _utf8(recs[0x81], "bad_renewal_note")
    return m


def read_a(blob: bytes, now: int | None = None) -> Report:
    head, pos = _take(blob, 0, 14, "truncated_header")
    if head[:8] != MAGIC:
        raise BundleError("bad_magic")
    version, mlen = struct.unpack(">HI", head[8:])
    if version != VERSION:
        raise BundleError("unknown_version")
    if mlen > MAX_MANIFEST:
        raise BundleError("manifest_too_large")
    body, pos = _take(blob, pos, mlen, "truncated_manifest")
    sig_head, pos = _take(blob, pos, 65, "truncated_signature")
    m = parse_manifest(body)
    if sig_head[0] != 0x01:
        raise BundleError("unknown_signature_alg")
    if not ed25519_verify(m.signing_key, DOMAIN + body, sig_head[1:]):
        raise BundleError("bad_signature")
    objects = []
    for e in m.entries:
        data, pos = _take(blob, pos, e.size, "truncated_objects")
        if blake3_256(data) != e.digest:
            raise BundleError("digest_mismatch")
        objects.append((KINDS[e.kind], e.digest.hex(), e.size))
    if pos != len(blob):
        raise BundleError("trailing_bytes")
    expiry = "none"
    if m.expires_at is not None and now is not None:
        expiry = "expired" if now >= m.expires_at else "valid"
    elif m.expires_at is not None:
        expiry = "valid"
    return Report(bundle_id_uri(body), m.creator_identity,
                  "unbound" if m.creator_binding is None else "present-unchecked",
                  objects, expiry, m.ignored_tags)


# ------------------------------------------------------------- reader B
# Written separately from the record's §3 text, as one forward pass with no
# Manifest object. It shares only the two primitives with reader A. Its
# output is the triple (bundle ID, signing key, [(digest, size, kind)]).

def read_b(blob: bytes):
    class Cur:
        def __init__(self, b):
            self.b, self.i = b, 0

        def get(self, n, code):
            if self.i + n > len(self.b):
                raise BundleError(code)
            self.i += n
            return self.b[self.i - n:self.i]

        def num(self, n, code):
            return int.from_bytes(self.get(n, code), "big")

    c = Cur(blob)
    if c.get(8, "truncated_header") != MAGIC:
        raise BundleError("bad_magic")
    if c.num(2, "truncated_header") != 1:
        raise BundleError("unknown_version")
    mlen = c.num(4, "truncated_header")
    if mlen > MAX_MANIFEST:
        raise BundleError("manifest_too_large")
    body = c.get(mlen, "truncated_manifest")
    alg = c.num(1, "truncated_signature")
    sig = c.get(64, "truncated_signature")

    r = Cur(body)
    seen, prev = {}, -1
    while r.i < len(body):
        tag = r.num(1, "truncated_record")
        val = r.get(r.num(4, "truncated_record"), "truncated_record")
        if tag <= prev:
            raise BundleError("records_not_ascending")
        prev = tag
        if tag < 0x80 and tag not in (1, 2, 3, 4, 5, 6, 7, 8, 9):
            raise BundleError("unknown_critical_tag")
        seen[tag] = val
    for t in (1, 2, 3, 5, 7):
        if t not in seen:
            raise BundleError("missing_required_field")
    if seen[1] != b"id-bundle/1":
        raise BundleError("unknown_format")
    if 8 in seen:
        raise BundleError("grants_reserved_slice_2")
    if len(seen[3]) != 33 or seen[3][0] != 1:
        raise BundleError("bad_signing_key")
    try:
        ident = seen[2].decode("utf-8")
    except UnicodeDecodeError:
        raise BundleError("bad_creator_identity") from None
    if ident == "" or min(map(ord, ident)) < 0x20:
        raise BundleError("bad_creator_identity")
    if len(seen[5]) != 8:
        raise BundleError("bad_time")
    if 6 in seen:
        if len(seen[6]) != 8:
            raise BundleError("bad_time")
        if seen[6] <= seen[5]:  # equal-width big-endian bytes order like the numbers
            raise BundleError("expiry_before_creation")
    if 9 in seen and (not seen[9].isascii() or not seen[9].startswith(b"id://")):
        raise BundleError("bad_scope")
    if 0x81 in seen:
        try:
            note = seen[0x81].decode("utf-8")
        except UnicodeDecodeError:
            raise BundleError("bad_renewal_note") from None
        if note == "" or min(map(ord, note)) < 0x20:
            raise BundleError("bad_renewal_note")

    k = Cur(seen[7])
    count = k.num(4, "bad_contents")
    if count == 0:
        raise BundleError("empty_contents")
    if count > 4096:
        raise BundleError("too_many_entries")
    listed = []
    for _ in range(count):
        if k.get(k.num(1, "bad_contents"), "bad_contents") != b"raw-blake3-256":
            raise BundleError("unknown_digest_alg")
        if k.num(1, "bad_contents") != 32:
            raise BundleError("bad_digest_length")
        dg = k.get(32, "bad_contents")
        size = k.num(8, "bad_contents")
        kind = k.num(1, "bad_contents")
        if kind not in (1, 2, 3, 4):
            raise BundleError("unknown_kind")
        if size > (16 << 20):
            raise BundleError("object_too_large")
        if listed and dg <= listed[-1][0]:
            raise BundleError("entries_not_ascending")
        listed.append((dg, size, kind))
    if k.i != len(seen[7]):
        raise BundleError("bad_contents")

    if alg != 1:
        raise BundleError("unknown_signature_alg")
    if not ed25519_verify(seen[3][1:], b"id-bundle/1\x00manifest\x00" + body, sig):
        raise BundleError("bad_signature")
    for dg, size, _kind in listed:
        if blake3_256(c.get(size, "truncated_objects")) != dg:
            raise BundleError("digest_mismatch")
    if c.i != len(blob):
        raise BundleError("trailing_bytes")
    bid = "id://any/graph/v2/obj/raw-blake3-256." + blake3_256(body).hex()
    return bid, seen[3][1:], [(d.hex(), s, kd) for d, s, kd in listed]


# ------------------------------------------------------------- example

# A fixed, published seed: the example key is a test key and signs nothing else.
EXAMPLE_SEED = hashlib.sha256(b"id-bundle/1 example key, test only").digest()

# Stand-in object bytes. D3 fixes the object fields but not yet their byte
# encoding (§2.3 "field-ordered, length-prefixed"); a bundle carries whatever
# bytes the object ID is the hash of, so the format does not depend on it.
EXAMPLE_OBJECTS = [
    (1, b'{"author":"ana","body":"Is the garden meetup still on Saturday?",'
        b'"created_claim":"2026-11-24T18:00:00Z","kind":"chat.message","schema_version":1}'),
    (1, b'{"author":"ben","body":"Yes, 10am at the north gate.",'
        b'"created_claim":"2026-11-24T18:05:00Z","kind":"chat.message","schema_version":1}'),
    (3, bytes(range(256)) * 5),  # a 1280-byte blob: crosses a BLAKE3 chunk boundary
]


def build_example() -> tuple[bytes, Manifest]:
    objects = {blake3_256(b): b for _, b in EXAMPLE_OBJECTS}
    q, a = (blake3_256(b) for _, b in EXAMPLE_OBJECTS[:2])
    link = (b'{"author":"ben","context":{"previous":["' + q.hex().encode() + b'"],"root":"'
            + q.hex().encode() + b'"},"created_claim":"2026-11-24T18:05:00Z",'
            b'"schema_version":1,"source":"' + a.hex().encode() + b'","target":"'
            + q.hex().encode() + b'","verb":"reply"}')
    # A second link points at an object this bundle does not carry: allowed (§6).
    dangling = (b'{"author":"ben","created_claim":"2026-11-24T18:06:00Z","schema_version":1,'
                b'"source":"' + a.hex().encode() + b'","target":"' + ("ab" * 32).encode()
                + b'","verb":"mention"}')
    for b in (link, dangling):
        objects[blake3_256(b)] = b
    kinds = {blake3_256(b): k for k, b in EXAMPLE_OBJECTS}
    kinds.update({blake3_256(link): 2, blake3_256(dangling): 2})
    m = Manifest(
        creator_identity="idp:example/ben",
        signing_key=ed25519_public(EXAMPLE_SEED),
        created_at=1795975200,  # 2026-11-24T18:00:00Z
        expires_at=1798567200,  # 30 days later
        entries=[Entry(d, len(b), kinds[d]) for d, b in objects.items()],
        scope="id://any/graph/v2/head/raw-blake3-256." + q.hex(),
        renewal_note="Ask ben for a fresh bundle after 30 days.",
    )
    return write_bundle(m, EXAMPLE_SEED, objects), m


# ------------------------------------------------------------- self-test

def _mutations(good: bytes, m: Manifest):
    body_len = struct.unpack(">I", good[10:14])[0]
    body = good[14:14 + body_len]
    tail = good[14 + body_len + 65:]

    def rebuild(new_body, sign=True, objs=tail, alg=1):
        sig = ed25519_sign(EXAMPLE_SEED, DOMAIN + new_body) if sign else good[15 + body_len:14 + body_len + 65]
        return MAGIC + struct.pack(">HI", 1, len(new_body)) + new_body + bytes([alg]) + sig + objs

    def with_manifest(**kw):
        mm = Manifest(**{**m.__dict__, **kw})
        return rebuild(encode_manifest(mm))

    def rec(tag, value):
        return bytes([tag]) + struct.pack(">I", len(value)) + value

    yield "bad_magic", b"IDBUNDLX" + good[8:]
    yield "unknown_version", good[:8] + b"\x00\x02" + good[10:]
    yield "truncated_header", good[:10]
    yield "manifest_too_large", good[:10] + struct.pack(">I", MAX_MANIFEST + 1) + good[14:]
    yield "truncated_manifest", good[:14 + body_len - 1]
    yield "truncated_signature", good[:14 + body_len + 30]
    yield "bad_signature", good[:14 + body_len + 1] + bytes([good[15 + body_len] ^ 1]) + good[16 + body_len:]
    yield "bad_signature", rebuild(body.replace(b"idp:example/ben", b"idp:example/bem"), sign=False)
    yield "unknown_signature_alg", rebuild(body, alg=2)
    yield "digest_mismatch", good[:-1] + bytes([good[-1] ^ 1])
    yield "truncated_objects", good[:-1]
    yield "trailing_bytes", good + b"\0"
    yield "records_not_ascending", rebuild(rec(0x02, b"x") + rec(0x01, FORMAT))
    yield "records_not_ascending", rebuild(body + rec(0x81, b"again"))
    yield "unknown_critical_tag", rebuild(body[:-len(rec(0x81, m.renewal_note.encode()))]
                                          + rec(0x7f, b"") + rec(0x81, m.renewal_note.encode()))
    yield "missing_required_field", rebuild(rec(0x01, FORMAT))
    yield "unknown_format", rebuild(body.replace(FORMAT, b"id-bundle/2", 1))
    yield "grants_reserved_slice_2", rebuild(body.replace(rec(0x09, m.scope.encode()),
                                                          rec(0x08, b"") + rec(0x09, m.scope.encode())))
    yield "bad_signing_key", rebuild(body.replace(b"\x00\x00\x00\x21\x01", b"\x00\x00\x00\x21\x02", 1))
    yield "bad_creator_identity", with_manifest(creator_identity="ben\nroot")
    yield "expiry_before_creation", with_manifest(expires_at=m.created_at)
    yield "bad_scope", with_manifest(scope="https://example.com/")
    yield "bad_renewal_note", with_manifest(renewal_note="")
    yield "empty_contents", with_manifest(entries=[])
    yield "unknown_kind", with_manifest(entries=[Entry(m.entries[0].digest, 1, 9)])
    yield "object_too_large", with_manifest(entries=[Entry(m.entries[0].digest, MAX_OBJECT + 1, 1)])


def self_test() -> int:
    # Primitive vectors: BLAKE3 official empty/"abc"; RFC 8032 test 1.
    assert blake3_256(b"").hex() == "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
    assert blake3_256(b"abc").hex() == "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85"
    seed = bytes.fromhex("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60")
    assert ed25519_public(seed).hex() == "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
    sig = ed25519_sign(seed, b"")
    assert sig.hex() == ("e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e06522490155"
                         "5fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b")
    assert ed25519_verify(ed25519_public(seed), b"", sig)
    assert not ed25519_verify(ed25519_public(seed), b"x", sig)

    good, m = build_example()
    assert hashlib.sha256(good).hexdigest() == EXAMPLE_SHA256, hashlib.sha256(good).hexdigest()
    import os
    committed = os.path.join(os.path.dirname(os.path.abspath(__file__)), "example.idbundle")
    if os.path.exists(committed):
        assert open(committed, "rb").read() == good, "example.idbundle differs from build_example()"
    ra = read_a(good, now=1796000000)
    rb = read_b(good)
    assert ra.bundle_id == rb[0]
    assert [(d, s) for _, d, s in ra.objects] == [(d, s) for d, s, _ in rb[2]]
    assert ra.binding == "unbound" and ra.expiry == "valid" and len(ra.objects) == 5
    assert read_a(good, now=1798567200).expiry == "expired"

    rejects = 0
    for code, blob in _mutations(good, m):
        got = []
        for reader in (read_a, read_b):
            try:
                reader(blob)
                got.append("accepted")
            except BundleError as e:
                got.append(str(e))
        assert got == [code, code], (code, got)
        rejects += 1

    # Non-critical unknown tag: both readers accept, the bundle ID changes.
    body_len = struct.unpack(">I", good[10:14])[0]
    body = good[14:14 + body_len] + b"\xf0\x00\x00\x00\x01z"
    extra = (MAGIC + struct.pack(">HI", 1, len(body)) + body + b"\x01"
             + ed25519_sign(EXAMPLE_SEED, DOMAIN + body) + good[14 + body_len + 65:])
    assert read_a(extra).ignored_tags == [0xF0] and read_b(extra)[0] != rb[0]

    print(f"ok: example bundle {len(good)} bytes, {len(ra.objects)} objects, "
          f"both readers agree on {ra.bundle_id}; {rejects} rejection cases agree")
    return 0


def cross_check() -> int:
    import os
    try:
        import blake3  # type: ignore
        for n in (0, 1, 63, 64, 65, 1023, 1024, 1025, 2048, 2049, 3072, 5000, 8193, 70000):
            data = os.urandom(n)
            assert blake3_256(data) == blake3.blake3(data).digest(), n
        print("blake3: agrees with the blake3 package on 14 lengths")
    except ImportError:
        print("blake3: package not installed, skipped")
    try:
        from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
        from cryptography.hazmat.primitives import serialization
        for _ in range(8):
            seed, msg = os.urandom(32), os.urandom(100)
            k = Ed25519PrivateKey.from_private_bytes(seed)
            pub = k.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)
            assert pub == ed25519_public(seed) and k.sign(msg) == ed25519_sign(seed, msg)
        print("ed25519: agrees with the cryptography package on 8 keys")
    except ImportError:
        print("ed25519: package not installed, skipped")
    return 0


if __name__ == "__main__":
    if sys.argv[1:2] == ["--write"]:
        open(sys.argv[2], "wb").write(build_example()[0])
        sys.exit(0)
    if sys.argv[1:2] == ["--cross-check"]:
        sys.exit(cross_check())
    sys.exit(self_test())
