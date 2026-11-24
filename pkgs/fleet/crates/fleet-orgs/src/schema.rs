//! The registry's tables and their upserts. Every statement is safe to
//! run again: `CREATE ... IF NOT EXISTS` for the schema, `ON CONFLICT ...
//! DO UPDATE` for the rows, so a re-sync rewrites the same keys.

use crate::sql::Sql;

pub const SCHEMA: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS sync_runs (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        started_at INTEGER NOT NULL,
        finished_at INTEGER,
        status TEXT NOT NULL,
        installations INTEGER,
        error TEXT
    )",
    "CREATE TABLE IF NOT EXISTS apps (
        slug TEXT PRIMARY KEY,
        app_id INTEGER NOT NULL,
        name TEXT NOT NULL,
        owner TEXT NOT NULL,
        permissions TEXT NOT NULL,
        events TEXT NOT NULL,
        verified_at INTEGER NOT NULL
    )",
    "CREATE TABLE IF NOT EXISTS orgs (
        login TEXT PRIMARY KEY,
        role TEXT,
        repo_count INTEGER,
        verified_at INTEGER NOT NULL
    )",
    "CREATE TABLE IF NOT EXISTS installations (
        app_slug TEXT NOT NULL,
        installation_id INTEGER NOT NULL,
        account_login TEXT NOT NULL,
        account_type TEXT NOT NULL,
        repository_selection TEXT NOT NULL,
        permissions TEXT NOT NULL,
        sync_run_id INTEGER NOT NULL,
        verified_at INTEGER NOT NULL,
        PRIMARY KEY (app_slug, account_login)
    )",
];

pub const START_RUN: &str =
    "INSERT INTO sync_runs (started_at, status) VALUES (?, 'running') RETURNING id";

pub const FINISH_RUN: &str =
    "UPDATE sync_runs SET finished_at = ?, status = ?, installations = ?, error = ? WHERE id = ?";

pub const UPSERT_APP: &str =
    "INSERT INTO apps (slug, app_id, name, owner, permissions, events, verified_at)
    VALUES (?, ?, ?, ?, ?, ?, ?)
    ON CONFLICT (slug) DO UPDATE SET
        app_id = excluded.app_id,
        name = excluded.name,
        owner = excluded.owner,
        permissions = excluded.permissions,
        events = excluded.events,
        verified_at = excluded.verified_at";

pub const UPSERT_ORG: &str =
    "INSERT INTO orgs (login, role, repo_count, verified_at) VALUES (?, NULL, NULL, ?)
    ON CONFLICT (login) DO UPDATE SET verified_at = excluded.verified_at";

pub const UPSERT_INSTALLATION: &str = "INSERT INTO installations
        (app_slug, installation_id, account_login, account_type, repository_selection,
         permissions, sync_run_id, verified_at)
    VALUES (?, ?, ?, ?, ?, ?, ?, ?)
    ON CONFLICT (app_slug, account_login) DO UPDATE SET
        account_login = excluded.account_login,
        installation_id = excluded.installation_id,
        account_type = excluded.account_type,
        repository_selection = excluded.repository_selection,
        permissions = excluded.permissions,
        sync_run_id = excluded.sync_run_id,
        verified_at = excluded.verified_at";

/// Installations of one App that this run did not see: removed on GitHub.
pub const PRUNE_STALE: &str = "DELETE FROM installations WHERE app_slug = ? AND sync_run_id <> ?";

pub fn migrate(db: &dyn Sql) -> Result<(), String> {
    for stmt in SCHEMA {
        db.execute(stmt, &[])?;
    }
    Ok(())
}
