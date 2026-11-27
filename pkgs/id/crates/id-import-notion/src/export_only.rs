//! Export-only degraded mode (slice 4 design record §2.5, #13524
//! sub-slice 5).
//!
//! When the API is unreachable, the importer can still run from a
//! Markdown+CSV export alone. That run is **degraded**, and says so:
//!
//! - The inventory is built from the export itself
//!   ([`export_only_manifest`]). A file whose name carries a Notion UUID
//!   keeps that UUID as its object ID; any other file (a CSV-only row, a
//!   truncated name) is keyed by its export path. Every page records its
//!   path and content hash in [`DegradedIdentity`], and the manifest's
//!   `page_id_to_note_id` keys are marked degraded through that map.
//! - Re-import idempotency weakens to hash equality
//!   ([`carry_degraded_identities`]): a UUID-named page keeps its note
//!   by UUID, but a path-keyed page keeps its note only when its path and
//!   content hash are both unchanged, or when its unchanged content hash
//!   identifies exactly one earlier page (a moved file). Anything else is
//!   a new note, and the carry result names it.
//! - The run carries no comments: every page is recorded as
//!   comment-uncovered with reason [`EXPORT_PATH_REASON`].
//! - No page claims full guarantees ([`identity_guarantee`]), every
//!   ledger row is named, and [`degradation_headline`] gives the line the
//!   fidelity report leads with.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::comments::CommentsCoverage;
use crate::export::{ExportArchive, ExportEntry, ExportKind, ExportMatch, MatchConfidence};
use crate::{Inventory, InventoryObject, Manifest, NOTION_API_VERSION};

/// Why an export-only page has no comment claims (§2.5).
pub const EXPORT_PATH_REASON: &str = "export-path";

/// Prefix of the object ID given to an export entry whose name carries
/// no Notion UUID.
pub const PATH_KEY_PREFIX: &str = "export-path-";

/// How a degraded-mode object's identity was keyed.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DegradedKey {
    /// The export file's name carried the Notion UUID.
    ExportUuid,
    /// No UUID: the object is keyed by its export path, and re-import
    /// recognises it only through its content hash.
    ContentHashPath,
}

/// The weaker identity record a degraded-mode page carries (§2.5).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DegradedIdentity {
    pub key: DegradedKey,
    /// Export-relative path of the entry (a CSV row is `<csv>#row<n>`).
    pub rel_path: String,
    /// BLAKE3 hex of the bytes that represent the entry: the page file,
    /// or the CSV row's fields.
    pub content_hash: String,
}

/// What a page's identity can be relied on for.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IdentityGuarantee {
    /// Identity from the Notion API: stable across re-imports.
    Full,
    /// Export-only, but the file name carried the Notion UUID.
    DegradedExportUuid,
    /// Export-only and path-keyed: stable only while the content hash is.
    DegradedContentHashPath,
}

impl IdentityGuarantee {
    /// Whether this is the full (API) guarantee.
    pub fn is_full(self) -> bool {
        self == IdentityGuarantee::Full
    }
}

fn dashed_uuid(hex: &str) -> String {
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

fn object_id(entry: &ExportEntry) -> (String, DegradedKey) {
    match &entry.uuid {
        Some(hex) if hex.len() == 32 => (dashed_uuid(hex), DegradedKey::ExportUuid),
        _ => {
            let hash = blake3::hash(entry.rel_path.as_bytes()).to_hex();
            (
                format!("{PATH_KEY_PREFIX}{}", &hash.as_str()[..32]),
                DegradedKey::ContentHashPath,
            )
        }
    }
}

fn title_spans(title: &str) -> Value {
    json!([{"type": "text", "plain_text": title, "text": {"content": title}}])
}

/// The bytes that stand for an entry's content.
fn entry_bytes(archive: &ExportArchive, entry: &ExportEntry) -> Result<Vec<u8>> {
    if let Some(row) = entry.csv_row {
        let csv_path = entry
            .rel_path
            .split_once('#')
            .map_or(entry.rel_path.as_str(), |(path, _)| path);
        let rows = archive.csv(csv_path)?;
        let fields = rows.get(row).cloned().unwrap_or_default();
        return Ok(serde_json::to_vec(&fields)?);
    }
    match archive.file(&entry.rel_path) {
        Some(bytes) => Ok(bytes.to_vec()),
        None => bail!("export has no file {}", entry.rel_path),
    }
}

/// The directory an entry's children live in: `a/b.md` -> `a/b`,
/// `a/db_all.csv` -> `a/db`.
fn child_dir(entry: &ExportEntry) -> Option<String> {
    if entry.csv_row.is_some() {
        return None;
    }
    let lower = entry.rel_path.to_ascii_lowercase();
    let cut = if lower.ends_with(".md") {
        3
    } else if lower.ends_with(".csv") {
        4
    } else {
        return None;
    };
    let stem = &entry.rel_path[..entry.rel_path.len() - cut];
    Some(stem.strip_suffix("_all").unwrap_or(stem).to_owned())
}

fn parent_dir(entry: &ExportEntry) -> Option<&str> {
    if entry.csv_row.is_some() {
        return None;
    }
    entry.rel_path.rsplit_once('/').map(|(dir, _)| dir)
}

/// Build an export-only (degraded-mode) manifest: inventory, match
/// ledger, degraded identities, acquisition header and comment coverage
/// all come from the export (§2.5). Note IDs are not assigned here; run
/// [`carry_degraded_identities`] against an earlier manifest (if any),
/// then [`Manifest::assign_page_identities`].
pub fn export_only_manifest(workspace_id: String, archive: &ExportArchive) -> Result<Manifest> {
    let mut ids: BTreeMap<String, (String, DegradedKey)> = BTreeMap::new();
    let mut kinds: BTreeMap<String, ExportKind> = BTreeMap::new();
    let mut by_child_dir: BTreeMap<String, String> = BTreeMap::new();
    let mut taken = BTreeSet::new();
    for entry in archive.entries() {
        let (id, key) = object_id(entry);
        if !taken.insert(id.clone()) {
            bail!(
                "export entries share the identity {id} (second: {})",
                entry.rel_path
            );
        }
        if let Some(dir) = child_dir(entry) {
            // A page's directory wins over a same-named database's: Notion
            // never writes both, and a page's children are its blocks.
            if entry.kind == ExportKind::Page || !by_child_dir.contains_key(&dir) {
                by_child_dir.insert(dir, id.clone());
            }
        }
        kinds.insert(id.clone(), entry.kind);
        ids.insert(entry.rel_path.clone(), (id, key));
    }

    let mut inventory = Inventory::default();
    let mut degraded = BTreeMap::new();
    let mut ledger = BTreeMap::new();
    for entry in archive.entries() {
        let (id, key) = ids[&entry.rel_path].clone();
        let parent_id = match entry.csv_row {
            Some(_) => entry
                .rel_path
                .split_once('#')
                .and_then(|(csv, _)| ids.get(csv))
                .map(|(id, _)| id.clone()),
            None => parent_dir(entry).and_then(|dir| by_child_dir.get(dir).cloned()),
        };
        let parent = match &parent_id {
            Some(parent) if kinds.get(parent) == Some(&ExportKind::Database) => {
                json!({"type": "database_id", "database_id": parent})
            }
            Some(parent) => json!({"type": "page_id", "page_id": parent}),
            None => json!({"type": "workspace", "workspace": true}),
        };
        let source = match entry.kind {
            ExportKind::Page => json!({
                "object": "page",
                "id": id,
                "parent": parent,
                "properties": {"title": {"id": "title", "type": "title", "title": title_spans(&entry.title)}},
            }),
            ExportKind::Database => json!({
                "object": "database",
                "id": id,
                "parent": parent,
                "title": title_spans(&entry.title),
            }),
        };
        let kind = match entry.kind {
            ExportKind::Page => "page",
            ExportKind::Database => "database",
        };
        if parent_id.is_none() {
            inventory.visible_shares.push(id.clone());
        }
        inventory.objects.insert(
            id.clone(),
            InventoryObject {
                id: id.clone(),
                kind: kind.to_owned(),
                parent_id,
                source,
            },
        );
        if entry.kind == ExportKind::Page {
            inventory.comments_available.insert(id.clone(), false);
            degraded.insert(
                id.clone(),
                DegradedIdentity {
                    key,
                    rel_path: entry.rel_path.clone(),
                    content_hash: blake3::hash(&entry_bytes(archive, entry)?)
                        .to_hex()
                        .to_string(),
                },
            );
        }
        ledger.insert(
            entry.rel_path.clone(),
            ExportMatch {
                kind: entry.kind,
                export_title: entry.title.clone(),
                confidence: MatchConfidence::ExportOnly,
                inventory_id: Some(id),
                candidates: Vec::new(),
                api_title: None,
            },
        );
    }

    let mut manifest = Manifest::new(
        workspace_id,
        vec!["export".to_owned()],
        NOTION_API_VERSION.to_owned(),
        inventory,
    )?;
    manifest.acquisition.degraded_mode = true;
    manifest.export_matches = ledger;
    for page_id in degraded.keys() {
        manifest
            .comments_coverage
            .insert(page_id.clone(), CommentsCoverage::Unavailable);
        manifest
            .comments_uncovered_reasons
            .insert(page_id.clone(), EXPORT_PATH_REASON.to_owned());
    }
    manifest.degraded_identities = degraded;
    Ok(manifest)
}

/// Which earlier note each degraded page kept on re-import, and which
/// pages could not be recognised.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CarryReport {
    /// Page ID -> carried note ID, by UUID.
    pub by_uuid: BTreeMap<String, String>,
    /// Page ID -> carried note ID, by unchanged path and content hash.
    pub by_path_and_hash: BTreeMap<String, String>,
    /// Page ID -> carried note ID, by an unchanged content hash that
    /// identifies exactly one earlier page (the file moved).
    pub by_hash_moved: BTreeMap<String, String>,
    /// Path-keyed pages that match no earlier page: they become new
    /// notes. A changed page without a UUID lands here (§2.5: re-import
    /// idempotency weakens to hash equality).
    pub unrecognised: Vec<String>,
}

/// Carry note IDs from an earlier degraded-mode manifest to a new one
/// built by [`export_only_manifest`] (§2.5). Pages already assigned in
/// `next` are left alone, and a note ID is never given to two pages.
pub fn carry_degraded_identities(previous: &Manifest, next: &mut Manifest) -> CarryReport {
    let mut report = CarryReport::default();
    let mut used: BTreeSet<String> = next.page_id_to_note_id.values().cloned().collect();
    let mut previous_by_path: BTreeMap<&str, (&str, &DegradedIdentity)> = BTreeMap::new();
    let mut previous_by_hash: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (page_id, identity) in &previous.degraded_identities {
        previous_by_path.insert(identity.rel_path.as_str(), (page_id.as_str(), identity));
        previous_by_hash
            .entry(identity.content_hash.as_str())
            .or_default()
            .push(page_id.as_str());
    }
    let pages: Vec<(String, DegradedIdentity)> = next
        .degraded_identities
        .iter()
        .map(|(id, identity)| (id.clone(), identity.clone()))
        .collect();
    for (page_id, identity) in pages {
        if next.page_id_to_note_id.contains_key(&page_id) {
            continue;
        }
        let mut carried = None;
        if identity.key == DegradedKey::ExportUuid
            && let Some(note) = previous.page_id_to_note_id.get(&page_id)
            && !used.contains(note)
        {
            carried = Some((note.clone(), 0));
        }
        if carried.is_none()
            && let Some((old_id, old)) = previous_by_path.get(identity.rel_path.as_str())
            && old.content_hash == identity.content_hash
            && let Some(note) = previous.page_id_to_note_id.get(*old_id)
            && !used.contains(note)
        {
            carried = Some((note.clone(), 1));
        }
        if carried.is_none()
            && let Some([only]) = previous_by_hash
                .get(identity.content_hash.as_str())
                .map(Vec::as_slice)
            && let Some(note) = previous.page_id_to_note_id.get(*only)
            && !used.contains(note)
        {
            carried = Some((note.clone(), 2));
        }
        match carried {
            Some((note, how)) => {
                used.insert(note.clone());
                next.page_id_to_note_id
                    .insert(page_id.clone(), note.clone());
                let bucket = match how {
                    0 => &mut report.by_uuid,
                    1 => &mut report.by_path_and_hash,
                    _ => &mut report.by_hash_moved,
                };
                bucket.insert(page_id, note);
            }
            None if identity.key == DegradedKey::ContentHashPath => {
                report.unrecognised.push(page_id);
            }
            None => {}
        }
    }
    report
}

/// What a page's identity can be relied on for, or `None` when the page
/// is not in the manifest's inventory.
pub fn identity_guarantee(manifest: &Manifest, page_id: &str) -> Option<IdentityGuarantee> {
    manifest.inventory.objects.get(page_id)?;
    if !manifest.acquisition.degraded_mode {
        return Some(IdentityGuarantee::Full);
    }
    Some(
        match manifest.degraded_identities.get(page_id).map(|d| d.key) {
            Some(DegradedKey::ExportUuid) => IdentityGuarantee::DegradedExportUuid,
            _ => IdentityGuarantee::DegradedContentHashPath,
        },
    )
}

/// The line a degraded-mode run's fidelity report leads with, or `None`
/// for a run that had the API.
pub fn degradation_headline(manifest: &Manifest) -> Option<String> {
    if !manifest.acquisition.degraded_mode {
        return None;
    }
    let pages = manifest.degraded_identities.len();
    let path_keyed = manifest
        .degraded_identities
        .values()
        .filter(|d| d.key == DegradedKey::ContentHashPath)
        .count();
    Some(format!(
        "Degraded import: export only, no Notion API. {pages} pages have export identity, \
         {path_keyed} of them keyed by path and content hash, so re-import recognises them only \
         while unchanged. No comments were imported (reason: {EXPORT_PATH_REASON})."
    ))
}
