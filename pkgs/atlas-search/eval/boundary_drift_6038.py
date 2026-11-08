#!/usr/bin/env python3
# lane-scratch: code-d /tmp/lane-d-20261108-0448 task 13010
"""Boundary-drift measurement for atlas span reuse (developing-today/code#6038).

Question: a span's stored vector is a function of its chunk's exact text
including window neighbors. When the same paragraph is embedded in a
perturbed window (shifted boundaries, changed neighbors), how much do
its vector (cosine drift) and its rank on a fixed query set move?

Method (numbers only — no corpus text, message ids, or query text are
printed or saved; the corpus is private):
- Corpus: ~/atlas/v3/corpus.db `messages` (the real Atlas corpus).
  Paragraphs = blank-line splits. A target is a paragraph with both a
  previous and a next paragraph in the same message (200..2000 chars).
  Deterministic sample of 50 targets (seed 6038).
- Original window W0 = prev + target + next (blank-line joined), the
  production chunk shape.
- Perturbations per target:
  * boundary_shift — drop W0's first sentence; append the first
    sentence of the paragraph after `next` when one exists.
  * neighbor_replace_prev / neighbor_replace_next — replace that
    neighbor with the target paragraph of another sampled message
    (deterministic donor offsets +7 / +13).
  * target_alone — the target paragraph with all context removed
    (bounding case).
- Embedder: the production pipeline replicated exactly from
  ~/atlas/v3/embed_onnx.py — pinned Snowflake
  snowflake-arctic-embed-m-v2.0 ONNX graph, AutoTokenizer
  (add_special_tokens=True), right-pad 0 / attention-mask 0, CLS
  pooling, L2 normalize. Documents prompt-free; queries embedded with
  the production "query: " prompt (atlas-core embed.rs QUERY_PROMPT).
- Queries: fixed set of 20 from ~/atlas/v3/golden-queries-v3.json
  (every 16th of 311).
- Rank movement: pool = the 50 original window vectors. For query q,
  rank0 = rank of W0_i in the pool; rank_p = rank of the perturbed
  vector when it replaces W0_i in the pool. Report |rank change|
  distribution and top-10 membership flips.

Run (laptop only):
  LD_LIBRARY_PATH=<gcc-lib>:<zlib-lib> ~/atlas/v3-venv/bin/python \
      boundary_drift_6038.py --out results.json
"""
import argparse, glob, json, random, re, sqlite3, sys, time
import numpy as np

SEED = 6038
N_TARGETS = 50
TOPK = 10

def embed_all(sess, tok, texts, batch_tokens=32768):
    enc = tok(texts, add_special_tokens=True)["input_ids"]
    order = sorted(range(len(texts)), key=lambda i: -len(enc[i]))
    out = [None] * len(texts)
    input_names = [i.name for i in sess.get_inputs()]
    cur, cur_max = [], 0
    batches = []
    for i in order:
        l = len(enc[i]); nm = max(cur_max, l)
        if cur and nm * (len(cur) + 1) > batch_tokens:
            batches.append(cur); cur, cur_max = [i], l
        else:
            cur.append(i); cur_max = nm
    if cur: batches.append(cur)
    for b in batches:
        ids = [enc[i] for i in b]; ml = max(len(x) for x in ids)
        ii = np.zeros((len(b), ml), dtype=np.int64); am = np.zeros((len(b), ml), dtype=np.int64)
        for j, x in enumerate(ids):
            ii[j, :len(x)] = x; am[j, :len(x)] = 1
        feed = {}
        for n in input_names:
            if n == "input_ids": feed[n] = ii
            elif n == "attention_mask": feed[n] = am
            elif n == "token_type_ids": feed[n] = np.zeros_like(ii)
        h = sess.run(None, feed)[0]
        v = h[:, 0, :].astype(np.float32)
        v = v / np.linalg.norm(v, axis=1, keepdims=True)
        for j, i in enumerate(b): out[i] = v[j]
    return np.stack(out)

def sentences(text):
    return [s for s in re.split(r"(?<=[.!?])\s+", text.strip()) if s]

def pct(a, q):
    return float(np.percentile(np.asarray(a, dtype=np.float64), q))

def stats(a):
    a = np.asarray(a, dtype=np.float64)
    return {"n": int(a.size), "median": float(np.median(a)), "p95": pct(a, 95),
            "mean": float(a.mean()), "max": float(a.max())}

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("--corpus", default="/home/user/atlas/v3/corpus.db")
    ap.add_argument("--golden", default="/home/user/atlas/v3/golden-queries-v3.json")
    args = ap.parse_args()
    import onnxruntime as ort
    from transformers import AutoTokenizer

    model_dir = sorted(glob.glob("/home/user/.cache/huggingface/hub/models--Snowflake--snowflake-arctic-embed-m-v2.0/snapshots/*"))[-1]
    tok = AutoTokenizer.from_pretrained(model_dir)
    so = ort.SessionOptions(); so.intra_op_num_threads = 16; so.inter_op_num_threads = 2
    sess = ort.InferenceSession(model_dir + "/onnx/model.onnx", sess_options=so, providers=["CPUExecutionProvider"])

    db = sqlite3.connect(f"file:{args.corpus}?mode=ro", uri=True)
    rows = db.execute("SELECT id, text FROM messages WHERE text IS NOT NULL ORDER BY id").fetchall()
    db.close()
    cands = []  # (prev, target, next, after_next_or_None)
    for _mid, text in rows:
        paras = [p.strip() for p in text.split("\n\n") if p.strip()]
        for i in range(1, len(paras) - 1):
            t = paras[i]
            if 200 <= len(t) <= 2000 and len(sentences(t)) >= 2:
                after = paras[i + 2] if i + 2 < len(paras) else None
                cands.append((paras[i - 1], t, paras[i + 1], after))
    rng = random.Random(SEED)
    sample = rng.sample(cands, N_TARGETS)
    donors = [c[1] for c in sample]

    W0, pert = [], {"boundary_shift": [], "neighbor_replace_prev": [], "neighbor_replace_next": [], "target_alone": []}
    shift_extended = 0
    for idx, (prev, tgt, nxt, after) in enumerate(sample):
        w0 = prev + "\n\n" + tgt + "\n\n" + nxt
        W0.append(w0)
        sents = sentences(w0)
        shifted = sents[1:]
        if after:
            shifted = shifted + [sentences(after)[0]]; shift_extended += 1
        pert["boundary_shift"].append(" ".join(shifted))
        pert["neighbor_replace_prev"].append(donors[(idx + 7) % N_TARGETS] + "\n\n" + tgt + "\n\n" + nxt)
        pert["neighbor_replace_next"].append(prev + "\n\n" + tgt + "\n\n" + donors[(idx + 13) % N_TARGETS])
        pert["target_alone"].append(tgt)

    golden = json.load(open(args.golden))["queries"]
    queries = [q["query"] for q in golden[::16][:20]]

    t0 = time.time()
    v0 = embed_all(sess, tok, W0)
    vp = {k: embed_all(sess, tok, v) for k, v in pert.items()}
    vq = embed_all(sess, tok, ["query: " + q for q in queries])
    embed_seconds = time.time() - t0

    S0 = vq @ v0.T  # (Q, N)
    order0 = np.argsort(-S0, axis=1)
    rank0 = np.empty_like(order0)
    for q in range(S0.shape[0]):
        rank0[q, order0[q]] = np.arange(1, S0.shape[1] + 1)
    top0 = [set(order0[q, :TOPK].tolist()) for q in range(S0.shape[0])]

    result = {
        "model_snapshot": model_dir.split("/")[-1],
        "corpus_messages_scanned": len(rows),
        "candidate_targets": len(cands),
        "n_targets": N_TARGETS, "n_queries": len(queries), "seed": SEED,
        "shift_extended_with_following_sentence": shift_extended,
        "embed_seconds": round(embed_seconds, 1),
        "per_type": {},
    }
    for name, V in vp.items():
        cos = np.sum(v0 * V, axis=1)
        drift = 1.0 - cos
        sp = vq @ V.T
        absdr, flips, pairs = [], 0, 0
        for q in range(S0.shape[0]):
            for i in range(N_TARGETS):
                col = S0[q].copy(); col[i] = sp[q, i]
                rp = int(np.sum(col > sp[q, i]) + 1)
                absdr.append(abs(rp - int(rank0[q, i]))); pairs += 1
                in0 = i in top0[q]; inp = rp <= TOPK
                if in0 != inp: flips += 1
        result["per_type"][name] = {
            "cosine_drift": stats(drift), "cosine_similarity_median": float(np.median(cos)),
            "abs_rank_change": stats(absdr), "top10_membership_flips": flips,
            "top10_flip_rate": flips / pairs, "query_target_pairs": pairs,
        }
    json.dump(result, open(args.out, "w"), indent=1)
    print(json.dumps(result, indent=1))

if __name__ == "__main__":
    main()
