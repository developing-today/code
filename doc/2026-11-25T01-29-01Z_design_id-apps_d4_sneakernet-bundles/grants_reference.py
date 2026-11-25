#!/usr/bin/env python3
"""Reference grant model for ID sneakernet bundles (D4 slice 2, #13889).
Python stdlib only; loads the slice 1, 3, 4 and 5 references beside it.

A bundle grant lets the receiver read the pairs this bundle carries in one
area, which slice 4 would otherwise hold as `held_no_read_grant`. It is one
delegation step down from a broker-issued read capability that the bundle's
signing device holds:

  grants = count:u8 (1-16), then count x (mode:u8 len:u32 body)
  mode   = 0x01 recipient-bound | 0x02 bearer
  body   = canonical JSON {area, from, parent, recipient, until} (recipient-bound)
                       or {area, from, parent, until}            (bearer)
  parent = {access: "read", area, from, grantee, until, sig}     (broker-signed)

Recipient-bound is the default and bearer is an explicit opt-in with a
lifetime of at most BEARER_MAX_SECONDS, per the decision recorded on #13889
on 2026-11-25. The grant needs no signature of its own: it sits in the
manifest, which the bundle signature covers, and its parent must name the
bundle's signing key as grantee.

Run with no arguments (or self-test) for the self-test.
"""

from __future__ import annotations

import copy
import importlib.util
import json
import pathlib
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
ir = _load("import_reference")
mr = _load("merge_reference")

T0 = mr.T0
DAY = 86400
MODE_RECIPIENT, MODE_BEARER = 0x01, 0x02
MODES = {MODE_RECIPIENT: "recipient", MODE_BEARER: "bearer"}
MAX_GRANTS = 16
BEARER_MAX_SECONDS = 7 * DAY
PARENT_KEYS = {"access", "area", "from", "grantee", "until", "sig"}
BODY_KEYS = {
    MODE_RECIPIENT: {"area", "from", "parent", "recipient", "until"},
    MODE_BEARER: {"area", "from", "parent", "until"},
}


class GrantError(ValueError):
    """A structural refusal at stage 1: the whole bundle is rejected."""


# ------------------------------------------------------------- encoding


def _canon(value) -> bytes:
    return json.dumps(value, separators=(",", ":"), sort_keys=True).encode()


def read_cap(device_seed: bytes, area: str, frm: int, until: int, broker=mr.BROKER):
    """The parent: a read capability the broker issued to one device at
    check-in (composition A's short-window renewal)."""
    return mr._signed(
        broker,
        {
            "access": "read",
            "area": area,
            "from": frm,
            "grantee": mr.pub(device_seed),
            "until": until,
        },
    )


def grant(mode: int, area: str, frm: int, until: int, parent: dict, recipient=None):
    body = {"area": area, "from": frm, "parent": parent, "until": until}
    if mode == MODE_RECIPIENT:
        body["recipient"] = recipient
    return mode, body


def encode_grants(grants: list[tuple[int, dict]]) -> bytes:
    items = sorted((mode, _canon(body)) for mode, body in grants)
    out = bytes([len(items)])
    for mode, raw in items:
        out += bytes([mode]) + struct.pack(">I", len(raw)) + raw
    return out


def _is_int(v) -> bool:
    return isinstance(v, int) and not isinstance(v, bool) and 0 <= v < 2**64


def _is_area(v) -> bool:
    return isinstance(v, str) and 0 < len(v) <= 1024 and v.isascii() and v.isprintable()


def decode_grants(raw: bytes) -> list[tuple[int, dict]]:
    """Structure only. Semantics are per grant, after the bundle is accepted."""
    if not raw or not 1 <= raw[0] <= MAX_GRANTS:
        raise GrantError("grants_malformed")
    pos, out, last = 1, [], None
    for _ in range(raw[0]):
        if pos + 5 > len(raw):
            raise GrantError("grants_malformed")
        mode, n = raw[pos], struct.unpack(">I", raw[pos + 1 : pos + 5])[0]
        body = raw[pos + 5 : pos + 5 + n]
        pos += 5 + n
        if len(body) != n:
            raise GrantError("grants_malformed")
        if mode not in MODES:
            raise GrantError("unknown_grant_mode")
        if last is not None and (mode, body) <= last:
            raise GrantError("grants_not_ascending")
        last = (mode, body)
        try:
            value = json.loads(body)
        except (ValueError, RecursionError):
            raise GrantError("grants_malformed") from None
        if not isinstance(value, dict) or _canon(value) != body:
            raise GrantError("grants_malformed")
        if set(value) != BODY_KEYS[mode]:
            raise GrantError("grants_malformed")
        parent = value["parent"]
        if not isinstance(parent, dict) or set(parent) != PARENT_KEYS:
            raise GrantError("grants_malformed")
        ints = (value["from"], value["until"], parent["from"], parent["until"])
        areas = (value["area"], parent["area"])
        if not all(map(_is_int, ints)) or not all(map(_is_area, areas)):
            raise GrantError("grants_malformed")
        if not all(isinstance(parent[k], str) for k in ("access", "grantee", "sig")):
            raise GrantError("grants_malformed")
        if mode == MODE_RECIPIENT:
            r = value["recipient"]
            hexpart = r[len("keyset:") :] if isinstance(r, str) else ""
            if not (
                isinstance(r, str)
                and r.startswith("keyset:")
                and len(hexpart) == 64
                and all(c in "0123456789abcdef" for c in hexpart)
            ):
                raise GrantError("grants_malformed")
        out.append((mode, value))
    if pos != len(raw):
        raise GrantError("grants_malformed")
    return out


def parent_id(parent: dict) -> str:
    return br.blake3_256(_canon(parent)).hex()


# ------------------------------------------------------------- per-grant check


def check_grant(mode: int, g: dict, m, host: GrantHost, now: int) -> str:
    """Return "applied" or the reason the grant is not applied. The bundle
    imports either way: its objects are signed history (slice 3 §7)."""
    p = g["parent"]
    if m.expires_at is not None and now >= m.expires_at:
        return "bundle_expired"
    if not any(mr._check(k, p) for k in host.brokers):
        return "grant_parent_unverified"
    if p["access"] != "read":
        return "grant_parent_not_read"
    if p["grantee"] != m.signing_key.hex():
        return "grant_parent_not_signer"
    if not g["area"].startswith(p["area"]):
        return "grant_area_outside_parent"
    if not g["from"] < g["until"]:
        return "grant_window_invalid"
    if g["from"] < p["from"] or g["until"] > p["until"]:
        return "grant_outlives_parent"
    if mode == MODE_BEARER and g["until"] - g["from"] > BEARER_MAX_SECONDS:
        return "bearer_lifetime_too_long"
    if parent_id(p) in host.withdrawn:
        return "grant_parent_withdrawn"
    if now < g["from"]:
        return "grant_not_yet_valid"
    if now >= g["until"]:
        return "grant_expired"
    if mode == MODE_RECIPIENT and g["recipient"] != host.identity:
        return "grant_not_for_you"
    return "applied"


# ------------------------------------------------------------- host


@dataclass
class GrantReport:
    result: str  # imported | held | rejected
    code: str
    grants: list[tuple[str, str, str]] = field(
        default_factory=list
    )  # mode, area, outcome
    released: list[str] = field(default_factory=list)  # digests admitted by a grant
    held: list[str] = field(default_factory=list)  # still held_no_read_grant


@dataclass
class GrantHost(mr.Host):
    """A slice-4 host that also applies slice-2 bundle grants."""

    identity: str = ""
    brokers: tuple = (mr.pub(mr.BROKER),)
    withdrawn: set = field(default_factory=set)  # parent ids the broker withdrew
    _grant_areas: tuple = ()

    def readable(self, area: str) -> bool:
        return super().readable(area) or any(
            area.startswith(a) for a in self._grant_areas
        )

    def offer(self, obj: bytes, env: bytes, path: str) -> str:
        out = super().offer(obj, env, path)
        if out == "admitted":
            self.held.pop(mr.did(obj), None)
        return out

    def export_with_grants(
        self,
        grants: list[tuple[int, dict]],
        pick=lambda d, o, e: True,
        created_at: int = T0 + 3600,
        expires_at: int | None = None,
        bodies: bool = True,
        envelopes: bool = True,
    ) -> list[bytes]:
        objects = {}
        for d, (o, e) in self.store.items():
            if pick(d, o, e):
                if bodies:
                    objects[bytes.fromhex(d)] = (2 if b'"verb"' in o else 1, o)
                if envelopes:
                    objects[br.blake3_256(e)] = (4, e)
        template = br.Manifest(
            creator_identity="keyset:" + self.chain[0]["entry"]["admin_key"],
            signing_key=br.ed25519_public(self.owner_device),
            created_at=created_at,
            entries=[],
            creator_binding=ir.encode_binding(self.chain),
            expires_at=expires_at,
            grants=encode_grants(grants) if grants else None,
        )
        return cp.split(
            template, self.owner_device, objects, cp.CARRIERS["removable-media"]
        )

    def import_with_grants(self, blob: bytes, now: int) -> GrantReport:
        # Stage 1, after slice 1's own stage-1 checks: grant structure. A
        # bundle whose header or manifest is already unreadable gets slice 1's
        # code from the importer below instead.
        raw = None
        try:
            mlen = struct.unpack(">I", blob[10:14])[0]
            raw = br.parse_manifest(blob[14 : 14 + mlen], grants=True).grants
        except (br.BundleError, struct.error):
            pass
        grants = []
        if raw is not None:
            try:
                grants = decode_grants(raw)
            except GrantError as exc:
                return GrantReport("rejected", str(exc))
        # Stages 2-4: signature, creator binding, digests (slice 3).
        out = ir.import_bundle(blob, self.rx, now, grants=True)
        if out.result != "imported":
            return GrantReport(out.result, out.code)
        m = br.parse_manifest(blob[14 : 14 + mlen], grants=True)
        report = GrantReport("imported", "ok")
        applied = []
        for mode, g in grants:
            verdict = check_grant(mode, g, m, self, now)
            report.grants.append((MODES[mode], g["area"], verdict))
            if verdict == "applied":
                applied.append(g["area"])
        # Stage 5: place pairs through slice 4's admission function. A grant
        # makes its area readable for this bundle's pairs only.
        pos, objs, envs = 14 + mlen + 65, {}, {}
        for e in sorted(m.entries, key=lambda e: e.digest):
            data = blob[pos : pos + e.size]
            pos += e.size
            if e.kind == 4:
                try:
                    envs[json.loads(data)["payload"]] = data
                except (ValueError, KeyError, TypeError):
                    continue
            else:
                objs[e.digest.hex()] = data
        self._grant_areas = tuple(applied)

        def place(obj: bytes, env: bytes) -> None:
            # offer() validates the envelope before its area is read here.
            if self.offer(obj, env, "bundle") == "admitted" and not mr.Host.readable(
                self, json.loads(env)["area"]
            ):
                report.released.append(mr.did(obj))

        try:
            for d in sorted(objs):
                if d in envs:
                    place(objs[d], envs[d])
                elif d not in self.store and d not in self.pruned:
                    self.unplaced[d] = objs[d]
            # An envelope whose object arrived earlier, in another bundle.
            for d in sorted(set(envs) - set(objs)):
                if d in self.unplaced:
                    place(self.unplaced[d], envs[d])
        finally:
            self._grant_areas = ()
        report.held = sorted(d for d in set(objs) | set(envs) if d in self.held)
        return report


# ------------------------------------------------------------- fixture


def hosts():
    """Slice 4's diverged hosts as grant-aware hosts, merged by sync so the
    shared area is equal, plus a third host (Cat's) that trusts Ben."""
    h1, h2, ids = mr.diverged()
    ana, ben = (
        GrantHost(**vars(h), identity="keyset:" + h.chain[0]["entry"]["admin_key"])
        for h in (h1, h2)
    )
    ana.sync_from(ben)
    ben.sync_from(ana)
    cat_chain = mr.keyset(mr.CAT, CAT_DEV)
    cat = GrantHost(
        "cat",
        CAT_DEV,
        cat_chain,
        ("garden/", "ben/"),
        frozenset(mr.ALL),
        ("garden/",),
        identity="keyset:" + cat_chain[0]["entry"]["admin_key"],
    )
    cat.rx = ir.Receiver()
    cat.rx.trust(ben.chain[0])
    cat.sync_from(ben)
    return ana, ben, cat, ids


CAT_DEV = mr._seed("cat-phone")
NOTES = "ben/notes"
# Ben's phone holds a 30-day read capability for his own area from the broker.
BEN_READ = read_cap(mr.BEN_DEV, "ben/", T0 - DAY, T0 + 30 * DAY)


def notes_only(d, o, e):
    return json.loads(e)["area"].startswith(NOTES)


def fail(msg: str) -> int:
    print(f"FAIL: {msg}")
    return 1


def self_test() -> int:
    ana0, ben0, cat0, ids = hosts()
    private = ids["private"]
    if private in ana0.store or private in cat0.store:
        return fail("private note visible before any grant")
    ana_id = ana0.identity
    now = T0 + 2 * DAY
    to_ana = grant(MODE_RECIPIENT, NOTES, T0, T0 + 14 * DAY, BEN_READ, ana_id)

    # 1. Recipient-bound: Ana's host applies it, the note is released, also
    # from the hold an earlier grant-less bundle left it in (slice 4 §6).
    ana = copy.deepcopy(ana0)
    (plain0,) = ben0.export(pick=notes_only)
    if ana.import_with_grants(plain0, now).held != [private]:
        return fail("a grant-less bundle did not hold the note")
    (blob,) = ben0.export_with_grants([to_ana], notes_only)
    r = ana.import_with_grants(blob, now)
    if r.grants != [("recipient", NOTES, "applied")] or r.released != [private]:
        return fail(f"recipient-bound grant: {r}")
    if private not in ana.store or ana.held:
        return fail("note not admitted on the recipient's host")
    granted = ana

    # 2. The same file on Cat's host (a copied stick): not for Cat, still held.
    cat = copy.deepcopy(cat0)
    r = cat.import_with_grants(blob, now)
    if r.grants != [("recipient", NOTES, "grant_not_for_you")] or r.held != [private]:
        return fail(f"copied stick: {r}")
    if private in cat.store:
        return fail("copied stick released the note")

    # 3. Bearer opt-in, 3 days: any host that imports it in time reads it.
    bearer = grant(MODE_BEARER, NOTES, T0, T0 + 3 * DAY, BEN_READ)
    (bblob,) = ben0.export_with_grants([bearer], notes_only)
    cat = copy.deepcopy(cat0)
    r = cat.import_with_grants(bblob, now)
    if r.grants != [("bearer", NOTES, "applied")] or r.released != [private]:
        return fail(f"bearer grant: {r}")

    # The grant covers this bundle's pairs only, never the area from now on.
    later = mr.content(mr.BEN, "Second private list", T0 + 200)
    ben = copy.deepcopy(ben0)
    ben.write(mr.BEN, later, NOTES, mr.cap(mr.BEN, "ben/", mr.ALL), T0 + 200)
    (plain,) = ben.export(pick=notes_only)
    r = cat.import_with_grants(plain, now)
    if mr.did(later) not in r.held or mr.did(later) in cat.store:
        return fail("a grant leaked to a later bundle's pairs")
    cat.sync_from(ben)
    if mr.did(later) in cat.store:
        return fail("a bundle grant let sync deliver the area")

    # The object and its envelope may arrive in separate bundles: the grant
    # travels with the envelope and still releases the object.
    ana = copy.deepcopy(ana0)
    (objs_only,) = ben0.export_with_grants([], notes_only, envelopes=False)
    (env_only,) = ben0.export_with_grants([to_ana], notes_only, bodies=False)
    if ana.import_with_grants(objs_only, now).result != "imported":
        return fail("object-only bundle")
    if private not in ana.unplaced:
        return fail("an object without its envelope was not kept unplaced")
    r = ana.import_with_grants(env_only, now)
    if r.released != [private] or private not in ana.store or ana.unplaced:
        return fail(f"envelope-only bundle with a grant: {r}")

    # An envelope missing its fields, from an enrolled device, is refused by
    # admission and does not crash the importer.
    ana = copy.deepcopy(ana0)
    bad_env = mr._json({"payload": private})
    m_bad = br.Manifest(
        creator_identity=ben0.identity,
        signing_key=br.ed25519_public(mr.BEN_DEV),
        created_at=T0 + 3600,
        entries=[],
        creator_binding=ir.encode_binding(ben0.chain),
        grants=encode_grants([to_ana]),
    )
    note = ben0.store[private][0]
    (bad,) = cp.split(
        m_bad,
        mr.BEN_DEV,
        {bytes.fromhex(private): (1, note), br.blake3_256(bad_env): (4, bad_env)},
        cp.CARRIERS["removable-media"],
    )
    r = ana.import_with_grants(bad, now)
    if r.result != "imported" or r.released or private in ana.store:
        return fail(f"malformed envelope: {r}")

    # 4. Renewal-collision and validity cases: one change each from case 1.
    other_dev = read_cap(mr.ANA_DEV, "ben/", T0 - DAY, T0 + 30 * DAY)
    forged = {**BEN_READ, "until": T0 + 40 * DAY}
    write_cap = mr._signed(
        mr.BROKER,
        {**{k: v for k, v in BEN_READ.items() if k != "sig"}, "access": "write"},
    )
    renewed = read_cap(mr.BEN_DEV, "ben/", T0 + 29 * DAY, T0 + 59 * DAY)
    cases = [
        # (name, grant, now, withdrawn parents, expires_at, expected)
        (
            "outlives parent",
            grant(MODE_RECIPIENT, NOTES, T0, T0 + 31 * DAY, BEN_READ, ana_id),
            now,
            (),
            None,
            "grant_outlives_parent",
        ),
        (
            "starts before parent",
            grant(MODE_RECIPIENT, NOTES, T0 - 2 * DAY, T0 + DAY, BEN_READ, ana_id),
            now,
            (),
            None,
            "grant_outlives_parent",
        ),
        (
            "creator not renewed, grant still in window",
            to_ana,
            T0 + 13 * DAY,
            (),
            None,
            "applied",
        ),
        ("imported after grant end", to_ana, T0 + 14 * DAY, (), None, "grant_expired"),
        (
            "parent renewed, old grant not extended",
            to_ana,
            T0 + 20 * DAY,
            (),
            None,
            "grant_expired",
        ),
        (
            "parent withdrawn before import",
            to_ana,
            now,
            (parent_id(BEN_READ),),
            None,
            "grant_parent_withdrawn",
        ),
        (
            "not yet valid",
            grant(MODE_RECIPIENT, NOTES, T0 + 5 * DAY, T0 + 9 * DAY, BEN_READ, ana_id),
            now,
            (),
            None,
            "grant_not_yet_valid",
        ),
        ("bundle expired", to_ana, now, (), T0 + DAY + 3600, "bundle_expired"),
        (
            "area outside parent",
            grant(MODE_RECIPIENT, "garden/", T0, T0 + 9 * DAY, BEN_READ, ana_id),
            now,
            (),
            None,
            "grant_area_outside_parent",
        ),
        (
            "parent is another device's",
            grant(MODE_RECIPIENT, NOTES, T0, T0 + 9 * DAY, other_dev, ana_id),
            now,
            (),
            None,
            "grant_parent_not_signer",
        ),
        (
            "parent not broker-signed",
            grant(MODE_RECIPIENT, NOTES, T0, T0 + 9 * DAY, forged, ana_id),
            now,
            (),
            None,
            "grant_parent_unverified",
        ),
        (
            "parent is a write capability",
            grant(MODE_RECIPIENT, NOTES, T0, T0 + 9 * DAY, write_cap, ana_id),
            now,
            (),
            None,
            "grant_parent_not_read",
        ),
        (
            "bearer longer than 7 days",
            grant(MODE_BEARER, NOTES, T0, T0 + 8 * DAY, BEN_READ),
            now,
            (),
            None,
            "bearer_lifetime_too_long",
        ),
        (
            "empty window",
            grant(MODE_RECIPIENT, NOTES, T0 + DAY, T0 + DAY, BEN_READ, ana_id),
            now,
            (),
            None,
            "grant_window_invalid",
        ),
        (
            "from renewed parent",
            grant(MODE_RECIPIENT, NOTES, T0 + 29 * DAY, T0 + 40 * DAY, renewed, ana_id),
            T0 + 31 * DAY,
            (),
            None,
            "applied",
        ),
    ]
    for name, g, at, withdrawn, expires, want in cases:
        h = copy.deepcopy(ana0)
        h.withdrawn = set(withdrawn)
        (b,) = ben0.export_with_grants([g], notes_only, expires_at=expires)
        r = h.import_with_grants(b, at)
        got = r.grants[0][2] if r.grants else r.code
        if r.result != "imported" or got != want:
            return fail(f"{name}: {r.result} {got}, want {want}")
        if (private in h.store) != (want == "applied"):
            return fail(f"{name}: release does not match the verdict")

    # 5. No onward delegation: Ana lifts Ben's grant into her own bundle.
    ana2 = copy.deepcopy(granted)  # holds the released note from case 1
    cat = copy.deepcopy(cat0)
    cat.rx.trust(ana2.chain[0])
    lifted = grant(MODE_BEARER, NOTES, T0, T0 + 3 * DAY, BEN_READ)
    (lb,) = ana2.export_with_grants([lifted], notes_only)
    r = cat.import_with_grants(lb, now)
    if (
        r.grants != [("bearer", NOTES, "grant_parent_not_signer")]
        or private in cat.store
    ):
        return fail(f"onward delegation: {r}")

    # 6. A revoked signing device rejects the whole bundle before any grant
    # is looked at (slice 3 stage 3).
    phone2 = mr._seed("ben-phone-2")
    g0 = ben0.chain[0]
    enrol = ir.sign_record(
        mr.BEN,
        2,
        g0["hash"],
        ir.entry("device_enrolled", device_key=mr.pub(phone2), label="phone 2"),
        counter=phone2,
    )
    revoke = ir.sign_record(
        mr.BEN,
        3,
        enrol["hash"],
        ir.entry("device_revoked", device_key=mr.pub(mr.BEN_DEV), reason="lost"),
    )
    h = copy.deepcopy(ana0)
    for rec in (enrol, revoke):
        h.rx.keysets[g0["entry"]["admin_key"]][rec["hash"]] = rec
    r = h.import_with_grants(blob, now)
    if (r.result, r.code, r.grants) != ("rejected", "signing_key_revoked", []):
        return fail(f"revoked device: {r}")

    # 7. Structural refusals reject the bundle at stage 1, before hashing.
    def with_raw(raw: bytes) -> bytes:
        (b,) = ben0.export_with_grants([to_ana], notes_only)
        return _swap_grants(b, raw, mr.BEN_DEV)

    good_raw = encode_grants([to_ana])
    _, body = to_ana
    body_raw = _canon(body)
    structural = [
        ("zero grants", b"\x00", "grants_malformed"),
        (
            "unknown mode",
            bytes([1, 3]) + struct.pack(">I", len(body_raw)) + body_raw,
            "unknown_grant_mode",
        ),
        (
            "non-canonical body",
            bytes([1, 1]) + struct.pack(">I", len(body_raw) + 1) + body_raw + b" ",
            "grants_malformed",
        ),
        (
            "bearer with a recipient",
            bytes([1, 2]) + struct.pack(">I", len(body_raw)) + body_raw,
            "grants_malformed",
        ),
        (
            "duplicate grant",
            bytes([2]) + good_raw[1:] + good_raw[1:],
            "grants_not_ascending",
        ),
        ("trailing bytes", good_raw + b"\x00", "grants_malformed"),
        (
            "deep nesting",
            bytes([1, 1]) + struct.pack(">I", 200000) + b"[" * 100000 + b"]" * 100000,
            "grants_malformed",
        ),
        (
            "bad recipient",
            encode_grants(
                [grant(MODE_RECIPIENT, NOTES, T0, T0 + DAY, BEN_READ, "idp:ana")]
            ),
            "grants_malformed",
        ),
    ]
    for name, raw, want in structural:
        h = copy.deepcopy(ana0)
        r = h.import_with_grants(with_raw(raw), now)
        if (r.result, r.code) != ("rejected", want):
            return fail(f"{name}: {r.result} {r.code}, want {want}")

    # 8. An older importer still refuses grants, as slice 1 and 3 promised.
    if (
        ir.import_bundle(blob, copy.deepcopy(ana0).rx, now).code
        != "grants_reserved_slice_2"
    ):
        return fail("a slice-3 importer accepted a grants record")

    # 9. Sizes: what a grant costs on each carrier.
    size = len(encode_grants([to_ana]))
    print(
        f"ok: recipient-bound grant releases the note on the recipient's host only, "
        f"bearer opt-in releases it anywhere within 3 days, grants cover the bundle's "
        f"own pairs and never sync, {len(cases)} renewal and validity cases, "
        f"no onward delegation, revoked device rejects first, "
        f"{len(structural)} structural refusals, older importers refuse; "
        f"one recipient-bound grant is {size} bytes"
    )
    return 0


def _swap_grants(blob: bytes, raw: bytes, seed: bytes) -> bytes:
    """Re-sign `blob` with its grants record replaced by `raw`."""
    mlen = struct.unpack(">I", blob[10:14])[0]
    body = blob[14 : 14 + mlen]
    recs, pos = [], 0
    while pos < len(body):
        tag, n = body[pos], struct.unpack(">I", body[pos + 1 : pos + 5])[0]
        recs.append((tag, raw if tag == 0x08 else body[pos + 5 : pos + 5 + n]))
        pos += 5 + n
    new = b"".join(bytes([t]) + struct.pack(">I", len(v)) + v for t, v in recs)
    return (
        br.MAGIC
        + struct.pack(">HI", br.VERSION, len(new))
        + new
        + b"\x01"
        + br.ed25519_sign(seed, br.DOMAIN + new)
        + blob[14 + mlen + 65 :]
    )


if __name__ == "__main__":
    sys.exit(self_test())
