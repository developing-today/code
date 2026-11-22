//! atlas-core — the Atlas corpus-search pipeline as a library
//! (developing-today/code#5352), a semantics-exact port of the
//! production Python `atlas_search.py`.
//!
//! Shape: a [`SearchBackend`] (Turso over the HTTPS pipeline
//! API, or a local SQLite publish file) gathers an FTS pool and
//! a vector pool with the caller's sensitivity ceiling and
//! filters applied **inside the branch queries** — before any
//! ranking — and [`run_search`] fuses the pools with reciprocal
//! rank fusion using the versioned per-query contract (defaults in
//! atlas-proto's query-parameters-v1.json).
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

pub mod aggregate;
pub mod backend;
pub mod classification;
pub mod detect;
pub mod embed;
pub mod embed_cache;
pub mod error;
pub mod eval;
pub mod feedback;
pub mod feedback_normalise;
pub mod filters;
pub mod fusion;
pub mod gate;
pub mod github_comments;
pub mod join;
pub mod local;
pub mod manifest;
pub mod owner_inventory;
pub mod search;
pub mod shards;
pub mod shingles;
pub mod stage;
pub mod sweep;
pub mod trace;
pub mod trace_ingest;
pub mod trace_stats;
pub mod turso;
pub mod types;

/// Maximum neighbor chunks search expansion may attach to each result.
/// Keep this aligned with the service's get-chunk neighbors ceiling.
pub const MAX_EXPAND_NEIGHBORS: usize = 100;

pub use aggregate::{PromoteReport, embed_unit_into_shard, tier_sensitivity_name};
pub use backend::SearchBackend;
pub use detect::{
    DETECTION_LEDGER_SCHEMA, DetectionLedger, DetectionPass, FileStat, LandedState, Mechanism,
    PathVerdict, PlainTreeOptions, SourceLedger, Verdict, VerdictCounts, default_ledger_path,
    git_tree_detection, plain_tree_detection, resolve_commit,
};
pub use embed::{
    AtlasEmbedder, DocEmbedder, FixedDocEmbedder, FixedEmbedder, QUERY_PROMPT, QueryEmbedder,
};
pub use embed_cache::{
    CachingEmbedder, QueryEmbeddingCache, RawQueryEmbedder, SharedEmbeddingStore,
    TursoEmbeddingStore, cache_key, normalize_query_text,
};
pub use error::{Error, Result};
pub use eval::{
    EVAL_FORMAT, EvalData, EvalReport, MAX_GRADE, Metric, REPORT_FORMAT, Split, evaluate,
    select_best_signal,
};
pub use feedback::{
    FEEDBACK_SCHEMA, FeedbackInput, FeedbackRecord, FeedbackSink, FeedbackView, JsonlFeedbackSink,
    MAX_COMMENT_BYTES, Vote, active_view, feedback_files, feedback_for, load_feedback_dir,
    load_feedback_records, record_feedback,
};
pub use feedback_normalise::{
    FEEDBACK_CATEGORIES, NORMALISED_SCHEMA, NormalisedFeedback, QueryRequesters, about_chunk,
    classify_comment, is_self_feedback, normalise_records, normalised_file_name, summarise,
    write_normalised,
};
pub use filters::{Filters, Sensitivity};
pub use fusion::FusedEntry;
pub use join::{
    Assembled, Hit, Paragraph, RenderRequest, RenderResult, Span, SpanOccurrence, SpanTable,
    Successor, VersionCatalog, VersionRecord, VersionState, assemble, assemble_version, render,
};
pub use local::LocalBackend;
pub use manifest::{
    LoadedManifest, Manifest, ManifestRow, ManifestSource, builtin_manifest, load_manifest,
    parse_manifest_rows,
};
pub use owner_inventory::{
    INVENTORY_FORMAT, OwnerInventory, OwnerInventoryFinding, PatternClass, SECRET_REFERENCE_ENV,
};
pub use search::{
    BackendBranchExecutor, BranchPoolExecutor, SearchMode, SearchOptions, TraceJob, run_get_chunk,
    run_search, run_search_traced, run_search_with_executor, run_search_with_executor_traced,
};
pub use stage::{
    REGISTRY_V1_DDL, Registry, STAGE_SCHEMA_VERSION, STAGE_V1_DDL, STAGE_V1_TABLES, StagingUnit,
    UnitState, check_stage_schema, provision_staging_db, schema_version,
};
pub use trace::{JsonlTraceSink, TRACE_SCHEMA, TraceContext, TraceSink};
pub use trace_ingest::{
    COLD_DIR_NAME, COMPACTION_SCHEMA, HOT_RETENTION_DAYS, RetentionReport, TRACE_SOURCE_CLASS,
    TRACE_SOURCE_ID, TraceCorpus, TraceDocument, TraceSourceRegistration, days_from_civil,
    ingest_traces, load_feedback_refs, record_included, run_retention, search_traces,
    trace_file_day, trace_source_registration, valid_civil_date,
};
pub use trace_stats::{
    DEFAULT_TOP_K, DEFAULT_WINDOWS, FeedbackStats, FeedbackVote, Nomination, QueryFeedbackStats,
    QueryFeedbackVote, REPORT_SCHEMA, REPORT_SOURCE_ID, STATS_SCHEMA, WeeklyReport, WindowStats,
    aggregate_traces, format_day, load_feedback_votes, load_query_feedback_votes,
    load_query_requesters, report_file_name, report_source_registration, requester_weight,
    weekly_report, write_report,
};
pub use turso::TursoBackend;
pub use types::{BranchQuery, DEFAULT_MODEL, DEFAULT_VEC_TABLE, VectorBackend};
