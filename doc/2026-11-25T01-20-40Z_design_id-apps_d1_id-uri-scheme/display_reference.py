#!/usr/bin/env python3
"""Reference display rules for id:// v2 graph URIs (D1 slice 5, #13886).

Stdlib only. Run with:  python3 -I display_reference.py self-test

Three things a person sees, none of which is ever parsed as a name:

  delimit(uri)    the plain-text form, `<id://...>` (RFC 3986 Appendix C)
  extract(text)   the URIs a reader finds in plain text, bracketed or bare
  display(uri)    the short label: shape words, digests elided, hints hidden
  badges(...)     the state words a resolver result adds next to the label

The canonical URI stays the only name. Copy, open, compare and QR always use it.
"""

from __future__ import annotations

import sys

sys.dont_write_bytecode = True

import hashlib
import importlib.util
import random
import re
from pathlib import Path

HERE = Path(__file__).resolve().parent


def _load(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


s = _load("set_uri_reference", HERE / "set_uri_reference.py")
g = s.g
UriError = g.UriError

ELLIPSIS = "…"
HEAD_HEX = 8  # hex digits kept before the ellipsis
TAIL_HEX = 8  # and after it: 64 bits shown in all
# Characters a bare URI in prose is often followed by. A URI that ends in one of
# them cannot be written bare and is always bracketed.
TRAILING = ".,;:!?)]}'\""
_BARE = re.compile(r"id://\S+")
_BRACKETED = re.compile(r"<\s*(id://[^<>]*?)\s*>", re.DOTALL)


def delimit(uri: str) -> str:
    """The only plain-text form an emitter writes."""
    s.parse(uri)
    return f"<{uri}>"


def bare_safe(uri: str) -> bool:
    """True when the URI also survives unbracketed in prose."""
    return uri[-1] not in TRAILING


def extract(text: str) -> list[str]:
    """Every graph URI in plain text, in order, each one parsed and canonical.

    Bracketed URIs may be broken across lines or indented by a mail client:
    whitespace inside the brackets is removed (RFC 3986 Appendix C). Bare URIs
    end at whitespace and lose trailing prose punctuation. Anything that does
    not parse afterwards is left alone, never repaired.
    """
    found: list[tuple[int, str]] = []
    taken: list[tuple[int, int]] = []
    for match in _BRACKETED.finditer(text):
        candidate = re.sub(r"\s+", "", match.group(1))
        taken.append(match.span())
        try:
            s.parse(candidate)
        except UriError:
            continue
        found.append((match.start(), candidate))
    for match in _BARE.finditer(text):
        if any(a <= match.start() < b for a, b in taken):
            continue
        candidate = match.group(0).rstrip(TRAILING)
        try:
            s.parse(candidate)
        except UriError:
            continue
        found.append((match.start(), candidate))
    return [uri for _, uri in sorted(found)]


def _digest(cid: bytes, profile: str = g._PROFILE) -> str:
    text = cid.hex()
    short = text[:HEAD_HEX] + ELLIPSIS + text[-TAIL_HEX:]
    return short if profile == g._PROFILE else f"{profile}:{short}"


def display(uri: str) -> str:
    """The short label. It contains a space and an ellipsis, so it never parses."""
    ref = s.parse(uri)
    where = ""
    if (ref.resolver_hint, ref.namespace) != (g.PORTABLE_ALIAS, g.PORTABLE_NAMESPACE):
        where = f"{ref.resolver_hint}/{ref.namespace} · "
    if isinstance(ref, g.ObjRef):
        words = ["obj", _digest(ref.digest)]
        if ref.byte_range is not None:
            start, end = ref.byte_range
            words += ["bytes", f"{start}-" + ("" if end is None else str(end))]
    elif isinstance(ref, g.HeadRef):
        words = ["head", _digest(ref.root)]
        if ref.expected_head is not None:
            words += ["at", _digest(ref.expected_head)]
    else:
        words = ["set", ref.root_shape, _digest(ref.root)]
        if ref.verbs is not None:
            words.append("verbs=" + ",".join(ref.verbs))
        if ref.direction is not None:
            words.append("dir=" + ref.direction)
        if ref.depth is not None:
            words.append(f"depth={ref.depth}")
        if ref.within is not None:
            words.append("within=" + _digest(ref.within))
        if ref.authors is not None:
            words.append(f"authors={len(ref.authors)}")
        if ref.since is not None:
            words.append(f"since={ref.since}")
        if ref.until is not None:
            words.append(f"until={ref.until}")
        if ref.count is not None:
            words.append(f"count={ref.count}")
        if ref.max_bytes is not None:
            words.append(f"bytes={ref.max_bytes}")
    label = "id:" + " ".join(words)
    if ref.fragment is not None:
        label += f" #{ref.fragment}"
    return where + label


# Badge words, in the fixed order a label shows them.
BADGE_ORDER = (
    "unavailable",
    "removed",
    "stale",
    "fork",
    "edited",
    "older version",
    "partial",
    "claimed times",
    "unknown verbs",
)


def badges(shape: str, result: dict) -> list[str]:
    """State words for one resolution result.

    `result` is what a resolver reports, never what the URI says:
      available       False when the coalesced "not authorised / not here" answer came back
      versions        objects in the chain the reader can see
      heads           current heads the reader can see (more than one is a fork)
      head_tombstone  the (single) current head is a tombstone
      at_matches      for head/at: whether the pinned head is still the current one
      newer_exists    for obj/: a readable later version of this object exists
      cuts            for set/: the bounds that removed members, resolver caps included
      claims_untrusted, unknown_verbs   for set/, as slice 2 reports them
    """
    if not result.get("available", True):
        return ["unavailable"]  # nothing else may leak through the coalesced answer
    words = set()
    if shape in ("head", "obj"):
        if result.get("head_tombstone"):
            words.add("removed")
        if result.get("heads", 1) > 1:
            words.add("fork")
        if result.get("versions", 1) > 1 and shape == "head":
            words.add("edited")
    if shape == "head" and result.get("at_matches") is False:
        words.add("stale")
    if shape == "obj" and result.get("newer_exists"):
        words.add("older version")
    if shape == "set":
        if result.get("cuts"):
            words.add("partial")
        if result.get("claims_untrusted"):
            words.add("claimed times")
        if result.get("unknown_verbs"):
            words.add("unknown verbs")
    return [w for w in BADGE_ORDER if w in words]


BADGE_CASES = [
    # (name, shape, resolver result, expected words)
    ("plain object", "obj", {}, []),
    (
        "unreadable object",
        "obj",
        {"available": False, "newer_exists": True},
        ["unavailable"],
    ),
    ("object with a newer version", "obj", {"newer_exists": True}, ["older version"]),
    (
        "object that was removed",
        "obj",
        {"head_tombstone": True, "newer_exists": True},
        ["removed", "older version"],
    ),
    ("head of a single object", "head", {"versions": 1}, []),
    ("edited chain", "head", {"versions": 3}, ["edited"]),
    ("forked chain", "head", {"versions": 3, "heads": 2}, ["fork", "edited"]),
    (
        "removed chain",
        "head",
        {"versions": 2, "head_tombstone": True},
        ["removed", "edited"],
    ),
    (
        "pinned head still current",
        "head",
        {"versions": 2, "at_matches": True},
        ["edited"],
    ),
    (
        "pinned head moved on",
        "head",
        {"versions": 3, "at_matches": False},
        ["stale", "edited"],
    ),
    ("unreadable chain", "head", {"available": False, "versions": 3}, ["unavailable"]),
    ("whole set", "set", {"cuts": []}, []),
    ("bounded set", "set", {"cuts": ["count"]}, ["partial"]),
    (
        "time-filtered set",
        "set",
        {"cuts": [], "claims_untrusted": True},
        ["claimed times"],
    ),
    (
        "set naming a new verb",
        "set",
        {"unknown_verbs": ["cites-v9"], "cuts": ["depth"]},
        ["partial", "unknown verbs"],
    ),
]


def fail(message: str) -> int:
    print(f"FAIL {message}", file=sys.stderr)
    return 1


def _corpus(rng: random.Random) -> list[str]:
    uris = []
    for _ in range(256):
        alias = rng.choice(["any", "local", "home.lan"])
        ns = "graph" if alias == "any" else rng.choice(["graph", "photos 2026"])
        frag = rng.choice([None, "n-3", "end.", "x~"])
        via = tuple(
            rng.choice(["pkarr:z", "https:a.example/b."])
            for _ in range(rng.randrange(2))
        )
        pick = rng.randrange(3)
        if pick == 0:
            ref = g.ObjRef(
                alias,
                ns,
                rng.randbytes(32),
                rng.choice([None, (5, None), (5, 90)]),
                via,
                frag,
            )
            uris.append(g.format_uri(ref))
        elif pick == 1:
            ref = g.HeadRef(
                alias,
                ns,
                rng.randbytes(32),
                rng.choice([None, rng.randbytes(32)]),
                via,
                frag,
            )
            uris.append(g.format_uri(ref))
        else:
            ref = s.SetRef(
                alias,
                ns,
                rng.choice(["obj", "head"]),
                rng.randbytes(32),
                rng.choice([None, ("nest",), ("mention", "reply")]),
                rng.choice([None, "in"]),
                depth=rng.choice([None, 2]),
                authors=rng.choice([None, (rng.randbytes(32),)]),
                count=rng.choice([None, 200]),
                via=via,
                fragment=frag,
            )
            uris.append(s.format_set(ref))
    return uris


def self_test() -> int:
    d1 = bytes(range(32))
    d2 = hashlib.sha256(b"d2").digest()
    cid1 = g._cid_text(d1)
    obj = f"id://any/graph/v2/obj/{cid1}"
    golden = {
        obj: "id:obj 00010203…1c1d1e1f",
        obj + "/bytes/100-200": "id:obj 00010203…1c1d1e1f bytes 100-200",
        f"id://any/graph/v2/head/{cid1}/at/{g._cid_text(d2)}?via=pkarr:yb4ndy": (
            f"id:head 00010203…1c1d1e1f at {d2.hex()[:8]}…{d2.hex()[-8:]}"
        ),
        f"id://home.lan/photos%202026/v2/obj/{cid1}#n-3": (
            "home.lan/photos 2026 · id:obj 00010203…1c1d1e1f #n-3"
        ),
        f"id://any/graph/v2/set/head/{cid1}/verbs=nest,reply/dir=in/count=4096": (
            "id:set head 00010203…1c1d1e1f verbs=nest,reply dir=in count=4096"
        ),
        f"id://any/graph/v2/set/obj/{cid1}/authors=ed25519.{'07' * 32},ed25519.{'08' * 32}": (
            "id:set obj 00010203…1c1d1e1f authors=2"
        ),
    }
    for uri, label in golden.items():
        if display(uri) != label:
            return fail(f"display {uri}: {display(uri)!r}")

    rng = random.Random(13886_5)
    corpus = _corpus(rng)
    unsafe = 0
    seen_labels = set()
    for uri in corpus:
        label = display(uri)
        seen_labels.add(label)
        try:
            s.parse(label)
        except UriError:
            pass
        else:
            return fail(f"display form parses: {label}")
        # Bracketed, in prose, wrapped by a mail client at 40 columns.
        wrapped = "\n    ".join(
            delimit(uri)[i : i + 40] for i in range(0, len(uri) + 2, 40)
        )
        for text in (
            f"See {delimit(uri)}.",
            f"(see {delimit(uri)}), then reply.",
            f"Quoted:\n> {wrapped}\nthanks",
        ):
            if extract(text) != [uri]:
                return fail(f"extract bracketed {text!r} -> {extract(text)}")
        # Bare, followed by prose punctuation.
        bare = extract(f"see {uri}.")
        if bare_safe(uri):
            if bare != [uri]:
                return fail(f"extract bare {uri} -> {bare}")
        else:
            unsafe += 1
            # Only a hint or fragment can end in prose punctuation, so a bare
            # copy may lose that character but never changes what is named.
            if len(bare) > 1 or any(s.identity(u) != s.identity(uri) for u in bare):
                return fail(f"bare form of {uri} changed identity: {bare}")
    if len(seen_labels) != len(set(corpus)):
        return fail("two corpus URIs share a display label")

    # Two URIs in one line keep their order; non-URIs and damaged ones are skipped.
    huge = f"<{obj}/bytes/{'9' * 5000}-> and {obj}"
    if extract(huge) != [obj]:
        return fail("an oversized decimal stopped extraction")
    two = f"{delimit(obj)} and {obj}/bytes/1-2, not id://any/graph/v2/obj/xyz or <id://nope>"
    if extract(two) != [obj, obj + "/bytes/1-2"]:
        return fail(f"two in one line: {extract(two)}")
    damaged = f"<{obj[:-1]}>"
    if extract(damaged):
        return fail("a truncated URI was repaired")

    for name, shape, result, want in BADGE_CASES:
        got = badges(shape, result)
        if got != want:
            return fail(f"badges {name}: {got} != {want}")

    print(
        f"ok: {len(golden)} golden labels, {len(corpus)} URIs delimited, wrapped, "
        f"extracted and labelled ({unsafe} need brackets), {len(BADGE_CASES)} badge cases"
    )
    return 0


def main(argv: list[str]) -> int:
    if len(argv) != 2 or argv[1] != "self-test":
        print("usage: display_reference.py self-test", file=sys.stderr)
        return 2
    return self_test()


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
