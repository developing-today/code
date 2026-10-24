//! Turso backend (crate `turso` 0.8.2, MIT). Same schema, SQL and transaction shape as
//! sqlite_store.rs, run through turso's async API via block.rs. Two settings the code
//! depends on: WITHOUT ROWID needs `experimental_without_rowid`, and a read-write open
//! takes an exclusive fcntl lock on the file (one process at a time), unless the
//! experimental multi-process WAL is on.

use crate::block::block_on;
use crate::keys::{self, Claim, Id};
use crate::store::{Batch, Counts, Store};
use anyhow::{Context, Result, bail};
use std::path::Path;
use std::time::Duration;
use turso::transaction::TransactionBehavior;
use turso::{Builder, Connection, Database, Value};

/// The same schema as sqlite_store.rs. Every table is WITHOUT ROWID, which turso accepts
/// only with `experimental_without_rowid` (COMPAT.md: insert-only).
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

/// The same tables as SCHEMA, as rowid tables. turso's WITHOUT ROWID support is insert-only
/// and its planner scans author_seq for the (author, seq) lookup; rowid tables with the
/// same PRIMARY KEY get a unique autoindex. Used only for the rowid configuration.
pub const SCHEMA_ROWID: &str = "
    CREATE TABLE IF NOT EXISTS claims (id BLOB PRIMARY KEY, body BLOB NOT NULL);
    CREATE TABLE IF NOT EXISTS alpha (key BLOB PRIMARY KEY);
    CREATE TABLE IF NOT EXISTS omega (key BLOB PRIMARY KEY);
    CREATE TABLE IF NOT EXISTS author_seq (
        author BLOB NOT NULL,
        seq INTEGER NOT NULL,
        id BLOB NOT NULL,
        PRIMARY KEY (author, seq)
    );
";

pub struct TursoStore {
    // Declared before `_db`, so the connection drops before the database it came from.
    conn: Connection,
    _db: Database,
}

impl TursoStore {
    /// Read-write open with the S6 settings: WAL, synchronous=FULL, busy timeout 5 s.
    /// `multiprocess` turns on turso's experimental multi-process WAL.
    pub fn open_with(path: &Path, multiprocess: bool) -> Result<Self> {
        Self::open_schema(path, multiprocess, SCHEMA, true)
    }

    /// The same open with another schema. `without_rowid` enables turso's experimental
    /// WITHOUT ROWID support, which the schema needs when it uses that clause.
    pub fn open_schema(path: &Path, multiprocess: bool, schema: &str, without_rowid: bool) -> Result<Self> {
        block_on(async {
            let uri = path.to_str().context("non-UTF-8 path")?;
            let db = Builder::new_local(uri)
                .experimental_without_rowid(without_rowid)
                .experimental_multiprocess_wal(multiprocess)
                .build()
                .await
                .with_context(|| format!("turso: opening {}", path.display()))?;
            let conn = db.connect()?;
            conn.busy_timeout(Duration::from_secs(5))?;
            conn.pragma_update("journal_mode", "'wal'").await?;
            conn.pragma_update("synchronous", "FULL").await?;
            conn.execute_batch(schema).await?;
            Ok(Self { conn, _db: db })
        })
    }

    /// The first integer a query returns (a pragma's value, for example).
    pub fn query_int(&self, sql: &str) -> Result<i64> {
        block_on(scalar_int(&self.conn, sql))
    }

    /// Runs SQL that returns no rows (a pragma, for example).
    pub fn exec(&self, sql: &str) -> Result<()> {
        block_on(async { Ok(self.conn.execute_batch(sql).await?) })
    }

    /// The journal mode and synchronous level this connection reports.
    pub fn settings(&self) -> Result<String> {
        block_on(async {
            let mode = scalar_text(&self.conn, "PRAGMA journal_mode").await?;
            let sync = scalar_int(&self.conn, "PRAGMA synchronous").await?;
            Ok(format!("journal_mode={mode} synchronous={sync}"))
        })
    }

    /// Writes a (author, seq) mapping with no check, as checks.rs does for the other stores.
    pub fn bypass_check(&self, c: &Claim) -> Result<String> {
        let id = c.id();
        block_on(async {
            let res = self
                .conn
                .execute(
                    "INSERT INTO author_seq (author, seq, id) VALUES (?1, ?2, ?3)",
                    (c.author.to_vec(), c.seq as i64, id.to_vec()),
                )
                .await;
            Ok(match res {
                Ok(_) => "raw insert returned no error".to_string(),
                Err(e) => format!("raw insert refused by the database: {e}"),
            })
        })
    }

    /// The indexes on author_seq, as turso reports them.
    pub fn describe_unique(&self) -> Result<Vec<String>> {
        block_on(async {
            let mut rows = self.conn.query("PRAGMA index_list('author_seq')", ()).await?;
            let mut out = Vec::new();
            while let Some(row) = rows.next().await? {
                out.push(format!("{:?}", row.get_value(1)?) + &format!(" {:?}", row.get_value(2)?));
            }
            Ok(out)
        })
    }

    /// Holds one IMMEDIATE transaction with `claims` written and not committed, for
    /// `secs` seconds, then rolls back. Used by the concurrency probe in another process.
    pub fn hold(path: &Path, secs: u64, claims: &[Claim]) -> Result<()> {
        let mut store = Self::open_with(path, false)?;
        block_on(async {
            let tx = store
                .conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .await?;
            let batch = insert_in(&tx, claims).await?;
            println!("ready: {} claims written, not committed", batch.inserted);
            std::thread::sleep(Duration::from_secs(secs));
            tx.rollback().await?;
            println!("released: rolled back");
            Ok(())
        })
    }
}

fn blob(v: Value) -> Result<Vec<u8>> {
    match v {
        Value::Blob(b) => Ok(b),
        other => bail!("expected a blob, got {other:?}"),
    }
}

async fn scalar_text(conn: &Connection, sql: &str) -> Result<String> {
    let mut rows = conn.query(sql, ()).await?;
    let row = rows.next().await?.context("no row")?;
    let out = match row.get_value(0)? {
        Value::Text(s) => s,
        other => format!("{other:?}"),
    };
    while rows.next().await?.is_some() {}
    Ok(out)
}

async fn scalar_int(conn: &Connection, sql: &str) -> Result<i64> {
    let mut rows = conn.query(sql, ()).await?;
    let row = rows.next().await?.context("no row")?;
    let out = match row.get_value(0)? {
        Value::Integer(n) => n,
        other => bail!("expected an integer, got {other:?}"),
    };
    while rows.next().await?.is_some() {}
    Ok(out)
}

/// Writes `claims` inside the caller's transaction. The first fork fails the call.
/// Statements are drained after reading, which turso's query_row does for the same reason.
pub async fn insert_in(conn: &Connection, claims: &[Claim]) -> Result<Batch> {
    let mut find = conn
        .prepare("SELECT id FROM author_seq WHERE author = ?1 AND seq = ?2")
        .await?;
    let mut ins_claim = conn.prepare("INSERT INTO claims (id, body) VALUES (?1, ?2)").await?;
    let mut ins_alpha = conn.prepare("INSERT INTO alpha (key) VALUES (?1)").await?;
    let mut ins_omega = conn.prepare("INSERT INTO omega (key) VALUES (?1)").await?;
    let mut ins_seq = conn
        .prepare("INSERT INTO author_seq (author, seq, id) VALUES (?1, ?2, ?3)")
        .await?;
    let mut batch = Batch::default();
    for c in claims {
        let id = c.id();
        let held = {
            let mut rows = find.query((c.author.to_vec(), c.seq as i64)).await?;
            let first = match rows.next().await? {
                Some(row) => Some(blob(row.get_value(0)?)?),
                None => None,
            };
            while rows.next().await?.is_some() {}
            first
        };
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
        ins_claim.execute((id.to_vec(), c.body())).await?;
        ins_alpha.execute((keys::alpha_key(c, &id),)).await?;
        ins_omega.execute((keys::omega_key(c, &id),)).await?;
        ins_seq.execute((c.author.to_vec(), c.seq as i64, id.to_vec())).await?;
        batch.inserted += 1;
    }
    Ok(batch)
}

/// Every row's first column as a blob, in the query's order.
async fn blob_rows(conn: &Connection, sql: &str, params: impl turso::IntoParams) -> Result<Vec<Vec<u8>>> {
    let mut rows = conn.query(sql, params).await?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        out.push(blob(row.get_value(0)?)?);
    }
    Ok(out)
}

async fn claim_by_id(conn: &Connection, id: &Id) -> Result<Claim> {
    let rows = blob_rows(conn, "SELECT body FROM claims WHERE id = ?1", (id.to_vec(),)).await?;
    let body = rows.first().context("claim row missing")?;
    Claim::from_body(body)
}

async fn count_rows(conn: &Connection, table: &str) -> Result<u64> {
    Ok(scalar_int(conn, &format!("SELECT count(*) FROM {table}")).await? as u64)
}

fn to_id(b: &[u8]) -> Result<Id> {
    Ok(b.try_into()?)
}

impl Store for TursoStore {
    const NAME: &'static str = "turso";

    fn open(path: &Path) -> Result<Self> {
        Self::open_with(path, false)
    }

    fn insert_batch(&mut self, claims: &[Claim]) -> Result<Batch> {
        block_on(async {
            let tx = self
                .conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .await?;
            match insert_in(&tx, claims).await {
                Ok(batch) => {
                    tx.commit().await?;
                    Ok(batch)
                }
                Err(e) => {
                    tx.rollback().await?;
                    Err(e)
                }
            }
        })
    }

    fn by_subject(&self, subject: &Id) -> Result<Vec<Claim>> {
        block_on(async {
            let keys_found = match keys::prefix_end(subject) {
                Some(hi) => {
                    blob_rows(
                        &self.conn,
                        "SELECT key FROM alpha WHERE key >= ?1 AND key < ?2 ORDER BY key",
                        (subject.to_vec(), hi),
                    )
                    .await?
                }
                None => {
                    blob_rows(
                        &self.conn,
                        "SELECT key FROM alpha WHERE key >= ?1 ORDER BY key",
                        (subject.to_vec(),),
                    )
                    .await?
                }
            };
            let mut out = Vec::with_capacity(keys_found.len());
            for k in &keys_found {
                out.push(claim_by_id(&self.conn, &keys::key_id(k)).await?);
            }
            Ok(out)
        })
    }

    fn subjects_with(&self, name: &[u8], value: &[u8]) -> Result<Vec<Id>> {
        block_on(async {
            let (lo, hi) = keys::omega_bounds(name, Some(value));
            let found = blob_rows(
                &self.conn,
                "SELECT key FROM omega WHERE key >= ?1 AND key < ?2 ORDER BY key",
                (lo, hi),
            )
            .await?;
            Ok(found.iter().map(|k| keys::omega_subject(k)).collect())
        })
    }

    fn by_author(&self, author: &Id) -> Result<Vec<Claim>> {
        block_on(async {
            let ids = blob_rows(
                &self.conn,
                "SELECT id FROM author_seq WHERE author = ?1 ORDER BY seq",
                (author.to_vec(),),
            )
            .await?;
            let mut out = Vec::with_capacity(ids.len());
            for id in &ids {
                out.push(claim_by_id(&self.conn, &to_id(id)?).await?);
            }
            Ok(out)
        })
    }

    fn counts(&self) -> Result<Counts> {
        block_on(async {
            Ok([
                count_rows(&self.conn, "claims").await?,
                count_rows(&self.conn, "alpha").await?,
                count_rows(&self.conn, "omega").await?,
                count_rows(&self.conn, "author_seq").await?,
            ])
        })
    }

    fn all_claims(&self) -> Result<Vec<Claim>> {
        block_on(async {
            let bodies = blob_rows(&self.conn, "SELECT body FROM claims ORDER BY id", ()).await?;
            bodies.iter().map(|b| Claim::from_body(b)).collect()
        })
    }
}

/// The rowid configuration: the same statements on SCHEMA_ROWID, with no experimental flag.
/// Named `turso-rowid` in the output, so it is not confused with the WITHOUT ROWID store.
pub struct TursoRowidStore(pub TursoStore);

impl Store for TursoRowidStore {
    const NAME: &'static str = "turso-rowid";

    fn open(path: &Path) -> Result<Self> {
        Ok(Self(TursoStore::open_schema(path, false, SCHEMA_ROWID, false)?))
    }

    fn insert_batch(&mut self, claims: &[Claim]) -> Result<Batch> {
        self.0.insert_batch(claims)
    }

    fn by_subject(&self, subject: &Id) -> Result<Vec<Claim>> {
        self.0.by_subject(subject)
    }

    fn subjects_with(&self, name: &[u8], value: &[u8]) -> Result<Vec<Id>> {
        self.0.subjects_with(name, value)
    }

    fn by_author(&self, author: &Id) -> Result<Vec<Claim>> {
        self.0.by_author(author)
    }

    fn counts(&self) -> Result<Counts> {
        self.0.counts()
    }

    fn all_claims(&self) -> Result<Vec<Claim>> {
        self.0.all_claims()
    }
}
