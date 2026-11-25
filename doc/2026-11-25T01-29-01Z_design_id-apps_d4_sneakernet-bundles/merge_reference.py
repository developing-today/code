#!/usr/bin/env python3
"""Reference merge-on-contact model for ID sneakernet bundles (D4 slice 4,
#13889). Python stdlib only; loads the slice 1, 3 and 5 references beside it.

Two hosts start from one shared graph, diverge, and are merged twice: once by
network sync and once by bundles. The reference checks that both paths reach
the same state, and that every difference is one this slice names (§5 of
slice-4-merge-on-contact.md).

The model of composition A here is deliberately small:

  envelope = {area, author, cap, payload, timestamp, sig}
    the data-plane operation that placed one object: the object's digest
    (payload), the area it lives in, the author's capability (signed by the
    broker), signed by the author
  cap = {area, from, grantee, moderate, until, verbs, sig}

A host admits an (object, envelope) pair by one function whichever way it
arrived; only what reaches that function differs between sync and bundles.

Run with no arguments (or self-test) for the self-test.
"""

from __future__ import annotations

import copy
import hashlib
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

KNOWN_VERBS = ("reply", "mention", "quote", "attach", "supersede", "nest")
T0 = ir.T0


def _seed(name: str) -> bytes:
    return hashlib.sha256(b"id-bundle slice 4 test key " + name.encode()).digest()


def _json(v) -> bytes:
    return json.dumps(v, separators=(",", ":"), sort_keys=True).encode()


def pub(seed: bytes) -> str:
    return br.ed25519_public(seed).hex()


BROKER = _seed("broker")


def _signed(seed: bytes, value: dict) -> dict:
    return {**value, "sig": br.ed25519_sign(seed, _json(value)).hex()}


def _check(key_hex: str, value: dict) -> bool:
    body = {k: v for k, v in value.items() if k != "sig"}
    try:
        return br.ed25519_verify(
            bytes.fromhex(key_hex), _json(body), bytes.fromhex(value["sig"])
        )
    except (KeyError, ValueError):
        return False


def cap(seed, area, verbs, moderate=False, frm=T0 - 86400, until=T0 + 30 * 86400):
    return _signed(
        BROKER,
        {
            "area": area,
            "from": frm,
            "grantee": pub(seed),
            "moderate": moderate,
            "until": until,
            "verbs": sorted(verbs),
        },
    )


def envelope(seed, obj, area, capd, t, payload=None, author=None) -> bytes:
    """One data-plane operation placing `obj` in `area`, signed by `seed`.
    `payload` and `author` are overridable only so the self-test can forge."""
    value = {
        "area": area,
        "author": author or pub(seed),
        "cap": capd,
        "payload": payload or did(obj),
        "timestamp": t,
    }
    return _json(_signed(seed, value))


def content(seed, body, t, kind="chat.message") -> bytes:
    return _json(
        {
            "author": pub(seed),
            "body": body,
            "created_claim": t,
            "kind": kind,
            "schema_version": 1,
        }
    )


def link(seed, verb, source, target, t, context=None) -> bytes:
    v = {
        "author": pub(seed),
        "created_claim": t,
        "schema_version": 1,
        "source": source,
        "target": target,
        "verb": verb,
    }
    if context:
        v["context"] = context
    return _json(v)


def did(obj: bytes) -> str:
    return br.blake3_256(obj).hex()


# ------------------------------------------------------------- host


@dataclass
class Host:
    name: str
    owner_device: bytes
    chain: list
    sub_areas: tuple
    sub_verbs: frozenset
    read_areas: tuple
    rx: object = None
    store: dict = field(default_factory=dict)  # digest -> (object, envelope)
    unplaced: dict = field(default_factory=dict)  # digest -> object, no envelope yet
    held: dict = field(default_factory=dict)  # digest -> (object, envelope)
    pruned: set = field(default_factory=set)

    def readable(self, area: str) -> bool:
        return any(area.startswith(a) for a in self.read_areas)

    def write(self, seed: bytes, obj: bytes, area: str, capd: dict, t: int) -> str:
        env = envelope(seed, obj, area, capd, t)
        return self.offer(obj, env, "local")

    def offer(self, obj: bytes, env: bytes, path: str) -> str:
        """The one admission function. `path` only decides what an unreadable
        area does: sync never delivers it, a bundle has already delivered it."""
        d = did(obj)
        if d in self.pruned:
            return "pruned"
        if d in self.store:
            return "have"
        try:
            e, o = json.loads(env), json.loads(obj)
            c = e["cap"]
            ok = (
                e["payload"] == d
                and _check(e["author"], e)
                and _check(pub(BROKER), c)
                and o["author"] == e["author"] == c["grantee"]
                and e["area"].startswith(c["area"])
                and c["from"] <= e["timestamp"] < c["until"]
                and o.get("verb", "content") in c["verbs"]
            )
        except (KeyError, TypeError, ValueError):
            ok = False
        if not ok:
            return "unauthorised"  # a forged envelope leaves an unplaced object waiting
        # A valid envelope settles the object: kept, held, or declined by subscription.
        self.unplaced.pop(d, None)
        if not any(e["area"].startswith(a) for a in self.sub_areas):
            return "not_subscribed"
        if (
            o.get("verb", "content") not in self.sub_verbs
            and o.get("verb") in KNOWN_VERBS
        ):
            return "verb_not_subscribed"
        if not self.readable(e["area"]):
            if path == "sync":
                return "not_sent"
            self.held[d] = (obj, env)
            return "held_no_read_grant"
        self.store[d] = (obj, env)
        return "admitted"

    # ---- network sync: the peer sends every pair it holds; the receiver admits

    def sync_from(self, peer: Host, reverse=False) -> None:
        for d in sorted(peer.store, reverse=reverse):
            if d not in self.store:
                self.offer(*peer.store[d], "sync")

    # ---- bundles

    def export(
        self, pick=lambda d, o, e: True, envelopes=True, bodies=True
    ) -> list[bytes]:
        objects = {}
        for d, (o, e) in self.store.items():
            if not pick(d, o, e):
                continue
            if bodies:
                objects[bytes.fromhex(d)] = (2 if b'"verb"' in o else 1, o)
            if envelopes:
                objects[br.blake3_256(e)] = (4, e)
        return ir.export(
            self.owner_device,
            self.chain,
            objects,
            cp.CARRIERS["removable-media"],
            T0 + 3600,
        )

    def import_bundle(self, blob: bytes) -> str:
        out = ir.import_bundle(blob, self.rx, T0 + 7200)
        if out.result != "imported":
            return out.code
        body_len = struct.unpack(">I", blob[10:14])[0]
        m = br.parse_manifest(blob[14 : 14 + body_len])
        pos, objs, envs = 14 + body_len + 65, {}, {}
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
        for d in sorted(objs):
            if d in envs:
                self.offer(objs[d], envs[d], "bundle")
            elif d not in self.store and d not in self.pruned:
                self.unplaced[d] = objs[d]
        for d in sorted(set(envs) - set(objs)):
            if d in self.unplaced:
                self.offer(self.unplaced[d], envs[d], "bundle")
        return "ok"

    # ---- derived state: computed only from the store, so order cannot matter

    def views(self) -> dict:
        objs = {d: json.loads(o) for d, (o, _) in self.store.items()}
        caps = {d: json.loads(e)["cap"] for d, (_, e) in self.store.items()}
        areas = {d: json.loads(e)["area"] for d, (_, e) in self.store.items()}
        links = {d: o for d, o in objs.items() if "verb" in o}
        unknown = sorted(d for d, o in links.items() if o["verb"] not in KNOWN_VERBS)
        valid_sup, ignored, pending = [], [], []
        for d, o in links.items():
            if o["verb"] != "supersede":
                continue
            src, tgt = objs.get(o["source"]), objs.get(o["target"])
            if src is None or tgt is None:
                pending.append(d)
            elif src["author"] == tgt["author"] or (
                caps[d]["moderate"] and areas[o["target"]].startswith(caps[d]["area"])
            ):
                valid_sup.append((o["source"], o["target"]))
            else:
                ignored.append(d)
        parent = {}

        def root(x):
            while x in parent:
                x = parent[x]
            return x

        for s, t in sorted(valid_sup):
            rs, rt = root(s), root(t)
            if rs != rt:
                parent[max(rs, rt)] = min(rs, rt)
        members = {}
        for s, t in valid_sup:
            members.setdefault(root(t), set()).update((s, t))
        superseded = {t for _, t in valid_sup}
        chains = {
            r: {"heads": sorted(m - superseded), "shown": min(m - superseded)}
            for r, m in members.items()
        }
        replies, depth = {}, {}

        def depth_of(x, rootid, seen=()):
            if x == rootid:
                return 0
            if x in depth:
                return depth[x]
            prev = [
                p
                for lk in links.values()
                if lk["verb"] == "reply" and lk["source"] == x
                for p in lk.get("context", {}).get("previous", [])
                if p in objs and p not in seen
            ]
            depth[x] = 1 + max(
                (depth_of(p, rootid, (*seen, x)) for p in prev), default=0
            )
            return depth[x]

        for d, o in links.items():
            if o["verb"] == "reply" and o["source"] in objs:
                r = o.get("context", {}).get("root", o["target"])
                replies.setdefault(r, []).append(o["source"])
        for r, srcs in replies.items():
            srcs.sort(key=lambda x: (depth_of(x, r), objs[x]["created_claim"], x))
        nest = {}
        for o in links.values():
            if o["verb"] == "nest":
                nest.setdefault(o["target"], []).append(o["source"])
        return {
            "objects": sorted(objs),
            "areas": areas,
            "chains": chains,
            "replies": replies,
            "nest": {k: sorted(v) for k, v in nest.items()},
            "unknown_verbs_skipped": unknown,
            "ignored_supersedes": sorted(ignored),
            "pending_supersedes": sorted(pending),
        }


# ------------------------------------------------------------- fixture

ANA, ANA_DEV, BEN, BEN_DEV, CAT, MOD = (
    _seed(n) for n in ("ana", "ana-phone", "ben", "ben-phone", "cat", "mod")
)
ALL = KNOWN_VERBS + ("content",)


def keyset(admin, device):
    return [
        ir.sign_record(
            admin,
            1,
            "",
            ir.entry(
                "genesis", admin_key=pub(admin), device_key=pub(device), label="phone"
            ),
        )
    ]


def diverged():
    """Two hosts that shared a graph, then went their own ways."""
    ana_chain, ben_chain = keyset(ANA, ANA_DEV), keyset(BEN, BEN_DEV)
    h1 = Host(
        "ana",
        ANA_DEV,
        ana_chain,
        ("garden/", "ben/"),
        frozenset(ALL) - {"quote"},
        ("garden/",),
    )
    h2 = Host(
        "ben",
        BEN_DEV,
        ben_chain,
        ("garden/", "ben/"),
        frozenset(ALL),
        ("garden/", "ben/"),
    )
    for h, other in ((h1, ben_chain), (h2, ana_chain)):
        h.rx = ir.Receiver()
        h.rx.trust(other[0])
    caps = {
        "ana": cap(ANA, "garden/", ALL),
        # Ben's newer client knows a "cheer" verb and his broker grants it.
        "ben": cap(BEN, "garden/", ALL + ("cheer",)),
        "ben-own": cap(BEN, "ben/", ALL),
        "cat": cap(CAT, "garden/chat", ("content", "reply", "supersede")),
        "mod": cap(MOD, "garden/", ("content", "supersede"), moderate=True),
    }
    who = {"ana": ANA, "ben": BEN, "ben-own": BEN, "cat": CAT, "mod": MOD}
    ids = {}

    def put(hosts, capname, area, obj, t, name):
        for h in hosts:
            assert h.write(who[capname], obj, area, caps[capname], t) == "admitted", (
                name
            )
        ids[name] = did(obj)
        return ids[name]

    both = (h1, h2)
    # Shared history.
    board = put(
        both,
        "ana",
        "garden/board",
        content(ANA, "Garden board", T0, "board"),
        T0,
        "board",
    )
    r = put(
        both,
        "ana",
        "garden/chat",
        content(ANA, "Meetup Saturday?", T0 + 10),
        T0 + 10,
        "root",
    )
    b1 = put(
        both, "ben", "garden/chat", content(BEN, "Yes, 10am.", T0 + 20), T0 + 20, "b1"
    )
    put(
        both,
        "ben",
        "garden/chat",
        link(BEN, "reply", b1, r, T0 + 20, {"root": r, "previous": [r]}),
        T0 + 20,
        "b1-link",
    )
    old = put(
        both, "ana", "garden/chat", content(ANA, "Old note", T0 + 30), T0 + 30, "old"
    )
    # Ana's host, offline.
    a2 = put(
        (h1,),
        "ana",
        "garden/chat",
        content(ANA, "Bring gloves.", T0 + 15),  # Ana's phone clock runs slow
        T0 + 100,
        "a2",
    )
    put(
        (h1,),
        "ana",
        "garden/chat",
        link(ANA, "reply", a2, r, T0 + 100, {"root": r, "previous": [b1]}),
        T0 + 100,
        "a2-link",
    )
    r1 = put(
        (h1,),
        "ana",
        "garden/chat",
        content(ANA, "Meetup Saturday 10am?", T0 + 110),
        T0 + 110,
        "r1",
    )
    put(
        (h1,),
        "ana",
        "garden/chat",
        link(ANA, "supersede", r1, r, T0 + 110),
        T0 + 110,
        "edit",
    )
    put(
        (h1,),
        "ana",
        "garden/board",
        link(ANA, "nest", r, board, T0 + 120),
        T0 + 120,
        "nest",
    )
    h1.store.pop(old)
    h1.pruned.add(old)
    # Ben's host, offline.
    b3 = put(
        (h2,),
        "ben",
        "garden/chat",
        content(BEN, "I'll bring seeds.", T0 + 105),
        T0 + 105,
        "b3",
    )
    put(
        (h2,),
        "ben",
        "garden/chat",
        link(BEN, "reply", b3, r, T0 + 105, {"root": r, "previous": [b1]}),
        T0 + 105,
        "b3-link",
    )
    photo = put(
        (h2,),
        "ben",
        "garden/chat",
        content(BEN, "photo bytes", T0 + 106, "image"),
        T0 + 106,
        "photo",
    )
    put(
        (h2,),
        "ben",
        "garden/chat",
        link(BEN, "attach", b3, photo, T0 + 106),
        T0 + 106,
        "attach",
    )
    put(
        (h2,),
        "ben",
        "garden/chat",
        link(BEN, "quote", b3, b1, T0 + 107),
        T0 + 107,
        "quote",
    )
    put(
        (h2,),
        "ben",
        "garden/chat",
        link(BEN, "cheer", b3, r, T0 + 108),
        T0 + 108,
        "cheer",
    )
    put(
        (h2,),
        "ben",
        "garden/chat",
        link(BEN, "mention", b3, "ab" * 32, T0 + 109),
        T0 + 109,
        "mention",
    )
    tomb = put(
        (h2,),
        "mod",
        "garden/chat",
        content(MOD, "Removed: off topic", T0 + 115, "tombstone"),
        T0 + 115,
        "tomb",
    )
    put(
        (h2,),
        "mod",
        "garden/chat",
        link(MOD, "supersede", tomb, r, T0 + 115),
        T0 + 115,
        "moderation",
    )
    hij = put(
        (h2,),
        "cat",
        "garden/chat",
        content(CAT, "Meetup cancelled!", T0 + 116),
        T0 + 116,
        "hijack-post",
    )
    put(
        (h2,),
        "cat",
        "garden/chat",
        link(CAT, "supersede", hij, r, T0 + 116),
        T0 + 116,
        "hijack",
    )
    put(
        (h2,),
        "ben-own",
        "ben/notes",
        content(BEN, "Private list", T0 + 117),
        T0 + 117,
        "private",
    )
    return h1, h2, ids


# ------------------------------------------------------------- self-test


def fail(msg: str) -> int:
    print(f"FAIL: {msg}")
    return 1


def self_test() -> int:
    h1, h2, ids = diverged()

    # Path 1: network sync both ways.
    s1, s2 = copy.deepcopy(h1), copy.deepcopy(h2)
    s1.sync_from(s2)
    s2.sync_from(s1)
    # Path 2: one full bundle each way.
    b1, b2 = copy.deepcopy(h1), copy.deepcopy(h2)
    to_ana, to_ben = b2.export(), b1.export()
    for part in to_ana:
        if b1.import_bundle(part) != "ok":
            return fail("bundle to ana refused")
    for part in to_ben:
        if b2.import_bundle(part) != "ok":
            return fail("bundle to ben refused")

    if s1.views() != b1.views() or s2.views() != b2.views():
        return fail("sync and bundle views differ")
    # The one named difference: a bundle delivers an area the receiver cannot
    # read yet (held until slice 2's bundle grants), sync never sends it.
    if set(b1.held) != {ids["private"]} or s1.held or b2.held or s2.held:
        return fail(f"held sets {sorted(b1.held)} {sorted(s1.held)}")

    v = b1.views()
    root_chain = v["chains"].get(min(ids["root"], ids["r1"], ids["tomb"]))
    if root_chain is None or set(root_chain["heads"]) != {ids["r1"], ids["tomb"]}:
        return fail(f"fork heads {v['chains']}")
    if v["ignored_supersedes"] != [ids["hijack"]]:
        return fail("hijack not ignored")
    if v["unknown_verbs_skipped"] != [ids["cheer"]]:
        return fail("unknown verb not kept and skipped")
    if ids["quote"] in v["objects"] or ids["quote"] not in b2.views()["objects"]:
        return fail("quote should be dropped by ana's subscription only")
    if ids["old"] in v["objects"] or ids["old"] not in b2.views()["objects"]:
        return fail("pruned object came back, or ben lost it")
    if v["replies"][ids["root"]] != [ids["b1"], ids["a2"], ids["b3"]]:
        return fail(f"reply order {v['replies']}")
    if v["nest"] != {ids["board"]: [ids["root"]]}:
        return fail("nest")

    # Order cannot matter: parts reversed, sync reversed, a bundle twice.
    r1 = copy.deepcopy(h1)
    for part in reversed(to_ana):
        r1.import_bundle(part)
    for part in to_ana:
        r1.import_bundle(part)
    r2 = copy.deepcopy(h1)
    r2.sync_from(s2, reverse=True)
    if r1.views() != b1.views() or r2.views() != s1.views():
        return fail("order changed the result")

    # A scoped bundle: the difference is exactly what the scope left out, and
    # a later sync closes it.
    thread = {ids[n] for n in ("b3", "b3-link", "tomb", "moderation")}
    n1 = copy.deepcopy(h1)
    for part in copy.deepcopy(h2).export(pick=lambda d, o, e: d in thread):
        n1.import_bundle(part)
    missing = set(s1.views()["objects"]) - set(n1.views()["objects"])
    expect_missing = {
        ids[n] for n in ("photo", "attach", "cheer", "mention", "hijack-post", "hijack")
    }
    if missing != expect_missing:
        return fail(f"scoped bundle missing {len(missing)}")
    n1.sync_from(s2)
    if n1.views() != s1.views():
        return fail("sync after a scoped bundle did not converge")

    # A slice-1 bundle with no envelopes: objects arrive unplaced, out of every
    # view, and the next sync or bundle with envelopes places them.
    u1 = copy.deepcopy(h1)
    for part in copy.deepcopy(h2).export(envelopes=False):
        u1.import_bundle(part)
    if u1.views() != h1.views() or not u1.unplaced:
        return fail("unplaced objects reached a view")
    if ids["old"] in u1.unplaced:
        return fail("a pruned object was kept unplaced")
    for part in to_ana:
        u1.import_bundle(part)
    if u1.unplaced or u1.views() != b1.views():
        return fail("envelopes did not place the unplaced objects")

    # An envelope with no object places nothing and invents nothing.
    e1 = copy.deepcopy(h1)
    for part in copy.deepcopy(h2).export(bodies=False):
        e1.import_bundle(part)
    if e1.views() != h1.views() or e1.unplaced:
        return fail("an envelope without its object changed the host")

    # Admission refusals, one check each, freshly signed so only that check fails.
    ok_cap = cap(BEN, "garden/", ALL)
    post = content(BEN, "Hello", T0 + 200)
    other = content(BEN, "Other", T0 + 201)
    nest_link = link(CAT, "nest", ids["root"], ids["board"], T0 + 202)
    refusals = {
        "payload_mismatch": (
            post,
            envelope(BEN, post, "garden/chat", ok_cap, T0 + 200, payload=did(other)),
        ),
        "signed_by_other": (
            post,
            envelope(CAT, post, "garden/chat", ok_cap, T0 + 200, author=pub(BEN)),
        ),
        "cap_not_broker": (
            post,
            envelope(
                BEN,
                post,
                "garden/chat",
                _signed(CAT, {k: v for k, v in ok_cap.items() if k != "sig"}),
                T0 + 200,
            ),
        ),
        "object_author": (
            content(ANA, "Not Ben's", T0 + 200),
            envelope(
                BEN,
                content(ANA, "Not Ben's", T0 + 200),
                "garden/chat",
                ok_cap,
                T0 + 200,
            ),
        ),
        "cap_grantee": (
            content(CAT, "Cat", T0 + 200),
            envelope(
                CAT, content(CAT, "Cat", T0 + 200), "garden/chat", ok_cap, T0 + 200
            ),
        ),
        "area_outside_cap": (
            content(CAT, "Cat", T0 + 200),
            envelope(
                CAT,
                content(CAT, "Cat", T0 + 200),
                "garden/board",
                cap(CAT, "garden/chat", ALL),
                T0 + 200,
            ),
        ),
        "cap_not_yet": (
            post,
            envelope(
                BEN,
                post,
                "garden/chat",
                cap(BEN, "garden/", ALL, frm=T0 + 201),
                T0 + 200,
            ),
        ),
        "cap_expired": (
            post,
            envelope(
                BEN,
                post,
                "garden/chat",
                cap(BEN, "garden/", ALL, until=T0 + 200),
                T0 + 200,
            ),
        ),
        "verb_not_granted": (
            nest_link,
            envelope(
                CAT,
                nest_link,
                "garden/chat",
                cap(CAT, "garden/", ("content",)),
                T0 + 202,
            ),
        ),
    }
    for name, (o, env) in refusals.items():
        for path in ("sync", "bundle"):
            if copy.deepcopy(h1).offer(o, env, path) != "unauthorised":
                return fail(f"refusal {name} admitted by {path}")
    # The same post properly signed is admitted, so each refusal is its one check.
    if (
        copy.deepcopy(h1).offer(
            post, envelope(BEN, post, "garden/chat", ok_cap, T0 + 200), "sync"
        )
        != "admitted"
    ):
        return fail("control post refused")
    if (
        copy.deepcopy(h1).offer(
            content(BEN, "Elsewhere", T0 + 200),
            envelope(
                BEN,
                content(BEN, "Elsewhere", T0 + 200),
                "town/hall",
                cap(BEN, "town/", ALL),
                T0 + 200,
            ),
            "bundle",
        )
        != "not_subscribed"
    ):
        return fail("an unsubscribed area was admitted")
    if v["chains"][min(ids["root"], ids["r1"], ids["tomb"])]["shown"] != min(
        ids["r1"], ids["tomb"]
    ):
        return fail("fork does not show the lowest-id head")

    # A grant starting at the write's own second counts; a stored pair is "have".
    edge = envelope(
        BEN, post, "garden/chat", cap(BEN, "garden/", ALL, frm=T0 + 200), T0 + 200
    )
    if copy.deepcopy(h1).offer(post, edge, "bundle") != "admitted":
        return fail("a grant starting at the write was refused")
    if copy.deepcopy(h1).offer(*h1.store[ids["root"]], "sync") != "have":
        return fail("a stored pair was not reported as held already")

    # Forged envelopes are refused on both paths.
    f1 = copy.deepcopy(h1)
    obj, env = h2.store[ids["b3"]]
    bad = json.loads(env)
    bad["area"] = "garden/board"
    if f1.offer(obj, _json(bad), "bundle") != "unauthorised":
        return fail("edited envelope admitted")
    stolen = json.loads(env)
    stolen["cap"] = json.loads(h2.store[ids["tomb"]][1])["cap"]
    if f1.offer(obj, _json(stolen), "sync") != "unauthorised":
        return fail("someone else's capability admitted")

    print(
        f"ok: diverged hosts ({len(h1.store)} and {len(h2.store)} objects) merge to the same "
        f"views by sync and by bundle ({len(v['objects'])} objects on ana's host), one named "
        f"difference (an unreadable area is held), order and repeats change nothing, a scoped "
        f"bundle misses exactly {len(missing)} objects until sync, envelope-less objects stay "
        f"unplaced until their envelopes arrive, {len(refusals)} admission refusals on both paths"
    )
    return 0


if __name__ == "__main__":
    sys.exit(self_test())
