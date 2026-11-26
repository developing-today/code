#!/usr/bin/env python3
"""Reference carrier profiles for ID sneakernet bundles (D4 slice 5, #13889).
Python stdlib only; loads the slice-1 writer and readers from
bundle_reference.py beside it.

A carrier profile only tightens slice 1's format ceilings and says how the
bundle's bytes ride on the carrier. The bundle bytes never change: a reader
recovers exactly the slice-1 file and verifies it with slice 1's reader.

Profiles:

  removable-media  one bundle per *.idbundle file; at most 2**32 - 1 bytes
                   (the FAT32 file limit, the lowest common filesystem)
  email            one bundle per attachment, base64 in MIME; the whole
                   message stays under a 10,000,000-byte budget
  qr-page          one QR code, level M, version <= 40, "IDB1:" + base45
  qr-small         one QR code, level M, version <= 25, same text

Bigger sets are split into independent parts, each a complete signed bundle
(no part depends on another: slice 1 §6 already accepts dangling links).

Run with no arguments (or self-test) for the self-test. --cross-check also
checks the QR capacity table and versions against the `segno` package when
it is installed. --table prints the ceilings.
"""

from __future__ import annotations

import importlib.util
import math
import pathlib
import struct
import sys
from dataclasses import dataclass
from email.message import EmailMessage

sys.dont_write_bytecode = True

_HERE = pathlib.Path(__file__).resolve().parent


def _load(name: str):
    spec = importlib.util.spec_from_file_location(name, _HERE / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


br = _load("bundle_reference")


class CarrierError(ValueError):
    """Rejection or refusal; the message is the stable code."""


# ------------------------------------------------------------- base45
# RFC 9285. Its 45 characters are exactly the QR alphanumeric set, in order.

B45 = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ $%*+-./:"
_B45_INDEX = {c: i for i, c in enumerate(B45)}


def b45encode(data: bytes) -> str:
    out = []
    for i in range(0, len(data) - 1, 2):
        n = data[i] * 256 + data[i + 1]
        out += [B45[n % 45], B45[n // 45 % 45], B45[n // 2025]]
    if len(data) % 2:
        n = data[-1]
        out += [B45[n % 45], B45[n // 45]]
    return "".join(out)


def b45decode(text: str) -> bytes:
    """Strict: every byte string has exactly one encoding, so anything that
    is not that encoding is refused."""
    if len(text) % 3 == 1:
        raise CarrierError("qr_bad_base45")
    try:
        vals = [_B45_INDEX[c] for c in text]
    except KeyError:
        raise CarrierError("qr_bad_base45") from None
    out = bytearray()
    for i in range(0, len(vals), 3):
        chunk = vals[i : i + 3]
        if len(chunk) == 3:
            n = chunk[0] + chunk[1] * 45 + chunk[2] * 2025
            if n > 0xFFFF:
                raise CarrierError("qr_bad_base45")
            out += n.to_bytes(2, "big")
        else:
            n = chunk[0] + chunk[1] * 45
            if n > 0xFF:
                raise CarrierError("qr_bad_base45")
            out.append(n)
    return bytes(out)


# ------------------------------------------------------------- QR capacity
# Data codewords per version 1-40 at error correction level M (ISO/IEC
# 18004 table 7). --cross-check compares them with segno.

QR_DATA_CODEWORDS_M = (
    16,
    28,
    44,
    64,
    86,
    108,
    124,
    154,
    182,
    216,
    254,
    290,
    334,
    365,
    415,
    453,
    507,
    563,
    627,
    669,
    714,
    782,
    860,
    914,
    1000,
    1062,
    1128,
    1193,
    1267,
    1373,
    1455,
    1541,
    1631,
    1725,
    1812,
    1914,
    1992,
    2102,
    2216,
    2334,
)


def qr_alnum_capacity(version: int) -> int:
    """Characters an alphanumeric-mode segment can hold at level M."""
    cci = 9 if version <= 9 else 11 if version <= 26 else 13
    bits = QR_DATA_CODEWORDS_M[version - 1] * 8 - 4 - cci
    return bits // 11 * 2 + (1 if bits % 11 >= 6 else 0)


def qr_version_for(chars: int, max_version: int = 40) -> int | None:
    return next(
        (v for v in range(1, max_version + 1) if chars <= qr_alnum_capacity(v)), None
    )


def qr_side_mm(version: int, module_mm: float) -> float:
    """Printed side including the 4-module quiet zone on each side."""
    return (17 + 4 * version + 8) * module_mm


QR_PREFIX = "IDB1:"


def qr_text(bundle: bytes) -> str:
    return QR_PREFIX + b45encode(bundle)


def qr_read(text: str, now: int | None = None):
    """What a scanner does with the text of a bundle code: strip the prefix,
    decode strictly, then hand the bytes to slice 1's reader unchanged."""
    if not text.startswith(QR_PREFIX):
        raise CarrierError("qr_not_a_bundle")
    return br.read_a(b45decode(text[len(QR_PREFIX) :]), now)


# ------------------------------------------------------------- email
# MIME base64 is 4 characters per 3 bytes in lines of 76 plus CRLF. The
# headers, the free-text body and the MIME boundaries get a fixed reserve.

EMAIL_BUDGET = 10_000_000
EMAIL_RESERVE = 64 * 1024


def email_attachment_bytes(raw: int) -> int:
    chars = 4 * math.ceil(raw / 3)
    return chars + 2 * math.ceil(chars / 76)


def email_raw_ceiling(budget: int = EMAIL_BUDGET, reserve: int = EMAIL_RESERVE) -> int:
    lo, hi = 0, budget
    while lo < hi:
        mid = (lo + hi + 1) // 2
        if email_attachment_bytes(mid) <= budget - reserve:
            lo = mid
        else:
            hi = mid - 1
    return lo


def email_message(bundle: bytes, note: str) -> bytes:
    """The shape the profile asks for: a free-text body and one attachment.
    Nothing outside the attachment is trusted by the receiver."""
    msg = EmailMessage()
    msg["Subject"] = "ID bundle"
    msg.set_content(note)
    msg.add_attachment(
        bundle,
        maintype="application",
        subtype="octet-stream",
        filename=media_filename(bundle),
    )
    return msg.as_bytes(policy=msg.policy.clone(linesep="\r\n"))


def email_read(raw_message: bytes, now: int | None = None):
    import email
    from email import policy

    msg = email.message_from_bytes(raw_message, policy=policy.default)
    parts = [
        p
        for p in msg.iter_attachments()
        if (p.get_filename() or "").endswith(".idbundle")
    ]
    if len(parts) != 1:
        raise CarrierError("email_needs_one_bundle")
    return br.read_a(parts[0].get_content(), now)


# ------------------------------------------------------------- media

FAT32_MAX_FILE = 2**32 - 1
MEDIA_DIR = "id-bundles"


def media_filename(bundle: bytes) -> str:
    """Advisory: the first 16 hex digits of the bundle ID. A reader ignores
    the name and computes the ID from the manifest."""
    body_len = struct.unpack(">I", bundle[10:14])[0]
    return br.blake3_256(bundle[14 : 14 + body_len]).hex()[:16] + ".idbundle"


FORMAT_MAX = 8 + 2 + 4 + br.MAX_MANIFEST + 1 + 64 + br.MAX_ENTRIES * br.MAX_OBJECT


# ------------------------------------------------------------- profiles


@dataclass(frozen=True)
class Carrier:
    name: str
    max_bundle: int  # raw bundle bytes the carrier takes
    private: bool  # can only the people it is handed to read it?
    note: str


def qr_raw_ceiling(max_version: int) -> int:
    chars = qr_alnum_capacity(max_version) - len(QR_PREFIX)
    return chars // 3 * 2 + (1 if chars % 3 == 2 else 0)


CARRIERS = {
    c.name: c
    for c in (
        Carrier(
            "removable-media",
            FAT32_MAX_FILE,
            True,
            "private only while the stick is in the right hands",
        ),
        Carrier(
            "email",
            email_raw_ceiling(),
            False,
            "mail providers on the path can read it",
        ),
        Carrier(
            "qr-page",
            qr_raw_ceiling(40),
            False,
            "anyone who sees the printout can read it",
        ),
        Carrier(
            "qr-small",
            qr_raw_ceiling(25),
            False,
            "anyone who sees the printout can read it",
        ),
    )
}


# ------------------------------------------------------------- splitting

HEADER_AND_SIG = 8 + 2 + 4 + 1 + 64
ENTRY = 1 + len(br.DIGEST_ALG) + 1 + 32 + 8 + 1


def bundle_size(template: br.Manifest, sizes: list[int]) -> int:
    """Exact size of a bundle with this manifest template and these objects,
    without signing anything."""
    m = br.Manifest(
        **{
            **template.__dict__,
            "entries": [
                br.Entry(i.to_bytes(32, "big"), s, 1) for i, s in enumerate(sizes)
            ],
        }
    )
    return HEADER_AND_SIG + len(br.encode_manifest(m)) + sum(sizes)


def split(
    template: br.Manifest,
    seed: bytes,
    objects: dict[bytes, tuple[int, bytes]],
    carrier: Carrier,
) -> list[bytes]:
    """Next-fit in digest order: deterministic, so two writers holding the
    same set make the same parts. Each part is a whole signed bundle with the
    template's creator, times and scope."""
    order = sorted(objects)
    for d in order:
        if bundle_size(template, [len(objects[d][1])]) > carrier.max_bundle:
            raise CarrierError("object_too_big_for_carrier")
    parts, current = [], []
    for d in order:
        trial = current + [d]
        too_big = (
            bundle_size(template, [len(objects[x][1]) for x in trial])
            > carrier.max_bundle
        )
        if too_big or len(trial) > br.MAX_ENTRIES:
            parts.append(current)
            current = [d]
        else:
            current = trial
    parts.append(current)
    out = []
    for part in parts:
        m = br.Manifest(
            **{
                **template.__dict__,
                "entries": [
                    br.Entry(d, len(objects[d][1]), objects[d][0]) for d in part
                ],
            }
        )
        out.append(br.write_bundle(m, seed, {d: objects[d][1] for d in part}))
    return out


# ------------------------------------------------------------- ceilings


def chat_object(i: int) -> bytes:
    """Same shape and length as the slice-1 example's chat messages."""
    return (
        b'{"author":"ana","body":"Garden thread note '
        + f"{i:04d}".encode()
        + b' about Saturday.","created_claim":"2026-11-24T18:00:00Z",'
        b'"kind":"chat.message","schema_version":1}'
    )


def link_object(i: int) -> bytes:
    """Same shape as the slice-1 example's reply link: two 64-hex digests."""
    a, b = br.blake3_256(b"a%d" % i).hex(), br.blake3_256(b"b%d" % i).hex()
    return (
        b'{"author":"ben","context":{"previous":["'
        + b.encode()
        + b'"],"root":"'
        + b.encode()
        + b'"},"created_claim":"2026-11-24T18:05:00Z",'
        b'"schema_version":1,"source":"'
        + a.encode()
        + b'","target":"'
        + b.encode()
        + b'","verb":"reply"}'
    )


def qr_template(binding: int = 0) -> br.Manifest:
    """What a QR export carries: required fields only, plus a creator
    binding of `binding` bytes once slice 3 makes one mandatory."""
    return br.Manifest(
        creator_identity="idp:example/ben",
        signing_key=br.ed25519_public(br.EXAMPLE_SEED),
        created_at=1795975200,
        entries=[],
        creator_binding=b"\x00" * binding if binding else None,
    )


def max_objects(carrier: Carrier, make, template: br.Manifest) -> tuple[int, bytes]:
    """Largest count of `make` objects in one real, signed bundle that fits;
    returns the count and that bundle."""
    best, n = b"", 0
    while True:
        objs = {br.blake3_256(make(i)): (1, make(i)) for i in range(n + 1)}
        if (
            bundle_size(template, [len(v[1]) for v in objs.values()])
            > carrier.max_bundle
        ):
            return n, best
        n += 1
        best = split(template, br.EXAMPLE_SEED, objs, carrier)[0]


# ------------------------------------------------------------- self-test


def fail(msg: str) -> int:
    print(f"FAIL: {msg}")
    return 1


def self_test() -> int:
    # base45: RFC 9285 section 4.3 and 4.4 examples.
    for raw, enc in (
        (b"AB", "BB8"),
        (b"Hello!!", "%69 VD92EX0"),
        (b"base-45", "UJCLQE7W581"),
        (b"ietf!", "QED8WEX0"),
    ):
        if b45encode(raw) != enc or b45decode(enc) != raw:
            return fail(f"base45 {raw!r}")
    for bad in ("GGW", "GGWGG", "A", "ABCD", "bb8", "BB8=", "::"):
        try:
            b45decode(bad)
            return fail(f"base45 accepted {bad!r}")
        except CarrierError as exc:
            if str(exc) != "qr_bad_base45":
                return fail(f"base45 {bad!r}: {exc}")
    for n in range(260):
        data = bytes((i * 37 + n) % 256 for i in range(n))
        if b45decode(b45encode(data)) != data or any(
            c not in B45 for c in b45encode(data)
        ):
            return fail(f"base45 round trip {n}")

    # QR capacity: published alphanumeric capacities at level M.
    for v, cap in (
        (1, 20),
        (2, 38),
        (10, 311),
        (12, 419),
        (13, 483),
        (25, 1451),
        (40, 3391),
    ):
        if qr_alnum_capacity(v) != cap:
            return fail(f"qr capacity v{v}: {qr_alnum_capacity(v)}")

    qr_page, qr_small = CARRIERS["qr-page"], CARRIERS["qr-small"]
    if (qr_page.max_bundle, qr_small.max_bundle) != (2257, 964):
        return fail(f"qr ceilings {qr_page.max_bundle} {qr_small.max_bundle}")

    # The slice-1 example (with its 1280-byte blob) does not fit one code.
    example, example_m = br.build_example()
    if len(example) <= qr_page.max_bundle:
        return fail("example unexpectedly fits one QR code")
    text = qr_text(example)
    if qr_version_for(len(text)) is not None:
        return fail("example text fits version 40")

    # Honest ceilings from real signed bundles, read back through the QR form.
    rows = {}
    for carrier in (qr_page, qr_small):
        for kind, make in (("chat", chat_object), ("link", link_object)):
            for binding in (0, 96):
                n, bundle = max_objects(carrier, make, qr_template(binding))
                text = qr_text(bundle)
                version = qr_version_for(len(text))
                rep = qr_read(text)
                if len(rep.objects) != n or version is None:
                    return fail(f"{carrier.name} {kind} read back")
                limit = 40 if carrier is qr_page else 25
                if version > limit:
                    return fail(f"{carrier.name} {kind} needs v{version}")
                rows[carrier.name, kind, binding] = (n, len(bundle), version)
    expect = {
        ("qr-page", "chat", 0): 10,
        ("qr-page", "chat", 96): 9,
        ("qr-page", "link", 0): 4,
        ("qr-page", "link", 96): 4,
        ("qr-small", "chat", 0): 3,
        ("qr-small", "chat", 96): 3,
        ("qr-small", "link", 0): 1,
        ("qr-small", "link", 96): 1,
    }
    got = {k: v[0] for k, v in rows.items()}
    if got != expect:
        return fail(f"qr object ceilings {got}")

    # QR reader refusals, then slice 1's own rejections come through unchanged.
    one = {br.blake3_256(chat_object(0)): (1, chat_object(0))}
    good_bytes = split(qr_template(), br.EXAMPLE_SEED, one, qr_small)[0]
    good = qr_text(good_bytes)
    cases = {
        "ID://ANY/GRAPH/V2": "qr_not_a_bundle",
        good.lower(): "qr_not_a_bundle",
        good[:-1]: None,  # either base45 or slice 1 refuses a cut code
        QR_PREFIX + "A": "qr_bad_base45",
        QR_PREFIX + "GGW" + good[len(QR_PREFIX) + 3 :]: "qr_bad_base45",
        qr_text(good_bytes + b"\0\0"): "trailing_bytes",
        qr_text(good_bytes[:-1]): "truncated_objects",
    }
    for text, code in cases.items():
        try:
            qr_read(text)
            return fail(f"qr accepted {text[:12]!r}")
        except (CarrierError, br.BundleError) as exc:
            if code is not None and str(exc) != code:
                return fail(f"qr {text[:12]!r}: {exc}")
    try:
        qr_read(QR_PREFIX + b45encode(b"IDBUNDLE" + good.encode()[:40]))
        return fail("qr accepted junk bytes")
    except br.BundleError:
        pass

    # Splitting: the example's five objects for the small code.
    objs = {}
    body_len = struct.unpack(">I", example[10:14])[0]
    pos = 14 + body_len + 65
    for e in sorted(example_m.entries, key=lambda e: e.digest):
        objs[e.digest] = (e.kind, example[pos : pos + e.size])
        pos += e.size
    template = br.Manifest(**{**example_m.__dict__, "entries": []})
    try:
        split(template, br.EXAMPLE_SEED, objs, qr_small)
        return fail("small code took the 1280-byte blob")
    except CarrierError as exc:
        if str(exc) != "object_too_big_for_carrier":
            return fail(f"split blob: {exc}")
    parts = split(template, br.EXAMPLE_SEED, objs, qr_page)
    again = split(template, br.EXAMPLE_SEED, objs, qr_page)
    reordered = split(template, br.EXAMPLE_SEED, dict(reversed(objs.items())), qr_page)
    if parts != again or parts != reordered or len(parts) != 2:
        return fail(f"split not deterministic or {len(parts)} parts")
    seen = []
    for p in parts:
        if len(p) > qr_page.max_bundle:
            return fail("part over ceiling")
        rep = qr_read(qr_text(p))
        if rep.binding != "unbound" or rep.expiry not in ("valid", "expired"):
            return fail("part lost its manifest fields")
        seen += [o[1] for o in rep.objects]
    if sorted(seen) != sorted(d.hex() for d in objs):
        return fail("split lost or duplicated an object")
    no_blob = {d: v for d, v in objs.items() if v[0] != 3}
    if len(split(template, br.EXAMPLE_SEED, no_blob, qr_page)) != 1:
        return fail("example without its blob should fit one page code")

    # Email: the size formula matches what the stdlib MIME writer produces.
    for n in (0, 1, 2, 3, 56, 57, 58, 1000, 100_000):
        payload = example + bytes(n)  # size only; the reader is not involved
        msg = EmailMessage()
        msg.add_attachment(
            payload,
            maintype="application",
            subtype="octet-stream",
            filename="x.idbundle",
        )
        part = next(msg.iter_attachments())
        encoded = part.get_payload().encode()
        lines = encoded.replace(b"\r\n", b"\n").split(b"\n")
        if lines[-1] == b"":
            lines.pop()
        if sum(len(x) + 2 for x in lines) != email_attachment_bytes(len(payload)):
            return fail(f"email size formula at {len(payload)}")
    ceiling = CARRIERS["email"].max_bundle
    if not (
        email_attachment_bytes(ceiling)
        <= EMAIL_BUDGET - EMAIL_RESERVE
        < email_attachment_bytes(ceiling + 1)
    ):
        return fail("email ceiling not tight")
    if ceiling != 7_259_799:
        return fail(f"email ceiling {ceiling}")
    rep = email_read(email_message(example, "From ben: the garden thread."))
    if rep.bundle_id != br.read_a(example).bundle_id:
        return fail("email round trip")
    twice = EmailMessage()
    twice.set_content("two bundles")
    for b in (example, example):
        twice.add_attachment(
            b,
            maintype="application",
            subtype="octet-stream",
            filename=media_filename(b),
        )
    try:
        email_read(twice.as_bytes())
        return fail("email accepted two bundles")
    except CarrierError as exc:
        if str(exc) != "email_needs_one_bundle":
            return fail(f"email two bundles: {exc}")
    try:
        email_read(email_message(example, "x").replace(b".idbundle", b".bin"))
        return fail("email accepted a message with no bundle")
    except CarrierError as exc:
        if str(exc) != "email_needs_one_bundle":
            return fail(f"email no bundle: {exc}")

    # Media: name rule; the reader ignores the name.
    name = media_filename(example)
    if name != br.read_a(example).bundle_id.rsplit(".", 1)[1][:16] + ".idbundle":
        return fail("media file name")
    if not FORMAT_MAX > FAT32_MAX_FILE:
        return fail("format max should exceed FAT32")

    print(
        f"ok: base45 4 RFC vectors and 260 round trips, 7 QR capacities, "
        f"QR ceilings {qr_page.max_bundle} / {qr_small.max_bundle} bytes, "
        f"object ceilings {expect['qr-page', 'chat', 0]} chat or "
        f"{expect['qr-page', 'link', 0]} link (page), {expect['qr-small', 'chat', 0]} "
        f"or {expect['qr-small', 'link', 0]} (small), example needs 2 page codes, "
        f"email ceiling {ceiling} bytes, {len(cases) + 1} QR refusals"
    )
    return 0


def table() -> int:
    for c in CARRIERS.values():
        print(f"{c.name:16} {c.max_bundle:>13,} bytes  private={c.private}  ({c.note})")
    print(f"format maximum   {FORMAT_MAX:>17,} bytes")
    for v in (25, 40):
        print(
            f"QR v{v} at 0.5 mm modules: {qr_side_mm(v, 0.5):.1f} mm a side, "
            f"0.33 mm: {qr_side_mm(v, 0.33):.1f} mm"
        )
    for carrier in (CARRIERS["qr-page"], CARRIERS["qr-small"]):
        for kind, make in (("chat", chat_object), ("link", link_object)):
            for binding in (0, 96):
                n, bundle = max_objects(carrier, make, qr_template(binding))
                print(
                    f"{carrier.name:9} {kind} objects ({len(make(0))} bytes each), "
                    f"binding {binding:>2}: {n:>2} objects, bundle {len(bundle)} bytes, "
                    f"QR v{qr_version_for(len(qr_text(bundle)))}"
                )
    return 0


def cross_check() -> int:
    try:
        import segno  # type: ignore
        import segno.consts as sc  # type: ignore
    except ImportError:
        print("segno: package not installed, skipped")
        return 0
    cws = tuple(
        sum(e.num_blocks * e.num_data for e in sc.ECC[v][sc.ERROR_LEVEL_M])
        for v in range(1, 41)
    )
    assert cws == QR_DATA_CODEWORDS_M
    for v in range(1, 41):
        for chars in (qr_alnum_capacity(v), qr_alnum_capacity(v) + 1):
            if chars > qr_alnum_capacity(40):
                continue
            q = segno.make(
                "A" * chars,
                error="m",
                mode="alphanumeric",
                boost_error=False,
                micro=False,
            )
            assert q.version == qr_version_for(chars), (v, chars, q.version)
    for n in (0, 10, 100, 500, 964, 1500, 2257):
        text = qr_text(bytes(range(256)) * (n // 256) + bytes(n % 256))
        q = segno.make(
            text, error="m", mode="alphanumeric", boost_error=False, micro=False
        )
        assert q.version == qr_version_for(len(text)), (n, q.version)
    print(
        "segno: agrees on 40 level-M codeword counts, every version boundary "
        "and 7 bundle-sized codes"
    )
    return 0


if __name__ == "__main__":
    arg = sys.argv[1:2]
    if arg == ["--cross-check"]:
        sys.exit(cross_check())
    if arg == ["--table"]:
        sys.exit(table())
    sys.exit(self_test())
