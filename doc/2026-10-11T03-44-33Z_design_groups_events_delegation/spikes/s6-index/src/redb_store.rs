//! redb implementation. One write transaction per batch. The (author, seq) check is a
//! lookup in that same transaction. redb has no unique constraints, so this lookup is
//! the only thing that keeps (author, seq) unique.

use crate::keys::{self, Claim, Id};
use crate::store::{Batch, Counts, Store};
use anyhow::{Context, Result, bail};
use redb::{Database, ReadableDatabase, ReadableTable, ReadableTableMetadata, TableDefinition};
use std::path::Path;

const CLAIMS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("claims");
const ALPHA: TableDefinition<&[u8], ()> = TableDefinition::new("alpha");
const OMEGA: TableDefinition<&[u8], ()> = TableDefinition::new("omega");
const AUTHOR_SEQ: TableDefinition<&[u8], &[u8]> = TableDefinition::new("author_seq");

pub struct RedbStore {
    db: Database,
}

impl RedbStore {
    /// Writes an (author, seq) mapping with no check, to show what happens when
    /// nothing enforces uniqueness.
    pub fn bypass_check(&self, c: &Claim) -> Result<String> {
        let id = c.id();
        let key = keys::author_seq_key(&c.author, c.seq);
        let txn = self.db.begin_write()?;
        let replaced = {
            let mut t = txn.open_table(AUTHOR_SEQ)?;
            let old = t.insert(key.as_slice(), id.as_slice())?;
            old.is_some()
        };
        txn.commit()?;
        Ok(format!(
            "raw insert returned no error; it replaced an existing mapping: {replaced}"
        ))
    }
}

/// Runs redb's compaction on a file. Returns the size before, the size after, and
/// whether redb shrank the file.
pub fn compact(path: &Path) -> Result<(u64, u64, bool)> {
    let before = std::fs::metadata(path)?.len();
    let mut db = Database::open(path)?;
    let shrunk = db.compact()?;
    drop(db);
    Ok((before, std::fs::metadata(path)?.len(), shrunk))
}

impl Store for RedbStore {
    const NAME: &'static str = "redb";

    fn open(path: &Path) -> Result<Self> {
        let db =
            Database::create(path).with_context(|| format!("redb: opening {}", path.display()))?;
        Ok(Self { db })
    }

    fn insert_batch(&mut self, claims: &[Claim]) -> Result<Batch> {
        let txn = self.db.begin_write()?;
        let mut batch = Batch::default();
        {
            let mut t_claims = txn.open_table(CLAIMS)?;
            let mut t_alpha = txn.open_table(ALPHA)?;
            let mut t_omega = txn.open_table(OMEGA)?;
            let mut t_author_seq = txn.open_table(AUTHOR_SEQ)?;
            for c in claims {
                let id = c.id();
                let seq_key = keys::author_seq_key(&c.author, c.seq);
                // Unique (author, seq): look the position up in this transaction first.
                let held = t_author_seq
                    .get(seq_key.as_slice())?
                    .map(|g| g.value().to_vec());
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
                t_claims.insert(id.as_slice(), c.body().as_slice())?;
                t_alpha.insert(keys::alpha_key(c, &id).as_slice(), ())?;
                t_omega.insert(keys::omega_key(c, &id).as_slice(), ())?;
                t_author_seq.insert(seq_key.as_slice(), id.as_slice())?;
                batch.inserted += 1;
            }
        }
        txn.commit()?;
        Ok(batch)
    }

    fn by_subject(&self, subject: &Id) -> Result<Vec<Claim>> {
        let txn = self.db.begin_read()?;
        let alpha = txn.open_table(ALPHA)?;
        let claims = txn.open_table(CLAIMS)?;
        let lo = subject.to_vec();
        let rows = match keys::prefix_end(&lo) {
            Some(hi) => alpha.range(lo.as_slice()..hi.as_slice())?,
            None => alpha.range(lo.as_slice()..)?,
        };
        let mut out = Vec::new();
        for row in rows {
            let (key, _) = row?;
            let id = keys::key_id(key.value());
            let body = claims
                .get(id.as_slice())?
                .context("alpha key without a claim")?;
            out.push(Claim::from_body(body.value())?);
        }
        Ok(out)
    }

    fn subjects_with(&self, name: &[u8], value: &[u8]) -> Result<Vec<Id>> {
        let txn = self.db.begin_read()?;
        let omega = txn.open_table(OMEGA)?;
        let (lo, hi) = keys::omega_bounds(name, Some(value));
        let rows = omega.range(lo.as_slice()..hi.as_slice())?;
        let mut out = Vec::new();
        for row in rows {
            let (key, _) = row?;
            out.push(keys::omega_subject(key.value()));
        }
        Ok(out)
    }

    fn by_author(&self, author: &Id) -> Result<Vec<Claim>> {
        let txn = self.db.begin_read()?;
        let author_seq = txn.open_table(AUTHOR_SEQ)?;
        let claims = txn.open_table(CLAIMS)?;
        let lo = author.to_vec();
        let rows = match keys::prefix_end(&lo) {
            Some(hi) => author_seq.range(lo.as_slice()..hi.as_slice())?,
            None => author_seq.range(lo.as_slice()..)?,
        };
        let mut out = Vec::new();
        for row in rows {
            let (_, id) = row?;
            let id: Id = id.value().try_into()?;
            let body = claims
                .get(id.as_slice())?
                .context("author_seq entry without a claim")?;
            out.push(Claim::from_body(body.value())?);
        }
        Ok(out)
    }

    fn counts(&self) -> Result<Counts> {
        let txn = self.db.begin_read()?;
        Ok([
            txn.open_table(CLAIMS)?.len()?,
            txn.open_table(ALPHA)?.len()?,
            txn.open_table(OMEGA)?.len()?,
            txn.open_table(AUTHOR_SEQ)?.len()?,
        ])
    }

    fn all_claims(&self) -> Result<Vec<Claim>> {
        let txn = self.db.begin_read()?;
        let claims = txn.open_table(CLAIMS)?;
        let mut out = Vec::new();
        for row in claims.iter()? {
            let (_, body) = row?;
            out.push(Claim::from_body(body.value())?);
        }
        Ok(out)
    }
}
