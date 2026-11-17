//! atlas-github-comments — the recurring GitHub-comments
//! extraction feed for Atlas (#6023).
//!
//! Usage: `atlas-github-comments --state <watermarks.json>
//! --out <records.jsonl> [--repo <owner/name>]... [--skip-reviews]
//! [--observed-date <YYYY-MM-DD>]`
//!
//! With no `--repo`, the estate is enumerated from the GitHub API
//! (the `developing-today` and `developing-today-forks` orgs and
//! the `dezren39` account — the our-repos estate of #6023).
//! External repos are never extracted here: that half is gated
//! on the #6024 licensing research.
//!
//! Extraction is gh-API only and incremental: per repo it reads
//! the thread list, the repo-wide issue-comment and
//! review-comment streams (bounded by the stored `updated_at`
//! watermark), and each PR's reviews, converts comments to
//! `atlas-github-comment/v1` records via
//! `atlas_core::github_comments`, appends them to `--out`, and
//! persists the advanced watermarks to `--state` after every
//! repo, so an interrupted run resumes where it stopped. Counts
//! per repo are printed as `<repo>: issue_comments=N
//! review_comments=N reviews=N documents=N`. Requests are issued
//! sequentially — that is the rate-limit discipline: no parallel
//! pressure on the API, and any gh failure stops the run with
//! the state saved and a non-zero exit. One exception: a repo
//! GitHub has access-blocked (HTTP 451, e.g. a DMCA block) can
//! never be extracted, so it is logged as skipped and the run
//! continues — otherwise one permanently blocked repo (found:
//! dezren39/edsebooks, blocked since 2019) would stop the whole
//! estate feed at the same repo on every run.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, ExitCode};

use atlas_core::github_comments::{
    CommentKind, CommentWatermarks, GhComment, GhThread, comment_document, document_to_record,
};
use serde_json::Value as Json;

struct Args {
    state: PathBuf,
    out: PathBuf,
    repos: Vec<String>,
    skip_reviews: bool,
    observed_date: Option<String>,
}

fn parse_args() -> Result<Args, String> {
    let mut state: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut repos: Vec<String> = Vec::new();
    let mut skip_reviews = false;
    let mut observed_date: Option<String> = None;
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--state" => state = Some(PathBuf::from(it.next().ok_or("--state needs a path")?)),
            "--out" => out = Some(PathBuf::from(it.next().ok_or("--out needs a path")?)),
            "--repo" => repos.push(it.next().ok_or("--repo needs owner/name")?),
            "--skip-reviews" => skip_reviews = true,
            "--observed-date" => {
                observed_date = Some(it.next().ok_or("--observed-date needs YYYY-MM-DD")?)
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(Args {
        state: state.ok_or("--state is required")?,
        out: out.ok_or("--out is required")?,
        repos,
        skip_reviews,
        observed_date,
    })
}

/// Run `gh api <path> --paginate --jq <jq>` and return stdout.
/// gh is the only GitHub client this feed uses.
fn gh_api(path: &str, jq: &str) -> Result<String, String> {
    let output = Command::new("gh")
        .args(["api", path, "--paginate", "--jq", jq])
        .output()
        .map_err(|e| format!("gh api {path}: spawn failed: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "gh api {path}: exited {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout).map_err(|e| format!("gh api {path}: non-UTF8 output: {e}"))
}

/// `gh api` with one JSON object per line (`--jq '.[]'`), parsed.
fn gh_api_objects(path: &str) -> Result<Vec<Json>, String> {
    let text = gh_api(path, ".[]")?;
    let mut objects = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        objects.push(
            serde_json::from_str(line)
                .map_err(|e| format!("gh api {path}: unparseable line: {e}"))?,
        );
    }
    Ok(objects)
}

fn str_field(obj: &Json, key: &str) -> String {
    obj.get(key)
        .and_then(Json::as_str)
        .unwrap_or("")
        .to_string()
}

/// The thread number a comment API object points at, from its
/// `issue_url` / `pull_request_url` (both end in `/<number>`).
fn thread_number(obj: &Json, key: &str) -> Option<u64> {
    let url = obj.get(key)?.as_str()?;
    url.rsplit('/').next()?.parse().ok()
}

fn enumerate_estate() -> Result<Vec<String>, String> {
    let mut repos: Vec<String> = Vec::new();
    for path in [
        "orgs/developing-today/repos?per_page=100",
        "orgs/developing-today-forks/repos?per_page=100",
        "users/dezren39/repos?per_page=100&type=owner",
    ] {
        let text = gh_api(path, ".[].full_name")?;
        for line in text.lines() {
            let name = line.trim();
            if !name.is_empty() && !repos.iter().any(|r| r == name) {
                repos.push(name.to_string());
            }
        }
    }
    repos.sort();
    Ok(repos)
}

fn load_watermarks(path: &PathBuf) -> CommentWatermarks {
    match fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str::<Json>(&text) {
            Ok(value) => CommentWatermarks::from_json(&value),
            Err(_) => CommentWatermarks::new(),
        },
        Err(_) => CommentWatermarks::new(),
    }
}

fn save_watermarks(path: &PathBuf, state: &CommentWatermarks) -> Result<(), String> {
    let tmp = path.with_extension("json.tmp");
    fs::write(
        &tmp,
        serde_json::to_string_pretty(&state.to_json()).unwrap(),
    )
    .map_err(|e| format!("write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| format!("rename to {}: {e}", path.display()))
}

struct RepoCounts {
    issue_comments: usize,
    review_comments: usize,
    reviews: usize,
    documents: usize,
}

fn extract_repo(
    repo: &str,
    state: &mut CommentWatermarks,
    skip_reviews: bool,
    observed_date: &str,
    out: &mut fs::File,
) -> Result<RepoCounts, String> {
    let since = state.since_for(repo).map(str::to_string);
    let since_q = since
        .as_deref()
        .map(|s| format!("&since={s}"))
        .unwrap_or_default();

    // Thread map: the issues list carries PRs too.
    let issues = gh_api_objects(&format!("repos/{repo}/issues?state=all&per_page=100"))?;
    let mut threads: BTreeMap<u64, GhThread> = BTreeMap::new();
    for issue in &issues {
        let Some(number) = issue.get("number").and_then(Json::as_u64) else {
            continue;
        };
        let labels = issue
            .get("labels")
            .and_then(Json::as_array)
            .map(|ls| {
                ls.iter()
                    .filter_map(|l| l.get("name").and_then(Json::as_str).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        threads.insert(
            number,
            GhThread {
                repo: repo.to_string(),
                number,
                is_pr: issue.get("pull_request").is_some(),
                title: str_field(issue, "title"),
                state: str_field(issue, "state"),
                labels,
            },
        );
    }

    let mut counts = RepoCounts {
        issue_comments: 0,
        review_comments: 0,
        reviews: 0,
        documents: 0,
    };
    let mut emit =
        |thread: &GhThread, comment: &GhComment, counts: &mut RepoCounts| -> Result<(), String> {
            let doc = comment_document(thread, comment, observed_date);
            let record = document_to_record(&doc);
            writeln!(out, "{record}").map_err(|e| format!("write records: {e}"))?;
            counts.documents += 1;
            Ok(())
        };

    // Issue comments (repo-wide stream, watermark-bounded).
    let raw = gh_api_objects(&format!(
        "repos/{repo}/issues/comments?per_page=100{since_q}"
    ))?;
    let comments: Vec<GhComment> = raw
        .iter()
        .filter_map(|c| {
            Some(GhComment {
                id: c.get("id")?.as_u64()?,
                kind: CommentKind::IssueComment,
                author: c.get("user")?.get("login")?.as_str()?.to_string(),
                body: str_field(c, "body"),
                created_at: str_field(c, "created_at"),
                updated_at: str_field(c, "updated_at"),
                url: str_field(c, "html_url"),
            })
        })
        .collect();
    for c in state.select_delta(repo, &comments) {
        let thread = raw
            .iter()
            .find(|r| r.get("id").and_then(Json::as_u64) == Some(c.id))
            .and_then(|r| thread_number(r, "issue_url"))
            .and_then(|n| threads.get(&n));
        if let Some(thread) = thread {
            emit(thread, c, &mut counts)?;
            counts.issue_comments += 1;
        }
        state.observe(repo, &c.updated_at);
    }

    // Review comments (repo-wide stream, watermark-bounded).
    let raw = gh_api_objects(&format!(
        "repos/{repo}/pulls/comments?per_page=100{since_q}"
    ))?;
    let comments: Vec<GhComment> = raw
        .iter()
        .filter_map(|c| {
            Some(GhComment {
                id: c.get("id")?.as_u64()?,
                kind: CommentKind::ReviewComment,
                author: c.get("user")?.get("login")?.as_str()?.to_string(),
                body: str_field(c, "body"),
                created_at: str_field(c, "created_at"),
                updated_at: str_field(c, "updated_at"),
                url: str_field(c, "html_url"),
            })
        })
        .collect();
    for c in state.select_delta(repo, &comments) {
        let thread = raw
            .iter()
            .find(|r| r.get("id").and_then(Json::as_u64) == Some(c.id))
            .and_then(|r| thread_number(r, "pull_request_url"))
            .and_then(|n| threads.get(&n));
        if let Some(thread) = thread {
            emit(thread, c, &mut counts)?;
            counts.review_comments += 1;
        }
        state.observe(repo, &c.updated_at);
    }

    // Reviews: no repo-wide stream exists, so walk the PR
    // threads. Reviews carry submitted_at only; it serves as
    // both timestamps and the watermark value.
    if !skip_reviews {
        let pr_numbers: Vec<u64> = threads
            .values()
            .filter(|t| t.is_pr)
            .map(|t| t.number)
            .collect();
        for number in pr_numbers {
            let raw = gh_api_objects(&format!("repos/{repo}/pulls/{number}/reviews?per_page=100"))?;
            let reviews: Vec<GhComment> = raw
                .iter()
                .filter_map(|r| {
                    let submitted = r.get("submitted_at")?.as_str()?;
                    Some(GhComment {
                        id: r.get("id")?.as_u64()?,
                        kind: CommentKind::Review,
                        author: r.get("user")?.get("login")?.as_str()?.to_string(),
                        body: str_field(r, "body"),
                        created_at: submitted.to_string(),
                        updated_at: submitted.to_string(),
                        url: str_field(r, "html_url"),
                    })
                })
                .collect();
            for c in state.select_delta(repo, &reviews) {
                if let Some(thread) = threads.get(&number) {
                    emit(thread, c, &mut counts)?;
                    counts.reviews += 1;
                }
                state.observe(repo, &c.updated_at);
            }
        }
    }

    Ok(counts)
}

fn run() -> Result<(), String> {
    let args = parse_args()?;
    let observed_date = match &args.observed_date {
        Some(d) => d.clone(),
        None => {
            let output = Command::new("date")
                .args(["-u", "+%F"])
                .output()
                .map_err(|e| format!("date: {e}"))?;
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        }
    };
    let repos = if args.repos.is_empty() {
        enumerate_estate()?
    } else {
        args.repos.clone()
    };
    let mut state = load_watermarks(&args.state);
    let mut out = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&args.out)
        .map_err(|e| format!("open {}: {e}", args.out.display()))?;

    println!("estate: {} repos", repos.len());
    let mut totals = (0usize, 0usize, 0usize, 0usize);
    let mut skipped = 0usize;
    for repo in &repos {
        match extract_repo(
            repo,
            &mut state,
            args.skip_reviews,
            &observed_date,
            &mut out,
        ) {
            Ok(counts) => {
                save_watermarks(&args.state, &state)?;
                println!(
                    "{repo}: issue_comments={} review_comments={} reviews={} documents={}",
                    counts.issue_comments, counts.review_comments, counts.reviews, counts.documents
                );
                totals.0 += counts.issue_comments;
                totals.1 += counts.review_comments;
                totals.2 += counts.reviews;
                totals.3 += counts.documents;
            }
            Err(e) if is_access_blocked(&e) => {
                // Permanently access-blocked repo (HTTP 451): no
                // extraction is possible, now or on a rerun. Log
                // the skip and continue; the watermark is left
                // unadvanced, so if the block is ever lifted a
                // later run extracts the repo in full.
                save_watermarks(&args.state, &state)?;
                println!("{repo}: skipped — repository access blocked (HTTP 451)");
                skipped += 1;
            }
            Err(e) => {
                save_watermarks(&args.state, &state)?;
                println!(
                    "totals: issue_comments={} review_comments={} reviews={} documents={}",
                    totals.0, totals.1, totals.2, totals.3
                );
                return Err(format!("{repo}: {e} (state saved; rerun resumes)"));
            }
        }
    }
    println!(
        "totals: issue_comments={} review_comments={} reviews={} documents={}",
        totals.0, totals.1, totals.2, totals.3
    );
    if skipped > 0 {
        println!("skipped_repos: {skipped} (repository access blocked, HTTP 451)");
    }
    Ok(())
}

/// True when a gh failure is GitHub's permanent access block for
/// a repo (HTTP 451, e.g. a DMCA takedown), as opposed to a
/// transient or auth failure that must stay fatal.
fn is_access_blocked(error: &str) -> bool {
    error.contains("Repository access blocked") || error.contains("HTTP 451")
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("atlas-github-comments: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::is_access_blocked;

    #[test]
    fn dmca_blocked_repo_error_is_access_blocked() {
        // Oracle: the verbatim gh failure for dezren39/edsebooks
        // (DMCA block, 2019) must be classified skippable.
        let e = "gh api repos/dezren39/edsebooks/issues?state=all&per_page=100: \
                   exited exit status: 1: gh: Repository access blocked (HTTP 451)";
        assert!(is_access_blocked(e));
    }

    #[test]
    fn ordinary_gh_failures_stay_fatal() {
        // Oracle: rate-limit / auth / not-found failures are NOT
        // access blocks and must keep stopping the run.
        assert!(!is_access_blocked(
            "gh api x: exited exit status: 1: gh: API rate limit exceeded"
        ));
        assert!(!is_access_blocked(
            "gh api x: exited exit status: 1: gh: Not Found (HTTP 404)"
        ));
        assert!(!is_access_blocked(
            "gh api x: spawn failed: No such file or directory"
        ));
    }
}
