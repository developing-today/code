# atlas-embed: embeddings as a library

Source: [`crates/atlas-embed/src/lib.rs`](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/crates/atlas-embed/src/lib.rs), [`crates/atlas-embed/tests/gate_a.rs`](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/crates/atlas-embed/tests/gate_a.rs), [`crates/atlas-embed/examples/timing.rs`](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/crates/atlas-embed/examples/timing.rs), [`fixtures/embed-probes.json`](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/fixtures/embed-probes.json), and the wrapper in [`crates/atlas-core/src/embed.rs`](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/crates/atlas-core/src/embed.rs).

`atlas-embed` is a single-file crate (`crates/atlas-embed/src/lib.rs`) that turns text into embedding vectors — fixed-length lists of floats that place texts with similar meaning near each other — for exactly one model. It wraps ONNX Runtime (a runtime for running neural-network models from `.onnx` files) and the model's own tokenizer (the component that splits text into the integer token ids the model consumes). Everything else in Atlas that needs a vector — the search engine, promotion at ingest — goes through this crate or through the thin wrapper in [atlas-core: the engine as a library](atlas-core.md), so there is exactly one place where a vector can be produced. That singularity is deliberate: vectors from different models, or from the same model handled differently, are not comparable, and the crate's whole design is a parity contract (see Gate A, below) with the Python pipeline that embedded the production corpus.

```rust
use std::path::Path;
use atlas_embed::Embedder;

let mut embedder = Embedder::load(Path::new(model_dir), Path::new(ort_lib))?;
let vectors = embedder.embed(&["a toy note about ballast and sailboats"])?;
assert_eq!(vectors[0].len(), 768);
```

## The pinned model identity

The model is Snowflake `snowflake-arctic-embed-m-v2.0`, at the pinned revision `95c2741480856aa9666782eb4afe11959938017f`. The crate itself carries no model-id string constant — identity strings live one layer up, in atlas-core (`DEFAULT_MODEL = "Snowflake/snowflake-arctic-embed-m-v2.0"`) and in the Gate A fixture provenance. What the crate pins is the snapshot layout `load` expects, and the numeric constants:

| Constant | Value | Meaning |
|---|---|---|
| `MODEL_CONTEXT` | `8192` | the model's real context length, in tokens |
| `EMBED_DIM` | `768` | output width, full width — no Matryoshka truncation (dimension-shortening) happens in this crate; width selection is a caller's concern |
| `PAD_ID` | `0` | the token id used for batch padding |

A model snapshot directory must contain `tokenizer.json` at its root and `onnx/model.onnx`, the fp32 graph. The pinned files are hash-verified at install time by the `fetch-model` verb: `onnx/model.onnx` has SHA-256 `c0c53d7f49a2db60761b92b7bbf5be87a7b3cf5d92dbbd7f1b5028bd5a40aa39` and `tokenizer.json` has SHA-256 `f1cc44ad7faaeec47241864835473fd5403f2da94673f3f764a77ebcb0a803ec`. For how the snapshot gets onto a machine, see [Configuration and environment](../reference/configuration.md).

## The public API

The whole surface is one struct, three entry points around it, one helper, and one error type.

| Entry point | Shape | Meaning |
|---|---|---|
| `Embedder::load(model_dir, ort_lib)` | `(&Path, &Path) -> Result<Embedder>` | load the tokenizer and ONNX session eagerly, with the recorded corpus-pass thread counts (intra-op 8, inter-op 2) |
| `Embedder::load_with_threads(model_dir, ort_lib, intra, inter)` | `(…, usize, usize) -> Result<Embedder>` | same, with explicit ONNX Runtime thread counts |
| `Embedder::tokenize(&self, text)` | `(&str) -> Result<Vec<u32>>` | token ids for one text, with special tokens added, no truncation, no padding |
| `Embedder::embed(&mut self, texts)` | `(&[&str]) -> Result<Vec<[f32; 768]>>` | document-mode embedding: prompt-free (see below) |
| `Embedder::embed_ids(&mut self, id_lists)` | `(&[Vec<u32>]) -> Result<Vec<[f32; 768]>>` | the core path: embed pre-tokenized inputs; batch composition never changes a vector |
| `pairwise_sum_f32(a)` | `(&[f32]) -> f32` | NumPy's pairwise summation, replicated bit-for-bit (block size 128, 8-way unrolled accumulators) |

An `Embedder` holds one ONNX `Session`, the model's `Tokenizer`, and the ORT `Environment` for the session's lifetime. Construction is explicit and eager — there is no lazy loading inside the crate; serve mode's lazy, once-only initialisation is the caller's once-cell, and because ONNX sessions take `&mut self`, callers share an embedder behind a mutex. Loading a session costs roughly 1.6 GB of resident memory, which is why it is shared state rather than per-request state.

## How a vector is produced

Every point in this pipeline is load-bearing for parity:

- **Tokenizer.** The model's own `tokenizer.json` — a SentencePiece Unigram tokenizer with a Metaspace pre-tokenizer and a precompiled normalizer, using a `&lt;s&gt;`/`&lt;/s&gt;` template. It is not BERT WordPiece. It is loaded through the `tokenizers` crate.
- **Truncation is off.** `load` explicitly disables the tokenizer's truncation and padding. The shipped tokenizer config carries a 512-token truncation default, but the model's real context is 8192 tokens; padding is built manually instead (below). An input longer than 8192 tokens is a loud `TooLong` error, never a silent cut.
- **Batching.** Batches pad on the right with pad token id `0` and attention mask `0` — replicating the corpus pipeline's manual `np.zeros` padding. (A fork sidecar elsewhere pads with id 1; a recorded fixture diagnostic shows the two are bit-identical on this fp32 graph, max delta 0.0 on every probe including a 1,272-token chunk, because masked positions never reach the CLS row. The crate replicates the corpus pipeline's 0.)
- **Validation before tensors.** Batch validation runs before any tensor is built: an empty batch is `EmptyBatch`, and any empty entry is `EmptyEntry { index }`. This check exists because, before it, an empty entry in a mixed batch silently returned a fabricated unit-norm embedding of pure padding.
- **Session shape.** Session inputs are `input_ids` and `attention_mask`; output 0 must have shape `[batch, seq, 768]` or the call fails with `BadShape`.
- **Pooling.** Pooling is CLS pooling: the vector is row 0 of the model's hidden states (`hidden[:, 0, :]`, per the model's shipped `1_Pooling` config), then L2-normalized in f32. The norm's sum of squares uses `pairwise_sum_f32`, replicating NumPy's summation order exactly, because Gate A grants no tolerance for a differently ordered accumulation.

## The `query: ` prompt — and who applies it

`atlas-embed` deliberately does **not** apply any prompt. The pinned model expects query-side text to be prefixed with the literal string `query: ` (the sentence-transformers `prompt_name="query"` convention, from the model's `config_sentence_transformers.json`), and documents to be embedded with no prefix at all. Applying the prefix is the search layer's job, in atlas-core's `embed` module:

- `QUERY_PROMPT = "query: "` lives in atlas-core, not in this crate.
- atlas-core's `AtlasEmbedder::embed_raw(text)` formats `"{QUERY_PROMPT}{text}"` and calls `Embedder::embed`; its `embed_doc_raw(text)` calls the same crate entry point with the bare text.
- The query path then finishes the vector: slice to the requested truncation width if one is set (default 0 = full width), then L2-renormalize with `pairwise_sum_f32`. A zero vector at that point is fatal (`query embedding produced a zero vector`). The document path does no truncation and no renormalization — that is the shape the production corpus was embedded in, and cosine distance is scale-invariant, so the query side owns all three adjustments.

The practical rule for library callers: call `embed` with document text as-is; never pass a query through it without adding `query: ` yourself, or — better — use the atlas-core wrapper, which makes the distinction for you. See [Embeddings: Snowflake Arctic and the parity contract](../concepts/embeddings.md) for why this asymmetry matters.

## ONNX session loading

The `ort` crate is built with `default-features = false` and the `load-dynamic` feature, so ONNX Runtime is loaded at run time from a caller-supplied library path via `ort::init_from(ort_lib)`, under the environment name `atlas-embed`. Nothing is downloaded at build time or at run time — downloading a prebuilt onnxruntime at build time is forbidden in the Nix sandbox, and the crate honours that by construction.

The crate reads no environment variables itself. Callers resolve two paths and pass them in:

- **Model directory** — resolved from `ATLAS_MODEL_DIR`.
- **ONNX Runtime library** — resolved from `ATLAS_ORT_LIB` (a path to `libonnxruntime.so`).

The `atlas-search` binary's full resolution chain, which the other callers (the Gate A test, the crate's `examples/timing.rs`, the atlas-core examples) follow in shorter form, is:

1. `ATLAS_MODEL_DIR`, if set, non-empty, and an existing directory; otherwise `~/.local/state/atlas-search/model/&lt;rev&gt;` if it is a directory; otherwise the Hugging Face cache snapshot at `~/.cache/huggingface/hub/models--Snowflake--snowflake-arctic-embed-m-v2.0/snapshots/&lt;rev&gt;`; otherwise a model-not-found error. `&lt;rev&gt;` is the pinned revision above.
2. `ATLAS_ORT_LIB`, if set, non-empty, and an existing file; otherwise `~/.local/state/atlas-search/ort/libonnxruntime.so` if it is a file; otherwise the v3 virtualenv's `libonnxruntime.so.1.31.1`; otherwise the compiled-in default, the build-time `ATLAS_SEARCH_ORT_LIB_DEFAULT` (a compile-time value baked into packaged builds, pointing at the build's onnxruntime store path; development builds never set it).

## The parity contract: Gate A

Gate A (developing-today/code#5351) is the contract that this crate reproduces the production Python corpus pipeline (`embed_onnx.py` in the corpus workspace) **bit-identically**: token ids equal, output vectors at delta 0.0, for single inputs and for batches. The release profile of the whole workspace carries no fast-math-style flags for the same reason — embedder numerics are the contract.

The reference data is `fixtures/embed-probes.json`: 11 probes (`spike_fox`, `spike_query_prefix`, `spike_semantic`, `spike_single_char`, `spike_japanese`, `spike_empty`, `corpus_6359`, `corpus_15202`, `corpus_39430`, `corpus_long`, `query_style`), recorded by `fixtures/gen_fixtures.py` on 2026-11-04 with onnxruntime 1.31.1, transformers 5.21.0, and python tokenizers 0.23.3, at intra-op 8 / inter-op 2 threads. Three probes and the long probe are real corpus chunks referenced by chunk id only; their text is not in the repository, and none is reproduced here.

The claim is version-matched. The gate reads the runtime version from the versioned library filename (`libonnxruntime.so.1.31.1` gives `1.31.1`) and refuses to run against a different version: a different runtime produces deltas around 1e-7 on most components of every probe, so against one the gate reports NOT APPLICABLE rather than a parity verdict. The Gate A test is ignored in ordinary CI because it needs `ATLAS_ORT_LIB` set and the pinned snapshot present.

## Worked example

Following the pattern of the crate's own `examples/timing.rs` and the Gate A test — resolve the two paths from the environment, load once, embed a small batch of toy documents:

```rust
use std::path::PathBuf;
use atlas_embed::Embedder;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let model_dir = PathBuf::from(std::env::var("ATLAS_MODEL_DIR")?);
    let ort_lib = PathBuf::from(std::env::var("ATLAS_ORT_LIB")?);

    let mut embedder = Embedder::load(&model_dir, &ort_lib)?;

    let docs = [
        "toy note: a model sailboat stays upright because of its ballast",
        "toy note: watering the garden before noon",
    ];
    let vectors = embedder.embed(&docs)?;
    for (doc, vector) in docs.iter().zip(&vectors) {
        println!("{doc:?} -> {} dims, first = {:.6}", vector.len(), vector[0]);
    }
    Ok(())
}
```

Each returned vector is a `[f32; 768]`, L2-normalized, document-side (no `query: ` prefix — these are documents). Embedding the same two texts in a different batch composition, or one at a time, returns bit-identical vectors; that is the property Gate A pins.

## Failure shapes

The crate's `Error` enum is the complete failure vocabulary; there are no panics on bad input and no partial results.

| Variant | When |
|---|---|
| `Ort(String)` | ONNX Runtime failed to load, or the session run failed |
| `Tokenizer(String)` | the tokenizer failed to load, or tokenization failed |
| `TooLong { tokens }` | an input exceeds the 8192-token context |
| `EmptyBatch` | `embed` / `embed_ids` was called with no inputs |
| `EmptyEntry { index }` | the entry at `index` tokenized to nothing |
| `BadShape(Vec&lt;i64&gt;)` | session output 0 was not `[batch, seq, 768]` |

One layer up, atlas-core wraps embedding failures as its own `Error::Embed(String)`, alongside `Fatal` and `Backend` — so a search caller sees one error type for the whole engine. As a library, `atlas-embed` has no exit codes; exit-code mapping belongs to the binaries built on it. The wire types its vectors travel in are covered in [atlas-proto: the wire contract](atlas-proto.md).
