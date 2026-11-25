#!/usr/bin/env python3
"""Encode golden/inputs.json to canonical CBOR bytes (hex on stdout).

Implements the subset of the id-core canonical-CBOR profile
(pkgs/id/crates/id-core/src/canonical_cbor.rs) that the composition
hash needs: uint, text, null, definite-length arrays and maps, map keys
sorted by their encoded bytes, shortest-form integers. No floats, no
tags, no indefinite lengths — the encoder refuses anything else, and
the decoder side (id-core) would reject it too.
"""
import json
import sys
from pathlib import Path


def encode_uint(n: int) -> bytes:
    if n < 24:
        return bytes([n])
    if n < 256:
        return bytes([0x18, n])
    if n < 65536:
        return b"\x19" + n.to_bytes(2, "big")
    if n < 2**32:
        return b"\x1a" + n.to_bytes(4, "big")
    if n < 2**64:
        return b"\x1b" + n.to_bytes(8, "big")
    raise ValueError(f"uint out of range: {n}")


def encode_len(major: int, n: int) -> bytes:
    assert major in (0x60, 0x80, 0xA0)
    if n < 24:
        return bytes([major | n])
    if n < 256:
        return bytes([(major | 24), n])
    if n < 65536:
        return bytes([major | 25]) + n.to_bytes(2, "big")
    raise ValueError(f"length out of range: {n}")


def encode(value) -> bytes:
    if value is None:
        return b"\xf6"
    if isinstance(value, bool):
        raise ValueError("booleans are not part of the composition-hash value model")
    if isinstance(value, int):
        if value < 0:
            raise ValueError("negative integers are not part of the composition-hash value model")
        return encode_uint(value)
    if isinstance(value, str):
        raw = value.encode("utf-8")
        return encode_len(0x60, len(raw)) + raw
    if isinstance(value, list):
        return encode_len(0x80, len(value)) + b"".join(encode(v) for v in value)
    if isinstance(value, dict):
        items = [(encode(k), encode(v)) for k, v in value.items()]
        for k, _ in items:
            pass
        items.sort(key=lambda kv: kv[0])
        for i in range(1, len(items)):
            if items[i][0] == items[i - 1][0]:
                raise ValueError("duplicate map key")
        return encode_len(0xA0, len(items)) + b"".join(k + v for k, v in items)
    raise ValueError(f"unsupported value: {value!r}")


def canonical_structure(doc: dict) -> dict:
    """Project inputs.json onto exactly the hashed structure, in any order —
    the encoder sorts map keys, so field order here does not matter."""
    features = [
        {"crates": sorted(f["crates"]), "name": f["name"]}
        for f in sorted(doc["features"], key=lambda f: f["name"])
    ]
    return {
        "catalogue": doc["catalogue"],
        "features": features,
        "lockfile": doc["lockfile"],
        "profile": doc["profile"],
        "schema": doc["schema"],
        "target": doc["target"],
        "toolchain": doc["toolchain"],
    }


def main() -> None:
    here = Path(__file__).resolve().parent
    doc = json.loads((here / "inputs.json").read_text())
    print(encode(canonical_structure(doc)).hex())


if __name__ == "__main__":
    main()
