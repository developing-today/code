//! Query embedding at the library boundary (design sections 4/5).
//!
//! The Python's `embed_query` contract for the Snowflake model:
//! sentence-transformers applies the model's own query prompt
//! (`prompt_name="query"`), then truncation (Matryoshka) slices,
//! then the vector is L2-renormalized. atlas-embed deliberately
//! ships the prompt step to this layer: for
//! `snowflake-arctic-embed-m-v2.0` the shipped query prompt is the
//! string prefix `"query: "`, and the production corpus was embedded
//! prompt-free on the document side (documents never pass through
//! here).

use std::sync::Mutex;

use crate::error::{Error, Result};

/// The query prompt ST applies for the pinned model
/// (`config_sentence_transformers.json`: `prompts.query`).
pub const QUERY_PROMPT: &str = "query: ";

/// A query embedder: raw text in, production-shaped query vector
/// out (prompt applied by the implementation, truncated and
/// renormalized by the shared tail in this module).
pub trait QueryEmbedder {
    fn embed_query(&self, text: &str, trunc: usize) -> Result<Vec<f32>>;
}

/// The shared tail of the Python's `embed_query`: slice to `trunc`
/// dims if asked, then renormalize; a zero vector is fatal there and
/// here. The norm is NumPy's (pairwise f32 sum of squares), via
/// atlas-embed's replicated summation.
pub fn finish_query_vector(mut v: Vec<f32>, trunc: usize) -> Result<Vec<f32>> {
    if trunc > 0 {
        v.truncate(trunc);
    }
    let squares: Vec<f32> = v.iter().map(|x| x * x).collect();
    let n = atlas_embed::pairwise_sum_f32(&squares).sqrt();
    if n == 0.0 {
        return Err(Error::fatal("query embedding produced a zero vector"));
    }
    for x in v.iter_mut() {
        *x /= n;
    }
    Ok(v)
}

/// A fixed-vector embedder for tests and fixture work: stands in for
/// a real model exactly where the Python's fixture generator
/// monkeypatches `embed_query`. The prompt step is bypassed (the
/// recorded vector already represents a query embedding).
pub struct FixedEmbedder {
    pub raw: Vec<f32>,
}

impl QueryEmbedder for FixedEmbedder {
    fn embed_query(&self, _text: &str, trunc: usize) -> Result<Vec<f32>> {
        finish_query_vector(self.raw.clone(), trunc)
    }
}

/// The production query embedder over atlas-embed. The embedder's
/// `Session::run` takes `&mut self` (step 1 finding), so concurrent
/// callers serialize on a mutex: one resident session, queries
/// served one at a time. Serve mode (step 4) should hold exactly one
/// of these per process (or a small pool behind the same mutex
/// shape) rather than an embedder per request — session load is the
/// expensive, ~1.6 GB-RSS part, and it is shared state by design.
pub struct AtlasEmbedder {
    inner: Mutex<atlas_embed::Embedder>,
}

impl AtlasEmbedder {
    pub fn new(embedder: atlas_embed::Embedder) -> AtlasEmbedder {
        AtlasEmbedder {
            inner: Mutex::new(embedder),
        }
    }
}

impl QueryEmbedder for AtlasEmbedder {
    fn embed_query(&self, text: &str, trunc: usize) -> Result<Vec<f32>> {
        let prompted = format!("{QUERY_PROMPT}{text}");
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| Error::Embed("embedder mutex poisoned".to_string()))?;
        let out = guard
            .embed(&[prompted.as_str()])
            .map_err(|e| Error::Embed(e.to_string()))?;
        let v = out.into_iter().next().map(|a| a.to_vec()).ok_or_else(|| {
            Error::Embed("embedder returned no vector for a single input".to_string())
        })?;
        finish_query_vector(v, trunc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finish_normalizes_and_truncates() {
        let v = finish_query_vector(vec![3.0, 4.0], 0).unwrap();
        assert!((v[0] - 0.6).abs() < 1e-7 && (v[1] - 0.8).abs() < 1e-7);
        let v = finish_query_vector(vec![3.0, 4.0, 5.0], 2).unwrap();
        assert_eq!(v.len(), 2);
        assert!((v[0] - 0.6).abs() < 1e-7);
        assert!(finish_query_vector(vec![0.0, 0.0], 0).is_err());
    }

    #[test]
    fn fixed_embedder_roundtrip() {
        let e = FixedEmbedder {
            raw: vec![0.9, 0.3, 0.1],
        };
        let v = e.embed_query("anything", 0).unwrap();
        let n = (0.9f32 * 0.9 + 0.3 * 0.3 + 0.1 * 0.1).sqrt();
        assert!((v[0] - 0.9 / n).abs() < 1e-6);
    }
}
