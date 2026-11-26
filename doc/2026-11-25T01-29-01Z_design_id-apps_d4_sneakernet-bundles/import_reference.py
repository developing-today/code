#!/usr/bin/env python3
"""Reference export/import flow for ID sneakernet bundles (D4 slice 3,
#13889): the creator binding, the verification order, and the rejection
catalogue with the import outcome for each case. Python stdlib only; loads
bundle_reference.py (slice 1) and carrier_profiles_reference.py (slice 5)
beside it.

The creator binding (manifest tag 0x04) proves that the bundle's signing key
is an enrolled, unrevoked device of the creator's identity, using the keyset
authorization log already designed (#2380 ADR) and implemented
(pkgs/id/crates/id-directory/src/keyset.rs):

  creator_binding = kind:u8 body
  kind 0x01 keyset:   count:u16 (1-256), then count times len:u32 record
                      record = one KeysetRecord in its serde JSON form,
                      in chain order from the genesis record
  kind 0x02 fleet-ca: reserved until fleet-cert has revocation and an ID leaf
                      profile (#9157); held
  kind 0x03 keyset-ref: identity (32 bytes) + head record hash (32 bytes),
                      for small carriers; the receiver must already hold the
                      chain through that head, or the bundle is held
  other kinds:        rejected

  creator_identity = "keyset:" + the identity ID (the genesis admin key, hex)

This reference folds linear keyset chains only. A fork, and the revocation
priority and checkpoint finality rules for one, are keyset.rs's fold; the
implementation calls that fold instead of this one.

Run with no arguments (or self-test) for the self-test; --table prints
binding sizes and what they do to the QR ceilings.
"""

from __future__ import annotations

import hashlib
import importlib.util
import itertools
import json
import pathlib
import re
import struct
import sys
from dataclasses import dataclass, field

sys.dont_write_bytecode = True

_HERE = pathlib.Path(__file__).resolve().parent


def _load(name: str):
    if name in sys.modules:
        return sys.modules[name]
    spec = importlib.util.spec_from_file_location(name, _HERE / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


br = _load("bundle_reference")
cp = _load("carrier_profiles_reference")

HEX64 = re.compile(r"[0-9a-f]{64}")
HEX128 = re.compile(r"[0-9a-f]{128}")
MAX_BINDING_RECORDS = 256
KIND_KEYSET, KIND_FLEET_CA, KIND_KEYSET_REF = 0x01, 0x02, 0x03


class Refused(ValueError):
    """The message is the stable code from the catalogue."""


# ------------------------------------------------------------- keyset records
# The same canonical JSON as keyset.rs body_bytes: compact serde_json of
# {author, lamport, prev_hash, entry}, entry internally tagged by "type".

ENTRY_FIELDS = {
    "genesis": ("admin_key", "device_key", "label"),
    "device_enrolled": ("device_key", "label"),
    "device_revoked": ("device_key", "reason"),
    "admin_key_rotated": ("new_admin_key",),
    "idp_binding_bound": ("binding_id",),
    "idp_binding_revoked": ("binding_id",),
    "checkpoint": ("state_hash", "record_count"),
}
COUNTERSIGNED = {"device_enrolled": "device_key", "admin_key_rotated": "new_admin_key"}


def _json(value) -> bytes:
    return json.dumps(value, separators=(",", ":"), ensure_ascii=False).encode()


def entry(kind: str, **fields) -> dict:
    return {"type": kind, **{k: fields[k] for k in ENTRY_FIELDS[kind]}}


def body_bytes(author: str, lamport: int, prev_hash: str, ent: dict) -> bytes:
    return _json(
        {"author": author, "lamport": lamport, "prev_hash": prev_hash, "entry": ent}
    )


def pub_hex(seed: bytes) -> str:
    return br.ed25519_public(seed).hex()


def sign_record(
    seed: bytes, lamport: int, prev_hash: str, ent: dict, counter: bytes | None = None
):
    author = pub_hex(seed)
    body = body_bytes(author, lamport, prev_hash, ent)
    return {
        "author": author,
        "lamport": lamport,
        "prev_hash": prev_hash,
        "hash": hashlib.sha256(body).hexdigest(),
        "entry": ent,
        "signature": br.ed25519_sign(seed, body).hex(),
        "countersignature": br.ed25519_sign(counter, body).hex() if counter else None,
    }


def verify_record(r: dict) -> bool:
    """keyset.rs KeysetRecord::verify: shape, hash, signature, countersignature."""
    try:
        ent = r["entry"]
        fields = ENTRY_FIELDS[ent["type"]]
        if set(ent) != {"type", *fields} or set(r) != {
            "author",
            "lamport",
            "prev_hash",
            "hash",
            "entry",
            "signature",
            "countersignature",
        }:
            return False
        for k in fields:
            if k.endswith("_key") and not HEX64.fullmatch(ent[k]):
                return False
        if not HEX64.fullmatch(r["author"]) or not isinstance(r["lamport"], int):
            return False
        genesis = ent["type"] == "genesis"
        if (r["prev_hash"] == "") != genesis or not (
            genesis or HEX64.fullmatch(r["prev_hash"])
        ):
            return False
        if genesis and r["author"] != ent["admin_key"]:
            return False
        body = body_bytes(r["author"], r["lamport"], r["prev_hash"], ent)
        if hashlib.sha256(body).hexdigest() != r["hash"]:
            return False
        if not HEX128.fullmatch(r["signature"]) or not br.ed25519_verify(
            bytes.fromhex(r["author"]), body, bytes.fromhex(r["signature"])
        ):
            return False
        counter_key = ent.get(COUNTERSIGNED.get(ent["type"], ""))
        cs = r["countersignature"]
        if counter_key is None:
            return cs is None
        return (
            isinstance(cs, str)
            and HEX128.fullmatch(cs) is not None
            and br.ed25519_verify(bytes.fromhex(counter_key), body, bytes.fromhex(cs))
        )
    except (KeyError, TypeError, ValueError, AttributeError):
        return False


@dataclass
class KeysetState:
    identity: str
    admins: dict[str, str] = field(default_factory=dict)  # key -> active | rotated
    devices: dict[str, bool] = field(default_factory=dict)  # key -> revoked?
    head: str = ""

    def is_device(self, key: str) -> bool:
        return self.devices.get(key) is False


def fold(records: dict[str, dict], genesis_hash: str) -> KeysetState:
    """Linear fold from a trusted genesis. Checkpoints never advance the head;
    a child folds only if it verifies, its lamport is above the head's, and
    its author is an active admin at that position."""
    g = records[genesis_hash]
    ent = g["entry"]
    st = KeysetState(
        ent["admin_key"],
        {ent["admin_key"]: "active"},
        {ent["device_key"]: False},
        g["hash"],
    )
    lamport = g["lamport"]
    while True:
        kids = [
            r
            for r in records.values()
            if r["prev_hash"] == st.head
            and r["entry"]["type"] not in ("genesis", "checkpoint")
            and r["lamport"] > lamport
            and st.admins.get(r["author"]) == "active"
            and verify_record(r)
        ]
        if not kids:
            return st
        if len(kids) > 1:
            raise Refused("fork_needs_keyset_fold")
        r = kids[0]
        e = r["entry"]
        if e["type"] == "device_enrolled":
            st.devices[e["device_key"]] = False
        elif e["type"] == "device_revoked":
            st.devices[e["device_key"]] = True
        elif e["type"] == "admin_key_rotated":
            st.admins[r["author"]] = "rotated"
            st.admins[e["new_admin_key"]] = "active"
        st.head, lamport = r["hash"], r["lamport"]


# ------------------------------------------------------------- binding codec


def encode_binding(records: list[dict]) -> bytes:
    out = bytes([KIND_KEYSET]) + struct.pack(">H", len(records))
    for r in records:
        raw = _json(r)
        out += struct.pack(">I", len(raw)) + raw
    return out


def encode_binding_ref(identity: str, head: str) -> bytes:
    return bytes([KIND_KEYSET_REF]) + bytes.fromhex(identity) + bytes.fromhex(head)


def decode_binding(raw: bytes) -> tuple[int, list]:
    if not raw:
        raise Refused("binding_malformed")
    if raw[0] == KIND_FLEET_CA:
        return KIND_FLEET_CA, []
    if raw[0] == KIND_KEYSET_REF:
        if len(raw) != 65:
            raise Refused("binding_malformed")
        return KIND_KEYSET_REF, [raw[1:33].hex(), raw[33:].hex()]
    if raw[0] != KIND_KEYSET:
        raise Refused("unknown_binding_kind")
    if len(raw) < 3:
        raise Refused("binding_malformed")
    (count,) = struct.unpack(">H", raw[1:3])
    if not 1 <= count <= MAX_BINDING_RECORDS:
        raise Refused("binding_malformed")
    pos, out = 3, []
    for _ in range(count):
        if pos + 4 > len(raw):
            raise Refused("binding_malformed")
        (n,) = struct.unpack(">I", raw[pos : pos + 4])
        chunk = raw[pos + 4 : pos + 4 + n]
        if len(chunk) != n:
            raise Refused("binding_malformed")
        try:
            r = json.loads(chunk)
        except (UnicodeDecodeError, ValueError):
            raise Refused("binding_malformed") from None
        if not isinstance(r, dict) or _json(r) != chunk:
            raise Refused("binding_malformed")  # one encoding per record
        out.append(r)
        pos += 4 + n
    if pos != len(raw):
        raise Refused("binding_malformed")
    return KIND_KEYSET, out


# ------------------------------------------------------------- receiver


@dataclass
class Receiver:
    """What the importing host already holds: identities it trusts (by their
    genesis record hash) and the keyset records it has seen."""

    trusted: dict[str, str] = field(default_factory=dict)  # identity -> genesis hash
    keysets: dict[str, dict[str, dict]] = field(
        default_factory=dict
    )  # identity -> records
    held: list[bytes] = field(default_factory=list)
    graph: dict[str, bytes] = field(default_factory=dict)  # digest hex -> object bytes

    def trust(self, genesis: dict) -> None:
        """The person accepts an identity (an invite, a directory entry, or a
        held bundle they approve). Outside the bundle flow."""
        ident = genesis["entry"]["admin_key"]
        self.trusted[ident] = genesis["hash"]
        self.keysets.setdefault(ident, {})[genesis["hash"]] = genesis


@dataclass
class Outcome:
    result: str  # imported | held | rejected
    code: str  # "ok" or the catalogue code
    stage: int
    objects: int = 0
    learned: int = 0  # keyset records the receiver did not have
    dangling: list[str] = field(default_factory=list)
    notes: list[str] = field(default_factory=list)


HOLD = {"unknown_identity", "binding_kind_unavailable", "binding_records_missing"}


def import_bundle(blob: bytes, rx: Receiver, now: int, grants: bool = False) -> Outcome:
    """`grants=True` is a slice-2 importer: a grants record passes stage 1
    here and is checked by grants_reference.py after stage 5."""
    stage = 1
    try:
        # 1. Header and manifest structure (slice 1 codes), no object bytes read.
        head, pos = br._take(blob, 0, 14, "truncated_header")
        if head[:8] != br.MAGIC:
            raise Refused("bad_magic")
        version, mlen = struct.unpack(">HI", head[8:])
        if version != br.VERSION:
            raise Refused("unknown_version")
        if mlen > br.MAX_MANIFEST:
            raise Refused("manifest_too_large")
        body, pos = br._take(blob, pos, mlen, "truncated_manifest")
        sig, pos = br._take(blob, pos, 65, "truncated_signature")
        m = br.parse_manifest(body, grants)
        # 2. The bundle signature over the manifest.
        stage = 2
        if sig[0] != 0x01:
            raise Refused("unknown_signature_alg")
        if not br.ed25519_verify(m.signing_key, br.DOMAIN + body, sig[1:]):
            raise Refused("bad_signature")
        # 3. The creator binding, still before any object is hashed.
        stage = 3
        if m.creator_binding is None:
            raise Refused("unbound_creator")
        kind, records = decode_binding(m.creator_binding)
        if kind == KIND_FLEET_CA:
            raise Refused("binding_kind_unavailable")
        if kind == KIND_KEYSET_REF:
            ident, head_hash = records
            records = []
            if m.creator_identity != "keyset:" + ident:
                raise Refused("binding_identity_mismatch")
            if ident not in rx.trusted:
                raise Refused("unknown_identity")
            if head_hash not in rx.keysets.get(ident, {}):
                raise Refused("binding_records_missing")
        else:
            if not all(verify_record(r) for r in records):
                raise Refused("binding_record_invalid")
            genesis = records[0]
            if genesis["entry"]["type"] != "genesis":
                raise Refused("binding_chain_broken")
            ident = genesis["entry"]["admin_key"]
            if m.creator_identity != "keyset:" + ident:
                raise Refused("binding_identity_mismatch")
            for prev, r in itertools.pairwise(records):
                if (
                    r["prev_hash"] != prev["hash"]
                    and r["entry"]["type"] != "checkpoint"
                ):
                    raise Refused("binding_chain_broken")
            if ident not in rx.trusted:
                raise Refused("unknown_identity")
        known = rx.keysets.setdefault(ident, {})
        union = {**{r["hash"]: r for r in records}, **known}
        state = fold(union, rx.trusted[ident])
        key = m.signing_key.hex()
        if state.devices.get(key) is True:
            raise Refused("signing_key_revoked")
        if not state.is_device(key):
            raise Refused("signing_key_not_enrolled")
        # 4. Object bytes: length and every digest (slice 1's full reader).
        stage = 4
        rep = br.read_a(blob, now, grants)
    except (Refused, br.BundleError) as exc:
        code = str(exc)
        if code in HOLD:
            rx.held.append(blob)
            return Outcome("held", code, stage)
        return Outcome("rejected", code, stage)

    # 5. Accept: keep the keyset records, hand objects to graph ingest
    # (the same path as network sync, slice 4), list dangling targets.
    learned = [r for r in records if r["hash"] not in known and verify_record(r)]
    for r in learned:
        known[r["hash"]] = r
    off = 14 + mlen + 65
    for e in sorted(m.entries, key=lambda e: e.digest):
        rx.graph[e.digest.hex()] = blob[off : off + e.size]
        off += e.size
    dangling = sorted(
        {
            t
            for b in rx.graph.values()
            if b[:1] == b"{"
            for t in _targets(b)
            if t not in rx.graph
        }
    )
    notes = []
    if rep.expiry == "expired":
        notes.append("expired: objects imported, grants not applied")
    return Outcome("imported", "ok", 5, len(rep.objects), len(learned), dangling, notes)


def _targets(raw: bytes) -> list[str]:
    try:
        obj = json.loads(raw)
    except ValueError:
        return []
    return (
        [obj["target"]]
        if isinstance(obj, dict) and isinstance(obj.get("target"), str)
        else []
    )


# ------------------------------------------------------------- export


def export(
    seed: bytes,
    chain: list[dict],
    objects: dict[bytes, tuple[int, bytes]],
    carrier,
    created_at: int,
    expires_at: int | None = None,
    scope: str | None = None,
) -> list[bytes]:
    """The writer's side: its device key signs, the binding is the writer's
    whole current keyset chain, and slice 5 splits for the carrier."""
    template = br.Manifest(
        creator_identity="keyset:" + chain[0]["entry"]["admin_key"],
        signing_key=br.ed25519_public(seed),
        created_at=created_at,
        entries=[],
        creator_binding=encode_binding(chain),
        expires_at=expires_at,
        scope=scope,
    )
    return cp.split(template, seed, objects, carrier)


# ------------------------------------------------------------- fixture


def _seed(name: str) -> bytes:
    return hashlib.sha256(b"id-bundle slice 3 test key " + name.encode()).digest()


ADMIN, PHONE, LAPTOP, OTHER = (_seed(n) for n in ("admin", "phone", "laptop", "other"))
T0 = 1795975200  # 2026-11-24T18:00:00Z


def keyset_fixture():
    g = sign_record(
        ADMIN,
        1,
        "",
        entry(
            "genesis",
            admin_key=pub_hex(ADMIN),
            device_key=pub_hex(PHONE),
            label="phone",
        ),
    )
    enrol = sign_record(
        ADMIN,
        2,
        g["hash"],
        entry("device_enrolled", device_key=pub_hex(LAPTOP), label="laptop"),
        counter=LAPTOP,
    )
    revoke = sign_record(
        ADMIN,
        3,
        enrol["hash"],
        entry("device_revoked", device_key=pub_hex(LAPTOP), reason="lost"),
    )
    return g, enrol, revoke


def example_objects() -> dict[bytes, tuple[int, bytes]]:
    _, m = br.build_example()
    blob, _ = br.build_example()
    body_len = struct.unpack(">I", blob[10:14])[0]
    pos, out = 14 + body_len + 65, {}
    for e in sorted(m.entries, key=lambda e: e.digest):
        out[e.digest] = (e.kind, blob[pos : pos + e.size])
        pos += e.size
    return out


# ------------------------------------------------------------- self-test


def fail(msg: str) -> int:
    print(f"FAIL: {msg}")
    return 1


def self_test() -> int:
    g, enrol, revoke = keyset_fixture()
    media = cp.CARRIERS["removable-media"]
    objs = example_objects()

    # The canonical body matches keyset.rs's own test of its field order.
    text = body_bytes(g["author"], 1, "", g["entry"]).decode()
    if not (
        text.startswith('{"author":')
        and '","lamport":1,"prev_hash":"","entry":{' in text
        and '"entry":{"type":"genesis","admin_key":' in text
    ):
        return fail("canonical body shape")
    for r in (g, enrol, revoke):
        if not verify_record(r):
            return fail(f"fixture record {r['entry']['type']}")

    def fresh(*known) -> Receiver:
        rx = Receiver()
        rx.trust(g)
        for r in known:
            rx.keysets[g["author"]][r["hash"]] = r
        return rx

    def one(seed, chain, **kw) -> bytes:
        return export(seed, chain, objs, media, T0, T0 + 30 * 86400, **kw)[0]

    by_phone = one(PHONE, [g])
    by_laptop = one(LAPTOP, [g, enrol])
    cases = []  # (name, blob, receiver, now, result, code)

    cases.append(("phone, genesis only", by_phone, fresh(), T0, "imported", "ok"))
    cases.append(
        ("laptop, receiver learns enrolment", by_laptop, fresh(), T0, "imported", "ok")
    )
    cases.append(
        (
            "laptop after its revocation",
            by_laptop,
            fresh(enrol, revoke),
            T0,
            "rejected",
            "signing_key_revoked",
        )
    )
    cases.append(
        (
            "revocation carried in the bundle",
            one(PHONE, [g, enrol, revoke]),
            fresh(),
            T0,
            "imported",
            "ok",
        )
    )
    cases.append(
        (
            "laptop without its enrolment",
            one(LAPTOP, [g]),
            fresh(),
            T0,
            "rejected",
            "signing_key_not_enrolled",
        )
    )
    stranger = Receiver()
    cases.append(
        (
            "identity the receiver never accepted",
            by_phone,
            stranger,
            T0,
            "held",
            "unknown_identity",
        )
    )
    cases.append(
        ("expired bundle", by_phone, fresh(), T0 + 31 * 86400, "imported", "ok")
    )

    def tweak(**kw) -> bytes:
        m = br.Manifest(
            creator_identity="keyset:" + g["author"],
            signing_key=br.ed25519_public(PHONE),
            created_at=T0,
            entries=[],
            creator_binding=encode_binding([g]),
        )
        for k, v in kw.items():
            setattr(m, k, v)
        return cp.split(m, PHONE, objs, media)[0]

    bad_label = dict(enrol, entry={**enrol["entry"], "label": "laptop2"})
    no_counter = dict(enrol, countersignature=None)
    by_device = sign_record(
        PHONE,
        2,
        g["hash"],
        entry("device_enrolled", device_key=pub_hex(LAPTOP), label="laptop"),
        counter=LAPTOP,
    )
    stale_lamport = sign_record(
        ADMIN,
        1,
        g["hash"],
        entry("device_enrolled", device_key=pub_hex(LAPTOP), label="laptop"),
        counter=LAPTOP,
    )
    gap = sign_record(
        ADMIN,
        3,
        "ab" * 32,
        entry("device_enrolled", device_key=pub_hex(LAPTOP), label="laptop"),
        counter=LAPTOP,
    )
    other_g = sign_record(
        OTHER,
        1,
        "",
        entry(
            "genesis",
            admin_key=pub_hex(OTHER),
            device_key=pub_hex(PHONE),
            label="phone",
        ),
    )
    rows = [
        ("no binding", tweak(creator_binding=None), "unbound_creator"),
        (
            "fleet CA binding",
            tweak(creator_binding=b"\x02cert"),
            "binding_kind_unavailable",
        ),
        (
            "unknown binding kind",
            tweak(creator_binding=b"\x7f"),
            "unknown_binding_kind",
        ),
        (
            "binding cut short",
            tweak(creator_binding=encode_binding([g])[:-1]),
            "binding_malformed",
        ),
        (
            "binding count zero",
            tweak(creator_binding=b"\x01\x00\x00"),
            "binding_malformed",
        ),
        (
            "record re-encoded",
            tweak(creator_binding=b"\x01\x00\x01" + _reencoded(g)),
            "binding_malformed",
        ),
        (
            "record edited",
            tweak(creator_binding=encode_binding([g, bad_label])),
            "binding_record_invalid",
        ),
        (
            "missing countersignature",
            tweak(creator_binding=encode_binding([g, no_counter])),
            "binding_record_invalid",
        ),
        (
            "first record not genesis",
            tweak(creator_binding=encode_binding([enrol])),
            "binding_chain_broken",
        ),
        (
            "records out of chain",
            tweak(creator_binding=encode_binding([g, gap])),
            "binding_chain_broken",
        ),
        (
            "identity names someone else",
            tweak(creator_identity="keyset:" + "cd" * 32),
            "binding_identity_mismatch",
        ),
        (
            "another identity's genesis",
            tweak(
                creator_identity="keyset:" + other_g["author"],
                creator_binding=encode_binding([other_g]),
            ),
            "unknown_identity",
        ),
    ]
    for name, blob, code in rows:
        cases.append(
            (name, blob, fresh(), T0, "held" if code in HOLD else "rejected", code)
        )
    ref = encode_binding_ref(g["author"], enrol["hash"])
    by_laptop_ref = cp.split(
        br.Manifest(
            creator_identity="keyset:" + g["author"],
            signing_key=br.ed25519_public(LAPTOP),
            created_at=T0,
            entries=[],
            creator_binding=ref,
        ),
        LAPTOP,
        objs,
        media,
    )[0]
    cases.append(
        (
            "reference binding, receiver has the chain",
            by_laptop_ref,
            fresh(enrol),
            T0,
            "imported",
            "ok",
        )
    )
    cases.append(
        (
            "reference binding, receiver lacks the head",
            by_laptop_ref,
            fresh(),
            T0,
            "held",
            "binding_records_missing",
        )
    )
    cases.append(
        (
            "reference binding, revoked since",
            by_laptop_ref,
            fresh(enrol, revoke),
            T0,
            "rejected",
            "signing_key_revoked",
        )
    )
    cases.append(
        (
            "reference binding, wrong identity",
            tweak(creator_binding=encode_binding_ref("cd" * 32, g["hash"])),
            fresh(),
            T0,
            "rejected",
            "binding_identity_mismatch",
        )
    )
    cases.append(
        (
            "reference binding, wrong length",
            tweak(creator_binding=ref[:-1]),
            fresh(),
            T0,
            "rejected",
            "binding_malformed",
        )
    )
    # Record rules beyond the fixture: no stray countersignature, a genesis
    # signed by its own admin key, and a rotated-away admin key loses power.
    stray = dict(revoke)
    stray["countersignature"] = br.ed25519_sign(PHONE, b"x").hex()
    cases.append(
        (
            "countersignature where none belongs",
            one(PHONE, [g, enrol, stray]),
            fresh(),
            T0,
            "rejected",
            "binding_record_invalid",
        )
    )
    forged_g = dict(g)
    forged_body = body_bytes(pub_hex(OTHER), 1, "", g["entry"])
    forged_g.update(
        author=pub_hex(OTHER),
        hash=hashlib.sha256(forged_body).hexdigest(),
        signature=br.ed25519_sign(OTHER, forged_body).hex(),
    )
    cases.append(
        (
            "genesis signed by another key",
            tweak(creator_binding=encode_binding([forged_g])),
            fresh(),
            T0,
            "rejected",
            "binding_record_invalid",
        )
    )
    admin2 = _seed("admin2")
    rotate = sign_record(
        ADMIN,
        2,
        g["hash"],
        entry("admin_key_rotated", new_admin_key=pub_hex(admin2)),
        counter=admin2,
    )
    old_admin_enrols = sign_record(
        ADMIN,
        3,
        rotate["hash"],
        entry("device_enrolled", device_key=pub_hex(LAPTOP), label="laptop"),
        counter=LAPTOP,
    )
    new_admin_enrols = sign_record(
        admin2,
        3,
        rotate["hash"],
        entry("device_enrolled", device_key=pub_hex(LAPTOP), label="laptop"),
        counter=LAPTOP,
    )
    cases.append(
        (
            "enrolled by a rotated-away admin key",
            one(LAPTOP, [g, rotate, old_admin_enrols]),
            fresh(),
            T0,
            "rejected",
            "signing_key_not_enrolled",
        )
    )
    cases.append(
        (
            "enrolled by the new admin key",
            one(LAPTOP, [g, rotate, new_admin_enrols]),
            fresh(),
            T0,
            "imported",
            "ok",
        )
    )
    # A device may not enrol another device; a lamport that does not rise does not fold.
    for name, rec in (
        ("enrolled by a device", by_device),
        ("lamport not rising", stale_lamport),
    ):
        cases.append(
            (
                name,
                one(LAPTOP, [g, rec]),
                fresh(),
                T0,
                "rejected",
                "signing_key_not_enrolled",
            )
        )

    # Order: the first failing stage reports, and binding comes before hashing.
    good = by_phone
    body_len = struct.unpack(">I", good[10:14])[0]
    flipped_obj = good[:-1] + bytes([good[-1] ^ 1])
    cases.append(
        ("object byte changed", flipped_obj, fresh(), T0, "rejected", "digest_mismatch")
    )
    flipped_sig = bytearray(good)
    flipped_sig[14 + body_len + 5] ^= 1
    cases.append(
        (
            "signature changed",
            bytes(flipped_sig),
            fresh(),
            T0,
            "rejected",
            "bad_signature",
        )
    )
    unbound_and_bad_obj = tweak(creator_binding=None)
    unbound_and_bad_obj = unbound_and_bad_obj[:-1] + bytes(
        [unbound_and_bad_obj[-1] ^ 1]
    )
    cases.append(
        (
            "unbound and an object changed",
            unbound_and_bad_obj,
            fresh(),
            T0,
            "rejected",
            "unbound_creator",
        )
    )
    cases.append(
        (
            "grants present",
            _with_grants(good),
            fresh(),
            T0,
            "rejected",
            "grants_reserved_slice_2",
        )
    )
    cases.append(
        ("not a bundle", b"PK\x03\x04" + good[4:], fresh(), T0, "rejected", "bad_magic")
    )

    stages = {}
    for name, blob, rx, now, result, code in cases:
        out = import_bundle(blob, rx, now)
        if (out.result, out.code) != (result, code):
            return fail(f"{name}: {out.result} {out.code}")
        stages[code] = out.stage
        if result == "imported":
            if out.objects != 5 or out.dangling != ["ab" * 32]:
                return fail(f"{name}: {out.objects} objects, dangling {out.dangling}")
            if name == "expired bundle" and not out.notes:
                return fail("expired bundle had no note")
            if name == "laptop, receiver learns enrolment" and out.learned != 1:
                return fail("enrolment record not learned")
            if name == "phone, genesis only" and out.learned != 0:
                return fail("genesis counted as learned")
        elif out.objects or rx.graph:
            return fail(f"{name}: objects reached the graph")
        if result == "held" and rx.held[-1] != blob:
            return fail(f"{name}: not kept for the person")
    if (
        not stages["bad_signature"]
        < stages["unbound_creator"]
        < stages["digest_mismatch"]
    ):
        return fail(f"stage order {stages}")

    # A held bundle imports once the person accepts the identity.
    stranger.trust(g)
    if import_bundle(stranger.held[0], stranger, T0).result != "imported":
        return fail("held bundle after accepting the identity")

    # The receiver's own newer record wins over an older chain in the bundle,
    # and the bundle can teach a receiver the revocation.
    rx = fresh()
    import_bundle(one(PHONE, [g, enrol, revoke]), rx, T0)
    if import_bundle(by_laptop, rx, T0).code != "signing_key_revoked":
        return fail("revocation learned from one bundle did not apply to the next")

    # Forks are keyset.rs's fold, not this reference's.
    sibling = sign_record(
        ADMIN,
        2,
        g["hash"],
        entry("device_enrolled", device_key=pub_hex(OTHER), label="tablet"),
        counter=OTHER,
    )
    try:
        fold({r["hash"]: r for r in (g, enrol, sibling)}, g["hash"])
        return fail("fork folded")
    except Refused as exc:
        if str(exc) != "fork_needs_keyset_fold":
            return fail(f"fork: {exc}")

    print(
        f"ok: {len(cases)} catalogue cases ({sum(c[4] == 'imported' for c in cases)} imported, "
        f"{sum(c[4] == 'held' for c in cases)} held, {sum(c[4] == 'rejected' for c in cases)} "
        f"rejected), binding checked before hashing, a held bundle imports after the identity "
        f"is accepted, a carried revocation applies to later bundles"
    )
    return 0


def _reencoded(r: dict) -> bytes:
    raw = json.dumps(r, separators=(", ", ": ")).encode()
    return struct.pack(">I", len(raw)) + raw


def _with_grants(good: bytes) -> bytes:
    body_len = struct.unpack(">I", good[10:14])[0]
    body = good[14 : 14 + body_len]
    recs, pos = [], 0
    while pos < len(body):
        tag, n = body[pos], struct.unpack(">I", body[pos + 1 : pos + 5])[0]
        recs.append((tag, body[pos + 5 : pos + 5 + n]))
        pos += 5 + n
    recs.append((0x08, b"x"))
    new = b"".join(bytes([t]) + struct.pack(">I", len(v)) + v for t, v in sorted(recs))
    tail = good[14 + body_len + 65 :]
    return (
        br.MAGIC
        + struct.pack(">HI", 1, len(new))
        + new
        + b"\x01"
        + br.ed25519_sign(PHONE, br.DOMAIN + new)
        + tail
    )


def table() -> int:
    g, enrol, revoke = keyset_fixture()
    for name, size in (
        (
            "reference (identity + head)",
            len(encode_binding_ref(g["author"], g["hash"])),
        ),
        ("genesis only", len(encode_binding([g]))),
        ("genesis + enrolment", len(encode_binding([g, enrol]))),
        ("genesis + enrolment + revocation", len(encode_binding([g, enrol, revoke]))),
    ):
        page = cp.max_objects(
            cp.CARRIERS["qr-page"], cp.chat_object, cp.qr_template(size)
        )[0]
        small = cp.max_objects(
            cp.CARRIERS["qr-small"], cp.chat_object, cp.qr_template(size)
        )[0]
        print(
            f"binding {name:34} {size:5} bytes; QR page {page:2} chat objects, small {small}"
        )
    return 0


if __name__ == "__main__":
    if sys.argv[1:2] == ["--table"]:
        sys.exit(table())
    sys.exit(self_test())
