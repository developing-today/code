//! Filesystem-backed Atlas sweep discovery adapters.
//!
//! Adapters only discover and classify source objects; the sweep core
//! still decides accept/reject/hold, and staging remains a separate
//! call through [`crate::stage::stage_sweep_item`].

use std::io::Read;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::shards::Tier;
use crate::sweep::{CandidateFacts, OVERSIZE_BYTES};

/// A discovered source object paired with the facts consumed by the
/// deterministic sweep filter and the text passed to staging on accept.
#[derive(Clone, Debug)]
pub struct DiscoveredItem {
    pub facts: CandidateFacts,
    pub text: String,
    pub sensitivity: Tier,
}

/// Discover final and in-flight report files beneath a watched root.
/// Traversal is deterministic, never follows symlinks, and delegates
/// sensitivity classification to the caller (there is no implicit
/// downgrade to internal/public). Files ending in `.tmp` are still
/// surfaced with `partial_write = true` so the core holds them.
pub fn discover_reports(
    root: &Path,
    mut classify_sensitivity: impl FnMut(&Path) -> Tier,
) -> Result<Vec<DiscoveredItem>> {
    let root = std::fs::canonicalize(root).map_err(|e| {
        Error::backend(format!(
            "sweep reports: resolving root {}: {e}",
            root.display()
        ))
    })?;
    let mut files = Vec::new();
    collect_report_files(&root, &mut files)?;
    files.sort();

    let mut items = Vec::with_capacity(files.len());
    for path in files {
        let source_ref = path.to_string_lossy().into_owned();
        let sensitivity = classify_sensitivity(&path);
        let partial_write = path.extension().is_some_and(|ext| ext == "tmp");
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|e| Error::backend(format!("sweep reports: stat {}: {e}", path.display())))?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            continue;
        }
        let mut bytes = metadata.len();
        // Do not read held, secret, or oversized source data. Apart
        // from enforcing the design's no-partial-read ceiling, this
        // prevents the adapter from consuming content before the
        // sweep core's secret/partial holds take effect.
        let must_not_read = sensitivity == Tier::Secret || partial_write || bytes > OVERSIZE_BYTES;
        let (text, content_address) = if must_not_read {
            (String::new(), None)
        } else {
            let file = open_report_file(&path, &root)?;
            let opened_metadata = file.metadata().map_err(|e| {
                Error::backend(format!(
                    "sweep reports: stat opened file {}: {e}",
                    path.display()
                ))
            })?;
            if !opened_metadata.is_file() {
                continue;
            }
            bytes = opened_metadata.len();
            if bytes > OVERSIZE_BYTES {
                (String::new(), None)
            } else {
                let mut bounded = file.take(OVERSIZE_BYTES + 1);
                let mut contents = Vec::with_capacity(bytes as usize);
                bounded.read_to_end(&mut contents).map_err(|e| {
                    Error::backend(format!("sweep reports: reading {}: {e}", path.display()))
                })?;
                bytes = contents.len() as u64;
                if bytes > OVERSIZE_BYTES {
                    (String::new(), None)
                } else {
                    let text = String::from_utf8(contents).map_err(|e| {
                        Error::backend(format!(
                            "sweep reports: decoding {} as UTF-8: {e}",
                            path.display()
                        ))
                    })?;
                    let content_address = blake3::hash(text.as_bytes()).to_hex().to_string();
                    (text, Some(content_address))
                }
            }
        };
        items.push(DiscoveredItem {
            facts: CandidateFacts {
                source_ref,
                bytes,
                content_address,
                secret_tier: sensitivity == Tier::Secret,
                partial_write,
                ..CandidateFacts::default()
            },
            text,
            sensitivity,
        });
    }
    Ok(items)
}

fn open_report_file(path: &Path, root: &Path) -> Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|e| {
            Error::backend(format!(
                "sweep reports: opening {} safely: {e}",
                path.display()
            ))
        })?;

    // Validate the opened descriptor, not just the path observed during
    // traversal: a parent directory may have been swapped for a symlink.
    #[cfg(target_os = "linux")]
    {
        let opened_path = std::fs::canonicalize(format!(
            "/proc/self/fd/{}",
            std::os::fd::AsRawFd::as_raw_fd(&file)
        ))
        .map_err(|e| {
            Error::backend(format!(
                "sweep reports: resolving opened file {}: {e}",
                path.display()
            ))
        })?;
        if !opened_path.starts_with(root) {
            return Err(Error::fatal(format!(
                "sweep reports: opened path escaped watched root: {}",
                path.display()
            )));
        }
    }
    Ok(file)
}

fn collect_report_files(path: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|e| Error::backend(format!("sweep reports: stat {}: {e}", path.display())))?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    if metadata.is_file() {
        out.push(path.to_path_buf());
        return Ok(());
    }
    if !metadata.is_dir() {
        return Ok(());
    }
    let entries = std::fs::read_dir(path)
        .map_err(|e| Error::backend(format!("sweep reports: listing {}: {e}", path.display())))?;
    for entry in entries {
        let entry = entry.map_err(|e| {
            Error::backend(format!("sweep reports: listing {}: {e}", path.display()))
        })?;
        collect_report_files(&entry.path(), out)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage::{provision_staging_db, stage_sweep_item};
    use crate::sweep::{FilterOutcome, ReasonCode, SourceClass, filter_candidate};
    use rusqlite::Connection;
    use std::collections::HashSet;

    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    /// Oracle: fixture metadata and §2 hold outcomes; held files
    /// carry no source text or content hash.
    #[test]
    fn report_discovery_sorts_hashes_and_preserves_partial_and_secret_facts() {
        let root =
            std::env::temp_dir().join(format!("atlas-report-adapter-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("nested")).expect("fixture dirs");
        std::fs::write(root.join("z.md"), "final report").expect("final report");
        std::fs::write(root.join("nested/a.md.tmp"), "partial report").expect("partial report");
        let items = discover_reports(&root, |path| {
            if path.file_name().is_some_and(|name| name == "z.md") {
                Tier::Secret
            } else {
                Tier::Internal
            }
        })
        .expect("discover reports");

        assert_eq!(items.len(), 2);
        assert!(items[0].facts.source_ref.ends_with("nested/a.md.tmp"));
        assert!(items[0].facts.partial_write);
        assert_eq!(items[0].facts.bytes, "partial report".len() as u64);
        assert!(items[0].text.is_empty(), "partial content is never read");
        assert!(matches!(
            filter_candidate(SourceClass::Reports, &items[0].facts, &HashSet::new()),
            FilterOutcome::Hold(None)
        ));
        assert!(items[1].facts.secret_tier);
        assert_eq!(items[1].sensitivity, Tier::Secret);
        assert!(items[1].text.is_empty(), "secret content is never read");
        assert_eq!(
            filter_candidate(SourceClass::Reports, &items[1].facts, &HashSet::new()),
            FilterOutcome::Hold(Some(ReasonCode::SecretTierHeld))
        );
        assert_eq!(items[1].facts.content_address, None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    /// Oracle: a root containing only a symlink yields no candidates.
    #[test]
    fn report_discovery_does_not_follow_symlinks() {
        let root =
            std::env::temp_dir().join(format!("atlas-report-symlink-{}", std::process::id()));
        let outside =
            std::env::temp_dir().join(format!("atlas-report-outside-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(&outside);
        std::fs::create_dir_all(&root).expect("fixture dir");
        std::fs::write(&outside, "outside report").expect("outside report");
        symlink(&outside, root.join("linked.md")).expect("symlink");
        let items = discover_reports(&root, |_| Tier::Internal).expect("discover reports");
        assert!(
            items.is_empty(),
            "symlink targets are not report candidates"
        );
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(&outside);
    }

    /// Oracle: the fixture's final report yields one database row
    /// across retries; the partial report remains held by §2.
    #[test]
    fn accepted_fixture_report_lands_once_while_partial_stays_held() {
        let root = std::env::temp_dir().join(format!("atlas-report-e2e-{}", std::process::id()));
        let db = root.with_extension("db");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(&db);
        std::fs::create_dir_all(&root).expect("fixture dir");
        std::fs::write(root.join("final.md"), "finished sweep report").expect("final report");
        std::fs::write(root.join("partial.md.tmp"), "unfinished").expect("partial report");
        provision_staging_db(&db).expect("provision fixture staging DB");
        let conn = Connection::open(&db).expect("open fixture staging DB");
        let items = discover_reports(&root, |_| Tier::Internal).expect("discover reports");
        let landed = HashSet::new();
        let accepted = items
            .iter()
            .find(|item| item.facts.source_ref.ends_with("final.md"))
            .expect("final item");
        assert_eq!(
            filter_candidate(SourceClass::Reports, &accepted.facts, &landed),
            FilterOutcome::Accept
        );
        let address = accepted.facts.content_address.as_deref().expect("hash");
        assert!(
            stage_sweep_item(
                &conn,
                "unit-sweep",
                SourceClass::Reports.name(),
                &accepted.facts.source_ref,
                address,
                &accepted.text,
                accepted.sensitivity,
                "2026-11-21T00:00:00Z",
            )
            .expect("first staging")
        );
        assert!(
            !stage_sweep_item(
                &conn,
                "unit-sweep",
                SourceClass::Reports.name(),
                &accepted.facts.source_ref,
                address,
                &accepted.text,
                accepted.sensitivity,
                "2026-11-21T00:01:00Z",
            )
            .expect("retry is idempotent")
        );
        let partial = items
            .iter()
            .find(|item| item.facts.partial_write)
            .expect("partial item");
        assert!(matches!(
            filter_candidate(SourceClass::Reports, &partial.facts, &landed),
            FilterOutcome::Hold(None)
        ));
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM items", [], |r| r.get(0))
            .expect("staged item count");
        assert_eq!(count, 1);
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(&db);
    }
}
