//! The 50-transaction workload and its model.
//!
//! Each transaction is one redb write transaction. It inserts a batch of event rows and their
//! author index rows, deletes the batch from five transactions earlier (so pages get freed and
//! reused), and updates two meta rows. The model is the map that results from applying the
//! first n transactions, so the verifier can say exactly which prefix a reopened database holds.

use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition, TableError};
use std::collections::BTreeMap;
use std::error::Error;

pub const TXNS: u64 = 50;

pub const EVENTS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("events");
pub const BY_AUTHOR: TableDefinition<&[u8], &[u8]> = TableDefinition::new("by_author");
pub const META: TableDefinition<&[u8], &[u8]> = TableDefinition::new("meta");

pub const EVENTS_TAG: u8 = 0;
pub const BY_AUTHOR_TAG: u8 = 1;
pub const META_TAG: u8 = 2;

/// Keyed by (table tag, key bytes). Values are the stored bytes.
pub type Model = BTreeMap<(u8, Vec<u8>), Vec<u8>>;

#[derive(Clone, Debug)]
pub enum Op {
    Put(u8, Vec<u8>, Vec<u8>),
    Del(u8, Vec<u8>),
}

/// Claims in transaction b. Every tenth transaction is large enough to split B-tree pages.
fn claims(b: u64) -> u64 {
    if b % 10 == 9 { 120 } else { 8 + (b * 7) % 25 }
}

fn first_event(b: u64) -> u64 {
    (0..b).map(claims).sum()
}

fn author_key(b: u64, seq: u64) -> Vec<u8> {
    let mut key = (b % 3).to_be_bytes().to_vec();
    key.extend_from_slice(&seq.to_be_bytes());
    key
}

fn payload(b: u64, i: u64) -> Vec<u8> {
    let mut v = format!("txn{b:03}-claim{i:03}-").into_bytes();
    v.resize(24 + ((b * 13 + i * 5) % 40) as usize, b'.');
    v
}

pub fn txn_ops(b: u64) -> Vec<Op> {
    let mut ops = Vec::new();
    let base = first_event(b);
    for i in 0..claims(b) {
        let event = (base + i).to_be_bytes().to_vec();
        ops.push(Op::Put(EVENTS_TAG, event.clone(), payload(b, i)));
        ops.push(Op::Put(BY_AUTHOR_TAG, author_key(b, b * 1000 + i), event));
    }
    if b >= 5 {
        let old = first_event(b - 5);
        ops.push(Op::Del(EVENTS_TAG, old.to_be_bytes().to_vec()));
        ops.push(Op::Del(BY_AUTHOR_TAG, author_key(b - 5, (b - 5) * 1000)));
    }
    ops.push(Op::Put(META_TAG, b"txns".to_vec(), (b + 1).to_be_bytes().to_vec()));
    ops.push(Op::Put(META_TAG, b"last".to_vec(), b.to_be_bytes().to_vec()));
    ops
}

pub fn apply_ops(model: &mut Model, ops: &[Op]) {
    for op in ops {
        match op {
            Op::Put(tag, k, v) => {
                model.insert((*tag, k.clone()), v.clone());
            }
            Op::Del(tag, k) => {
                model.remove(&(*tag, k.clone()));
            }
        }
    }
}

/// `models()[n]` is the state after the first n transactions, for n in 0..=TXNS.
pub fn models() -> Vec<Model> {
    let mut out = vec![Model::new()];
    let mut m = Model::new();
    for b in 0..TXNS {
        apply_ops(&mut m, &txn_ops(b));
        out.push(m.clone());
    }
    out
}

/// One write transaction with the given operations.
pub fn commit(db: &Database, ops: &[Op]) -> Result<(), Box<dyn Error>> {
    let txn = db.begin_write()?;
    {
        let mut events = txn.open_table(EVENTS)?;
        let mut by_author = txn.open_table(BY_AUTHOR)?;
        let mut meta = txn.open_table(META)?;
        for op in ops {
            match op {
                Op::Put(tag, k, v) => match *tag {
                    EVENTS_TAG => {
                        events.insert(k.as_slice(), v.as_slice())?;
                    }
                    BY_AUTHOR_TAG => {
                        by_author.insert(k.as_slice(), v.as_slice())?;
                    }
                    _ => {
                        meta.insert(k.as_slice(), v.as_slice())?;
                    }
                },
                Op::Del(tag, k) => match *tag {
                    EVENTS_TAG => {
                        events.remove(k.as_slice())?;
                    }
                    BY_AUTHOR_TAG => {
                        by_author.remove(k.as_slice())?;
                    }
                    _ => {
                        meta.remove(k.as_slice())?;
                    }
                },
            }
        }
    }
    txn.commit()?;
    Ok(())
}

/// Every row in the database, as a model. A table that does not exist yet counts as empty.
pub fn dump(db: &Database) -> Result<Model, Box<dyn Error>> {
    let txn = db.begin_read()?;
    let mut model = Model::new();
    for (tag, def) in [(EVENTS_TAG, EVENTS), (BY_AUTHOR_TAG, BY_AUTHOR), (META_TAG, META)] {
        let table = match txn.open_table(def) {
            Ok(t) => t,
            Err(TableError::TableDoesNotExist(_)) => continue,
            Err(e) => return Err(e.into()),
        };
        for row in table.iter()? {
            let (k, v) = row?;
            model.insert((tag, k.value().to_vec()), v.value().to_vec());
        }
    }
    Ok(model)
}
