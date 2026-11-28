//! GitHub comments ingestion — the issue/PR conversation layer
//! for Atlas (developing-today/code#6023, parent program #6018).
//!
//! Atlas held code, docs, and sessions, but not the GitHub
//! conversation layer where much of the design record lives. This
//! module is the ingestion writer for that layer, following the
//! same shape as [`crate::trace_ingest`]: a source registration
//! that lives beside the writer (no source-set registry exists
//! anywhere else in this repo), a record inclusion test, and a
//! pure conversion from source records to corpus documents.
//!
//! The rules this module encodes, from #6023:
//!
//! 1. **The chunk unit is the comment, with its thread context.**
//!    A [`CommentDocument`]'s text carries the thread's repo,
//!    number, title, state, and labels ahead of the comment body,
//!    and every document carries the program's provenance mark —
//!    `github.com/<owner>/<repo>#<n>` + comment id + observed
//!    date — plus the comment's author, so our comments and
//!    others' comments stay distinguishable (search quality and
//!    licensing posture both depend on it).
//! 2. **Extraction is a recurring feed, not a dump.** Per-repo
//!    `updated_at` watermarks ([`CommentWatermarks`]) make each
//!    run delta-only: a comment is extracted exactly when its
//!    `updated_at` is newer than the repo's watermark, and the
//!    watermark only ever advances.
//! 3. **Our repos only.** This writer serves the our-repos estate.
//!    External-repo comment text is gated on the licensing
//!    research (#6024) and must never pass through here; the
//!    external set exists only as an enumeration artifact.

use std::collections::BTreeMap;

use serde_json::{Value as Json, json};

/// The ingestion source class this module registers GitHub
/// comments under.
pub const GITHUB_COMMENTS_SOURCE_CLASS: &str = "github comments";

/// The ingestion source id for the GitHub comments source.
pub const GITHUB_COMMENTS_SOURCE_ID: &str = "github-comments";

/// Schema stamped on every emitted comment record.
pub const GITHUB_COMMENT_SCHEMA: &str = "atlas-github-comment/v1";

/// One row of the ingestion source set: the GitHub comments
/// source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubCommentsSourceRegistration {
    pub source_id: &'static str,
    pub source_class: &'static str,
    pub record_schema: &'static str,
    pub file_pattern: &'static str,
}

/// The GitHub comments source registration.
pub fn github_comments_source_registration() -> GithubCommentsSourceRegistration {
    GithubCommentsSourceRegistration {
        source_id: GITHUB_COMMENTS_SOURCE_ID,
        source_class: GITHUB_COMMENTS_SOURCE_CLASS,
        record_schema: GITHUB_COMMENT_SCHEMA,
        file_pattern: "atlas-github-comments-YYYY-MM-DD.jsonl",
    }
}

/// The registration's inclusion test: a record belongs to the
/// GitHub comments source exactly when it is an
/// `atlas-github-comment/v1` record.
pub fn record_included(record: &Json) -> bool {
    record.get("schema").and_then(Json::as_str) == Some(GITHUB_COMMENT_SCHEMA)
}

/// Which GitHub conversation surface a comment came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentKind {
    /// A comment on an issue or PR conversation (`issues/comments`).
    IssueComment,
    /// A comment inside a PR review (`pulls/comments`).
    ReviewComment,
    /// A submitted PR review itself (`pulls/<n>/reviews`).
    Review,
}

impl CommentKind {
    /// The record spelling of the kind.
    pub fn as_str(&self) -> &'static str {
        match self {
            CommentKind::IssueComment => "issue_comment",
            CommentKind::ReviewComment => "review_comment",
            CommentKind::Review => "review",
        }
    }

    /// Parse the record spelling, if it is one.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "issue_comment" => Some(CommentKind::IssueComment),
            "review_comment" => Some(CommentKind::ReviewComment),
            "review" => Some(CommentKind::Review),
            _ => None,
        }
    }
}

/// The thread a comment belongs to: an issue or a pull request,
/// with the context a chunk must carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhThread {
    /// `owner/repo`.
    pub repo: String,
    /// Issue/PR number within the repo.
    pub number: u64,
    /// True for a pull request thread, false for an issue thread.
    pub is_pr: bool,
    pub title: String,
    /// `open` / `closed` (PRs may also be `merged` upstream; the
    /// caller normalises before constructing).
    pub state: String,
    pub labels: Vec<String>,
}

/// One GitHub comment, normalised across the three surfaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhComment {
    /// The surface-specific numeric id (unique per surface, so
    /// identity is `(kind, id)`).
    pub id: u64,
    pub kind: CommentKind,
    /// Author login; attribution is preserved verbatim.
    pub author: String,
    pub body: String,
    pub created_at: String,
    /// For reviews, the `submitted_at` (reviews carry no separate
    /// update timestamp).
    pub updated_at: String,
    pub url: String,
}

/// The provenance mark the program's standard requires:
/// `github.com/<owner>/<repo>#<n>` + comment id + observed date.
pub fn provenance_mark(repo: &str, number: u64, comment_id: u64, observed_date: &str) -> String {
    format!("github.com/{repo}#{number} comment {comment_id} observed {observed_date}")
}

/// One ingested GitHub comment as a searchable corpus document.
/// `text` is the chunk the corpus search matches over: the
/// thread-context header followed by the comment body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentDocument {
    pub repo: String,
    pub thread_number: u64,
    pub is_pr: bool,
    pub comment_id: u64,
    pub kind: CommentKind,
    pub author: String,
    pub created_at: String,
    pub updated_at: String,
    pub provenance: String,
    pub text: String,
}

/// Build the corpus document for one comment in its thread.
/// `observed_date` is the extraction date (`YYYY-MM-DD`) stamped
/// into the provenance mark.
pub fn comment_document(
    thread: &GhThread,
    comment: &GhComment,
    observed_date: &str,
) -> CommentDocument {
    let provenance = provenance_mark(&thread.repo, thread.number, comment.id, observed_date);
    let thread_kind = if thread.is_pr {
        "pull_request"
    } else {
        "issue"
    };
    let labels = if thread.labels.is_empty() {
        "-".to_string()
    } else {
        thread.labels.join(", ")
    };
    let text = format!(
        "[{provenance}]\n\
         thread: {thread_kind} {repo}#{number} — {title}\n\
         state: {state} · labels: {labels}\n\
         comment {cid} ({kind}) by {author} at {created} (updated {updated})\n\
         {body}",
        repo = thread.repo,
        number = thread.number,
        title = thread.title,
        state = thread.state,
        cid = comment.id,
        kind = comment.kind.as_str(),
        author = comment.author,
        created = comment.created_at,
        updated = comment.updated_at,
        body = comment.body,
    );
    CommentDocument {
        repo: thread.repo.clone(),
        thread_number: thread.number,
        is_pr: thread.is_pr,
        comment_id: comment.id,
        kind: comment.kind,
        author: comment.author.clone(),
        created_at: comment.created_at.clone(),
        updated_at: comment.updated_at.clone(),
        provenance,
        text,
    }
}

/// Serialise a document as the schema-stamped JSONL record the
/// corpus files carry.
pub fn document_to_record(doc: &CommentDocument) -> Json {
    json!({
        "schema": GITHUB_COMMENT_SCHEMA,
        "repo": doc.repo,
        "thread_number": doc.thread_number,
        "thread_kind": if doc.is_pr { "pull_request" } else { "issue" },
        "comment_id": doc.comment_id,
        "kind": doc.kind.as_str(),
        "author": doc.author,
        "created_at": doc.created_at,
        "updated_at": doc.updated_at,
        "provenance": doc.provenance,
        "text": doc.text,
    })
}

/// Parse a record back into a document; `None` for records the
/// inclusion test rejects or that lack the required fields.
pub fn document_from_record(record: &Json) -> Option<CommentDocument> {
    if !record_included(record) {
        return None;
    }
    Some(CommentDocument {
        repo: record.get("repo")?.as_str()?.to_string(),
        thread_number: record.get("thread_number")?.as_u64()?,
        is_pr: record.get("thread_kind")?.as_str()? == "pull_request",
        comment_id: record.get("comment_id")?.as_u64()?,
        kind: CommentKind::parse(record.get("kind")?.as_str()?)?,
        author: record.get("author")?.as_str()?.to_string(),
        created_at: record.get("created_at")?.as_str()?.to_string(),
        updated_at: record.get("updated_at")?.as_str()?.to_string(),
        provenance: record.get("provenance")?.as_str()?.to_string(),
        text: record.get("text")?.as_str()?.to_string(),
    })
}

/// Per-repo `updated_at` watermarks for the recurring extraction
/// feed. A repo's watermark is the newest comment `updated_at`
/// a completed run has observed there; the next run extracts
/// only comments newer than it. Watermarks never move backwards.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommentWatermarks {
    repos: BTreeMap<String, String>,
    reviews: BTreeMap<String, String>,
}

impl CommentWatermarks {
    /// An empty state: every repo extracts from the beginning.
    pub fn new() -> Self {
        Self::default()
    }

    /// The `since` bound for the next run over `repo`, if any
    /// run has completed there.
    pub fn since_for(&self, repo: &str) -> Option<&str> {
        self.repos.get(repo).map(String::as_str)
    }

    /// Advance `repo`'s watermark to `updated_at` if that is
    /// newer than the stored value. Returns true when it moved.
    pub fn observe(&mut self, repo: &str, updated_at: &str) -> bool {
        let newer = match self.repos.get(repo) {
            Some(current) => updated_at > current.as_str(),
            None => true,
        };
        if newer {
            self.repos.insert(repo.to_string(), updated_at.to_string());
        }
        newer
    }

    /// The delta a run over `repo` must extract from `comments`:
    /// exactly those newer than the repo's watermark (all of
    /// them when no watermark exists). This is the local
    /// guarantee behind the API's `since` parameter — the API
    /// bound is inclusive at the boundary, so the boundary
    /// comment is filtered here and never extracted twice.
    pub fn select_delta<'a>(&self, repo: &str, comments: &'a [GhComment]) -> Vec<&'a GhComment> {
        match self.since_for(repo) {
            None => comments.iter().collect(),
            Some(wm) => comments
                .iter()
                .filter(|c| c.updated_at.as_str() > wm)
                .collect(),
        }
    }

    /// Cutoff for submitted reviews, whose walk can be skipped independently.
    pub fn reviews_since_for(&self, repo: &str) -> Option<&str> {
        self.reviews.get(repo).map(String::as_str)
    }

    /// Record a submitted review in a completed review walk.
    pub fn observe_review(&mut self, repo: &str, submitted_at: &str) {
        if self
            .reviews_since_for(repo)
            .is_none_or(|old| submitted_at > old)
        {
            self.reviews
                .insert(repo.to_owned(), submitted_at.to_owned());
        }
    }

    /// Serialise the state as the JSON the extraction CLI
    /// persists between runs.
    pub fn to_json(&self) -> Json {
        json!({ "repos": self.repos, "reviews": self.reviews })
    }

    /// Load state persisted by [`CommentWatermarks::to_json`];
    /// malformed state yields an empty state rather than a
    /// guess, so a corrupt file restarts extraction instead of
    /// silently skipping comments.
    pub fn from_json(value: &Json) -> Self {
        let mut state = CommentWatermarks::new();
        fn read_map(value: Option<&Json>) -> BTreeMap<String, String> {
            value
                .and_then(Json::as_object)
                .into_iter()
                .flatten()
                .filter_map(|(repo, wm)| Some((repo.clone(), wm.as_str()?.to_string())))
                .collect()
        }
        state.repos = read_map(value.get("repos"));
        // V1 persisted a single cutoff for every stream. Preserve its resume
        // position on upgrade. An explicit empty reviews map means no review
        // pass has completed yet, e.g. after running with --skip-reviews.
        state.reviews = match value.get("reviews") {
            Some(reviews) => read_map(Some(reviews)),
            None => state.repos.clone(),
        };
        state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thread() -> GhThread {
        GhThread {
            repo: "developing-today/code".to_string(),
            number: 6023,
            is_pr: false,
            title: "Atlas GitHub corpus extraction".to_string(),
            state: "open".to_string(),
            labels: vec!["epic/atlas-program".to_string()],
        }
    }

    fn comment() -> GhComment {
        GhComment {
            id: 6913627009,
            kind: CommentKind::IssueComment,
            author: "dezren39".to_string(),
            body: "the design record lives here".to_string(),
            created_at: "2026-11-16T08:49:33Z".to_string(),
            updated_at: "2026-11-16T08:49:33Z".to_string(),
            url: "https://github.com/developing-today/code/issues/6023#issuecomment-6913627009"
                .to_string(),
        }
    }

    #[test]
    fn chunk_carries_thread_context_provenance_and_author() {
        let doc = comment_document(&thread(), &comment(), "2026-11-16");
        assert_eq!(
            doc.provenance,
            "github.com/developing-today/code#6023 comment 6913627009 observed 2026-11-16"
        );
        assert_eq!(doc.author, "dezren39");
        assert!(doc.text.contains("Atlas GitHub corpus extraction"));
        assert!(doc.text.contains("epic/atlas-program"));
        assert!(doc.text.contains("the design record lives here"));
        assert!(doc.text.contains("issue_comment"));
    }

    #[test]
    fn record_round_trips_and_inclusion_is_schema_exact() {
        let doc = comment_document(&thread(), &comment(), "2026-11-16");
        let record = document_to_record(&doc);
        assert!(record_included(&record));
        assert_eq!(document_from_record(&record), Some(doc));
        let other = json!({ "schema": "atlas-query-trace/v1" });
        assert!(!record_included(&other));
        assert_eq!(document_from_record(&other), None);
    }

    #[test]
    fn registration_names_the_source() {
        let reg = github_comments_source_registration();
        assert_eq!(reg.source_class, "github comments");
        assert_eq!(reg.record_schema, GITHUB_COMMENT_SCHEMA);
    }

    #[test]
    fn watermark_selects_only_newer_comments() {
        let mut state = CommentWatermarks::new();
        let older = GhComment {
            updated_at: "2026-11-15T00:00:00Z".to_string(),
            ..comment()
        };
        let boundary = GhComment {
            id: 2,
            updated_at: "2026-11-16T00:00:00Z".to_string(),
            ..comment()
        };
        let newer = GhComment {
            id: 3,
            updated_at: "2026-11-17T00:00:00Z".to_string(),
            ..comment()
        };
        let all = vec![older, boundary, newer];
        // No watermark: everything is delta.
        assert_eq!(state.select_delta("developing-today/code", &all).len(), 3);
        state.observe("developing-today/code", "2026-11-16T00:00:00Z");
        let delta = state.select_delta("developing-today/code", &all);
        // The boundary comment is not re-extracted; only the
        // strictly newer one is.
        assert_eq!(delta.len(), 1);
        assert_eq!(delta[0].id, 3);
    }

    #[test]
    fn watermark_never_moves_backwards_and_round_trips() {
        let mut state = CommentWatermarks::new();
        assert!(state.observe("a/b", "2026-11-16T00:00:00Z"));
        assert!(!state.observe("a/b", "2026-11-15T00:00:00Z"));
        assert_eq!(state.since_for("a/b"), Some("2026-11-16T00:00:00Z"));
        let loaded = CommentWatermarks::from_json(&state.to_json());
        assert_eq!(loaded, state);
        // Malformed state restarts empty rather than guessing.
        assert_eq!(
            CommentWatermarks::from_json(&json!({ "repos": 42 })),
            CommentWatermarks::new()
        );
    }
}
