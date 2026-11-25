#!/usr/bin/env python3
"""Reference codec and resolver for the id:// v2 collection form (D1 slice 2, #13886).

Stdlib only. Run with:  python3 -I set_uri_reference.py self-test

This fills the `set/` shape that slice 1 reserved. It reuses the slice 1 codec
(graph_uri_reference.py) for everything outside the set path, so the resolver
hint, namespace, `any`, hints and fragment rules are unchanged.

Grammar (canonical form only; every other spelling is rejected):

  set-shape = "set/" root *( "/" param )       ; params in the order below, each once
  root      = ( "obj/" / "head/" ) cid
  param     = "verbs=" verb *( "," verb )      ; ASCII-sorted, no repeats, not supersede
            / "dir=" ( "in" / "out" )          ; omitted means both
            / "depth=" pos-u64                 ; omitted means unbounded
            / "within=" cid                    ; nest descendants of a container
            / "authors=" key *( "," key )      ; sorted, no repeats; omitted means any
            / "since=" u64 / "until=" u64      ; unix seconds, claimed time, since < until
            / "count=" pos-u64 / "bytes=" pos-u64
  verb      = %x61-7A *31( %x61-7A / DIGIT / "-" )
  key       = "ed25519." 64HEXLC

A set URI names a query, not a member list. Resolving it for one reader gives
the members, in the canonical breadth-first order that D5 already draws.

The acceptance checks load three sibling references: D5's view model (every D5
case written as a set URI resolves to the same nodes, hops, edges and cuts as
D5's build_frame), D4's bundle writer (a bundle whose scope is a set URI), and
D4's BLAKE3 and Ed25519 for real digests and keys over the fixture objects.
"""

from __future__ import annotations

import sys

sys.dont_write_bytecode = True

import datetime
import hashlib
import importlib.util
import json
import random
import re
from dataclasses import dataclass, field, replace
from pathlib import Path

HERE = Path(__file__).resolve().parent
DOC = HERE.parent


def _load(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module  # dataclasses look their module up by name
    spec.loader.exec_module(module)
    return module


g = _load("graph_uri_reference", HERE / "graph_uri_reference.py")
UriError = g.UriError

PARAM_ORDER = (
    "verbs",
    "dir",
    "depth",
    "within",
    "authors",
    "since",
    "until",
    "count",
    "bytes",
)
# What an omitted `verbs=` means: every followable verb in the registry, which
# is D3 v1 without supersede, plus D2's embed. A verb admitted later joins it;
# a URI that must not change with the registry lists its verbs.
REGISTRY_FOLLOWABLE = ("reply", "quote", "embed", "mention", "attach", "nest")
NOT_FOLLOWABLE = frozenset({"supersede"})
_VERB = re.compile(r"[a-z][a-z0-9-]{0,31}\Z", re.ASCII)
_KEY = re.compile(r"ed25519\.([0-9a-f]{64})\Z", re.ASCII)
_MAX_VERBS = 32
_MAX_AUTHORS = 64

# Pinned after the first run; any codec change that moves it is a format change.
CORPUS_SHA256 = "29679520fc281213e52402b09f52b8a66334f0b36f4a95be7ce307d6699619b0"


@dataclass(frozen=True)
class SetRef:
    resolver_hint: str
    namespace: str
    root_shape: str  # "obj" or "head"
    root: bytes
    verbs: tuple[str, ...] | None = None
    direction: str | None = None  # None means both
    depth: int | None = None
    within: bytes | None = None
    authors: tuple[bytes, ...] | None = None
    since: int | None = None
    until: int | None = None
    count: int | None = None
    max_bytes: int | None = None
    via: tuple[str, ...] = field(default=())
    fragment: str | None = None


def _pos(value: int | None) -> str | None:
    if value is None:
        return None
    if not isinstance(value, int) or not 1 <= value <= g._U64_MAX:
        raise UriError("invalid_bound")
    return str(value)


def _u64(value: int | None) -> str | None:
    if value is None:
        return None
    if not isinstance(value, int) or not 0 <= value <= g._U64_MAX:
        raise UriError("decimal_out_of_range")
    return str(value)


def format_set(ref: SetRef) -> str:
    g._check_alias(ref.resolver_hint)
    g._check_portable(ref.resolver_hint, ref.namespace)
    if ref.root_shape not in ("obj", "head"):
        raise UriError("invalid_set_root")
    parts = ["set", ref.root_shape, g._cid_text(ref.root)]
    values: dict[str, str | None] = {}
    if ref.verbs is not None:
        if not ref.verbs:
            raise UriError("empty_parameter_value")
        for verb in ref.verbs:
            if not _VERB.fullmatch(verb):
                raise UriError("invalid_verb_token")
            if verb in NOT_FOLLOWABLE:
                raise UriError("verb_not_followable")
        if list(ref.verbs) != sorted(set(ref.verbs)):
            raise UriError("noncanonical_list_order")
        if len(ref.verbs) > _MAX_VERBS:
            raise UriError("too_many_verbs")
        values["verbs"] = ",".join(ref.verbs)
    if ref.direction is not None:
        if ref.direction == "both":
            raise UriError("noncanonical_default")
        if ref.direction not in ("in", "out"):
            raise UriError("invalid_direction")
        values["dir"] = ref.direction
    values["depth"] = _pos(ref.depth)
    if ref.within is not None:
        values["within"] = g._cid_text(ref.within)
    if ref.authors is not None:
        if not ref.authors:
            raise UriError("empty_parameter_value")
        if any(len(key) != 32 for key in ref.authors):
            raise UriError("invalid_author_key")
        if list(ref.authors) != sorted(set(ref.authors)):
            raise UriError("noncanonical_list_order")
        if len(ref.authors) > _MAX_AUTHORS:
            raise UriError("too_many_authors")
        values["authors"] = ",".join("ed25519." + key.hex() for key in ref.authors)
    values["since"] = _u64(ref.since)
    values["until"] = _u64(ref.until)
    if ref.since is not None and ref.until is not None and ref.since >= ref.until:
        raise UriError("invalid_time_window")
    values["count"] = _pos(ref.count)
    values["bytes"] = _pos(ref.max_bytes)
    for key in PARAM_ORDER:
        if values.get(key) is not None:
            parts.append(f"{key}={values[key]}")
    head = f"id://{ref.resolver_hint}/{g._ns_text(ref.namespace)}/v2/"
    uri = head + "/".join(parts) + g._hints(ref.via, ref.fragment)
    if len(uri) > g._MAX_LEN:
        raise UriError("uri_too_long")
    return uri


def _split(uri: str) -> tuple[str, str, list[str]] | None:
    """(alias, suffix, set tail) for a v2 set URI, else None."""
    if not uri.startswith("id://"):
        return None
    rest = uri[5:]
    path = rest.partition("#")[0].partition("?")[0]
    suffix = rest[len(path) :]
    alias, _, path = path.partition("/")
    segs = path.split("/")
    if len(segs) < 3 or segs[1] != "v2" or segs[2] != "set":
        return None
    return alias, suffix, [segs[0]] + segs[3:]


def parse(uri: str) -> SetRef | g.ObjRef | g.HeadRef:
    """Slice 1 parse, with the `set/` shape filled in."""
    if len(uri) > g._MAX_LEN:
        raise UriError("uri_too_long")
    if any(ord(c) < 0x21 or ord(c) > 0x7E for c in uri):
        raise UriError("raw_control_space_or_nonascii")
    split = _split(uri)
    if split is None:
        return g.parse(uri)
    alias, suffix, segs = split
    tail = segs[1:]
    if len(tail) < 2 or any(s == "" for s in tail):
        raise UriError("empty_or_missing_path_component")
    if tail[0] not in ("obj", "head"):
        raise UriError("invalid_set_root")
    # The slice 1 codec checks the hint, namespace, root cid, hints and fragment.
    base = g.parse(f"id://{alias}/{segs[0]}/v2/obj/{tail[1]}{suffix}")
    found: dict[str, str] = {}
    last = -1
    for param in tail[2:]:
        key, eq, value = param.partition("=")
        if not eq or key not in PARAM_ORDER:
            raise UriError("unknown_set_parameter")
        if key in found:
            raise UriError("duplicate_set_parameter")
        if PARAM_ORDER.index(key) < last:
            raise UriError("noncanonical_parameter_order")
        last = PARAM_ORDER.index(key)
        if value == "":
            raise UriError("empty_parameter_value")
        found[key] = value

    def number(key: str) -> int | None:
        if key not in found:
            return None
        value = g._parse_u64(found[key])
        if key in ("depth", "count", "bytes") and value == 0:
            raise UriError("invalid_bound")
        return value

    verbs = None
    if "verbs" in found:
        items = found["verbs"].split(",")
        if any(v == "" for v in items):
            raise UriError("empty_parameter_value")
        for verb in items:
            if not _VERB.fullmatch(verb):
                raise UriError("invalid_verb_token")
            if verb in NOT_FOLLOWABLE:
                raise UriError("verb_not_followable")
        verbs = tuple(items)
    direction = found.get("dir")
    if direction == "both":
        raise UriError("noncanonical_default")
    if direction is not None and direction not in ("in", "out"):
        raise UriError("invalid_direction")
    authors = None
    if "authors" in found:
        keys = []
        for item in found["authors"].split(","):
            match = _KEY.fullmatch(item)
            if not match:
                raise UriError("invalid_author_key")
            keys.append(bytes.fromhex(match.group(1)))
        authors = tuple(keys)
    ref = SetRef(
        base.resolver_hint,
        base.namespace,
        tail[0],
        base.digest,
        verbs=verbs,
        direction=direction,
        depth=number("depth"),
        within=g._cid_parse(found["within"]) if "within" in found else None,
        authors=authors,
        since=number("since"),
        until=number("until"),
        count=number("count"),
        max_bytes=number("bytes"),
        via=base.via,
        fragment=base.fragment,
    )
    # Round trip: only the canonical spelling is accepted.
    if format_set(ref) != uri:
        raise UriError("noncanonical_spelling")
    return ref


def identity(uri: str) -> str:
    ref = parse(uri)
    if isinstance(ref, SetRef):
        return format_set(replace(ref, via=(), fragment=None))
    return g.identity(uri)


# ----------------------------------------------------------------- resolver


class ResolveError(Exception):
    """A set the resolver refuses for this reader, with a stable code."""


def unix(stamp: str) -> int:
    if not stamp.endswith("Z"):
        raise ValueError(stamp)
    return int(datetime.datetime.fromisoformat(stamp[:-1] + "+00:00").timestamp())


def resolve(d5, view, ref: SetRef, by_cid: dict, author_of: dict, size_of: dict):
    """Members of a set for one reader (view), in the canonical order.

    by_cid maps root/within digests to object ids, author_of maps key bytes to
    the fixture's author names and size_of gives each object's encoded size.
    This walk is written from the slice 2 text, not copied from D5's
    build_frame; the self-test compares the two.
    """
    graph = view.graph
    root_id = by_cid.get(ref.root)
    if root_id is None or root_id not in view.node_of:
        raise ResolveError("root_unavailable")  # unreadable and absent answer alike
    root = view.node_of[root_id]
    if ref.root_shape == "head" and root != root_id:
        raise ResolveError("not_a_chain_root")

    inside = None
    if ref.within is not None:
        within_id = by_cid.get(ref.within)
        if within_id is None or within_id not in view.node_of:
            raise ResolveError("within_unavailable")
        inside = {view.node_of[within_id]}
        grew = True
        while grew:
            grew = False
            for edge in view.edges:
                if (
                    edge["verb"] == "nest"
                    and edge["target_node"] in inside
                    and edge["source_node"] not in inside
                ):
                    inside.add(edge["source_node"])
                    grew = True
        if root not in inside:
            raise ResolveError("root_outside_within")

    verbs = set(ref.verbs if ref.verbs is not None else REGISTRY_FOLLOWABLE)
    unknown = sorted(verb for verb in verbs if verb not in d5.DRAWN_VERBS)
    authors = None
    if ref.authors is not None:
        authors = {author_of[key] for key in ref.authors if key in author_of}

    def shown(node):
        return graph.objects[view.shown(node)]

    def keep(node):
        if node == root:
            return True
        if inside is not None and node not in inside:
            return False
        obj = shown(node)
        if authors is not None and obj["author"] not in authors:
            return False
        claim = unix(obj["created_claim"])
        if ref.since is not None and claim < ref.since:
            return False
        return ref.until is None or claim < ref.until

    links = [
        edge
        for edge in view.edges
        if edge.get("of_verb", edge["verb"]) in verbs
        and keep(edge["source_node"])
        and keep(edge["target_node"])
    ]
    steps: dict[str, list] = {}
    for edge in links:
        if ref.direction != "in":
            steps.setdefault(edge["source_node"], []).append(
                (edge, edge["target_node"], 1)
            )
        if ref.direction != "out":
            steps.setdefault(edge["target_node"], []).append(
                (edge, edge["source_node"], 0)
            )
    for items in steps.values():
        items.sort(
            key=lambda item: (
                d5.VERB_RANK[item[0]["verb"]],
                item[2],  # inbound before outbound
                unix(shown(item[1])["created_claim"]),
                item[1],
                item[0]["object_id"],
            )
        )

    hop = {root: 0}
    order = [root]
    used = size_of[view.shown(root)]
    cuts = set()
    if ref.max_bytes is not None and used > ref.max_bytes:
        order, cuts = [], {"bytes"}
    head = 0
    stopped = not order
    while head < len(order) and not stopped:
        node = order[head]
        head += 1
        for edge, other, _ in steps.get(node, []):
            if other in hop:
                continue
            if ref.depth is not None and hop[node] == ref.depth:
                cuts.add("depth")
                continue
            if ref.count is not None and len(order) == ref.count:
                cuts.add("count")
                continue
            weight = size_of[view.shown(other)]
            if ref.max_bytes is not None and used + weight > ref.max_bytes:
                cuts.add("bytes")
                stopped = True
                break
            hop[other] = hop[node] + 1
            order.append(other)
            used += weight
    members = set(order)
    return {
        "members": order,
        "hops": {node: hop[node] for node in order},
        "links": sorted(
            edge["object_id"]
            for edge in links
            if edge["source_node"] in members and edge["target_node"] in members
        ),
        "cuts": sorted(cuts),
        "claims_untrusted": ref.since is not None or ref.until is not None,
        "unknown_verbs": unknown,
        "bytes": used if order else 0,
    }


# ------------------------------------------------------------ acceptance


def canonical(obj: dict) -> bytes:
    """Stand-in for D3's canonical encoding: sorted-key compact JSON."""
    return json.dumps(
        obj, sort_keys=True, separators=(",", ":"), ensure_ascii=False
    ).encode()


class Fixture:
    """D5's graph with real BLAKE3 ids and Ed25519 keys for its objects."""

    def __init__(self, d4, d5):
        raw = json.loads((D5_FIXTURES / "graph.json").read_text())
        self.d4, self.d5 = d4, d5
        self.graph = d5.Graph(raw)
        self.raw = {o["object_id"]: o for o in raw["objects"] + raw["links"]}
        self.bytes_of = {oid: canonical(obj) for oid, obj in self.raw.items()}
        self.cid = {oid: d4.blake3_256(b) for oid, b in self.bytes_of.items()}
        self.by_cid = {c: oid for oid, c in self.cid.items()}
        self.size_of = {oid: len(b) for oid, b in self.bytes_of.items()}
        names = sorted({o["author"] for o in self.raw.values()})
        self.key = {
            name: d4.ed25519_public(
                d4.blake3_256(b"stand-in author key:" + name.encode())
            )
            for name in names
        }
        self.author_of = {k: name for name, k in self.key.items()}

    def from_d5(self, query: dict) -> SetRef:
        q = self.d5.normalise(query)
        verbs = None
        if set(q["verbs"]) != set(REGISTRY_FOLLOWABLE):
            verbs = tuple(sorted(q["verbs"]))
        return SetRef(
            "any",
            "graph",
            "obj",
            self.cid[q["focus"]],
            verbs=verbs,
            depth=q["radius"],
            within=None if q["within"] is None else self.cid[q["within"]],
            authors=None
            if q["authors"] is None
            else tuple(sorted(self.key[a] for a in q["authors"])),
            since=None if q["since"] is None else unix(q["since"]),
            until=None if q["until"] is None else unix(q["until"]),
            count=q["budget"],
        )

    def resolve(self, viewer: str, ref: SetRef):
        view = self.d5.View(self.graph, viewer)
        return resolve(self.d5, view, ref, self.by_cid, self.author_of, self.size_of)


D5_FIXTURES = DOC / "2026-11-25T02-40-00Z_design_id-apps_d5_link-display" / "fixtures"
D4_DIR = DOC / "2026-11-25T01-29-01Z_design_id-apps_d4_sneakernet-bundles"

BUNDLE_ORDER = ["e-13880", "d-3", "r-1", "d-4", "d-2", "d-5", "d-1", "c-d5-triage"]


def fail(message: str) -> int:
    print(f"FAIL {message}", file=sys.stderr)
    return 1


def self_test() -> int:
    d1 = bytes(range(32))
    d2 = bytes(range(32, 64))
    k1, k2 = bytes([7]) * 32, bytes([3]) * 32
    cid1 = g._cid_text(d1)
    golden = {
        "everything": (
            SetRef("any", "graph", "obj", d1),
            f"id://any/graph/v2/set/obj/{cid1}",
        ),
        "thread_for_bundle": (
            SetRef("any", "graph", "head", d1, ("nest", "reply"), "in", count=4096),
            f"id://any/graph/v2/set/head/{cid1}/verbs=nest,reply/dir=in/count=4096",
        ),
        "collection": (
            SetRef("any", "graph", "obj", d1, ("nest",), "in", depth=1),
            f"id://any/graph/v2/set/obj/{cid1}/verbs=nest/dir=in/depth=1",
        ),
        "every_parameter": (
            SetRef(
                "home.lan",
                "photos 2026",
                "obj",
                d1,
                ("mention", "reply"),
                "out",
                2,
                d2,
                (k2, k1),
                1795996800,
                1796601600,
                200,
                65536,
                ("pkarr:yb4ndy",),
                "n-3",
            ),
            (
                f"id://home.lan/photos%202026/v2/set/obj/{cid1}/verbs=mention,reply"
                f"/dir=out/depth=2/within={g._cid_text(d2)}"
                f"/authors=ed25519.{k2.hex()},ed25519.{k1.hex()}"
                "/since=1795996800/until=1796601600/count=200/bytes=65536"
                "?via=pkarr:yb4ndy#n-3"
            ),
        ),
    }
    for name, (ref, text) in golden.items():
        if format_set(ref) != text:
            return fail(f"golden {name}: {format_set(ref)}")
        if parse(text) != ref:
            return fail(f"golden parse {name}")
    bare = golden["every_parameter"][1].split("?")[0]
    if identity(golden["every_parameter"][1]) != bare:
        return fail("identity strips hints and fragment")
    slice1 = f"id://any/graph/v2/obj/{cid1}"
    if parse(slice1) != g.ObjRef("any", "graph", d1):
        return fail("slice 1 shapes still parse")

    s = f"id://any/graph/v2/set/obj/{cid1}"
    rejects = {
        s + "/dir=both": "noncanonical_default",
        s + "/dir=up": "invalid_direction",
        s + "/verbs=reply,nest": "noncanonical_list_order",
        s + "/verbs=nest,nest": "noncanonical_list_order",
        s + "/verbs=supersede": "verb_not_followable",
        s + "/verbs=Reply": "invalid_verb_token",
        s + "/verbs=": "empty_parameter_value",
        s + "/verbs=nest,": "empty_parameter_value",
        s + "/count=9/dir=in": "noncanonical_parameter_order",
        s + "/depth=1/depth=2": "duplicate_set_parameter",
        s + "/colour=red": "unknown_set_parameter",
        s + f"/at/{cid1}": "unknown_set_parameter",
        s + "/depth=0": "invalid_bound",
        s + "/count=0": "invalid_bound",
        s + "/depth=02": "noncanonical_decimal",
        s + "/since=10/until=10": "invalid_time_window",
        s + "/authors=ed25519." + "AB" * 32: "invalid_author_key",
        s
        + "/authors=ed25519."
        + "02" * 32
        + ",ed25519."
        + "01" * 32: "noncanonical_list_order",
        s + "/within=raw-blake3-256." + "AB" * 32: "invalid_digest_text",
        s + "/": "empty_or_missing_path_component",
        s + "?cap=x": "undeclared_query_parameter",
        "id://any/graph/v2/set": "empty_or_missing_path_component",
        f"id://any/graph/v2/set/bytes/{cid1}": "invalid_set_root",
        f"id://any/main/v2/set/obj/{cid1}": "portable_hint_requires_graph_namespace",
        f"id://any/graph/v2/set/obj/{cid1}/depth=1 ": "raw_control_space_or_nonascii",
    }
    for text, code in rejects.items():
        try:
            parse(text)
        except UriError as exc:
            if str(exc) != code:
                return fail(f"reject {text!r}: {exc} != {code}")
        else:
            return fail(f"accepted {text!r}")

    # Round trips over random sets, pinned as a corpus digest.
    rng = random.Random(13886_2)
    corpus = hashlib.sha256()
    pool = ["reply", "quote", "embed", "mention", "attach", "nest", "collect-v2"]
    for _ in range(512):
        alias = rng.choice(["any", "local", "home.lan"])
        ns = "graph" if alias == "any" else rng.choice(["graph", "main", "photos 2026"])
        since = rng.choice([None, rng.randrange(1 << 31)])
        until = rng.choice([None, (since or 0) + 1 + rng.randrange(1 << 20)])
        ref = SetRef(
            alias,
            ns,
            rng.choice(["obj", "head"]),
            rng.randbytes(32),
            rng.choice([None, tuple(sorted(rng.sample(pool, rng.randrange(1, 4))))]),
            rng.choice([None, "in", "out"]),
            rng.choice([None, 1 + rng.randrange(6)]),
            rng.choice([None, rng.randbytes(32)]),
            rng.choice(
                [
                    None,
                    tuple(
                        sorted({rng.randbytes(32) for _ in range(1 + rng.randrange(3))})
                    ),
                ]
            ),
            since,
            until,
            rng.choice([None, 1 + rng.randrange(4096)]),
            rng.choice([None, 1 + rng.randrange(1 << 24)]),
            tuple(
                rng.choice(["pkarr:z", "bundle:b 1"]) for _ in range(rng.randrange(3))
            ),
            rng.choice([None, "f"]),
        )
        text = format_set(ref)
        if parse(text) != ref:
            return fail(f"round trip {text}")
        corpus.update(text.encode() + b"\n")
    digest = corpus.hexdigest()
    if CORPUS_SHA256 != "PIN_ME" and digest != CORPUS_SHA256:
        return fail(f"corpus {digest}")

    # Acceptance 1: every D5 case, written as a set URI, resolves to D5's frame.
    d5 = _load("view_reference", D5_FIXTURES / "view_reference.py")
    d4 = _load("bundle_reference", D4_DIR / "bundle_reference.py")
    fx = Fixture(d4, d5)
    longest = 0
    for name, case in d5.CASES.items():
        frame = d5.build_frame(fx.graph, case["viewer"], case["query"])
        ref = fx.from_d5(case["query"])
        text = format_set(ref)
        longest = max(longest, len(text))
        if parse(text) != ref:
            return fail(f"d5 {name}: uri does not round trip")
        got = fx.resolve(case["viewer"], parse(text))
        want_hops = {n["id"]: n["hop"] for n in frame["nodes"]}
        want_cuts = sorted(
            {"budget": "count", "radius": "depth"}[k]
            for k, v in frame["cuts"].items()
            if v
        )
        if got["hops"] != want_hops:
            return fail(f"d5 {name}: members {got['hops']} != {want_hops}")
        if got["links"] != sorted(e["id"] for e in frame["edges"]):
            return fail(f"d5 {name}: links differ")
        if got["cuts"] != want_cuts:
            return fail(f"d5 {name}: cuts {got['cuts']} != {want_cuts}")
        if got["claims_untrusted"] != frame["claims_untrusted"]:
            return fail(f"d5 {name}: claims_untrusted differs")

    # Count and bytes take a prefix of the unbounded canonical order.
    whole = fx.resolve("ana", SetRef("any", "graph", "obj", fx.cid["n-spaced"]))
    if whole["cuts"]:
        return fail("unbounded set reported a cut")
    order = whole["members"]
    shown = [fx.d5.View(fx.graph, "ana").shown(n) for n in order]
    for k in range(1, len(order) + 1):
        by_count = fx.resolve(
            "ana", SetRef("any", "graph", "obj", fx.cid["n-spaced"], count=k)
        )
        budget = sum(fx.size_of[o] for o in shown[:k])
        by_bytes = fx.resolve(
            "ana", SetRef("any", "graph", "obj", fx.cid["n-spaced"], max_bytes=budget)
        )
        if by_count["members"] != order[:k] or by_bytes["members"] != order[:k]:
            return fail(f"prefix property at {k}")
        if (k < len(order)) != ("count" in by_count["cuts"]):
            return fail(f"count cut flag at {k}")
        if (k < len(order)) != ("bytes" in by_bytes["cuts"]):
            return fail(f"bytes cut flag at {k}")
    # Direction narrows: out-only from n-spaced follows what it points at.
    out = fx.resolve(
        "ana",
        SetRef("any", "graph", "obj", fx.cid["n-spaced"], direction="out", depth=1),
    )
    if out["members"] != ["n-spaced", "n-leitner", "n-forget"]:
        return fail(f"dir=out {out['members']}")
    inbound = fx.resolve(
        "ana",
        SetRef("any", "graph", "obj", fx.cid["n-spaced"], direction="in", depth=1),
    )
    if inbound["members"] != ["n-spaced", "n-reply-1", "n-forget", "n-index"]:
        return fail(f"dir=in {inbound['members']}")
    # An unreadable root and an absent one answer alike.
    for viewer, oid in (("ana", "n-private"), ("ana", "n-missing")):
        cid = fx.cid.get(oid, bytes(32))
        try:
            fx.resolve(viewer, SetRef("any", "graph", "obj", cid))
        except ResolveError as exc:
            if str(exc) != "root_unavailable":
                return fail(f"root {oid}: {exc}")
        else:
            return fail(f"root {oid} resolved")
    # A verb this resolver does not know matches nothing and is reported.
    odd = fx.resolve(
        "ana", SetRef("any", "graph", "obj", fx.cid["n-leitner"], ("cites-v9",))
    )
    if odd["members"] != ["n-leitner"] or odd["unknown_verbs"] != ["cites-v9"]:
        return fail("unknown verb")

    # Acceptance 2: a D4 bundle whose scope is a set URI.
    scope = SetRef(
        "any", "graph", "head", fx.cid["e-13880"], ("nest", "reply"), "in", count=4096
    )
    view = fx.d5.View(fx.graph, "anon")
    got = resolve(fx.d5, view, scope, fx.by_cid, fx.author_of, fx.size_of)
    if got["members"] != BUNDLE_ORDER or got["cuts"]:
        return fail(f"bundle members {got['members']}")
    carried = set(got["links"])
    for node in got["members"]:
        carried.update(view.chains[node])
    carried.update(
        link["object_id"]
        for link in fx.graph.links
        if link["verb"] == "supersede"
        and link["source"] in carried
        and link["target"] in carried
    )
    objects = {fx.cid[o]: fx.bytes_of[o] for o in sorted(carried)}
    manifest = d4.Manifest(
        creator_identity="idp:example/ben",
        signing_key=d4.ed25519_public(d4.EXAMPLE_SEED),
        created_at=1796083200,  # 2026-11-25T00:00:00Z
        entries=[
            d4.Entry(c, len(b), 2 if fx.graph.objects[fx.by_cid[c]]["is_link"] else 1)
            for c, b in objects.items()
        ],
        scope=format_set(scope),
    )
    blob = d4.write_bundle(manifest, d4.EXAMPLE_SEED, objects)
    report = d4.read_a(blob)
    if not {"d-1-v2", "l-d1-v2"} <= carried:
        return fail("bundle drops the d-1 chain")
    if len(report.objects) != len(carried) or len(carried) > 4096:
        return fail("bundle object count")
    body_len = int.from_bytes(blob[10:14], "big")
    if parse(d4.parse_manifest(blob[14 : 14 + body_len]).scope) != scope:
        return fail("bundle scope does not parse back")

    print(
        f"ok: {len(golden)} golden, {len(rejects)} rejects, 512 round trips, corpus sha256 "
        f"{digest}; {len(d5.CASES)} D5 cases match build_frame (longest URI {longest} chars); "
        f"prefix property over {len(order)} members; bundle scope {len(format_set(scope))} chars, "
        f"{len(got['members'])} members, {len(carried)} objects, read back"
    )
    return 0


def main(argv: list[str]) -> int:
    if len(argv) != 2 or argv[1] != "self-test":
        print("usage: set_uri_reference.py self-test", file=sys.stderr)
        return 2
    return self_test()


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
