#!/usr/bin/env python3
"""Generate fixtures/embed-probes.json — the Gate A reference data for the
atlas-embed crate (developing-today/code#5351, design §5 Gate A).

The reference pipeline is the one that produced the production Atlas
corpus: ~/atlas/v3/embed_onnx.py — onnxruntime + AutoTokenizer over the
Snowflake arctic-embed-m-v2.0 snapshot's onnx/model.onnx (fp32 graph),
CLS pooling, L2 normalize, manual right-padding with 0 and attention
mask 0. This script replicates that session construction and per-batch
math exactly (same SessionOptions thread counts as the recorded pass:
intra=8, inter=2) and records, per probe:

  token_ids    — tokenizer output (add_special_tokens=True)
  single_bits  — embedding of the probe alone (batch of 1, unpadded),
                 as IEEE-754 f32 bit patterns (u32)
  batch_bits   — embedding of the probe inside one fixed mixed batch
                 containing every probe in listed order, padded to the
                 batch max length with 0 (embed_onnx.py's per-batch math)

Probe set:
  spike_*     — the five fixed texts from the fork's native-embeddings
                spike (developing-today/opencode#1554), plus an empty
                string edge probe
  corpus_*    — real corpus chunks, referenced by chunk id from
                ~/atlas/v3/corpus.db; the chunk TEXT is deliberately
                not copied into the repo (it is personal corpus
                content). The generator re-derives it from corpus.db
                on every run; the gate data is ids + vectors.
  query_style — a natural query-shaped sentence embedded prompt-free
                (document mode; the "query: " prompt path is step 2)

Rerun (on the laptop, amd-6 — never the sandbox):
  LD_LIBRARY_PATH=<gcc-lib>:<zlib-lib> ~/atlas/v3-venv/bin/python gen_fixtures.py
with ATLAS_MODEL_DIR pointing at the pinned HF snapshot (or the HF
cache default below). The script fails loudly if any invariant breaks
(long probe not >512 tokens, tokenizer disagreement between batched
and per-probe calls, runtime nondeterminism on a repeat embed).
"""

import hashlib
import json
import os
import sqlite3
import struct
import subprocess
import sys
import time

MODEL_REV = "95c2741480856aa9666782eb4afe11959938017f"
DEFAULT_MODEL_DIR = os.path.expanduser(
    "~/.cache/huggingface/hub/models--Snowflake--snowflake-arctic-embed-m-v2.0"
    f"/snapshots/{MODEL_REV}"
)
CORPUS_DB = os.path.expanduser("~/atlas/v3/corpus.db")
INTRA_THREADS = 8
INTER_THREADS = 2

SPIKE_TEXTS = [
    ("spike_fox", "The quick brown fox jumps over the lazy dog."),
    ("spike_query_prefix", "query: what is the capital of France?"),
    (
        "spike_semantic",
        "Embeddings turn text into vectors that capture semantic meaning, "
        "so similar sentences land close together in a high-dimensional space.",
    ),
    ("spike_single_char", "a"),
    (
        "spike_japanese",
        "Snowflake Arctic Embed is a family of text embedding models "
        "optimized for retrieval workloads across many languages — "
        "日本語のテキストも含まれます。",
    ),
    ("spike_empty", ""),
]
GATE_CHUNK_IDS = [6359, 15202, 39430]
QUERY_STYLE_TEXT = "how do I rebuild the atlas vector index?"


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def bits(vec):
    return [struct.unpack("<I", struct.pack("<f", float(x)))[0] for x in vec]


def main():
    model_dir = os.environ.get("ATLAS_MODEL_DIR", DEFAULT_MODEL_DIR)
    import numpy as np
    import onnxruntime as ort
    import tokenizers as tokenizers_rs
    import transformers
    from transformers import AutoTokenizer

    tok = AutoTokenizer.from_pretrained(model_dir)
    so = ort.SessionOptions()
    so.intra_op_num_threads = INTRA_THREADS
    so.inter_op_num_threads = INTER_THREADS
    sess = ort.InferenceSession(
        os.path.join(model_dir, "onnx", "model.onnx"),
        sess_options=so,
        providers=["CPUExecutionProvider"],
    )
    input_names = [i.name for i in sess.get_inputs()]
    assert input_names == ["input_ids", "attention_mask"], input_names

    def tokenize(text):
        return tok([text], add_special_tokens=True)["input_ids"][0]

    def run_batch(id_lists, pad_id=0):
        # embed_onnx.py's per-batch math, verbatim in structure.
        maxlen = max(len(x) for x in id_lists)
        input_ids = np.full((len(id_lists), maxlen), pad_id, dtype=np.int64)
        attn = np.zeros((len(id_lists), maxlen), dtype=np.int64)
        for j, x in enumerate(id_lists):
            input_ids[j, : len(x)] = x
            attn[j, : len(x)] = 1
        hidden = sess.run(None, {"input_ids": input_ids, "attention_mask": attn})[0]
        vecs = hidden[:, 0, :].astype(np.float32)
        vecs = vecs / np.linalg.norm(vecs, axis=1, keepdims=True)
        return vecs

    # ---- assemble probes -------------------------------------------------
    probes = []  # (name, source, corpus_id|None, text)
    for name, text in SPIKE_TEXTS:
        probes.append((name, "opencode#1554 spike text", None, text))
    con = sqlite3.connect(CORPUS_DB)
    for cid in GATE_CHUNK_IDS:
        row = con.execute("SELECT text FROM chunks WHERE id = ?", (cid,)).fetchone()
        assert row, f"gate chunk {cid} missing from corpus"
        probes.append((f"corpus_{cid}", "atlas v3 gate chunk", cid, row[0]))
    # long probe: prefer the first chunk (by id) tokenizing to [600, 800]
    # tokens; fall back to the smallest count above 512 seen in the scan.
    long_pick = None
    fallback = None
    scanned = 0
    for cid, text in con.execute(
        "SELECT id, text FROM chunks WHERE length(text) BETWEEN 2000 AND 6000 ORDER BY id"
    ):
        n = len(tokenize(text))
        scanned += 1
        if 600 <= n <= 800:
            long_pick = (cid, text, n)
            break
        if n > 512 and (fallback is None or n < fallback[2]):
            fallback = (cid, text, n)
        if scanned >= 400 and fallback is not None:
            break
    long_pick = long_pick or fallback
    assert long_pick, "no corpus chunk with >512 tokens found"
    print(
        f"long probe: chunk {long_pick[0]} = {long_pick[2]} tokens (scanned {scanned})"
    )
    probes.append(
        ("corpus_long", "atlas v3 chunk, >512 tokens", long_pick[0], long_pick[1])
    )
    con.close()
    probes.append(
        ("query_style", "synthetic query-shaped text", None, QUERY_STYLE_TEXT)
    )

    # ---- tokenize ---------------------------------------------------------
    all_texts = [p[3] for p in probes]
    bulk = tok(all_texts, add_special_tokens=True)["input_ids"]
    id_lists = []
    for i, p in enumerate(probes):
        ids = tokenize(p[3])
        assert ids == bulk[i], f"tokenizer disagreement on probe {p[0]}"
        id_lists.append(ids)

    # ---- embed: singles, one mixed batch (pad 0), diagnostic batch (pad 1)
    singles = [run_batch([ids])[0] for ids in id_lists]
    again = run_batch([id_lists[0]])[0]
    assert bits(again) == bits(singles[0]), "runtime nondeterminism on repeat embed"
    batch0 = run_batch(id_lists, pad_id=0)
    batch1 = run_batch(id_lists, pad_id=1)

    out_probes = []
    print(
        f"{'probe':<20} {'tokens':>6}  single-vs-batch0 max|d|   batch pad0-vs-pad1 max|d|"
    )
    for i, (name, source, cid, text) in enumerate(probes):
        d_sb = float(np.max(np.abs(singles[i] - batch0[i])))
        d_pp = float(np.max(np.abs(batch0[i] - batch1[i])))
        print(f"{name:<20} {len(id_lists[i]):>6}  {d_sb:.3e}              {d_pp:.3e}")
        rec = {
            "name": name,
            "source": source,
            "token_ids": [int(x) for x in id_lists[i]],
            "single_bits": bits(singles[i]),
            "batch_bits": bits(batch0[i]),
        }
        if cid is not None:
            rec["corpus_chunk_id"] = cid
        else:
            rec["text"] = text
        out_probes.append(rec)

    long_rec = next(p for p in out_probes if p["name"] == "corpus_long")
    assert len(long_rec["token_ids"]) > 512, "long probe is not >512 tokens"

    fixture = {
        "provenance": {
            "generated_by": "pkgs/atlas-search/fixtures/gen_fixtures.py",
            "reference_pipeline": "~/atlas/v3/embed_onnx.py (production corpus pipeline)",
            "model": "Snowflake/snowflake-arctic-embed-m-v2.0",
            "model_rev": MODEL_REV,
            "onnx_file": "onnx/model.onnx (fp32 graph)",
            "model_onnx_sha256": sha256_file(
                os.path.join(model_dir, "onnx", "model.onnx")
            ),
            "tokenizer_json_sha256": sha256_file(
                os.path.join(model_dir, "tokenizer.json")
            ),
            "onnxruntime_version": ort.__version__,
            "transformers_version": transformers.__version__,
            "python_tokenizers_version": tokenizers_rs.__version__,
            "intra_op_threads": INTRA_THREADS,
            "inter_op_threads": INTER_THREADS,
            "padding": "right, pad token id 0, attention mask 0 (embed_onnx.py semantics)",
            "pooling": "CLS (hidden[:, 0, :]), L2 normalize, f32",
            "truncation": "off — model context is 8192; >8192 tokens is a loud error in the crate",
            "batch_composition": "single mixed batch, every probe in listed order, padded to batch max length",
            "generated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
            "generator_host": subprocess.run(
                ["hostname"], capture_output=True, text=True
            ).stdout.strip(),
        },
        "probes": out_probes,
    }
    out_path = os.path.join(
        os.path.dirname(os.path.abspath(__file__)), "embed-probes.json"
    )
    with open(out_path, "w") as f:
        json.dump(fixture, f, indent=1, ensure_ascii=False)
    print(
        f"wrote {out_path} ({os.path.getsize(out_path)} bytes, {len(out_probes)} probes)"
    )


if __name__ == "__main__":
    sys.exit(main())
