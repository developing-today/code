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
}
