//! Append-only audit record (SQLite) for authorize/token attempts.
//!
//! Every authorize and token attempt — allowed and denied — is
//! appended here, per the Slice 3 plan on #13409. The table is
//! append-only by construction: this module exposes no update or
//! delete path, and SQLite triggers abort any UPDATE/DELETE issued
//! against the table by anything else.

use rusqlite::Connection;
use std::path::Path;
use std::sync::Mutex;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditEntry {
    pub ts_unix: i64,
    pub endpoint: String,
    pub outcome: String, // "allowed" | "denied" | "challenge"
    pub reason: String,
    pub client_id: String,
    pub identity: String,
}

pub struct AuditLog {
    conn: Mutex<Connection>,
}

impl AuditLog {
    /// Open (or create) the audit database at `path`; `None` opens an
    /// in-memory database (tests, public-only mode).
    pub fn open(path: Option<&Path>) -> Result<Self, String> {
        let conn = match path {
            Some(p) => {
                if let Some(parent) = p.parent() {
                    if !parent.as_os_str().is_empty() {
                        std::fs::create_dir_all(parent).map_err(|e| {
                            format!(
                                "failed to create audit db directory {}: {e}",
                                parent.display()
                            )
                        })?;
                    }
                }
                Connection::open(p)
                    .map_err(|e| format!("failed to open audit db {}: {e}", p.display()))?
            }
            None => Connection::open_in_memory()
                .map_err(|e| format!("failed to open in-memory audit db: {e}"))?,
        };
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS audit (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ts_unix INTEGER NOT NULL,
                endpoint TEXT NOT NULL,
                outcome TEXT NOT NULL,
                reason TEXT NOT NULL,
                client_id TEXT NOT NULL,
                identity TEXT NOT NULL
            );
            CREATE TRIGGER IF NOT EXISTS audit_no_update
            BEFORE UPDATE ON audit BEGIN
                SELECT RAISE(ABORT, 'audit is append-only');
            END;
            CREATE TRIGGER IF NOT EXISTS audit_no_delete
            BEFORE DELETE ON audit BEGIN
                SELECT RAISE(ABORT, 'audit is append-only');
            END;",
        )
        .map_err(|e| format!("failed to initialise audit schema: {e}"))?;
        Ok(AuditLog {
            conn: Mutex::new(conn),
        })
    }

    /// Append one attempt record. Audit failures are returned to the
    /// caller; the flow handlers treat a failed audit write on an
    /// otherwise-allowed attempt as fatal for that attempt (fail
    /// closed: no unaudited access).
    pub fn record(
        &self,
        endpoint: &str,
        outcome: &str,
        reason: &str,
        client_id: &str,
        identity: &str,
    ) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "audit lock poisoned")?;
        conn.execute(
            "INSERT INTO audit (ts_unix, endpoint, outcome, reason, client_id, identity)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                crate::registry::now_unix(),
                endpoint,
                outcome,
                reason,
                client_id,
                identity
            ],
        )
        .map_err(|e| format!("audit insert failed: {e}"))?;
        Ok(())
    }

    /// All entries in insertion order (proof / tests / operator read).
    pub fn entries(&self) -> Result<Vec<AuditEntry>, String> {
        let conn = self.conn.lock().map_err(|_| "audit lock poisoned")?;
        let mut stmt = conn
            .prepare(
                "SELECT ts_unix, endpoint, outcome, reason, client_id, identity
                 FROM audit ORDER BY id ASC",
            )
            .map_err(|e| format!("audit query failed: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(AuditEntry {
                    ts_unix: row.get(0)?,
                    endpoint: row.get(1)?,
                    outcome: row.get(2)?,
                    reason: row.get(3)?,
                    client_id: row.get(4)?,
                    identity: row.get(5)?,
                })
            })
            .map_err(|e| format!("audit query failed: {e}"))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| format!("audit row read failed: {e}"))?);
        }
        Ok(out)
    }
}
