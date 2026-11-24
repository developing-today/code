//! Offline, replay-safe connector for the four fleet repositories' Markdown docs.
use std::path::PathBuf;
use rusqlite::Connection;
use serde::Deserialize;
use crate::error::{Error, Result};
use crate::shards::Tier;

pub const REPO_DOCS_UNIT: &str = "repo-docs";
pub const OWN_REPOS: [&str; 4] = ["developing-today/code", "developing-today-forks/opencode", "developing-today-forks/openchamber", "dezren39/fleet"];

/// An explicit local checkout; no remote discovery or fetching is performed.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepoRoot {
    pub repo: String,
    pub root: PathBuf,
}

/// Stage changed documents only, returning their stable item IDs.
pub fn ingest_repo_docs(_conn: &Connection, _roots: &[RepoRoot], _agent: Option<&str>, _sensitivity: Tier, _now: &str) -> Result<Vec<i64>> {
    Err(Error::fatal("repo-docs connector not implemented".to_string()))
}
