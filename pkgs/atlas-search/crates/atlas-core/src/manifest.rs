//! The classification manifest: schema, validation, and loader
//! (developing-today/code#6228 design
//! `doc/2026-11-05T13-03-49Z_design_atlas_manifest_filter_surface`,
//! §§3–7; phase 2 of its §8 delivery plan, issue #6993).
//!
//! The manifest is a small table in the corpus database declaring the
//! filterable dimensions — name, storage shape (§4), match semantics
//! (§5), and the label column a column shape binds to. atlas-core
//! loads it at backend open and (from phase 3) builds its filter
//! surface from it. The code-defined surface of today is the
//! **built-in manifest**: the fallback when no manifest is present,
//! and the reference a loaded manifest is validated against.
//!
//! The safety invariant (§6) is enforced here, at load: the manifest
//! schema has no sensitivity keys at all; column shapes bind only to
//! the code-known label columns; `generic_list` dimension keys are
//! data but fenced by a code-side pattern and a reserved-name list;
//! an unknown shape, semantics, column, or key fails validation and
//! the caller falls back to the built-in manifest, loudly (§7).

use rusqlite::Connection;
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

/// The manifest table's name in the corpus database (§3).
pub const MANIFEST_TABLE: &str = "classification_manifest";

/// The generic label store's table (§4 `generic_list`): one row per
/// (session, dimension key), value = JSON list text.
pub const GENERIC_LABELS_TABLE: &str = "generic_labels";

/// DDL for the generic label store. The store is code, built once;
/// only its dimension keys are data (§4).
pub const GENERIC_LABELS_DDL: &str = "CREATE TABLE IF NOT EXISTS generic_labels (\
    session_id TEXT NOT NULL, \
    dimension TEXT NOT NULL, \
    values_json TEXT NOT NULL, \
    PRIMARY KEY (session_id, dimension))";

/// The highest manifest version this binary understands. A manifest
/// newer than this falls back to the built-in surface (§7).
pub const SUPPORTED_MANIFEST_VERSION: i64 = 1;

/// Label columns a column shape may bind to (§6.2 allowlist, in
/// code). A manifest naming anything else fails validation.
pub const LABEL_COLUMN_ALLOWLIST: [&str; 5] =
    ["kind", "client", "topics", "projects", "created_at"];

/// Dimension names of the built-in surface. A `generic_list` key may
/// not collide with one (§6.2 reserved names).
pub const BUILTIN_DIMENSION_NAMES: [&str; 5] =
    ["kind", "client", "topics", "projects", "created_at"];

/// Names in the sensitivity namespace. No manifest dimension may
/// live there — the schema has no sensitivity keys (§6.1), so a row
/// reaching for one is a validation error, never a feature.
pub const SENSITIVITY_NAMESPACE: [&str; 4] = ["sensitivity", "max_sensitivity", "ceiling", "sens"];

/// A dimension name (or `generic_list` key) in the sensitivity
/// namespace: exactly a reserved word, or built from one
/// (`sensitivity_level`, `sens_tier`, ...).
fn in_sensitivity_namespace(name: &str) -> bool {
    SENSITIVITY_NAMESPACE.iter().any(|w| {
        name == *w || name.starts_with(&format!("{w}_")) || name.starts_with(&format!("{w}-"))
    })
}

/// The code-side pattern for `generic_list` dimension keys (§6.2):
/// lowercase snake_case starting with a letter.
fn valid_generic_key(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Storage shapes (§4's closed vocabulary). A new shape is a code
/// change, deliberately: shapes are where SQL is born.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    ScalarIn,
    JsonList,
    TimestampRange,
    GenericList,
}

impl Shape {
    pub fn from_name(name: &str) -> Option<Shape> {
        match name {
            "scalar_in" => Some(Shape::ScalarIn),
            "json_list" => Some(Shape::JsonList),
            "timestamp_range" => Some(Shape::TimestampRange),
            "generic_list" => Some(Shape::GenericList),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Shape::ScalarIn => "scalar_in",
            Shape::JsonList => "json_list",
            Shape::TimestampRange => "timestamp_range",
            Shape::GenericList => "generic_list",
        }
    }
}

/// Match semantics (§5). `exact` is the default; `pattern` is an
/// explicit per-dimension opt-in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Semantics {
    Exact,
    Pattern,
}

impl Semantics {
    pub fn from_name(name: &str) -> Option<Semantics> {
        match name {
            "exact" => Some(Semantics::Exact),
            "pattern" => Some(Semantics::Pattern),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Semantics::Exact => "exact",
            Semantics::Pattern => "pattern",
        }
    }
}

/// One declared dimension. For the column shapes, `label_column` is
/// the allowlisted label column the dimension binds to; for
/// `generic_list` it is `None` and the dimension key is the name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dimension {
    pub name: String,
    pub shape: Shape,
    pub semantics: Semantics,
    pub label_column: Option<String>,
}

/// A validated manifest: header (version + content hash) and its
/// dimension rows, in declaration order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub version: i64,
    pub content_hash: String,
    pub dimensions: Vec<Dimension>,
}

impl Manifest {
    /// The canonical serialization the content hash covers: one
    /// line per dimension, in declaration order.
    fn canonical(&self) -> String {
        let mut out = format!("v{}\n", self.version);
        for d in &self.dimensions {
            out.push_str(&format!(
                "{}|{}|{}|{}\n",
                d.name,
                d.shape.name(),
                d.semantics.name(),
                d.label_column.as_deref().unwrap_or("")
            ));
        }
        out
    }

    /// SHA-256 over the canonical form — the `content_hash` a stored
    /// manifest must carry (§3) and the surface identity (§7).
    pub fn compute_hash(&self) -> String {
        let mut h = Sha256::new();
        h.update(self.canonical().as_bytes());
        h.finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    }

    /// The effective surface identity for dedupe keys and source
    /// descriptions (§7): `(version, hash)` of the loaded manifest,
    /// or the fixed `builtin` sentinel for the built-in surface.
    pub fn surface_identity(&self, loaded: bool) -> String {
        if loaded {
            format!("manifest v{}:{}", self.version, self.content_hash)
        } else {
            "builtin".to_string()
        }
    }

    pub fn dimension(&self, name: &str) -> Option<&Dimension> {
        self.dimensions.iter().find(|d| d.name == name)
    }

    /// Validate against §6. Returns the first violation found.
    pub fn validate(&self) -> Result<()> {
        let mut seen: Vec<&str> = Vec::new();
        for d in &self.dimensions {
            if in_sensitivity_namespace(&d.name) {
                return Err(Error::fatal(format!(
                    "classification manifest: dimension '{}' is in the sensitivity namespace — \
                     the manifest schema has no sensitivity keys (design §6.1)",
                    d.name
                )));
            }
            if seen.contains(&d.name.as_str()) {
                return Err(Error::fatal(format!(
                    "classification manifest: duplicate dimension '{}'",
                    d.name
                )));
            }
            seen.push(d.name.as_str());
            match d.shape {
                Shape::GenericList => {
                    if d.label_column.is_some() {
                        return Err(Error::fatal(format!(
                            "classification manifest: generic_list dimension '{}' must not name a label column — \
                             its binding is the generic label store (design §6.2)",
                            d.name
                        )));
                    }
                    if !valid_generic_key(&d.name) {
                        return Err(Error::fatal(format!(
                            "classification manifest: generic_list key '{}' fails the code-side key pattern \
                             (lowercase snake_case, letter first; design §6.2)",
                            d.name
                        )));
                    }
                    if BUILTIN_DIMENSION_NAMES.contains(&d.name.as_str()) {
                        return Err(Error::fatal(format!(
                            "classification manifest: generic_list key '{}' collides with a built-in dimension \
                             (design §6.2 reserved names)",
                            d.name
                        )));
                    }
                }
                _ => {
                    let col = d.label_column.as_deref().ok_or_else(|| {
                        Error::fatal(format!(
                            "classification manifest: dimension '{}' ({}) names no label column",
                            d.name,
                            d.shape.name()
                        ))
                    })?;
                    if !LABEL_COLUMN_ALLOWLIST.contains(&col) {
                        return Err(Error::fatal(format!(
                            "classification manifest: dimension '{}' binds unknown label column '{col}' — \
                             bindings are the code-known columns only (design §6.2)",
                            d.name
                        )));
                    }
                }
            }
        }
        Ok(())
    }
}

/// Today's code-defined surface, expressed in the manifest's own
/// schema (§2/§7): the fallback when no manifest is present, and the
/// reference a loaded manifest is validated against.
pub fn builtin_manifest() -> Manifest {
    let mut m = Manifest {
        version: SUPPORTED_MANIFEST_VERSION,
        content_hash: String::new(),
        dimensions: vec![
            Dimension {
                name: "kind".to_string(),
                shape: Shape::ScalarIn,
                semantics: Semantics::Exact,
                label_column: Some("kind".to_string()),
            },
            Dimension {
                name: "client".to_string(),
                shape: Shape::ScalarIn,
                semantics: Semantics::Exact,
                label_column: Some("client".to_string()),
            },
            Dimension {
                name: "topics".to_string(),
                shape: Shape::JsonList,
                semantics: Semantics::Exact,
                label_column: Some("topics".to_string()),
            },
            Dimension {
                name: "projects".to_string(),
                shape: Shape::JsonList,
                semantics: Semantics::Exact,
                label_column: Some("projects".to_string()),
            },
            Dimension {
                name: "created_at".to_string(),
                shape: Shape::TimestampRange,
                semantics: Semantics::Exact,
                label_column: Some("created_at".to_string()),
            },
        ],
    };
    m.content_hash = m.compute_hash();
    m
}

/// Where a loaded surface came from, for the backend's source
/// description (§7's provenance pattern).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestSource {
    /// No manifest table (or no rows): the built-in surface applies.
    BuiltinAbsent,
    /// A manifest was present but rejected; the built-in surface
    /// applies and `warning` says why (§7: fall back, loudly).
    BuiltinFallback,
    /// A manifest table in this schema supplied the surface.
    Table { schema: String },
}

/// The loader's outcome: the effective manifest, its source, and the
/// warning to log when the surface fell back (§7).
#[derive(Debug, Clone)]
pub struct LoadedManifest {
    pub manifest: Manifest,
    pub source: ManifestSource,
    pub warning: Option<String>,
}

impl LoadedManifest {
    fn fallback(warning: String) -> LoadedManifest {
        LoadedManifest {
            manifest: builtin_manifest(),
            source: ManifestSource::BuiltinFallback,
            warning: Some(warning),
        }
    }

    /// Provenance string in the pattern `local.rs` uses for
    /// sensitivity sources.
    pub fn describe(&self) -> String {
        match &self.source {
            ManifestSource::BuiltinAbsent => {
                "built-in manifest (no classification_manifest table)".to_string()
            }
            ManifestSource::BuiltinFallback => format!(
                "built-in manifest (classification_manifest rejected: {})",
                self.warning.as_deref().unwrap_or("invalid")
            ),
            ManifestSource::Table { schema } => format!(
                "classification_manifest ({schema}), {}",
                self.manifest.surface_identity(true)
            ),
        }
    }
}

/// Read and validate the manifest from the first of `schemas` that
/// carries the table. Absent → built-in; malformed, failing §6
/// validation, hash-mismatched, or newer than supported → built-in
/// with a warning (§7: fall back, loudly, never widen). This never
/// fails: a corpus with a bad manifest is searched under the
/// built-in surface, not refused.
pub fn load_manifest(con: &Connection, schemas: &[&str]) -> LoadedManifest {
    for schema in schemas {
        let present: bool = con
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM {schema}.sqlite_master WHERE type='table' AND name='{MANIFEST_TABLE}'"
                ),
                [],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n > 0)
            .unwrap_or(false);
        if !present {
            continue;
        }
        return match read_manifest_table(con, schema) {
            Ok(m) => LoadedManifest {
                manifest: m,
                source: ManifestSource::Table {
                    schema: schema.to_string(),
                },
                warning: None,
            },
            Err(e) => LoadedManifest::fallback(e.to_string()),
        };
    }
    LoadedManifest {
        manifest: builtin_manifest(),
        source: ManifestSource::BuiltinAbsent,
        warning: None,
    }
}

/// The stored shape (§3): one header row (`name IS NULL`) carrying
/// `manifest_version` + `content_hash`, plus one row per dimension
/// repeating them. All rows must agree on version and hash.
fn read_manifest_table(con: &Connection, schema: &str) -> Result<Manifest> {
    let mut stmt = con
        .prepare(&format!(
            "SELECT manifest_version, content_hash, name, shape, semantics, label_column \
             FROM {schema}.{MANIFEST_TABLE} ORDER BY rowid"
        ))
        .map_err(|e| Error::backend(format!("classification manifest: unreadable: {e}")))?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
            ))
        })
        .map_err(|e| Error::backend(format!("classification manifest: unreadable: {e}")))?;
    let mut raw: Vec<ManifestRow> = Vec::new();
    for row in rows {
        raw.push(
            row.map_err(|e| Error::backend(format!("classification manifest: unreadable: {e}")))?,
        );
    }
    parse_manifest_rows(raw)
}

/// One stored manifest row: `(manifest_version, content_hash, name,
/// shape, semantics, label_column)`.
pub type ManifestRow = (
    i64,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// Parse and validate stored manifest rows (§§3/6/7). Transport-free,
/// so the Turso backend can feed it pipeline rows exactly as the
/// local backend feeds it rusqlite rows.
pub fn parse_manifest_rows(rows: Vec<ManifestRow>) -> Result<Manifest> {
    let mut version: Option<i64> = None;
    let mut hash: Option<String> = None;
    let mut dimensions: Vec<Dimension> = Vec::new();
    for (v, h, name, shape, semantics, label_column) in rows {
        match (&version, &hash) {
            (None, None) => {
                version = Some(v);
                hash = Some(h.clone());
            }
            _ => {
                if version != Some(v) || hash.as_deref() != Some(h.as_str()) {
                    return Err(Error::fatal(
                        "classification manifest: rows disagree on manifest_version/content_hash",
                    ));
                }
            }
        }
        let Some(name) = name else { continue }; // header row
        let shape = Shape::from_name(shape.as_deref().unwrap_or("")).ok_or_else(|| {
            Error::fatal(format!(
                "classification manifest: dimension '{name}' has unknown shape '{}'",
                shape.as_deref().unwrap_or("")
            ))
        })?;
        let semantics =
            Semantics::from_name(semantics.as_deref().unwrap_or("exact")).ok_or_else(|| {
                Error::fatal(format!(
                    "classification manifest: dimension '{name}' has unknown semantics '{}'",
                    semantics.as_deref().unwrap_or("")
                ))
            })?;
        dimensions.push(Dimension {
            name,
            shape,
            semantics,
            label_column,
        });
    }
    let version = version
        .ok_or_else(|| Error::fatal("classification manifest: table is empty — no header row"))?;
    if version > SUPPORTED_MANIFEST_VERSION {
        return Err(Error::fatal(format!(
            "classification manifest: manifest_version {version} is newer than this binary supports \
             ({SUPPORTED_MANIFEST_VERSION})"
        )));
    }
    let manifest = Manifest {
        version,
        content_hash: hash.unwrap_or_default(),
        dimensions,
    };
    manifest.validate()?;
    if manifest.compute_hash() != manifest.content_hash {
        return Err(Error::fatal(
            "classification manifest: content_hash does not match the dimension rows",
        ));
    }
    Ok(manifest)
}

/// Query construction for a `generic_list` dimension (§4/§5): the
/// sessions whose generic-store list for `dimension` contains any of
/// `values` as a decoded JSON element. `session_expr` is the SQL
/// expression naming the session id in the caller's query (e.g.
/// `s.id`). Exact semantics are a `json_each` decoded-element
/// comparison — the implementation §5 names as equally valid — so a
/// requested value can never line up across element boundaries.
/// Pattern semantics translate each glob (`*`/`?`) to `LIKE` with
/// everything else escaped; a raw user `LIKE` pattern is never
/// interpolated (§5).
pub fn generic_list_filter(
    dimension: &str,
    semantics: Semantics,
    values: &[String],
    session_expr: &str,
) -> (String, Vec<String>) {
    let mut params: Vec<String> = vec![dimension.to_string()];
    let condition = match semantics {
        Semantics::Exact => {
            let placeholders = values.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
            params.extend(values.iter().cloned());
            format!("je.value IN ({placeholders})")
        }
        Semantics::Pattern => {
            let parts: Vec<String> = values
                .iter()
                .map(|_| "je.value LIKE ? ESCAPE '\\'".to_string())
                .collect();
            params.extend(values.iter().map(|v| glob_to_like(v)));
            parts.join(" OR ")
        }
    };
    let sql = format!(
        "EXISTS (SELECT 1 FROM {GENERIC_LABELS_TABLE} g, json_each(g.values_json) je \
         WHERE g.session_id = {session_expr} AND g.dimension = ? AND ({condition}))"
    );
    (sql, params)
}

/// Glob → `LIKE` translation (§5): `*` → `%`, `?` → `_`, and the
/// `LIKE` metacharacters in the literal text escaped with `\`.
fn glob_to_like(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '*' => out.push('%'),
            '?' => out.push('_'),
            '%' | '_' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}
