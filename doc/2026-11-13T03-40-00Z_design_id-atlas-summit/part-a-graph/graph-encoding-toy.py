#!/usr/bin/env python3
"""Deterministic, stdlib-only toy for graph encoding size and round-trip.

Contract: no positional arguments; optional --check-oracles. Parse the entire
command line before constructing the corpus, opening SQLite, or producing
output. This is not a production storage benchmark.
"""

import argparse
import hashlib
import json
import sqlite3
import struct
import sys
import zlib


def parse_args(argv):
    parser = argparse.ArgumentParser(allow_abbrev=False)
    parser.add_argument("--check-oracles", action="store_true")
    return parser.parse_args(argv)


def corpus_rows():
    rows = []
    for i in range(1000):
        subject = f"urn:user:{i % 100}".encode()
        predicate = (b"name", b"follows", b"comment")[i % 3]
        if predicate == b"name":
            object_value = f"User {i % 100}".encode()
        elif predicate == b"follows":
            object_value = f"urn:user:{(i * 17 + 3) % 100}".encode()
        else:
            object_value = f"b3:{(i * 2654435761) % (1 << 32):08x}".encode()
        rows.append((subject, predicate, object_value))
    return rows


def lp(value):
    return struct.pack(">I", len(value)) + value


def encode_framed(rows):
    return b"".join(b"\x01" + b"".join(lp(part) for part in row) for row in rows)


def decode_framed(blob):
    rows = []
    pos = 0
    while pos < len(blob):
        if blob[pos] != 1:
            raise ValueError("unknown toy record tag")
        pos += 1
        row = []
        for _ in range(3):
            if pos + 4 > len(blob):
                raise ValueError("truncated length")
            size = struct.unpack(">I", blob[pos:pos + 4])[0]
            pos += 4
            if pos + size > len(blob):
                raise ValueError("truncated field")
            row.append(blob[pos:pos + size])
            pos += size
        rows.append(tuple(row))
    return rows


def encode_map(rows):
    result = []
    for row in sorted(rows):
        key = b"".join(b"\x01" + lp(part) for part in row)
        result.append(lp(key) + lp(b""))
    return b"".join(result)


def decode_map(blob):
    rows = []
    pos = 0
    while pos < len(blob):
        key_size = struct.unpack(">I", blob[pos:pos + 4])[0]
        pos += 4
        key = blob[pos:pos + key_size]
        pos += key_size
        value_size = struct.unpack(">I", blob[pos:pos + 4])[0]
        pos += 4
        value = blob[pos:pos + value_size]
        pos += value_size
        if value:
            raise ValueError("toy map value must be empty")
        key_pos = 0
        row = []
        for _ in range(3):
            if key[key_pos] != 1:
                raise ValueError("unknown toy term tag")
            key_pos += 1
            size = struct.unpack(">I", key[key_pos:key_pos + 4])[0]
            key_pos += 4
            row.append(key[key_pos:key_pos + size])
            key_pos += size
        if key_pos != len(key):
            raise ValueError("trailing composite-key bytes")
        rows.append(tuple(row))
    return rows


def sqlite_jsonb(rows):
    # Type-tag bytes as JSON arrays of octets; this keeps arbitrary bytes typed
    # rather than making UTF-8 decoding an unstated precondition.
    document = [[[1, list(part)] for part in row] for row in rows]
    source = json.dumps(document, separators=(",", ":"))
    connection = sqlite3.connect(":memory:")
    try:
        blob = connection.execute("SELECT jsonb(?)", (source,)).fetchone()[0]
        decoded = json.loads(connection.execute("SELECT json(?)", (blob,)).fetchone()[0])
    finally:
        connection.close()
    invalid_terms = (
        term[0] != 1
        or any(not isinstance(octet, int) or not 0 <= octet <= 255 for octet in term[1])
        for row in decoded
        for term in row
    )
    if any(invalid_terms):
        raise ValueError("SQLite JSONB changed a type tag or byte value")
    restored = [tuple(bytes(term[1]) for term in row) for row in decoded]
    return blob, restored


def require_equal(actual, expected, label):
    if actual != expected:
        raise AssertionError(f"{label}: decoded output differs from reference corpus")


def check_mutation_is_rejected(rows):
    mutated = list(rows)
    mutated[0] = (b"mutated-subject", *mutated[0][1:])
    try:
        require_equal(mutated, rows, "mutation probe")
    except AssertionError:
        return
    raise AssertionError("mutation probe unexpectedly passed the equality oracle")


def main(argv):
    args = parse_args(argv)
    rows = corpus_rows()
    compact_json = json.dumps(
        [[part.decode("utf-8") for part in row] for row in rows],
        ensure_ascii=False,
        separators=(",", ":"),
    ).encode("utf-8")
    framed = encode_framed(rows)
    compressed = zlib.compress(framed, 6)
    mapped = encode_map(rows)
    jsonb, jsonb_rows = sqlite_jsonb(rows)

    # The separate probe ensures the byte contract is not UTF-8-only.
    arbitrary_rows = [(b"subject\x00\xff", b"pred\x80", b"obj\xfe\x00")]
    arbitrary_jsonb, arbitrary_decoded = sqlite_jsonb(arbitrary_rows)

    # Equality checks run in every mode. Do not use Python `assert`: `-O`
    # removes assert statements and must never turn a mismatch into PASS.
    decoded_json = [tuple(value.encode("utf-8") for value in row) for row in json.loads(compact_json)]
    require_equal(decoded_json, rows, "compact JSON")
    require_equal(decode_framed(framed), rows, "framed binary")
    require_equal(decode_framed(zlib.decompress(compressed)), rows, "compressed binary")
    require_equal(decode_map(mapped), sorted(rows), "normalized byte map")
    require_equal(jsonb_rows, rows, "SQLite JSONB")
    require_equal(arbitrary_decoded, arbitrary_rows, "arbitrary non-UTF-8 SQLite JSONB")
    check_mutation_is_rejected(rows)

    encodings = {
        "compact_json_utf8": compact_json,
        "framed_binary_typed": framed,
        "framed_binary_zlib6": compressed,
        "normalized_byte_map_typed": mapped,
        "sqlite_jsonb_typed_octet_arrays": jsonb,
    }
    print(
        "corpus=1000 triples; deterministic=subject urn:user:0..99, "
        "predicates{name,follows,comment}, fixed formula objects; "
        "oracle=exact tuple equality (map order normalized)"
    )
    for name, data in encodings.items():
        print(f"{name}: bytes={len(data)} sha256={hashlib.sha256(data).hexdigest()}")
    json_size = len(compact_json)
    print(
        "ratios_vs_compact_json="
        + " ".join(
            f"{name}:{len(data) / json_size:.3f}"
            for name, data in encodings.items()
            if name != "compact_json_utf8"
        )
    )
    print(f"sqlite_version={sqlite3.sqlite_version}; sqlite_jsonb_available=yes")
    print("roundtrip=PASS for compact JSON, framed binary, compressed binary, byte map, and SQLite JSONB; mismatch=0")
    print("arbitrary_non_utf8_typed_bytes=PASS (00 ff / 80 / fe 00); probe_bytes=" + str(len(arbitrary_jsonb)))
    print("mutation_rejection=PASS")
    if args.check_oracles:
        print("check_oracles=PASS; explicit equality and mutation-rejection checks ran")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
