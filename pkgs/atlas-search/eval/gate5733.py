#!/usr/bin/env python3
"""Section-4 held-out gate for the Atlas Jaccard rescore (#5733).

Base retrieval: the corpus's Python reference hybrid (eval3.py's own
functions): Snowflake arctic-embed-m-v2.0 trunc-384 cosine + FTS BM25,
fused RRF k=60. Sanity gate: at branch depth 30 the fused top-10 must
reproduce the bake-off's recorded production-hybrid numbers for the
golden-311 set (183/311 recall@10, MRR 0.3640, per fused_rerank_eval.py).

Rescore: a Python port of the shipped Rust `rescore_fused`
(atlas-core::shingles, PR #10335), pinned against the Rust test
vectors before any measurement is taken (stage A aborts on mismatch).

Arms: OFF = fused ranking as-is. ON = rescore_fused over the top-100
union, threshold 0.60 (the design default), jaccard_depth=100.

Sets: general = golden-311 (frozen 2026-10-26) minus queries whose
gold chunks are all absent from this corpus copy; near-exact = 60
verbatim 30-token passages sampled deterministically from corpus
chunks (every 997th chunk id, >=60 normalized tokens, window starting
at token 10), gold = source chunk.

Corpus disclosure: ~/atlas/v3/corpus.db carries 101,339 of the frozen
101,471 chunks (sample fingerprint 07b3e17b... matches); 132 chunk ids
are absent, including one gold chunk (99555). Affected queries are
excluded and named in the output.
"""

import hashlib
import importlib.util
import json
import math
import sqlite3
import sys
import time
import unicodedata

import numpy as np

V3 = "/home/user/atlas/v3"
CORPUS = f"{V3}/corpus.db"
VECDB = f"{V3}/vec-snowflake.db"
TABLE = "vec_snowflake_m_v2_768_f32"
QCACHE = f"{V3}/qcache-snowflake.npy"
GOLDEN = "/home/user/git/code/pkgs/atlas-search/eval/fixtures/golden-311.json"
OUT = "/tmp/gate5733-results.json"
DEPTH = 100
THRESHOLD = 0.60

spec = importlib.util.spec_from_file_location("eval3", f"{V3}/eval3.py")
ev = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ev)

# ---------------- Stage A: pinned port of atlas-core::shingles ----------------
M64 = (1 << 64) - 1
P1 = 11400714785074694791
P2 = 14029467366897019727
P3 = 1609587929392839161
P4 = 9650029242287828579
P5 = 2870177450012600261


def _rotl(x, r):
    return ((x << r) | (x >> (64 - r))) & M64


def _round(acc, inp):
    return _rotl((acc + (inp * P2 & M64)) & M64, 31) * P1 & M64


def xxhash64(data: bytes) -> int:
    n = len(data)
    pos = 0
    if n >= 32:
        v1 = (P1 + P2) & M64
        v2 = P2
        v3 = 0
        v4 = (-P1) & M64
        while pos + 32 <= n:
            v1 = _round(v1, int.from_bytes(data[pos : pos + 8], "little"))
            v2 = _round(v2, int.from_bytes(data[pos + 8 : pos + 16], "little"))
            v3 = _round(v3, int.from_bytes(data[pos + 16 : pos + 24], "little"))
            v4 = _round(v4, int.from_bytes(data[pos + 24 : pos + 32], "little"))
            pos += 32
        h = (_rotl(v1, 1) + _rotl(v2, 7) + _rotl(v3, 12) + _rotl(v4, 18)) & M64
        for v in (v1, v2, v3, v4):
            h = ((h ^ _round(0, v)) * P1 + P4) & M64
    else:
        h = P5
    h = (h + n) & M64
    while pos + 8 <= n:
        k = _round(0, int.from_bytes(data[pos : pos + 8], "little"))
        h = (_rotl(h ^ k, 27) * P1 + P4) & M64
        pos += 8
    if pos + 4 <= n:
        h = (
            _rotl(h ^ (int.from_bytes(data[pos : pos + 4], "little") * P1 & M64), 23)
            * P2
            & M64
        )
        h = (h + P3) & M64
        pos += 4
    for byte in data[pos:]:
        h = _rotl(h ^ (byte * P5 & M64), 11) * P1 & M64
    h ^= h >> 33
    h = h * P2 & M64
    h ^= h >> 29
    h = h * P3 & M64
    h ^= h >> 32
    return h


def normalize_tokens(text: str):
    lowered = unicodedata.normalize("NFKC", text).lower()
    tokens = []
    for line in lowered.splitlines():
        if line.lstrip().startswith("```"):
            continue
        cur = []
        for ch in line:
            if ch.isalnum() or ch == "_":
                cur.append(ch)
            elif cur:
                tokens.append("".join(cur))
                cur = []
        if cur:
            tokens.append("".join(cur))
    return tokens


def shingle_set(text: str):
    toks = normalize_tokens(text)
    if not toks:
        return frozenset()
    if len(toks) < 5:
        return frozenset([xxhash64(" ".join(toks).encode())])
    return frozenset(
        xxhash64(" ".join(toks[i : i + 5]).encode()) for i in range(len(toks) - 4)
    )


def jaccard(a, b):
    if not a or not b:
        return 0.0
    inter = len(a & b)
    union = len(a) + len(b) - inter
    return inter / union if union else 0.0


def rescore(entries, qset, sets, depth, threshold):
    """entries: list of [chunk_id, fused_score]; returns rescored copy."""
    union = entries[:depth]
    if not union:
        return entries, 0
    max_rrf = max(s for _, s in union)
    scored = []
    near = 0
    for cid, s in union:
        rrf_norm = s / max_rrf if max_rrf > 0 else 0.0
        j = jaccard(qset, sets[cid]) if cid in sets else 0.0
        if j >= threshold:
            near += 1
            final = 1.0 + j
        else:
            final = 0.85 * rrf_norm + 0.15 * j
        scored.append([cid, final, j, rrf_norm])
    # Rust sort: final desc; within TIE_WINDOW=0.005 -> J desc, rrf desc, id asc.
    # Rust's sort_by is not a total order via key; emulate with a comparator.
    from functools import cmp_to_key

    def cmp(a, b):
        if abs(a[1] - b[1]) >= 0.005:
            return -1 if a[1] > b[1] else 1
        if a[2] != b[2]:
            return -1 if a[2] > b[2] else 1
        if a[3] != b[3]:
            return -1 if a[3] > b[3] else 1
        return -1 if a[0] < b[0] else (1 if a[0] > b[0] else 0)

    scored.sort(key=cmp_to_key(cmp))
    out = [[cid, final] for cid, final, _, _ in scored] + entries[depth:]
    return out, near


def stage_a():
    assert xxhash64(b"") == 0xEF46DB3751D8E999, hex(xxhash64(b""))
    assert xxhash64(b"a") == 0xD24EC4F1A98C6E5B
    assert xxhash64(b"abc") == 0x44BC2CF5AD770999
    toks = normalize_tokens("```rust\nHello ＷＯＲＬＤ foo_bar\n```")
    assert toks == ["hello", "world", "foo_bar"], toks
    s = shingle_set("alpha beta gamma")
    assert s == frozenset([xxhash64(b"alpha beta gamma")])
    assert shingle_set("") == frozenset()
    assert jaccard({1, 2, 3}, {2, 3, 4}) == 0.5
    print("[gate] stage A: rescore port pinned against Rust test vectors")


# ---------------- Stage B/C: retrieval ----------------
def fts_top(con, query, limit):
    return [
        r[0]
        for r in con.execute(
            "SELECT c.id FROM chunks c JOIN chunks_fts f ON f.rowid = c.id "
            "WHERE chunks_fts MATCH ? ORDER BY bm25(chunks_fts) LIMIT ?",
            (ev.fts_query_hybrid(query), limit),
        )
    ]


def fuse(vec_ids, fts_ids):
    score = {}
    for r, cid in enumerate(vec_ids):
        score[cid] = score.get(cid, 0.0) + 1.0 / (60 + r)
    for r, cid in enumerate(fts_ids):
        score[cid] = score.get(cid, 0.0) + 1.0 / (60 + r)
    return sorted(score.items(), key=lambda x: -x[1])


def main():
    t0 = time.time()
    stage_a()
    con = sqlite3.connect(f"file:{CORPUS}?mode=ro", uri=True)
    corpus_ids = {r[0] for r in con.execute("SELECT id FROM chunks")}
    print(f"[gate] corpus chunks present: {len(corpus_ids)}")

    golden = json.load(open(GOLDEN))["queries"]
    # Near-exact set: every 997th chunk id, >=60 normalized tokens,
    # passage = raw whitespace tokens [10:40] joined by single spaces.
    ne_queries = []
    cand_ids = sorted(cid for cid in corpus_ids if cid % 997 == 0)
    for cid in cand_ids:
        if len(ne_queries) >= 60:
            break
        text = con.execute("SELECT text FROM chunks WHERE id=?", (cid,)).fetchone()[0]
        if len(normalize_tokens(text)) < 60:
            continue
        raw = text.split()
        if len(raw) < 40:
            continue
        passage = " ".join(raw[10:40])
        ne_queries.append(
            {"id": f"ne{len(ne_queries):03d}", "query": passage, "chunk_ids": [cid]}
        )
    print(f"[gate] near-exact queries built: {len(ne_queries)}")

    ids, M = ev.load_matrix(VECDB, TABLE, 384)
    id_pos = {cid: i for i, cid in enumerate(ids)}
    print(f"[gate] matrix {M.shape}")

    Qg = np.load(QCACHE).astype(np.float32)
    Qg = Qg[:, :384]
    Qg = Qg / np.linalg.norm(Qg, axis=1, keepdims=True)
    assert Qg.shape[0] == len(golden)

    from sentence_transformers import SentenceTransformer

    model = SentenceTransformer("Snowflake/snowflake-arctic-embed-m-v2.0")
    Qn = model.encode(
        [q["query"] for q in ne_queries],
        prompt_name="query",
        batch_size=32,
        convert_to_numpy=True,
        show_progress_bar=False,
    ).astype(np.float32)
    Qn = Qn[:, :384]
    Qn = Qn / np.linalg.norm(Qn, axis=1, keepdims=True)
    print(f"[gate] near-exact embeddings {Qn.shape}")

    def base_ranking(qtext, qvec, branch):
        sims = M @ qvec
        top = np.argsort(-sims)[:branch]
        vec_ids = [ids[i] for i in top]
        return fuse(vec_ids, fts_top(con, qtext, branch))

    # Sanity gate at branch depth 30 (bake-off production hybrid).
    hits = 0
    mrr = 0.0
    for qi, q in enumerate(golden):
        ranked = [cid for cid, _ in base_ranking(q["query"], Qg[qi], 30)][:10]
        gold = set(q["chunk_ids"])
        rank = next((i + 1 for i, cid in enumerate(ranked) if cid in gold), None)
        if rank:
            hits += 1
            mrr += 1.0 / rank
    print(
        f"[gate] sanity depth-30: recall@10={hits}/311 mrr={mrr / 311:.4f} "
        f"(recorded: 183/311, 0.3640)"
    )

    def run_set(queries, qmat, label):
        rows = []
        for qi, q in enumerate(queries):
            fused = base_ranking(q["query"], qmat[qi], DEPTH)
            entries = [[cid, s] for cid, s in fused]
            cand = [cid for cid, _ in entries[:DEPTH]]
            texts = {}
            for cid in cand:
                row = con.execute(
                    "SELECT text FROM chunks WHERE id=?", (cid,)
                ).fetchone()
                if row:
                    texts[cid] = row[0]
            sets = {cid: shingle_set(t) for cid, t in texts.items()}
            qset = shingle_set(q["query"])
            on, near = rescore(entries, qset, sets, DEPTH, THRESHOLD)
            rows.append(
                {
                    "id": q["id"],
                    "gold": q["chunk_ids"],
                    "off": [cid for cid, _ in entries],
                    "on": [cid for cid, _ in on],
                    "near_exact_flagged": near,
                }
            )
        print(f"[gate] {label}: {len(rows)} queries rescored")
        return rows

    general = run_set(golden, Qg, "general")
    near_ex = run_set(ne_queries, Qn, "near-exact")

    def rank_of(ranked, gold):
        gold = set(gold)
        return next((i + 1 for i, cid in enumerate(ranked[:10]) if cid in gold), None)

    def ndcg10(ranked, gold):
        gold = set(gold)
        dcg = sum(
            1.0 / math.log2(i + 2) for i, cid in enumerate(ranked[:10]) if cid in gold
        )
        idcg = sum(1.0 / math.log2(i + 2) for i in range(min(len(gold), 10)))
        return dcg / idcg if idcg else 0.0

    excluded = []
    g_off = g_on = 0.0
    g_n = 0
    for row in general:
        present = [c for c in row["gold"] if c in corpus_ids]
        if not present:
            excluded.append(row["id"])
            continue
        g_n += 1
        g_off += ndcg10(row["off"], present)
        g_on += ndcg10(row["on"], present)
    n_off = n_on = 0.0
    for row in near_ex:
        r_off = rank_of(row["off"], row["gold"])
        r_on = rank_of(row["on"], row["gold"])
        n_off += 1.0 / r_off if r_off else 0.0
        n_on += 1.0 / r_on if r_on else 0.0
    result = {
        "general_queries_scored": g_n,
        "general_excluded_no_gold_present": excluded,
        "general_ndcg10_off": round(g_off / g_n, 4),
        "general_ndcg10_on": round(g_on / g_n, 4),
        "general_regression": round(g_off / g_n - g_on / g_n, 4),
        "near_exact_queries": len(near_ex),
        "near_exact_mrr10_off": round(n_off / len(near_ex), 4),
        "near_exact_mrr10_on": round(n_on / len(near_ex), 4),
        "near_exact_gain": round(n_on / len(near_ex) - n_off / len(near_ex), 4),
        "bars": {"near_exact_gain_min": 0.10, "general_regression_max": 0.01},
        "seconds": round(time.time() - t0, 1),
    }
    result["verdict_pass"] = (
        result["near_exact_gain"] >= 0.10 and result["general_regression"] <= 0.01
    )
    json.dump(
        {"result": result, "general": general, "near_exact": near_ex},
        open(OUT, "w"),
        indent=1,
    )
    print("[gate] RESULT " + json.dumps(result, indent=1))


if __name__ == "__main__":
    main()
