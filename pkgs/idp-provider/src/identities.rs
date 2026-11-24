//! Runtime identity store (#13415): the link state of the account's
//! external identities, with an append-only event history.
//!
//! Slice 3 held the account's linked identities as static rows in
//! the provider config. Linking needs runtime state: a door login
//! can link an allowlisted identity, an operator can unlink one, and
//! every transition must be remembered. This store is that state:
//!
//! - `identities` — one row per (provider, subject): status
//!   (`active` | `unlinked`), link/unlink timestamps. Rows are never
//!   hard-deleted while the account lives; unlinking flips status.
//! - `identity_events` — append-only history (`seed` | `link` |
//!   `relink` | `unlink`), protected by the same no-UPDATE /
//!   no-DELETE triggers as the audit table, so the continuity of
//!   who-was-linked-when survives every later change.
//!
//! The provider config's identity list is the seed: at startup each
//! configured identity that the store has never seen is inserted as
//! active with a `seed` event. After that the store is authoritative.

use rusqlite::Connection;
use std::path::Path;
use std::sync::Mutex;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdentityRow {
    pub provider: String,
    pub subject: String,
    pub account_id: String,
    pub status: String, // "active" | "unlinked"
    pub linked_at: i64,
    pub unlinked_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdentityEvent {
    pub ts_unix: i64,
    pub event: String, // "seed" | "link" | "relink" | "unlink"
    pub provider: String,
    pub subject: String,
    pub account_id: String,
    pub actor: String, // "config-seed" | "door-login" | "operator-cli"
}

pub struct IdentityStore {
    conn: Mutex<Connection>,
}

impl IdentityStore {
    /// Open (or create) the identity database at `path`; `None` opens
    /// an in-memory database (tests).
    pub fn open(path: Option<&Path>) -> Result<Self, String> {
        let conn = match path {
            Some(p) => {
                if let Some(parent) = p.parent() {
                    if !parent.as_os_str().is_empty() {
                        std::fs::create_dir_all(parent).map_err(|e| {
                            format!(
                                "failed to create identity db directory {}: {e}",
                                parent.display()
                            )
                        })?;
                    }
                }
                Connection::open(p)
                    .map_err(|e| format!("failed to open identity db {}: {e}", p.display()))?
            }
            None => Connection::open_in_memory()
                .map_err(|e| format!("failed to open in-memory identity db: {e}"))?,
        };
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS identities (
                provider TEXT NOT NULL,
                subject TEXT NOT NULL,
                account_id TEXT NOT NULL,
                status TEXT NOT NULL,
                linked_at INTEGER NOT NULL,
                unlinked_at INTEGER,
                PRIMARY KEY (provider, subject)
            );
            CREATE TABLE IF NOT EXISTS identity_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ts_unix INTEGER NOT NULL,
                event TEXT NOT NULL,
                provider TEXT NOT NULL,
                subject TEXT NOT NULL,
                account_id TEXT NOT NULL,
                actor TEXT NOT NULL
            );
            CREATE TRIGGER IF NOT EXISTS identity_events_no_update
            BEFORE UPDATE ON identity_events BEGIN
                SELECT RAISE(ABORT, 'identity history is append-only');
            END;
            CREATE TRIGGER IF NOT EXISTS identity_events_no_delete
            BEFORE DELETE ON identity_events BEGIN
                SELECT RAISE(ABORT, 'identity history is append-only');
            END;",
        )
        .map_err(|e| format!("failed to initialise identity schema: {e}"))?;
        Ok(IdentityStore {
            conn: Mutex::new(conn),
        })
    }

    fn record_event(
        conn: &Connection,
        event: &str,
        provider: &str,
        subject: &str,
        account_id: &str,
        actor: &str,
    ) -> Result<(), String> {
        conn.execute(
            "INSERT INTO identity_events (ts_unix, event, provider, subject, account_id, actor)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                crate::registry::now_unix(),
                event,
                provider,
                subject,
                account_id,
                actor
            ],
        )
        .map_err(|e| format!("identity event insert failed: {e}"))?;
        Ok(())
    }

    /// Insert any configured identity the store has never seen, as
    /// active, with a `seed` event. Existing rows are untouched, so
    /// an operator unlink survives restarts and config re-reads.
    pub fn seed(
        &self,
        account_id: &str,
        identities: &[crate::registry::Identity],
    ) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "identity lock poisoned")?;
        for id in identities {
            let present: bool = conn
                .query_row(
                    "SELECT COUNT(*) FROM identities WHERE provider = ?1 AND subject = ?2",
                    rusqlite::params![id.provider, id.subject],
                    |row| row.get::<_, i64>(0),
                )
                .map(|n| n > 0)
                .map_err(|e| format!("identity seed check failed: {e}"))?;
            if !present {
                conn.execute(
                    "INSERT INTO identities (provider, subject, account_id, status, linked_at, unlinked_at)
                     VALUES (?1, ?2, ?3, 'active', ?4, NULL)",
                    rusqlite::params![
                        id.provider,
                        id.subject,
                        account_id,
                        crate::registry::now_unix()
                    ],
                )
                .map_err(|e| format!("identity seed insert failed: {e}"))?;
                Self::record_event(
                    &conn,
                    "seed",
                    &id.provider,
                    &id.subject,
                    account_id,
                    "config-seed",
                )?;
            }
        }
        Ok(())
    }

    /// Current link status of an identity: `Some("active")`,
    /// `Some("unlinked")`, or `None` if the store has never seen it.
    pub fn status(&self, provider: &str, subject: &str) -> Result<Option<String>, String> {
        let conn = self.conn.lock().map_err(|_| "identity lock poisoned")?;
        let mut stmt = conn
            .prepare("SELECT status FROM identities WHERE provider = ?1 AND subject = ?2")
            .map_err(|e| format!("identity status query failed: {e}"))?;
        let mut rows = stmt
            .query(rusqlite::params![provider, subject])
            .map_err(|e| format!("identity status query failed: {e}"))?;
        match rows
            .next()
            .map_err(|e| format!("identity row read failed: {e}"))?
        {
            Some(row) => Ok(Some(
                row.get(0)
                    .map_err(|e| format!("identity row read failed: {e}"))?,
            )),
            None => Ok(None),
        }
    }

    /// Is this identity currently linked (active)? Store errors fail
    /// closed (treated as not linked).
    pub fn is_active(&self, provider: &str, subject: &str) -> bool {
        matches!(self.status(provider, subject), Ok(Some(ref s)) if s == "active")
    }

    /// Link an identity to the account. Returns the event recorded:
    /// `"link"` (first time), `"relink"` (was unlinked), or `"none"`
    /// (already active — no event written).
    pub fn link(
        &self,
        provider: &str,
        subject: &str,
        account_id: &str,
        actor: &str,
    ) -> Result<&'static str, String> {
        let mut conn = self.conn.lock().map_err(|_| "identity lock poisoned")?;
        let tx = conn
            .transaction()
            .map_err(|e| format!("identity link transaction failed: {e}"))?;
        let existing: Option<String> = {
            let mut stmt = tx
                .prepare("SELECT status FROM identities WHERE provider = ?1 AND subject = ?2")
                .map_err(|e| format!("identity link query failed: {e}"))?;
            let mut rows = stmt
                .query(rusqlite::params![provider, subject])
                .map_err(|e| format!("identity link query failed: {e}"))?;
            match rows
                .next()
                .map_err(|e| format!("identity row read failed: {e}"))?
            {
                Some(row) => Some(
                    row.get(0)
                        .map_err(|e| format!("identity row read failed: {e}"))?,
                ),
                None => None,
            }
        };
        let now = crate::registry::now_unix();
        let event = match existing.as_deref() {
            Some("active") => "none",
            Some(_) => {
                tx.execute(
                    "UPDATE identities SET status = 'active', linked_at = ?1, unlinked_at = NULL
                     WHERE provider = ?2 AND subject = ?3",
                    rusqlite::params![now, provider, subject],
                )
                .map_err(|e| format!("identity relink failed: {e}"))?;
                "relink"
            }
            None => {
                tx.execute(
                    "INSERT INTO identities (provider, subject, account_id, status, linked_at, unlinked_at)
                     VALUES (?1, ?2, ?3, 'active', ?4, NULL)",
                    rusqlite::params![provider, subject, account_id, now],
                )
                .map_err(|e| format!("identity link insert failed: {e}"))?;
                "link"
            }
        };
        if event != "none" {
            Self::record_event(&tx, event, provider, subject, account_id, actor)?;
        }
        tx.commit()
            .map_err(|e| format!("identity link commit failed: {e}"))?;
        Ok(event)
    }

    /// Unlink an identity: the row is retained with status
    /// `unlinked` and the event joins the history. Returns false
    /// when the identity was not active (nothing changed).
    pub fn unlink(&self, provider: &str, subject: &str, actor: &str) -> Result<bool, String> {
        let mut conn = self.conn.lock().map_err(|_| "identity lock poisoned")?;
        let tx = conn
            .transaction()
            .map_err(|e| format!("identity unlink transaction failed: {e}"))?;
        let row: Option<(String, String)> = {
            let mut stmt = tx
                .prepare(
                    "SELECT status, account_id FROM identities WHERE provider = ?1 AND subject = ?2",
                )
                .map_err(|e| format!("identity unlink query failed: {e}"))?;
            let mut rows = stmt
                .query(rusqlite::params![provider, subject])
                .map_err(|e| format!("identity unlink query failed: {e}"))?;
            match rows
                .next()
                .map_err(|e| format!("identity row read failed: {e}"))?
            {
                Some(row) => Some((
                    row.get(0)
                        .map_err(|e| format!("identity row read failed: {e}"))?,
                    row.get(1)
                        .map_err(|e| format!("identity row read failed: {e}"))?,
                )),
                None => None,
            }
        };
        let changed = match row {
            Some((ref status, ref account_id)) if status == "active" => {
                tx.execute(
                    "UPDATE identities SET status = 'unlinked', unlinked_at = ?1
                     WHERE provider = ?2 AND subject = ?3",
                    rusqlite::params![crate::registry::now_unix(), provider, subject],
                )
                .map_err(|e| format!("identity unlink failed: {e}"))?;
                Self::record_event(&tx, "unlink", provider, subject, account_id, actor)?;
                true
            }
            _ => false,
        };
        tx.commit()
            .map_err(|e| format!("identity unlink commit failed: {e}"))?;
        Ok(changed)
    }

    /// All identity rows (operator read / tests).
    pub fn list(&self) -> Result<Vec<IdentityRow>, String> {
        let conn = self.conn.lock().map_err(|_| "identity lock poisoned")?;
        let mut stmt = conn
            .prepare(
                "SELECT provider, subject, account_id, status, linked_at, unlinked_at
                 FROM identities ORDER BY provider, subject",
            )
            .map_err(|e| format!("identity list query failed: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(IdentityRow {
                    provider: row.get(0)?,
                    subject: row.get(1)?,
                    account_id: row.get(2)?,
                    status: row.get(3)?,
                    linked_at: row.get(4)?,
                    unlinked_at: row.get(5)?,
                })
            })
            .map_err(|e| format!("identity list query failed: {e}"))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| format!("identity row read failed: {e}"))?);
        }
        Ok(out)
    }

    /// The full append-only event history, in order.
    pub fn events(&self) -> Result<Vec<IdentityEvent>, String> {
        let conn = self.conn.lock().map_err(|_| "identity lock poisoned")?;
        let mut stmt = conn
            .prepare(
                "SELECT ts_unix, event, provider, subject, account_id, actor
                 FROM identity_events ORDER BY id ASC",
            )
            .map_err(|e| format!("identity events query failed: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(IdentityEvent {
                    ts_unix: row.get(0)?,
                    event: row.get(1)?,
                    provider: row.get(2)?,
                    subject: row.get(3)?,
                    account_id: row.get(4)?,
                    actor: row.get(5)?,
                })
            })
            .map_err(|e| format!("identity events query failed: {e}"))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| format!("identity event read failed: {e}"))?);
        }
        Ok(out)
    }
}
