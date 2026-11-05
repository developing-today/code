# Fixtures — Gate A reference data

`embed-probes.json` is the recorded output of the production Python
corpus pipeline for a fixed probe set. The `atlas-embed` crate's
Gate A test (`crates/atlas-embed/tests/gate_a.rs`) must reproduce it
**bit-identically**: token ids equal, output vectors delta 0.0,
embedded singly and in one fixed mixed batch. Provenance (model
revision, file hashes, runtime version, thread counts, padding and
pooling semantics) is recorded inside the JSON itself.

## Regenerating

`gen_fixtures.py` is the recorded, rerunnable generator — run it on
the laptop (amd-6), never the sandbox:

```sh
LD_LIBRARY_PATH=<x86-64 gcc lib>:<x86-64 zlib lib> \
  ~/atlas/v3-venv/bin/python gen_fixtures.py
```

It replicates `~/atlas/v3/embed_onnx.py` (the pipeline that embedded
the production corpus) exactly: fp32 `onnx/model.onnx`, CLS pooling,
L2 normalize, right-padding with token id 0 / attention mask 0. It
also prints a padding diagnostic — on this fp32 graph, pad id 0 vs
the fork sidecar's pad id 1 are bit-identical (masked positions
never reach the CLS row); the corpus pipeline's 0 is what's
recorded. Regenerate on any model revision bump, through this same
ceremony, and commit the result with the bump.

## Why corpus probes have no text here

Three probes (and the >512-token long probe) are real corpus chunks.
Their text is personal conversation content, so the fixture
references them by `corpus_chunk_id` (in `~/atlas/v3/corpus.db`) and
records the gate data — token ids + vectors — without copying the
text into this repo. The generator re-derives the text from
corpus.db on every run. The synthetic probes (the opencode#1554
spike texts, an empty string, a query-shaped sentence) carry their
text inline.
