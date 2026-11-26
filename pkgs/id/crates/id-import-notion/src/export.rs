//! Export reader (slice 4 design record §2.5, #13524 sub-slice 4).
//!
//! Reads Notion's Markdown+CSV export flavour and feeds it into the
//! **same** conversion pipeline as the API path: there is one
//! converter ([`crate::content::convert_page`]) with two acquisition
//! front ends, not two pipelines.
//!
//! - [`ExportArchive`] holds the export's files and indexes its page
//!   (`.md`) and database (`.csv`) entries. Notion names those files
//!   `<title> <uuid>.<ext>` and nests a page's children in a directory
//!   named like the page, so UUID matching is the common case.
//! - [`match_export`] matches every entry to the API inventory and
//!   records the result in a match ledger with a confidence:
//!   `uuid_exact`, `title_path_unique`, `title_path_ambiguous`, or
//!   `unmatched`. Title+path matching is used only where an entry names
//!   no UUID (CSV row pages, truncated names), and every non-exact
//!   match is named (§2.7 invariant 5: no export match is ever silent).
//!   Where the export's title differs from the API's, the API wins for
//!   identity and the conflict is named on the entry.
//! - [`ExportApi`] normalizes matched Markdown pages into raw block
//!   trees in the Notion API's shape and serves them through the
//!   [`NotionApi`] block-children call, so `convert_page` converts an
//!   exported page exactly as it converts a crawled one.
//! - [`ExportFileFetcher`] serves files the export carries (images,
//!   attachments) to the converter's file step by export-relative path.
//!
//! The export flavour carries no comments and no block IDs: block IDs
//! are synthesized deterministically per page (`export-<page>-<n>`), so
//! re-reading the same export yields the same identities. Export-only
//! runs (no API inventory) are sub-slice 5 and are not handled here.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::content::{FileFetcher, normalize_uuid, page_title};
use crate::{Inventory, Manifest, NOTION_API_VERSION, NotionApi};

/// Whether an export entry is a page (`.md`) or a database (`.csv`).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExportKind {
    Page,
    Database,
}

impl ExportKind {
    fn inventory_kind(self) -> &'static str {
        match self {
            ExportKind::Page => "page",
            ExportKind::Database => "database",
        }
    }
}

/// One page or database file in the export.
#[derive(Clone, Debug, PartialEq)]
pub struct ExportEntry {
    /// Export-relative path with `/` separators.
    pub rel_path: String,
    pub kind: ExportKind,
    /// The page's `# ` title line when it has one, else the title part
    /// of the file name.
    pub title: String,
    /// Normalized (dashless, lowercase) UUID from the file name.
    pub uuid: Option<String>,
    /// Titles of the enclosing directories, nearest first.
    pub ancestors: Vec<String>,
    /// For a CSV row without its own page file: the row's index.
    pub csv_row: Option<usize>,
}

/// How an export entry was matched to the API inventory.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MatchConfidence {
    /// The file name's UUID is an inventory object of the same kind.
    UuidExact,
    /// No UUID; exactly one inventory object has this title and path.
    TitlePathUnique,
    /// No UUID; several inventory objects have this title and path. The
    /// first (by ID) is taken and every candidate is named.
    TitlePathAmbiguous,
    /// No inventory object matches; the entry's content is not imported.
    Unmatched,
}

/// One row of the export match ledger (§2.5, §2.6).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ExportMatch {
    pub kind: ExportKind,
    pub export_title: String,
    pub confidence: MatchConfidence,
    /// The inventory object the entry's content is imported into.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inventory_id: Option<String>,
    /// Every inventory object a title+path match considered.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<String>,
    /// The API title when it differs from the export's (the API wins
    /// for identity; the conflict is named, never silently resolved).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_title: Option<String>,
}

impl ExportMatch {
    /// Whether the fidelity report must name this entry: every match
    /// that is not UUID-exact, and every title conflict.
    pub fn needs_naming(&self) -> bool {
        self.confidence != MatchConfidence::UuidExact || self.api_title.is_some()
    }
}

/// The files of one Markdown+CSV export, indexed.
#[derive(Clone, Debug, Default)]
pub struct ExportArchive {
    files: BTreeMap<String, Vec<u8>>,
    entries: Vec<ExportEntry>,
}

impl ExportArchive {
    /// Index an export given as export-relative path -> bytes.
    pub fn from_files(files: BTreeMap<String, Vec<u8>>) -> Result<Self> {
        let mut files_norm = BTreeMap::new();
        for (path, bytes) in files {
            let path = path.replace('\\', "/").trim_start_matches("./").to_owned();
            if path.split('/').any(|part| part == "..") {
                bail!("export path {path} escapes the export root");
            }
            files_norm.insert(path, bytes);
        }
        let mut archive = Self {
            files: files_norm,
            entries: Vec::new(),
        };
        archive.index()?;
        Ok(archive)
    }

    /// Read an unpacked export directory.
    pub fn read_dir(root: &Path) -> Result<Self> {
        let mut files = BTreeMap::new();
        let mut pending = vec![root.to_path_buf()];
        while let Some(dir) = pending.pop() {
            let listing = std::fs::read_dir(&dir)
                .with_context(|| format!("reading export directory {}", dir.display()))?;
            for item in listing {
                let path = item?.path();
                let file_type = std::fs::symlink_metadata(&path)?.file_type();
                if file_type.is_dir() {
                    pending.push(path);
                } else if file_type.is_file() {
                    let rel = path
                        .strip_prefix(root)
                        .context("export file outside the export root")?
                        .components()
                        .map(|c| c.as_os_str().to_string_lossy().into_owned())
                        .collect::<Vec<_>>()
                        .join("/");
                    files.insert(rel, std::fs::read(&path)?);
                }
                // Symlinks are skipped: an export never contains them, and
                // following one could read outside the export root.
            }
        }
        Self::from_files(files)
    }

    /// The indexed page and database entries, in path order.
    pub fn entries(&self) -> &[ExportEntry] {
        &self.entries
    }

    /// A file's bytes by export-relative path.
    pub fn file(&self, rel_path: &str) -> Option<&[u8]> {
        self.files.get(rel_path).map(Vec::as_slice)
    }

    /// A database entry's CSV as a header row plus data rows.
    pub fn csv(&self, rel_path: &str) -> Result<Vec<Vec<String>>> {
        let bytes = self
            .file(rel_path)
            .with_context(|| format!("export has no file {rel_path}"))?;
        let text = std::str::from_utf8(bytes)
            .with_context(|| format!("export CSV {rel_path} is not UTF-8"))?;
        parse_csv(text.trim_start_matches('\u{feff}'))
    }

    fn text(&self, rel_path: &str) -> Result<&str> {
        let bytes = self
            .file(rel_path)
            .with_context(|| format!("export has no file {rel_path}"))?;
        std::str::from_utf8(bytes).with_context(|| format!("export file {rel_path} is not UTF-8"))
    }

    fn index(&mut self) -> Result<()> {
        let mut entries = Vec::new();
        let csv_paths: BTreeSet<&String> = self
            .files
            .keys()
            .filter(|p| p.to_ascii_lowercase().ends_with(".csv"))
            .collect();
        for path in self.files.keys() {
            let lower = path.to_ascii_lowercase();
            let (kind, ext_len) = if lower.ends_with(".md") {
                (ExportKind::Page, 3)
            } else if lower.ends_with(".csv") {
                (ExportKind::Database, 4)
            } else {
                continue;
            };
            let (dir, name) = split_path(path);
            let mut stem = &name[..name.len() - ext_len];
            if kind == ExportKind::Database {
                // Notion writes `<db>.csv` (current view) and
                // `<db>_all.csv` (every row); one entry per database,
                // preferring the complete file.
                if let Some(base) = stem.strip_suffix("_all") {
                    stem = base;
                } else {
                    let all = format!("{}{}_all.csv", dir_prefix(dir), stem);
                    if csv_paths.contains(&all) {
                        continue;
                    }
                }
            }
            let (file_title, uuid) = split_title_uuid(stem);
            let title = if kind == ExportKind::Page {
                markdown_title(std::str::from_utf8(&self.files[path]).unwrap_or(""))
                    .unwrap_or_else(|| file_title.clone())
            } else {
                file_title
            };
            entries.push(ExportEntry {
                rel_path: path.clone(),
                kind,
                title,
                uuid,
                ancestors: ancestor_titles_of_dir(dir),
                csv_row: None,
            });
        }
        // CSV rows without their own page file: title from the first
        // column, path = the database's directory. Rows that do have a
        // page file are matched through that file.
        let mut row_entries = Vec::new();
        for entry in entries.iter().filter(|e| e.kind == ExportKind::Database) {
            let (dir, name) = split_path(&entry.rel_path);
            let stem = name
                .trim_end_matches(".csv")
                .trim_end_matches(".CSV")
                .trim_end_matches("_all");
            let row_dir = format!("{}{}/", dir_prefix(dir), stem);
            let page_titles: BTreeSet<&str> = entries
                .iter()
                .filter(|e| e.kind == ExportKind::Page && e.rel_path.starts_with(&row_dir))
                .filter(|e| !e.rel_path[row_dir.len()..].contains('/'))
                .map(|e| e.title.as_str())
                .collect();
            let rows = self.csv(&entry.rel_path)?;
            let mut ancestors = vec![entry.title.clone()];
            ancestors.extend(entry.ancestors.iter().cloned());
            for (index, row) in rows.iter().enumerate().skip(1) {
                let Some(title) = row.first().map(|t| t.trim()) else {
                    continue;
                };
                if title.is_empty() || page_titles.contains(title) {
                    continue;
                }
                row_entries.push(ExportEntry {
                    rel_path: format!("{}#row{index}", entry.rel_path),
                    kind: ExportKind::Page,
                    title: title.to_owned(),
                    uuid: None,
                    ancestors: ancestors.clone(),
                    csv_row: Some(index),
                });
            }
        }
        entries.extend(row_entries);
        entries.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
        self.entries = entries;
        Ok(())
    }
}

fn split_path(path: &str) -> (&str, &str) {
    match path.rsplit_once('/') {
        Some((dir, name)) => (dir, name),
        None => ("", path),
    }
}

fn dir_prefix(dir: &str) -> String {
    if dir.is_empty() {
        String::new()
    } else {
        format!("{dir}/")
    }
}

/// Split `<title> <32-hex uuid>` into its title and normalized UUID.
fn split_title_uuid(stem: &str) -> (String, Option<String>) {
    if let Some((title, tail)) = stem.rsplit_once(' ')
        && tail.len() == 32
        && tail.chars().all(|c| c.is_ascii_hexdigit())
    {
        return (title.to_owned(), Some(tail.to_ascii_lowercase()));
    }
    if stem.len() == 32 && stem.chars().all(|c| c.is_ascii_hexdigit()) {
        return (String::new(), Some(stem.to_ascii_lowercase()));
    }
    (stem.to_owned(), None)
}

fn ancestor_titles_of_dir(dir: &str) -> Vec<String> {
    if dir.is_empty() {
        return Vec::new();
    }
    dir.split('/')
        .rev()
        .map(|part| split_title_uuid(part).0)
        .collect()
}

fn markdown_title(text: &str) -> Option<String> {
    let first = text
        .trim_start_matches('\u{feff}')
        .lines()
        .find(|line| !line.trim().is_empty())?;
    first
        .strip_prefix("# ")
        .map(|title| title.trim().to_owned())
}

/// Minimal RFC 4180 CSV: quoted fields, doubled quotes, CRLF or LF.
pub fn parse_csv(text: &str) -> Result<Vec<Vec<String>>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    let mut any = false;
    while let Some(c) = chars.next() {
        any = true;
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => quoted = true,
            ',' => row.push(std::mem::take(&mut field)),
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
                any = false;
            }
            _ => field.push(c),
        }
    }
    if quoted {
        bail!("CSV ends inside a quoted field");
    }
    if any {
        row.push(field);
        rows.push(row);
    }
    Ok(rows)
}

// ------------------------------------------------------------ matching

fn object_title(inventory: &Inventory, id: &str) -> Option<String> {
    let object = inventory.objects.get(id)?;
    match object.kind.as_str() {
        "page" => Some(page_title(&object.source)),
        "database" => Some(
            object
                .source
                .get("title")
                .and_then(Value::as_array)
                .map(|spans| {
                    spans
                        .iter()
                        .filter_map(|s| s.get("plain_text").and_then(Value::as_str))
                        .collect::<String>()
                })
                .unwrap_or_default(),
        ),
        _ => None,
    }
}

/// Titles of an inventory object's ancestors, nearest first. A row
/// page's parent is its data source, which stands for its database.
fn inventory_ancestor_titles(inventory: &Inventory, id: &str) -> Vec<String> {
    let source_to_database: BTreeMap<&str, &str> = inventory
        .data_sources
        .iter()
        .flat_map(|(db, sources)| sources.iter().map(move |s| (s.id.as_str(), db.as_str())))
        .collect();
    let mut titles = Vec::new();
    let mut seen = BTreeSet::new();
    let mut current = inventory.objects.get(id).and_then(|o| o.parent_id.clone());
    while let Some(parent) = current {
        if !seen.insert(parent.clone()) {
            break;
        }
        let parent = source_to_database
            .get(parent.as_str())
            .map(|db| (*db).to_owned())
            .unwrap_or(parent);
        let Some(object) = inventory.objects.get(&parent) else {
            break;
        };
        // Blocks between a page and its child (e.g. a column) are not
        // export directories; skip them.
        if object.kind == "page" || object.kind == "database" {
            titles.push(object_title(inventory, &parent).unwrap_or_default());
        }
        current = object.parent_id.clone();
    }
    titles
}

/// Match every export entry to the API inventory (§2.5).
pub fn match_export(
    archive: &ExportArchive,
    inventory: &Inventory,
) -> BTreeMap<String, ExportMatch> {
    let by_uuid: BTreeMap<String, &str> = inventory
        .objects
        .iter()
        .filter(|(_, o)| o.kind == "page" || o.kind == "database")
        .map(|(id, _)| (normalize_uuid(id), id.as_str()))
        .collect();
    let mut ledger = BTreeMap::new();
    let mut claimed: BTreeSet<String> = BTreeSet::new();

    // Pass 1: UUID-exact, so title matching never steals a claimed object.
    for entry in archive.entries() {
        let Some(uuid) = &entry.uuid else { continue };
        let found = by_uuid
            .get(uuid)
            .copied()
            .filter(|id| inventory.objects[*id].kind == entry.kind.inventory_kind());
        let (confidence, inventory_id) = match found {
            Some(id) if claimed.insert(id.to_owned()) => {
                (MatchConfidence::UuidExact, Some(id.to_owned()))
            }
            _ => (MatchConfidence::Unmatched, None),
        };
        let api_title = inventory_id
            .as_deref()
            .and_then(|id| object_title(inventory, id))
            .filter(|t| !entry.title.is_empty() && *t != entry.title);
        ledger.insert(
            entry.rel_path.clone(),
            ExportMatch {
                kind: entry.kind,
                export_title: entry.title.clone(),
                confidence,
                inventory_id,
                candidates: Vec::new(),
                api_title,
            },
        );
    }

    // Pass 2: title+path for entries that name no UUID.
    for entry in archive.entries().iter().filter(|e| e.uuid.is_none()) {
        let candidates: Vec<String> = inventory
            .objects
            .iter()
            .filter(|(id, o)| o.kind == entry.kind.inventory_kind() && !claimed.contains(*id))
            .filter(|(id, _)| object_title(inventory, id).as_deref() == Some(entry.title.as_str()))
            .filter(|(id, _)| {
                let chain = inventory_ancestor_titles(inventory, id);
                entry
                    .ancestors
                    .iter()
                    .zip(chain.iter())
                    .all(|(a, b)| a == b)
            })
            .map(|(id, _)| id.clone())
            .collect();
        // A file whose folders spell an object's whole ancestor chain
        // beats one whose folders only spell the nearest part of it (a
        // partial export), so a root-level file never ties with a nested
        // namesake.
        let exact: Vec<String> = candidates
            .iter()
            .filter(|id| inventory_ancestor_titles(inventory, id).len() == entry.ancestors.len())
            .cloned()
            .collect();
        let candidates = if exact.is_empty() { candidates } else { exact };
        let (confidence, inventory_id) = match candidates.as_slice() {
            [] => (MatchConfidence::Unmatched, None),
            [only] => (MatchConfidence::TitlePathUnique, Some(only.clone())),
            [first, ..] => (MatchConfidence::TitlePathAmbiguous, Some(first.clone())),
        };
        if let Some(id) = &inventory_id {
            claimed.insert(id.clone());
        }
        ledger.insert(
            entry.rel_path.clone(),
            ExportMatch {
                kind: entry.kind,
                export_title: entry.title.clone(),
                confidence,
                inventory_id,
                candidates: if confidence == MatchConfidence::TitlePathUnique {
                    Vec::new()
                } else {
                    candidates
                },
                api_title: None,
            },
        );
    }
    ledger
}

/// Match the export against the manifest's inventory and record the
/// ledger and the export acquisition flag in the manifest (§2.6).
pub fn record_export_matches(manifest: &mut Manifest, archive: &ExportArchive) {
    manifest.export_matches = match_export(archive, &manifest.inventory);
    manifest.acquisition.export = true;
}

// ------------------------------------------------------------ normalization

/// Serves an export's matched pages as Notion block-children responses.
pub struct ExportApi {
    children: BTreeMap<String, Vec<Value>>,
    visible: Vec<String>,
}

impl ExportApi {
    /// Normalize every matched page of the export into raw block trees.
    /// The manifest must hold the match ledger ([`record_export_matches`]).
    pub fn new(archive: &ExportArchive, manifest: &Manifest) -> Result<Self> {
        let mut uuid_to_id: BTreeMap<String, String> = BTreeMap::new();
        let mut path_to_id: BTreeMap<String, (String, ExportKind)> = BTreeMap::new();
        for (path, entry) in &manifest.export_matches {
            if let Some(id) = &entry.inventory_id {
                path_to_id.insert(path.clone(), (id.clone(), entry.kind));
                if let Some(e) = archive.entries().iter().find(|e| &e.rel_path == path)
                    && let Some(uuid) = &e.uuid
                {
                    uuid_to_id.insert(uuid.clone(), id.clone());
                }
            }
        }
        let database_dirs: BTreeSet<String> = archive
            .entries()
            .iter()
            .filter(|e| e.kind == ExportKind::Database)
            .map(|e| {
                let stem = e.rel_path.trim_end_matches(".csv").trim_end_matches("_all");
                format!("{stem}/")
            })
            .collect();
        let mut children = BTreeMap::new();
        let mut visible = Vec::new();
        for entry in archive.entries() {
            if entry.kind != ExportKind::Page || entry.csv_row.is_some() {
                continue;
            }
            let Some((page_id, _)) = path_to_id.get(&entry.rel_path) else {
                continue;
            };
            let text = archive.text(&entry.rel_path)?;
            let is_row = database_dirs.iter().any(|d| {
                entry.rel_path.starts_with(d.as_str()) && !entry.rel_path[d.len()..].contains('/')
            });
            let mut normalizer = Normalizer {
                page_path: &entry.rel_path,
                page_id: normalize_uuid(page_id),
                next: 0,
                children: BTreeMap::new(),
                path_to_id: &path_to_id,
                uuid_to_id: &uuid_to_id,
                archive,
            };
            let lines = page_body_lines(text, &entry.title, is_row);
            let top = normalizer.blocks(&lines);
            let Normalizer {
                children: nested, ..
            } = normalizer;
            children.extend(nested);
            children.insert(page_id.clone(), top);
            visible.push(page_id.clone());
        }
        Ok(Self { children, visible })
    }
}

impl NotionApi for ExportApi {
    fn notion_version(&self) -> &str {
        NOTION_API_VERSION
    }
    fn search(&mut self, _start_cursor: Option<&str>) -> Result<Value> {
        Ok(json!({"object": "list", "results": [], "has_more": false}))
    }
    fn block_children(&mut self, block_id: &str, _start_cursor: Option<&str>) -> Result<Value> {
        let results = self
            .children
            .get(block_id)
            .with_context(|| format!("the export carries no content for {block_id}"))?;
        Ok(json!({"object": "list", "results": results, "has_more": false}))
    }
    fn database(&mut self, database_id: &str) -> Result<Value> {
        bail!("the export flavour carries no database object for {database_id}")
    }
    fn data_source(&mut self, data_source_id: &str) -> Result<Value> {
        bail!("the export flavour carries no data source object for {data_source_id}")
    }
    fn query_data_source(&mut self, data_source_id: &str, _cursor: Option<&str>) -> Result<Value> {
        bail!("the export flavour carries no data source query for {data_source_id}")
    }
    fn comments_available(&mut self, _page_id: &str) -> Result<bool> {
        // The Markdown+CSV flavour carries no comments (§2.5).
        Ok(false)
    }
    fn visible_share_ids(&self) -> Vec<String> {
        self.visible.clone()
    }
    fn workspace_edition(&self) -> Option<String> {
        None
    }
}

/// Serves files the export carries by export-relative path.
pub struct ExportFileFetcher<'a> {
    archive: &'a ExportArchive,
}

impl<'a> ExportFileFetcher<'a> {
    pub fn new(archive: &'a ExportArchive) -> Self {
        Self { archive }
    }
}

impl FileFetcher for ExportFileFetcher<'_> {
    fn fetch(&mut self, url: &str) -> Result<Vec<u8>> {
        self.archive
            .file(url)
            .map(<[u8]>::to_vec)
            .with_context(|| format!("the export does not contain {url}"))
    }
}

/// A page's lines after the title line and, for database rows, the
/// property block Notion writes under the title.
fn page_body_lines(text: &str, title: &str, is_row: bool) -> Vec<String> {
    let text = text.trim_start_matches('\u{feff}');
    let mut lines: Vec<String> = text.lines().map(|l| l.replace('\t', "    ")).collect();
    let first = lines.iter().position(|l| !l.trim().is_empty());
    if let Some(first) = first
        && lines[first].trim() == format!("# {title}").trim()
    {
        lines.drain(..=first);
        if is_row {
            let start = lines.iter().position(|l| !l.trim().is_empty());
            if let Some(start) = start {
                let end = lines[start..]
                    .iter()
                    .position(|l| l.trim().is_empty())
                    .map_or(lines.len(), |n| start + n);
                if lines[start..end].iter().all(|l| l.contains(": ")) {
                    lines.drain(..end);
                }
            }
        }
    }
    lines
}

struct Normalizer<'a> {
    page_path: &'a str,
    page_id: String,
    next: usize,
    children: BTreeMap<String, Vec<Value>>,
    path_to_id: &'a BTreeMap<String, (String, ExportKind)>,
    uuid_to_id: &'a BTreeMap<String, String>,
    archive: &'a ExportArchive,
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

fn is_list_marker(rest: &str) -> Option<(&'static str, &str, Option<bool>)> {
    for (prefix, checked) in [("- [ ] ", false), ("- [x] ", true), ("- [X] ", true)] {
        if let Some(text) = rest.strip_prefix(prefix) {
            return Some(("to_do", text, Some(checked)));
        }
    }
    for prefix in ["- ", "* ", "+ "] {
        if let Some(text) = rest.strip_prefix(prefix) {
            return Some(("bulleted_list_item", text, None));
        }
    }
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    if digits > 0
        && let Some(text) = rest[digits..].strip_prefix(". ")
    {
        return Some(("numbered_list_item", text, None));
    }
    None
}

fn starts_structure(rest: &str) -> bool {
    rest.starts_with("```")
        || rest.starts_with("$$")
        || rest.starts_with('#')
        || rest.starts_with("> ")
        || rest == ">"
        || rest.starts_with('|')
        || rest.starts_with("<aside")
        || rest == "---"
        || is_list_marker(rest).is_some()
}

impl Normalizer<'_> {
    fn next_id(&mut self) -> String {
        self.next += 1;
        format!("export-{}-{}", self.page_id, self.next)
    }

    fn block(&mut self, block_type: &str, payload: Value, children: Vec<Value>) -> Value {
        let id = self.next_id();
        self.block_with_id(id, block_type, payload, children)
    }

    fn block_with_id(
        &mut self,
        id: String,
        block_type: &str,
        payload: Value,
        children: Vec<Value>,
    ) -> Value {
        let has_children = !children.is_empty();
        if has_children {
            self.children.insert(id.clone(), children);
        }
        let mut value = json!({
            "object": "block",
            "id": id,
            "type": block_type,
            "has_children": has_children,
        });
        value[block_type] = payload;
        value
    }

    /// Parse lines (already de-indented to this level) into blocks.
    fn blocks(&mut self, lines: &[String]) -> Vec<Value> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < lines.len() {
            let line = &lines[i];
            let rest = line.trim();
            if rest.is_empty() {
                i += 1;
                continue;
            }
            let indent = indent_of(line);
            if let Some(lang) = rest.strip_prefix("```") {
                let mut body = Vec::new();
                i += 1;
                while i < lines.len() && lines[i].trim() != "```" {
                    let l = &lines[i];
                    body.push(l.get(indent.min(indent_of(l))..).unwrap_or("").to_owned());
                    i += 1;
                }
                i += 1;
                let language = if lang.trim().is_empty() {
                    "plain text"
                } else {
                    lang.trim()
                };
                let text = body.join("\n");
                let block = self.block(
                    "code",
                    json!({"rich_text": [text_item(&text, Annot::default(), None)], "language": language}),
                    Vec::new(),
                );
                out.push(block);
                continue;
            }
            if rest.starts_with("$$") {
                let mut body = Vec::new();
                let first = rest.trim_start_matches("$$");
                if let Some(single) = first.strip_suffix("$$") {
                    body.push(single.to_owned());
                    i += 1;
                } else {
                    if !first.is_empty() {
                        body.push(first.to_owned());
                    }
                    i += 1;
                    while i < lines.len() && !lines[i].trim().ends_with("$$") {
                        body.push(lines[i].trim().to_owned());
                        i += 1;
                    }
                    if i < lines.len() {
                        let last = lines[i].trim().trim_end_matches("$$");
                        if !last.is_empty() {
                            body.push(last.to_owned());
                        }
                        i += 1;
                    }
                }
                let block = self.block(
                    "equation",
                    json!({"expression": body.join("\n").trim()}),
                    Vec::new(),
                );
                out.push(block);
                continue;
            }
            if rest == "---" {
                let block = self.block("divider", json!({}), Vec::new());
                out.push(block);
                i += 1;
                continue;
            }
            if let Some((level, text)) = heading(rest) {
                let rich = self.rich(text);
                let block = self.block(
                    &format!("heading_{level}"),
                    json!({"rich_text": rich, "is_toggleable": false, "color": "default"}),
                    Vec::new(),
                );
                out.push(block);
                i += 1;
                continue;
            }
            if rest.starts_with("<aside") {
                let mut body = Vec::new();
                i += 1;
                while i < lines.len() && !lines[i].trim().starts_with("</aside>") {
                    if !lines[i].trim().is_empty() {
                        body.push(lines[i].trim().to_owned());
                    }
                    i += 1;
                }
                i += 1;
                let mut text = body.join("\n");
                let mut icon = Value::Null;
                if let Some(first) = text.chars().next()
                    && !first.is_ascii()
                    && let Some((emoji, after)) = text.split_once(' ')
                    && emoji.chars().count() <= 2
                {
                    icon = json!({"type": "emoji", "emoji": emoji});
                    text = after.trim_start().to_owned();
                }
                let rich = self.rich(&text);
                let block = self.block(
                    "callout",
                    json!({"rich_text": rich, "icon": icon, "color": "default"}),
                    Vec::new(),
                );
                out.push(block);
                continue;
            }
            if rest.starts_with("> ") || rest == ">" {
                let mut body = Vec::new();
                while i < lines.len() {
                    let r = lines[i].trim();
                    if let Some(t) = r.strip_prefix("> ") {
                        body.push(t.to_owned());
                    } else if r == ">" {
                        body.push(String::new());
                    } else {
                        break;
                    }
                    i += 1;
                }
                let rich = self.rich(&body.join("\n"));
                let block = self.block(
                    "quote",
                    json!({"rich_text": rich, "color": "default"}),
                    Vec::new(),
                );
                out.push(block);
                continue;
            }
            if rest.starts_with('|') {
                let mut rows = Vec::new();
                let mut header = false;
                while i < lines.len() && lines[i].trim().starts_with('|') {
                    let cells = table_cells(lines[i].trim());
                    if cells
                        .iter()
                        .all(|c| !c.is_empty() && c.chars().all(|ch| matches!(ch, '-' | ':' | ' ')))
                    {
                        header = rows.len() == 1;
                    } else {
                        rows.push(cells);
                    }
                    i += 1;
                }
                let width = rows.iter().map(Vec::len).max().unwrap_or(0);
                let mut row_blocks = Vec::new();
                for row in rows {
                    let mut cells = Vec::new();
                    for col in 0..width {
                        cells.push(Value::Array(
                            self.rich(row.get(col).map(String::as_str).unwrap_or("")),
                        ));
                    }
                    row_blocks.push(self.block("table_row", json!({"cells": cells}), Vec::new()));
                }
                let block = self.block(
                    "table",
                    json!({"table_width": width, "has_column_header": header, "has_row_header": false}),
                    row_blocks,
                );
                out.push(block);
                continue;
            }
            if let Some((block_type, text, checked)) = is_list_marker(rest) {
                let text = text.to_owned();
                i += 1;
                let child_lines = take_children(lines, &mut i, indent);
                let id = self.next_id();
                let rich = self.rich(&text);
                let children = self.blocks(&child_lines);
                let mut payload = json!({"rich_text": rich, "color": "default"});
                if let Some(checked) = checked {
                    payload["checked"] = json!(checked);
                }
                let block = self.block_with_id(id, block_type, payload, children);
                out.push(block);
                continue;
            }
            // A line that is only a link to a direct child page/database
            // file becomes that child's block, as in the API.
            if let Some(child) = self.child_link(rest) {
                out.push(child);
                i += 1;
                continue;
            }
            if let Some(image) = self.image(rest) {
                out.push(image);
                i += 1;
                continue;
            }
            // Paragraph: consecutive plain lines at this indent.
            let mut body = vec![rest.to_owned()];
            i += 1;
            while i < lines.len() {
                let r = lines[i].trim();
                if r.is_empty() || indent_of(&lines[i]) != indent || starts_structure(r) {
                    break;
                }
                body.push(r.to_owned());
                i += 1;
            }
            let child_lines = take_children(lines, &mut i, indent);
            let id = self.next_id();
            let rich = self.rich(&body.join("\n"));
            let children = self.blocks(&child_lines);
            let block = self.block_with_id(
                id,
                "paragraph",
                json!({"rich_text": rich, "color": "default"}),
                children,
            );
            out.push(block);
        }
        out
    }

    fn resolve_rel(&self, target: &str) -> String {
        let decoded = percent_decode(target);
        let (dir, _) = split_path(self.page_path);
        let mut parts: Vec<&str> = if dir.is_empty() {
            Vec::new()
        } else {
            dir.split('/').collect()
        };
        for part in decoded.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    parts.pop();
                }
                other => parts.push(other),
            }
        }
        parts.join("/")
    }

    fn child_link(&mut self, rest: &str) -> Option<Value> {
        let (title, target) = whole_link(rest)?;
        if target.contains("://") {
            return None;
        }
        let path = self.resolve_rel(target);
        let page_dir = format!("{}/", self.page_path.trim_end_matches(".md"));
        let tail = path.strip_prefix(&page_dir)?;
        if tail.contains('/') {
            return None;
        }
        let (id, kind) = self.path_to_id.get(&path).or_else(|| {
            let all = path.trim_end_matches(".csv").to_owned() + "_all.csv";
            self.path_to_id.get(&all)
        })?;
        let (id, kind) = (id.clone(), *kind);
        let block_type = match kind {
            ExportKind::Page => "child_page",
            ExportKind::Database => "child_database",
        };
        Some(self.block_with_id(id, block_type, json!({"title": title}), Vec::new()))
    }

    fn image(&mut self, rest: &str) -> Option<Value> {
        let inner = rest.strip_prefix('!')?;
        let (alt, target) = whole_link(inner)?;
        let caption = if alt.is_empty() {
            Vec::new()
        } else {
            self.rich(alt)
        };
        let payload = if target.contains("://") {
            json!({"type": "external", "external": {"url": target}, "caption": caption})
        } else {
            let path = self.resolve_rel(target);
            self.archive.file(&path)?;
            json!({"type": "file", "file": {"url": path}, "caption": caption})
        };
        Some(self.block("image", payload, Vec::new()))
    }

    /// Rewrite a link target: an export-relative page/database file
    /// becomes the Notion URL of its matched object, so the converter's
    /// §3 link rewriting resolves it like an API link.
    fn link_target(&self, target: &str) -> String {
        if target.contains("://") {
            return target.to_owned();
        }
        let path = self.resolve_rel(target);
        if let Some((id, _)) = self.path_to_id.get(&path) {
            return format!("https://www.notion.so/{}", normalize_uuid(id));
        }
        let (_, name) = split_path(&path);
        let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
        if let (_, Some(uuid)) = split_title_uuid(stem.trim_end_matches("_all")) {
            let target = self
                .uuid_to_id
                .get(&uuid)
                .map_or(uuid.clone(), |id| normalize_uuid(id));
            return format!("https://www.notion.so/{target}");
        }
        path
    }

    fn rich(&self, text: &str) -> Vec<Value> {
        let mut spans = Vec::new();
        parse_inline(text, Annot::default(), None, self, &mut spans);
        spans
    }
}

fn heading(rest: &str) -> Option<(usize, &str)> {
    for (level, prefix) in [(3, "### "), (2, "## "), (1, "# ")] {
        if let Some(text) = rest.strip_prefix(prefix) {
            return Some((level, text));
        }
    }
    None
}

/// Lines indented deeper than `indent` after position `*i` (blank lines
/// inside the run included), de-indented to the child level.
fn take_children(lines: &[String], i: &mut usize, indent: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut j = *i;
    while j < lines.len() {
        let line = &lines[j];
        if line.trim().is_empty() {
            let next_deeper = lines[j..]
                .iter()
                .find(|l| !l.trim().is_empty())
                .is_some_and(|l| indent_of(l) > indent);
            if !next_deeper {
                break;
            }
            out.push(String::new());
            j += 1;
            continue;
        }
        if indent_of(line) <= indent {
            break;
        }
        out.push(line.clone());
        j += 1;
    }
    *i = j;
    let min = out
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| indent_of(l))
        .min()
        .unwrap_or(0);
    out.into_iter()
        .map(|l| l.get(min..).map_or_else(String::new, str::to_owned))
        .collect()
}

fn table_cells(line: &str) -> Vec<String> {
    let inner = line.trim().trim_start_matches('|');
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    inner.split('|').map(|c| c.trim().to_owned()).collect()
}

/// `[text](target)` spanning the whole string.
fn whole_link(rest: &str) -> Option<(&str, &str)> {
    let inner = rest.strip_prefix('[')?;
    let (text, after) = split_link(inner)?;
    if after.trim().is_empty() {
        Some(text)
    } else {
        None
    }
}

/// Given the text after `[`, return ((text, target), remainder after `)`).
fn split_link(inner: &str) -> Option<((&str, &str), &str)> {
    let close = inner.find("](")?;
    let text = &inner[..close];
    let after = &inner[close + 2..];
    let mut depth = 0usize;
    for (pos, c) in after.char_indices() {
        match c {
            '(' => depth += 1,
            ')' if depth == 0 => return Some(((text, &after[..pos]), &after[pos + 1..])),
            ')' => depth -= 1,
            _ => {}
        }
    }
    None
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Some(hi) = (bytes[i + 1] as char).to_digit(16)
            && let Some(lo) = (bytes[i + 2] as char).to_digit(16)
        {
            out.push((hi * 16 + lo) as u8);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[derive(Clone, Copy, Default, PartialEq)]
struct Annot {
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
}

fn text_item(text: &str, annot: Annot, link: Option<&str>) -> Value {
    let mut item = json!({
        "type": "text",
        "plain_text": text,
        "text": {"content": text},
        "annotations": {
            "bold": annot.bold,
            "italic": annot.italic,
            "strikethrough": annot.strike,
            "underline": false,
            "code": annot.code,
            "color": "default",
        },
    });
    if let Some(url) = link {
        item["text"]["link"] = json!({"url": url});
        item["href"] = json!(url);
    }
    item
}

fn flush(buf: &mut String, annot: Annot, link: Option<&str>, out: &mut Vec<Value>) {
    if !buf.is_empty() {
        out.push(text_item(buf, annot, link));
        buf.clear();
    }
}

/// Inline Markdown -> Notion rich-text items: `**bold**`, `*italic*` /
/// `_italic_`, `~~strike~~`, `` `code` ``, `[text](url)`, `\` escapes.
fn parse_inline(
    text: &str,
    mut annot: Annot,
    link: Option<&str>,
    normalizer: &Normalizer<'_>,
    out: &mut Vec<Value>,
) {
    let mut buf = String::new();
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut k = 0;
    while k < chars.len() {
        let (pos, c) = chars[k];
        let next = chars.get(k + 1).map(|(_, c)| *c);
        let prev = if k == 0 { None } else { Some(chars[k - 1].1) };
        match c {
            '\\' if next.is_some_and(|n| n.is_ascii_punctuation()) => {
                if let Some(n) = next {
                    buf.push(n);
                }
                k += 2;
            }
            '`' => {
                let rest = &text[pos + 1..];
                if let Some(end) = rest.find('`') {
                    flush(&mut buf, annot, link, out);
                    let code = Annot {
                        code: true,
                        ..annot
                    };
                    out.push(text_item(&rest[..end], code, link));
                    let consumed = pos + 1 + end + 1;
                    k = chars
                        .iter()
                        .position(|(p, _)| *p >= consumed)
                        .unwrap_or(chars.len());
                } else {
                    buf.push(c);
                    k += 1;
                }
            }
            '[' if link.is_none() => {
                if let Some(((inner, target), _)) = split_link(&text[pos + 1..]) {
                    flush(&mut buf, annot, link, out);
                    let url = normalizer.link_target(target);
                    parse_inline(inner, annot, Some(&url), normalizer, out);
                    let consumed = pos + 1 + inner.len() + 2 + target.len() + 1;
                    k = chars
                        .iter()
                        .position(|(p, _)| *p >= consumed)
                        .unwrap_or(chars.len());
                } else {
                    buf.push(c);
                    k += 1;
                }
            }
            '*' if next == Some('*') => {
                flush(&mut buf, annot, link, out);
                annot.bold = !annot.bold;
                k += 2;
            }
            '~' if next == Some('~') => {
                flush(&mut buf, annot, link, out);
                annot.strike = !annot.strike;
                k += 2;
            }
            '*' => {
                flush(&mut buf, annot, link, out);
                annot.italic = !annot.italic;
                k += 1;
            }
            '_' if (!annot.italic
                && !prev.is_some_and(char::is_alphanumeric)
                && next.is_some_and(|n| !n.is_whitespace()))
                || (annot.italic && !next.is_some_and(char::is_alphanumeric)) =>
            {
                flush(&mut buf, annot, link, out);
                annot.italic = !annot.italic;
                k += 1;
            }
            _ => {
                buf.push(c);
                k += 1;
            }
        }
    }
    flush(&mut buf, annot, link, out);
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn file_names_split_into_title_and_uuid() {
        assert_eq!(
            split_title_uuid("Alpha Notes 0123456789abcdef0123456789ABCDEF"),
            (
                "Alpha Notes".to_owned(),
                Some("0123456789abcdef0123456789abcdef".to_owned())
            )
        );
        assert_eq!(split_title_uuid("Plain"), ("Plain".to_owned(), None));
        // A 32-character title that is not hex keeps its name.
        assert_eq!(
            split_title_uuid("Notes zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz").1,
            None
        );
    }

    #[test]
    fn csv_handles_quotes_commas_and_newlines() {
        let rows = parse_csv("Name,Notes\r\n\"A, B\",\"say \"\"hi\"\"\"\nC,\"x\ny\"\n").unwrap();
        assert_eq!(
            rows,
            vec![
                vec!["Name".to_owned(), "Notes".to_owned()],
                vec!["A, B".to_owned(), "say \"hi\"".to_owned()],
                vec!["C".to_owned(), "x\ny".to_owned()],
            ]
        );
        assert!(parse_csv("\"open").is_err());
    }

    #[test]
    fn percent_decoding_and_links() {
        assert_eq!(percent_decode("A%20B%2Fc"), "A B/c");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(
            whole_link("[Beta](Alpha/Beta%20(x).md)"),
            Some(("Beta", "Alpha/Beta%20(x).md"))
        );
        assert_eq!(whole_link("[Beta](b.md) tail"), None);
    }

    #[test]
    fn export_paths_may_not_escape_the_root() {
        let files = BTreeMap::from([("../x.md".to_owned(), b"# X".to_vec())]);
        assert!(ExportArchive::from_files(files).is_err());
    }
}
