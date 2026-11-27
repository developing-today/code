//! Runtime identity store (#13415): the link state of the account's
//! external identities, with an append-only event history.
//!
//! Slice 3 held the account's linked identities as static rows in
//! the provider config. Linking needs runtime state: a door login
//! can link an allowlisted identity, an operator can unlink one, and
//! every transition must be remembered. This store is that state:
//!
//! - `identities` — one row per (provider, subject): status
//!   (`active` | `pending` | `unlinked`), link/unlink timestamps,
//!   and pending-claim provenance. Rows are never
//!   hard-deleted while the account lives; unlinking flips status.
//! - `identity_events` — append-only history (`seed` | `link` |
//!   `relink` | `unlink` | `register` | `approve` | `reject` |
//!   `provision` | `merge`), protected by the same no-UPDATE /
//!   no-DELETE triggers as the audit table, so the continuity of
//!   who-was-linked-when survives every later change.
//!
//! The provider config's identity list is the seed: at startup each
//! configured identity that the store has never seen is inserted as
//! active with a `seed` event. After that the store is authoritative.
//!
//! Accounts (#15249): the store also holds the account rows tokens
//! are issued for. The config account (the owner's) is seeded at
//! startup; a never-seen allowlisted door identity that is not one
//! of the config account's declared identities ENROLLS into a fresh
//! account of its own whose only role is `read-only` — the design
//! #9086 §4.4 default (read-only on no tenant) — instead of being
//! linked to the owner's account. Role grants and re-linking to
//! another account are explicit operator actions afterwards.
//!
//! Account merge (#13975 slice 2, store side): one person can end up
//! with two accounts (auto-provisioning makes that a designed-for
//! outcome), so two accounts can join into the one the user chooses
//! as primary. The secondary row survives as a shell whose
//! `merged_into` names the primary; its identities repoint to the
//! primary with a `merge` event each; roles only the secondary held
//! need an explicit keep or drop decision, never a silent grant or
//! loss; and `account_events` is the append-only account history
//! the merge writes on both accounts. Stored history is never
//! rewritten: an old `account_id` (an audit row, an earlier token's
//! `sub`) reads through the pointer via `resolve_account_id`. The
//! two-session check that makes merge a two-key act belongs to the
//! flow that calls this, not to the store.
//!
//! Per-attribute repointing (#13975 slice 3, store side): after a
//! merge, each profile attribute (`PROFILE_ATTRIBUTES`) of a live
//! account can be sourced, one by one, from the account itself or
//! from any account merged into it. Values are never copied: every
//! row keeps its own, and `account_attribute_sources` records which
//! row each attribute is read from, so every change can be undone.
//! Each change is an `attribute-repoint` account event (and each
//! own-value edit an `attribute-set` one).

use rusqlite::{Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::Mutex;

/// The only role a freshly enrolled account holds (#15249): the
/// #1253 role set's least role, with no tenant grant attached.
pub const ENROLLED_ACCOUNT_ROLES: [&str; 1] = ["read-only"];

/// One account row: the subject tokens are issued for, and the
/// role set those tokens carry. Roles live on the account, never
/// on a door identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountRow {
    pub account_id: String,
    /// The effective profile attributes (#13975 slice 3): each is
    /// read from the account the attribute is sourced from, which
    /// is the account itself unless it was repointed after a merge.
    pub display_name: String,
    pub picture: Option<String>,
    pub summary: Option<String>,
    pub roles: Vec<String>,
}

/// The profile attributes an account carries and per-attribute
/// repointing covers (#13975 slice 3): the merge-critical display
/// name from the slice 1 audit, plus the picture and summary the
/// identity-linking policy names. A new attribute joins this list
/// and the `accounts` table together.
pub const PROFILE_ATTRIBUTES: [&str; 3] = ["display_name", "picture", "summary"];

/// One profile attribute of a live account as it reads now: the
/// account it is sourced from and that account's own value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttributeSource {
    pub attribute: String,
    pub source_account_id: String,
    pub value: Option<String>,
}

/// The account id an enrolling identity lands on, derived
/// deterministically from the identity key (`provider:subject`):
/// the same identity always enrolls into the same account, across
/// restarts and store rebuilds, and never into the owner's.
pub fn enrolled_account_id(provider: &str, subject: &str) -> String {
    let digest = Sha256::digest(format!("{provider}:{subject}").as_bytes());
    format!("acct-{}", &crate::b64url(&digest)[..22])
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdentityRow {
    pub provider: String,
    pub subject: String,
    pub account_id: String,
    pub status: String, // "active" | "pending" | "unlinked"
    pub linked_at: i64,
    pub unlinked_at: Option<i64>,
    pub claimed_via: Option<String>,
    pub claimed_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdentityEvent {
    pub ts_unix: i64,
    pub event: String, // seed | link | relink | unlink | register | approve | reject | provision | merge
    pub provider: String,
    pub subject: String,
    pub account_id: String,
    pub actor: String, // e.g. config-seed, door-login, operator-cli, established-session
}

/// One row of the append-only account history (#13975): the merge
/// writes `merge-absorb` on the primary and `merge-into` on the
/// secondary, plus one `role-keep` or `role-drop` per role only the
/// secondary held.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountEvent {
    pub ts_unix: i64,
    /// merge-absorb | merge-into | role-keep | role-drop |
    /// attribute-repoint | attribute-set
    pub event: String,
    pub account_id: String,
    pub other_account_id: String,
    pub detail: String,
    pub actor: String,
}

/// What merging `secondary` into `primary` would do, for the merge
/// flow to show before the user confirms (#13975 slice 2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergePreview {
    pub primary: String,
    pub secondary: String,
    /// Roles the secondary holds that the primary does not. Each
    /// needs an explicit keep or drop decision at merge time.
    pub secondary_only_roles: Vec<String>,
    /// Identity rows, `(provider, subject)`, that repoint to the
    /// primary, whatever their status.
    pub identities: Vec<(String, String)>,
}

/// What a merge did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeOutcome {
    /// The primary's role set after the merge: its own roles, then
    /// the kept secondary-only roles.
    pub primary_roles: Vec<String>,
    pub kept_roles: Vec<String>,
    pub dropped_roles: Vec<String>,
    /// Identity rows repointed from the secondary to the primary.
    pub repointed: Vec<(String, String)>,
}

/// Longest `merged_into` chain `resolve_account_id` follows. Each
/// merge needs a live primary, so a chain only grows when an
/// earlier primary is later merged itself; anything this long is a
/// broken store, and resolution fails closed on it.
const MAX_MERGE_CHAIN: usize = 32;

pub struct IdentityStore {
    conn: Mutex<Connection>,
    /// The config account (the owner's), recorded by `seed_account`
    /// at startup. Its row and roles are config-authoritative, so it
    /// can never be a merge secondary and never take kept roles.
    config_account: Mutex<Option<String>>,
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
                claimed_via TEXT,
                claimed_at INTEGER,
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
            END;
            CREATE TABLE IF NOT EXISTS accounts (
                account_id TEXT PRIMARY KEY,
                display_name TEXT NOT NULL,
                roles TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS account_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ts_unix INTEGER NOT NULL,
                event TEXT NOT NULL,
                account_id TEXT NOT NULL,
                other_account_id TEXT NOT NULL,
                detail TEXT NOT NULL,
                actor TEXT NOT NULL
            );
            CREATE TRIGGER IF NOT EXISTS account_events_no_update
            BEFORE UPDATE ON account_events BEGIN
                SELECT RAISE(ABORT, 'account history is append-only');
            END;
            CREATE TRIGGER IF NOT EXISTS account_events_no_delete
            BEFORE DELETE ON account_events BEGIN
                SELECT RAISE(ABORT, 'account history is append-only');
            END;
            CREATE TABLE IF NOT EXISTS account_attribute_sources (
                account_id TEXT NOT NULL,
                attribute TEXT NOT NULL,
                source_account_id TEXT NOT NULL,
                PRIMARY KEY (account_id, attribute)
            );",
        )
        .map_err(|e| format!("failed to initialise identity schema: {e}"))?;
        // Existing runtime databases predate pending claims. Keep this
        // migration additive so restarts preserve their rows.
        for (table, column, declaration) in [
            ("identities", "claimed_via", "TEXT"),
            ("identities", "claimed_at", "INTEGER"),
            // #13975: the merged-into pointer; NULL on a live account.
            ("accounts", "merged_into", "TEXT"),
            // #13975 slice 3: profile attributes beside display_name.
            ("accounts", "picture", "TEXT"),
            ("accounts", "summary", "TEXT"),
        ] {
            let exists: bool = conn
                .prepare(&format!("PRAGMA table_info({table})"))
                .and_then(|mut stmt| {
                    stmt.query_map([], |row| row.get::<_, String>("name"))
                        .and_then(|rows| {
                            rows.collect::<Result<Vec<_>, _>>()
                                .map(|names| names.iter().any(|name| name == column))
                        })
                })
                .map_err(|e| format!("failed to inspect identity schema: {e}"))?;
            if !exists {
                conn.execute(
                    &format!("ALTER TABLE {table} ADD COLUMN {column} {declaration}"),
                    [],
                )
                .map_err(|e| format!("failed to migrate identity schema: {e}"))?;
            }
        }
        Ok(IdentityStore {
            conn: Mutex::new(conn),
            config_account: Mutex::new(None),
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

    /// Seed the config account row (the owner's). Insert-or-ignore:
    /// the config is authoritative for this account at resolution
    /// time, so the row exists for completeness of the account
    /// table, and a restart never rewrites it.
    pub fn seed_account(&self, account: &crate::registry::Account) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "identity lock poisoned")?;
        let roles = serde_json::to_string(&account.roles)
            .map_err(|e| format!("account roles encode failed: {e}"))?;
        conn.execute(
            "INSERT OR IGNORE INTO accounts (account_id, display_name, roles)
             VALUES (?1, ?2, ?3)",
            rusqlite::params![account.account_id, account.display_name, roles],
        )
        .map_err(|e| format!("account seed insert failed: {e}"))?;
        drop(conn);
        *self
            .config_account
            .lock()
            .map_err(|_| "identity lock poisoned")? = Some(account.account_id.clone());
        Ok(())
    }

    /// One account row by id: the config account is NOT read here
    /// (callers resolve it from config first); this serves enrolled
    /// accounts, which exist only in the store. Profile attributes
    /// are the effective ones: each read from its source account
    /// (#13975 slice 3).
    pub fn account(&self, account_id: &str) -> Result<Option<AccountRow>, String> {
        let conn = self.conn.lock().map_err(|_| "identity lock poisoned")?;
        let roles_json: Option<String> = conn
            .query_row(
                "SELECT roles FROM accounts WHERE account_id = ?1",
                rusqlite::params![account_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| format!("account query failed: {e}"))?;
        let Some(roles_json) = roles_json else {
            return Ok(None);
        };
        let roles: Vec<String> = serde_json::from_str(&roles_json)
            .map_err(|e| format!("account roles decode failed: {e}"))?;
        let mut values = Vec::with_capacity(PROFILE_ATTRIBUTES.len());
        for attribute in PROFILE_ATTRIBUTES {
            values.push(Self::attribute_source_tx(&conn, account_id, attribute)?.value);
        }
        let mut values = values.into_iter();
        Ok(Some(AccountRow {
            account_id: account_id.to_string(),
            display_name: values.next().flatten().unwrap_or_default(),
            picture: values.next().flatten(),
            summary: values.next().flatten(),
            roles,
        }))
    }

    fn check_attribute(attribute: &str) -> Result<&'static str, String> {
        PROFILE_ATTRIBUTES
            .iter()
            .copied()
            .find(|known| *known == attribute)
            .ok_or_else(|| {
                format!(
                    "unknown profile attribute {attribute} (known: {})",
                    PROFILE_ATTRIBUTES.join(", ")
                )
            })
    }

    /// One account's own stored value of an attribute (its row, not
    /// its effective value). `attribute` must come from
    /// `PROFILE_ATTRIBUTES`: it names a column.
    fn own_value_tx(
        conn: &Connection,
        account_id: &str,
        attribute: &'static str,
    ) -> Result<Option<String>, String> {
        conn.query_row(
            &format!("SELECT {attribute} FROM accounts WHERE account_id = ?1"),
            rusqlite::params![account_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map_err(|e| format!("account {attribute} query failed: {e}"))?
        .ok_or_else(|| format!("unknown account {account_id}"))
    }

    fn attribute_source_tx(
        conn: &Connection,
        account_id: &str,
        attribute: &'static str,
    ) -> Result<AttributeSource, String> {
        let source: String = conn
            .query_row(
                "SELECT source_account_id FROM account_attribute_sources
                 WHERE account_id = ?1 AND attribute = ?2",
                rusqlite::params![account_id, attribute],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| format!("attribute source query failed: {e}"))?
            .unwrap_or_else(|| account_id.to_string());
        let value = Self::own_value_tx(conn, &source, attribute)?;
        Ok(AttributeSource {
            attribute: attribute.to_string(),
            source_account_id: source,
            value,
        })
    }

    /// Where each profile attribute of `account_id` reads from now,
    /// in `PROFILE_ATTRIBUTES` order (#13975 slice 3): the surface
    /// the repointing flow shows before a change.
    pub fn attribute_sources(&self, account_id: &str) -> Result<Vec<AttributeSource>, String> {
        let conn = self.conn.lock().map_err(|_| "identity lock poisoned")?;
        Self::merged_into_tx(&conn, account_id)?;
        PROFILE_ATTRIBUTES
            .iter()
            .map(|attribute| Self::attribute_source_tx(&conn, account_id, attribute))
            .collect()
    }

    /// Accounts an attribute of `account_id` may be sourced from:
    /// itself, and every account whose `merged_into` chain ends at
    /// it, in id order.
    fn attribute_candidates_tx(conn: &Connection, account_id: &str) -> Result<Vec<String>, String> {
        let mut stmt = conn
            .prepare(
                "SELECT account_id FROM accounts WHERE merged_into IS NOT NULL
                 ORDER BY account_id",
            )
            .map_err(|e| format!("merged account query failed: {e}"))?;
        let shells = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| format!("merged account query failed: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("merged account read failed: {e}"))?;
        let mut out = vec![account_id.to_string()];
        for shell in shells {
            let mut current = shell.clone();
            for _ in 0..MAX_MERGE_CHAIN {
                match Self::pointer_tx(conn, &current)? {
                    Some(Some(next)) => current = next,
                    Some(None) | None => break,
                }
            }
            if current == account_id {
                out.push(shell);
            }
        }
        Ok(out)
    }

    /// Refuse a profile change on an account it cannot apply to:
    /// unknown, merged away, or the config account's display name,
    /// which tokens read from config.
    fn check_profile_target_tx(
        &self,
        conn: &Connection,
        account_id: &str,
        attribute: &str,
    ) -> Result<(), String> {
        if let Some(into) = Self::merged_into_tx(conn, account_id)? {
            return Err(format!(
                "account {account_id} is merged into {into}; change {into}'s profile instead"
            ));
        }
        let config = self
            .config_account
            .lock()
            .map_err(|_| "identity lock poisoned")?
            .clone();
        if attribute == "display_name" && config.as_deref() == Some(account_id) {
            return Err(format!(
                "account {account_id} is the config account; its display name is set in config"
            ));
        }
        Ok(())
    }

    /// Source one profile attribute of a live account from
    /// `source_account_id` (#13975 slice 3): the account itself, or
    /// any account merged into it. Values are not copied, so a later
    /// repoint back restores the earlier value exactly. Returns
    /// whether anything changed; a real change writes one
    /// `attribute-repoint` event on the account, naming the source.
    /// The flow that calls this owns the session check (the account
    /// session from #13972), as with merge.
    pub fn repoint_attribute(
        &self,
        account_id: &str,
        attribute: &str,
        source_account_id: &str,
        actor: &str,
    ) -> Result<bool, String> {
        let attribute = Self::check_attribute(attribute)?;
        let mut conn = self.conn.lock().map_err(|_| "identity lock poisoned")?;
        let tx = conn
            .transaction()
            .map_err(|e| format!("attribute repoint transaction failed: {e}"))?;
        self.check_profile_target_tx(&tx, account_id, attribute)?;
        if !Self::attribute_candidates_tx(&tx, account_id)?
            .iter()
            .any(|candidate| candidate == source_account_id)
        {
            return Err(format!(
                "account {source_account_id} is neither {account_id} nor merged into it"
            ));
        }
        let before = Self::attribute_source_tx(&tx, account_id, attribute)?;
        if before.source_account_id == source_account_id {
            return Ok(false);
        }
        if source_account_id == account_id {
            tx.execute(
                "DELETE FROM account_attribute_sources WHERE account_id = ?1 AND attribute = ?2",
                rusqlite::params![account_id, attribute],
            )
        } else {
            tx.execute(
                "INSERT INTO account_attribute_sources (account_id, attribute, source_account_id)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT (account_id, attribute)
                 DO UPDATE SET source_account_id = excluded.source_account_id",
                rusqlite::params![account_id, attribute, source_account_id],
            )
        }
        .map_err(|e| format!("attribute source update failed: {e}"))?;
        let detail = serde_json::json!({
            "attribute": attribute,
            "from": before.source_account_id,
            "to": source_account_id,
        })
        .to_string();
        Self::record_account_event(
            &tx,
            "attribute-repoint",
            account_id,
            source_account_id,
            &detail,
            actor,
        )?;
        tx.commit()
            .map_err(|e| format!("attribute repoint commit failed: {e}"))?;
        Ok(true)
    }

    /// Set a live account's own value of one profile attribute
    /// (`None` clears it). This edits the account's row only: if the
    /// attribute is repointed elsewhere, the effective value does not
    /// change until it is repointed back. Returns whether anything
    /// changed; a real change writes one `attribute-set` event.
    pub fn set_attribute(
        &self,
        account_id: &str,
        attribute: &str,
        value: Option<&str>,
        actor: &str,
    ) -> Result<bool, String> {
        let attribute = Self::check_attribute(attribute)?;
        if attribute == "display_name" && value.is_none() {
            return Err("display_name cannot be cleared".to_string());
        }
        let mut conn = self.conn.lock().map_err(|_| "identity lock poisoned")?;
        let tx = conn
            .transaction()
            .map_err(|e| format!("attribute set transaction failed: {e}"))?;
        self.check_profile_target_tx(&tx, account_id, attribute)?;
        if Self::own_value_tx(&tx, account_id, attribute)?.as_deref() == value {
            return Ok(false);
        }
        tx.execute(
            &format!("UPDATE accounts SET {attribute} = ?1 WHERE account_id = ?2"),
            rusqlite::params![value, account_id],
        )
        .map_err(|e| format!("account {attribute} update failed: {e}"))?;
        let detail = serde_json::json!({ "attribute": attribute }).to_string();
        Self::record_account_event(&tx, "attribute-set", account_id, account_id, &detail, actor)?;
        tx.commit()
            .map_err(|e| format!("attribute set commit failed: {e}"))?;
        Ok(true)
    }

    /// The account an identity row currently points at, whatever
    /// its status. `None` when the store has never seen it.
    pub fn account_id_for(&self, provider: &str, subject: &str) -> Result<Option<String>, String> {
        let conn = self.conn.lock().map_err(|_| "identity lock poisoned")?;
        let mut stmt = conn
            .prepare("SELECT account_id FROM identities WHERE provider = ?1 AND subject = ?2")
            .map_err(|e| format!("identity account query failed: {e}"))?;
        let mut rows = stmt
            .query(rusqlite::params![provider, subject])
            .map_err(|e| format!("identity account query failed: {e}"))?;
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

    /// Enroll a never-seen allowlisted identity (#15249): create
    /// its own account holding only `read-only` (design #9086
    /// §4.4's default — read-only on no tenant) and link the
    /// identity to it, in one transaction, with the `link` event
    /// in the identity history naming the NEW account. Returns the
    /// new account id and the recorded event (`"link"`).
    ///
    /// Callers must have already established the identity is
    /// allowlisted and unseen; enrolling an identity the store
    /// knows is an error, never a silent re-link.
    pub fn enroll(
        &self,
        provider: &str,
        subject: &str,
        actor: &str,
    ) -> Result<(String, &'static str), String> {
        let mut conn = self.conn.lock().map_err(|_| "identity lock poisoned")?;
        let tx = conn
            .transaction()
            .map_err(|e| format!("identity enroll transaction failed: {e}"))?;
        let account_id = enrolled_account_id(provider, subject);
        let roles = serde_json::to_string(&ENROLLED_ACCOUNT_ROLES)
            .map_err(|e| format!("account roles encode failed: {e}"))?;
        tx.execute(
            "INSERT OR IGNORE INTO accounts (account_id, display_name, roles)
             VALUES (?1, ?2, ?3)",
            rusqlite::params![account_id, format!("{provider}:{subject}"), roles],
        )
        .map_err(|e| format!("enrolled account insert failed: {e}"))?;
        let now = crate::registry::now_unix();
        tx.execute(
            "INSERT INTO identities (provider, subject, account_id, status, linked_at, unlinked_at)
             VALUES (?1, ?2, ?3, 'active', ?4, NULL)",
            rusqlite::params![provider, subject, account_id, now],
        )
        .map_err(|e| format!("enrolled identity insert failed: {e}"))?;
        Self::record_event(&tx, "link", provider, subject, &account_id, actor)?;
        tx.commit()
            .map_err(|e| format!("identity enroll commit failed: {e}"))?;
        Ok((account_id, "link"))
    }

    /// Current link status of an identity: `Some("active")`,
    /// `Some("pending")`, `Some("unlinked")`, or `None` if unseen.
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

    /// Register a previously unseen identity as pending. Claim provenance
    /// and its timestamp remain on the row after approval or rejection.
    pub fn register_pending(
        &self,
        provider: &str,
        subject: &str,
        account_id: &str,
        claimed_via: &str,
    ) -> Result<bool, String> {
        let mut conn = self.conn.lock().map_err(|_| "identity lock poisoned")?;
        let tx = conn
            .transaction()
            .map_err(|e| format!("identity registration transaction failed: {e}"))?;
        let now = crate::registry::now_unix();
        let inserted = tx
            .execute(
                "INSERT OR IGNORE INTO identities
                 (provider, subject, account_id, status, linked_at, unlinked_at, claimed_via, claimed_at)
                 VALUES (?1, ?2, ?3, 'pending', ?4, NULL, ?5, ?4)",
                rusqlite::params![provider, subject, account_id, now, claimed_via],
            )
            .map_err(|e| format!("pending identity registration failed: {e}"))?;
        if inserted == 1 {
            Self::record_event(&tx, "register", provider, subject, account_id, claimed_via)?;
        }
        tx.commit()
            .map_err(|e| format!("identity registration commit failed: {e}"))?;
        Ok(inserted == 1)
    }

    pub fn approve_pending(
        &self,
        provider: &str,
        subject: &str,
        actor: &str,
    ) -> Result<bool, String> {
        self.transition_pending(provider, subject, "active", "approve", actor)
    }

    pub fn reject_pending(
        &self,
        provider: &str,
        subject: &str,
        actor: &str,
    ) -> Result<bool, String> {
        self.transition_pending(provider, subject, "unlinked", "reject", actor)
    }

    fn transition_pending(
        &self,
        provider: &str,
        subject: &str,
        status: &str,
        event: &str,
        actor: &str,
    ) -> Result<bool, String> {
        let mut conn = self.conn.lock().map_err(|_| "identity lock poisoned")?;
        let tx = conn
            .transaction()
            .map_err(|e| format!("identity transition transaction failed: {e}"))?;
        let account_id: Option<String> = tx
            .query_row(
                "SELECT account_id FROM identities WHERE provider = ?1 AND subject = ?2 AND status = 'pending'",
                rusqlite::params![provider, subject],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| format!("pending identity lookup failed: {e}"))?;
        let Some(account_id) = account_id else {
            return Ok(false);
        };
        let now = crate::registry::now_unix();
        tx.execute(
            "UPDATE identities SET status = ?1, linked_at = ?2,
             unlinked_at = CASE WHEN ?1 = 'unlinked' THEN ?2 ELSE NULL END
             WHERE provider = ?3 AND subject = ?4 AND status = 'pending'",
            rusqlite::params![status, now, provider, subject],
        )
        .map_err(|e| format!("pending identity transition failed: {e}"))?;
        Self::record_event(&tx, event, provider, subject, &account_id, actor)?;
        tx.commit()
            .map_err(|e| format!("identity transition commit failed: {e}"))?;
        Ok(true)
    }

    /// Record the identity established as part of provisioning. Account
    /// creation itself is deliberately outside this store.
    pub fn record_provision(
        &self,
        provider: &str,
        subject: &str,
        account_id: &str,
        actor: &str,
    ) -> Result<(), String> {
        let mut conn = self.conn.lock().map_err(|_| "identity lock poisoned")?;
        let tx = conn
            .transaction()
            .map_err(|e| format!("identity provision transaction failed: {e}"))?;
        let now = crate::registry::now_unix();
        tx.execute(
            "INSERT INTO identities
             (provider, subject, account_id, status, linked_at, unlinked_at, claimed_via, claimed_at)
             VALUES (?1, ?2, ?3, 'active', ?4, NULL, NULL, NULL)",
            rusqlite::params![provider, subject, account_id, now],
        )
        .map_err(|e| format!("provisioned identity insert failed: {e}"))?;
        Self::record_event(&tx, "provision", provider, subject, account_id, actor)?;
        tx.commit()
            .map_err(|e| format!("identity provision commit failed: {e}"))?;
        Ok(())
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
                // A relink names its account: this is the explicit
                // operator grant that moves an identity (e.g. one
                // enrolled into its own read-only account, #15249)
                // onto the named account, so the row must move too,
                // not just the event.
                tx.execute(
                    "UPDATE identities SET status = 'active', linked_at = ?1, unlinked_at = NULL,
                     account_id = ?4
                     WHERE provider = ?2 AND subject = ?3",
                    rusqlite::params![now, provider, subject, account_id],
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
                "SELECT provider, subject, account_id, status, linked_at, unlinked_at, claimed_via, claimed_at
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
                    claimed_via: row.get(6)?,
                    claimed_at: row.get(7)?,
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

    /// One account's `merged_into` pointer: `None` when the store
    /// has no row for the id, `Some(None)` for a live account,
    /// `Some(Some(primary))` for a merged shell.
    fn pointer_tx(conn: &Connection, account_id: &str) -> Result<Option<Option<String>>, String> {
        conn.query_row(
            "SELECT merged_into FROM accounts WHERE account_id = ?1",
            rusqlite::params![account_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map_err(|e| format!("account merge pointer query failed: {e}"))
    }

    /// As `pointer_tx`, with an unknown account an error: both sides
    /// of a merge must be rows in the store.
    fn merged_into_tx(conn: &Connection, account_id: &str) -> Result<Option<String>, String> {
        Self::pointer_tx(conn, account_id)?.ok_or_else(|| format!("unknown account {account_id}"))
    }

    fn roles_tx(conn: &Connection, account_id: &str) -> Result<Vec<String>, String> {
        let roles_json: String = conn
            .query_row(
                "SELECT roles FROM accounts WHERE account_id = ?1",
                rusqlite::params![account_id],
                |row| row.get(0),
            )
            .map_err(|e| format!("account roles query failed: {e}"))?;
        serde_json::from_str(&roles_json).map_err(|e| format!("account roles decode failed: {e}"))
    }

    /// The account this id reads as now (#13975): itself when live
    /// or unknown to the store (the config account before seeding),
    /// otherwise the end of its `merged_into` chain. Stored rows are
    /// never rewritten by a merge; this is how an old `account_id`
    /// in history, or an earlier token's `sub`, keeps resolving.
    /// A cycle or an over-long chain is an error (fail closed).
    pub fn resolve_account_id(&self, account_id: &str) -> Result<String, String> {
        let conn = self.conn.lock().map_err(|_| "identity lock poisoned")?;
        let mut current = account_id.to_string();
        for _ in 0..MAX_MERGE_CHAIN {
            match Self::pointer_tx(&conn, &current)? {
                Some(Some(next)) => current = next,
                Some(None) | None => return Ok(current),
            }
        }
        Err(format!(
            "account {account_id}: merged_into chain longer than {MAX_MERGE_CHAIN}"
        ))
    }

    /// The `merged_into` pointer of one account: `Some(primary)` for
    /// a merged shell, `None` for a live account or an unknown id.
    pub fn merged_into(&self, account_id: &str) -> Result<Option<String>, String> {
        let conn = self.conn.lock().map_err(|_| "identity lock poisoned")?;
        Ok(Self::pointer_tx(&conn, account_id)?.flatten())
    }

    fn preview_tx(
        &self,
        conn: &Connection,
        primary: &str,
        secondary: &str,
    ) -> Result<MergePreview, String> {
        if primary == secondary {
            return Err("an account cannot be merged into itself".to_string());
        }
        let config = self
            .config_account
            .lock()
            .map_err(|_| "identity lock poisoned")?
            .clone();
        if config.as_deref() == Some(secondary) {
            return Err(format!(
                "account {secondary} is the config account and cannot be merged away"
            ));
        }
        for (side, id) in [("primary", primary), ("secondary", secondary)] {
            if let Some(into) = Self::merged_into_tx(conn, id)? {
                return Err(format!("{side} account {id} is already merged into {into}"));
            }
        }
        let primary_roles = Self::roles_tx(conn, primary)?;
        let secondary_only_roles: Vec<String> = Self::roles_tx(conn, secondary)?
            .into_iter()
            .filter(|role| !primary_roles.contains(role))
            .collect();
        let mut stmt = conn
            .prepare(
                "SELECT provider, subject FROM identities WHERE account_id = ?1
                 ORDER BY provider, subject",
            )
            .map_err(|e| format!("merge identity query failed: {e}"))?;
        let identities = stmt
            .query_map(rusqlite::params![secondary], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("merge identity query failed: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("merge identity read failed: {e}"))?;
        Ok(MergePreview {
            primary: primary.to_string(),
            secondary: secondary.to_string(),
            secondary_only_roles,
            identities,
        })
    }

    /// What merging `secondary` into `primary` would do, without
    /// doing it: the merge flow shows the secondary-only roles for
    /// an explicit keep or drop before the user confirms.
    pub fn merge_preview(&self, primary: &str, secondary: &str) -> Result<MergePreview, String> {
        let conn = self.conn.lock().map_err(|_| "identity lock poisoned")?;
        self.preview_tx(&conn, primary, secondary)
    }

    /// Merge `secondary` into `primary` (#13975 slice 2), in one
    /// transaction:
    ///
    /// - every role only the secondary held must appear in exactly
    ///   one of `keep_roles` and `drop_roles` (never silently granted,
    ///   never silently dropped); anything else is refused with
    ///   nothing written;
    /// - every identity row of the secondary, whatever its status,
    ///   repoints to the primary with a `merge` identity event;
    /// - the primary's roles become its own plus the kept ones;
    /// - the secondary's row is kept as a shell with `merged_into`
    ///   set, its profile attributes untouched (primary wins for
    ///   display; `repoint_attribute` can then source any attribute
    ///   from the shell, one by one);
    /// - both accounts' histories record the merge and each role
    ///   decision.
    ///
    /// The config account can be a primary but never a secondary,
    /// and as a primary it takes no kept roles: its roles come from
    /// config. The caller (the merge flow) is responsible for the
    /// two-session check; the store only enforces the data rules.
    pub fn merge(
        &self,
        primary: &str,
        secondary: &str,
        keep_roles: &[String],
        drop_roles: &[String],
        actor: &str,
    ) -> Result<MergeOutcome, String> {
        let mut conn = self.conn.lock().map_err(|_| "identity lock poisoned")?;
        let tx = conn
            .transaction()
            .map_err(|e| format!("account merge transaction failed: {e}"))?;
        let preview = self.preview_tx(&tx, primary, secondary)?;
        let decided: Vec<&String> = keep_roles.iter().chain(drop_roles).collect();
        if let Some(both) = keep_roles.iter().find(|role| drop_roles.contains(*role)) {
            return Err(format!("role {both} is both kept and dropped"));
        }
        if let Some(stray) = decided
            .iter()
            .copied()
            .find(|role| !preview.secondary_only_roles.contains(*role))
        {
            return Err(format!(
                "role {stray} is not a secondary-only role of {secondary}"
            ));
        }
        let undecided: Vec<&String> = preview
            .secondary_only_roles
            .iter()
            .filter(|role| !decided.contains(role))
            .collect();
        if !undecided.is_empty() {
            let names: Vec<&str> = undecided.iter().map(|role| role.as_str()).collect();
            return Err(format!(
                "secondary-only roles need a keep or drop decision: {}",
                names.join(", ")
            ));
        }
        let config = self
            .config_account
            .lock()
            .map_err(|_| "identity lock poisoned")?
            .clone();
        if !keep_roles.is_empty() && config.as_deref() == Some(primary) {
            return Err(format!(
                "account {primary} is the config account; its roles are set in config"
            ));
        }

        let mut primary_roles = Self::roles_tx(&tx, primary)?;
        let kept: Vec<String> = preview
            .secondary_only_roles
            .iter()
            .filter(|role| keep_roles.contains(*role))
            .cloned()
            .collect();
        let dropped: Vec<String> = preview
            .secondary_only_roles
            .iter()
            .filter(|role| drop_roles.contains(*role))
            .cloned()
            .collect();
        primary_roles.extend(kept.iter().cloned());
        let roles_json = serde_json::to_string(&primary_roles)
            .map_err(|e| format!("account roles encode failed: {e}"))?;
        tx.execute(
            "UPDATE accounts SET roles = ?1 WHERE account_id = ?2",
            rusqlite::params![roles_json, primary],
        )
        .map_err(|e| format!("primary roles update failed: {e}"))?;
        for (provider, subject) in &preview.identities {
            tx.execute(
                "UPDATE identities SET account_id = ?1 WHERE provider = ?2 AND subject = ?3",
                rusqlite::params![primary, provider, subject],
            )
            .map_err(|e| format!("identity repoint failed: {e}"))?;
            Self::record_event(&tx, "merge", provider, subject, primary, actor)?;
        }
        tx.execute(
            "UPDATE accounts SET merged_into = ?1 WHERE account_id = ?2",
            rusqlite::params![primary, secondary],
        )
        .map_err(|e| format!("merge pointer update failed: {e}"))?;
        let detail = serde_json::json!({
            "kept_roles": kept,
            "dropped_roles": dropped,
            "identities": preview
                .identities
                .iter()
                .map(|(provider, subject)| format!("{provider}:{subject}"))
                .collect::<Vec<_>>(),
        })
        .to_string();
        Self::record_account_event(&tx, "merge-absorb", primary, secondary, &detail, actor)?;
        Self::record_account_event(&tx, "merge-into", secondary, primary, &detail, actor)?;
        for role in &kept {
            Self::record_account_event(&tx, "role-keep", primary, secondary, role, actor)?;
        }
        for role in &dropped {
            Self::record_account_event(&tx, "role-drop", primary, secondary, role, actor)?;
        }
        tx.commit()
            .map_err(|e| format!("account merge commit failed: {e}"))?;
        Ok(MergeOutcome {
            primary_roles,
            kept_roles: kept,
            dropped_roles: dropped,
            repointed: preview.identities,
        })
    }

    fn record_account_event(
        conn: &Connection,
        event: &str,
        account_id: &str,
        other_account_id: &str,
        detail: &str,
        actor: &str,
    ) -> Result<(), String> {
        conn.execute(
            "INSERT INTO account_events
             (ts_unix, event, account_id, other_account_id, detail, actor)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                crate::registry::now_unix(),
                event,
                account_id,
                other_account_id,
                detail,
                actor
            ],
        )
        .map_err(|e| format!("account event insert failed: {e}"))?;
        Ok(())
    }

    /// The full append-only account history, in order.
    pub fn account_events(&self) -> Result<Vec<AccountEvent>, String> {
        let conn = self.conn.lock().map_err(|_| "identity lock poisoned")?;
        let mut stmt = conn
            .prepare(
                "SELECT ts_unix, event, account_id, other_account_id, detail, actor
                 FROM account_events ORDER BY id ASC",
            )
            .map_err(|e| format!("account events query failed: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(AccountEvent {
                    ts_unix: row.get(0)?,
                    event: row.get(1)?,
                    account_id: row.get(2)?,
                    other_account_id: row.get(3)?,
                    detail: row.get(4)?,
                    actor: row.get(5)?,
                })
            })
            .map_err(|e| format!("account events query failed: {e}"))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| format!("account event read failed: {e}"))?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with(accounts: &[(&str, &[&str])]) -> IdentityStore {
        let store = IdentityStore::open(None).unwrap();
        {
            let conn = store.conn.lock().unwrap();
            for (id, roles) in accounts {
                conn.execute(
                    "INSERT INTO accounts (account_id, display_name, roles) VALUES (?1, ?1, ?2)",
                    rusqlite::params![id, serde_json::to_string(roles).unwrap()],
                )
                .unwrap();
            }
        }
        store
    }

    fn strings(roles: &[&str]) -> Vec<String> {
        roles.iter().map(|r| r.to_string()).collect()
    }

    #[test]
    fn preview_lists_secondary_only_roles_and_every_identity_row() {
        let store = store_with(&[("a", &["read-only"]), ("b", &["read-only", "agent"])]);
        store.link("google", "b-sub", "b", "test").unwrap();
        store.link("github", "b-gh", "b", "test").unwrap();
        store.unlink("github", "b-gh", "test").unwrap();
        let preview = store.merge_preview("a", "b").unwrap();
        assert_eq!(preview.secondary_only_roles, strings(&["agent"]));
        assert_eq!(
            preview.identities,
            vec![
                ("github".to_string(), "b-gh".to_string()),
                ("google".to_string(), "b-sub".to_string()),
            ],
            "unlinked rows repoint too, so nothing is lost"
        );
    }

    #[test]
    fn merge_refuses_undecided_stray_and_conflicting_roles_and_writes_nothing() {
        let store = store_with(&[
            ("a", &["read-only"]),
            ("b", &["read-only", "agent", "orchestrator"]),
        ]);
        store.link("google", "b-sub", "b", "test").unwrap();
        let before = store.events().unwrap().len();

        let undecided = store
            .merge("a", "b", &strings(&["agent"]), &[], "test")
            .unwrap_err();
        assert!(undecided.contains("orchestrator"), "{undecided}");
        let stray = store
            .merge(
                "a",
                "b",
                &strings(&["agent", "owner"]),
                &strings(&["orchestrator"]),
                "test",
            )
            .unwrap_err();
        assert!(stray.contains("owner"), "{stray}");
        let both = store
            .merge(
                "a",
                "b",
                &strings(&["agent", "orchestrator"]),
                &strings(&["agent"]),
                "test",
            )
            .unwrap_err();
        assert!(both.contains("both kept and dropped"), "{both}");

        assert_eq!(store.merged_into("b").unwrap(), None);
        assert_eq!(
            store.account_id_for("google", "b-sub").unwrap().unwrap(),
            "b"
        );
        assert_eq!(store.events().unwrap().len(), before);
        assert!(store.account_events().unwrap().is_empty());
        assert_eq!(
            store.account("a").unwrap().unwrap().roles,
            strings(&["read-only"])
        );
    }

    #[test]
    fn merge_repoints_identities_decides_roles_and_records_both_histories() {
        let store = store_with(&[
            ("a", &["read-only"]),
            ("b", &["read-only", "agent", "orchestrator"]),
        ]);
        store.link("google", "a-sub", "a", "test").unwrap();
        store.link("google", "b-sub", "b", "test").unwrap();
        store.link("github", "b-gh", "b", "test").unwrap();
        store.unlink("github", "b-gh", "test").unwrap();
        let history_before = store.events().unwrap();

        let outcome = store
            .merge(
                "a",
                "b",
                &strings(&["agent"]),
                &strings(&["orchestrator"]),
                "established-sessions",
            )
            .unwrap();
        assert_eq!(outcome.primary_roles, strings(&["read-only", "agent"]));
        assert_eq!(outcome.kept_roles, strings(&["agent"]));
        assert_eq!(outcome.dropped_roles, strings(&["orchestrator"]));
        assert_eq!(outcome.repointed.len(), 2);

        // Sign-in through a former-secondary identity reaches the
        // primary; an unlinked row moved too but stays unlinked.
        assert_eq!(
            store.account_id_for("google", "b-sub").unwrap().unwrap(),
            "a"
        );
        assert_eq!(
            store.account_id_for("github", "b-gh").unwrap().unwrap(),
            "a"
        );
        assert_eq!(store.status("github", "b-gh").unwrap().unwrap(), "unlinked");
        assert_eq!(
            store.account("a").unwrap().unwrap().roles,
            outcome.primary_roles
        );

        // The secondary survives as a shell; its id reads as the primary.
        let shell = store.account("b").unwrap().unwrap();
        assert_eq!(shell.display_name, "b");
        assert_eq!(store.merged_into("b").unwrap().as_deref(), Some("a"));
        assert_eq!(store.resolve_account_id("b").unwrap(), "a");
        assert_eq!(store.resolve_account_id("a").unwrap(), "a");
        assert_eq!(
            store.resolve_account_id("never-seen").unwrap(),
            "never-seen"
        );

        // History is appended, never rewritten: the earlier rows still
        // name the secondary, and one merge event per repointed row.
        let history = store.events().unwrap();
        assert_eq!(&history[..history_before.len()], &history_before[..]);
        let merges: Vec<_> = history[history_before.len()..].iter().collect();
        assert_eq!(merges.len(), 2);
        assert!(merges.iter().all(|e| e.event == "merge"
            && e.account_id == "a"
            && e.actor == "established-sessions"));

        let account_history = store.account_events().unwrap();
        let kinds: Vec<(&str, &str, &str)> = account_history
            .iter()
            .map(|e| {
                (
                    e.event.as_str(),
                    e.account_id.as_str(),
                    e.other_account_id.as_str(),
                )
            })
            .collect();
        assert_eq!(
            kinds,
            [
                ("merge-absorb", "a", "b"),
                ("merge-into", "b", "a"),
                ("role-keep", "a", "b"),
                ("role-drop", "a", "b"),
            ]
        );
        assert_eq!(account_history[2].detail, "agent");
        assert_eq!(account_history[3].detail, "orchestrator");
    }

    #[test]
    fn merge_refuses_self_merged_and_config_secondary() {
        let store = IdentityStore::open(None).unwrap();
        store
            .seed_account(&crate::registry::Account {
                account_id: "owner".to_string(),
                display_name: "Owner".to_string(),
                email: None,
                roles: strings(&["owner", "read-only"]),
                identities: vec![],
                local_credential_hash: String::new(),
            })
            .unwrap();
        let (a, _) = store.enroll("google", "a-sub", "door-login").unwrap();
        let (b, _) = store.enroll("google", "b-sub", "door-login").unwrap();
        let (c, _) = store.enroll("google", "c-sub", "door-login").unwrap();

        assert!(store.merge(&a, &a, &[], &[], "t").is_err());
        let config = store.merge(&a, "owner", &[], &[], "t").unwrap_err();
        assert!(config.contains("config account"), "{config}");
        assert!(store.merge_preview("nobody", &a).is_err());

        // The config account can absorb an enrolled account; nothing
        // is secondary-only there, so no decision is needed.
        store.merge("owner", &a, &[], &[], "t").unwrap();
        assert_eq!(
            store.account_id_for("google", "a-sub").unwrap().unwrap(),
            "owner"
        );
        let again = store.merge("owner", &a, &[], &[], "t").unwrap_err();
        assert!(again.contains("already merged"), "{again}");
        let from_shell = store.merge(&a, &b, &[], &[], "t").unwrap_err();
        assert!(from_shell.contains("already merged"), "{from_shell}");

        // A chain resolves to its end: c absorbs b, then the owner
        // absorbs c, and b's id reads as the owner's.
        store.merge(&c, &b, &[], &[], "t").unwrap();
        store.merge("owner", &c, &[], &[], "t").unwrap();
        assert_eq!(store.resolve_account_id(&b).unwrap(), "owner");
    }

    #[test]
    fn kept_roles_never_land_on_the_config_account() {
        let store = store_with(&[("x", &["read-only", "agent"])]);
        store
            .seed_account(&crate::registry::Account {
                account_id: "owner".to_string(),
                display_name: "Owner".to_string(),
                email: None,
                roles: strings(&["owner"]),
                identities: vec![],
                local_credential_hash: String::new(),
            })
            .unwrap();
        let err = store
            .merge("owner", "x", &strings(&["read-only", "agent"]), &[], "t")
            .unwrap_err();
        assert!(err.contains("set in config"), "{err}");
        store
            .merge("owner", "x", &[], &strings(&["read-only", "agent"]), "t")
            .unwrap();
    }

    #[test]
    fn account_history_is_append_only() {
        let store = store_with(&[("a", &["read-only"]), ("b", &["read-only"])]);
        store.merge("a", "b", &[], &[], "t").unwrap();
        let conn = store.conn.lock().unwrap();
        assert!(
            conn.execute("UPDATE account_events SET actor = 'x'", [])
                .is_err()
        );
        assert!(conn.execute("DELETE FROM account_events", []).is_err());
    }

    fn events_named(store: &IdentityStore, event: &str) -> Vec<AccountEvent> {
        store
            .account_events()
            .unwrap()
            .into_iter()
            .filter(|e| e.event == event)
            .collect()
    }

    #[test]
    fn repointing_sources_attributes_one_by_one_and_back() {
        let store = store_with(&[("a", &["read-only"]), ("b", &["read-only"])]);
        store
            .set_attribute("a", "picture", Some("a.png"), "t")
            .unwrap();
        store
            .set_attribute("a", "summary", Some("A's summary"), "t")
            .unwrap();
        store
            .set_attribute("b", "picture", Some("b.png"), "t")
            .unwrap();
        store
            .set_attribute("b", "summary", Some("B's summary"), "t")
            .unwrap();
        store.merge("a", "b", &[], &[], "t").unwrap();

        // Primary wins until a repoint.
        let row = store.account("a").unwrap().unwrap();
        assert_eq!(row.picture.as_deref(), Some("a.png"));
        assert_eq!(row.summary.as_deref(), Some("A's summary"));

        // Picture alone from the secondary: the summary is untouched.
        assert!(
            store
                .repoint_attribute("a", "picture", "b", "user")
                .unwrap()
        );
        let row = store.account("a").unwrap().unwrap();
        assert_eq!(row.picture.as_deref(), Some("b.png"));
        assert_eq!(row.summary.as_deref(), Some("A's summary"));

        // Then the summary, independently.
        assert!(
            store
                .repoint_attribute("a", "summary", "b", "user")
                .unwrap()
        );
        assert_eq!(
            store.account("a").unwrap().unwrap().summary.as_deref(),
            Some("B's summary")
        );

        // Picture back: the primary's own value returns exactly,
        // because nothing was copied over it.
        assert!(
            store
                .repoint_attribute("a", "picture", "a", "user")
                .unwrap()
        );
        let row = store.account("a").unwrap().unwrap();
        assert_eq!(row.picture.as_deref(), Some("a.png"));
        assert_eq!(row.summary.as_deref(), Some("B's summary"));
        assert_eq!(row.display_name, "a");

        // The sources read back as set, in attribute order.
        let sources = store.attribute_sources("a").unwrap();
        let by_attr: Vec<(&str, &str)> = sources
            .iter()
            .map(|s| (s.attribute.as_str(), s.source_account_id.as_str()))
            .collect();
        assert_eq!(
            by_attr,
            vec![("display_name", "a"), ("picture", "a"), ("summary", "b")]
        );

        // Each step is one audited event on the account, naming the
        // source, in order.
        let repoints = events_named(&store, "attribute-repoint");
        assert_eq!(repoints.len(), 3);
        let steps: Vec<serde_json::Value> = repoints
            .iter()
            .map(|e| serde_json::from_str(&e.detail).unwrap())
            .collect();
        assert_eq!(
            steps[0],
            serde_json::json!({"attribute": "picture", "from": "a", "to": "b"})
        );
        assert_eq!(
            steps[1],
            serde_json::json!({"attribute": "summary", "from": "a", "to": "b"})
        );
        assert_eq!(
            steps[2],
            serde_json::json!({"attribute": "picture", "from": "b", "to": "a"})
        );
        assert!(
            repoints
                .iter()
                .all(|e| e.account_id == "a" && e.actor == "user")
        );
        assert_eq!(repoints[0].other_account_id, "b");

        // The shell keeps its own values throughout.
        let shell = store.account("b").unwrap().unwrap();
        assert_eq!(shell.picture.as_deref(), Some("b.png"));
        assert_eq!(shell.summary.as_deref(), Some("B's summary"));
    }

    #[test]
    fn a_repoint_to_the_current_source_changes_nothing() {
        let store = store_with(&[("a", &["read-only"]), ("b", &["read-only"])]);
        store.merge("a", "b", &[], &[], "t").unwrap();
        assert!(!store.repoint_attribute("a", "picture", "a", "t").unwrap());
        assert!(store.repoint_attribute("a", "picture", "b", "t").unwrap());
        assert!(!store.repoint_attribute("a", "picture", "b", "t").unwrap());
        assert_eq!(events_named(&store, "attribute-repoint").len(), 1);
    }

    #[test]
    fn repointing_refuses_bad_targets_and_sources_and_writes_nothing() {
        let store = store_with(&[
            ("a", &["read-only"]),
            ("b", &["read-only"]),
            ("c", &["read-only"]),
        ]);
        store.merge("a", "b", &[], &[], "t").unwrap();
        let before = store.account_events().unwrap().len();

        let err = store.repoint_attribute("a", "email", "b", "t").unwrap_err();
        assert!(err.contains("unknown profile attribute email"), "{err}");
        let err = store
            .repoint_attribute("a", "picture", "c", "t")
            .unwrap_err();
        assert!(err.contains("neither a nor merged into it"), "{err}");
        let err = store
            .repoint_attribute("b", "picture", "a", "t")
            .unwrap_err();
        assert!(err.contains("merged into a"), "{err}");
        let err = store
            .repoint_attribute("zz", "picture", "a", "t")
            .unwrap_err();
        assert!(err.contains("unknown account zz"), "{err}");
        let err = store
            .set_attribute("b", "summary", Some("x"), "t")
            .unwrap_err();
        assert!(err.contains("merged into a"), "{err}");
        let err = store
            .set_attribute("a", "display_name", None, "t")
            .unwrap_err();
        assert!(err.contains("cannot be cleared"), "{err}");

        assert_eq!(store.account_events().unwrap().len(), before);
        let conn = store.conn.lock().unwrap();
        let sources: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM account_attribute_sources",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(sources, 0);
    }

    #[test]
    fn an_attribute_can_come_from_anywhere_down_a_merge_chain() {
        let store = store_with(&[
            ("a", &["read-only"]),
            ("b", &["read-only"]),
            ("c", &["read-only"]),
        ]);
        store
            .set_attribute("c", "display_name", Some("Cee"), "t")
            .unwrap();
        store.merge("b", "c", &[], &[], "t").unwrap();
        store.merge("a", "b", &[], &[], "t").unwrap();
        assert!(
            store
                .repoint_attribute("a", "display_name", "c", "t")
                .unwrap()
        );
        assert_eq!(store.account("a").unwrap().unwrap().display_name, "Cee");
    }

    #[test]
    fn an_own_value_edit_waits_behind_a_repoint() {
        let store = store_with(&[("a", &["read-only"]), ("b", &["read-only"])]);
        store
            .set_attribute("b", "summary", Some("from b"), "t")
            .unwrap();
        store.merge("a", "b", &[], &[], "t").unwrap();
        store.repoint_attribute("a", "summary", "b", "t").unwrap();
        assert!(
            store
                .set_attribute("a", "summary", Some("mine"), "t")
                .unwrap()
        );
        assert_eq!(
            store.account("a").unwrap().unwrap().summary.as_deref(),
            Some("from b"),
            "the attribute still reads from b"
        );
        store.repoint_attribute("a", "summary", "a", "t").unwrap();
        assert_eq!(
            store.account("a").unwrap().unwrap().summary.as_deref(),
            Some("mine")
        );
        assert!(
            !store
                .set_attribute("a", "summary", Some("mine"), "t")
                .unwrap()
        );
        assert_eq!(events_named(&store, "attribute-set").len(), 2);
    }

    #[test]
    fn the_config_accounts_display_name_stays_in_config() {
        let store = store_with(&[("x", &["read-only"])]);
        store
            .seed_account(&crate::registry::Account {
                account_id: "owner".to_string(),
                display_name: "Owner".to_string(),
                email: None,
                roles: strings(&["owner"]),
                identities: vec![],
                local_credential_hash: String::new(),
            })
            .unwrap();
        store
            .set_attribute("x", "picture", Some("x.png"), "t")
            .unwrap();
        store
            .merge("owner", "x", &[], &strings(&["read-only"]), "t")
            .unwrap();
        let err = store
            .repoint_attribute("owner", "display_name", "x", "t")
            .unwrap_err();
        assert!(err.contains("set in config"), "{err}");
        let err = store
            .set_attribute("owner", "display_name", Some("Other"), "t")
            .unwrap_err();
        assert!(err.contains("set in config"), "{err}");
        assert!(
            store
                .repoint_attribute("owner", "picture", "x", "t")
                .unwrap()
        );
        assert_eq!(
            store.account("owner").unwrap().unwrap().picture.as_deref(),
            Some("x.png")
        );
    }

    #[test]
    fn an_older_store_gains_the_profile_columns_on_open() {
        let dir = std::env::temp_dir().join(format!(
            "idp-profile-migrate-{}-{}",
            std::process::id(),
            crate::registry::now_unix()
        ));
        let path = dir.join("identities.db");
        std::fs::create_dir_all(&dir).unwrap();
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE accounts (
                    account_id TEXT PRIMARY KEY,
                    display_name TEXT NOT NULL,
                    roles TEXT NOT NULL
                );
                INSERT INTO accounts VALUES ('old', 'Old', '[\"read-only\"]');",
            )
            .unwrap();
        }
        let store = IdentityStore::open(Some(&path)).unwrap();
        let row = store.account("old").unwrap().unwrap();
        assert_eq!(row.display_name, "Old");
        assert_eq!(row.picture, None);
        assert!(
            store
                .set_attribute("old", "picture", Some("p.png"), "t")
                .unwrap()
        );
        drop(store);
        let reopened = IdentityStore::open(Some(&path)).unwrap();
        assert_eq!(
            reopened.account("old").unwrap().unwrap().picture.as_deref(),
            Some("p.png")
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
