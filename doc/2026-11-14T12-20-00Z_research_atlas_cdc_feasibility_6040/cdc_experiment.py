#!/usr/bin/env python3
# lane-scratch: code-d /tmp/lane-code-d-20261114-0546 task 15258
"""FastCDC feasibility at embedding scale (issue #6040).

Compares three chunkers over a real corpus slice (this repo's doc/ +
planning/ Markdown at HEAD), at an average chunk target of ~2048 bytes
(~512 tokens at ~4 chars/token, the embed-layer scale named on #6040):

  1. fastcdc  -- Gear-hash FastCDC with normalized chunking
                 (min 1024, avg 2048, max 8192 bytes).
  2. fixed    -- fixed-size 2048-byte blocks (naive baseline).
  3. span     -- span-anchored packing: split on blank-line (paragraph)
                 boundaries, greedily pack whole spans up to ~2048 B
                 without splitting a span unless it exceeds 8192 B
                 (approximation of the #6028 span approach).

For each chunker and each simulated revision class, measures:
  - dedup_ratio_v1 : 1 - unique-chunk-bytes / total-bytes within v1
                     (what CDC is supposed to buy at this scale);
  - reuse_v2       : fraction of v2 bytes covered by chunks whose hash
                     already exists in v1 (bytes that need no re-embed
                     under delta-only ingestion, the #6028 consumer);
  - survival       : fraction of v1 chunks (by count) whose hash
                     appears unchanged in v2 (boundary stability).

Revision classes (deterministic, seeded):
  (a) insert15 : a 15-byte insertion at 5 spread positions per document
                 (the structural-shift fragility case from prior art);
  (b) rewrite  : a ~200-byte region replaced at 3 spread positions.

Stdlib only. Oracle: an unchanged document must give reuse == survival
== 1.0 for every chunker (asserted in self_check()).
"""
import hashlib, json, random, sys
from pathlib import Path

MASK64 = (1 << 64) - 1
_rng = random.Random(0xCDC6040)
GEAR = [_rng.getrandbits(64) for _ in range(256)]

MIN_C, AVG_C, MAX_C = 1024, 2048, 8192
# normalized chunking masks: stricter before the midpoint, looser after
BITS = AVG_C.bit_length() - 1  # 11 for 2048
MASK_STRICT = (1 << BITS) - 1
MASK_LOOSE = (1 << (BITS - 2)) - 1


def fastcdc(data: bytes):
    chunks, i, n = [], 0, len(data)
    while i < n:
        remaining = n - i
        if remaining <= MIN_C:
            chunks.append(data[i:]); break
        end_limit = min(remaining, MAX_C)
        fp, cut = 0, end_limit
        for j in range(MIN_C, end_limit):
            fp = (((fp << 1) & MASK64) + GEAR[data[i + j]]) & MASK64
            mask = MASK_STRICT if j < AVG_C else MASK_LOOSE
            if fp & mask == 0:
                cut = j + 1
                break
        chunks.append(data[i:i + cut])
        i += cut
    return chunks


def fixed(data: bytes, size=AVG_C):
    return [data[i:i + size] for i in range(0, len(data), size)]


def span(data: bytes, target=AVG_C, max_c=MAX_C):
    text = data.decode("utf-8", errors="ignore").encode()
    parts = text.split(b"\n\n")
    spans = [p + b"\n\n" for p in parts]
    chunks, cur = [], b""
    for s in spans:
        if len(s) > max_c:
            if cur:
                chunks.append(cur); cur = b""
            chunks.extend(fixed(s, target)); continue
        if len(cur) + len(s) > target and cur:
            chunks.append(cur); cur = b""
        cur += s
    if cur:
        chunks.append(cur)
    return chunks


CHUNKERS = {"fastcdc": fastcdc, "fixed": fixed, "span": span}


def h(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


def metrics(v1_chunks, v2_chunks):
    v1_hashes = {h(c) for c in v1_chunks}
    total1 = sum(len(c) for c in v1_chunks)
    uniq1 = sum(len(c) for c in v1_chunks if True)
    seen, uniq_bytes = set(), 0
    for c in v1_chunks:
        if h(c) not in seen:
            seen.add(h(c)); uniq_bytes += len(c)
    total2 = sum(len(c) for c in v2_chunks)
    reused = sum(len(c) for c in v2_chunks if h(c) in v1_hashes)
    survived = sum(1 for c in v1_chunks if h(c) in {h(x) for x in v2_chunks})
    return {
        "chunks_v1": len(v1_chunks),
        "mean_chunk_bytes": round(total1 / max(1, len(v1_chunks))),
        "dedup_ratio_v1": round(1 - uniq_bytes / total1, 4) if total1 else 0.0,
        "reuse_v2": round(reused / total2, 4) if total2 else 0.0,
        "survival": round(survived / max(1, len(v1_chunks)), 4),
    }


def edit_insert15(data: bytes) -> bytes:
    out, step = data, max(1, len(data) // 5)
    for k in range(5):
        pos = min(len(out), step * k + 997)
        out = out[:pos] + b" [rev: see also] "[:15] + out[pos:]
    return out


def edit_rewrite(data: bytes) -> bytes:
    out = bytearray(data)
    step = max(1, len(out) // 3)
    for k in range(3):
        pos = step * k + 499
        if pos + 200 <= len(out):
            out[pos:pos + 200] = (b"Revised guidance supersedes the earlier text. " * 5)[:200]
    return bytes(out)


def self_check():
    doc = (b"# Title\n\n" + b"Some paragraph text about ingestion. " * 200)
    for name, fn in CHUNKERS.items():
        m = metrics(fn(doc), fn(doc))
        assert m["reuse_v2"] == 1.0 and m["survival"] == 1.0, (name, m)
    print("self-check OK: unchanged document => reuse == survival == 1.0 for all chunkers")


def main(root: str):
    self_check()
    files = sorted(p for sub in ("doc", "planning") for p in (Path(root) / sub).rglob("*.md") if p.stat().st_size >= 4096)
    files = [p for p in files if "queue-15258" not in str(p)]
    docs = [p.read_bytes() for p in files]
    total = sum(len(d) for d in docs)
    print(f"corpus: {len(docs)} markdown files >=4KiB under doc/ + planning/, {total} bytes")
    results = {}
    for name, fn in CHUNKERS.items():
        results[name] = {}
        for edit_name, edit in [("insert15", edit_insert15), ("rewrite", edit_rewrite)]:
            agg = {"chunks_v1": 0, "dedup_ratio_v1": [], "reuse_v2": [], "survival": [],
                   "_tot1": 0, "_tot2": 0, "_uniq": 0, "_reused": 0, "_surv": 0, "_cnt": 0}
            for d in docs:
                c1, c2 = fn(d), fn(edit(d))
                m = metrics(c1, c2)
                agg["chunks_v1"] += m["chunks_v1"]; agg["_cnt"] += 1
                agg["dedup_ratio_v1"].append(m["dedup_ratio_v1"])
                agg["reuse_v2"].append(m["reuse_v2"]); agg["survival"].append(m["survival"])
            avg = lambda xs: round(sum(xs) / max(1, len(xs)), 4)
            results[name][edit_name] = {
                "chunks_v1_total": agg["chunks_v1"],
                "dedup_ratio_v1_mean": avg(agg["dedup_ratio_v1"]),
                "reuse_v2_mean": avg(agg["reuse_v2"]),
                "survival_mean": avg(agg["survival"]),
            }
    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else ".")
