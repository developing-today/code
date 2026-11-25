//! atlas-core — the Atlas corpus-search pipeline as a library
//! (developing-today/code#5352), a semantics-exact port of the
//! production Python `atlas_search.py` — exact except for the
//! intentional divergences named in #13280 (the topics/projects
//! LIKE escaping, #5375; the FTS digit-token retention in
//! `fusion::fts_match_expr`; and `index_depths` at pool 0), each
//! recorded in the Atlas book's python-implementation reference.
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
//! allowed set — the monolith backend's own strategy, and its
//! default), [`VectorBackend::IndexFill`] (ANN index, depth
//! escalates until the pool survives the filter), and
//! [`VectorBackend::IndexFixed`] (single fixed-depth fetch).
//! Which of the index strategies becomes the monolith backend's
//! default is decided at the #5356 cutover from A/B evidence, not
//! here. In the service composition the vector branch is already
//! served from the shard core:
//! [`ShardedBackend`](crate::shards::ShardedBackend) fans out over
//! the shards and delegates FTS, details and everything else to
//! the wrapped monolith backend (#13284).
//!
//! Concurrency note for front-ends (steps 3–4): the library is
//! synchronous; backends are `Send + Sync` behind interior
//! locks. The query embedder is the exception — see
//! [`AtlasEmbedder`]'s docs: ONNX sessions take `&mut self`, so
//! serve mode holds ONE embedder (or a small pool) behind a
//! mutex; session load (~1.6 GB RSS) is shared state by design,
//! never per-request.

pub mod aggregate;
pub mod attestation;
pub mod backend;
pub mod boundary;
pub mod c8_fingerprint;
pub mod c8_profile;
pub mod classification;
pub mod classify;
pub mod cold;
pub mod detect;
pub mod disclosure;
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
pub mod id_projection;
pub mod join;
pub mod local;
pub mod manifest;
pub mod memory;
pub mod occurrences;
pub mod owner_inventory;
pub mod search;
pub mod shards;
pub mod shingles;
pub mod source_registry;
pub mod stage;
pub mod sweep;
pub mod trace;
pub mod trace_ingest;
pub mod trace_stats;
pub mod turso;
pub mod types;
pub mod version_projection;

/// Maximum neighbor chunks search expansion may attach to each result.
/// Keep this aligned with the service's get-chunk neighbors ceiling.
pub const MAX_EXPAND_NEIGHBORS: usize = 100;

pub use aggregate::{PromoteReport, embed_unit_into_shard, tier_sensitivity_name};
pub use backend::SearchBackend;
pub use c8_fingerprint::{
    C8_FINGERPRINT_SCHEMA_VERSION, CandidateChange, EdgeLabel, ErrorFingerprint, ErrorOccurrence,
    EvidenceBundle, FINGERPRINT_ALGORITHM, FingerprintGroup, FirstObservation,
    FirstObservationKind, InvestigationEdge, InvestigationFindings, RejectedCandidate,
    UnpushedCommitCheck, UnpushedCommitStatus, build_bundle, build_bundle_with_findings,
    first_observation, group_occurrences, normalize_error_string,
};
pub use c8_profile::{
    C8Link, C8Page, C8Source, LinkEvidence, LinkKind, PageKind, UrlParts, extract_links,
    sanitize_url,
};
pub use classification::{
    ClassificationDisposition, ClassificationProvenance, ClassificationRecord,
    EffectiveClassification,
};
pub use detect::{
    DETECTION_LEDGER_SCHEMA, DetectionLedger, DetectionPass, FileStat, LandedState, LandedTable,
    Mechanism, PathVerdict, PlainTreeOptions, RowDetectOptions, RowMode, RowStoreSpec,
    SourceLedger, TablePass, TableSpec, Verdict, VerdictCounts, WatermarkSpec, WatermarkValue,
    default_ledger_path, git_tree_detection, plain_tree_detection, resolve_commit,
    row_store_detection,
};
pub use disclosure::{DisclosureContext, chunk_classification};
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
pub use occurrences::{ChunkOccurrence, lookup_by_hash};
pub use owner_inventory::{
    INVENTORY_FORMAT, OwnerInventory, OwnerInventoryFinding, PatternClass, SECRET_REFERENCE_ENV,
};
pub use search::{
    BackendBranchExecutor, BranchPoolExecutor, SearchMode, SearchOptions, TraceJob, run_get_chunk,
    run_get_chunk_with_context, run_search, run_search_traced, run_search_with_executor,
    run_search_with_executor_traced,
};
pub use source_registry::{
    LocatorRegistration, RootRegistration, SOURCE_REGISTRY_DDL, SourceResolution, SourceRoot,
    ensure_source_registry, locators_for_root, open_source_registry, register_locator,
    register_source_root, resolve_source, source_root,
};
pub use stage::{
    ChunkClassification, REGISTRY_V1_DDL, Registry, STAGE_SCHEMA_VERSION, STAGE_V1_DDL,
    STAGE_V1_TABLES, StagingUnit, UnitState, check_stage_schema, chunk_classifications,
    provision_staging_db, schema_version, stage_classified_artifact,
    stage_classified_artifact_in_db,
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

// ------------------------------------------------------------- home ----

/// Home-resolution contract (issues #5640 / #5762): a missing or
/// empty HOME is an error naming HOME, never a substitute
/// directory. Every atlas-core path rooted at the user's home goes
/// through here so the contract cannot drift per call site - the
/// trace and feedback sinks' `default_dir` once fell back to the
/// CWD (issue #13279), landing state files wherever the process
/// happened to run.
pub(crate) fn require_home() -> std::path::PathBuf {
    home_from(std::env::var_os("HOME"))
}

fn home_from(home: Option<std::ffi::OsString>) -> std::path::PathBuf {
    match home {
        Some(home) if !home.is_empty() => std::path::PathBuf::from(home),
        _ => panic!(
            "HOME is not set or is empty: atlas-core requires a non-empty HOME to resolve its state/config paths (issue #5640)"
        ),
    }
}

#[cfg(test)]
mod home_tests {
    use super::home_from;
    use std::ffi::OsString;
    use std::path::PathBuf;

    #[test]
    fn home_from_accepts_a_non_empty_home() {
        assert_eq!(
            home_from(Some(OsString::from("/home/someone"))),
            PathBuf::from("/home/someone")
        );
    }

    #[test]
    #[should_panic(expected = "HOME is not set or is empty")]
    fn home_from_rejects_a_missing_home() {
        home_from(None);
    }

    #[test]
    #[should_panic(expected = "HOME is not set or is empty")]
    fn home_from_rejects_an_empty_home() {
        home_from(Some(OsString::new()));
    }
}
