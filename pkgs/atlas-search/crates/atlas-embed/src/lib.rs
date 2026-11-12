//! atlas-embed — the Snowflake `snowflake-arctic-embed-m-v2.0` embedder
//! for Atlas, and nothing else (design §4, developing-today/code#5350).
//!
//! The crate exists to be parity-tested in isolation: Gate A
//! (code#5351) requires it to reproduce the production Python corpus
//! pipeline (`~/atlas/v3/embed_onnx.py`) **bit-identically** — token
//! ids equal, output vectors delta 0.0, single and batched. The
//! reference fixtures live in `pkgs/atlas-search/fixtures/` with a
//! recorded, rerunnable generator.
//!
//! Pipeline contract (every point is load-bearing for the gate):
//! - The model's own `tokenizer.json`: SentencePiece Unigram with a
//!   Metaspace pre-tokenizer and a precompiled normalizer, `<s>`/`</s>`
//!   template. NOT BERT WordPiece. Loaded via the `tokenizers` crate.
//! - **Truncation is OFF.** The shipped tokenizer config carries a
//!   512-token truncation default; the model's real context is 8192.
//!   The corpus pipeline tokenizes whole texts and a >8192-token input
//!   is a loud error here, never a silent cut.
//! - Documents embed **prompt-free**. The sentence-transformers
//!   `prompt_name="query"` handling for query texts is a string-prefix
//!   step exercised by the search layer (step 2), not this crate.
//! - Batching pads **right, pad token id 0, attention mask 0** — the
//!   corpus pipeline's manual padding (`embed_onnx.py` builds its
//!   batch arrays with `np.zeros`). The fork's :18752 sidecar pads
//!   with id 1 instead; the recorded fixture diagnostic
//!   (gen_fixtures.py) shows the two are bit-identical on this fp32
//!   graph (max delta 0.0 on every probe, incl. a 1,272-token chunk),
//!   because masked positions never reach the CLS row. We replicate
//!   the corpus pipeline's 0 because it is the pipeline of record.
//! - Pooling is **CLS** (`hidden[:, 0, :]`, per the model's shipped
//!   1_Pooling config), then **L2 normalize** in f32. The norm's sum
//!   of squares replicates NumPy's pairwise summation order — see
//!   [`pairwise_sum_f32`] — because the reference does
//!   `vecs / np.linalg.norm(vecs, axis=1)` and Gate A grants no
//!   tolerance for a differently-ordered accumulation.
//! - ONNX Runtime is **dynamically loaded** from a caller-supplied
//!   library path; nothing is downloaded at build or run time
//!   (design §4 loader lesson). Session threading matches the
//!   recorded corpus pass: intra-op 8, inter-op 2 by default.

use std::path::Path;

use ort::environment::Environment;
use ort::session::Session;
use ort::value::Tensor;
use tokenizers::Tokenizer;

/// The model's real context length. Inputs longer than this are a
/// loud error, matching the corpus pipeline's contract.
pub const MODEL_CONTEXT: usize = 8192;
/// Full output width of arctic-embed-m-v2.0 (no Matryoshka truncation
/// in this crate; width selection is a caller concern).
pub const EMBED_DIM: usize = 768;
/// Pad token id used when batching. See the crate docs: this is the
/// corpus pipeline's 0, not the tokenizer's declared pad id 1, and
/// the choice is provably inert on this graph.
pub const PAD_ID: i64 = 0;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    // ort's error type is generic over the value a failed builder
    // call would have produced (ort::Error<SessionBuilder> etc.), so
    // the crate stringifies at the boundary instead of naming a
    // single instantiation.
    #[error("onnxruntime: {0}")]
    Ort(String),
    #[error("tokenizer: {0}")]
    Tokenizer(String),
    #[error("input tokenizes to {tokens} tokens, exceeding the model context of {MODEL_CONTEXT}")]
    TooLong { tokens: usize },
    #[error("cannot embed an empty batch")]
    EmptyBatch,
    #[error("token list at batch index {index} is empty")]
    EmptyEntry { index: usize },
    #[error("unexpected model output shape: {0:?}")]
    BadShape(Vec<i64>),
}

pub type Result<T> = std::result::Result<T, Error>;

/// A loaded embedder: one ONNX session + the model's tokenizer.
///
/// Construction is explicit and eager; callers that want the design's
/// lazy init (serve mode) wrap this in a once-cell.
pub struct Embedder {
    // Kept alive for the session's lifetime; sessions clone the
    // environment's Arc internally, but holding it here makes the
    // lifetime contract explicit at this crate's boundary.
    _env: Environment,
    session: Session,
    tokenizer: Tokenizer,
}

/// Stringify any ort error into this crate's error type.
fn ort<E: std::fmt::Display>(e: E) -> Error {
    Error::Ort(e.to_string())
}

/// Validate a batch of pre-tokenized inputs BEFORE any tensor is
/// built or the session runs (#5373). The batch as a whole must be
/// non-empty, and every entry must be non-empty and within the model
/// context. An empty entry used to be padded to the batch maxlen and
/// returned as a plausible unit-norm embedding of nothing (a lone
/// empty entry failed only by accident, as a raw ONNX error from a
/// zero-length tensor) — in a crate whose contract is loud errors,
/// that silent fabrication is refused here, at the boundary.
fn validate_id_lists(id_lists: &[Vec<u32>]) -> Result<()> {
    if id_lists.is_empty() {
        return Err(Error::EmptyBatch);
    }
    for (index, ids) in id_lists.iter().enumerate() {
        if ids.is_empty() {
            return Err(Error::EmptyEntry { index });
        }
        if ids.len() > MODEL_CONTEXT {
            return Err(Error::TooLong { tokens: ids.len() });
        }
    }
    Ok(())
}

impl Embedder {
    /// Load from a model directory (the pinned snapshot layout:
    /// `tokenizer.json` at the root, `onnx/model.onnx` the fp32
    /// graph) and a dynamically-loaded ONNX Runtime library.
    ///
    /// `intra_threads` / `inter_threads` default to the recorded
    /// corpus-pass values (8 / 2) via [`Embedder::load`]; use
    /// [`Embedder::load_with_threads`] to override.
    pub fn load(model_dir: &Path, ort_lib: &Path) -> Result<Self> {
        Self::load_with_threads(model_dir, ort_lib, 8, 2)
    }

    pub fn load_with_threads(
        model_dir: &Path,
        ort_lib: &Path,
        intra_threads: usize,
        inter_threads: usize,
    ) -> Result<Self> {
        let env = ort::init_from(ort_lib)
            .map_err(ort)?
            .with_name("atlas-embed")
            .build()
            .map_err(ort)?;

        let mut tokenizer = Tokenizer::from_file(model_dir.join("tokenizer.json"))
            .map_err(|e| Error::Tokenizer(e.to_string()))?;
        // The shipped tokenizer.json carries the stock 512-token
        // truncation default. The model's context is 8192 and the
        // corpus pipeline truncates nothing — kill it explicitly.
        tokenizer
            .with_truncation(None)
            .map_err(|e| Error::Tokenizer(e.to_string()))?;
        // Padding is built manually in embed_ids (the reference
        // pipeline pads its own arrays); never let the tokenizer pad.
        tokenizer.with_padding(None);

        let session = Session::builder(&env)
            .map_err(ort)?
            .with_intra_threads(intra_threads)
            .map_err(ort)?
            .with_inter_threads(inter_threads)
            .map_err(ort)?
            .commit_from_file(model_dir.join("onnx").join("model.onnx"))
            .map_err(ort)?;

        Ok(Self {
            _env: env,
            session,
            tokenizer,
        })
    }

    /// Tokenize one text exactly as the corpus pipeline does
    /// (`add_special_tokens=True`, no truncation, no padding).
    pub fn tokenize(&self, text: &str) -> Result<Vec<u32>> {
        let enc = self
            .tokenizer
            .encode(text, true)
            .map_err(|e| Error::Tokenizer(e.to_string()))?;
        Ok(enc.get_ids().to_vec())
    }

    /// Embed a batch of texts (document mode: prompt-free).
    pub fn embed(&mut self, texts: &[&str]) -> Result<Vec<[f32; EMBED_DIM]>> {
        let mut id_lists = Vec::with_capacity(texts.len());
        for text in texts {
            id_lists.push(self.tokenize(text)?);
        }
        self.embed_ids(&id_lists)
    }

    /// Embed pre-tokenized inputs. This is the core path; `embed` is
    /// its tokenizing front door. Batch composition never changes a
    /// vector (masked right-padding; Gate A asserts it).
    pub fn embed_ids(&mut self, id_lists: &[Vec<u32>]) -> Result<Vec<[f32; EMBED_DIM]>> {
        validate_id_lists(id_lists)?;
        let batch = id_lists.len();
        let maxlen = id_lists.iter().map(|v| v.len()).max().unwrap_or(0);

        let mut input_ids = vec![PAD_ID; batch * maxlen];
        let mut attn = vec![0i64; batch * maxlen];
        for (j, ids) in id_lists.iter().enumerate() {
            for (k, &id) in ids.iter().enumerate() {
                input_ids[j * maxlen + k] = i64::from(id);
                attn[j * maxlen + k] = 1;
            }
        }

        let ids_tensor =
            Tensor::from_array(([batch as i64, maxlen as i64], input_ids)).map_err(ort)?;
        let attn_tensor = Tensor::from_array(([batch as i64, maxlen as i64], attn)).map_err(ort)?;
        let outputs = self
            .session
            .run(ort::inputs![
                "input_ids" => ids_tensor,
                "attention_mask" => attn_tensor,
            ])
            .map_err(ort)?;
        let hidden = outputs[0].try_extract_array::<f32>().map_err(ort)?;
        let shape: Vec<i64> = hidden.shape().iter().map(|&d| d as i64).collect();
        if shape.len() != 3 || shape[0] != batch as i64 || shape[2] != EMBED_DIM as i64 {
            return Err(Error::BadShape(shape));
        }
        let view = hidden.view();
        let mut out = Vec::with_capacity(batch);
        for j in 0..batch {
            let mut cls = [0f32; EMBED_DIM];
            for k in 0..EMBED_DIM {
                cls[k] = view[[j, 0, k]];
            }
            l2_normalize(&mut cls);
            out.push(cls);
        }
        Ok(out)
    }
}

/// L2-normalize in place, replicating the reference pipeline's
/// arithmetic: the norm is `sqrt(pairwise_sum(x_i^2))` with the
/// squares and the sum in f32, then an elementwise f32 division.
fn l2_normalize(v: &mut [f32; EMBED_DIM]) {
    let squares: [f32; EMBED_DIM] = std::array::from_fn(|i| v[i] * v[i]);
    let norm = pairwise_sum_f32(&squares).sqrt();
    for x in v.iter_mut() {
        *x /= norm;
    }
}

/// NumPy's pairwise summation (PW_BLOCKSIZE = 128, 8-way unrolled
/// accumulators), replicated so the accumulation order — and
/// therefore the rounding — matches `np.add.reduce` bit for bit.
pub fn pairwise_sum_f32(a: &[f32]) -> f32 {
    let n = a.len();
    if n < 8 {
        let mut r = 0.0f32;
        for &x in a {
            r += x;
        }
        return r;
    }
    if n <= 128 {
        let mut r = [a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7]];
        let mut i = 8;
        while i < n - (n % 8) {
            for (k, acc) in r.iter_mut().enumerate() {
                *acc += a[i + k];
            }
            i += 8;
        }
        let mut res = ((r[0] + r[1]) + (r[2] + r[3])) + ((r[4] + r[5]) + (r[6] + r[7]));
        while i < n {
            res += a[i];
            i += 1;
        }
        return res;
    }
    let mut n2 = n / 2;
    n2 -= n2 % 8;
    pairwise_sum_f32(&a[..n2]) + pairwise_sum_f32(&a[n2..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn validate_refuses_lone_empty_entry() {
        // #5373: previously a raw ONNX Gather error from a [1, 0] tensor.
        let err = validate_id_lists(&[vec![]]).unwrap_err();
        assert!(matches!(err, Error::EmptyEntry { index: 0 }), "{err}");
    }

    #[test]
    fn validate_refuses_empty_entry_in_mixed_batch() {
        // #5373: previously Ok — the empty entry came back as a
        // unit-norm embedding fabricated entirely from padding.
        let valid = vec![0u32, 42, 2];
        let err = validate_id_lists(&[valid.clone(), vec![], valid]).unwrap_err();
        assert!(matches!(err, Error::EmptyEntry { index: 1 }), "{err}");
    }

    #[test]
    fn validate_keeps_existing_controls() {
        assert!(matches!(
            validate_id_lists(&[]).unwrap_err(),
            Error::EmptyBatch
        ));
        let too_long = vec![0u32; MODEL_CONTEXT + 1];
        assert!(matches!(
            validate_id_lists(&[too_long]).unwrap_err(),
            Error::TooLong { tokens } if tokens == MODEL_CONTEXT + 1
        ));
        // A normal batch (including a max-context entry) validates.
        assert!(validate_id_lists(&[vec![0u32, 2], vec![7u32; MODEL_CONTEXT]]).is_ok());
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// PBT (#5373, testing doctrine): the validator accepts
        /// exactly the nonempty batches whose entry lengths are all
        /// in 1..=MODEL_CONTEXT — the oracle below restates that
        /// invariant directly (first offending entry, in batch
        /// order, decides the error and its payload).
        #[test]
        fn validate_matches_the_batch_invariant(
            lengths in prop::collection::vec(prop_oneof![
                Just(0usize),
                1usize..=16,
                Just(MODEL_CONTEXT),
                (MODEL_CONTEXT + 1)..=(MODEL_CONTEXT + 64),
            ], 0..8),
        ) {
            let batch: Vec<Vec<u32>> =
                lengths.iter().map(|&n| vec![7u32; n]).collect();
            // First offending entry in batch order decides: an
            // empty entry reports EmptyEntry, an over-context one
            // reports TooLong — mirroring the per-entry loop.
            let oracle: Result<()> = if batch.is_empty() {
                Err(Error::EmptyBatch)
            } else if let Some(index) =
                lengths.iter().position(|&n| n == 0 || n > MODEL_CONTEXT)
            {
                if lengths[index] == 0 {
                    Err(Error::EmptyEntry { index })
                } else {
                    Err(Error::TooLong { tokens: lengths[index] })
                }
            } else {
                Ok(())
            };
            match (validate_id_lists(&batch), oracle) {
                (Ok(()), Ok(())) => {}
                (Err(got), Err(want)) => prop_assert_eq!(
                    format!("{got:?}"), format!("{want:?}")
                ),
                (got, want) => prop_assert!(
                    false, "validate={got:?} oracle={want:?} lengths={lengths:?}"
                ),
            }
        }
    }

    #[test]
    fn pairwise_sum_matches_naive_for_short_inputs() {
        let a: Vec<f32> = (0..7).map(|i| (i as f32) * 0.25).collect();
        assert_eq!(pairwise_sum_f32(&a), a.iter().sum::<f32>());
    }

    #[test]
    fn pairwise_sum_handles_block_boundaries() {
        // Exercise all three branches (n<8, 8..=128, >128 recursion).
        for n in [8usize, 9, 127, 128, 129, 255, 768, 1000] {
            let a: Vec<f32> = (0..n).map(|i| ((i * 37 % 101) as f32) * 0.01).collect();
            let got = pairwise_sum_f32(&a);
            assert!(got.is_finite() && got > 0.0, "n={n} got={got}");
        }
    }
}
