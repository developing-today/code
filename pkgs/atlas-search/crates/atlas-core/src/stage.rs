//! The Atlas staging tier: the versioned per-unit schema, one
//! provisioning tool, and the unit registry (developing-today/code#5669,
//! design `doc/2026-11-04T21-27-34Z_design_atlas_scaler_revision`
//! section 4.1).
//!
//! The design's staging tier is "many small DBs, free writes": one
//! database per unit of work that produces data (a queue task, a
//! session, an epic, a notes batch), each carrying the *same* versioned
//! schema — `atlas_stage` v1: `items`, `chunks_raw`, `notes`, `kv`,
//! `_schema_version` — applied by one tool. A registry (internal core
//! tier) maps unit id → staging DB, owner, sensitivity level, state
//! (open / closing / merged) and schema version.
//!
//! Scope of this module (the slice the design's foundation needs, and
//! no more):
//! * [`provision_staging_db`] creates one staging DB and applies the
//!   v1 DDL from the versioned artifact [`STAGE_V1_DDL`];
//! * [`Registry`] records units and looks them up;
//! * [`Registry::provision`] is the one tool that does both.
//!
//! Deliberately out of scope here (later slices of #5669): the
//! aggregator/embedder and merge-on-close promotion, the physical core
//! sensitivity stores, the session-close extraction hook, and the
//! end-to-end demo. Provisioning targets a local libSQL/SQLite file
//! today; hosted Turso creation (`turso db create`) is the deployment
//! surface the aggregator slice names, not this foundation.

use std::collections::BTreeSet;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::{Error, Result};
use crate::shards::Tier;

/// The staging schema version this build knows how to provision and
/// read. Bump only together with a new `atlas_stage_vN.sql` artifact;
/// an applied version is never edited in place (a staging DB is
/// promoted by being read, never by being migrated).
pub const STAGE_SCHEMA_VERSION: i64 = 1;

/// The v1 staging DDL, as the single versioned artifact
/// (`pkgs/atlas-search/schema/atlas_stage_v1.sql`) — the same file one
/// tool applies to every unit.
pub const STAGE_V1_DDL: &str = include_str!("../../../schema/atlas_stage_v1.sql");

/// The registry's own v1 DDL artifact
/// (`pkgs/atlas-search/schema/stage_registry_v1.sql`).
pub const REGISTRY_V1_DDL: &str = include_str!("../../../schema/stage_registry_v1.sql");

/// The exact v1 staging table set, in one place so the schema-version
/// invariant (and tests) can name it.
pub const STAGE_V1_TABLES: [&str; 5] = ["_schema_version", "items", "chunks_raw", "notes", "kv"];

/// A staging unit's lifecycle state (design 4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitState {
    /// Writes are free and unmediated.
    Open,
    /// Closing: the aggregator may pick it up.
    Closing,
    /// Merged: promoted, archived by a branch, dropped from active use.
    Merged,
}

impl UnitState {
    pub fn name(self) -> &'static str {
        match self {
            UnitState::Open => "open",
            UnitState::Closing => "closing",
            UnitState::Merged => "merged",
        }
    }

    pub fn parse(s: &str) -> Option<UnitState> {
        match s {
            "open" => Some(UnitState::Open),
            "closing" => Some(UnitState::Closing),
            "merged" => Some(UnitState::Merged),
            _ => None,
        }
    }
}

impl std::fmt::Display for UnitState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// One row of the registry: unit id → staging DB, owner, sensitivity
/// level, state, schema version (design 4.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagingUnit {
    pub unit_id: String,
    /// The staging database's location as the provisioning tool
    /// recorded it (a local file path today).
    pub db_uri: String,
    pub owner: String,
    /// Physical sensitivity level (design 4.2: public / internal /
    /// secret; the default is internal).
    pub sensitivity: Tier,
    pub state: UnitState,
    pub schema_version: i64,
    pub created_at: String,
    pub updated_at: String,
}

// ------------------------------------------------------- provisioning --

/// Create one staging DB at `path` and apply the v1 DDL. The
/// destination must be a new or empty database: applying v1 over an
/// existing staging set is an error, not a silent no-op, so a
/// re-provision can never half-clobber a unit's data. On success the
/// database reports `_schema_version = 1` and exactly
/// [`STAGE_V1_TABLES`].
pub fn provision_staging_db(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| {
                Error::backend(format!("staging: creating {}: {e}", parent.display()))
            })?;
        }
    }
    let conn = open(path)?;
    let existing = user_tables(&conn)?;
    if !existing.is_empty() {
        return Err(Error::fatal(format!(
            "staging: {} is not empty (tables: {}); refusing to re-provision",
            path.display(),
            existing.into_iter().collect::<Vec<_>>().join(", ")
        )));
    }
    conn.execute_batch(STAGE_V1_DDL).map_err(|e| {
        Error::backend(format!(
            "staging: applying v1 DDL to {}: {e}",
            path.display()
        ))
    })?;
    check_stage_schema(&conn)?;
    Ok(())
}

/// The `_schema_version` a staging database reports.
pub fn schema_version(conn: &Connection) -> Result<i64> {
    conn.query_row("SELECT version FROM _schema_version", [], |r| r.get(0))
        .map_err(|e| Error::backend(format!("staging: reading _schema_version: {e}")))
}

/// The set of user tables in a database (internal SQLite tables
/// excluded), used both for the empty check and the table-set
/// invariant.
pub fn user_tables(conn: &Connection) -> Result<BTreeSet<String>> {
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
        .map_err(|e| Error::backend(format!("staging: listing tables: {e}")))?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| Error::backend(format!("staging: listing tables: {e}")))?;
    let mut out = BTreeSet::new();
    for row in rows {
        out.insert(row.map_err(|e| Error::backend(format!("staging: listing tables: {e}")))?);
    }
    Ok(out)
}

/// Assert a database is exactly a v1 staging DB: `_schema_version = 1`
/// and the exact [`STAGE_V1_TABLES`] table set. This is the invariant
/// the schema oracle checks.
pub fn check_stage_schema(conn: &Connection) -> Result<()> {
    let version = schema_version(conn)?;
    if version != STAGE_SCHEMA_VERSION {
        return Err(Error::fatal(format!(
            "staging: schema version {version} (want {STAGE_SCHEMA_VERSION})"
        )));
    }
    let want: BTreeSet<&str> = STAGE_V1_TABLES.into_iter().collect();
    let got = user_tables(conn)?;
    let got_refs: BTreeSet<&str> = got.iter().map(String::as_str).collect();
    if got_refs != want {
        return Err(Error::fatal(format!(
            "staging: table set {:?} (want {:?})",
            got, STAGE_V1_TABLES
        )));
    }
    Ok(())
}

// ---------------------------------------------------------- registry --

/// The staging-unit registry (design 4.1: unit id → staging DB, owner,
/// sensitivity level, state, schema version). Backed by a libSQL/SQLite
/// file so the foundation works offline; the internal-core-tier
/// placement is the deployment choice, not this API's.
pub struct Registry {
    conn: Connection,
}

impl Registry {
    /// Open (creating if needed) a registry at `path` and ensure its
    /// v1 table exists.
    pub fn open(path: &Path) -> Result<Registry> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    Error::backend(format!("registry: creating {}: {e}", parent.display()))
                })?;
            }
        }
        let conn = open(path)?;
        ensure_registry(&conn)?;
        Ok(Registry { conn })
    }

    /// An in-memory registry (tests, ephemeral tools).
    pub fn in_memory() -> Result<Registry> {
        let conn = Connection::open_in_memory()
            .map_err(|e| Error::backend(format!("registry: opening memory db: {e}")))?;
        ensure_registry(&conn)?;
        Ok(Registry { conn })
    }

    /// The one provisioning tool: create the unit's staging DB, apply
    /// the v1 schema, and register the unit as `open`.
    pub fn provision(
        &self,
        staging_db: &Path,
        unit_id: &str,
        owner: &str,
        sensitivity: Tier,
        now: &str,
    ) -> Result<StagingUnit> {
        provision_staging_db(staging_db)?;
        // Persist an absolute path: a relative one is only meaningful
        // in the provisioning process's working directory, so a
        // registry read from any other directory would resolve the
        // unit's db_uri to a different (or nonexistent) database
        // (Codex P2, PR #7365).
        let db_path = absolute_path(staging_db);
        let unit = StagingUnit {
            unit_id: unit_id.to_string(),
            db_uri: db_path.to_string_lossy().into_owned(),
            owner: owner.to_string(),
            sensitivity,
            state: UnitState::Open,
            schema_version: STAGE_SCHEMA_VERSION,
            created_at: now.to_string(),
            updated_at: now.to_string(),
        };
        // If registration fails, remove the just-provisioned database:
        // leaving it behind would strand an unregistered staging DB
        // that a retry can neither re-provision (the non-empty check
        // refuses) nor recover through the CLI (Codex P2, PR #7365).
        // Cleanup is best-effort; the registration error is the one
        // the caller needs.
        if let Err(e) = self.register(&unit) {
            remove_staging_db(&db_path);
            return Err(e);
        }
        Ok(unit)
    }

    /// Insert (or replace) a unit record. Re-registering a unit id
    /// overwrites it — the registry is a mapping, not an append log.
    pub fn register(&self, unit: &StagingUnit) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO staging_units \
                 (unit_id, db_uri, owner, sensitivity, state, schema_version, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
                 ON CONFLICT(unit_id) DO UPDATE SET \
                   db_uri = excluded.db_uri, owner = excluded.owner, \
                   sensitivity = excluded.sensitivity, state = excluded.state, \
                   schema_version = excluded.schema_version, updated_at = excluded.updated_at",
                params![
                    unit.unit_id,
                    unit.db_uri,
                    unit.owner,
                    unit.sensitivity.name(),
                    unit.state.name(),
                    unit.schema_version,
                    unit.created_at,
                    unit.updated_at,
                ],
            )
            .map_err(|e| Error::backend(format!("registry: registering {}: {e}", unit.unit_id)))?;
        Ok(())
    }

    /// Look a unit up by id.
    pub fn lookup(&self, unit_id: &str) -> Result<Option<StagingUnit>> {
        self.conn
            .query_row(
                "SELECT unit_id, db_uri, owner, sensitivity, state, schema_version, created_at, updated_at \
                 FROM staging_units WHERE unit_id = ?1",
                params![unit_id],
                row_to_unit,
            )
            .optional()
            .map_err(|e| Error::backend(format!("registry: looking up {unit_id}: {e}")))
    }

    /// Everything the aggregator's poll reads: units in a given state.
    pub fn units_in_state(&self, state: UnitState) -> Result<Vec<StagingUnit>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT unit_id, db_uri, owner, sensitivity, state, schema_version, created_at, updated_at \
                 FROM staging_units WHERE state = ?1 ORDER BY unit_id",
            )
            .map_err(|e| Error::backend(format!("registry: listing {}: {e}", state.name())))?;
        let rows = stmt
            .query_map(params![state.name()], row_to_unit)
            .map_err(|e| Error::backend(format!("registry: listing {}: {e}", state.name())))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(
                row.map_err(|e| {
                    Error::backend(format!("registry: listing {}: {e}", state.name()))
                })?,
            );
        }
        Ok(out)
    }

    /// Move a unit to a new lifecycle state.
    pub fn set_state(&self, unit_id: &str, state: UnitState, now: &str) -> Result<()> {
        let changed = self
            .conn
            .execute(
                "UPDATE staging_units SET state = ?2, updated_at = ?3 WHERE unit_id = ?1",
                params![unit_id, state.name(), now],
            )
            .map_err(|e| Error::backend(format!("registry: setting {unit_id} → {state}: {e}")))?;
        if changed == 0 {
            return Err(Error::fatal(format!(
                "registry: no staging unit {unit_id:?} to move to {state}"
            )));
        }
        Ok(())
    }
}

fn row_to_unit(row: &rusqlite::Row<'_>) -> rusqlite::Result<StagingUnit> {
    let state_s: String = row.get(4)?;
    let sens_s: String = row.get(3)?;
    let state = UnitState::parse(&state_s).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            4,
            rusqlite::types::Type::Text,
            format!("unknown staging state {state_s:?}").into(),
        )
    })?;
    let sensitivity = Tier::parse(&sens_s).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            3,
            rusqlite::types::Type::Text,
            format!("unknown sensitivity tier {sens_s:?}").into(),
        )
    })?;
    Ok(StagingUnit {
        unit_id: row.get(0)?,
        db_uri: row.get(1)?,
        owner: row.get(2)?,
        sensitivity,
        state,
        schema_version: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

/// Resolve `path` to an absolute path. The file exists by the time
/// provisioning persists it, so canonicalize is the primary route;
/// fall back to anchoring a relative path at the current directory.
fn absolute_path(path: &Path) -> std::path::PathBuf {
    if let Ok(p) = std::fs::canonicalize(path) {
        return p;
    }
    if path.is_absolute() {
        return path.to_path_buf();
    }
    match std::env::current_dir() {
        Ok(cwd) => cwd.join(path),
        Err(_) => path.to_path_buf(),
    }
}

/// Best-effort removal of a staging database file and its SQLite
/// sidecars (-wal / -shm / -journal). Errors are ignored on purpose:
/// this runs while returning a registration error, and that error —
/// not a cleanup failure — is what the caller must see.
fn remove_staging_db(path: &Path) {
    let _ = std::fs::remove_file(path);
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut sidecar = path.as_os_str().to_owned();
        sidecar.push(suffix);
        let _ = std::fs::remove_file(std::path::PathBuf::from(sidecar));
    }
}

fn ensure_registry(conn: &Connection) -> Result<()> {
    conn.execute_batch(REGISTRY_V1_DDL)
        .map_err(|e| Error::backend(format!("registry: applying v1 DDL: {e}")))
}

fn open(path: &Path) -> Result<Connection> {
    Connection::open(path)
        .map_err(|e| Error::backend(format!("staging: opening {}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Oracle: the filesystem after a failed provision. When the
    /// registry INSERT fails (here: the registry table is gone, the
    /// deterministic stand-in for a lock timeout), the newly
    /// provisioned staging DB must not be left behind — a retry of
    /// the same provision command must be possible once the
    /// transient failure clears (Codex P2, PR #7365).
    #[test]
    fn failed_registration_removes_the_new_staging_db() {
        let dir =
            std::env::temp_dir().join(format!("atlas-stage-unit-{}-regfail", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let conn = Connection::open_in_memory().expect("memory registry");
        ensure_registry(&conn).expect("registry ddl");
        conn.execute_batch("DROP TABLE staging_units")
            .expect("drop registry table so register() fails");
        let reg = Registry { conn };

        let db = dir.join("unit.db");
        let err = reg
            .provision(
                &db,
                "task-12742",
                "repo-worker-code",
                Tier::Internal,
                "2026-11-08T00:00:00Z",
            )
            .expect_err("provision must fail when registration fails");
        assert!(
            err.to_string().contains("registry: registering"),
            "unexpected error: {err}"
        );
        assert!(
            !db.exists(),
            "a failed registration must not strand the provisioned staging DB"
        );

        // And the retry path works once registration can succeed:
        // the same provision command against a healthy registry
        // succeeds instead of dying on the non-empty check.
        let reg2 = Registry::in_memory().expect("healthy registry");
        reg2.provision(
            &db,
            "task-12742",
            "repo-worker-code",
            Tier::Internal,
            "2026-11-08T00:00:00Z",
        )
        .expect("retry after cleanup must succeed");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
