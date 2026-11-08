#!/usr/bin/env python3
"""Golden-set v3 evaluator for the Atlas embedding bake-off.

Scoring differs from the v2 harness in ONE deliberate way: a question is a
hit when one of its recorded GOLD CHUNK IDS (golden-queries-v3.json
`chunk_ids`) appears in the top-k ranked chunk ids; the rank used for MRR
is the position of the first gold id. (v2 scored by expected_hint string
presence in chunk text; v3 gold ids are canonical chunk ids, verified at
set construction, so id scoring is exact and index-independent.)

Everything else is a faithful port of the v2 harness:
- Pure FTS: query construction copied from eval-fts-baseline.py (content
  words >3 chars, STOP list removed, OR-joined quoted phrases, BM25).
- Hybrid: copied from eval_vectors.py --hybrid (vector top-30 + FTS
  top-30 fused by reciprocal rank fusion, k=60; the hybrid FTS branch
  uses words >3 chars WITHOUT the STOP filter and LIMIT 30, exactly as
  the v2 harness did).
- bge query prefix: same rule as eval_vectors.py ("bge" in model name ->
  "Represent this sentence for searching relevant passages: ").
- ST models: queries encoded with prompt_name="query" so the model's own
  shipped config_sentence_transformers.json prompt is applied (Snowflake
  arctic-embed-m-v2.0: "query: "; Qwen3-Embedding-0.6B: its Instruct
  template). Documents were embedded with no prompt (shipped default).

Corpus guard: before scoring, chunk count and the v3 fingerprint
(content_hash of chunks with id % 997 = 0, SHA-256 of concatenation)
are recomputed and compared with the frozen values; mismatch = abort.

Usage:
  eval3.py --mode fts   --corpus DB --queries golden-queries-v3.json --out res.json
  eval3.py --mode vector --vecdb VDB --table vec_x --framework st --model M [--trunc 384]
  eval3.py --mode hybrid (same vector args; FTS fused in)
"""
import argparse, hashlib, json, os, re, sqlite3, sys, time
from pathlib import Path
import numpy as np

for _k in ("NO_PROXY", "no_proxy"):
    if "[" in os.environ.get(_k, ""):
        os.environ[_k] = "localhost,127.0.0.1"

EXPECTED_CHUNKS = 101471
EXPECTED_SHA = "07b3e17b88c07e8b46e466feb58f0da64c9e1f4213b0292eb253ff6f8c214075"
QUERY_PREFIX = "Represent this sentence for searching relevant passages: "
STOP = {"what", "when", "where", "which", "that", "this", "with", "from", "have",
        "does", "did", "the", "and", "for", "how", "many", "much", "were", "was",
        "are", "you", "your"}


def fts_query_baseline(q):
    words = [w for w in re.findall(r"[a-z0-9]+", q.lower()) if len(w) > 3 and w not in STOP]
    return " OR ".join(f'"{w}"' for w in words) or '""'


def fts_query_hybrid(q):
    words = [w for w in re.findall(r"[a-z0-9]+", q.lower()) if len(w) > 3]
    return " OR ".join('"' + w + '"' for w in words) or '""'


def check_corpus(con):
    n = con.execute("SELECT count(*) FROM chunks").fetchone()[0]
    h = hashlib.sha256()
    for (ch,) in con.execute("SELECT content_hash FROM chunks WHERE id % 997 = 0 ORDER BY id"):
        h.update(ch.encode())
    sha = h.hexdigest()
    print(f"[eval3] corpus chunks={n} fingerprint={sha}")
    if n != EXPECTED_CHUNKS or sha != EXPECTED_SHA:
        print("[eval3] FATAL: corpus does not match the frozen v3 fingerprint", file=sys.stderr)
        sys.exit(2)


def load_matrix(vecdb, table, trunc):
    con = sqlite3.connect(vecdb)
    ids, vecs = [], []
    for cid, blob in con.execute(f"SELECT chunk_id, vec FROM {table} ORDER BY chunk_id"):
        v = np.frombuffer(blob, dtype=np.float32)
        if trunc:
            v = v[:trunc]
        ids.append(cid)
        vecs.append(v)
    con.close()
    M = np.stack(vecs).astype(np.float32)
    M = M / np.linalg.norm(M, axis=1, keepdims=True)
    return ids, M


def embed_queries(framework, model, texts, qcache):
    if qcache and Path(qcache).exists():
        Q = np.load(qcache)
        if Q.shape[0] == len(texts):
            print(f"[eval3] query embeddings from cache {qcache} shape={Q.shape}")
            return Q
    if framework == "fastembed":
        from fastembed import TextEmbedding
        emb = TextEmbedding(model_name=model)
        qs = [(QUERY_PREFIX + t) if "bge" in model.lower() else t for t in texts]
        Q = np.stack([np.asarray(v, dtype=np.float32) for v in emb.embed(qs)])
    elif framework == "st":
        from sentence_transformers import SentenceTransformer
        m = SentenceTransformer(model)
        Q = m.encode(texts, prompt_name="query", batch_size=32,
                     convert_to_numpy=True, show_progress_bar=False).astype(np.float32)
    else:
        raise ValueError(framework)
    Q = Q / np.linalg.norm(Q, axis=1, keepdims=True)
    if qcache:
        np.save(qcache, Q)
    return Q


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--mode", choices=["fts", "vector", "hybrid"], required=True)
    ap.add_argument("--corpus", required=True)
    ap.add_argument("--queries", required=True)
    ap.add_argument("--vecdb")
    ap.add_argument("--table")
    ap.add_argument("--framework", choices=["fastembed", "st"])
    ap.add_argument("--model")
    ap.add_argument("--trunc", type=int, default=0)
    ap.add_argument("--k", type=int, default=10)
    ap.add_argument("--qcache")
    ap.add_argument("--out")
    ap.add_argument("--tag", default="")
    args = ap.parse_args()

    con = sqlite3.connect(args.corpus)
    check_corpus(con)
    data = json.load(open(args.queries))
    queries = data["queries"]
    t0 = time.time()

    ids = M = Q = None
    if args.mode in ("vector", "hybrid"):
        ids, M = load_matrix(args.vecdb, args.table, args.trunc)
        Q = embed_queries(args.framework, args.model, [q["query"] for q in queries], args.qcache)
        print(f"[eval3] matrix {M.shape} queries {Q.shape}")

    per_q = []
    hits5 = hits10 = 0
    mrr_sum = 0.0
    per_cat = {}
    for qi, q in enumerate(queries):
        gold = set(q["chunk_ids"])
        if args.mode == "fts":
            ranked = [r[0] for r in con.execute(
                "SELECT c.id FROM chunks c JOIN chunks_fts f ON f.rowid = c.id "
                "WHERE chunks_fts MATCH ? ORDER BY bm25(chunks_fts) LIMIT ?",
                (fts_query_baseline(q["query"]), args.k))]
        else:
            sims = M @ Q[qi]
            order = np.argsort(-sims)[: args.k * 3]
            ranked = [ids[i] for i in order]
            if args.mode == "hybrid":
                fts_ids = [r[0] for r in con.execute(
                    "SELECT c.id FROM chunks c JOIN chunks_fts f ON f.rowid = c.id "
                    "WHERE chunks_fts MATCH ? ORDER BY bm25(chunks_fts) LIMIT 30",
                    (fts_query_hybrid(q["query"]),))]
                score = {}
                for r, cid in enumerate(ranked):
                    score[cid] = score.get(cid, 0) + 1.0 / (60 + r)
                for r, cid in enumerate(fts_ids):
                    score[cid] = score.get(cid, 0) + 1.0 / (60 + r)
                ranked = [cid for cid, _ in sorted(score.items(), key=lambda x: -x[1])]
        rank = next((i + 1 for i, cid in enumerate(ranked[: args.k]) if cid in gold), None)
        if rank:
            mrr_sum += 1.0 / rank
            if rank <= 5:
                hits5 += 1
            if rank <= 10:
                hits10 += 1
        c = per_cat.setdefault(q["category"], [0, 0, 0.0])
        c[1] += 1
        c[0] += 1 if rank else 0
        c[2] += (1.0 / rank) if rank else 0.0
        per_q.append({"id": q["id"], "category": q["category"], "rank": rank})
        print("%-8s %-16s %5s  %s" % (q["id"], q["category"], str(rank or "-"), q["query"][:60]))

    n = len(queries)
    summary = {
        "tag": args.tag, "mode": args.mode, "table": args.table, "model": args.model,
        "trunc": args.trunc, "n": n,
        "recall@5": hits5, "recall@10": hits10, "mrr": round(mrr_sum / n, 4),
        "per_category": {cat: {"recall@10": h, "n": t, "mrr": round(ms / t, 4)}
                         for cat, (h, t, ms) in sorted(per_cat.items())},
        "seconds": round(time.time() - t0, 1),
    }
    print("\n[eval3] SUMMARY " + json.dumps(summary, indent=1))
    if args.out:
        json.dump({"summary": summary, "per_question": per_q}, open(args.out, "w"), indent=1)
        print(f"[eval3] wrote {args.out}")


if __name__ == "__main__":
    main()
