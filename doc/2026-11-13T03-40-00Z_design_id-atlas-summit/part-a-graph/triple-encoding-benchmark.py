#!/usr/bin/env python3
"""Local microbenchmark for three typed-triple storage prototypes.

Contract: --rows and --repeats are the complete CLI; parse and validate all
arguments before creating temporary files or opening SQLite. Stdlib only.
This compares toy adapters, not production redb/Turso/SQLite configurations.
"""

import argparse
import bisect
import hashlib
import json
import os
import sqlite3
import statistics
import struct
import tempfile
import time
import zlib

BLOCK_ROWS = 128


def args(argv):
    p = argparse.ArgumentParser(allow_abbrev=False)
    p.add_argument("--rows", type=int, default=10000)
    p.add_argument("--repeats", type=int, default=5)
    a = p.parse_args(argv)
    if not 1 <= a.rows <= 1000000:
        p.error("--rows must be in [1, 1000000]")
    if not 1 <= a.repeats <= 25:
        p.error("--repeats must be in [1, 25]")
    return a


def corpus(n):
    out = []
    for i in range(n):
        s = f"urn:user:{i % 1000:04d}".encode()
        p = (b"name", b"follows", b"comment")[i % 3]
        if p == b"name":
            o = f"User {i % 1000:04d}#{i // 1000:04d}".encode()
        elif p == b"follows":
            o = f"urn:user:{(i * 17 + 3) % 1000:04d}#{i // 1000:04d}".encode()
        else:
            o = b"b3:" + f"{(i * 2654435761) % (1 << 64):016x}".encode()
        out.append((s, p, o))
    return sorted(set(out), key=key)


def lp(b):
    return struct.pack(">I", len(b)) + b


def key(row):
    return b"".join(b"\x01" + lp(x) for x in row)


def decode_key(data):
    pos, parts = 0, []
    while pos < len(data):
        if data[pos] != 1 or pos + 5 > len(data):
            raise ValueError("bad typed key")
        size = struct.unpack_from(">I", data, pos + 1)[0]
        pos += 5
        if pos + size > len(data):
            raise ValueError("truncated typed key")
        parts.append(data[pos:pos + size])
        pos += size
    if len(parts) != 3:
        raise ValueError("wrong key arity")
    return tuple(parts)


def encode_block(rows):
    return b"".join(b"\x01" + b"".join(lp(x) for x in r) for r in rows)


def decode_block(data):
    pos, out = 0, []
    while pos < len(data):
        if data[pos] != 1:
            raise ValueError("bad record tag")
        pos += 1
        row = []
        for _ in range(3):
            if pos + 4 > len(data):
                raise ValueError("truncated length")
            n = struct.unpack_from(">I", data, pos)[0]
            pos += 4
            if pos + n > len(data):
                raise ValueError("truncated term")
            row.append(data[pos:pos + n])
            pos += n
        out.append(tuple(row))
    return out


def jsonb_value(row):
    # Explicit typed octet arrays preserve arbitrary bytes, not just UTF-8.
    return json.dumps([[1, list(term)] for term in row], separators=(",", ":"))


def from_jsonb(conn, blob):
    value = json.loads(conn.execute("SELECT json(?)", (blob,)).fetchone()[0])
    if len(value) != 3 or any(t[0] != 1 for t in value):
        raise ValueError("JSONB changed term shape/type")
    return tuple(bytes(t[1]) for t in value)


def require(actual, expected, label):
    if actual != expected:
        raise AssertionError(f"{label}: mismatch ({len(actual)} vs {len(expected)})")


def median_ms(fn, repeats):
    samples = []
    for _ in range(repeats):
        start = time.perf_counter_ns()
        fn()
        samples.append((time.perf_counter_ns() - start) / 1_000_000)
    return statistics.median(samples)


def main(argv):
    a = args(argv)
    rows = corpus(a.rows)
    target = rows[len(rows) // 2]
    target_key = key(target)
    with tempfile.TemporaryDirectory(prefix="triple-encoding-") as tmp:
        # Binary: sorted independent zlib blocks plus a small in-memory directory.
        stream_path = os.path.join(tmp, "triples.zblocks")
        directory = []
        with open(stream_path, "wb") as f:
            for start in range(0, len(rows), BLOCK_ROWS):
                block = rows[start:start + BLOCK_ROWS]
                raw = encode_block(block)
                packed = zlib.compress(raw, 6)
                offset = f.tell()
                f.write(struct.pack(">II", len(raw), len(packed)))
                f.write(packed)
                directory.append((key(block[0]), key(block[-1]), offset))
        def read_binary_block(i):
            lo, hi, offset = directory[i]
            with open(stream_path, "rb") as f:
                f.seek(offset)
                raw_n, packed_n = struct.unpack(">II", f.read(8))
                raw = zlib.decompress(f.read(packed_n))
            if len(raw) != raw_n:
                raise ValueError("binary block length mismatch")
            return decode_block(raw)
        def binary_scan():
            return [row for i in range(len(directory)) for row in read_binary_block(i)]
        def binary_point():
            starts = [entry[0] for entry in directory]
            i = max(0, bisect.bisect_right(starts, target_key) - 1)
            for row in read_binary_block(i):
                if row == target:
                    return row
            return None
        def binary_partial():
            start = len(rows) // 2
            block_i, in_block = divmod(start, BLOCK_ROWS)
            result = []
            while len(result) < 16 and block_i < len(directory):
                result.extend(read_binary_block(block_i)[in_block:])
                block_i += 1
                in_block = 0
            return result[:16]

        # JSONB: one opaque typed document per row, table rowid as scan position.
        json_path = os.path.join(tmp, "jsonb.sqlite")
        jc = sqlite3.connect(json_path)
        jc.execute("CREATE TABLE docs (doc BLOB NOT NULL)")
        jc.executemany("INSERT INTO docs VALUES (jsonb(?))", ((jsonb_value(r),) for r in rows))
        jc.commit()
        def json_scan():
            return [from_jsonb(jc, b) for (b,) in jc.execute("SELECT doc FROM docs ORDER BY rowid")]
        def json_point():
            for (b,) in jc.execute("SELECT doc FROM docs"):
                row = from_jsonb(jc, b)
                if row == target:
                    return row
            return None
        def json_partial():
            start = len(rows) // 2
            return [from_jsonb(jc, b) for (b,) in jc.execute(
                "SELECT doc FROM docs WHERE rowid > ? ORDER BY rowid LIMIT 16", (start,))]

        # Byte map: canonical typed composite BLOB key, WITHOUT ROWID primary map.
        map_path = os.path.join(tmp, "byte-map.sqlite")
        mc = sqlite3.connect(map_path)
        mc.execute("CREATE TABLE kv (k BLOB PRIMARY KEY, v BLOB NOT NULL) WITHOUT ROWID")
        mc.executemany("INSERT INTO kv VALUES (?, X'')", ((key(r),) for r in rows))
        mc.commit()
        def map_scan():
            return [bytes(k) for (k,) in mc.execute("SELECT k FROM kv ORDER BY k")]
        def map_point():
            return mc.execute("SELECT k FROM kv WHERE k = ?", (target_key,)).fetchone()[0]
        def map_partial():
            cursor = key(rows[len(rows) // 2 - 1]) if len(rows) > 1 else b""
            return [bytes(k) for (k,) in mc.execute(
                "SELECT k FROM kv WHERE k > ? ORDER BY k LIMIT 16", (cursor,))]

        # Decidable correctness oracles run before timed output.
        require(binary_scan(), rows, "binary exact scan")
        require([from_jsonb(jc, b) for (b,) in jc.execute("SELECT doc FROM docs ORDER BY rowid")], rows, "JSONB exact scan")
        require([decode_key(k) for (k,) in mc.execute("SELECT k FROM kv ORDER BY k")], rows, "byte-map exact scan")
        if binary_point() != target or json_point() != target or map_point() != target_key:
            raise AssertionError("point-query oracle failed")
        partial_start = len(rows) // 2
        require(binary_partial(), rows[partial_start:partial_start + 16], "binary partial-read oracle")
        require(json_partial(), rows[partial_start:partial_start + 16], "JSONB partial-read oracle")
        if len(rows) > 1:
            require(map_partial(), [key(r) for r in rows if key(r) > key(rows[len(rows) // 2 - 1])][:16], "map partial-read oracle")
        else:
            require(map_partial(), [key(rows[0])], "map partial-read oracle")

        # Warm once; report medians over repeated in-process operations.
        for f in (binary_scan, json_scan, map_scan, binary_point, json_point, map_point, binary_partial, json_partial, map_partial):
            f()
        timings = {
            "compressed_binary": (binary_scan, binary_point, binary_partial),
            "sqlite_jsonb": (json_scan, json_point, json_partial),
            "byte_key_map": (map_scan, map_point, map_partial),
        }
        sizes = {
            "compressed_binary_file_bytes": os.path.getsize(stream_path),
            "sqlite_jsonb_file_bytes": os.path.getsize(json_path),
            "byte_map_file_bytes": os.path.getsize(map_path),
            "binary_directory_estimated_bytes": sum(len(lo) + len(hi) + 8 for lo, hi, _ in directory),
        }
        print(f"rows={len(rows)} block_rows={BLOCK_ROWS} repeats={a.repeats} sqlite={sqlite3.sqlite_version}")
        print("oracle=exact ordered typed-byte rows; exact point hit; 16-row partial read; timings=warm median ms")
        for name, ops in timings.items():
            print(f"{name}: full_scan_ms={median_ms(ops[0], a.repeats):.3f} point_query_ms={median_ms(ops[1], a.repeats):.3f} partial_16_ms={median_ms(ops[2], a.repeats):.3f}")
        print("sizes=" + json.dumps(sizes, sort_keys=True))
        print("binary_sha256=" + hashlib.sha256(open(stream_path, "rb").read()).hexdigest())
        print("oracles=PASS")
        jc.close()
        mc.close()
    return 0


if __name__ == "__main__":
    import sys
    raise SystemExit(main(sys.argv[1:]))
