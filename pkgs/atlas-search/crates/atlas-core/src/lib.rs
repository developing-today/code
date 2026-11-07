//! atlas-core — the Atlas corpus-search pipeline as a library
//! (developing-today/code#5352), a semantics-exact port of the
//! production Python `atlas_search.py`.
//!
//! Shape: a [`SearchBackend`] (Turso over the HTTPS pipeline
//! API, or a local SQLite publish file) gathers an FTS pool and
//! a vector pool with the caller's sensitivity ceiling and
//! filters applied **inside the branch queries** — before any
//! ranking — and [`run_search`] fuses the pools with reciprocal
//! rank fusion (k = 10, pool 100/branch by default).
//!
//! Vector strategies all live behind the backend:
//! [`VectorBackend::Scan`] (exact `vector_distance_cos` over the
//! allowed set — the current production path and the default),
//! [`VectorBackend::IndexFill`] (ANN index, depth escalates
//! until the pool survives the filter), and
//! [`VectorBackend::IndexFixed`] (single fixed-depth fetch).
//! Which index strategy becomes the default is decided at the
//! #5356 cutover from A/B evidence, not here.
//!
//! Concurrency note for front-ends (steps 3–4): the library is
//! synchronous; backends are `Send + Sync` behind interior
//! locks. The query embedder is the exception — see
//! [`AtlasEmbedder`]'s docs: ONNX sessions take `&mut self`, so
//! serve mode holds ONE embedder (or a small pool) behind a
//! mutex; session load (~1.6 GB RSS) is shared state by design,
//! never per-request.

pub mod backend;
pub mod embed;
pub mod error;
pub mod eval;
pub mod filters;
pub mod fusion;
pub mod local;
pub mod search;
pub mod shards;
pub mod turso;
pub mod types;

pub use backend::SearchBackend;
pub use embed::{AtlasEmbedder, FixedEmbedder, QUERY_PROMPT, QueryEmbedder};
pub use error::{Error, Result};
pub use eval::{
    EVAL_FORMAT, EvalData, EvalReport, MAX_GRADE, Metric, REPORT_FORMAT, Split, evaluate,
    select_best_signal,
};
pub use filters::{Filters, Sensitivity};
pub use fusion::{FusedEntry, RRF_K};
pub use local::LocalBackend;
pub use search::{
    BackendBranchExecutor, BranchPoolExecutor, SearchMode, SearchOptions, run_get_chunk,
    run_search, run_search_with_executor,
};
pub use turso::TursoBackend;
pub use types::{BranchQuery, DEFAULT_MODEL, DEFAULT_VEC_TABLE, VectorBackend};
