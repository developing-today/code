//! The waiting rows: the events held on disk, and the [`Mirror`] that keeps them in step with
//! the waiting pool.
//!
//! An event is held when it has arrived but cannot apply yet: a dependency is missing, or it
//! can never apply and is parked, so that a conflict with it is still seen. The pool holds it
//! in memory, under the caps in [`WaitingLimits`](crate::waiting::WaitingLimits). The same
//! events are written to two tables:
//!
//! - `waiting`: an event's ID to its row. The row holds the author, the sequence number, the
//!   arrival time, the arrival number, the dependencies the event still waits for, and the
//!   encoded event. A parked event has no dependencies.
//! - `waiting_by_dep`: a dependency's ID to the IDs of the rows that wait for it.
//!
//! The pool and the rows change together through [`Mirror`]. Each call on a `Mirror` makes one
//! change to the pool and the matching change to the rows, inside one write transaction. If
//! the transaction commits, the two agree, and a crash leaves the state as of the last commit.
//! On open the pool is rebuilt from the rows.

use p2panda_core::{Hash, SeqNum};
use redb::{ReadableMultimapTable, ReadableTable, WriteTransaction};

use super::{StoreError, WAITING, WAITING_BY_DEP, be_u64, corrupt, hash_of, row_event, seq_num};
use crate::chain::Event;
use crate::waiting::{AuthorKey, WaitRefusal, WaitingPool};

/// Bytes before a row's dependencies: the author, the sequence number, the arrival time, the
/// arrival number, and the dependency count.
const HEAD_LEN: usize = 32 + 8 + 8 + 8 + 1;

/// One held event, as its row holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Row {
    /// The event's ID, which is the row's key.
    pub id: Hash,
    /// The event's author.
    pub author: AuthorKey,
    /// The event's sequence number.
    pub seq: SeqNum,
    /// The clock time the event was held at. The age cap counts from here.
    pub arrived_at: u64,
    /// The order the event was held in. Rows reload in this order, so the pool releases the
    /// events in the order it did before a restart.
    pub arrival: u64,
    /// The dependencies the event still waits for. Empty for a parked event.
    pub deps: Vec<Hash>,
    /// The encoded event.
    pub bytes: Vec<u8>,
}

impl Row {
    fn encode(&self) -> Result<Vec<u8>, StoreError> {
        let count = u8::try_from(self.deps.len())
            .map_err(|_err| corrupt("a waiting event names too many dependencies"))?;
        let mut row = Vec::with_capacity(HEAD_LEN + 32 * self.deps.len() + self.bytes.len());
        row.extend_from_slice(&self.author);
        row.extend_from_slice(&u64::from(self.seq).to_be_bytes());
        row.extend_from_slice(&self.arrived_at.to_be_bytes());
        row.extend_from_slice(&self.arrival.to_be_bytes());
        row.push(count);
        for dep in &self.deps {
            row.extend_from_slice(dep.as_bytes());
        }
        row.extend_from_slice(&self.bytes);
        Ok(row)
    }

    pub(super) fn decode(id: Hash, row: &[u8]) -> Result<Self, StoreError> {
        if row.len() < HEAD_LEN {
            return Err(corrupt("a waiting row is shorter than its header"));
        }
        let author: AuthorKey = row[..32]
            .try_into()
            .map_err(|_err| corrupt("a waiting row has a short author"))?;
        let seq = seq_num(be_u64(&row[32..40])?)?;
        let arrived_at = be_u64(&row[40..48])?;
        let arrival = be_u64(&row[48..56])?;
        let deps_end = HEAD_LEN + 32 * usize::from(row[56]);
        if row.len() < deps_end {
            return Err(corrupt(
                "a waiting row names more dependencies than it holds",
            ));
        }
        let deps = row[HEAD_LEN..deps_end]
            .chunks_exact(32)
            .map(hash_of)
            .collect::<Result<Vec<Hash>, StoreError>>()?;
        Ok(Self {
            id,
            author,
            seq,
            arrived_at,
            arrival,
            deps,
            bytes: row[deps_end..].to_vec(),
        })
    }
}

/// The row for `id`, if one is held.
pub(super) fn get(txn: &WriteTransaction, id: &Hash) -> Result<Option<Row>, StoreError> {
    let rows = txn.open_table(WAITING)?;
    let found = rows.get(id.as_bytes().as_slice())?;
    found
        .map(|found| Row::decode(*id, found.value()))
        .transpose()
}

/// Every row, in arrival order.
pub(super) fn all(txn: &WriteTransaction) -> Result<Vec<Row>, StoreError> {
    let rows = txn.open_table(WAITING)?;
    let mut all = Vec::new();
    for entry in rows.iter()? {
        let (key, value) = entry?;
        all.push(Row::decode(hash_of(key.value())?, value.value())?);
    }
    all.sort_by_key(|row| row.arrival);
    Ok(all)
}

/// Writes `row` in place of any row with its ID, and indexes the dependencies it waits for.
pub(super) fn put(txn: &WriteTransaction, row: &Row) -> Result<(), StoreError> {
    if let Some(old) = get(txn, &row.id)? {
        unindex(txn, &old)?;
    }
    let encoded = row.encode()?;
    let mut rows = txn.open_table(WAITING)?;
    rows.insert(row.id.as_bytes().as_slice(), encoded.as_slice())?;
    drop(rows);
    let mut by_dep = txn.open_multimap_table(WAITING_BY_DEP)?;
    for dep in &row.deps {
        by_dep.insert(dep.as_bytes().as_slice(), row.id.as_bytes().as_slice())?;
    }
    Ok(())
}

/// Removes the row for `id`, if there is one.
pub(super) fn delete(txn: &WriteTransaction, id: &Hash) -> Result<(), StoreError> {
    let Some(old) = get(txn, id)? else {
        return Ok(());
    };
    unindex(txn, &old)?;
    let mut rows = txn.open_table(WAITING)?;
    rows.remove(id.as_bytes().as_slice())?;
    Ok(())
}

fn unindex(txn: &WriteTransaction, row: &Row) -> Result<(), StoreError> {
    let mut by_dep = txn.open_multimap_table(WAITING_BY_DEP)?;
    for dep in &row.deps {
        by_dep.remove(dep.as_bytes().as_slice(), row.id.as_bytes().as_slice())?;
    }
    Ok(())
}

/// The IDs of the rows that wait for `dep`.
fn waiters_of(txn: &WriteTransaction, dep: &Hash) -> Result<Vec<Hash>, StoreError> {
    let by_dep = txn.open_multimap_table(WAITING_BY_DEP)?;
    let mut waiters = Vec::new();
    for waiter in by_dep.get(dep.as_bytes().as_slice())? {
        waiters.push(hash_of(waiter?.value())?);
    }
    Ok(waiters)
}

/// What [`Mirror::hold`] did with an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Hold {
    /// The event is held, and its row is written.
    Held,
    /// The author already holds the cap, so the event is not held.
    Full,
    /// The pool did not keep the event: it had nothing left to wait for, so it is in the
    /// pool's ready list, or it had already arrived. Nothing is held.
    Skipped,
}

/// The waiting pool and its rows, inside one write transaction. Every method changes the pool
/// and the rows together, so the two agree whenever the transaction commits.
pub(super) struct Mirror<'a> {
    /// The transaction that every row change runs in.
    pub txn: &'a WriteTransaction,
    /// The pool. Callers read it for rival checks and take ready events from it.
    pub pool: &'a mut WaitingPool<Hash, Event>,
    /// The arrival number the next held event takes.
    pub arrivals: &'a mut u64,
}

impl Mirror<'_> {
    /// Drops every held event that is due at `now`, with its row. Returns how many were
    /// dropped.
    pub fn expire(&mut self, now: u64) -> Result<usize, StoreError> {
        self.flush_expired(now)
    }

    /// Deletes the rows of the events the pool has expired into its buffer, and returns
    /// how many. `pool.arrived` expires due events as a side effect of a release, into
    /// the buffer only `pool.expire` drains; a caller that releases without expiring
    /// first would commit with the pool and the rows disagreeing -- the expired events
    /// gone from the pool, their rows still on disk, and their expiry left for a later,
    /// unrelated `expire` to report (or, across a restart, for the reload to drop
    /// silently). Flushing here, the way [`expire`](Self::expire) does, makes the
    /// releasing call itself the one that effects the expiry, in its own transaction.
    fn flush_expired(&mut self, now: u64) -> Result<usize, StoreError> {
        let expired = self.pool.expire(now);
        for (_author, event) in &expired {
            delete(self.txn, &event.id)?;
        }
        Ok(expired.len())
    }

    /// Holds `event` at clock time `now`, either parked or waiting for `deps`, and writes its
    /// row. An event that is already held is left as it is.
    pub fn hold(
        &mut self,
        event: Event,
        deps: Vec<Hash>,
        parked: bool,
        now: u64,
    ) -> Result<Hold, StoreError> {
        let id = event.id;
        if self.pool.contains(&id) {
            return Ok(Hold::Held);
        }
        let author = *event.header.verifying_key.as_bytes();
        let seq = event.header.seq_num;
        let bytes = event.bytes.clone();
        let mut deps = deps;
        deps.sort_unstable_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        deps.dedup();
        let stored_deps = if parked { Vec::new() } else { deps.clone() };
        // An event that waits again keeps its first arrival time, so the age cap counts all of its
        // waiting. A released event keeps its row until the caller decides it, and the row holds
        // that time. An event the store has not held before arrived now.
        let arrived_at = match get(self.txn, &id)? {
            Some(row) => row.arrived_at,
            None => now,
        };
        // The ingest call has already expired what is due at `now`, and the dependencies are not
        // present, so the pool holds the event as given: it expires nothing and releases nothing.
        let missing = if parked { Vec::new() } else { deps };
        let result = self.pool.restore(author, id, missing, event, arrived_at);
        match result {
            Ok(()) if self.pool.contains(&id) => {
                let arrival = *self.arrivals;
                *self.arrivals += 1;
                let row = Row {
                    id,
                    author,
                    seq,
                    arrived_at,
                    arrival,
                    deps: stored_deps,
                    bytes,
                };
                put(self.txn, &row)?;
                Ok(Hold::Held)
            }
            Ok(()) => {
                delete(self.txn, &id)?;
                Ok(Hold::Skipped)
            }
            Err(WaitRefusal::AuthorFull | WaitRefusal::PoolFull) => {
                delete(self.txn, &id)?;
                Ok(Hold::Full)
            }
        }
    }

    /// Releases the events that waited only for `id`, which has just applied. The held copy of
    /// `id`, if there is one, ends here, and its row goes. Each row that waited for `id` loses
    /// that dependency. Returns the released events in arrival order. Their rows stay until the
    /// caller decides each one.
    pub fn release(&mut self, id: &Hash, now: u64) -> Result<Vec<Event>, StoreError> {
        let released = self.pool.arrived(*id, now);
        // `arrived` expired every event due at `now` into the pool's buffer; their
        // rows go in this transaction too, not a later one's.
        self.flush_expired(now)?;
        delete(self.txn, id)?;
        self.drop_dep(id)?;
        Ok(released)
    }

    /// Makes `id`, a fork proof header, present. Its waiters are released as [`release`](Self::release)
    /// releases them. A held copy of `id` is held for good, not dropped: a proof header never
    /// applies, so it waits for nothing, as a parked event does. The proof keeps its conflicts
    /// seen. Returns the released events in arrival order.
    pub fn present_proof(&mut self, id: &Hash, now: u64) -> Result<Vec<Event>, StoreError> {
        let held = get(self.txn, id)?;
        let released = self.pool.arrived(*id, now);
        // As in `release`: the events `arrived` expired lose their rows here. The
        // flush runs before the park below, so a held proof that was itself due is
        // re-held for good by the park rather than deleted by the flush.
        self.flush_expired(now)?;
        self.drop_dep(id)?;
        if let Some(row) = held {
            self.park(row)?;
        }
        Ok(released)
    }

    /// Holds the proof header `id` for good again. The pool released it, and its row is still
    /// there. A header can be released by another header's arrival, and then it is decided
    /// before its own arrival parks it. Nothing is held for an ID without a row.
    pub fn park_proof(&mut self, id: &Hash) -> Result<(), StoreError> {
        match get(self.txn, id)? {
            Some(row) => self.park(row),
            None => Ok(()),
        }
    }

    /// Holds `row` parked: it waits for nothing, so no arrival releases it, and only the age cap
    /// drops it. It keeps its first arrival time and its arrival number.
    fn park(&mut self, row: Row) -> Result<(), StoreError> {
        let event = row_event(&row)?;
        let parked = Row {
            deps: Vec::new(),
            ..row
        };
        self.pool
            .restore(parked.author, parked.id, [], event, parked.arrived_at)
            .map_err(|_refused| corrupt("a held proof header cannot be held for good"))?;
        put(self.txn, &parked)
    }

    /// Removes `id` from the dependencies of every row that waits for it.
    fn drop_dep(&self, id: &Hash) -> Result<(), StoreError> {
        for waiter in waiters_of(self.txn, id)? {
            let mut row = get(self.txn, &waiter)?
                .ok_or_else(|| corrupt("a dependency is indexed to a waiting event with no row"))?;
            row.deps.retain(|dep| dep != id);
            put(self.txn, &row)?;
        }
        Ok(())
    }

    /// Removes the row of an event the pool has released, once it is decided for good.
    pub fn drop_row(&self, id: &Hash) -> Result<(), StoreError> {
        delete(self.txn, id)
    }
}
