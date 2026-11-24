//! Scoped social-feed freshness cursor and delta pages
//! (slice A-GRAPH-9128-02, #10268; design part A §7, statements
//! S5.37, S5.38, S5.39).
//!
//! One durable projection ledger exists per complete feed scope
//! `(server, tenant, query, generation)`. Every feed entry receives a
//! monotonically increasing `projection_seq` and a stable `entry_id`
//! from that ledger, even when the entry came from another source
//! authority; the entry also retains its source provenance
//! `(authority, epoch, sequence, object_id)`, which is never the
//! merged cursor position. Two source authorities with identical
//! source epoch/sequence still receive distinct, ordered projection
//! entries, so neither is omitted nor conflated.
//!
//! The durable per-feed high-water mark continues across
//! projection-generation changes, so a projection sequence is never
//! reused; sequence exhaustion stops allocation with
//! [`LedgerError::SequenceExhausted`] rather than wrapping. On
//! restart the ledger and high-water mark are restored from durable
//! state; loss or rollback rotates the generation and prior cursors
//! report [`FeedCursorError::Expired`], never "unchanged". Access,
//! moderation, deletion, retention or query-policy changes advance
//! the generation and invalidate prior cursors the same way.
//!
//! A cursor is opaque to callers. Inside, it is the canonical CBOR
//! array `[1, tenant, query, generation, last_seq, last_entry_id,
//! cutoff, expires_at, mac]` where `mac` is the server-key BLAKE3
//! keyed hash over the preceding fields and `cutoff` is the pinned
//! vector of source frontiers included in the complete feed
//! snapshot. [`FeedLedger::since`] returns entries after
//! `(last_seq, last_entry_id)` up to the cursor's pinned cutoff;
//! [`FeedLedger::before`] pages older entries by the same stable
//! total order. A cursor minted for another tenant or query, or with
//! a bad MAC, reports [`FeedCursorError::Mismatch`]; a cursor for a
//! rotated generation, a pruned retention range, a reset ledger, or a
//! past `expires_at` reports [`FeedCursorError::Expired`].
//!
//! The cursor proves freshness only for its server, tenant, query
//! and pinned source-frontier vector; it says nothing about
//! disconnected remote peers.

use std::collections::BTreeSet;

use blake3::keyed_hash;

use crate::canonical_cbor::{Value, decode_canonical, encode_canonical};

/// Cursor format version.
pub const FEED_CURSOR_VERSION: u64 = 1;

/// First projection sequence allocated by a fresh ledger.
pub const FIRST_PROJECTION_SEQ: u64 = 1;

/// Source provenance retained on every feed entry.
///
/// It records the authority that accepted the object and its position
/// in that authority's own `(epoch, sequence)` order. Source positions
/// are not the merged cursor position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcePosition {
    /// Authority that accepted the source object.
    pub authority: String,
    /// Source epoch of the object in its authority's order.
    pub epoch: u64,
    /// Source sequence of the object in its authority's order.
    pub sequence: u64,
    /// Stable identifier of the source object.
    pub object_id: String,
}

/// One entry of the complete feed: a durable projection-ledger
/// sequence plus a stable entry identity, with source provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedEntry {
    /// Monotonic projection-ledger sequence; never reused.
    pub projection_seq: u64,
    /// Stable entry identity derived from `projection_seq`.
    pub entry_id: String,
    /// Source provenance of the projected object.
    pub source: SourcePosition,
}

/// One pinned source frontier of a complete-feed snapshot: entries
/// from `authority` are visible up to and including
/// `(epoch, sequence)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontierPin {
    /// Source authority this pin bounds.
    pub authority: String,
    /// Greatest visible source epoch for the authority.
    pub epoch: u64,
    /// Greatest visible source sequence at `epoch`.
    pub sequence: u64,
}

/// Identity of one complete feed: server, tenant, query fingerprint
/// and projection generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedScope {
    /// Server this ledger belongs to.
    pub server: String,
    /// Tenant this ledger belongs to.
    pub tenant: String,
    /// Fingerprint of the query this ledger projects.
    pub query: [u8; 32],
    /// Projection generation; policy changes advance it.
    pub generation: u64,
}

/// Why a feed cursor was refused.
///
/// `Expired` means the snapshot moved on: callers must take a fresh
/// `latest` page, never treat the cursor as "no changes". `Mismatch`
/// means the cursor does not belong to this feed at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedCursorError {
    /// The cursor's generation, retention horizon, `expires_at` or
    /// pinned position no longer matches this ledger.
    Expired,
    /// The cursor is malformed, has a bad MAC, or was minted for
    /// another tenant or query.
    Mismatch,
}

/// Why a projection sequence could not be allocated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedgerError {
    /// The durable high-water mark reached `u64::MAX`; allocation
    /// stops rather than wrapping and reusing a sequence.
    SequenceExhausted,
}

/// An opaque continuation token for one complete feed at a pinned
/// source-frontier vector.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FeedCursor(Vec<u8>);

impl FeedCursor {
    /// Wraps cursor bytes received from a caller.
    #[must_use]
    pub const fn from_bytes(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// The opaque bytes to hand to a caller.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// Durable projection ledger for one complete feed scope.
///
/// The ledger itself is an in-memory view: durability is the
/// caller's duty to persist [`FeedLedger::high_water`] and the
/// entries and restore them (see [`FeedLedger::restore`]) after a
/// restart, so no projection sequence is ever reused.
#[derive(Debug, Clone)]
pub struct FeedLedger {
    scope: FeedScope,
    high_water: u64,
    retained_from: u64,
    entries: Vec<FeedEntry>,
    seen_sources: BTreeSet<(String, u64, u64, String)>,
}

impl FeedLedger {
    /// Opens the ledger for `scope`, continuing from
    /// `durable_high_water` (zero for a fresh feed, the persisted
    /// mark after a restart).
    #[must_use]
    pub const fn new(
        server: String,
        tenant: String,
        query: [u8; 32],
        generation: u64,
        durable_high_water: u64,
    ) -> Self {
        Self {
            scope: FeedScope {
                server,
                tenant,
                query,
                generation,
            },
            high_water: durable_high_water,
            retained_from: 0,
            entries: Vec::new(),
            seen_sources: BTreeSet::new(),
        }
    }

    /// Restores a restarted ledger: the persisted entries plus the
    /// durable high-water mark they were allocated from. The mark
    /// must be at least the greatest restored projection sequence;
    /// the next append continues after the mark either way, so a
    /// short mark can never reuse a sequence.
    #[must_use]
    pub fn restore(
        server: String,
        tenant: String,
        query: [u8; 32],
        generation: u64,
        durable_high_water: u64,
        entries: Vec<FeedEntry>,
    ) -> Self {
        let restored_high_water = entries
            .iter()
            .map(|entry| entry.projection_seq)
            .max()
            .unwrap_or(0)
            .max(durable_high_water);
        let mut seen_sources = BTreeSet::new();
        for entry in &entries {
            seen_sources.insert((
                entry.source.authority.clone(),
                entry.source.epoch,
                entry.source.sequence,
                entry.source.object_id.clone(),
            ));
        }
        Self {
            scope: FeedScope {
                server,
                tenant,
                query,
                generation,
            },
            high_water: restored_high_water,
            retained_from: 0,
            entries,
            seen_sources,
        }
    }

    /// The feed scope this ledger projects.
    #[must_use]
    pub const fn scope(&self) -> &FeedScope {
        &self.scope
    }

    /// The durable high-water mark: persist it with the entries.
    #[must_use]
    pub const fn high_water(&self) -> u64 {
        self.high_water
    }

    /// The entries currently held, in projection-sequence order.
    #[must_use]
    pub fn entries(&self) -> &[FeedEntry] {
        &self.entries
    }

    /// Projects one accepted source object into the complete feed.
    ///
    /// Repeating the exact source key `(authority, epoch, sequence,
    /// object_id)` is idempotent and returns the existing entry
    /// without allocating a sequence. A new key allocates the next
    /// projection sequence; at `u64::MAX` allocation stops with
    /// [`LedgerError::SequenceExhausted`] instead of wrapping.
    ///
    /// # Errors
    ///
    /// [`LedgerError::SequenceExhausted`] when the high-water mark
    /// cannot advance.
    pub fn append(
        &mut self,
        authority: String,
        epoch: u64,
        sequence: u64,
        object_id: String,
    ) -> Result<FeedEntry, LedgerError> {
        let key = (authority, epoch, sequence, object_id);
        if self.seen_sources.contains(&key) {
            for entry in &self.entries {
                if entry.source.authority == key.0
                    && entry.source.epoch == key.1
                    && entry.source.sequence == key.2
                    && entry.source.object_id == key.3
                {
                    return Ok(entry.clone());
                }
            }
        }
        if self.high_water == u64::MAX {
            return Err(LedgerError::SequenceExhausted);
        }
        self.high_water += 1;
        let entry = FeedEntry {
            projection_seq: self.high_water,
            entry_id: entry_id_for(self.high_water),
            source: SourcePosition {
                authority: key.0.clone(),
                epoch: key.1,
                sequence: key.2,
                object_id: key.3.clone(),
            },
        };
        self.seen_sources.insert(key);
        self.entries.push(entry.clone());
        Ok(entry)
    }

    /// The complete snapshot at a pinned source-frontier vector:
    /// retained entries whose source position is at or below the pin
    /// for their authority, in the stable `(projection_seq,
    /// entry_id)` total order. Authorities without a pin, and
    /// entries past their pin, are excluded.
    #[must_use]
    pub fn snapshot(&self, cutoff: &[FrontierPin]) -> Vec<FeedEntry> {
        let mut visible: Vec<FeedEntry> = self
            .entries
            .iter()
            .filter(|entry| {
                entry.projection_seq >= self.retained_from
                    && cutoff.iter().any(|pin| {
                        pin.authority == entry.source.authority
                            && (entry.source.epoch, entry.source.sequence)
                                <= (pin.epoch, pin.sequence)
                    })
            })
            .cloned()
            .collect();
        visible
            .sort_by(|a, b| (a.projection_seq, &a.entry_id).cmp(&(b.projection_seq, &b.entry_id)));
        visible
    }

    /// Mints a fresh cursor over `cutoff` and returns the complete
    /// snapshot as the first page. The cursor position is the last
    /// snapshot entry, or the origin when the snapshot is empty.
    #[must_use]
    pub fn latest(
        &self,
        server_key: &[u8; 32],
        cutoff: &[FrontierPin],
        expires_at: u64,
    ) -> (FeedCursor, Vec<FeedEntry>) {
        let snapshot = self.snapshot(cutoff);
        let position = snapshot.last().map_or((0, String::new()), |entry| {
            (entry.projection_seq, entry.entry_id.clone())
        });
        (
            self.mint(server_key, cutoff, position, expires_at),
            snapshot,
        )
    }

    /// Returns entries after the cursor's `(last_seq, last_entry_id)`
    /// up to the cursor's pinned cutoff, with the cursor advanced to
    /// the last returned entry (unchanged when the page is empty).
    ///
    /// # Errors
    ///
    /// [`FeedCursorError`] when the cursor is malformed, has a bad
    /// MAC, belongs to another tenant or query, or is expired by a
    /// generation change, retention pruning, reset, or `expires_at`.
    pub fn since(
        &self,
        server_key: &[u8; 32],
        cursor_bytes: &[u8],
        now: u64,
    ) -> Result<(FeedCursor, Vec<FeedEntry>), FeedCursorError> {
        let presented = self.validate(server_key, cursor_bytes, now)?;
        let snapshot = self.snapshot(&presented.cutoff);
        let page: Vec<FeedEntry> = snapshot
            .into_iter()
            .filter(|entry| (entry.projection_seq, entry.entry_id.clone()) > presented.position)
            .collect();
        let position = page.last().map_or(presented.position, |entry| {
            (entry.projection_seq, entry.entry_id.clone())
        });
        Ok((
            self.mint(
                server_key,
                &presented.cutoff,
                position,
                presented.expires_at,
            ),
            page,
        ))
    }

    /// Returns entries before the cursor's `(last_seq,
    /// last_entry_id)` in the stable total order, with the cursor
    /// moved back to the oldest returned entry (unchanged when the
    /// page is empty) for continued backward paging.
    ///
    /// # Errors
    ///
    /// [`FeedCursorError`] under the same conditions as [`FeedLedger::since`].
    pub fn before(
        &self,
        server_key: &[u8; 32],
        cursor_bytes: &[u8],
        now: u64,
    ) -> Result<(FeedCursor, Vec<FeedEntry>), FeedCursorError> {
        let presented = self.validate(server_key, cursor_bytes, now)?;
        let snapshot = self.snapshot(&presented.cutoff);
        let page: Vec<FeedEntry> = snapshot
            .into_iter()
            .filter(|entry| (entry.projection_seq, entry.entry_id.clone()) < presented.position)
            .collect();
        let position = page.first().map_or(presented.position, |entry| {
            (entry.projection_seq, entry.entry_id.clone())
        });
        Ok((
            self.mint(
                server_key,
                &presented.cutoff,
                position,
                presented.expires_at,
            ),
            page,
        ))
    }

    /// Advances to a new projection generation after an access,
    /// moderation, deletion, retention or query-policy change. The
    /// high-water mark and entries carry over so no projection
    /// sequence is reused; cursors minted under older generations
    /// report [`FeedCursorError::Expired`].
    pub const fn advance_generation(&mut self, generation: u64) {
        self.scope.generation = generation;
    }

    /// Drops entries below `projection_seq` from the retention
    /// horizon. Cursors positioned inside the pruned range report
    /// [`FeedCursorError::Expired`]; the pruned sequences are never
    /// reallocated.
    pub fn prune_below(&mut self, projection_seq: u64) {
        if projection_seq > self.retained_from {
            self.retained_from = projection_seq;
        }
        self.entries
            .retain(|entry| entry.projection_seq >= self.retained_from);
    }

    /// Resets the ledger after durable-state loss: entries are cleared
    /// and the fresh `generation` invalidates every prior cursor, but
    /// the durable high-water mark is retained so a new entry cannot
    /// reuse a projection sequence. If the high-water mark itself was
    /// lost or rolled back, the caller must restore the last known
    /// durable mark before resetting; generation rotation alone does
    /// not make sequence reuse safe.
    pub fn reset(&mut self, generation: u64) {
        self.scope.generation = generation;
        self.retained_from = 0;
        self.entries.clear();
        self.seen_sources.clear();
    }

    /// Encodes and MACs a cursor for `cutoff` at `position`.
    fn mint(
        &self,
        server_key: &[u8; 32],
        cutoff: &[FrontierPin],
        position: (u64, String),
        expires_at: u64,
    ) -> FeedCursor {
        let mut pins: Vec<&FrontierPin> = cutoff.iter().collect();
        pins.sort_by(|a, b| a.authority.cmp(&b.authority));
        let pin_values: Vec<Value> = pins
            .iter()
            .map(|pin| {
                Value::Array(vec![
                    Value::Text(pin.authority.clone()),
                    Value::UInt(pin.epoch),
                    Value::UInt(pin.sequence),
                ])
            })
            .collect();
        let body = Value::Array(vec![
            Value::UInt(FEED_CURSOR_VERSION),
            Value::Text(self.scope.tenant.clone()),
            Value::Bytes(self.scope.query.to_vec()),
            Value::UInt(self.scope.generation),
            Value::UInt(position.0),
            Value::Text(position.1),
            Value::Array(pin_values),
            Value::UInt(expires_at),
        ]);
        let body_bytes = encode_canonical(&body).unwrap_or_default();
        let mac = keyed_hash(server_key, &body_bytes);
        let mut items = match body {
            Value::Array(items) => items,
            _ => Vec::new(),
        };
        items.push(Value::Bytes(mac.as_bytes().to_vec()));
        FeedCursor(encode_canonical(&Value::Array(items)).unwrap_or_default())
    }

    /// Decodes a presented cursor, checks its MAC, scope binding and
    /// freshness against this ledger.
    fn validate(
        &self,
        server_key: &[u8; 32],
        cursor_bytes: &[u8],
        now: u64,
    ) -> Result<PresentedCursor, FeedCursorError> {
        let Ok(Value::Array(items)) = decode_canonical(cursor_bytes) else {
            return Err(FeedCursorError::Mismatch);
        };
        let [
            Value::UInt(FEED_CURSOR_VERSION),
            Value::Text(tenant),
            Value::Bytes(query),
            Value::UInt(generation),
            Value::UInt(last_seq),
            Value::Text(last_entry_id),
            Value::Array(pin_values),
            Value::UInt(expires_at),
            Value::Bytes(mac),
        ] = items.as_slice()
        else {
            return Err(FeedCursorError::Mismatch);
        };
        let body = Value::Array(items[..items.len().saturating_sub(1)].to_vec());
        let body_bytes = encode_canonical(&body).unwrap_or_default();
        let expected = keyed_hash(server_key, &body_bytes);
        if mac.as_slice() != expected.as_bytes().as_slice() {
            return Err(FeedCursorError::Mismatch);
        }
        if tenant != &self.scope.tenant || query.as_slice() != self.scope.query.as_slice() {
            return Err(FeedCursorError::Mismatch);
        }
        if generation != &self.scope.generation {
            return Err(FeedCursorError::Expired);
        }
        let mut cutoff = Vec::with_capacity(pin_values.len());
        for pin in pin_values {
            let Value::Array(fields) = pin else {
                return Err(FeedCursorError::Mismatch);
            };
            let [
                Value::Text(authority),
                Value::UInt(epoch),
                Value::UInt(sequence),
            ] = fields.as_slice()
            else {
                return Err(FeedCursorError::Mismatch);
            };
            cutoff.push(FrontierPin {
                authority: authority.clone(),
                epoch: *epoch,
                sequence: *sequence,
            });
        }
        if expires_at < &now {
            return Err(FeedCursorError::Expired);
        }
        if last_seq < &self.retained_from {
            return Err(FeedCursorError::Expired);
        }
        Ok(PresentedCursor {
            cutoff,
            position: (*last_seq, last_entry_id.clone()),
            expires_at: *expires_at,
        })
    }
}

/// A presented cursor after MAC, scope and freshness checks.
struct PresentedCursor {
    cutoff: Vec<FrontierPin>,
    position: (u64, String),
    expires_at: u64,
}

/// Stable entry identity for a projection sequence.
fn entry_id_for(projection_seq: u64) -> String {
    format!("feed-entry-{projection_seq:08}")
}
