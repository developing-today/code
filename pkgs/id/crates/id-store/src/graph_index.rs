//! Rebuildable ordered triple indexes and bounded continuation scans
//! (slice A-GRAPH-9125-02, issue #10234; design part A §4.2, statements
//! S5.4, S5.6, S5.7 and S5.11).
//!
//! An initial redb materialization of graph assertions. Each assertion is
//! one triple of a verified signed graph record, named by its record ID and
//! its position in that record. It is stored three times, in SPO, POS and
//! OSP tables, keyed by `id_core::tuple` encodings of
//!
//! ```text
//! (graph, first term, second term, third term, record, index)
//! ```
//!
//! where the terms are the canonical term bytes of slice A-GRAPH-9125-01,
//! permuted for the table's order. Every pattern with a fixed graph has a
//! leading-component prefix in one of the three tables, so a scan never
//! filters: each row it reads is a row it returns.
//!
//! The index is derived data. It holds no signature and is never the
//! authority for a triple: signed record bytes are, and [`GraphIndex::rebuild`]
//! recreates the index from them. A meta table records the index schema
//! version, the assertion count and an order-independent set digest (the XOR
//! of a BLAKE3 hash per assertion). [`GraphIndex::verify`] recomputes both
//! from each of the three tables, so a missing, extra, altered or
//! undecodable row in any table is reported as corruption, never served as
//! truth; a missing table or meta row makes the index unavailable.
//!
//! The crate root does not declare this module yet (slice 9110-09 wires
//! graph exports); `tests/graph_index_rebuild.rs` includes it by path. It
//! depends only on redb, `id_core::tuple` and BLAKE3, and takes terms as the
//! canonical bytes the caller has already verified.

use std::fmt;
use std::ops::Bound;
use std::path::Path;

use id_core::tuple::{TupleEncoder, TupleValue, decode as decode_tuple};
use redb::{
    Database, ReadOnlyTable, ReadTransaction, ReadableDatabase, ReadableTable, TableDefinition,
    TableError,
};

/// Version of this index's key layout and meta rows.
pub const INDEX_SCHEMA: u64 = 1;

/// Largest page a scan may ask for (the design's initial row ceiling).
pub const ROW_CEILING: usize = 1_000;

const SPO: TableDefinition<'static, &'static [u8], ()> = TableDefinition::new("graph_spo");
const POS: TableDefinition<'static, &'static [u8], ()> = TableDefinition::new("graph_pos");
const OSP: TableDefinition<'static, &'static [u8], ()> = TableDefinition::new("graph_osp");
const META: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("graph_index_meta");

const META_SCHEMA: &str = "schema";
const META_COUNT: &str = "count";
const META_DIGEST: &str = "digest";

/// Domain separation for the per-assertion digest.
const ROW_DIGEST_CONTEXT: &str = "id graph index row v1";
/// Domain separation for a scan pattern's fingerprint.
const PATTERN_CONTEXT: &str = "id graph index pattern v1";

/// One of the three stored orders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Order {
    /// Subject, predicate, object.
    Spo,
    /// Predicate, object, subject.
    Pos,
    /// Object, subject, predicate.
    Osp,
}

impl Order {
    /// Every order, in table order.
    pub const ALL: [Self; 3] = [Self::Spo, Self::Pos, Self::Osp];

    const fn table(self) -> TableDefinition<'static, &'static [u8], ()> {
        match self {
            Self::Spo => SPO,
            Self::Pos => POS,
            Self::Osp => OSP,
        }
    }

    const fn tag(self) -> i64 {
        match self {
            Self::Spo => 0,
            Self::Pos => 1,
            Self::Osp => 2,
        }
    }

    /// The order's stored name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Spo => "spo",
            Self::Pos => "pos",
            Self::Osp => "osp",
        }
    }
}

/// An assertion's identity: the signed record and the triple's position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AssertionKey {
    /// The record's envelope ID digest.
    pub record: [u8; 32],
    /// The triple's index in the record.
    pub index: u32,
}

/// One assertion as the index stores it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IndexedTriple {
    /// The assertion's identity.
    pub key: AssertionKey,
    /// The logical graph.
    pub graph: [u8; 32],
    /// Canonical subject term bytes.
    pub subject: Vec<u8>,
    /// Canonical predicate term bytes.
    pub predicate: Vec<u8>,
    /// Canonical object term bytes.
    pub object: Vec<u8>,
}

/// A pattern over one graph: each term is fixed or open.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Pattern {
    /// The logical graph.
    pub graph: [u8; 32],
    /// Canonical subject bytes, if fixed.
    pub subject: Option<Vec<u8>>,
    /// Canonical predicate bytes, if fixed.
    pub predicate: Option<Vec<u8>>,
    /// Canonical object bytes, if fixed.
    pub object: Option<Vec<u8>>,
}

/// What one scan read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScanCost {
    /// Index rows read, including the one row read past a full page to
    /// learn whether another page follows.
    pub scanned_rows: u64,
    /// Index key bytes read.
    pub scanned_bytes: u64,
}

/// One page of a scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanPage {
    /// The order the rows are in.
    pub order: Order,
    /// The matching assertions, in that order.
    pub rows: Vec<IndexedTriple>,
    /// Where the next page starts; `None` when the scan is complete.
    pub next: Option<Continuation>,
    /// What this page read.
    pub cost: ScanCost,
}

impl ScanPage {
    /// Whether the scan has no further rows.
    #[must_use]
    pub const fn complete(&self) -> bool {
        self.next.is_none()
    }
}

/// An opaque position inside one pattern's scan.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Continuation(Vec<u8>);

impl Continuation {
    /// The continuation's bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Wraps bytes received from a caller; they are checked on use.
    #[must_use]
    pub const fn from_bytes(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }
}

/// Counts written by a rebuild.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexState {
    /// Distinct assertions indexed.
    pub count: u64,
    /// Order-independent digest of the indexed set.
    pub digest: [u8; 32],
}

/// Why an index operation failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexError {
    /// The storage engine failed.
    Storage(String),
    /// A table or meta row is missing; rebuild from signed records.
    Unavailable(&'static str),
    /// The index was written under a schema this reader does not know.
    UnsupportedSchema(u64),
    /// The index disagrees with itself; rebuild from signed records.
    Corrupt {
        /// The table where the disagreement was found.
        table: &'static str,
        /// What was wrong.
        problem: &'static str,
    },
    /// The continuation does not belong to this pattern or schema.
    CursorMismatch,
    /// The page size is zero or above [`ROW_CEILING`].
    LimitOutOfRange(usize),
}

impl IndexError {
    /// A stable failure class.
    #[must_use]
    pub const fn class(&self) -> &'static str {
        match self {
            Self::Storage(_) => "storage",
            Self::Unavailable(_) => "index_unavailable",
            Self::UnsupportedSchema(_) => "unsupported_schema",
            Self::Corrupt { .. } => "index_corrupt",
            Self::CursorMismatch => "cursor_mismatch",
            Self::LimitOutOfRange(_) => "limit_out_of_range",
        }
    }
}

impl fmt::Display for IndexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Storage(e) => write!(f, "graph index storage error: {e}"),
            Self::Unavailable(what) => write!(f, "graph index unavailable: {what} missing"),
            Self::UnsupportedSchema(v) => write!(f, "graph index schema {v} is not supported"),
            Self::Corrupt { table, problem } => write!(f, "graph index {table} corrupt: {problem}"),
            Self::CursorMismatch => f.write_str("continuation does not belong to this scan"),
            Self::LimitOutOfRange(n) => {
                write!(f, "page size {n} is outside 1..={ROW_CEILING}")
            }
        }
    }
}

impl std::error::Error for IndexError {}

fn storage(err: impl fmt::Display) -> IndexError {
    IndexError::Storage(err.to_string())
}

const fn corrupt(table: &'static str, problem: &'static str) -> IndexError {
    IndexError::Corrupt { table, problem }
}

/// The three terms in `order`.
const fn permute(t: &IndexedTriple, order: Order) -> [&[u8]; 3] {
    let (s, p, o) = (
        t.subject.as_slice(),
        t.predicate.as_slice(),
        t.object.as_slice(),
    );
    match order {
        Order::Spo => [s, p, o],
        Order::Pos => [p, o, s],
        Order::Osp => [o, s, p],
    }
}

/// The table key for `t` in `order`.
#[must_use]
pub fn row_key(t: &IndexedTriple, order: Order) -> Vec<u8> {
    let [a, b, c] = permute(t, order);
    TupleEncoder::new()
        .bytes(&t.graph)
        .bytes(a)
        .bytes(b)
        .bytes(c)
        .bytes(&t.key.record)
        .int(i64::from(t.key.index))
        .build()
}

/// Decodes a table key written in `order`.
///
/// # Errors
///
/// [`IndexError::Corrupt`] when the key is not a well-formed row.
pub fn decode_row(key: &[u8], order: Order) -> Result<IndexedTriple, IndexError> {
    let table = order.name();
    let values = decode_tuple(key).map_err(|_| corrupt(table, "key does not decode"))?;
    let [
        TupleValue::Bytes(graph),
        TupleValue::Bytes(a),
        TupleValue::Bytes(b),
        TupleValue::Bytes(c),
        TupleValue::Bytes(record),
        TupleValue::Int(index),
    ] = <[TupleValue; 6]>::try_from(values)
        .map_err(|_| corrupt(table, "key has the wrong arity"))?
    else {
        return Err(corrupt(table, "key has the wrong element types"));
    };
    let graph = <[u8; 32]>::try_from(graph).map_err(|_| corrupt(table, "graph is not 32 bytes"))?;
    let record =
        <[u8; 32]>::try_from(record).map_err(|_| corrupt(table, "record is not 32 bytes"))?;
    let index = u32::try_from(index).map_err(|_| corrupt(table, "index is out of range"))?;
    let (subject, predicate, object) = match order {
        Order::Spo => (a, b, c),
        Order::Pos => (c, a, b),
        Order::Osp => (b, c, a),
    };
    Ok(IndexedTriple {
        key: AssertionKey { record, index },
        graph,
        subject,
        predicate,
        object,
    })
}

/// A hash of one assertion, the same whichever table it was read from.
fn row_digest(t: &IndexedTriple) -> [u8; 32] {
    blake3::derive_key(ROW_DIGEST_CONTEXT, &row_key(t, Order::Spo))
}

fn xor_into(acc: &mut [u8; 32], digest: &[u8; 32]) {
    for (a, d) in acc.iter_mut().zip(digest) {
        *a ^= d;
    }
}

/// The table and key prefix that answer `pattern`, every fixed term
/// leading.
#[must_use]
pub fn plan(pattern: &Pattern) -> (Order, Vec<u8>) {
    let (s, p, o) = (
        pattern.subject.as_deref(),
        pattern.predicate.as_deref(),
        pattern.object.as_deref(),
    );
    let (order, lead): (Order, Vec<&[u8]>) = match (s, p, o) {
        (Some(s), Some(p), Some(o)) => (Order::Spo, vec![s, p, o]),
        (Some(s), Some(p), None) => (Order::Spo, vec![s, p]),
        (Some(s), None, None) => (Order::Spo, vec![s]),
        (None, Some(p), Some(o)) => (Order::Pos, vec![p, o]),
        (None, Some(p), None) => (Order::Pos, vec![p]),
        (Some(s), None, Some(o)) => (Order::Osp, vec![o, s]),
        (None, None, Some(o)) => (Order::Osp, vec![o]),
        (None, None, None) => (Order::Spo, vec![]),
    };
    let mut encoder = TupleEncoder::new();
    encoder.bytes(&pattern.graph);
    for term in lead {
        encoder.bytes(term);
    }
    (order, encoder.build())
}

fn fingerprint(pattern: &Pattern) -> Vec<u8> {
    let mut encoder = TupleEncoder::new();
    encoder.bytes(&pattern.graph);
    for term in [&pattern.subject, &pattern.predicate, &pattern.object] {
        match term {
            Some(bytes) => encoder.bytes(bytes),
            None => encoder.null(),
        };
    }
    blake3::derive_key(PATTERN_CONTEXT, &encoder.build()).to_vec()
}

fn continuation(order: Order, pattern: &Pattern, last: &[u8]) -> Continuation {
    Continuation(
        TupleEncoder::new()
            .int(i64::try_from(INDEX_SCHEMA).unwrap_or(i64::MAX))
            .int(order.tag())
            .bytes(&fingerprint(pattern))
            .bytes(last)
            .build(),
    )
}

/// The last key the continuation names, if it belongs to this scan.
fn resume(
    cont: &Continuation,
    order: Order,
    pattern: &Pattern,
    prefix: &[u8],
) -> Result<Vec<u8>, IndexError> {
    let values = decode_tuple(&cont.0).map_err(|_| IndexError::CursorMismatch)?;
    let [
        TupleValue::Int(schema),
        TupleValue::Int(tag),
        TupleValue::Bytes(print),
        TupleValue::Bytes(last),
    ] = <[TupleValue; 4]>::try_from(values).map_err(|_| IndexError::CursorMismatch)?
    else {
        return Err(IndexError::CursorMismatch);
    };
    if u64::try_from(schema).ok() != Some(INDEX_SCHEMA)
        || tag != order.tag()
        || print != fingerprint(pattern)
        || !last.starts_with(prefix)
    {
        return Err(IndexError::CursorMismatch);
    }
    Ok(last)
}

/// A redb-backed SPO/POS/OSP index over graph assertions.
#[derive(Debug)]
pub struct GraphIndex {
    db: Database,
}

impl GraphIndex {
    /// Opens or creates the index file. A new file has no tables until
    /// [`GraphIndex::rebuild`] runs, so it reports unavailable.
    ///
    /// # Errors
    ///
    /// [`IndexError::Storage`].
    pub fn open(path: impl AsRef<Path>) -> Result<Self, IndexError> {
        Ok(Self {
            db: Database::create(path).map_err(storage)?,
        })
    }

    /// Replaces the whole index with `assertions`, in one transaction.
    /// Duplicate assertions are indexed once.
    ///
    /// # Errors
    ///
    /// [`IndexError::Storage`].
    pub fn rebuild(
        &self,
        assertions: impl IntoIterator<Item = IndexedTriple>,
    ) -> Result<IndexState, IndexError> {
        let txn = self.db.begin_write().map_err(storage)?;
        for order in Order::ALL {
            txn.delete_table(order.table()).map_err(storage)?;
        }
        txn.delete_table(META).map_err(storage)?;
        let mut state = IndexState {
            count: 0,
            digest: [0; 32],
        };
        Self::write_rows(&txn, assertions, &mut state)?;
        txn.commit().map_err(storage)?;
        Ok(state)
    }

    /// Adds assertions to an available index, in one transaction.
    /// Assertions already present are skipped.
    ///
    /// # Errors
    ///
    /// [`IndexError::Unavailable`] or [`IndexError::UnsupportedSchema`]
    /// when the index has not been built, and [`IndexError::Storage`].
    pub fn append(
        &self,
        assertions: impl IntoIterator<Item = IndexedTriple>,
    ) -> Result<IndexState, IndexError> {
        // Check the index is built (and of a known schema) without
        // creating tables, then take the count and digest from inside
        // the write transaction: redb has one writer at a time, so state
        // read there is current, and concurrent appends cannot both
        // build on the same stale count (#12011).
        {
            let read = self.db.begin_read().map_err(storage)?;
            Self::meta(&read)?;
        }
        let txn = self.db.begin_write().map_err(storage)?;
        let mut state = {
            let meta = txn.open_table(META).map_err(storage)?;
            Self::state_from(&meta)?
        };
        Self::write_rows(&txn, assertions, &mut state)?;
        txn.commit().map_err(storage)?;
        Ok(state)
    }

    fn write_rows(
        txn: &redb::WriteTransaction,
        assertions: impl IntoIterator<Item = IndexedTriple>,
        state: &mut IndexState,
    ) -> Result<(), IndexError> {
        let mut spo = txn.open_table(SPO).map_err(storage)?;
        let mut pos = txn.open_table(POS).map_err(storage)?;
        let mut osp = txn.open_table(OSP).map_err(storage)?;
        for t in assertions {
            let fresh = spo
                .insert(row_key(&t, Order::Spo).as_slice(), ())
                .map_err(storage)?
                .is_none();
            if !fresh {
                continue;
            }
            pos.insert(row_key(&t, Order::Pos).as_slice(), ())
                .map_err(storage)?;
            osp.insert(row_key(&t, Order::Osp).as_slice(), ())
                .map_err(storage)?;
            state.count += 1;
            xor_into(&mut state.digest, &row_digest(&t));
        }
        let mut meta = txn.open_table(META).map_err(storage)?;
        meta.insert(META_SCHEMA, INDEX_SCHEMA.to_be_bytes().as_slice())
            .map_err(storage)?;
        meta.insert(META_COUNT, state.count.to_be_bytes().as_slice())
            .map_err(storage)?;
        meta.insert(META_DIGEST, state.digest.as_slice())
            .map_err(storage)?;
        Ok(())
    }

    fn open_read(
        txn: &ReadTransaction,
        def: TableDefinition<'static, &'static [u8], ()>,
        name: &'static str,
    ) -> Result<ReadOnlyTable<&'static [u8], ()>, IndexError> {
        match txn.open_table(def) {
            Ok(table) => Ok(table),
            Err(TableError::TableDoesNotExist(_)) => Err(IndexError::Unavailable(name)),
            Err(err) => Err(storage(err)),
        }
    }

    /// The recorded state, after checking that every table exists and the
    /// schema is known.
    fn meta(txn: &ReadTransaction) -> Result<IndexState, IndexError> {
        let meta = match txn.open_table(META) {
            Ok(table) => table,
            Err(TableError::TableDoesNotExist(_)) => {
                return Err(IndexError::Unavailable("meta"));
            }
            Err(err) => return Err(storage(err)),
        };
        let state = Self::state_from(&meta)?;
        for order in Order::ALL {
            Self::open_read(txn, order.table(), order.name())?;
        }
        Ok(state)
    }

    /// The schema-checked count and digest from a meta table, read-only
    /// or inside a write transaction.
    fn state_from<T: ReadableTable<&'static str, &'static [u8]>>(
        meta: &T,
    ) -> Result<IndexState, IndexError> {
        let read = |key: &'static str| -> Result<Vec<u8>, IndexError> {
            meta.get(key)
                .map_err(storage)?
                .map(|v| v.value().to_vec())
                .ok_or(IndexError::Unavailable(key))
        };
        let schema = <[u8; 8]>::try_from(read(META_SCHEMA)?)
            .map_err(|_| corrupt("meta", "schema is not 8 bytes"))?;
        let schema = u64::from_be_bytes(schema);
        if schema != INDEX_SCHEMA {
            return Err(IndexError::UnsupportedSchema(schema));
        }
        let count = <[u8; 8]>::try_from(read(META_COUNT)?)
            .map_err(|_| corrupt("meta", "count is not 8 bytes"))?;
        let digest = <[u8; 32]>::try_from(read(META_DIGEST)?)
            .map_err(|_| corrupt("meta", "digest is not 32 bytes"))?;
        Ok(IndexState {
            count: u64::from_be_bytes(count),
            digest,
        })
    }

    /// Reads one page of `pattern`, after `after` if given, of at most
    /// `limit` rows, from a single read snapshot.
    ///
    /// # Errors
    ///
    /// [`IndexError::LimitOutOfRange`], [`IndexError::CursorMismatch`],
    /// [`IndexError::Unavailable`], [`IndexError::UnsupportedSchema`],
    /// [`IndexError::Corrupt`] for a row that does not decode or does not
    /// match its prefix, and [`IndexError::Storage`].
    pub fn scan(
        &self,
        pattern: &Pattern,
        after: Option<&Continuation>,
        limit: usize,
    ) -> Result<ScanPage, IndexError> {
        if limit == 0 || limit > ROW_CEILING {
            return Err(IndexError::LimitOutOfRange(limit));
        }
        let (order, prefix) = plan(pattern);
        let last = after
            .map(|c| resume(c, order, pattern, &prefix))
            .transpose()?;
        let txn = self.db.begin_read().map_err(storage)?;
        Self::meta(&txn)?;
        let table = Self::open_read(&txn, order.table(), order.name())?;
        let lower = last
            .as_deref()
            .map_or(Bound::Included(prefix.as_slice()), Bound::Excluded);
        let mut rows = Vec::new();
        let mut cost = ScanCost::default();
        let mut last_key = None;
        let mut more = false;
        for entry in table.range((lower, Bound::Unbounded)).map_err(storage)? {
            let (key, _) = entry.map_err(storage)?;
            let key = key.value();
            if !key.starts_with(&prefix) {
                break;
            }
            cost.scanned_rows += 1;
            cost.scanned_bytes += key.len() as u64;
            if rows.len() == limit {
                more = true;
                break;
            }
            let row = decode_row(key, order)?;
            rows.push(row);
            last_key = Some(key.to_vec());
        }
        let next = match (more, last_key) {
            (true, Some(last)) => Some(continuation(order, pattern, &last)),
            _ => None,
        };
        Ok(ScanPage {
            order,
            rows,
            next,
            cost,
        })
    }

    /// Checks that every table holds exactly the recorded set: each row
    /// decodes, and each table's count and set digest equal the meta rows.
    ///
    /// # Errors
    ///
    /// [`IndexError::Unavailable`], [`IndexError::UnsupportedSchema`],
    /// [`IndexError::Corrupt`] and [`IndexError::Storage`].
    pub fn verify(&self) -> Result<IndexState, IndexError> {
        let txn = self.db.begin_read().map_err(storage)?;
        let state = Self::meta(&txn)?;
        for order in Order::ALL {
            let table = Self::open_read(&txn, order.table(), order.name())?;
            let mut found = IndexState {
                count: 0,
                digest: [0; 32],
            };
            for entry in table.iter().map_err(storage)? {
                let (key, _) = entry.map_err(storage)?;
                let row = decode_row(key.value(), order)?;
                if row_key(&row, order) != key.value() {
                    return Err(corrupt(order.name(), "key is not in canonical form"));
                }
                found.count += 1;
                xor_into(&mut found.digest, &row_digest(&row));
            }
            if found.count != state.count {
                return Err(corrupt(order.name(), "row count differs from meta"));
            }
            if found.digest != state.digest {
                return Err(corrupt(order.name(), "row set differs from meta"));
            }
        }
        Ok(state)
    }
}
