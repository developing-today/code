//! SQLite implementation. WAL with synchronous=FULL. Each batch is one BEGIN IMMEDIATE
//! transaction. The unique (author, seq) rule is the primary key of a WITHOUT ROWID
//! table, which SQLite enforces through its unique key, with no second B-tree. The
//! application lookup gives a readable error and treats an identical replay as a
//! duplicate.

use crate::keys::{self, Claim, Id};
use crate::store::{Batch, Counts, Store};
use anyhow::{Context, Result, bail};
use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior, params,
};
use std::path::Path;
use std::time::Duration;

pub const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS claims (id BLOB PRIMARY KEY, body BLOB NOT NULL) WITHOUT ROWID;
    CREATE TABLE IF NOT EXISTS alpha (key BLOB PRIMARY KEY) WITHOUT ROWID;
    CREATE TABLE IF NOT EXISTS omega (key BLOB PRIMARY KEY) WITHOUT ROWID;
    CREATE TABLE IF NOT EXISTS author_seq (
        author BLOB NOT NULL,
        seq INTEGER NOT NULL,
        id BLOB NOT NULL,
        PRIMARY KEY (author, seq)
    ) WITHOUT ROWID;
";

pub struct SqliteStore {
    conn: Connection,
}

impl SqliteStore {
    pub fn open_rw(path: &Path) -> Result<Self> {
        Self::open_rw_with(path, Duration::from_secs(5))
    }

    /// Read-write connection with WAL, synchronous=FULL, and the given busy timeout.
    pub fn open_rw_with(path: &Path, busy: Duration) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("sqlite: opening {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        conn.busy_timeout(busy)?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    pub fn connection_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }

    /// The indexes on author_seq, as SQLite reports them.
    pub fn describe_unique(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("PRAGMA index_list('author_seq')")?;
        let rows = stmt.query_map([], |r| {
            Ok(format!(
                "name={} unique={} origin={}",
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?
            ))
        })?;
        let out = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(out)
    }

    /// Runs a plain INSERT into author_seq with no application check.
    pub fn bypass_check(&self, c: &Claim) -> Result<String> {
        let id = c.id();
        match self.conn.execute(
            "INSERT INTO author_seq (author, seq, id) VALUES (?1, ?2, ?3)",
            params![c.author.as_slice(), c.seq as i64, id.as_slice()],
        ) {
            Ok(_) => Ok("raw insert returned no error".to_string()),
            Err(e) => Ok(format!("raw insert refused by the database: {e}")),
        }
    }
}

/// A read-only connection, for a second process.
pub fn open_read_only(path: &Path) -> Result<Connection> {
    Ok(Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?)
}

/// Writes `claims` inside `tx`. The first fork fails the call. The caller's rollback
/// then removes everything written so far.
pub fn insert_in(tx: &Transaction<'_>, claims: &[Claim]) -> Result<Batch> {
    let mut batch = Batch::default();
    for c in claims {
        let id = c.id();
        let held: Option<Vec<u8>> = tx
            .query_row(
                "SELECT id FROM author_seq WHERE author = ?1 AND seq = ?2",
                params![c.author.as_slice(), c.seq as i64],
                |r| r.get(0),
            )
            .optional()?;
        match held {
            Some(prev) if prev.as_slice() == id.as_slice() => {
                batch.duplicates += 1;
                continue;
            }
            Some(prev) => bail!(
                "fork: (author, seq {}) already holds claim {}",
                c.seq,
                hex::encode(prev)
            ),
            None => {}
        }
        tx.prepare_cached("INSERT INTO claims (id, body) VALUES (?1, ?2)")?
            .execute(params![id.as_slice(), c.body()])?;
        tx.prepare_cached("INSERT INTO alpha (key) VALUES (?1)")?
            .execute(params![keys::alpha_key(c, &id)])?;
        tx.prepare_cached("INSERT INTO omega (key) VALUES (?1)")?
            .execute(params![keys::omega_key(c, &id)])?;
        tx.prepare_cached("INSERT INTO author_seq (author, seq, id) VALUES (?1, ?2, ?3)")?
            .execute(params![c.author.as_slice(), c.seq as i64, id.as_slice()])?;
        batch.inserted += 1;
    }
    Ok(batch)
}

fn blob_rows(conn: &Connection, sql: &str, args: impl rusqlite::Params) -> Result<Vec<Vec<u8>>> {
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt.query_map(args, |r| r.get::<_, Vec<u8>>(0))?;
    let out = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(out)
}

/// Keys in `table` within [lo, hi), in key order: a range scan on the primary key.
/// `hi` of None means no upper bound.
fn range_rows(conn: &Connection, table: &str, lo: &[u8], hi: Option<&[u8]>) -> Result<Vec<Vec<u8>>> {
    match hi {
        Some(hi) => blob_rows(
            conn,
            &format!("SELECT key FROM {table} WHERE key >= ?1 AND key < ?2 ORDER BY key"),
            params![lo, hi],
        ),
        None => blob_rows(
            conn,
            &format!("SELECT key FROM {table} WHERE key >= ?1 ORDER BY key"),
            params![lo],
        ),
    }
}

fn claim_by_id(conn: &Connection, id: &Id) -> Result<Claim> {
    let body: Vec<u8> = conn.query_row(
        "SELECT body FROM claims WHERE id = ?1",
        params![id.as_slice()],
        |r| r.get(0),
    )?;
    Claim::from_body(&body)
}

fn to_id(b: &[u8]) -> Result<Id> {
    Ok(b.try_into()?)
}

impl Store for SqliteStore {
    const NAME: &'static str = "sqlite";

    fn open(path: &Path) -> Result<Self> {
        Self::open_rw(path)
    }

    fn insert_batch(&mut self, claims: &[Claim]) -> Result<Batch> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let batch = insert_in(&tx, claims)?;
        tx.commit()?;
        Ok(batch)
    }

    fn by_subject(&self, subject: &Id) -> Result<Vec<Claim>> {
        let tx = self.conn.unchecked_transaction()?;
        let hi = keys::prefix_end(subject);
        let found = range_rows(&tx, "alpha", subject, hi.as_deref())?;
        let mut out = Vec::with_capacity(found.len());
        for k in &found {
            out.push(claim_by_id(&tx, &keys::key_id(k))?);
        }
        Ok(out)
    }

    fn subjects_with(&self, name: &[u8], value: &[u8]) -> Result<Vec<Id>> {
        let tx = self.conn.unchecked_transaction()?;
        let (lo, hi) = keys::omega_bounds(name, Some(value));
        let found = range_rows(&tx, "omega", &lo, Some(&hi))?;
        Ok(found.iter().map(|k| keys::omega_subject(k)).collect())
    }

    fn by_author(&self, author: &Id) -> Result<Vec<Claim>> {
        let tx = self.conn.unchecked_transaction()?;
        let ids = blob_rows(
            &tx,
            "SELECT id FROM author_seq WHERE author = ?1 ORDER BY seq",
            params![author.as_slice()],
        )?;
        let mut out = Vec::with_capacity(ids.len());
        for id in &ids {
            out.push(claim_by_id(&tx, &to_id(id)?)?);
        }
        Ok(out)
    }

    fn counts(&self) -> Result<Counts> {
        let count = |table: &str| -> Result<u64> {
            let n: i64 = self.conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))?;
            Ok(n as u64)
        };
        Ok([count("claims")?, count("alpha")?, count("omega")?, count("author_seq")?])
    }

    fn all_claims(&self) -> Result<Vec<Claim>> {
        let bodies = blob_rows(&self.conn, "SELECT body FROM claims ORDER BY id", params![])?;
        bodies.iter().map(|b| Claim::from_body(b)).collect()
    }
}
