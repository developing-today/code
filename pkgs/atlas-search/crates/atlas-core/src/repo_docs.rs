//! Offline, replay-safe connector for the four fleet repositories' Markdown docs.
use crate::error::{Error, Result};
use crate::shards::Tier;
use crate::stage::{check_stage_schema, chunk_artifact_text, sha256_hex};
use crate::trace::rfc3339_utc;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

pub const REPO_DOCS_UNIT: &str = "repo-docs";
pub const OWN_REPOS: [&str; 4] = [
    "developing-today/code",
    "developing-today-forks/opencode",
    "developing-today-forks/openchamber",
    "dezren39/fleet",
];

/// An explicit local checkout; no remote discovery or fetching is performed.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepoRoot {
    pub repo: String,
    pub root: PathBuf,
}

/// Open an existing staging database without creating one on a mistyped path.
pub fn open_repo_docs_db(path: &Path) -> Result<Connection> {
    Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE).map_err(db_error)
}

/// Stage changed documents only, returning their stable item IDs.
pub fn ingest_repo_docs(
    conn: &Connection,
    roots: &[RepoRoot],
    agent: Option<&str>,
    sensitivity: Tier,
    now: &str,
) -> Result<Vec<i64>> {
    validate_roots(roots)?;
    // Complete discovery and reads before starting writes: malformed UTF-8,
    // inaccessible files or boundary violations cannot leave a partial scan.
    let mut documents = Vec::new();
    for root in roots {
        let mut files = Vec::new();
        discover(&root.root, Path::new(""), &mut files)?;
        for relative in files {
            let path = root.root.join(&relative);
            let metadata = std::fs::symlink_metadata(&path).map_err(io_error)?;
            if !metadata.is_file() {
                return Err(invalid("document is not a regular file"));
            }
            let text = std::fs::read_to_string(&path).map_err(io_error)?;
            let millis = metadata
                .modified()
                .map_err(io_error)?
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| invalid(&format!("invalid document modification time: {e}")))?
                .as_millis();
            let relative = relative
                .to_str()
                .ok_or_else(|| invalid("non-UTF-8 document path"))?
                .to_string();
            documents.push((
                root.repo.clone(),
                relative,
                text,
                rfc3339_utc(
                    millis
                        .try_into()
                        .map_err(|_| invalid("modification time overflow"))?,
                ),
            ));
        }
    }
    let tx = conn.unchecked_transaction().map_err(db_error)?;
    check_stage_schema(&tx)?;
    let mut changed = Vec::new();
    for (repo, path, text, updated_at) in documents {
        let source = format!("{repo}:{path}");
        let chunks = chunk_artifact_text(&text);
        let meta = serde_json::json!({
            "corpus": REPO_DOCS_UNIT, "agent": agent, "source_ref": source,
            "updated_at": updated_at, "sensitivity": sensitivity.name(), "zone": "fleet",
            "repo": repo, "path": path, "language": "markdown", "chunks": chunks.len()
        })
        .to_string();
        let existing = tx.query_row(
            "SELECT id,text,meta,sensitivity FROM items WHERE unit_id=?1 AND kind='prose' AND source=?2",
            params![REPO_DOCS_UNIT, source],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, String>(3)?))
        ).optional().map_err(db_error)?;
        let id = if let Some((id, old_text, old_meta, old_tier)) = existing {
            if old_text == text
                && old_meta.as_deref() == Some(meta.as_str())
                && old_tier == sensitivity.name()
            {
                continue;
            }
            tx.execute(
                "UPDATE items SET text=?1,meta=?2,sensitivity=?3,created_at=?4 WHERE id=?5",
                params![text, meta, sensitivity.name(), updated_at, id],
            )
            .map_err(db_error)?;
            tx.execute("DELETE FROM chunks_raw WHERE item_id=?1", [id])
                .map_err(db_error)?;
            id
        } else {
            tx.execute("INSERT INTO items(unit_id,kind,source,text,sensitivity,created_at,meta) VALUES (?1,'prose',?2,?3,?4,?5,?6)", params![REPO_DOCS_UNIT, source, text, sensitivity.name(), updated_at, meta]).map_err(db_error)?;
            tx.last_insert_rowid()
        };
        for (idx, chunk) in chunks.iter().enumerate() {
            tx.execute("INSERT INTO chunks_raw(item_id,idx,text,content_hash,created_at) VALUES (?1,?2,?3,?4,?5)", params![id, idx as i64, chunk, sha256_hex(chunk), now]).map_err(db_error)?;
        }
        changed.push(id);
    }
    tx.commit().map_err(db_error)?;
    Ok(changed)
}

fn invalid(message: &str) -> Error {
    Error::fatal(format!("repo-docs: {message}"))
}
fn io_error(e: std::io::Error) -> Error {
    Error::backend(format!("repo-docs filesystem: {e}"))
}
fn db_error(e: rusqlite::Error) -> Error {
    Error::backend(format!("repo-docs staging: {e}"))
}

fn validate_roots(roots: &[RepoRoot]) -> Result<()> {
    if roots.is_empty() {
        return Err(invalid("at least one explicit repo root is required"));
    }
    let mut repos = BTreeSet::new();
    for root in roots {
        if !OWN_REPOS.contains(&root.repo.as_str()) || !repos.insert(&root.repo) {
            return Err(invalid("unknown or duplicate fleet repo ID"));
        }
        if !root.root.is_absolute()
            || root
                .root
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        {
            return Err(invalid(
                "root must be an absolute path without traversal components",
            ));
        }
        // Reject symlinks in ancestors too, not just the checkout's last component.
        for ancestor in root.root.ancestors() {
            let metadata = std::fs::symlink_metadata(ancestor).map_err(io_error)?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(invalid(
                    "root and ancestors must be real directories, not symlinks",
                ));
            }
        }
    }
    for (i, a) in roots.iter().enumerate() {
        for b in &roots[i + 1..] {
            if a.root.starts_with(&b.root) || b.root.starts_with(&a.root) {
                return Err(invalid("repo roots must not overlap"));
            }
        }
    }
    Ok(())
}

fn discover(root: &Path, relative: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    let mut entries = std::fs::read_dir(root.join(relative))
        .map_err(io_error)?
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(io_error)?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or_else(|| invalid("non-UTF-8 path in documentation tree"))?;
        if name.starts_with('.') || matches!(name, "node_modules" | "target" | "vendor") {
            continue;
        }
        let path = relative.join(name);
        let parts: Vec<_> = path.iter().filter_map(|p| p.to_str()).collect();
        let in_package = matches!(parts.first(), Some(&"pkgs") | Some(&"packages"));
        let in_docs = parts.iter().any(|p| matches!(*p, "doc" | "docs"));
        let markdown = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("md"));
        let readme = name.eq_ignore_ascii_case("README.md");
        let candidate =
            markdown && (in_docs || (readme && (relative.as_os_str().is_empty() || in_package)));
        let descend = in_docs
            || in_package
            || (relative.as_os_str().is_empty() && matches!(name, "doc" | "docs"));
        if !candidate && !descend {
            continue;
        }
        let ty = entry.file_type().map_err(io_error)?;
        if ty.is_symlink() {
            return Err(invalid(&format!(
                "symlink in approved documentation tree: {}",
                path.display()
            )));
        }
        if ty.is_dir() && descend {
            discover(root, &path, files)?;
        } else if ty.is_file() && candidate {
            files.push(path);
        } else if candidate && !ty.is_dir() {
            return Err(invalid("document is not a regular file"));
        }
    }
    Ok(())
}
