//! Max-items result paging with snapshot-bound cursors.
//!
//! Slice A9119-query-cursor (issue #10274), design
//! `part-a-addressing/versioned-design.md` section "#9119 —
//! shorthand and streaming query API", statement A-9119.2
//! (S4.41–S4.44): `query(address, max_items, cursor?, snapshot?)`
//! returns pages of rows; each page carries its rows, the next
//! cursor, a `truncated` flag, the snapshot token and a terminal
//! reason.
//!
//! The rules this module pins:
//!
//! - `max_items` is a result-page cap only. It never caps the
//!   number of valid stored addresses or the total matching
//!   result domain: a page that fills the cap while more matches
//!   remain reports `truncated` and hands out a cursor instead
//!   of silently dropping the remainder.
//! - A query runs against one immutable [`ArraySnapshot`]: the
//!   sorted row set captured under a snapshot token (the blake3
//!   digest of the sorted rows). Pages over one snapshot are
//!   disjoint, ordered slices of that snapshot's matching rows,
//!   ordered by the A9118 typed lexicographic address order
//!   ([`id_core::array_order::cmp_address`], issue #10202).
//! - The cursor is opaque and authenticated (blake3 keyed hash
//!   under a pager-held key). It binds the tenant, the
//!   namespace, the query prefix, the sort profile and the
//!   snapshot token. Replaying it against a different scope,
//!   query or sort profile fails with
//!   [`QueryError::CursorContextChanged`]; replaying it against
//!   a snapshot other than the one it was issued under fails
//!   with [`QueryError::CursorExpired`]. A cursor whose bytes
//!   were tampered with fails authentication before any binding
//!   is read.
//! - The cursor string is untrusted input: a presented cursor
//!   longer than [`MAX_CURSOR_LEN`] is refused as malformed
//!   before any decoding allocates storage for it, and the
//!   pager never issues a cursor longer than that bound.
//! - Cancellation is observable: a [`CancelFlag`] checked while
//!   a page is produced ends the stream with a terminal page
//!   whose reason is [`TerminalReason::Cancelled`], never a
//!   silently short page.
//!
//! The authenticated tenant/namespace context is consumed, not
//! defined, here (#9083): the caller hands in the scope strings
//! its admission path established. The registry read/mutate seam
//! (#10257) is likewise consumed elsewhere; this module owns
//! only the paging core. The crate-root export belongs to slice
//! 9110-02; until that lands, the integration tests include this
//! file by path.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};

use id_core::array_address::ArrayAddress;
use id_core::array_order::cmp_address;
use id_core::canonical_cbor::{Value, decode_canonical, encode_canonical};

/// The one sort profile this slice issues cursors under:
/// ascending A9118 typed lexicographic address order. The
/// profile string is bound into every cursor, so a future
/// profile can never be confused with this one.
pub const SORT_PROFILE: &str = "address-asc/v1";

/// Cursor format version bound into every cursor payload.
const CURSOR_VERSION: u64 = 1;

/// Maximum length, in hex characters, of a cursor string this
/// module issues or accepts. A presented cursor is untrusted
/// input: without a bound, a caller could hand in an arbitrarily
/// long string and make every page request allocate roughly
/// half its size in decoded bytes before authentication refuses
/// it. The sealed form is two hex characters per byte; the
/// payload's fixed fields (version, sort profile, the 32-byte
/// snapshot token and the 32-byte MAC) total well under 200
/// bytes, so this bound leaves roughly 16 KiB of sealed payload
/// for the variable fields — the scope strings and the two
/// bound array addresses — far beyond any scope or address the
/// store admits in practice. [`CursorPager::seal_cursor`]
/// enforces the same bound when issuing, so a pager never hands
/// out a cursor it would later refuse as oversized: the bound
/// is the supported cursor-size limit, enforced consistently on
/// both the issuing and the accepting path.
pub const MAX_CURSOR_LEN: usize = 32_768;

/// A query row: one array address and its stored payload bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryRow {
    address: ArrayAddress,
    payload: Vec<u8>,
}

impl QueryRow {
    /// Build a row from an address and its payload.
    pub fn new(address: ArrayAddress, payload: Vec<u8>) -> Self {
        Self { address, payload }
    }

    /// The row's array address.
    pub fn address(&self) -> &ArrayAddress {
        &self.address
    }

    /// The row's stored payload bytes.
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
}

/// The token identifying one immutable snapshot: the blake3
/// digest of the snapshot's sorted rows. Two snapshots with the
/// same rows in the same order share a token; any difference in
/// membership, order or payload changes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SnapshotToken([u8; 32]);

impl SnapshotToken {
    /// The token's raw digest bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// The token as lowercase hex.
    pub fn to_hex(&self) -> String {
        let mut out = String::with_capacity(64);
        for byte in self.0 {
            out.push_str(&format!("{byte:02x}"));
        }
        out
    }
}

impl fmt::Display for SnapshotToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// An immutable, sorted set of rows captured as one snapshot.
/// Construction sorts by the A9118 address order and rejects a
/// duplicate address: a snapshot is a store view, and one
/// address holds one payload in it.
#[derive(Clone, Debug)]
pub struct ArraySnapshot {
    rows: Vec<QueryRow>,
    token: SnapshotToken,
}

impl ArraySnapshot {
    /// Capture `rows` as a snapshot, sorting by address.
    ///
    /// # Errors
    /// [`QueryError::DuplicateAddress`] if two rows share an
    /// address.
    pub fn new(mut rows: Vec<QueryRow>) -> Result<Self, QueryError> {
        rows.sort_by(|a, b| cmp_address(a.address.slots(), b.address.slots()));
        for pair in rows.windows(2) {
            if pair[0].address == pair[1].address {
                return Err(QueryError::DuplicateAddress);
            }
        }
        let token = SnapshotToken(digest_rows(&rows));
        Ok(Self { rows, token })
    }

    /// This snapshot's token.
    pub fn token(&self) -> SnapshotToken {
        self.token
    }

    /// The snapshot's rows, in ascending address order.
    pub fn rows(&self) -> &[QueryRow] {
        &self.rows
    }

    /// The number of rows in the snapshot.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether the snapshot holds no rows.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

fn digest_rows(rows: &[QueryRow]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    for row in rows {
        let address = row.address.encode();
        hasher.update(&(address.len() as u64).to_le_bytes());
        hasher.update(&address);
        hasher.update(&(row.payload.len() as u64).to_le_bytes());
        hasher.update(&row.payload);
    }
    *hasher.finalize().as_bytes()
}

/// The authenticated scope a query runs in: the tenant and
/// namespace strings the caller's admission path established
/// (#9083). Consumed here, never defined here; both are bound
/// into every cursor the query issues.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryScope {
    tenant: String,
    namespace: String,
}

impl QueryScope {
    /// Build a scope from the authenticated tenant and
    /// namespace.
    pub fn new(tenant: impl Into<String>, namespace: impl Into<String>) -> Self {
        Self {
            tenant: tenant.into(),
            namespace: namespace.into(),
        }
    }

    /// The tenant scope string.
    pub fn tenant(&self) -> &str {
        &self.tenant
    }

    /// The namespace string.
    pub fn namespace(&self) -> &str {
        &self.namespace
    }
}

/// A query over array addresses: every row whose address has
/// this address as a slotwise prefix matches. The empty address
/// matches the whole snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArrayQuery {
    prefix: ArrayAddress,
}

impl ArrayQuery {
    /// Build a query from its prefix address.
    pub fn new(prefix: ArrayAddress) -> Self {
        Self { prefix }
    }

    /// The query's prefix address.
    pub fn prefix(&self) -> &ArrayAddress {
        &self.prefix
    }

    fn matches(&self, address: &ArrayAddress) -> bool {
        let prefix = self.prefix.slots();
        let slots = address.slots();
        slots.len() >= prefix.len() && slots[..prefix.len()] == prefix[..]
    }
}

/// Why a result stream ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalReason {
    /// Every matching row has been delivered.
    Completed,
    /// The caller cancelled; delivery stopped early.
    Cancelled,
}

/// One page of a query result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResultPage {
    rows: Vec<QueryRow>,
    next_cursor: Option<Cursor>,
    truncated: bool,
    snapshot: SnapshotToken,
    terminal: Option<TerminalReason>,
}

impl ResultPage {
    /// The page's rows, in ascending address order.
    pub fn rows(&self) -> &[QueryRow] {
        &self.rows
    }

    /// The cursor for the next page, present exactly when the
    /// stream has not terminated and more matches remain.
    pub fn next_cursor(&self) -> Option<&Cursor> {
        self.next_cursor.as_ref()
    }

    /// Whether the matching result domain had more rows than
    /// this page delivered — the page was cut by the cap or by
    /// cancellation, never a silent drop.
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    /// The token of the snapshot this page was read from.
    pub fn snapshot(&self) -> SnapshotToken {
        self.snapshot
    }

    /// The terminal reason, present on the final page only.
    pub fn terminal(&self) -> Option<TerminalReason> {
        self.terminal
    }
}

/// An opaque, authenticated paging cursor. The string form is
/// the whole cursor; it carries no readable state and verifies
/// only under the key of the [`CursorPager`] that issued it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cursor(String);

impl Cursor {
    /// Wrap a cursor string received from a caller. This does
    /// not validate anything: authentication and binding are
    /// checked when the cursor is presented to
    /// [`CursorPager::next_page`], and only there.
    pub fn from_string(s: String) -> Self {
        Self(s)
    }

    /// The cursor's opaque string form.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Cursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A shared cancellation flag. Clones observe the same flag;
/// setting it ends the next page production with
/// [`TerminalReason::Cancelled`].
#[derive(Clone, Debug, Default)]
pub struct CancelFlag(Arc<AtomicBool>);

impl CancelFlag {
    /// A new, unset flag.
    pub fn new() -> Self {
        Self::default()
    }

    /// Cancel: subsequent page production terminates.
    pub fn cancel(&self) {
        self.0.store(true, AtomicOrdering::SeqCst);
    }

    /// Whether cancellation was requested.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(AtomicOrdering::SeqCst)
    }
}

/// The paging core. Holds the cursor authentication key; every
/// cursor it issues verifies only under that key.
#[derive(Clone, Debug)]
pub struct CursorPager {
    key: [u8; 32],
}

impl CursorPager {
    /// Build a pager from its 32-byte cursor key.
    pub fn new(key: [u8; 32]) -> Self {
        Self { key }
    }

    /// Produce the first page of `query` over `snapshot`.
    ///
    /// # Errors
    /// [`QueryError::InvalidMaxItems`] when `max_items` is 0.
    pub fn first_page(
        &self,
        scope: &QueryScope,
        query: &ArrayQuery,
        snapshot: &ArraySnapshot,
        max_items: usize,
        cancel: Option<&CancelFlag>,
    ) -> Result<ResultPage, QueryError> {
        if max_items == 0 {
            return Err(QueryError::InvalidMaxItems);
        }
        self.page(scope, query, snapshot, None, max_items, cancel)
    }

    /// Produce the page after `cursor`.
    ///
    /// # Errors
    /// [`QueryError::InvalidMaxItems`] when `max_items` is 0;
    /// [`QueryError::MalformedCursor`] or
    /// [`QueryError::UnauthenticatedCursor`] when the cursor
    /// does not decode or verify;
    /// [`QueryError::CursorContextChanged`] when the cursor was
    /// issued under another scope, query or sort profile;
    /// [`QueryError::CursorExpired`] when it was issued under a
    /// different snapshot.
    pub fn next_page(
        &self,
        scope: &QueryScope,
        query: &ArrayQuery,
        snapshot: &ArraySnapshot,
        cursor: &Cursor,
        max_items: usize,
        cancel: Option<&CancelFlag>,
    ) -> Result<ResultPage, QueryError> {
        if max_items == 0 {
            return Err(QueryError::InvalidMaxItems);
        }
        let bound = self.open_cursor(cursor)?;
        if bound.tenant != scope.tenant
            || bound.namespace != scope.namespace
            || bound.query != query.prefix.encode()
            || bound.sort != SORT_PROFILE
        {
            return Err(QueryError::CursorContextChanged);
        }
        if bound.snapshot != snapshot.token.0 {
            return Err(QueryError::CursorExpired);
        }
        let after = ArrayAddress::decode(&bound.after).map_err(|_| QueryError::MalformedCursor)?;
        self.page(scope, query, snapshot, Some(&after), max_items, cancel)
    }

    fn page(
        &self,
        scope: &QueryScope,
        query: &ArrayQuery,
        snapshot: &ArraySnapshot,
        after: Option<&ArrayAddress>,
        max_items: usize,
        cancel: Option<&CancelFlag>,
    ) -> Result<ResultPage, QueryError> {
        let mut rows: Vec<QueryRow> = Vec::new();
        let mut truncated = false;
        let mut cancelled = false;
        for row in snapshot.rows() {
            if !query.matches(&row.address) {
                continue;
            }
            if let Some(after) = after {
                if cmp_address(row.address.slots(), after.slots()) != std::cmp::Ordering::Greater {
                    continue;
                }
            }
            if rows.len() >= max_items {
                truncated = true;
                break;
            }
            if cancel.is_some_and(CancelFlag::is_cancelled) {
                truncated = true;
                cancelled = true;
                break;
            }
            rows.push(row.clone());
        }
        let terminal = if cancelled {
            Some(TerminalReason::Cancelled)
        } else if truncated {
            None
        } else {
            Some(TerminalReason::Completed)
        };
        let next_cursor = if terminal.is_none() {
            match rows.last() {
                Some(last) => Some(self.seal_cursor(scope, query, snapshot, last.address())?),
                None => None,
            }
        } else {
            None
        };
        Ok(ResultPage {
            rows,
            next_cursor,
            truncated,
            snapshot: snapshot.token,
            terminal,
        })
    }

    fn seal_cursor(
        &self,
        scope: &QueryScope,
        query: &ArrayQuery,
        snapshot: &ArraySnapshot,
        after: &ArrayAddress,
    ) -> Result<Cursor, QueryError> {
        let payload = encode_canonical(&Value::Map(vec![
            (text("v"), Value::UInt(CURSOR_VERSION)),
            (text("tenant"), text(&scope.tenant)),
            (text("namespace"), text(&scope.namespace)),
            (text("query"), Value::Bytes(query.prefix.encode())),
            (text("sort"), text(SORT_PROFILE)),
            (text("snapshot"), Value::Bytes(snapshot.token.0.to_vec())),
            (text("after"), Value::Bytes(after.encode())),
        ]))
        .map_err(|_| QueryError::CursorEncode)?;
        let mac = blake3::keyed_hash(&self.key, &payload);
        let mut sealed = payload;
        sealed.extend_from_slice(mac.as_bytes());
        let encoded = hex_encode(&sealed);
        if encoded.len() > MAX_CURSOR_LEN {
            return Err(QueryError::CursorEncode);
        }
        Ok(Cursor(encoded))
    }

    fn open_cursor(&self, cursor: &Cursor) -> Result<BoundCursor, QueryError> {
        if cursor.as_str().len() > MAX_CURSOR_LEN {
            return Err(QueryError::MalformedCursor);
        }
        let sealed = hex_decode(cursor.as_str()).ok_or(QueryError::MalformedCursor)?;
        if sealed.len() <= 32 {
            return Err(QueryError::MalformedCursor);
        }
        let (payload, mac) = sealed.split_at(sealed.len() - 32);
        let expected = blake3::keyed_hash(&self.key, payload);
        if mac != expected.as_bytes() {
            return Err(QueryError::UnauthenticatedCursor);
        }
        let value = decode_canonical(payload).map_err(|_| QueryError::MalformedCursor)?;
        let Value::Map(entries) = value else {
            return Err(QueryError::MalformedCursor);
        };
        let field = |name: &str| -> Result<&Value, QueryError> {
            entries
                .iter()
                .find_map(|(key, value)| {
                    if key == &Value::Text(name.to_string()) {
                        Some(value)
                    } else {
                        None
                    }
                })
                .ok_or(QueryError::MalformedCursor)
        };
        let version = match field("v")? {
            Value::UInt(n) => *n,
            _ => return Err(QueryError::MalformedCursor),
        };
        if version != CURSOR_VERSION {
            return Err(QueryError::MalformedCursor);
        }
        let text_field = |name: &str| -> Result<String, QueryError> {
            match field(name)? {
                Value::Text(s) => Ok(s.clone()),
                _ => Err(QueryError::MalformedCursor),
            }
        };
        let bytes_field = |name: &str| -> Result<Vec<u8>, QueryError> {
            match field(name)? {
                Value::Bytes(b) => Ok(b.clone()),
                _ => Err(QueryError::MalformedCursor),
            }
        };
        let snapshot_bytes = bytes_field("snapshot")?;
        let snapshot: [u8; 32] = snapshot_bytes
            .try_into()
            .map_err(|_| QueryError::MalformedCursor)?;
        Ok(BoundCursor {
            tenant: text_field("tenant")?,
            namespace: text_field("namespace")?,
            query: bytes_field("query")?,
            sort: text_field("sort")?,
            snapshot,
            after: bytes_field("after")?,
        })
    }
}

struct BoundCursor {
    tenant: String,
    namespace: String,
    query: Vec<u8>,
    sort: String,
    snapshot: [u8; 32],
    after: Vec<u8>,
}

fn text(s: &str) -> Value {
    Value::Text(s.to_string())
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(s.len() / 2);
    for pair in bytes.chunks_exact(2) {
        let hi = hex_nibble(pair[0])?;
        let lo = hex_nibble(pair[1])?;
        out.push((hi << 4) | lo);
    }
    Some(out)
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Everything that can refuse a page or a snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueryError {
    /// `max_items` was 0: a page cap must admit at least one
    /// row, and a zero cap is a caller bug, not an empty result.
    InvalidMaxItems,
    /// Two rows in a snapshot shared one address.
    DuplicateAddress,
    /// The cursor string does not decode to a cursor payload.
    MalformedCursor,
    /// The cursor's authentication tag does not verify under
    /// this pager's key.
    UnauthenticatedCursor,
    /// The cursor was issued under a different tenant,
    /// namespace, query or sort profile (`cursor_context_changed`).
    CursorContextChanged,
    /// The cursor was issued under a different snapshot than
    /// the one presented (`cursor_expired`).
    CursorExpired,
    /// The cursor payload could not be canonically encoded.
    CursorEncode,
}

impl fmt::Display for QueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidMaxItems => "max_items must be at least 1",
            Self::DuplicateAddress => "snapshot rows contain a duplicate address",
            Self::MalformedCursor => "cursor is malformed",
            Self::UnauthenticatedCursor => "cursor authentication failed",
            Self::CursorContextChanged => "cursor_context_changed",
            Self::CursorExpired => "cursor_expired",
            Self::CursorEncode => "cursor encoding failed",
        };
        f.write_str(message)
    }
}

impl std::error::Error for QueryError {}
