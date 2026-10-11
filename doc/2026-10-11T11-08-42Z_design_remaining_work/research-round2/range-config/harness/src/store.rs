//! Event store on redb, with S6's layout (`/home/user/work/spikes-s6/src/redb_store.rs`).
//!
//! - `author_seq`: author (32 B) || seq (8 B, big-endian) -> event ID. This is the range index.
//! - `events`: event ID -> record (see `chain::record`).
//!
//! As in S6, one write transaction per batch, and the (author, seq) check runs inside it:
//! redb has no unique constraint. p2panda's seq is u32; it is widened to S6's 8-byte field.

use crate::range::Party;
use anyhow::{Context, Result, bail};
use redb::{Database, ReadableDatabase, ReadableTable, ReadableTableMetadata, TableDefinition};
use std::path::Path;

/// author (32 B) || seq (8 B, big-endian).
pub const KEY_LEN: usize = 40;
pub type Key = [u8; KEY_LEN];
pub type Id = [u8; 32];

const EVENTS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("events");
const AUTHOR_SEQ: TableDefinition<&[u8], &[u8]> = TableDefinition::new("author_seq");

/// S6's `author_seq_key`, with the u32 seq widened to u64.
pub fn key_of(author: &[u8; 32], seq: u32) -> Key {
    let mut key = [0u8; KEY_LEN];
    key[..32].copy_from_slice(author);
    key[32..].copy_from_slice(&(seq as u64).to_be_bytes());
    key
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Batch {
    pub inserted: usize,
    pub duplicates: usize,
}

/// One verified event ready to store.
pub struct Item {
    pub key: Key,
    pub id: Id,
    pub record: Vec<u8>,
}

pub struct Store {
    db: Database,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let db = Database::create(path).with_context(|| format!("redb: opening {}", path.display()))?;
        // redb has no tables until a write creates them, and reads of a missing table fail. Create
        // both now, so that an empty store can be read (the bootstrap case). S7's original did not.
        let txn = db.begin_write()?;
        {
            txn.open_table(AUTHOR_SEQ)?;
            txn.open_table(EVENTS)?;
        }
        txn.commit()?;
        Ok(Self { db })
    }

    /// Writes `items` in one transaction. An identical event already held is a duplicate. A
    /// different event at an occupied (author, seq) is a fork, and it fails the whole batch.
    pub fn insert(&self, items: &[Item]) -> Result<Batch> {
        let txn = self.db.begin_write()?;
        let mut batch = Batch::default();
        {
            let mut author_seq = txn.open_table(AUTHOR_SEQ)?;
            let mut events = txn.open_table(EVENTS)?;
            for item in items {
                let held = author_seq.get(item.key.as_slice())?.map(|g| g.value().to_vec());
                match held {
                    Some(prev) if prev.as_slice() == item.id.as_slice() => {
                        batch.duplicates += 1;
                        continue;
                    }
                    Some(_) => bail!("fork at key {}", hex::encode(item.key)),
                    None => {}
                }
                events.insert(item.id.as_slice(), item.record.as_slice())?;
                author_seq.insert(item.key.as_slice(), item.id.as_slice())?;
                batch.inserted += 1;
            }
        }
        txn.commit()?;
        Ok(batch)
    }

    /// The sorted (key, ID) index that the range scheme runs on.
    pub fn index(&self) -> Result<Party> {
        let txn = self.db.begin_read()?;
        let table = txn.open_table(AUTHOR_SEQ)?;
        let mut keys = Vec::with_capacity(table.len()? as usize);
        let mut ids = Vec::with_capacity(keys.capacity());
        for row in table.iter()? {
            let (key, id) = row?;
            keys.push(key.value().try_into()?);
            ids.push(id.value().try_into()?);
        }
        Ok(Party::new(keys, ids))
    }

    pub fn record(&self, id: &Id) -> Result<Vec<u8>> {
        let txn = self.db.begin_read()?;
        let events = txn.open_table(EVENTS)?;
        let row = events.get(id.as_slice())?.with_context(|| format!("no event {}", hex::encode(id)))?;
        Ok(row.value().to_vec())
    }

    pub fn has(&self, id: &Id) -> Result<bool> {
        let txn = self.db.begin_read()?;
        Ok(txn.open_table(EVENTS)?.get(id.as_slice())?.is_some())
    }

    /// Every (key, ID) row, in key order.
    pub fn pairs(&self) -> Result<Vec<(Key, Id)>> {
        let txn = self.db.begin_read()?;
        let table = txn.open_table(AUTHOR_SEQ)?;
        let mut out = Vec::with_capacity(table.len()? as usize);
        for row in table.iter()? {
            let (key, id) = row?;
            out.push((key.value().try_into()?, id.value().try_into()?));
        }
        Ok(out)
    }

    /// Row counts: (events, author_seq).
    pub fn counts(&self) -> Result<(u64, u64)> {
        let txn = self.db.begin_read()?;
        Ok((txn.open_table(EVENTS)?.len()?, txn.open_table(AUTHOR_SEQ)?.len()?))
    }

    /// Removes one event from both tables. Used only to build a gap in a test store.
    pub fn remove(&self, key: &Key) -> Result<()> {
        let txn = self.db.begin_write()?;
        {
            let mut author_seq = txn.open_table(AUTHOR_SEQ)?;
            let mut events = txn.open_table(EVENTS)?;
            let id = author_seq.remove(key.as_slice())?.map(|g| g.value().to_vec());
            if let Some(id) = id {
                events.remove(id.as_slice())?;
            }
        }
        txn.commit()?;
        Ok(())
    }
}
