//! The shared content-creation ledger writer for `fleet gh`
//! (developing-today/code#2227, slice: route the gh-shim bypass
//! through the shared gate).
//!
//! Every `fleet gh` content-creating call — `issue create`,
//! `pr create`, `issue comment`, `pr comment`, `pr review
//! --comment`, `release create`, `gist create`, and POST-class
//! `api` calls — reserves a row in
//! the shared creation ledger
//! (`~/.local/state/fleet/content-ledger.sqlite`, override with
//! `CONTENT_LEDGER_PATH`) BEFORE any GitHub write and finalises it
//! once the outcome is known. Table, columns, kind/outcome
//! vocabulary and reservation ordering are identical to the hatch
//! reference implementation (`~/hatch/tools/content-ledger`),
//! so rows written here and rows written by gh-post share one DB
//! and one `counts`/`tail` read path (SQLite WAL +
//! `BEGIN IMMEDIATE`, cross-process atomic on one host).
//!
//! Decisions recorded (decide-and-proceed, nothing waits):
//!   - Fail-closed like gh-post: a reservation failure refuses the
//!     create (exit 5), never an unledgered filing. `busy_timeout`
//!     (30 s, same as the reference) absorbs lock contention; only
//!     a genuine DB failure refuses. Reads are untouched — the hook
//!     runs for classified creates only.
//!   - gh's exit code always stands: finalise records the truth
//!     (the metadata create path's posture). An exit-0 create with
//!     empty stdout is recorded `failed` with a loud stderr notice —
//!     the exact 2026-10-28 silent-success regression — while the
//!     caller's exit code is unchanged.
//!   - Reservation only in this slice: no pacing budgets, no
//!     circuit breaker, no verify-by-search. Those are the warn-mode
//!     throttle (hatch#13) and later slices; this slice makes the
//!     bypass visible in the ledger first.
//!   - Scope is issues/PRs/comments, plus `pr review --comment`
//!     (kind `review`, already in the reference vocabulary),
//!     `release create` (kind `release`), `gist create` (kind
//!     `gist`), and `api --method POST` (kind `other`, the
//!     reference `KINDS` catch-all). The reference `KINDS`
//!     has neither a `release` nor a `gist` entry — the reference
//!     filing path files neither shape — so both extend this
//!     writer's vocabulary by one kind each, while `other` is
//!     byte-identical to the reference. The shared read paths do
//!     not gate on kind (`counts` groups by outcome, `tail`
//!     reads kind as data), so `release`/`gist`/`other` rows
//!     reconcile in the one DB exactly like the existing kinds.
//!     Non-POST `api` calls and the non-comment `pr review`
//!     shapes (`--approve`, `--request-changes`) pass through
//!     unledgered as today; routing them is the named follow-up
//!     slice (see the PR body).
//!
//! Oracle: the hatch spec's shared-ledger invariant — every
//! content-creating call has a reservation row written before the
//! call and exactly one final outcome (`created` |
//! `verified_existing` | `failed`); kind and outcome strings are
//! byte-identical to the reference so both writers reconcile.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Env override for the ledger path (same name as the reference).
pub const LEDGER_PATH_ENV: &str = "CONTENT_LEDGER_PATH";
/// Actor/job defaults mirror the reference (`FLEET_ACTOR`/default,
/// `FLEET_JOB`/own default — the shim is never the `cli` the
/// reference CLI defaults to).
pub const LEDGER_ACTOR_ENV: &str = "FLEET_ACTOR";
pub const LEDGER_JOB_ENV: &str = "FLEET_JOB";

/// Schema, kept identical to the reference implementation's
/// `SCHEMA` so both writers share one database file.
const LEDGER_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS events(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  ts_utc TEXT NOT NULL,
  actor TEXT NOT NULL,
  job TEXT NOT NULL,
  repo TEXT NOT NULL,
  kind TEXT NOT NULL,
  ref TEXT,
  outcome TEXT NOT NULL,
  nonce TEXT,
  detail TEXT
);
CREATE INDEX IF NOT EXISTS events_ts ON events(ts_utc);
CREATE INDEX IF NOT EXISTS events_nonce ON events(nonce);
";

/// The create shapes this slice ledgers (reference `KINDS`
/// subset: issues, PRs, comments, reviews, plus the `other`
/// catch-all; `release` and `gist` extend the reference
/// vocabulary — see the header).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateKind {
    Issue,
    Pr,
    IssueComment,
    PrComment,
    PrReview,
    Release,
    Gist,
    Other,
}

impl CreateKind {
    /// The reference implementation's kind string, byte-identical.
    pub fn as_ledger_kind(self) -> &'static str {
        match self {
            CreateKind::Issue => "issue",
            CreateKind::Pr => "pr",
            CreateKind::IssueComment => "issue_comment",
            CreateKind::PrComment => "pr_comment",
            CreateKind::PrReview => "review",
            CreateKind::Release => "release",
            CreateKind::Gist => "gist",
            CreateKind::Other => "other",
        }
    }
}

/// A classified content-creating call: what it creates and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classified {
    pub kind: CreateKind,
    pub repo: String,
}

/// A held reservation: the row to finalise once the call returns.
#[derive(Debug, Clone)]
pub struct Pending {
    path: PathBuf,
    row: i64,
}

/// Classify `args` (the canonical gh argv, post sync-rewrite) as a
/// ledgered create or not. Anything unrecognised is not a create —
/// reads and the deferred shapes pass through untouched.
pub fn classify(args: &[String]) -> Option<Classified> {
    let verb = args.first().map(|s| s.as_str())?;
    let sub = args.get(1).map(|s| s.as_str())?;
    // POST-class `gh api` calls file content through the raw
    // endpoint route (issues, comments, releases, gists — anything
    // the REST API creates). gh defaults to GET, so only an
    // explicit POST method is a create; GET reads and the in-place
    // methods stay unledgered in this slice.
    if verb == "api" {
        return api_post_endpoint(args).map(|endpoint| Classified {
            repo: api_repo(&endpoint).unwrap_or_else(|| repo_of(args)),
            kind: CreateKind::Other,
        });
    }
    let kind = match (verb, sub) {
        ("issue", "create") => CreateKind::Issue,
        ("pr", "create") => CreateKind::Pr,
        ("issue", "comment") => CreateKind::IssueComment,
        ("pr", "comment") => CreateKind::PrComment,
        // Only the comment-bearing review shape is ledgered: it is
        // the one that files new content with a body. `--approve`
        // and `--request-changes` stay unledgered in this slice.
        ("pr", "review") if args.iter().any(|a| a == "--comment") => CreateKind::PrReview,
        ("release", "create") => CreateKind::Release,
        ("gist", "create") => CreateKind::Gist,
        _ => return None,
    };
    Some(Classified {
        kind,
        repo: repo_of(args),
    })
}

/// The endpoint of a POST-class `gh api` call, if the call is one:
/// an explicit `--method POST`/`-X POST` plus a first endpoint
/// (the first non-flag token that is not a data/header flag's
/// value). Anything else — GET reads, in-place methods, a POST
/// with no endpoint — is not a create and yields None.
fn api_post_endpoint(args: &[String]) -> Option<String> {
    // Flags whose next token is a value, never the endpoint.
    fn takes_value(a: &str) -> bool {
        matches!(
            a,
            "--method"
                | "-X"
                | "-f"
                | "--field"
                | "--raw-field"
                | "--input"
                | "-H"
                | "--header"
                | "--paginate"
                | "--jq"
                | "--template"
        )
    }
    let mut method: Option<&str> = None;
    let mut endpoint: Option<&str> = None;
    let mut i = 1;
    while i < args.len() {
        let a = args[i].as_str();
        if a == "--method" || a == "-X" {
            method = args.get(i + 1).map(|s| s.as_str());
            i += 2;
            continue;
        }
        if let Some(v) = a
            .strip_prefix("--method=")
            .or_else(|| a.strip_prefix("-X="))
        {
            method = Some(v);
        } else if takes_value(a) {
            i += 2;
            continue;
        } else if a.contains('=') && !a.starts_with('-') {
            // A bare `key=value` field, never the endpoint.
        } else if !a.starts_with('-') && endpoint.is_none() {
            endpoint = Some(a);
        }
        i += 1;
    }
    if method.is_some_and(|m| m.eq_ignore_ascii_case("post")) {
        endpoint.filter(|e| !e.is_empty()).map(|e| e.to_string())
    } else {
        None
    }
}

/// The `owner/repo` embedded in a `repos/{owner}/{repo}/...`
/// endpoint, if that is the endpoint's shape. Other endpoints
/// (e.g. `user/repos`, `gists`) record no repo in the path and
/// fall back to the call's `-R`/`--repo` flags.
fn api_repo(endpoint: &str) -> Option<String> {
    let mut segs = endpoint.trim_start_matches('/').split('/');
    if segs.next()? != "repos" {
        return None;
    }
    let owner = segs.next()?;
    let repo = segs.next()?;
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some(format!("{owner}/{repo}"))
}

/// The `-R`/`--repo` target of the call, else `"unknown"`. gh
/// resolves the repo from the checkout when the flag is absent; the
/// ledger records that the call did not name one.
fn repo_of(args: &[String]) -> String {
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if a == "-R" || a == "--repo" {
            if let Some(r) = args.get(i + 1) {
                if !r.trim().is_empty() {
                    return r.clone();
                }
            }
            return "unknown".to_string();
        }
        if let Some(r) = a.strip_prefix("--repo=") {
            if !r.trim().is_empty() {
                return r.to_string();
            }
            return "unknown".to_string();
        }
        i += 1;
    }
    "unknown".to_string()
}

/// The ledger file: `$CONTENT_LEDGER_PATH`, else the reference
/// default under `$HOME`.
fn ledger_path() -> PathBuf {
    if let Ok(p) = std::env::var(LEDGER_PATH_ENV) {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".local/state/fleet/content-ledger.sqlite")
}

fn env_or(env: &str, fallback: &str) -> String {
    std::env::var(env)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

static NONCE_CTR: AtomicU64 = AtomicU64::new(0);

/// A 32-hex-char nonce (uuid-hex shaped, like the reference's
/// `uuid4().hex`): time + pid + counter, so concurrent processes
/// never repeat one.
fn new_nonce() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let c = NONCE_CTR.fetch_add(1, Ordering::Relaxed);
    format!(
        "{:016x}{:08x}{:08x}",
        (nanos & 0xffff_ffff_ffff_ffff) as u64,
        std::process::id(),
        (c ^ (nanos >> 64) as u64) & 0xffff_ffff,
    )
}

fn open_ledger(path: &Path) -> Result<rusqlite::Connection, String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create ledger dir {}: {e}", parent.display()))?;
        }
    }
    let conn = rusqlite::Connection::open(path)
        .map_err(|e| format!("cannot open ledger {}: {e}", path.display()))?;
    conn.execute_batch("PRAGMA busy_timeout=30000; PRAGMA journal_mode=WAL;")
        .map_err(|e| format!("cannot configure ledger {}: {e}", path.display()))?;
    conn.execute_batch(LEDGER_SCHEMA)
        .map_err(|e| format!("cannot initialize ledger {}: {e}", path.display()))?;
    Ok(conn)
}

fn txn<T>(
    conn: &rusqlite::Connection,
    path: &Path,
    op: impl FnOnce() -> Result<T, rusqlite::Error>,
) -> Result<T, String> {
    conn.execute("BEGIN IMMEDIATE", [])
        .map_err(|e| format!("ledger {} begin failed: {e}", path.display()))?;
    let out = match op() {
        Ok(out) => out,
        Err(e) => {
            let _ = conn.execute("ROLLBACK", []);
            return Err(format!("ledger {} write failed: {e}", path.display()));
        }
    };
    conn.execute("COMMIT", [])
        .map_err(|e| format!("ledger {} commit failed: {e}", path.display()))?;
    Ok(out)
}

/// Insert the reservation row (`outcome='reserved'`); the row id is
/// the finalise handle. Fails visibly on unknown kinds and on any
/// DB error — the caller refuses the create rather than filing
/// unledgered.
fn reserve(
    path: &Path,
    actor: &str,
    job: &str,
    repo: &str,
    kind: &str,
) -> Result<(i64, String), String> {
    if !matches!(
        kind,
        "issue" | "issue_comment" | "pr" | "pr_comment" | "review" | "release" | "gist" | "other"
    ) {
        return Err(format!("unknown ledger kind {kind:?}"));
    }
    let conn = open_ledger(path)?;
    let nonce = new_nonce();
    let ts = crate::gh_metadata::rfc3339(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
    );
    txn(&conn, path, || {
        conn.execute(
            "INSERT INTO events(ts_utc, actor, job, repo, kind, outcome, nonce) \
             VALUES(?,?,?,?,?,'reserved',?)",
            rusqlite::params![ts, actor, job, repo, kind, nonce],
        )?;
        Ok(conn.last_insert_rowid())
    })
    .map(|row| (row, nonce))
}

/// Flip the reservation row to its final outcome. Only a `reserved`
/// row finalises (guarded `UPDATE ... WHERE outcome='reserved'`),
/// so a double finalise is an error, never a silent duplicate.
fn finalize(
    path: &Path,
    row: i64,
    outcome: &str,
    rref: Option<&str>,
    detail: Option<&str>,
) -> Result<(), String> {
    if !matches!(outcome, "created" | "verified_existing" | "failed") {
        return Err(format!("unknown ledger outcome {outcome:?}"));
    }
    let conn = open_ledger(path)?;
    txn(&conn, path, || {
        let n = conn.execute(
            "UPDATE events SET outcome=?, ref=COALESCE(?, ref), \
             detail=COALESCE(?, detail) \
             WHERE id=? AND outcome='reserved'",
            rusqlite::params![outcome, rref, detail, row],
        )?;
        if n != 1 {
            return Err(rusqlite::Error::InvalidQuery);
        }
        Ok(())
    })
    .map_err(|e| {
        if e.contains("write failed") {
            format!("no reserved row {row} to finalize (already finalized or never reserved)")
        } else {
            e
        }
    })
}

/// Reserve the ledger row for this call, before any GitHub write.
/// `Ok(None)` for non-creates (no hook); `Err` names the ledger
/// failure the caller must refuse on.
pub fn start(args: &[String]) -> Result<Option<Pending>, String> {
    start_at(args, &ledger_path())
}

/// Reserve against an explicit ledger path. `start` passes the
/// configured path; tests pass a temporary one so they never mutate
/// process-global environment variables.
pub fn start_at(args: &[String], path: &Path) -> Result<Option<Pending>, String> {
    let Some(classified) = classify(args) else {
        return Ok(None);
    };
    let actor = env_or(LEDGER_ACTOR_ENV, "dezren39");
    let job = env_or(LEDGER_JOB_ENV, "fleet-gh");
    let (row, _) = reserve(
        path,
        &actor,
        &job,
        &classified.repo,
        classified.kind.as_ledger_kind(),
    )?;
    Ok(Some(Pending {
        path: path.to_path_buf(),
        row,
    }))
}

/// The decided outcome of a finished call: the final outcome, the
/// item ref, a detail, and an optional loud notice for stderr.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decided {
    pub outcome: &'static str,
    pub rref: Option<String>,
    pub detail: Option<String>,
    pub notice: Option<String>,
}

/// Decide the final outcome from gh's exit code and captured
/// stdout. A non-zero exit is `failed`; exit 0 with an item URL is
/// `created`; exit 0 with empty output is `failed` with a loud
/// notice (the 2026-10-28 silent-success mode: `gh issue create`
/// exited 0 with empty output while creating nothing); exit 0 with
/// non-empty but URL-less output is `created` with a detail (the
/// metadata path's NoUrl leniency — unusual flag shapes, not proof
/// of failure).
pub fn decide(exit_code: i32, stdout: Option<&[u8]>) -> Decided {
    if exit_code != 0 {
        return Decided {
            outcome: "failed",
            rref: None,
            detail: Some(format!("gh exit {exit_code}")),
            notice: None,
        };
    }
    let Some(bytes) = stdout else {
        return Decided {
            outcome: "created",
            rref: None,
            detail: Some("stdout was not captured; recorded without URL verification".to_string()),
            notice: None,
        };
    };
    let text = String::from_utf8_lossy(bytes);
    if text.trim().is_empty() {
        return Decided {
            outcome: "failed",
            rref: None,
            detail: Some("gh exited 0 with empty output".to_string()),
            notice: Some(
                "content-ledger: gh exited 0 with empty output on a create; \
                 recorded as failed, never as success (code#2227's silent-success mode) — \
                 the item may not exist"
                    .to_string(),
            ),
        };
    }
    let url = text
        .split_whitespace()
        .find(|t| t.starts_with("https://github.com/"))
        .map(|t| t.to_string());
    match url {
        Some(u) => Decided {
            outcome: "created",
            rref: Some(u),
            detail: None,
            notice: None,
        },
        None => Decided {
            outcome: "created",
            rref: None,
            detail: Some("gh exited 0 but its output carried no item URL".to_string()),
            notice: None,
        },
    }
}

/// Finalise the pending reservation with what actually happened.
/// Returns the loud notice for stderr, if any. The row records the
/// truth; gh's exit code is the caller's and stands either way.
pub fn finish(
    pending: &Pending,
    exit_code: i32,
    stdout: Option<&[u8]>,
) -> Result<Option<String>, String> {
    let d = decide(exit_code, stdout);
    finalize(
        &pending.path,
        pending.row,
        d.outcome,
        d.rref.as_deref(),
        d.detail.as_deref(),
    )?;
    Ok(d.notice)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|s| s.to_string()).collect()
    }

    fn temp_ledger(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fleet-gh-ledger-test-{tag}-{}", new_nonce()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("content-ledger.sqlite")
    }

    fn row_outcome(path: &Path, row: i64) -> (String, Option<String>, Option<String>) {
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.query_row(
            "SELECT outcome, ref, detail FROM events WHERE id=?",
            rusqlite::params![row],
            |r| {
                Ok((
                    r.get::<_, String>(0).unwrap(),
                    r.get::<_, Option<String>>(1).unwrap(),
                    r.get::<_, Option<String>>(2).unwrap(),
                ))
            },
        )
        .unwrap()
    }

    fn count_rows(path: &Path) -> i64 {
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
            .unwrap()
    }

    // oracle: the eight ledgered shapes classify with their kind
    // strings; everything else (reads, merges, deferred
    // shapes) is None and passes through untouched.
    #[test]
    fn classify_create_shapes() {
        let c = classify(&args(&["issue", "create", "--title", "x"])).unwrap();
        assert_eq!(c.kind, CreateKind::Issue);
        assert_eq!(c.kind.as_ledger_kind(), "issue");
        let c = classify(&args(&["pr", "create", "--title", "x"])).unwrap();
        assert_eq!(c.kind.as_ledger_kind(), "pr");
        let c = classify(&args(&["issue", "comment", "12", "--body", "x"])).unwrap();
        assert_eq!(c.kind.as_ledger_kind(), "issue_comment");
        let c = classify(&args(&["pr", "comment", "12", "--body", "x"])).unwrap();
        assert_eq!(c.kind.as_ledger_kind(), "pr_comment");
        let c = classify(&args(&["pr", "review", "12", "--comment", "--body", "x"])).unwrap();
        assert_eq!(c.kind, CreateKind::PrReview);
        assert_eq!(c.kind.as_ledger_kind(), "review");
        let c = classify(&args(&["release", "create", "v1.0.0", "--title", "x"])).unwrap();
        assert_eq!(c.kind, CreateKind::Release);
        assert_eq!(c.kind.as_ledger_kind(), "release");
        let c = classify(&args(&["gist", "create", "--public", "--desc", "x"])).unwrap();
        assert_eq!(c.kind, CreateKind::Gist);
        assert_eq!(c.kind.as_ledger_kind(), "gist");
        // POST-class `api` calls file content through the raw
        // endpoint route; the reference `other` kind is the
        // catch-all for them, and the repo comes from the
        // `repos/{owner}/{repo}/...` endpoint.
        let c = classify(&args(&[
            "api",
            "repos/o/r/issues",
            "--method",
            "POST",
            "-f",
            "title=x",
        ]))
        .unwrap();
        assert_eq!(c.kind, CreateKind::Other);
        assert_eq!(c.kind.as_ledger_kind(), "other");
        assert_eq!(c.repo, "o/r");

        assert!(classify(&args(&["issue", "list"])).is_none());
        // Non-comment review shapes stay unledgered in this slice.
        assert!(classify(&args(&["pr", "review", "12", "--approve"])).is_none());
        assert!(
            classify(&args(&[
                "pr",
                "review",
                "12",
                "--request-changes",
                "--body",
                "x"
            ]))
            .is_none()
        );
        assert!(classify(&args(&["pr", "review", "12"])).is_none());
        assert!(classify(&args(&["issue", "view", "12"])).is_none());
        assert!(classify(&args(&["pr", "merge", "12"])).is_none());
        assert!(classify(&args(&["pr", "status"])).is_none());
        assert!(classify(&args(&["api", "repos/o/r/issues", "-f", "title=x"])).is_none());
        // Non-POST `api` methods never create: GET reads, PUT/PATCH/DELETE
        // mutate in place — both stay unledgered in this slice, as does a
        // POST with no endpoint to file against.
        assert!(classify(&args(&["api", "repos/o/r/issues"])).is_none());
        assert!(classify(&args(&["api", "repos/o/r/issues", "--method", "GET"])).is_none());
        assert!(
            classify(&args(&[
                "api",
                "repos/o/r/issues/1",
                "--method",
                "PUT",
                "-f",
                "title=x"
            ]))
            .is_none()
        );
        assert!(classify(&args(&["api", "--method", "POST", "-f", "title=x"])).is_none());
        // Deferred shapes stay unledgered in this slice.
        assert!(classify(&args(&["gist", "view", "abc123"])).is_none());
        assert!(classify(&args(&["release", "view", "v1.0.0"])).is_none());
        assert!(classify(&args(&["frobnicate", "create"])).is_none());
        assert!(classify(&args(&[])).is_none());
        assert!(classify(&args(&["issue"])).is_none());
    }

    // oracle: repo comes from -R/--repo in any spelling, else the
    // call is recorded as not naming one.
    #[test]
    fn classify_repo_extraction() {
        let c = classify(&args(&["issue", "create", "-R", "o/r", "--title", "x"])).unwrap();
        assert_eq!(c.repo, "o/r");
        let c = classify(&args(&[
            "pr", "comment", "1", "--repo", "o/r", "--body", "x",
        ]))
        .unwrap();
        assert_eq!(c.repo, "o/r");
        let c = classify(&args(&["issue", "create", "--repo=o/r"])).unwrap();
        assert_eq!(c.repo, "o/r");
        let c = classify(&args(&["issue", "create", "--title", "x"])).unwrap();
        assert_eq!(c.repo, "unknown");
    }

    // oracle: the reservation invariant — reserve writes a
    // `reserved` row before the call; exactly one finalise flips
    // it; a second finalise errors instead of duplicating.
    #[test]
    fn reserve_then_finalize_roundtrip() {
        let path = temp_ledger("roundtrip");
        let (row, nonce) = reserve(&path, "actor", "job", "o/r", "issue").unwrap();
        assert!(!nonce.is_empty());
        assert_eq!(row_outcome(&path, row).0, "reserved");
        finalize(
            &path,
            row,
            "created",
            Some("https://github.com/o/r/issues/1"),
            None,
        )
        .unwrap();
        let (outcome, rref, _) = row_outcome(&path, row);
        assert_eq!(outcome, "created");
        assert_eq!(rref.as_deref(), Some("https://github.com/o/r/issues/1"));
        let err = finalize(&path, row, "failed", None, None).unwrap_err();
        assert!(
            err.contains("no reserved row"),
            "double finalise must error: {err}"
        );
    }

    // oracle: the guarded vocabulary — unknown kinds never
    // reserve, unknown outcomes never finalise.
    #[test]
    fn guarded_vocabulary() {
        let path = temp_ledger("vocab");
        let err = reserve(&path, "a", "j", "o/r", "bogus-kind").unwrap_err();
        assert!(err.contains("unknown ledger kind"), "{err}");
        // `review` is in the reference vocabulary and reserves.
        let (review_row, _) = reserve(&path, "a", "j", "o/r", "review").unwrap();
        assert_eq!(row_outcome(&path, review_row).0, "reserved");
        // `release` extends the vocabulary (see the header) and reserves.
        let (release_row, _) = reserve(&path, "a", "j", "o/r", "release").unwrap();
        assert_eq!(row_outcome(&path, release_row).0, "reserved");
        // `gist` extends the vocabulary (see the header) and reserves.
        let (gist_row, _) = reserve(&path, "a", "j", "o/r", "gist").unwrap();
        assert_eq!(row_outcome(&path, gist_row).0, "reserved");
        // `other` is the reference `KINDS` catch-all for POST-class
        // `api` calls and reserves.
        let (other_row, _) = reserve(&path, "a", "j", "o/r", "other").unwrap();
        assert_eq!(row_outcome(&path, other_row).0, "reserved");
        let (row, _) = reserve(&path, "a", "j", "o/r", "pr").unwrap();
        let err = finalize(&path, row, "maybe", None, None).unwrap_err();
        assert!(err.contains("unknown ledger outcome"), "{err}");
        assert_eq!(row_outcome(&path, row).0, "reserved");
    }

    // oracle: the decide table — the 2026-10-28 mode (exit 0,
    // empty output) is failed and loud; a URL is created with
    // the ref; failures name the exit.
    #[test]
    fn decide_table() {
        let d = decide(1, Some(b"error\n"));
        assert_eq!(d.outcome, "failed");
        assert!(d.notice.is_none());

        let d = decide(0, Some(b"https://github.com/o/r/issues/12\n"));
        assert_eq!(d.outcome, "created");
        assert_eq!(d.rref.as_deref(), Some("https://github.com/o/r/issues/12"));

        let d = decide(0, Some(b"   \n"));
        assert_eq!(d.outcome, "failed");
        let notice = d.notice.expect("empty-output success must be loud");
        assert!(notice.contains("empty output"), "{notice}");

        let d = decide(0, Some(b"147\n"));
        assert_eq!(d.outcome, "created");
        assert!(d.rref.is_none());
        assert!(d.detail.unwrap_or_default().contains("no item URL"));

        let d = decide(0, None);
        assert_eq!(d.outcome, "created");
    }

    // oracle: the batch shape — N creates through reserve+finish
    // leave exactly N finalized rows with the right tally, the
    // unit-scale form of the 100-create replay the acceptance
    // criteria demand of the wrapper.
    #[test]
    fn batch_replay_shape() {
        let path = temp_ledger("batch");
        let mut rows = Vec::new();
        for i in 0..50 {
            let (row, _) = reserve(&path, "a", "j", "o/r", "issue").unwrap();
            rows.push((row, i));
        }
        assert_eq!(count_rows(&path), 50);
        for (row, i) in rows {
            if i % 5 == 4 {
                // every fifth create fails visibly
                let notice = finish(
                    &Pending {
                        path: path.clone(),
                        row,
                    },
                    1,
                    Some(b"error"),
                )
                .unwrap();
                assert!(notice.is_none());
            } else {
                finish(
                    &Pending {
                        path: path.clone(),
                        row,
                    },
                    0,
                    Some(b"https://github.com/o/r/issues/9\n"),
                )
                .unwrap();
            }
        }
        let conn = rusqlite::Connection::open(&path).unwrap();
        let mut tally: std::collections::HashMap<String, i64> = Default::default();
        let mut stmt = conn
            .prepare("SELECT outcome, COUNT(*) FROM events GROUP BY outcome")
            .unwrap();
        let _ = stmt
            .query_map([], |r| {
                Ok((r.get::<_, String>(0).unwrap(), r.get::<_, i64>(1).unwrap()))
            })
            .unwrap()
            .map(|r| {
                let (o, n) = r.unwrap();
                tally.insert(o, n);
            })
            .count();
        assert_eq!(tally.get("created"), Some(&40));
        assert_eq!(tally.get("failed"), Some(&10));
        assert_eq!(tally.get("reserved"), None, "no row left reserved");
    }
}
