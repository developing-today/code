#!/usr/bin/env python3
"""#10706 reference: a world announcement carried in a pkarr signed packet.

Stdlib only.

  python3 -I pkarr_reference.py check   # run every check against vectors.json
  python3 -I pkarr_reference.py write   # regenerate vectors.json

What it pins (note.md in the parent folder is the prose):

- RFC 8032 ed25519, cross-checked by reproducing the signature that
  pkgs/id/crates/id-sync/src/announcement.rs already pins (seed 0x07 x 32
  over WORLD_BODY_HEX), so the CBOR encoder here is checked byte for byte too.
- The DNS packet: one TXT record named `_ida.<z32 world key>`, whose rdata is
  the signed announcement's CBOR split into binary character-strings.
- The pkarr relay payload: signature(64) || timestamp_us_be(8) || dns_packet,
  the signature being BEP44 over bencoded `3:seqi<ts>e1:v<len>:<dns>`.
- The budget: the largest signed announcement that still fits the 1000-byte
  DNS packet limit.
- The resolver's decode rules, including the refusals.
"""

import hashlib
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
VECTORS = HERE / "vectors.json"

# ---------------------------------------------------------------------------
# RFC 8032 ed25519 (section 5.1), written for clarity rather than speed.

P = 2**255 - 19
L = 2**252 + 27742317777372353535851937790883648493
D = -121665 * pow(121666, P - 2, P) % P
SQRT_M1 = pow(2, (P - 1) // 4, P)


def _sha512_int(data):
    return int.from_bytes(hashlib.sha512(data).digest(), "little")


def _add(a, b):
    x1, y1, z1, t1 = a
    x2, y2, z2, t2 = b
    a_ = (y1 - x1) * (y2 - x2) % P
    b_ = (y1 + x1) * (y2 + x2) % P
    c_ = 2 * t1 * t2 * D % P
    d_ = 2 * z1 * z2 % P
    e, f, g, h = b_ - a_, d_ - c_, d_ + c_, b_ + a_
    return (e * f % P, g * h % P, f * g % P, e * h % P)


def _mul(s, point):
    q = (0, 1, 1, 0)
    while s > 0:
        if s & 1:
            q = _add(q, point)
        point = _add(point, point)
        s >>= 1
    return q


def _equal(a, b):
    x1, y1, z1, _ = a
    x2, y2, z2, _ = b
    return (x1 * z2 - x2 * z1) % P == 0 and (y1 * z2 - y2 * z1) % P == 0


def _recover_x(y, sign):
    if y >= P:
        return None
    x2 = (y * y - 1) * pow(D * y * y + 1, P - 2, P)
    if x2 == 0:
        return None if sign else 0
    x = pow(x2, (P + 3) // 8, P)
    if (x * x - x2) % P != 0:
        x = x * SQRT_M1 % P
    if (x * x - x2) % P != 0:
        return None
    if (x & 1) != sign:
        x = P - x
    return x


_GY = 4 * pow(5, P - 2, P) % P
_GX = _recover_x(_GY, 0)
G = (_GX, _GY, 1, _GX * _GY % P)


def _compress(point):
    x, y, z, _ = point
    zinv = pow(z, P - 2, P)
    x, y = x * zinv % P, y * zinv % P
    return int.to_bytes(y | ((x & 1) << 255), 32, "little")


def _decompress(data):
    if len(data) != 32:
        return None
    y = int.from_bytes(data, "little")
    sign = y >> 255
    y &= (1 << 255) - 1
    x = _recover_x(y, sign)
    if x is None:
        return None
    return (x, y, 1, x * y % P)


def _expand(seed):
    h = hashlib.sha512(seed).digest()
    a = int.from_bytes(h[:32], "little")
    a &= (1 << 254) - 8
    a |= 1 << 254
    return a, h[32:]


def public_key(seed):
    a, _ = _expand(seed)
    return _compress(_mul(a, G))


def sign(seed, message):
    a, prefix = _expand(seed)
    pub = _compress(_mul(a, G))
    r = _sha512_int(prefix + message) % L
    big_r = _compress(_mul(r, G))
    h = _sha512_int(big_r + pub + message) % L
    s = (r + h * a) % L
    return big_r + int.to_bytes(s, 32, "little")


def verify(pub, message, signature):
    if len(signature) != 64:
        return False
    a = _decompress(pub)
    big_r = _decompress(signature[:32])
    if a is None or big_r is None:
        return False
    s = int.from_bytes(signature[32:], "little")
    if s >= L:
        return False
    h = _sha512_int(signature[:32] + pub + message) % L
    return _equal(_mul(s, G), _add(big_r, _mul(h, a)))


# ---------------------------------------------------------------------------
# Canonical CBOR, the subset the announcement uses (cbor_core's ordering:
# map keys sorted by encoded length, then bytewise).


def _head(major, value):
    if value < 24:
        return bytes([major << 5 | value])
    for extra, width in ((24, 1), (25, 2), (26, 4), (27, 8)):
        if value < 1 << (8 * width):
            return bytes([major << 5 | extra]) + value.to_bytes(width, "big")
    raise ValueError("integer too large")


def cbor_encode(value):
    if value is None:
        return b"\xf6"
    if isinstance(value, bool):
        return b"\xf5" if value else b"\xf4"
    if isinstance(value, int):
        if value < 0:
            raise ValueError("negative integers are not used")
        return _head(0, value)
    if isinstance(value, bytes):
        return _head(2, len(value)) + value
    if isinstance(value, str):
        raw = value.encode()
        return _head(3, len(raw)) + raw
    if isinstance(value, list):
        return _head(4, len(value)) + b"".join(cbor_encode(v) for v in value)
    if isinstance(value, dict):
        items = sorted(
            ((cbor_encode(k), cbor_encode(v)) for k, v in value.items()),
            key=lambda kv: (len(kv[0]), kv[0]),
        )
        return _head(5, len(items)) + b"".join(k + v for k, v in items)
    raise TypeError(type(value))


def cbor_decode(data):
    value, end = _cbor_item(data, 0)
    if end != len(data):
        raise ValueError("trailing bytes after CBOR item")
    return value


def _cbor_item(data, i):
    if i >= len(data):
        raise ValueError("truncated CBOR")
    initial = data[i]
    major, info = initial >> 5, initial & 31
    i += 1
    if major == 7:
        simple = {20: False, 21: True, 22: None}
        if info not in simple:
            raise ValueError("unsupported simple value")
        return simple[info], i
    if info < 24:
        arg = info
    elif info in (24, 25, 26, 27):
        width = 1 << (info - 24)
        if i + width > len(data):
            raise ValueError("truncated CBOR")
        arg = int.from_bytes(data[i : i + width], "big")
        i += width
    else:
        raise ValueError("indefinite or reserved CBOR length")
    if major == 0:
        return arg, i
    if major in (2, 3):
        if i + arg > len(data):
            raise ValueError("truncated CBOR")
        raw = bytes(data[i : i + arg])
        return (raw if major == 2 else raw.decode()), i + arg
    if major == 4:
        out = []
        for _ in range(arg):
            item, i = _cbor_item(data, i)
            out.append(item)
        return out, i
    if major == 5:
        out = {}
        for _ in range(arg):
            key, i = _cbor_item(data, i)
            val, i = _cbor_item(data, i)
            out[key] = val
        return out, i
    raise ValueError("unsupported CBOR major type")


# ---------------------------------------------------------------------------
# The announcement, in the serde shape announcement.rs encodes.


def announcement(world_seed, node_seed, *, seq, issued_at, expires_at):
    return {
        "world_id": public_key(world_seed),
        "name": "harbor",
        "description": "a quiet test world",
        "subject": {
            "endpoint": {
                "node_id": public_key(node_seed),
                "relay": "https://relay.example.net/",
            }
        },
        "occupancy": "has_room",
        "tags": ["calm", "test"],
        "visibility": "listed",
        "issued_at": issued_at,
        "expires_at": expires_at,
        "seq": seq,
    }


def sign_announcement(world_seed, body):
    encoded = cbor_encode(body)
    return cbor_encode({"announcement": body, "signature": sign(world_seed, encoded)})


# ---------------------------------------------------------------------------
# z-base-32, as pkarr names keys.

Z32 = "ybndrfg8ejkmcpqxot1uwisza345h769"


def z32_encode(data):
    bits = int.from_bytes(data, "big")
    nbits = len(data) * 8
    chars = (nbits + 4) // 5
    bits <<= chars * 5 - nbits
    return "".join(Z32[(bits >> (5 * (chars - 1 - i))) & 31] for i in range(chars))


# ---------------------------------------------------------------------------
# The DNS packet.

MAX_DNS_PACKET = 1000
RECORD_LABEL = "_ida"
RECORD_TTL = 300
TYPE_TXT = 16
CLASS_IN = 1
FLAGS = 0x8400  # QR + AA: a response, as pkarr builds its packets


def _name(labels):
    out = b""
    for label in labels:
        raw = label.encode()
        if not 0 < len(raw) < 64:
            raise ValueError("bad DNS label")
        out += bytes([len(raw)]) + raw
    return out + b"\x00"


def dns_packet(world_key, signed):
    chunks = [signed[i : i + 255] for i in range(0, len(signed), 255)]
    rdata = b"".join(bytes([len(c)]) + c for c in chunks)
    record = (
        _name([RECORD_LABEL, z32_encode(world_key)])
        + TYPE_TXT.to_bytes(2, "big")
        + CLASS_IN.to_bytes(2, "big")
        + RECORD_TTL.to_bytes(4, "big")
        + len(rdata).to_bytes(2, "big")
        + rdata
    )
    header = (
        (0).to_bytes(2, "big")
        + FLAGS.to_bytes(2, "big")
        + bytes([0, 0, 0, 1, 0, 0, 0, 0])
    )
    packet = header + record
    if len(packet) > MAX_DNS_PACKET:
        raise ValueError(f"DNS packet is {len(packet)} bytes, over {MAX_DNS_PACKET}")
    return packet


def packet_size(signed_len):
    """DNS packet bytes for a signed announcement of `signed_len` bytes."""
    name = 1 + len(RECORD_LABEL) + 1 + 52 + 1
    return 12 + name + 10 + signed_len + -(-signed_len // 255)


def max_signed_bytes():
    n = 0
    while packet_size(n + 1) <= MAX_DNS_PACKET:
        n += 1
    return n


def _read_name(packet, i):
    labels = []
    while True:
        if i >= len(packet):
            raise ValueError("truncated name")
        length = packet[i]
        if length & 0xC0:
            raise ValueError("compressed names are not used in announcement packets")
        i += 1
        if length == 0:
            return labels, i
        labels.append(packet[i : i + length].decode("ascii").lower())
        i += length


def read_announcement_record(packet, world_key):
    """Every `_ida` TXT rdata in the packet, as joined character-strings."""
    if len(packet) < 12 or len(packet) > MAX_DNS_PACKET:
        raise ValueError("DNS packet size out of range")
    qd, an, ns, ar = (
        int.from_bytes(packet[4 + 2 * k : 6 + 2 * k], "big") for k in range(4)
    )
    i = 12
    for _ in range(qd):
        _, i = _read_name(packet, i)
        i += 4
    origin = z32_encode(world_key)
    found = []
    for _ in range(an + ns + ar):
        labels, i = _read_name(packet, i)
        rtype = int.from_bytes(packet[i : i + 2], "big")
        rclass = int.from_bytes(packet[i + 2 : i + 4], "big")
        rdlen = int.from_bytes(packet[i + 8 : i + 10], "big")
        i += 10
        rdata = packet[i : i + rdlen]
        if len(rdata) != rdlen:
            raise ValueError("truncated rdata")
        i += rdlen
        named = labels in ([RECORD_LABEL], [RECORD_LABEL, origin])
        if named and rtype == TYPE_TXT and rclass == CLASS_IN:
            joined, j = b"", 0
            while j < len(rdata):
                n = rdata[j]
                if j + 1 + n > len(rdata):
                    raise ValueError("truncated character-string")
                joined += rdata[j + 1 : j + 1 + n]
                j += 1 + n
            found.append(joined)
    if i != len(packet):
        raise ValueError("trailing bytes after the records")
    return found


# ---------------------------------------------------------------------------
# The relay payload and the BEP44 signature.


def bep44_signable(timestamp_us, dns):
    return (
        b"3:seqi"
        + str(timestamp_us).encode()
        + b"e1:v"
        + str(len(dns)).encode()
        + b":"
        + dns
    )


def relay_payload(world_seed, timestamp_us, dns):
    signature = sign(world_seed, bep44_signable(timestamp_us, dns))
    return signature + timestamp_us.to_bytes(8, "big") + dns


def resolve(world_key, payload, now):
    """The resolver's checks, in order. Returns the announcement body or raises."""
    if len(payload) < 72:
        raise ValueError("relay payload is shorter than its header")
    signature, timestamp_us, dns = (
        payload[:64],
        int.from_bytes(payload[64:72], "big"),
        payload[72:],
    )
    if not verify(world_key, bep44_signable(timestamp_us, dns), signature):
        raise ValueError("pkarr signature does not verify under the queried key")
    records = read_announcement_record(dns, world_key)
    if len(records) != 1:
        raise ValueError(
            f"packet carries {len(records)} announcement records, not exactly one"
        )
    signed = cbor_decode(records[0])
    if set(signed) != {"announcement", "signature"}:
        raise ValueError("record is not a signed announcement")
    body = signed["announcement"]
    if body.get("world_id") != world_key:
        raise ValueError("announcement world_id is not the queried key")
    if not verify(world_key, cbor_encode(body), signed["signature"]):
        raise ValueError("announcement signature does not verify")
    if body["seq"] != timestamp_us:
        raise ValueError("announcement seq is not the packet timestamp")
    if now >= body["expires_at"]:
        raise ValueError("announcement is expired")
    return body


# ---------------------------------------------------------------------------
# The publisher's timing rules.

DHT_RETENTION_S = 2 * 60 * 60
DEFAULT_LIFETIME_S = 3 * 60 * 60


def next_timestamp(now_us, previous_us):
    return now_us if previous_us is None else max(now_us, previous_us + 1)


def heartbeat_seconds(lifetime_s):
    return min(lifetime_s, DHT_RETENTION_S) // 2


# ---------------------------------------------------------------------------
# Vectors and checks.

WORLD_SEED = bytes([7] * 32)
NODE_SEED = bytes([8] * 32)
OTHER_SEED = bytes([9] * 32)
ISSUED = 1_763_500_000
PINNED_BODY_HEX = (
    "aa637365711829646e616d6566686172626f726474616773826463616c6d6474657374677375626a656374a168656e64"
    "706f696e74a26572656c6179781a68747470733a2f2f72656c61792e6578616d706c652e6e65742f676e6f64655f6964"
    "58201398f62c6d1a457c51ba6a4b5f3dbd2f69fca93216218dc8997e416bd17d93ca68776f726c645f69645820ea4a6c"
    "63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c696973737565645f61741a691cdfe0696f6363"
    "7570616e6379686861735f726f6f6d6a657870697265735f61741a6920d4606a7669736962696c697479666c69737465"
    "646b6465736372697074696f6e7261207175696574207465737420776f726c64"
)
PINNED_SIGNATURE_HEX = (
    "0ba4d643e3c7e3428b8a3e320f024b180d0d3711847ca094ed466456840325669b4981e3f57d70888a2053df906b59f3"
    "f27dca44488a8a78b93a1a0ed3dd4800"
)


def build_vectors():
    timestamp_us = ISSUED * 1_000_000
    body = announcement(
        WORLD_SEED,
        NODE_SEED,
        seq=timestamp_us,
        issued_at=ISSUED,
        expires_at=ISSUED + DEFAULT_LIFETIME_S,
    )
    signed = sign_announcement(WORLD_SEED, body)
    world_key = public_key(WORLD_SEED)
    dns = dns_packet(world_key, signed)
    payload = relay_payload(WORLD_SEED, timestamp_us, dns)
    return {
        "world_key_hex": world_key.hex(),
        "world_key_z32": z32_encode(world_key),
        "timestamp_us": timestamp_us,
        "body_hex": cbor_encode(body).hex(),
        "signed_announcement_hex": signed.hex(),
        "dns_packet_hex": dns.hex(),
        "bep44_signable_hex": bep44_signable(timestamp_us, dns).hex(),
        "relay_payload_hex": payload.hex(),
        "max_signed_announcement_bytes": max_signed_bytes(),
        "heartbeat_seconds_default": heartbeat_seconds(DEFAULT_LIFETIME_S),
    }


def _expect_refusal(failures, label, fn):
    try:
        fn()
    except (ValueError, KeyError, UnicodeDecodeError):
        return
    failures.append(f"{label}: accepted, expected a refusal")


def run_checks():
    failures = []

    def check(label, ok):
        if not ok:
            failures.append(label)

    # 1. The ed25519 and CBOR here reproduce announcement.rs's pinned vector.
    pinned = announcement(
        WORLD_SEED,
        NODE_SEED,
        seq=41,
        issued_at=ISSUED,
        expires_at=ISSUED + 3 * 24 * 60 * 60,
    )
    check(
        "CBOR body reproduces WORLD_BODY_HEX",
        cbor_encode(pinned).hex() == PINNED_BODY_HEX,
    )
    check(
        "ed25519 reproduces WORLD_SIGNATURE_HEX",
        sign(WORLD_SEED, bytes.fromhex(PINNED_BODY_HEX)).hex() == PINNED_SIGNATURE_HEX,
    )
    check("RFC 8032 test 1 public key", public_key(bytes(32)).hex()[:8] == "3b6a27bc")

    # 2. The committed vectors match a fresh build.
    vectors = build_vectors()
    stored = json.loads(VECTORS.read_text())
    for key, value in vectors.items():
        check(f"vector {key} matches vectors.json", stored.get(key) == value)

    world_key = bytes.fromhex(vectors["world_key_hex"])
    payload = bytes.fromhex(vectors["relay_payload_hex"])
    now = ISSUED + 60

    # 3. The happy path.
    try:
        body = resolve(world_key, payload, now)
        check(
            "resolved body is the published one",
            cbor_encode(body).hex() == vectors["body_hex"],
        )
    except ValueError as err:
        failures.append(f"happy path refused: {err}")

    # 4. The budget: the largest signed announcement fits, one more byte does not.
    limit = vectors["max_signed_announcement_bytes"]
    check("budget limit fits", packet_size(limit) <= MAX_DNS_PACKET)
    check("budget limit + 1 does not fit", packet_size(limit + 1) > MAX_DNS_PACKET)
    check(
        "packet_size agrees with dns_packet",
        packet_size(len(bytes.fromhex(vectors["signed_announcement_hex"])))
        == len(bytes.fromhex(vectors["dns_packet_hex"])),
    )
    big = bytes(limit)
    check(
        "a max-size payload builds", len(dns_packet(world_key, big)) <= MAX_DNS_PACKET
    )
    _expect_refusal(
        failures,
        "an over-budget payload",
        lambda: dns_packet(world_key, bytes(limit + 1)),
    )

    # 5. A relative `_ida` name decodes the same as the full name.
    signed = bytes.fromhex(vectors["signed_announcement_hex"])
    dns = bytes.fromhex(vectors["dns_packet_hex"])
    origin = _name([RECORD_LABEL, z32_encode(world_key)])
    relative = dns.replace(origin, _name([RECORD_LABEL]), 1)
    check(
        "relative name decodes",
        read_announcement_record(relative, world_key) == [signed],
    )

    # 6. Refusals.
    ts = vectors["timestamp_us"]
    tampered = bytearray(payload)
    tampered[-1] ^= 1
    _expect_refusal(
        failures, "a tampered packet", lambda: resolve(world_key, bytes(tampered), now)
    )
    _expect_refusal(
        failures,
        "a packet for another key",
        lambda: resolve(public_key(OTHER_SEED), payload, now),
    )
    _expect_refusal(
        failures,
        "an expired announcement",
        lambda: resolve(world_key, payload, ISSUED + DEFAULT_LIFETIME_S),
    )

    # Another world's valid announcement republished under this key.
    other_body = announcement(
        OTHER_SEED, NODE_SEED, seq=ts, issued_at=ISSUED, expires_at=ISSUED + 3600
    )
    foreign = dns_packet(world_key, sign_announcement(OTHER_SEED, other_body))
    _expect_refusal(
        failures,
        "a foreign announcement under this key",
        lambda: resolve(world_key, relay_payload(WORLD_SEED, ts, foreign), now),
    )

    # The inner seq must equal the packet timestamp.
    _expect_refusal(
        failures,
        "a seq that is not the timestamp",
        lambda: resolve(world_key, relay_payload(WORLD_SEED, ts + 1, dns), now),
    )

    # Two announcement records.
    rr = dns[12:]
    doubled = dns[:6] + (2).to_bytes(2, "big") + dns[8:12] + rr + rr
    _expect_refusal(
        failures,
        "two announcement records",
        lambda: resolve(world_key, relay_payload(WORLD_SEED, ts, doubled), now),
    )

    # No announcement record at all.
    renamed = dns.replace(b"\x04_ida", b"\x04_idb", 1)
    _expect_refusal(
        failures,
        "no announcement record",
        lambda: resolve(world_key, relay_payload(WORLD_SEED, ts, renamed), now),
    )

    # 7. Timing.
    check("timestamp is now when there is no previous", next_timestamp(5, None) == 5)
    check(
        "timestamp advances past a previous in the future", next_timestamp(5, 9) == 10
    )
    check(
        "default heartbeat is one hour", heartbeat_seconds(DEFAULT_LIFETIME_S) == 3600
    )
    check("a short lifetime halves itself", heartbeat_seconds(1200) == 600)

    return failures


def main(argv):
    command = argv[1] if len(argv) > 1 else "check"
    if command == "write":
        VECTORS.write_text(json.dumps(build_vectors(), indent=2) + "\n")
        print(f"wrote {VECTORS.name}")
        return 0
    if command != "check":
        print(__doc__)
        return 2
    failures = run_checks()
    for failure in failures:
        print(f"FAIL {failure}")
    print(f"pkarr reference: {len(failures)} failures")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
