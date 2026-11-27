//! Resumable durable page ingestion — HYD-05
//! (developing-today/code#9207, program #9196).
//!
//! This module is the page-iterator/cursor slice of the PM
//! orchestration design (`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/components/9207-context-hydration-incremental-activity.md`,
//! §4 and the crash table): GitHub `Link` continuation parsing,
//! per-source page cursors, atomic page-ingest/cursor-commit
//! semantics, documented API caps, and query/API fingerprinting.
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
//! never trusted. The test below compares expected and observed
//! committed sequences explicitly. No mock-only claim of
//! deployed/live behavior.

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
}
