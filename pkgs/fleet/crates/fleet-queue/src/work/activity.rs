//! Resumable durable page ingestion — HYD-05 — plus per-source
//! incremental activity revisions and tombstones — HYD-06
//! (developing-today/code#9207, program #9196).
//!
//! This module is the page-iterator/cursor slice of the PM
//! orchestration design (`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/components/9207-context-hydration-incremental-activity.md`,
//! §4 and the crash table): GitHub `Link` continuation parsing,
//! per-source page cursors, atomic page-ingest/cursor-commit
//! semantics, documented API caps, query/API fingerprinting, and the
//! revision/tombstone observation layer (design §4 rows "Old comment
//! edited after creation cursor advanced" and "Comment
//! deleted/minimized").
//! It is pure types and validation: no I/O, no queue, no storage —
//! the `turso_queue.rs` transaction integration named by the slice
//! is explicitly deferred until storage approval, so a rejection is
//! a typed [`ContractError`] that mutates nothing.
//!
//! Oracle (reference model): the component's own transaction table —
//! page data and cursor advance together; a page fetch without
//! durable ingest does not advance; crash before commit means refetch
//! the page; the unique event key makes repeats idempotent; unknown
//! cursor forces a full snapshot and legacy `last_checked_at` is
//! never trusted. For revisions: ascending creation-ID order never
//! hides edits — a newer revision of an old native ID observed
//! behind the creation watermark is an edit, never nothing and never
//! a second creation; disappearance without a tombstone is a
//! reported visibility gap, never an inferred delete. The tests below
//! compare expected and observed committed sequences explicitly. No
//! mock-only claim of deployed/live behavior.

use serde::{Deserialize, Serialize};

use super::contracts::ContractError;

/// GitHub PR commit-listing maximum reported by the API reference
/// (design §4). Reaching it without an alternate complete method is
/// `truncated`, never `complete`.
pub const GITHUB_PR_COMMIT_CAP: usize = 250;

/// GitHub PR changed-file-listing maximum reported by the API
/// reference (design §4). Same truncation rule as commits.
pub const GITHUB_PR_FILE_CAP: usize = 3000;

/// Resumable position of one paginated source fetch.
///
/// Identity is `(source_ref, endpoint, query_fingerprint,
/// api_version, generation)` per the design: the opaque
/// continuation is stored, never a bare work-level
/// `last_checked_at`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageCursor {
    /// Canonical source identity (e.g. `github://owner/repo/...`).
    pub source_ref: String,
    /// Endpoint path that produced the pages (e.g. `issues/comments`).
    pub endpoint: String,
    /// Stable digest of the query that produced the pages.
    pub query_fingerprint: String,
    /// API version the pages were fetched under.
    pub api_version: String,
    /// Cursor generation: bumped when the query/API version changes
    /// so an opaque token is never reused across incompatible queries.
    pub generation: u64,
    /// Unique event key of the last durably committed item, if any.
    pub last_event_key: Option<String>,
    /// Opaque continuation (`Link: rel="next"` URL) for the next page.
    pub next_continuation: Option<String>,
    /// Count of durably committed pages.
    pub pages_committed: u64,
}

impl PageCursor {
    /// A fresh cursor that has committed nothing. The first fetch is
    /// always a full snapshot.
    pub fn initial(
        source_ref: impl Into<String>,
        endpoint: impl Into<String>,
        query_fingerprint: impl Into<String>,
        api_version: impl Into<String>,
    ) -> Result<Self, ContractError> {
        let source_ref = source_ref.into();
        let endpoint = endpoint.into();
        let query_fingerprint = query_fingerprint.into();
        let api_version = api_version.into();
        for (field, value) in [
            ("source_ref", &source_ref),
            ("endpoint", &endpoint),
            ("query_fingerprint", &query_fingerprint),
            ("api_version", &api_version),
        ] {
            if value.is_empty() {
                return Err(ContractError::InvalidSchema {
                    detail: format!("page cursor has empty {field}"),
                });
            }
        }
        Ok(PageCursor {
            source_ref,
            endpoint,
            query_fingerprint,
            api_version,
            generation: 1,
            last_event_key: None,
            next_continuation: None,
            pages_committed: 0,
        })
    }
}

/// Whether a fetch must start over as a full snapshot.
///
/// Unknown cursor forces a full snapshot; a query-fingerprint or API
/// version change starts a distinct cursor generation instead of
/// reusing the opaque token. Legacy `last_checked_at` is never
/// trusted, so it is not an input here at all.
pub fn snapshot_needed(
    committed: Option<&PageCursor>,
    query_fingerprint: &str,
    api_version: &str,
) -> bool {
    match committed {
        None => true,
        Some(cursor) => {
            cursor.query_fingerprint != query_fingerprint || cursor.api_version != api_version
        }
    }
}

/// Parse the GitHub `Link` header continuation authority: return the
/// `rel="next"` URL, or `None` when absent or malformed. A missing or
/// unparseable header ends pagination; it is never an error that
/// silently drops pages — the caller treats `None` as "no further
/// page", and truncation accounting stays explicit.
pub fn parse_link_next(header: &str) -> Option<String> {
    for segment in header.split(',') {
        let segment = segment.trim();
        let (url_part, rel_part) = segment.split_once(';')?;
        let url = url_part.trim().strip_prefix('<')?.strip_suffix('>')?;
        if rel_part.trim() == r#"rel="next""# && !url.is_empty() {
            return Some(url.to_string());
        }
    }
    None
}

/// One fetched page: items plus its continuation state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedPage {
    /// Items in arrival order; each carries a unique event key.
    pub items: Vec<PageItem>,
    /// Parsed `rel="next"` continuation, if the endpoint offered one.
    pub link_next: Option<String>,
    /// True when an endpoint cap or budget stopped the listing.
    pub truncated: bool,
    /// Machine-readable reason when `truncated` (e.g. `commit_cap`).
    pub truncation_reason: Option<String>,
}

/// One page item keyed for idempotent ingest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageItem {
    /// Unique event key (source ID + revision). Repeats dedupe.
    pub event_key: String,
    /// Opaque payload reference (never interpreted here).
    pub payload: String,
}

/// Outcome of ingesting one page against the committed cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestOutcome {
    /// The cursor to commit when the page is durably stored, or
    /// `None` when durability failed: the committed cursor stays and
    /// the page must be refetched.
    pub advanced: Option<PageCursor>,
    /// Items newly committed by this ingest (repeats excluded).
    pub durable_items: Vec<PageItem>,
}

/// Ingest one page: page data and cursor advance together, or not at
/// all.
///
/// * `durable_store_ok = false` models a crash between fetch and
///   commit (or object-store success with DB failure): the returned
///   cursor is `None`, nothing is committed, and refetching the same
///   page later commits each item exactly once.
/// * `durable_store_ok = true` commits the page: items whose event
///   key is already in `seen_event_keys` are idempotent repeats and
///   excluded from `durable_items`; the advanced cursor records the
///   last item key and the page's continuation.
pub fn ingest_page(
    committed: &PageCursor,
    seen_event_keys: &[String],
    page: &FetchedPage,
    durable_store_ok: bool,
) -> IngestOutcome {
    if !durable_store_ok {
        return IngestOutcome {
            advanced: None,
            durable_items: Vec::new(),
        };
    }
    let durable_items: Vec<PageItem> = page
        .items
        .iter()
        .filter(|item| !seen_event_keys.contains(&item.event_key))
        .cloned()
        .collect();
    let mut advanced = committed.clone();
    if let Some(last) = page.items.last() {
        advanced.last_event_key = Some(last.event_key.clone());
    }
    advanced.next_continuation.clone_from(&page.link_next);
    advanced.pages_committed += 1;
    IngestOutcome {
        advanced: Some(advanced),
        durable_items,
    }
}

/// Cap accounting for a listing that fetched `fetched` entries
/// against an endpoint maximum of `cap`. Returns `(truncated,
/// reason)`: reaching the cap without an alternate complete method
/// is `truncated` with the endpoint's reason code.
pub fn cap_status(fetched: usize, cap: usize, reason: &'static str) -> (bool, Option<String>) {
    if fetched >= cap {
        (true, Some(reason.to_string()))
    } else {
        (false, None)
    }
}

/// Presence of one observed revision. Absence from a listing is
/// never a state — it is a visibility gap (see
/// [`RefreshOutcome::visibility_gaps`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActivityState {
    /// A live item the source still reports.
    Present,
    /// A minimized item the API still reports (e.g. minimized
    /// comment): retained as an observation, never dropped.
    Minimized,
    /// A tombstone the authoritative source/event provided (e.g.
    /// deleted comment): retained, never inferred from absence.
    Deleted,
}

/// Normalized observation of one source-native activity item at one
/// revision — HYD-06 (design §4: "Activity identity separates
/// occurrence and edits").
///
/// Identity is `(native_id, revision)`: the source-native event or
/// comment ID separates occurrence from edits, while the revision
/// (source revision or `updated_at`) plus the content digest detects
/// edits to old IDs that ascending creation order would miss. The
/// three clocks stay distinguished: `occurred_at` is the source's
/// creation time (stable across edits), `observed_at` is when this
/// revision was seen, and `source_updated_at` is the source's own
/// update time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityRecord {
    /// Source-native event/comment ID (occurrence identity).
    pub native_id: String,
    /// Source revision or `updated_at` (edit identity).
    pub revision: String,
    /// Current content digest.
    pub content_digest: String,
    /// Source creation time; stable across edits of the same item.
    pub occurred_at: String,
    /// When this revision was observed locally.
    pub observed_at: String,
    /// The source's own update time for this revision.
    pub source_updated_at: String,
    /// Presence of this observation.
    pub state: ActivityState,
}

impl ActivityRecord {
    /// Build one observation; empty identity is a typed rejection,
    /// never a silently accepted blank record.
    pub fn new(
        native_id: impl Into<String>,
        revision: impl Into<String>,
        content_digest: impl Into<String>,
        occurred_at: impl Into<String>,
        observed_at: impl Into<String>,
        source_updated_at: impl Into<String>,
        state: ActivityState,
    ) -> Result<Self, ContractError> {
        let native_id = native_id.into();
        let revision = revision.into();
        if native_id.is_empty() {
            return Err(ContractError::InvalidSchema {
                detail: "activity record has empty native_id".to_string(),
            });
        }
        if revision.is_empty() {
            return Err(ContractError::InvalidSchema {
                detail: "activity record has empty revision".to_string(),
            });
        }
        Ok(ActivityRecord {
            native_id,
            revision,
            content_digest: content_digest.into(),
            occurred_at: occurred_at.into(),
            observed_at: observed_at.into(),
            source_updated_at: source_updated_at.into(),
            state,
        })
    }

    /// Dedupe key: repeats of a committed `(native_id, revision)`
    /// pair are idempotent; the same native ID under a new revision
    /// is an edit, never a second creation.
    pub fn revision_key(&self) -> (String, String) {
        (self.native_id.clone(), self.revision.clone())
    }
}

/// Outcome of refreshing committed activity against a refetch —
/// HYD-06 (design §4 rows "Old comment edited after creation cursor
/// advanced" and "Comment deleted/minimized").
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RefreshOutcome {
    /// Native IDs never committed before, first seen `Present`.
    pub creations: Vec<ActivityRecord>,
    /// Committed native IDs whose revision, digest, or state moved
    /// (includes edits observed behind the creation watermark).
    pub edits: Vec<ActivityRecord>,
    /// Newly observed minimized/deleted states for committed IDs,
    /// plus first observations already minimized/deleted.
    pub tombstones: Vec<ActivityRecord>,
    /// Committed native IDs whose revision, digest, and state are
    /// all unchanged — however many times re-observed.
    pub unchanged_ids: Vec<String>,
    /// Committed IDs absent from the refetch with no tombstone:
    /// reported, never inferred as deleted (design §4: "absent API
    /// evidence cannot be inferred from disappearance in a paginated
    /// list").
    pub visibility_gaps: Vec<String>,
}

/// Refresh committed activity against one refetch.
///
/// * A refetch carrying a newer revision (or digest, or state) of a
///   committed native ID is an edit — even when that ID sits behind
///   the creation watermark. Creation order never hides edits.
/// * A refetch minimized/deleted state for a committed `Present`
///   item — or a first observation already minimized/deleted — is a
///   retained tombstone, not a creation and not a drop.
/// * Equal revision + equal digest + equal state is unchanged (the
///   equal-`updated_at` negative case): re-observation alone changes
///   nothing.
/// * A committed ID absent from the refetch with no tombstone is a
///   visibility gap, never an inferred delete.
/// * Duplicate `(native_id, revision)` pairs inside one refetch
///   report once (first occurrence wins); order is refetch order.
pub fn refresh_activity(
    committed: &[ActivityRecord],
    refetch: &[ActivityRecord],
) -> RefreshOutcome {
    use std::collections::{HashMap, HashSet};
    let committed_by_id: HashMap<&str, &ActivityRecord> = committed
        .iter()
        .map(|record| (record.native_id.as_str(), record))
        .collect();
    let mut seen_pairs: HashSet<(String, String)> = HashSet::new();
    let mut seen_ids: HashSet<&str> = HashSet::new();
    let mut outcome = RefreshOutcome::default();
    for record in refetch {
        if !seen_pairs.insert((record.native_id.clone(), record.revision.clone())) {
            continue;
        }
        seen_ids.insert(record.native_id.as_str());
        match committed_by_id.get(record.native_id.as_str()) {
            None => {
                if record.state == ActivityState::Present {
                    outcome.creations.push(record.clone());
                } else {
                    outcome.tombstones.push(record.clone());
                }
            }
            Some(previous) => {
                if record.state != ActivityState::Present
                    && previous.state == ActivityState::Present
                {
                    outcome.tombstones.push(record.clone());
                } else if record.revision != previous.revision
                    || record.content_digest != previous.content_digest
                    || record.state != previous.state
                {
                    outcome.edits.push(record.clone());
                } else {
                    outcome.unchanged_ids.push(record.native_id.clone());
                }
            }
        }
    }
    for record in committed {
        if !seen_ids.contains(record.native_id.as_str()) {
            outcome.visibility_gaps.push(record.native_id.clone());
        }
    }
    outcome
}

/// Drop already-committed revisions from a redelivery — HYD-06
/// (design §4: "Dedup webhook delivery ID separately from resource
/// revision").
///
/// The dedupe key is the `(native_id, revision)` pair, not the
/// creation: reordered duplicate redelivery of committed revisions
/// yields nothing, while the same native ID under a new revision
/// passes through exactly once as a change. Order is delivery order;
/// duplicates inside one delivery report once.
pub fn dedupe_replay(
    seen_revision_keys: &[(String, String)],
    delivery: &[ActivityRecord],
) -> Vec<ActivityRecord> {
    use std::collections::HashSet;
    let seen: HashSet<(String, String)> = seen_revision_keys.iter().cloned().collect();
    let mut emitted: HashSet<(String, String)> = HashSet::new();
    delivery
        .iter()
        .filter(|record| {
            let key = (record.native_id.clone(), record.revision.clone());
            !seen.contains(&key) && emitted.insert(key)
        })
        .cloned()
        .collect()
}

/// Bounded reconcile planning — HYD-08 (child #9510; design §4
/// crash table and §8: bounded `refresh` / `activity reconcile`
/// behavior mapped to shared contracts).
///
/// The planner turns per-source refresh state into one bounded pass
/// plan: each source is `UpToDate`, resumed from its continuation,
/// or restarted as a full snapshot — and fetch work beyond
/// `max_sources_per_pass` is explicitly deferred, never silently
/// dropped and never folded into an unbounded pass. Input order is
/// the caller's priority order: the first fetch actions win the
/// budget and the rest wait for the next pass.
///
/// What this is not: no fetching, no cursor commits, no CLI. Those
/// are later HYD-08 slices; this is the decision model they share.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotReason {
    /// No committed cursor: the first fetch is always a snapshot.
    NoCursor,
    /// Query fingerprint or API version moved: the opaque token is
    /// never reused across incompatible queries — a new generation
    /// starts from a snapshot.
    QueryChanged,
    /// Committed cursor is current but the last pass left work
    /// (visibility gaps or tombstone follow-ups) with no
    /// continuation pending: re-list from a snapshot.
    FollowUp,
}

/// One source's refresh input for reconcile planning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRefreshState {
    /// Canonical source identity (matches [`PageCursor::source_ref`]).
    pub source_ref: String,
    /// Last committed cursor, if the source was ever fetched.
    pub committed: Option<PageCursor>,
    /// Query fingerprint the next fetch would use.
    pub query_fingerprint: String,
    /// API version the next fetch would use.
    pub api_version: String,
    /// True when the last pass left work (visibility gaps or
    /// tombstone follow-ups) even though the cursor is committed.
    pub needs_refetch: bool,
}

impl SourceRefreshState {
    /// Build one source's refresh input; empty identity is a typed
    /// rejection, never a silently accepted blank source.
    pub fn new(
        source_ref: impl Into<String>,
        committed: Option<PageCursor>,
        query_fingerprint: impl Into<String>,
        api_version: impl Into<String>,
        needs_refetch: bool,
    ) -> Result<Self, ContractError> {
        let source_ref = source_ref.into();
        let query_fingerprint = query_fingerprint.into();
        let api_version = api_version.into();
        for (field, value) in [
            ("source_ref", &source_ref),
            ("query_fingerprint", &query_fingerprint),
            ("api_version", &api_version),
        ] {
            if value.is_empty() {
                return Err(ContractError::InvalidSchema {
                    detail: format!("source refresh state has empty {field}"),
                });
            }
        }
        Ok(SourceRefreshState {
            source_ref,
            committed,
            query_fingerprint,
            api_version,
            needs_refetch,
        })
    }
}

/// Planned refresh action for one source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshAction {
    /// Cursor is current and nothing is pending: no fetch. Never
    /// counts against the pass budget.
    UpToDate { source_ref: String },
    /// Resume listing from the committed continuation.
    Resume {
        source_ref: String,
        continuation: String,
    },
    /// Restart the listing as a full snapshot.
    FullSnapshot {
        source_ref: String,
        reason: SnapshotReason,
    },
}

/// One bounded reconcile pass over all sources.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshPlan {
    /// Decided actions in caller priority order. Sources beyond the
    /// budget have no action here — see `deferred`.
    pub actions: Vec<RefreshAction>,
    /// Source identities postponed for lack of budget, in priority
    /// order. Explicit, never silently dropped.
    pub deferred: Vec<String>,
    /// The budget this plan was built under.
    pub budget: usize,
}

/// Plan one bounded refresh pass.
///
/// * A source whose cursor is committed, current (fingerprint and
///   API version match), uncontinued, and unflagged is `UpToDate`.
/// * A source with no cursor, or a moved fingerprint/version, is a
///   `FullSnapshot` — the opaque token is never reused across
///   incompatible queries.
/// * A current cursor with no continuation but `needs_refetch` is a
///   `FullSnapshot` for follow-up; with a continuation it resumes.
/// * Fetch actions (resume and snapshot) consume the pass budget in
///   caller priority order; sources beyond `max_sources_per_pass`
///   are `deferred`, never silently skipped.
/// * A zero budget is a typed rejection: a pass that can fetch
///   nothing would silently mark everything complete.
pub fn plan_refresh(
    sources: &[SourceRefreshState],
    max_sources_per_pass: usize,
) -> Result<RefreshPlan, ContractError> {
    if max_sources_per_pass == 0 {
        return Err(ContractError::InvalidSchema {
            detail: "refresh pass budget is zero: nothing could be fetched".to_string(),
        });
    }
    let mut actions = Vec::with_capacity(sources.len());
    let mut deferred = Vec::new();
    let mut fetches_planned = 0usize;
    for source in sources {
        if source.source_ref.is_empty()
            || source.query_fingerprint.is_empty()
            || source.api_version.is_empty()
        {
            return Err(ContractError::InvalidSchema {
                detail: "source refresh state has empty identity field".to_string(),
            });
        }
        let action = match &source.committed {
            None => RefreshAction::FullSnapshot {
                source_ref: source.source_ref.clone(),
                reason: SnapshotReason::NoCursor,
            },
            Some(cursor)
                if cursor.query_fingerprint != source.query_fingerprint
                    || cursor.api_version != source.api_version =>
            {
                RefreshAction::FullSnapshot {
                    source_ref: source.source_ref.clone(),
                    reason: SnapshotReason::QueryChanged,
                }
            }
            Some(cursor) => match &cursor.next_continuation {
                Some(continuation) => RefreshAction::Resume {
                    source_ref: source.source_ref.clone(),
                    continuation: continuation.clone(),
                },
                None if source.needs_refetch => RefreshAction::FullSnapshot {
                    source_ref: source.source_ref.clone(),
                    reason: SnapshotReason::FollowUp,
                },
                None => RefreshAction::UpToDate {
                    source_ref: source.source_ref.clone(),
                },
            },
        };
        let is_fetch = !matches!(action, RefreshAction::UpToDate { .. });
        if is_fetch {
            if fetches_planned >= max_sources_per_pass {
                deferred.push(source.source_ref.clone());
                continue;
            }
            fetches_planned += 1;
        }
        actions.push(action);
    }
    Ok(RefreshPlan {
        actions,
        deferred,
        budget: max_sources_per_pass,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cursor() -> PageCursor {
        PageCursor::initial(
            "github://alpha/repo/issues/10",
            "issues/comments",
            "qfp1",
            "v3",
        )
        .unwrap()
    }

    fn page(keys: &[&str], link_next: Option<&str>) -> FetchedPage {
        FetchedPage {
            items: keys
                .iter()
                .map(|k| PageItem {
                    event_key: k.to_string(),
                    payload: format!("payload:{k}"),
                })
                .collect(),
            link_next: link_next.map(str::to_string),
            truncated: false,
            truncation_reason: None,
        }
    }

    /// HYD-05 oracle: page data and cursor commit atomically —
    /// crashes before/after the page-2 ingest skip nothing, repeat
    /// nothing, and advance only on durability.
    #[test]
    fn pmo_hyd_page_ingest_and_cursor_commit_atomically() {
        // Page 1 commits: cursor advances, both items durable.
        let committed = cursor();
        let page1 = page(&["k1", "k2"], Some("https://api/p2"));
        let out1 = ingest_page(&committed, &[], &page1, true);
        let c1 = out1.advanced.expect("durable ingest advances");
        assert_eq!(
            out1.durable_items
                .iter()
                .map(|i| &i.event_key)
                .collect::<Vec<_>>(),
            ["k1", "k2"]
        );
        assert_eq!(c1.pages_committed, 1);
        assert_eq!(c1.last_event_key.as_deref(), Some("k2"));

        // Crash before page-2 ingest (fetch ok, commit failed):
        // nothing advances, nothing is committed.
        let page2 = page(&["k3", "k4"], None);
        let crashed = ingest_page(&c1, &["k1".into(), "k2".into()], &page2, false);
        assert_eq!(crashed.advanced, None);
        assert!(crashed.durable_items.is_empty());

        // Refetch page 2 after the crash: both items commit exactly
        // once — expected sequence has no skipped item.
        let out2 = ingest_page(&c1, &["k1".into(), "k2".into()], &page2, true);
        let c2 = out2.advanced.expect("refetch after crash advances");
        assert_eq!(
            out2.durable_items
                .iter()
                .map(|i| &i.event_key)
                .collect::<Vec<_>>(),
            ["k3", "k4"]
        );
        assert_eq!(c2.pages_committed, 2);

        // Reordered duplicate redelivery of page 2: idempotent, the
        // cursor is unchanged and no item commits twice.
        let replay = page(&["k4", "k3"], None);
        let seen = ["k1", "k2", "k3", "k4"]
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
        let out3 = ingest_page(&c2, &seen, &replay, true);
        let c3 = out3
            .advanced
            .expect("durable replay still advances page count");
        assert!(out3.durable_items.is_empty());
        assert_eq!(c3.last_event_key.as_deref(), Some("k3"));

        // Committed event stream across crash + replay: every
        // revision appears at most once, none skipped.
        let mut committed_keys = vec!["k1", "k2"];
        committed_keys.extend(out2.durable_items.iter().map(|i| i.event_key.as_str()));
        assert_eq!(committed_keys, ["k1", "k2", "k3", "k4"]);

        // `Link` continuation is the paging authority: next URL
        // extracted, absent header ends pagination, malformed never
        // fabricates a continuation.
        assert_eq!(
            parse_link_next(r#"<https://api/p2>; rel="next", <https://api/p9>; rel="last""#),
            Some("https://api/p2".to_string())
        );
        assert_eq!(parse_link_next(""), None);
        assert_eq!(parse_link_next(r#"<https://api/p2>; rel="last""#), None);
        assert_eq!(parse_link_next("not a link header;;;"), None);

        // Documented endpoint caps surface as explicit truncation.
        assert_eq!(
            cap_status(250, GITHUB_PR_COMMIT_CAP, "commit_cap"),
            (true, Some("commit_cap".to_string()))
        );
        assert_eq!(
            cap_status(3000, GITHUB_PR_FILE_CAP, "file_cap"),
            (true, Some("file_cap".to_string()))
        );
        assert_eq!(
            cap_status(30, GITHUB_PR_COMMIT_CAP, "commit_cap"),
            (false, None)
        );

        // Cursor identity: unknown cursor forces a full snapshot and
        // a query/API change starts a new generation instead of
        // reusing the opaque token.
        assert!(snapshot_needed(None, "qfp1", "v3"));
        assert!(!snapshot_needed(Some(&c2), "qfp1", "v3"));
        assert!(snapshot_needed(Some(&c2), "qfp2", "v3"));
        assert!(snapshot_needed(Some(&c2), "qfp1", "v4"));
    }

    fn record(
        native_id: &str,
        revision: &str,
        digest: &str,
        occurred_at: &str,
        observed_at: &str,
        source_updated_at: &str,
        state: ActivityState,
    ) -> ActivityRecord {
        ActivityRecord::new(
            native_id,
            revision,
            digest,
            occurred_at,
            observed_at,
            source_updated_at,
            state,
        )
        .expect("valid activity record")
    }

    /// HYD-06 oracle (design §4, rows "Old comment edited after
    /// creation cursor advanced" and "Comment deleted/minimized"):
    /// ascending creation-ID order never hides edits — a refetch
    /// that carries a newer revision of an old native ID behind the
    /// creation watermark is an edit, not nothing; disappearance
    /// without a tombstone is a reported visibility gap, never an
    /// inferred delete.
    #[test]
    fn pmo_hyd_refresh_observes_edit_behind_creation_cursor() {
        // Committed store: c5 r1 and c9 r1. The creation watermark
        // has advanced past c5 (max observed creation is c9).
        let committed = vec![
            record(
                "c5",
                "r1",
                "dig-old",
                "t0",
                "o0",
                "u1",
                ActivityState::Present,
            ),
            record(
                "c9",
                "r1",
                "dig-9",
                "t9",
                "o9",
                "u9",
                ActivityState::Present,
            ),
            record(
                "c8",
                "r1",
                "dig-8",
                "t8",
                "o8",
                "u8",
                ActivityState::Present,
            ),
        ];
        // Refetch: c5 edited behind the cursor (same native ID, new
        // revision + digest + later source time); c9 byte-identical
        // but re-observed (equal updated_at must stay unchanged);
        // c10 brand new; c7 minimized tombstone newly observed.
        // c8 is absent with no tombstone anywhere in the refetch.
        let refetch = vec![
            record(
                "c5",
                "r2",
                "dig-new",
                "t0",
                "o1",
                "u2",
                ActivityState::Present,
            ),
            record(
                "c9",
                "r1",
                "dig-9",
                "t9",
                "o10",
                "u9",
                ActivityState::Present,
            ),
            record(
                "c10",
                "r1",
                "dig-10",
                "t10",
                "o10",
                "u10",
                ActivityState::Present,
            ),
            record(
                "c7",
                "r1",
                "dig-7",
                "t7",
                "o10",
                "u7",
                ActivityState::Minimized,
            ),
        ];
        let outcome = refresh_activity(&committed, &refetch);

        // The behind-cursor edit is observed as exactly one edit.
        assert_eq!(
            outcome
                .edits
                .iter()
                .map(|r| &r.native_id)
                .collect::<Vec<_>>(),
            ["c5"]
        );
        assert_eq!(outcome.edits[0].revision, "r2");
        assert_eq!(outcome.edits[0].content_digest, "dig-new");
        // Occurrence time is stable across the edit; observation and
        // source-update times move forward — the three clocks stay
        // distinguished.
        assert_eq!(outcome.edits[0].occurred_at, "t0");
        assert_eq!(outcome.edits[0].observed_at, "o1");
        assert_eq!(outcome.edits[0].source_updated_at, "u2");

        // Only the genuinely new native ID is a creation: the edit
        // behind the cursor is never double-counted as one.
        assert_eq!(
            outcome
                .creations
                .iter()
                .map(|r| &r.native_id)
                .collect::<Vec<_>>(),
            ["c10"]
        );

        // Equal updated_at + equal digest + equal revision is
        // unchanged, however many times it is re-observed.
        assert_eq!(outcome.unchanged_ids, ["c9"]);

        // The minimized record is retained as a tombstone
        // observation, not dropped and not a creation.
        assert_eq!(
            outcome
                .tombstones
                .iter()
                .map(|r| &r.native_id)
                .collect::<Vec<_>>(),
            ["c7"]
        );

        // c8 vanished with no tombstone: reported gap, never an
        // inferred delete.
        assert_eq!(outcome.visibility_gaps, ["c8"]);
        assert!(!outcome.creations.iter().any(|r| r.native_id == "c8"));
        assert!(!outcome.edits.iter().any(|r| r.native_id == "c8"));
        assert!(!outcome.tombstones.iter().any(|r| r.native_id == "c8"));

        // Empty identity is rejected, never silently accepted.
        assert!(ActivityRecord::new("", "r1", "d", "t", "o", "u", ActivityState::Present).is_err());
    }

    /// HYD-06 oracle (design §4: "Dedup webhook delivery ID
    /// separately from resource revision"): redelivery of an already
    /// committed (native ID, revision) pair commits nothing, while
    /// the same native ID under a new revision is one edit — the
    /// dedupe key is the revision, not the creation.
    #[test]
    fn pmo_hyd_replay_deduplicates_revision_not_creation() {
        let seen: Vec<(String, String)> = vec![
            ("c5".to_string(), "r1".to_string()),
            ("c5".to_string(), "r2".to_string()),
        ];
        // Reordered duplicate redelivery of committed revisions:
        // nothing new, in any order.
        let replay = vec![
            record(
                "c5",
                "r2",
                "dig-new",
                "t0",
                "o2",
                "u2",
                ActivityState::Present,
            ),
            record(
                "c5",
                "r1",
                "dig-old",
                "t0",
                "o2",
                "u1",
                ActivityState::Present,
            ),
        ];
        assert!(dedupe_replay(&seen, &replay).is_empty());

        // Same native ID, new revision: exactly one change, and it
        // is an edit of c5 — never a second creation of it.
        let edit = vec![record(
            "c5",
            "r3",
            "dig-v3",
            "t0",
            "o3",
            "u3",
            ActivityState::Present,
        )];
        let fresh = dedupe_replay(&seen, &edit);
        assert_eq!(fresh.len(), 1);
        assert_eq!(fresh[0].native_id, "c5");
        assert_eq!(fresh[0].revision, "r3");

        // Committed key stream across replay + edit: every
        // (native ID, revision) pair appears at most once.
        let mut keys: Vec<(String, String)> = seen.clone();
        keys.extend(
            fresh
                .iter()
                .map(|r| (r.native_id.clone(), r.revision.clone())),
        );
        let mut sorted = keys.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(keys.len(), sorted.len());
        assert_eq!(
            keys,
            [
                ("c5".to_string(), "r1".to_string()),
                ("c5".to_string(), "r2".to_string()),
                ("c5".to_string(), "r3".to_string()),
            ]
        );

        // Full-refresh view agrees: the new revision lands in edits,
        // creations stay empty, and a straight repeat lands nowhere.
        let committed = vec![record(
            "c5",
            "r2",
            "dig-new",
            "t0",
            "o2",
            "u2",
            ActivityState::Present,
        )];
        let again = refresh_activity(
            &committed,
            &[record(
                "c5",
                "r2",
                "dig-new",
                "t0",
                "o9",
                "u2",
                ActivityState::Present,
            )],
        );
        assert!(again.creations.is_empty());
        assert!(again.edits.is_empty());
        assert_eq!(again.unchanged_ids, ["c5"]);
    }

    fn observed(
        native_id: &str,
        actor: Option<&str>,
        remote: Option<&str>,
        op: Option<&str>,
        session: Option<&str>,
        outbox: Option<&str>,
    ) -> ObservedActivity {
        ObservedActivity::new(
            native_id,
            actor.map(str::to_string),
            remote.map(str::to_string),
            op.map(str::to_string),
            session.map(str::to_string),
            outbox.map(str::to_string),
        )
        .expect("valid observed activity")
    }

    /// HYD-07 oracle (design §8: "this test uses same bot login from
    /// two sessions with only one op/remote-ID map"): shared actor
    /// login never attributes by itself; only a full
    /// operation-ID-plus-remote-ID mapping is `Fleet`; `Uncertain`
    /// and `External` stay visible under `--external-only`.
    #[test]
    fn pmo_hyd_external_only_includes_uncertain_shared_actor() {
        let fleet_actors = ["fleet-bot"];
        // One durable mapping: op-A in session-A produced remote r1.
        let mappings = vec![
            OperationMapping::new("op-A", "session-A", Some("out-1".to_string()), vec![
                "r1".to_string(),
            ])
            .expect("valid mapping"),
        ];

        // Same bot login, full mapping: reliably Fleet, excluded
        // from external-only.
        let fleet_item = observed(
            "c1",
            Some("fleet-bot"),
            Some("r1"),
            Some("op-A"),
            Some("session-A"),
            Some("out-1"),
        );
        assert_eq!(
            classify_origin(&mappings, &fleet_actors, &fleet_item),
            ActivityOrigin::Fleet {
                operation_id: "op-A".to_string(),
                session_id: "session-A".to_string(),
            }
        );
        assert!(!is_external_only_visible(&classify_origin(
            &mappings,
            &fleet_actors,
            &fleet_item
        )));

        // Same bot login from another session's write: no operation
        // map, so Uncertain — included in external-only, never Fleet.
        // Username never assigns session.
        let other_session = observed("c2", Some("fleet-bot"), None, None, None, None);
        assert_eq!(
            classify_origin(&mappings, &fleet_actors, &other_session),
            ActivityOrigin::Uncertain
        );
        assert!(is_external_only_visible(&ActivityOrigin::Uncertain));

        // Distinct human actor with no Fleet markers: External,
        // included in external-only.
        let human = observed("c3", Some("alice"), None, None, None, None);
        assert_eq!(
            classify_origin(&mappings, &fleet_actors, &human),
            ActivityOrigin::External
        );
        assert!(is_external_only_visible(&ActivityOrigin::External));

        // Negative: coincidental same-body human comment. Body is not
        // an input to the classifier at all, so a human item that
        // happens to repeat Fleet wording is still External, never
        // Fleet — and stays visible.
        let same_body_human = observed("c4", Some("alice"), None, None, None, None);
        assert_eq!(
            classify_origin(&mappings, &fleet_actors, &same_body_human),
            ActivityOrigin::External
        );
        assert!(is_external_only_visible(&classify_origin(
            &mappings,
            &fleet_actors,
            &same_body_human
        )));

        // Negative: stale mapping. The mapping is gone (operation
        // retired), but the observation still claims op-A/r1: without
        // a live mapping it is Uncertain, never Fleet — and stays
        // visible.
        let stale = observed(
            "c1",
            Some("fleet-bot"),
            Some("r1"),
            Some("op-A"),
            Some("session-A"),
            None,
        );
        assert_eq!(
            classify_origin(&[], &fleet_actors, &stale),
            ActivityOrigin::Uncertain
        );
        assert!(is_external_only_visible(&classify_origin(
            &[],
            &fleet_actors,
            &stale
        )));

        // Session disagreement: op-A's remote claimed under a
        // different session is Uncertain, never Fleet.
        let wrong_session = observed(
            "c1",
            Some("fleet-bot"),
            Some("r1"),
            Some("op-A"),
            Some("session-B"),
            None,
        );
        assert_eq!(
            classify_origin(&mappings, &fleet_actors, &wrong_session),
            ActivityOrigin::Uncertain
        );

        // Migration: legacy observation with no IDs at all is
        // Uncertain (included), never silently excluded and never
        // Fleet.
        let legacy = observed("c0", None, None, None, None, None);
        assert_eq!(
            classify_origin(&mappings, &fleet_actors, &legacy),
            ActivityOrigin::Uncertain
        );
        assert!(is_external_only_visible(&classify_origin(
            &mappings,
            &fleet_actors,
            &legacy
        )));

        // Empty identity is rejected, never silently accepted.
        assert!(OperationMapping::new("", "session-A", None, vec![]).is_err());
        assert!(OperationMapping::new("op-A", "", None, vec![]).is_err());
        assert!(
            ObservedActivity::new("", None::<String>, None, None, None, None).is_err()
        );
    }

    fn refresh_state(
        source_ref: &str,
        committed: Option<PageCursor>,
        query_fingerprint: &str,
        api_version: &str,
        needs_refetch: bool,
    ) -> SourceRefreshState {
        SourceRefreshState::new(
            source_ref,
            committed,
            query_fingerprint,
            api_version,
            needs_refetch,
        )
        .expect("valid refresh state")
    }

    fn committed_cursor(next_continuation: Option<&str>) -> PageCursor {
        PageCursor {
            source_ref: "github://alpha/repo/issues/10".to_string(),
            endpoint: "issues/comments".to_string(),
            query_fingerprint: "qfp1".to_string(),
            api_version: "v3".to_string(),
            generation: 2,
            last_event_key: Some("e-9".to_string()),
            next_continuation: next_continuation.map(str::to_string),
            pages_committed: 3,
        }
    }

    #[test]
    fn pmo_hyd_reconcile_plan_is_bounded_and_prefers_resume() {
        // A: current cursor, nothing pending — UpToDate.
        let a = refresh_state(
            "github://alpha/repo/issues/10",
            Some(committed_cursor(None)),
            "qfp1",
            "v3",
            false,
        );
        // B: current cursor with continuation — resume wins the
        // budget over later snapshots.
        let b = refresh_state(
            "github://alpha/repo/pulls/4",
            Some(committed_cursor(Some("next-1"))),
            "qfp1",
            "v3",
            false,
        );
        // C: never fetched — full snapshot.
        let c = refresh_state("github://alpha/repo/issues/11", None, "qfp1", "v3", false);
        // D: fingerprint moved — token never reused: snapshot, and
        // with budget 2 it is explicitly deferred, not dropped.
        let mut stale = committed_cursor(None);
        stale.query_fingerprint = "qfp0".to_string();
        let d = refresh_state(
            "github://alpha/repo/issues/12",
            Some(stale),
            "qfp1",
            "v3",
            false,
        );
        // E: current cursor, no continuation, but the last pass
        // left follow-up work — snapshot; deferred under budget 2.
        let e = refresh_state(
            "github://alpha/repo/issues/13",
            Some(committed_cursor(None)),
            "qfp1",
            "v3",
            true,
        );

        let plan = plan_refresh(&[a, b, c, d, e], 2).expect("valid budget");
        assert_eq!(plan.budget, 2);
        assert_eq!(
            plan.actions,
            vec![
                RefreshAction::UpToDate {
                    source_ref: "github://alpha/repo/issues/10".to_string(),
                },
                RefreshAction::Resume {
                    source_ref: "github://alpha/repo/pulls/4".to_string(),
                    continuation: "next-1".to_string(),
                },
                RefreshAction::FullSnapshot {
                    source_ref: "github://alpha/repo/issues/11".to_string(),
                    reason: SnapshotReason::NoCursor,
                },
            ]
        );
        // D and E are postponed explicitly — present in deferred,
        // absent from actions, never silently skipped.
        assert_eq!(
            plan.deferred,
            vec![
                "github://alpha/repo/issues/12".to_string(),
                "github://alpha/repo/issues/13".to_string(),
            ]
        );

        // Negative: UpToDate never consumes budget — with budget 1
        // the up-to-date source plus one fetch still both decide.
        let plan = plan_refresh(
            &[
                refresh_state(
                    "github://alpha/repo/issues/10",
                    Some(committed_cursor(None)),
                    "qfp1",
                    "v3",
                    false,
                ),
                refresh_state(
                    "github://alpha/repo/pulls/4",
                    Some(committed_cursor(Some("next-1"))),
                    "qfp1",
                    "v3",
                    false,
                ),
            ],
            1,
        )
        .expect("valid budget");
        assert_eq!(plan.actions.len(), 2);
        assert!(plan.deferred.is_empty());

        // Negative: a zero budget would silently complete nothing —
        // typed rejection, never an empty plan.
        assert!(plan_refresh(&[], 0).is_err());

        // Negative: empty identity is rejected, never silently
        // accepted.
        assert!(SourceRefreshState::new("", None, "qfp1", "v3", false).is_err());
    }
}

/// Fleet operation mapping for attribution — HYD-07 (design §4 and
/// §8: "External attribution maps Fleet `operation_id`/`session_id`/
/// `outbox_id` to remote returned IDs and content markers").
///
/// A mapping states that one Fleet operation (in one session,
/// optionally via one outbox entry) produced the listed remote
/// resource IDs. Shared GitHub App/bot logins are deliberately
/// absent here: the login is passed separately to
/// [`classify_origin`] and can only ever yield `Uncertain`, never
/// `Fleet` on its own ("username never assigns session").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationMapping {
    /// Fleet operation identity (e.g. queue operation ID).
    pub operation_id: String,
    /// Fleet session that performed the operation.
    pub session_id: String,
    /// Outbox entry that carried the write, if any.
    pub outbox_id: Option<String>,
    /// Remote resource IDs this operation is known to have produced.
    pub remote_ids: Vec<String>,
}

impl OperationMapping {
    /// Build one mapping; empty operation/session identity is a
    /// typed rejection, never a silently accepted blank map.
    pub fn new(
        operation_id: impl Into<String>,
        session_id: impl Into<String>,
        outbox_id: Option<String>,
        remote_ids: Vec<String>,
    ) -> Result<Self, ContractError> {
        let operation_id = operation_id.into();
        let session_id = session_id.into();
        if operation_id.is_empty() {
            return Err(ContractError::InvalidSchema {
                detail: "operation mapping has empty operation_id".to_string(),
            });
        }
        if session_id.is_empty() {
            return Err(ContractError::InvalidSchema {
                detail: "operation mapping has empty session_id".to_string(),
            });
        }
        Ok(OperationMapping {
            operation_id,
            session_id,
            outbox_id,
            remote_ids,
        })
    }
}

/// Attribution of one observed source activity item — HYD-07.
///
/// `Fleet` requires a reliable operation mapping (operation ID plus
/// remote ID, with session agreement); everything else is either
/// confidently external or of uncertain origin. Uncertain events are
/// included by `--external-only` by default, alongside external
/// ones; only `Fleet` is excluded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActivityOrigin {
    /// Reliably mapped to one Fleet operation in one session.
    Fleet {
        /// Matched operation identity.
        operation_id: String,
        /// Matched session identity.
        session_id: String,
    },
    /// Confidently not Fleet-caused (e.g. distinct human actor with
    /// no Fleet markers).
    External,
    /// Origin cannot be proven either way (e.g. shared bot login
    /// with no operation map, partial/stale mapping, or legacy
    /// observation with no IDs at all). Included as external by
    /// default; never silently dropped.
    Uncertain,
}

/// One observed source activity item for attribution — HYD-07.
///
/// The content body is deliberately not an input: coincidental
/// same-body text (a human comment repeating Fleet wording) must
/// never attribute to a Fleet session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedActivity {
    /// Source-native item ID (occurrence identity).
    pub native_id: String,
    /// Actor login reported by the source, if any.
    pub actor_login: Option<String>,
    /// Remote resource ID reported by the source, if any.
    pub remote_id: Option<String>,
    /// Fleet operation ID claimed by local provenance, if any.
    pub operation_id: Option<String>,
    /// Fleet session ID claimed by local provenance, if any.
    pub session_id: Option<String>,
    /// Outbox entry claimed by local provenance, if any.
    pub outbox_id: Option<String>,
}

impl ObservedActivity {
    /// Build one observation; empty native identity is a typed
    /// rejection, never a silently accepted blank record.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        native_id: impl Into<String>,
        actor_login: Option<String>,
        remote_id: Option<String>,
        operation_id: Option<String>,
        session_id: Option<String>,
        outbox_id: Option<String>,
    ) -> Result<Self, ContractError> {
        let native_id = native_id.into();
        if native_id.is_empty() {
            return Err(ContractError::InvalidSchema {
                detail: "observed activity has empty native_id".to_string(),
            });
        }
        Ok(ObservedActivity {
            native_id,
            actor_login,
            remote_id,
            operation_id,
            session_id,
            outbox_id,
        })
    }
}

/// Classify one observed item against the durable operation mappings
/// — HYD-07 canonical attribution classifier.
///
/// * `Fleet` requires the observed `operation_id` **and**
///   `remote_id` to match one mapping together (same operation
///   produced this remote ID). When the observation carries a
///   `session_id`, it must agree with the mapping's session;
///   disagreement is `Uncertain`, never `Fleet`.
/// * A shared Fleet actor login (bot/App) with no full mapping is
///   `Uncertain`: the login proves nothing by itself, but the item
///   could be another session's write, so it stays visible under
///   `--external-only`.
/// * An observation with no Fleet markers and a distinct (non-Fleet)
///   actor is `External`.
/// * An observation with no IDs at all (legacy) is `Uncertain`
///   ("legacy uncertain"); it is included, never silently excluded.
/// * Body/content similarity is not an input and can never produce
///   `Fleet`.
pub fn classify_origin(
    mappings: &[OperationMapping],
    fleet_actor_logins: &[&str],
    observed: &ObservedActivity,
) -> ActivityOrigin {
    if let (Some(op), Some(remote)) = (observed.operation_id.as_deref(), observed.remote_id.as_deref())
    {
        for mapping in mappings {
            if mapping.operation_id == op && mapping.remote_ids.iter().any(|id| id == remote) {
                match observed.session_id.as_deref() {
                    Some(session) if session != mapping.session_id => continue,
                    _ => {
                        return ActivityOrigin::Fleet {
                            operation_id: mapping.operation_id.clone(),
                            session_id: mapping.session_id.clone(),
                        };
                    }
                }
            }
        }
        // An operation claim or a mapped remote ID without a full
        // match is a partial/stale mapping: uncertain, never Fleet.
        return ActivityOrigin::Uncertain;
    }
    if observed.operation_id.is_some()
        || observed.session_id.is_some()
        || observed.outbox_id.is_some()
    {
        return ActivityOrigin::Uncertain;
    }
    if let Some(remote) = observed.remote_id.as_deref() {
        if mappings
            .iter()
            .any(|m| m.remote_ids.iter().any(|id| id == remote))
        {
            return ActivityOrigin::Uncertain;
        }
    }
    match observed.actor_login.as_deref() {
        None => ActivityOrigin::Uncertain,
        Some(login) if fleet_actor_logins.contains(&login) => ActivityOrigin::Uncertain,
        Some(_) => ActivityOrigin::External,
    }
}

/// Whether an attributed item is visible under `--external-only`
/// with default semantics: `External` and `Uncertain` are included;
/// only reliably attributed `Fleet` items are excluded.
pub fn is_external_only_visible(origin: &ActivityOrigin) -> bool {
    !matches!(
        origin,
        ActivityOrigin::Fleet {
            operation_id: _,
            session_id: _
        }
    )
}
