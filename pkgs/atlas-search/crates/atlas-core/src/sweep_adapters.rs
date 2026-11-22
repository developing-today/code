//! Filesystem-backed Atlas sweep discovery adapters.
//!
//! Adapters only discover and classify source objects; the sweep core
//! still decides accept/reject/hold, and staging remains a separate
//! call through [`crate::stage::stage_sweep_item`].

use std::io::Read;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::shards::Tier;
use crate::stage::stage_sweep_item;
use crate::sweep::{CandidateFacts, OVERSIZE_BYTES, RunOutcome, RunRequest, run_once_with_landing};
use rusqlite::Connection;

/// A discovered source object paired with the facts consumed by the
/// deterministic sweep filter and the text passed to staging on accept.
#[derive(Clone, Debug)]
pub struct DiscoveredItem {
    pub facts: CandidateFacts,
    pub text: String,
    pub sensitivity: Tier,
}

/// Run discovered candidates and stage each accepted item before the
/// sweep state records its content address. A failed staging write leaves
/// runner state retryable; staging itself is content-addressed and idempotent.
pub fn run_once_with_staging(
    state_dir: &Path,
    req: &RunRequest,
    items: &[DiscoveredItem],
    conn: &Connection,
    unit_id: &str,
) -> Result<RunOutcome> {
    run_once_with_landing(state_dir, req, |facts| {
        let item = items
            .iter()
            .find(|item| item.facts.source_ref == facts.source_ref)
            .ok_or_else(|| {
                Error::fatal(format!(
                    "sweep staging: no discovered item for accepted source {}",
                    facts.source_ref
                ))
            })?;
        let address = facts.content_address.as_deref().ok_or_else(|| {
            Error::fatal(format!(
                "sweep staging: accepted source {} has no content address",
                facts.source_ref
            ))
        })?;
        stage_sweep_item(
            conn,
            unit_id,
            req.source_class.name(),
            &facts.source_ref,
            address,
            &item.text,
            item.sensitivity,
            &req.at,
        )?;
        Ok(())
    })
}

/// Discover boot segments from JSONL init event files. Each event needs
/// a stable `boot_id` and `state`; only `success`, `failed`, or
/// `complete` closes a segment. Temporary files are emitted as held
/// candidates without reading their contents.
pub fn discover_boot_records(
    root: &Path,
    mut classify_sensitivity: impl FnMut(&Path) -> Tier,
) -> Result<Vec<DiscoveredItem>> {
    let root = std::fs::canonicalize(root).map_err(|e| {
        Error::backend(format!(
            "sweep boot: resolving root {}: {e}",
            root.display()
        ))
    })?;
    let mut files = Vec::new();
    collect_report_files(&root, &mut files)?;
    files.retain(|path| {
        path.file_name().is_some_and(|name| {
            name.to_string_lossy() == "init.jsonl" || name.to_string_lossy() == "init.jsonl.tmp"
        })
    });
    files.sort();

    let mut items = Vec::new();
    for path in files {
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|e| Error::backend(format!("sweep boot: stat {}: {e}", path.display())))?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            continue;
        }
        let sensitivity = classify_sensitivity(&path);
        let partial = path.extension().is_some_and(|ext| ext == "tmp");
        let mut bytes = metadata.len();
        if sensitivity == Tier::Secret || partial || bytes > OVERSIZE_BYTES {
            items.push(DiscoveredItem {
                facts: CandidateFacts {
                    source_ref: path.to_string_lossy().into_owned(),
                    bytes,
                    secret_tier: sensitivity == Tier::Secret,
                    ..CandidateFacts::default()
                },
                text: String::new(),
                sensitivity,
            });
            continue;
        }
        let file = open_report_file(&path, &root)?;
        let mut bounded = file.take(OVERSIZE_BYTES + 1);
        let mut contents = Vec::with_capacity(bytes as usize);
        bounded
            .read_to_end(&mut contents)
            .map_err(|e| Error::backend(format!("sweep boot: reading {}: {e}", path.display())))?;
        bytes = contents.len() as u64;
        if bytes > OVERSIZE_BYTES {
            items.push(DiscoveredItem {
                facts: CandidateFacts {
                    source_ref: path.to_string_lossy().into_owned(),
                    bytes,
                    ..CandidateFacts::default()
                },
                text: String::new(),
                sensitivity,
            });
            continue;
        }
        let text = String::from_utf8(contents).map_err(|e| {
            Error::backend(format!(
                "sweep boot: decoding {} as UTF-8: {e}",
                path.display()
            ))
        })?;
        let mut segments = std::collections::BTreeMap::<String, (Vec<String>, bool, u64)>::new();
        for (line_no, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let event: serde_json::Value = serde_json::from_str(line).map_err(|e| {
                Error::backend(format!(
                    "sweep boot: parsing {} line {}: {e}",
                    path.display(),
                    line_no + 1
                ))
            })?;
            let boot_id = event
                .get("boot_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    Error::fatal(format!(
                        "sweep boot: {} line {} has no boot_id",
                        path.display(),
                        line_no + 1
                    ))
                })?;
            let state = event.get("state").and_then(|v| v.as_str()).ok_or_else(|| {
                Error::fatal(format!(
                    "sweep boot: {} line {} has no state",
                    path.display(),
                    line_no + 1
                ))
            })?;
            let (events, terminal, segment_bytes) = segments.entry(boot_id.to_owned()).or_default();
            events.push(line.to_owned());
            *terminal |= matches!(state, "success" | "failed" | "complete");
            *segment_bytes += line.len() as u64 + 1;
        }
        for (boot_id, (events, terminal, segment_bytes)) in segments {
            let segment = events.join("\n");
            let address = blake3::hash(segment.as_bytes()).to_hex().to_string();
            items.push(DiscoveredItem {
                facts: CandidateFacts {
                    source_ref: format!("{}#boot_id={boot_id}", path.display()),
                    bytes: segment_bytes.min(bytes),
                    content_address: Some(address),
                    terminal,
                    has_substance: !events.is_empty(),
                    ..CandidateFacts::default()
                },
                text: segment,
                sensitivity,
            });
        }
    }
    Ok(items)
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

/// Discover Atlas weekly statistics artifacts. Files are grouped by the
/// `report_day` in their JSON payload; when more than one artifact names
/// the same period, only the newest complete file is marked final. Draft
/// and `.tmp` files are never read, and the caller remains responsible
/// for sensitivity classification.
pub fn discover_statistics_reports(
    root: &Path,
    mut classify_sensitivity: impl FnMut(&Path) -> Tier,
) -> Result<Vec<DiscoveredItem>> {
    let root = std::fs::canonicalize(root).map_err(|e| {
        Error::backend(format!(
            "sweep statistics: resolving root {}: {e}",
            root.display()
        ))
    })?;
    let mut files = Vec::new();
    collect_report_files(&root, &mut files)?;
    files.retain(|path| {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        name.starts_with("atlas-statistics-report-")
            && (name.ends_with(".jsonl") || name.ends_with(".jsonl.tmp"))
    });
    files.sort();

    let mut candidates = Vec::with_capacity(files.len());
    for path in files {
        let source_ref = path.to_string_lossy().into_owned();
        let sensitivity = classify_sensitivity(&path);
        let partial_write = path.extension().is_some_and(|ext| ext == "tmp");
        let metadata = std::fs::symlink_metadata(&path).map_err(|e| {
            Error::backend(format!("sweep statistics: stat {}: {e}", path.display()))
        })?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            continue;
        }
        let modified = metadata.modified().unwrap_or(std::time::UNIX_EPOCH);
        let mut bytes = metadata.len();
        let must_not_read = sensitivity == Tier::Secret || partial_write || bytes > OVERSIZE_BYTES;
        let (text, content_address, report_day) = if must_not_read {
            (String::new(), None, None)
        } else {
            let file = open_report_file(&path, &root)?;
            let opened_metadata = file.metadata().map_err(|e| {
                Error::backend(format!(
                    "sweep statistics: stat opened file {}: {e}",
                    path.display()
                ))
            })?;
            if !opened_metadata.is_file() {
                continue;
            }
            bytes = opened_metadata.len();
            if bytes > OVERSIZE_BYTES {
                (String::new(), None, None)
            } else {
                let mut bounded = file.take(OVERSIZE_BYTES + 1);
                let mut contents = Vec::with_capacity(bytes as usize);
                bounded.read_to_end(&mut contents).map_err(|e| {
                    Error::backend(format!("sweep statistics: reading {}: {e}", path.display()))
                })?;
                bytes = contents.len() as u64;
                if bytes > OVERSIZE_BYTES {
                    (String::new(), None, None)
                } else {
                    let text = String::from_utf8(contents).map_err(|e| {
                        Error::backend(format!(
                            "sweep statistics: decoding {} as UTF-8: {e}",
                            path.display()
                        ))
                    })?;
                    let record: serde_json::Value =
                        serde_json::from_str(text.trim()).map_err(|e| {
                            Error::backend(format!(
                                "sweep statistics: parsing {}: {e}",
                                path.display()
                            ))
                        })?;
                    let schema = record.get("schema").and_then(|v| v.as_str());
                    let report_day = record.get("report_day").and_then(|v| v.as_str());
                    if schema != Some(crate::trace_stats::REPORT_SCHEMA) || report_day.is_none() {
                        return Err(Error::fatal(format!(
                            "sweep statistics: {} is not a recognized statistics report",
                            path.display()
                        )));
                    }
                    let content_address = blake3::hash(text.as_bytes()).to_hex().to_string();
                    (text, Some(content_address), report_day.map(str::to_owned))
                }
            }
        };
        candidates.push((
            DiscoveredItem {
                facts: CandidateFacts {
                    source_ref,
                    bytes,
                    content_address,
                    secret_tier: sensitivity == Tier::Secret,
                    final_version: false,
                    partial_write,
                    ..CandidateFacts::default()
                },
                text,
                sensitivity,
            },
            report_day,
            modified,
        ));
    }

    let mut newest_by_period =
        std::collections::HashMap::<String, (usize, std::time::SystemTime)>::new();
    for (index, (_, day, modified)) in candidates.iter().enumerate() {
        let Some(day) = day else {
            continue;
        };
        let replace = newest_by_period.get(day).map_or(true, |(current, at)| {
            *modified > *at || (*modified == *at && index > *current)
        });
        if replace {
            newest_by_period.insert(day.to_owned(), (index, *modified));
        }
    }
    let mut items = Vec::with_capacity(candidates.len());
    for (index, (mut item, day, _)) in candidates.into_iter().enumerate() {
        if let Some(day) = day {
            item.facts.final_version = newest_by_period
                .get(&day)
                .is_some_and(|(newest_index, _)| index == *newest_index);
        }
        items.push(item);
    }
    Ok(items)
}

/// Discover finalized session snapshots (`*.session.json`) below a
/// watched root. The normalized snapshot format has `state` (one of
/// `closed`, `completed`, or `failed`) and a `messages` array. A message
/// is substantive only when its role is `user` or `assistant`, it is
/// not marked `synthetic`, and it has non-whitespace text. Secret and
/// oversized snapshots are never read.
pub fn discover_session_logs(
    root: &Path,
    mut classify_sensitivity: impl FnMut(&Path) -> Tier,
) -> Result<Vec<DiscoveredItem>> {
    let root = std::fs::canonicalize(root).map_err(|e| {
        Error::backend(format!(
            "sweep sessions: resolving root {}: {e}",
            root.display()
        ))
    })?;
    let mut files = Vec::new();
    collect_report_files(&root, &mut files)?;
    files.retain(|path| {
        path.file_name()
            .is_some_and(|name| name.to_string_lossy().ends_with(".session.json"))
    });
    files.sort();

    let mut items = Vec::with_capacity(files.len());
    for path in files {
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|e| Error::backend(format!("sweep sessions: stat {}: {e}", path.display())))?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            continue;
        }
        let sensitivity = classify_sensitivity(&path);
        let mut bytes = metadata.len();
        let (text, content_address, terminal, has_substance) = if sensitivity == Tier::Secret
            || bytes > OVERSIZE_BYTES
        {
            (String::new(), None, false, false)
        } else {
            let file = open_report_file(&path, &root)?;
            let mut bounded = file.take(OVERSIZE_BYTES + 1);
            let mut contents = Vec::with_capacity(bytes as usize);
            bounded.read_to_end(&mut contents).map_err(|e| {
                Error::backend(format!("sweep sessions: reading {}: {e}", path.display()))
            })?;
            bytes = contents.len() as u64;
            if bytes > OVERSIZE_BYTES {
                (String::new(), None, false, false)
            } else {
                let text = String::from_utf8(contents).map_err(|e| {
                    Error::backend(format!(
                        "sweep sessions: decoding {} as UTF-8: {e}",
                        path.display()
                    ))
                })?;
                let snapshot: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
                    Error::backend(format!("sweep sessions: parsing {}: {e}", path.display()))
                })?;
                let terminal = matches!(
                    snapshot.get("state").and_then(|v| v.as_str()),
                    Some("closed" | "completed" | "failed")
                );
                let has_substance = snapshot
                    .get("messages")
                    .and_then(|v| v.as_array())
                    .is_some_and(|messages| {
                        messages.iter().any(|message| {
                            matches!(
                                message.get("role").and_then(|v| v.as_str()),
                                Some("user" | "assistant")
                            ) && message.get("synthetic").and_then(|v| v.as_bool()) != Some(true)
                                && message
                                    .get("text")
                                    .and_then(|v| v.as_str())
                                    .is_some_and(|s| !s.trim().is_empty())
                        })
                    });
                let content_address = blake3::hash(text.as_bytes()).to_hex().to_string();
                (text, Some(content_address), terminal, has_substance)
            }
        };
        items.push(DiscoveredItem {
            facts: CandidateFacts {
                source_ref: path.to_string_lossy().into_owned(),
                bytes,
                content_address,
                secret_tier: sensitivity == Tier::Secret,
                terminal,
                has_substance,
                ..CandidateFacts::default()
            },
            text,
            sensitivity,
        });
    }
    Ok(items)
}

/// Discover one message per JSONL line from allowlisted channel folders
/// (`root/<channel>/.../*.jsonl`). The directory name is checked before
/// opening a file, so off-allowlist exports are not read at all. Each
/// message object uses `text`, `bot`, and `heartbeat` fields; source
/// adapters deliberately accept an explicit allowlist from the caller.
pub fn discover_chat_exports(
    root: &Path,
    allowlisted_channels: &std::collections::HashSet<String>,
    mut classify_sensitivity: impl FnMut(&Path) -> Tier,
) -> Result<Vec<DiscoveredItem>> {
    let root = std::fs::canonicalize(root).map_err(|e| {
        Error::backend(format!(
            "sweep chat: resolving root {}: {e}",
            root.display()
        ))
    })?;
    let mut files = Vec::new();
    collect_report_files(&root, &mut files)?;
    files.retain(|path| path.extension().is_some_and(|ext| ext == "jsonl"));
    files.sort();

    let mut items = Vec::new();
    for path in files {
        let Ok(relative) = path.strip_prefix(&root) else {
            continue;
        };
        let Some(channel) = relative
            .components()
            .next()
            .and_then(|part| part.as_os_str().to_str())
        else {
            continue;
        };
        if !allowlisted_channels.contains(channel) {
            continue;
        }
        let sensitivity = classify_sensitivity(&path);
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|e| Error::backend(format!("sweep chat: stat {}: {e}", path.display())))?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            continue;
        }
        if sensitivity == Tier::Secret || metadata.len() > OVERSIZE_BYTES {
            items.push(DiscoveredItem {
                facts: CandidateFacts {
                    source_ref: path.to_string_lossy().into_owned(),
                    bytes: metadata.len(),
                    secret_tier: sensitivity == Tier::Secret,
                    ..CandidateFacts::default()
                },
                text: String::new(),
                sensitivity,
            });
            continue;
        }
        let file = open_report_file(&path, &root)?;
        let mut bounded = file.take(OVERSIZE_BYTES + 1);
        let mut contents = Vec::with_capacity(metadata.len() as usize);
        bounded
            .read_to_end(&mut contents)
            .map_err(|e| Error::backend(format!("sweep chat: reading {}: {e}", path.display())))?;
        if contents.len() as u64 > OVERSIZE_BYTES {
            items.push(DiscoveredItem {
                facts: CandidateFacts {
                    source_ref: path.to_string_lossy().into_owned(),
                    bytes: contents.len() as u64,
                    ..CandidateFacts::default()
                },
                text: String::new(),
                sensitivity,
            });
            continue;
        }
        let text = String::from_utf8(contents).map_err(|e| {
            Error::backend(format!(
                "sweep chat: decoding {} as UTF-8: {e}",
                path.display()
            ))
        })?;
        for (line_no, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let message: serde_json::Value = serde_json::from_str(line).map_err(|e| {
                Error::backend(format!(
                    "sweep chat: parsing {} line {}: {e}",
                    path.display(),
                    line_no + 1
                ))
            })?;
            let body = line.to_owned();
            let address = blake3::hash(body.as_bytes()).to_hex().to_string();
            let bot_or_heartbeat = message.get("bot").and_then(|v| v.as_bool()) == Some(true)
                || message.get("heartbeat").and_then(|v| v.as_bool()) == Some(true);
            items.push(DiscoveredItem {
                facts: CandidateFacts {
                    source_ref: format!("{}#line={}", path.display(), line_no + 1),
                    bytes: body.len() as u64,
                    content_address: Some(address),
                    bot_or_heartbeat,
                    allowlisted: true,
                    ..CandidateFacts::default()
                },
                text: body,
                sensitivity,
            });
        }
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
    use crate::stage::provision_staging_db;
    use crate::sweep::{
        EnvelopeLine, FilterOutcome, Measurements, ReasonCode, RunRequest, SourceClass,
        filter_candidate, load_state,
    };
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

    /// Oracle: boot events group by id; terminal states accept while
    /// open and temporary segments remain held (without reading temp data).
    #[test]
    fn boot_discovery_groups_events_and_holds_open_segments() {
        let root = std::env::temp_dir().join(format!("atlas-boot-adapter-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("fixture dir");
        std::fs::write(
            root.join("init.jsonl"),
            "{\"boot_id\":\"b1\",\"state\":\"started\"}\n{\"boot_id\":\"b1\",\"state\":\"failed\"}\n{\"boot_id\":\"b2\",\"state\":\"started\"}\n",
        )
        .expect("boot events");
        std::fs::write(root.join("init.jsonl.tmp"), "not-json partial").expect("partial boot");

        let items = discover_boot_records(&root, |_| Tier::Internal).expect("discover boots");
        assert_eq!(items.len(), 3);
        let completed = items
            .iter()
            .find(|item| item.facts.source_ref.ends_with("#boot_id=b1"))
            .expect("terminal boot");
        assert!(completed.facts.terminal);
        assert_eq!(
            filter_candidate(SourceClass::BootRecords, &completed.facts, &HashSet::new()),
            FilterOutcome::Accept
        );
        let open = items
            .iter()
            .find(|item| item.facts.source_ref.ends_with("#boot_id=b2"))
            .expect("open boot");
        assert!(!open.facts.terminal);
        assert_eq!(
            filter_candidate(SourceClass::BootRecords, &open.facts, &HashSet::new()),
            FilterOutcome::Hold(None)
        );
        let partial = items
            .iter()
            .find(|item| item.facts.source_ref.ends_with("init.jsonl.tmp"))
            .expect("partial boot");
        assert!(partial.text.is_empty(), "partial segment was not read");
        assert_eq!(
            filter_candidate(SourceClass::BootRecords, &partial.facts, &HashSet::new()),
            FilterOutcome::Hold(None)
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Oracle: the newest complete statistics artifact per report period
    /// is final; older copies are superseded and partial files are not read.
    #[test]
    fn statistics_discovery_marks_one_final_per_period() {
        let root =
            std::env::temp_dir().join(format!("atlas-statistics-adapter-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("fixture dir");
        std::fs::create_dir_all(root.join("a")).expect("older fixture dir");
        std::fs::create_dir_all(root.join("z")).expect("nested fixture dir");
        let record = |detail: &str| {
            serde_json::json!({
                "schema": crate::trace_stats::REPORT_SCHEMA,
                "report_day": "2026-11-21",
                "detail": detail
            })
            .to_string()
        };
        std::fs::write(
            root.join("a/atlas-statistics-report-2026-11-21.jsonl"),
            record("older"),
        )
        .expect("older report");
        std::fs::write(
            root.join("z/atlas-statistics-report-2026-11-21.jsonl"),
            record("final"),
        )
        .expect("final report");
        std::fs::write(
            root.join("atlas-statistics-report-2026-11-21.jsonl.tmp"),
            "not complete json",
        )
        .expect("partial report");

        let items = discover_statistics_reports(&root, |_| Tier::Internal)
            .expect("discover statistics reports");
        assert_eq!(items.len(), 3);
        let final_item = items
            .iter()
            .find(|item| {
                item.facts
                    .source_ref
                    .ends_with("/z/atlas-statistics-report-2026-11-21.jsonl")
            })
            .expect("deterministic tie winner");
        assert!(final_item.facts.final_version);
        assert_eq!(
            filter_candidate(SourceClass::Statistics, &final_item.facts, &HashSet::new()),
            FilterOutcome::Accept
        );
        let superseded = items
            .iter()
            .find(|item| {
                item.facts
                    .source_ref
                    .ends_with("/a/atlas-statistics-report-2026-11-21.jsonl")
            })
            .expect("older report");
        assert!(!superseded.facts.final_version);
        assert_eq!(
            filter_candidate(SourceClass::Statistics, &superseded.facts, &HashSet::new()),
            FilterOutcome::Reject(ReasonCode::SupersededVersion)
        );
        let partial = items
            .iter()
            .find(|item| item.facts.partial_write)
            .expect("partial report");
        assert!(partial.text.is_empty(), "partial content is never read");
        assert!(!partial.facts.final_version);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Oracle: run-once stages accepted fixture content once, then the
    /// persisted sweep state reports it as duplicate-content on retry.
    #[test]
    fn accepted_fixture_report_lands_once_while_partial_stays_held() {
        let root = std::env::temp_dir().join(format!("atlas-report-e2e-{}", std::process::id()));
        let db = root.with_extension("db");
        let state_dir = root.with_extension("state");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&state_dir);
        let _ = std::fs::remove_file(&db);
        std::fs::create_dir_all(&root).expect("fixture dir");
        std::fs::write(root.join("final.md"), "finished sweep report").expect("final report");
        std::fs::write(root.join("partial.md.tmp"), "unfinished").expect("partial report");
        provision_staging_db(&db).expect("provision fixture staging DB");
        let conn = Connection::open(&db).expect("open fixture staging DB");
        let items = discover_reports(&root, |_| Tier::Internal).expect("discover reports");
        let make_request = |run_id: &str, at: &str| RunRequest {
            run_id: run_id.to_owned(),
            batch_id: "batch-fixture".to_owned(),
            source_class: SourceClass::Reports,
            host: "fixture-host".to_owned(),
            at: at.to_owned(),
            now_unix: 1,
            candidates: items.iter().map(|item| item.facts.clone()).collect(),
            item_failures: Vec::new(),
            measurements: Measurements {
                src_free_bytes: 20 * 1024 * 1024 * 1024,
                src_volume_bytes: 20 * 1024 * 1024 * 1024,
                stg_free_bytes: 20 * 1024 * 1024 * 1024,
                ..Measurements::default()
            },
            envelope: Vec::<EnvelopeLine>::new(),
            embed_usd: 0.0,
            embed_usd_median: None,
        };
        let first = run_once_with_staging(
            &state_dir,
            &make_request("run-1", "2026-11-21T00:00:00Z"),
            &items,
            &conn,
            "unit-sweep",
        )
        .expect("first fixture run");
        assert_eq!(first.report.as_ref().expect("report").items_accepted, 1);
        assert!(first.records.iter().any(|record| {
            record.outcome == "held" && record.source_ref.ends_with("partial.md.tmp")
        }));

        let retry = run_once_with_staging(
            &state_dir,
            &make_request("run-2", "2026-11-21T00:01:00Z"),
            &items,
            &conn,
            "unit-sweep",
        )
        .expect("retry fixture run");
        assert_eq!(retry.report.as_ref().expect("report").items_rejected, 1);
        assert!(retry.records.iter().any(|record| {
            record.reason_code.as_deref() == Some("duplicate-content")
                && record.source_ref.ends_with("final.md")
        }));
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM items WHERE unit_id = 'unit-sweep'",
                [],
                |r| r.get(0),
            )
            .expect("staged item count");
        assert_eq!(count, 1, "accepted report lands exactly once");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&state_dir);
        let _ = std::fs::remove_file(&db);
    }

    /// Oracle: a failed staging write leaves no landed address in sweep
    /// state, allowing the same source to be retried after recovery.
    #[test]
    fn staging_failure_does_not_advance_sweep_state() {
        let root = std::env::temp_dir().join(format!("atlas-stage-failure-{}", std::process::id()));
        let state_dir = root.with_extension("state");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&state_dir);
        std::fs::create_dir_all(&root).expect("fixture dir");
        std::fs::write(root.join("final.md"), "finished report").expect("fixture report");
        let items = discover_reports(&root, |_| Tier::Internal).expect("discover report");
        let req = RunRequest {
            run_id: "run-fail".to_owned(),
            batch_id: "batch-fail".to_owned(),
            source_class: SourceClass::Reports,
            host: "fixture-host".to_owned(),
            at: "2026-11-21T00:00:00Z".to_owned(),
            now_unix: 1,
            candidates: items.iter().map(|item| item.facts.clone()).collect(),
            item_failures: Vec::new(),
            measurements: Measurements {
                src_free_bytes: 20 * 1024 * 1024 * 1024,
                src_volume_bytes: 20 * 1024 * 1024 * 1024,
                stg_free_bytes: 20 * 1024 * 1024 * 1024,
                ..Measurements::default()
            },
            envelope: Vec::new(),
            embed_usd: 0.0,
            embed_usd_median: None,
        };
        let conn = Connection::open_in_memory().expect("open unprovisioned staging DB");
        assert!(run_once_with_staging(&state_dir, &req, &items, &conn, "unit-sweep").is_err());
        assert!(
            load_state(&state_dir)
                .expect("load retryable state")
                .landed
                .is_empty()
        );
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&state_dir);
    }

    /// Oracle: only a terminal session with a real exchange passes §2;
    /// synthetic-only sessions are rejected and open sessions are held.
    #[test]
    fn session_discovery_classifies_terminality_and_real_exchanges() {
        let root =
            std::env::temp_dir().join(format!("atlas-session-adapter-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("fixture dir");
        std::fs::write(
            root.join("real.session.json"),
            r#"{"state":"completed","messages":[{"role":"user","text":"review this change"},{"role":"assistant","text":"the test passes","synthetic":false}]}"#,
        )
        .expect("real session");
        std::fs::write(
            root.join("synthetic.session.json"),
            r#"{"state":"closed","messages":[{"role":"assistant","text":"generated summary","synthetic":true}]}"#,
        )
        .expect("synthetic session");
        std::fs::write(
            root.join("open.session.json"),
            r#"{"state":"running","messages":[{"role":"user","text":"in progress"}]}"#,
        )
        .expect("open session");

        let items = discover_session_logs(&root, |_| Tier::Internal).expect("discover sessions");
        assert_eq!(items.len(), 3);
        let by_name = |name: &str| {
            items
                .iter()
                .find(|item| item.facts.source_ref.ends_with(name))
                .expect("session candidate")
        };
        let real = by_name("real.session.json");
        assert!(real.facts.terminal && real.facts.has_substance);
        assert_eq!(
            filter_candidate(SourceClass::SessionLogs, &real.facts, &HashSet::new()),
            FilterOutcome::Accept
        );
        let synthetic = by_name("synthetic.session.json");
        assert!(synthetic.facts.terminal && !synthetic.facts.has_substance);
        assert_eq!(
            filter_candidate(SourceClass::SessionLogs, &synthetic.facts, &HashSet::new()),
            FilterOutcome::Reject(ReasonCode::NoQuestion)
        );
        let open = by_name("open.session.json");
        assert!(!open.facts.terminal && open.facts.has_substance);
        assert_eq!(
            filter_candidate(SourceClass::SessionLogs, &open.facts, &HashSet::new()),
            FilterOutcome::Hold(None)
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Oracle: off-allowlist exports are absent without parsing; the
    /// allowlisted channel yields per-message candidates and bot posts
    /// are rejected by the sweep filter.
    #[test]
    fn chat_discovery_checks_allowlist_before_read_and_splits_messages() {
        let root = std::env::temp_dir().join(format!("atlas-chat-adapter-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("fleet-announcements")).expect("allowlisted dir");
        std::fs::create_dir_all(root.join("private-support")).expect("private dir");
        std::fs::write(
            root.join("fleet-announcements/export.jsonl"),
            "{\"text\":\"deployment approved\",\"bot\":false}\n{\"text\":\"heartbeat\",\"bot\":true,\"heartbeat\":true}\n",
        )
        .expect("allowlisted export");
        // Invalid JSON proves this channel is not opened or parsed.
        std::fs::write(root.join("private-support/export.jsonl"), "not-json")
            .expect("private export");
        let allowlist = HashSet::from(["fleet-announcements".to_owned()]);
        let items = discover_chat_exports(&root, &allowlist, |_| Tier::Internal)
            .expect("discover allowlisted chat");
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|item| item.facts.allowlisted));
        assert_eq!(
            filter_candidate(SourceClass::ChatLogs, &items[0].facts, &HashSet::new()),
            FilterOutcome::Accept
        );
        assert!(items[1].facts.bot_or_heartbeat);
        assert_eq!(
            filter_candidate(SourceClass::ChatLogs, &items[1].facts, &HashSet::new()),
            FilterOutcome::Reject(ReasonCode::NoQuestion)
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
