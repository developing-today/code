//! The provenance source registry (developing-today/code#3125,
//! slice 4): stable source-root and source-locator registration.
//!
//! The occurrence registry ([`crate::aggregate`]'s shard-side
//! `chunk_occurrences`) records, for every promoted chunk, the source
//! locator it was found at. Locators hang from **source roots**: a
//! root is a stable named origin — a pinned repository
//! (`repo@commit`), an extracts corpus, a filesystem base — and a
//! locator is the full source string as it appears in occurrence
//! rows (a plain path, or the pinned form `repo:path@rev` the book's
//! corpus-and-provenance chapter uses). Phase 4 of #3125 resolves an
//! ID to a file, a chunk, or a section through a catalog; resolution
//! starts from knowing, stably, which roots and locators exist. This
//! module is that registration layer.
//!
//! The stability contract, which the oracle tests in
//! `tests/source_registry.rs` pin:
//!
//! - Registering the same root with the same base locator, or the
//!   same locator under the same root, any number of times yields
//!   exactly one row. The first registration wins: a repeat returns
//!   the stored row unchanged (including its original
//!   `registered_at`) and reports `created: false`.
//! - A root id may not be re-pointed: registering an existing root
//!   id with a *different* base locator is refused, because silently
//!   moving a root would rewrite the meaning of every locator and
//!   occurrence already recorded under it.
//! - A locator may only be registered under a root that exists;
//!   an unknown root is refused rather than leaving an orphan row
//!   resolution could never anchor.
//! - Registration is additive only. It creates the two registry
//!   tables when absent and inserts registration rows; it never
//!   updates or deletes any row, in its own tables or in any other
//!   table of the database it is given (the non-alteration oracle
//!   snapshots a seeded shard schema — `chunks` plus
//!   `chunk_occurrences` — across registrations).
//!
//! The registry lives in whichever database the caller opens — in
//! production the phase-4 catalog database; the functions here take
//! the connection, mirroring how [`crate::stage`] takes its unit
//! databases.

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::{Error, Result};

/// DDL for the source registry's two tables. The registry is code,
/// built once per database; only registrations are data.
pub const SOURCE_REGISTRY_DDL: &str = "CREATE TABLE IF NOT EXISTS source_roots (\
    root_id TEXT PRIMARY KEY, \
    base_locator TEXT NOT NULL, \
    registered_at TEXT NOT NULL); \
    CREATE TABLE IF NOT EXISTS source_locators (\
    root_id TEXT NOT NULL REFERENCES source_roots(root_id), \
    locator TEXT NOT NULL, \
    registered_at TEXT NOT NULL, \
    PRIMARY KEY (root_id, locator))";

/// One registered source root: a stable id, the base locator every
/// locator under it hangs from, and when it was first registered.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceRoot {
    pub root_id: String,
    pub base_locator: String,
    pub registered_at: String,
}

/// The outcome of a root registration: the stored root (the first
/// registration's row on a repeat) and whether this call created it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RootRegistration {
    pub root: SourceRoot,
    pub created: bool,
}

/// The outcome of a locator registration: whether this call created
/// the row. A repeat leaves the stored row — including its original
/// `registered_at` — untouched.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocatorRegistration {
    pub created: bool,
}

/// Create the registry tables in `conn`'s database when absent.
/// Idempotent and additive: existing tables and rows are untouched.
pub fn ensure_source_registry(conn: &Connection) -> Result<()> {
    conn.execute_batch(SOURCE_REGISTRY_DDL)
        .map_err(|e| Error::backend(format!("source registry: creating tables: {e}")))
}

fn require_non_empty(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(Error::fatal(format!(
            "source registry: {field} must not be empty"
        )));
    }
    Ok(())
}

/// Read one root back, or `None` when it was never registered.
pub fn source_root(conn: &Connection, root_id: &str) -> Result<Option<SourceRoot>> {
    ensure_source_registry(conn)?;
    conn.query_row(
        "SELECT root_id, base_locator, registered_at FROM source_roots WHERE root_id = ?1",
        params![root_id],
        |r| {
            Ok(SourceRoot {
                root_id: r.get(0)?,
                base_locator: r.get(1)?,
                registered_at: r.get(2)?,
            })
        },
    )
    .optional()
    .map_err(|e| Error::backend(format!("source registry: reading root {root_id:?}: {e}")))
}

/// Register a source root, stably.
///
/// The first registration stores `(root_id, base_locator,
/// registered_at)` and reports `created: true`. A repeat with the
/// same base locator returns the stored row with `created: false`
/// and changes nothing — the original `registered_at` stands. A
/// repeat with a different base locator is refused: a root id may
/// not be re-pointed.
pub fn register_source_root(
    conn: &Connection,
    root_id: &str,
    base_locator: &str,
    registered_at: &str,
) -> Result<RootRegistration> {
    require_non_empty("root_id", root_id)?;
    require_non_empty("base_locator", base_locator)?;
    require_non_empty("registered_at", registered_at)?;
    ensure_source_registry(conn)?;
    if let Some(existing) = source_root(conn, root_id)? {
        if existing.base_locator != base_locator {
            return Err(Error::fatal(format!(
                "source registry: root {root_id:?} is already registered with base locator \
                 {:?}; refusing to re-point it to {base_locator:?}",
                existing.base_locator
            )));
        }
        return Ok(RootRegistration {
            root: existing,
            created: false,
        });
    }
    conn.execute(
        "INSERT INTO source_roots (root_id, base_locator, registered_at) VALUES (?1, ?2, ?3)",
        params![root_id, base_locator, registered_at],
    )
    .map_err(|e| {
        Error::backend(format!(
            "source registry: registering root {root_id:?}: {e}"
        ))
    })?;
    Ok(RootRegistration {
        root: SourceRoot {
            root_id: root_id.to_owned(),
            base_locator: base_locator.to_owned(),
            registered_at: registered_at.to_owned(),
        },
        created: true,
    })
}

/// Register a source locator under an existing root, stably.
///
/// The first registration stores the `(root_id, locator)` row and
/// reports `created: true`. A repeat returns `created: false` and
/// changes nothing. Registering under a root that does not exist is
/// refused — a locator with no root could never be resolved.
pub fn register_locator(
    conn: &Connection,
    root_id: &str,
    locator: &str,
    registered_at: &str,
) -> Result<LocatorRegistration> {
    require_non_empty("root_id", root_id)?;
    require_non_empty("locator", locator)?;
    require_non_empty("registered_at", registered_at)?;
    ensure_source_registry(conn)?;
    if source_root(conn, root_id)?.is_none() {
        return Err(Error::fatal(format!(
            "source registry: refusing to register locator {locator:?} under unknown root \
             {root_id:?}; register the root first"
        )));
    }
    let inserted = conn
        .execute(
            "INSERT OR IGNORE INTO source_locators (root_id, locator, registered_at) \
             VALUES (?1, ?2, ?3)",
            params![root_id, locator, registered_at],
        )
        .map_err(|e| {
            Error::backend(format!(
                "source registry: registering locator {locator:?} under root {root_id:?}: {e}"
            ))
        })?;
    Ok(LocatorRegistration {
        created: inserted == 1,
    })
}

/// The locators registered under one root, sorted — the stable
/// order resolution and listings consume. An unknown root yields an
/// empty list.
pub fn locators_for_root(conn: &Connection, root_id: &str) -> Result<Vec<String>> {
    ensure_source_registry(conn)?;
    let mut stmt = conn
        .prepare("SELECT locator FROM source_locators WHERE root_id = ?1 ORDER BY locator")
        .map_err(|e| Error::backend(format!("source registry: listing locators: {e}")))?;
    let rows = stmt
        .query_map(params![root_id], |r| r.get::<_, String>(0))
        .map_err(|e| Error::backend(format!("source registry: listing locators: {e}")))?;
    let mut locators = Vec::new();
    for row in rows {
        locators.push(
            row.map_err(|e| Error::backend(format!("source registry: listing locators: {e}")))?,
        );
    }
    Ok(locators)
}
