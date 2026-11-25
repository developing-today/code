//! Live and materialized read paths (design #5805, §5.2–§5.4; slice
//! #13763).
//!
//! This module drives the translator's [`PostgrestRequest`]s through a
//! [`Transport`] and turns the pages that come back into results with
//! honest metadata. It is pure: the transport and the row decoder are
//! traits, so every rule here is testable without a network or a typed
//! mapping. The HTTP client (`reqwest` or similar) implements
//! [`Transport`]; the typed mapping (§5.1, #13761) implements
//! [`PageDecoder`].
//!
//! What the module guarantees:
//!
//! - **Owner mode choice (§5.4).** A collection is read only in the mode
//!   the owner picked for it in [`ModeChoices`]. Until the owner picks,
//!   the collection is unavailable ([`ReadError::ModeNotChosen`]). There
//!   is no default.
//! - **Live reads (§5.2).** One request per page. Every [`LiveResult`]
//!   states [`Consistency::PerRequest`] and the observation time of the
//!   last page, and says whether the result is complete. A page is
//!   complete when `Content-Range` says so or when it holds fewer rows
//!   than the limit; otherwise the result is marked `truncated`. A failed
//!   page is an error, never a short success, and nothing falls back to a
//!   snapshot.
//! - **Materialized snapshots (§5.3).** [`SnapshotStore::rebuild`] keyset
//!   pages over the whole allowed set under [`SnapshotCaps`]. The caps are
//!   checked before a page's rows are copied; exceeding one is
//!   [`ReadError::SnapshotTooLarge`] and no partial snapshot is kept. The
//!   stored snapshot is replaced only when every page succeeded, so an
//!   interrupted build leaves the previous complete snapshot in place. A
//!   snapshot's consistency is [`Consistency::PerRequest`]: `PostgREST`
//!   cannot pin one database snapshot across requests, so a snapshot is
//!   never described as atomic or point-in-time.
//! - **No write path (§9 invariant 10).** [`Transport`] has one method,
//!   [`Transport::get`], and it only takes a translator-built
//!   [`PostgrestRequest`], which is always a `GET`.
//! - **No silent errors (§9 invariant 8).** A deadline, a connection
//!   failure, a status outside 2xx, or a 2xx that is not JSON is an
//!   error, never an empty result.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::allowlist::CollectionAllowlist;
use crate::query::{OrderKey, Pagination, Query, Target};
use crate::refusal::Refusal;
use crate::registration::RegisteredEndpoint;
use crate::translate::{PostgrestRequest, effective_ordering};
use crate::value::Value;

/// How the owner chose to serve one collection (§5.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadMode {
    /// Every read goes to the source (§5.2). The recommended choice for
    /// Postgres.
    Live,
    /// Reads are served from a snapshot built by keyset paging (§5.3).
    Materialized,
}

impl ReadMode {
    /// The mode's stable name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Materialized => "materialized",
        }
    }
}

/// The owner's per-collection mode choices (§5.4).
///
/// An empty set is the starting state: nothing has been chosen, so
/// nothing is readable. The adapter never fills in a mode on its own.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModeChoices {
    /// Relation name → the mode the owner picked.
    pub modes: BTreeMap<String, ReadMode>,
}

impl ModeChoices {
    /// No choices yet: every collection is unavailable.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the owner's choice for one collection, replacing any
    /// earlier choice.
    pub fn choose(&mut self, relation: &str, mode: ReadMode) {
        self.modes.insert(relation.to_owned(), mode);
    }

    /// The owner's choice for `relation`, or
    /// [`ReadError::ModeNotChosen`] when there is none.
    pub fn mode_for(&self, relation: &str) -> Result<ReadMode, ReadError> {
        self.modes
            .get(relation)
            .copied()
            .ok_or(ReadError::ModeNotChosen)
    }
}

/// The consistency boundary a result can claim.
///
/// `PostgREST` cannot pin one database snapshot across requests, so the
/// only value is [`Consistency::PerRequest`]: each page observes its own
/// database state. There is deliberately no "atomic" or "snapshot"
/// variant (§5.2, §5.3, §9 invariant 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Consistency {
    /// Each request observes its own database state; two pages of one
    /// read can observe different states.
    PerRequest,
}

impl Consistency {
    /// The stable name reported in responses and provenance.
    pub const fn name(self) -> &'static str {
        match self {
            Self::PerRequest => "per_request",
        }
    }
}

/// What the transport got back for one request.
///
/// The transport fills only these fields. It drops every other response
/// header, so nothing the source echoes (including a credential) can
/// reach a result or an error (§9 invariant 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    /// The HTTP status.
    pub status: u16,
    /// The `Content-Type` header, if any.
    pub content_type: Option<String>,
    /// The `Content-Range` header, if any.
    pub content_range: Option<String>,
    /// The response body.
    pub body: Vec<u8>,
    /// When the response was received, in milliseconds since the Unix
    /// epoch.
    pub observed_at_ms: u64,
}

/// A transport failure that produced no response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportError {
    /// The per-request deadline passed.
    Deadline,
    /// The connection failed (DNS, TCP, TLS, or a reset).
    Connection,
}

/// The HTTP side of the adapter.
///
/// It has exactly one operation, a `GET` built by the translator, so a
/// write cannot be expressed through it (§9 invariant 10). An
/// implementation sends `request.accept_profile` as `Accept-Profile`,
/// resolves the endpoint's credential just before the call, and never
/// puts the credential in the URL (§2, §9 invariants 2 and 4).
pub trait Transport {
    /// Send one `GET` and return the response, or the failure that
    /// stopped it.
    fn get(
        &mut self,
        endpoint: &RegisteredEndpoint,
        request: &PostgrestRequest,
    ) -> Result<HttpResponse, TransportError>;
}

/// Why a page body could not be decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// The body is not a JSON array of row objects.
    NotJson,
    /// A value failed its declared type (§5.1). The value itself is never
    /// kept (§9 invariant 6).
    TypeMapping {
        /// The declared column.
        column: String,
        /// The column's declared type.
        declared_type: String,
    },
}

/// Turns a page body into rows, and a row into its keyset cursor.
///
/// The typed mapping (§5.1, #13761) implements this.
pub trait PageDecoder {
    /// One decoded row.
    type Row;

    /// Decode one page body: a JSON array of row objects.
    fn decode(&self, body: &[u8]) -> Result<Vec<Self::Row>, DecodeError>;

    /// The keyset cursor for `row`: one value per key of `ordering`, in
    /// order. The cursor of a page's last row is where the next page
    /// starts.
    fn cursor(&self, row: &Self::Row, ordering: &[OrderKey]) -> Result<Vec<Value>, DecodeError>;
}

/// Why a read failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    /// The translator refused the query before any network call.
    Refused(Refusal),
    /// The owner has not picked a mode for the collection (§5.4).
    ModeNotChosen,
    /// The collection is served in the other mode: a live read of a
    /// materialized collection, or a snapshot of a live one.
    WrongMode {
        /// The mode the owner picked.
        chosen: ReadMode,
    },
    /// The query does not address a collection.
    NotACollection,
    /// `PostgREST` answered with a status outside 2xx.
    Status(u16),
    /// `PostgREST` answered 2xx, but the body is not JSON.
    NonJsonBody,
    /// The per-request deadline passed.
    Deadline,
    /// The connection failed.
    Connection,
    /// The page broke the protocol: more rows than the limit, or a
    /// `Content-Range` that cannot be parsed.
    MalformedPage,
    /// A value failed its declared type.
    TypeMapping {
        /// The declared column.
        column: String,
        /// The column's declared type.
        declared_type: String,
    },
    /// A snapshot build would exceed the row or byte cap (§5.3). No
    /// partial snapshot was kept.
    SnapshotTooLarge,
}

impl ReadError {
    /// The stable code for the owner's view (§6.1). The grantee collapse
    /// (§6.2) is applied on top of this by the error mapping (#13762).
    pub fn code(&self) -> &'static str {
        match self {
            Self::Refused(refusal) => refusal.code.name(),
            Self::ModeNotChosen | Self::WrongMode { .. } | Self::NotACollection => "not_available",
            Self::Status(401 | 403) => "credential_unavailable",
            Self::Status(429) => "rate_limited",
            Self::Status(status) if *status >= 400 && *status < 500 => "not_available",
            Self::Status(_) | Self::NonJsonBody | Self::Connection | Self::MalformedPage => {
                "adapter_unavailable"
            }
            Self::Deadline => "timeout",
            Self::TypeMapping { .. } => "type_mapping_failed",
            Self::SnapshotTooLarge => "snapshot_too_large",
        }
    }
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(refusal) => write!(f, "{refusal}"),
            Self::ModeNotChosen => f.write_str("the owner has not chosen a read mode"),
            Self::WrongMode { chosen } => {
                write!(f, "the collection is served in {} mode", chosen.name())
            }
            Self::NotACollection => f.write_str("the query does not address a collection"),
            Self::Status(status) => write!(f, "the source answered HTTP {status}"),
            Self::NonJsonBody => f.write_str("the source answered with a non-JSON body"),
            Self::Deadline => f.write_str("the request deadline passed"),
            Self::Connection => f.write_str("the connection to the source failed"),
            Self::MalformedPage => f.write_str("the source returned a malformed page"),
            Self::TypeMapping {
                column,
                declared_type,
            } => write!(
                f,
                "column {column:?} failed its declared type {declared_type}"
            ),
            Self::SnapshotTooLarge => f.write_str("the snapshot would exceed its declared caps"),
        }
    }
}

impl std::error::Error for ReadError {}

impl From<Refusal> for ReadError {
    fn from(refusal: Refusal) -> Self {
        Self::Refused(refusal)
    }
}

impl From<TransportError> for ReadError {
    fn from(error: TransportError) -> Self {
        match error {
            TransportError::Deadline => Self::Deadline,
            TransportError::Connection => Self::Connection,
        }
    }
}

impl From<DecodeError> for ReadError {
    fn from(error: DecodeError) -> Self {
        match error {
            DecodeError::NotJson => Self::NonJsonBody,
            DecodeError::TypeMapping {
                column,
                declared_type,
            } => Self::TypeMapping {
                column,
                declared_type,
            },
        }
    }
}

/// What a `Content-Range` header says about a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentRange {
    /// `*/<total>` or `*/*`: the page is empty.
    Empty {
        /// The total, when the source reported one.
        total: Option<u64>,
    },
    /// `<first>-<last>/<total>` or `<first>-<last>/*`.
    Rows {
        /// The first row index, inclusive.
        first: u64,
        /// The last row index, inclusive.
        last: u64,
        /// The total, when the source reported one.
        total: Option<u64>,
    },
}

impl ContentRange {
    /// Parse a `PostgREST` `Content-Range` value. An optional leading
    /// unit (`items `) is accepted. Anything else is `None`.
    pub fn parse(header: &str) -> Option<Self> {
        let header = header.trim();
        let header = header.split_once(' ').map_or(header, |(_unit, rest)| rest);
        let (range, total) = header.split_once('/')?;
        let total = match total {
            "*" => None,
            n => Some(n.parse::<u64>().ok()?),
        };
        if range == "*" {
            return Some(Self::Empty { total });
        }
        let (first, last) = range.split_once('-')?;
        let first = first.parse::<u64>().ok()?;
        let last = last.parse::<u64>().ok()?;
        if last < first {
            return None;
        }
        Some(Self::Rows { first, last, total })
    }

    /// Whether the range says the result ends with this page. `None`
    /// means the range does not say (the total is unknown).
    pub const fn says_complete(self) -> Option<bool> {
        match self {
            Self::Empty { .. } => Some(true),
            Self::Rows {
                last,
                total: Some(total),
                ..
            } => Some(last.saturating_add(1) >= total),
            Self::Rows { total: None, .. } => None,
        }
    }
}

/// Whether a media type is JSON (`application/json`,
/// `application/vnd.pgrst.object+json`, with or without parameters).
fn is_json_media_type(content_type: &str) -> bool {
    let media = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    media == "application/json" || (media.starts_with("application/") && media.ends_with("+json"))
}

/// One decoded page and what it says about the rest of the result.
#[derive(Debug, Clone, PartialEq)]
struct Page<Row> {
    rows: Vec<Row>,
    complete: bool,
    body_bytes: u64,
    observed_at_ms: u64,
}

/// Send one request and decode the page, or fail. Never a short success.
fn fetch_page<T: Transport, D: PageDecoder>(
    endpoint: &RegisteredEndpoint,
    transport: &mut T,
    decoder: &D,
    request: &PostgrestRequest,
    limit: u32,
) -> Result<Page<D::Row>, ReadError> {
    let response = transport.get(endpoint, request)?;
    if !(200..=299).contains(&response.status) {
        return Err(ReadError::Status(response.status));
    }
    if !response
        .content_type
        .as_deref()
        .is_some_and(is_json_media_type)
    {
        return Err(ReadError::NonJsonBody);
    }
    let range = match response.content_range.as_deref() {
        Some(header) => Some(ContentRange::parse(header).ok_or(ReadError::MalformedPage)?),
        None => None,
    };
    let rows = decoder.decode(&response.body)?;
    let row_count = u64::try_from(rows.len())
        .ok()
        .ok_or(ReadError::MalformedPage)?;
    if row_count > u64::from(limit) {
        return Err(ReadError::MalformedPage);
    }
    let complete = range.and_then(ContentRange::says_complete).unwrap_or(false)
        || row_count < u64::from(limit);
    Ok(Page {
        rows,
        complete,
        body_bytes: u64::try_from(response.body.len())
            .ok()
            .ok_or(ReadError::MalformedPage)?,
        observed_at_ms: response.observed_at_ms,
    })
}

/// The collection a query addresses, as registered.
fn collection_for<'e>(
    endpoint: &'e RegisteredEndpoint,
    query: &Query,
) -> Result<&'e CollectionAllowlist, ReadError> {
    let Target::Collection(relation) = &query.target else {
        return Err(ReadError::NotACollection);
    };
    endpoint
        .collections()
        .iter()
        .find(|c| &c.relation == relation)
        .ok_or_else(|| ReadError::Refused(Refusal::not_available()))
}

/// The next page's query: the same query with the cursor of the last
/// row.
fn next_query<D: PageDecoder>(
    query: &Query,
    decoder: &D,
    last: &D::Row,
    ordering: &[OrderKey],
) -> Result<Query, ReadError> {
    let mut next = query.clone();
    next.pagination = Pagination::Keyset {
        cursor: decoder.cursor(last, ordering)?,
    };
    Ok(next)
}

/// The result of a live read (§5.2).
#[derive(Debug, Clone, PartialEq)]
pub struct LiveResult<Row> {
    /// The rows, in the effective ordering.
    pub rows: Vec<Row>,
    /// Always [`Consistency::PerRequest`].
    pub consistency: Consistency,
    /// When the last page was received, in milliseconds since the Unix
    /// epoch.
    pub observed_at_ms: u64,
    /// The number of requests sent: one per page.
    pub pages: u32,
    /// Whether the rows are the whole result.
    pub complete: bool,
    /// The explicit truncation marker: `true` whenever `complete` is
    /// `false`.
    pub truncated: bool,
    /// Where to continue a truncated read: the keyset cursor after the
    /// last row. `None` when the result is complete.
    pub next_cursor: Option<Vec<Value>>,
}

/// Read a live collection (§5.2): send the query, then follow the keyset
/// cursor for at most `max_pages` pages in total, one request per page.
///
/// Every failure is an error; there is no short success and no fallback
/// to a snapshot. When `max_pages` runs out first, the result is marked
/// `truncated` and carries the cursor to continue from.
pub fn live_read<T: Transport, D: PageDecoder>(
    endpoint: &RegisteredEndpoint,
    modes: &ModeChoices,
    transport: &mut T,
    decoder: &D,
    query: &Query,
    max_pages: u32,
) -> Result<LiveResult<D::Row>, ReadError> {
    let translator = endpoint.translator();
    // Refusals come first and need no mode: they depend on the query text
    // alone, and nothing is sent until the query passes (§9 invariant 1).
    let mut request = translator.translate(query)?;
    let collection = collection_for(endpoint, query)?;
    match modes.mode_for(&collection.relation)? {
        ReadMode::Live => {}
        chosen @ ReadMode::Materialized => return Err(ReadError::WrongMode { chosen }),
    }
    let limit = query
        .limit
        .unwrap_or_else(|| collection.effective_max_limit());
    let ordering = effective_ordering(collection, &query.order);
    let max_pages = max_pages.max(1);

    let mut current = query.clone();
    let mut rows: Vec<D::Row> = Vec::new();
    let mut pages = 0_u32;
    loop {
        let page = fetch_page(endpoint, transport, decoder, &request, limit)?;
        pages = pages.saturating_add(1);
        let observed_at_ms = page.observed_at_ms;
        let complete = page.complete;
        rows.extend(page.rows);
        let next_cursor = match rows.last() {
            _ if complete => None,
            Some(last) => {
                current = next_query(&current, decoder, last, &ordering)?;
                match &current.pagination {
                    Pagination::Keyset { cursor } => Some(cursor.clone()),
                    _ => None,
                }
            }
            // Not complete and no rows: only a zero limit gets here. There
            // is no cursor to continue from, and the result is still
            // marked truncated rather than claimed complete.
            None => None,
        };
        if complete || next_cursor.is_none() || pages >= max_pages {
            return Ok(LiveResult {
                rows,
                consistency: Consistency::PerRequest,
                observed_at_ms,
                pages,
                complete,
                truncated: !complete,
                next_cursor,
            });
        }
        request = translator.translate(&current)?;
    }
}

/// The declared caps on one materialized snapshot (§5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotCaps {
    /// The most rows the snapshot may hold.
    pub max_rows: u64,
    /// The most response-body bytes the build may copy, summed over pages.
    pub max_bytes: u64,
}

/// A complete materialized snapshot of one collection (§5.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot<Row> {
    /// Every allowed row, in the collection's unique-key order.
    pub rows: Vec<Row>,
    /// Always [`Consistency::PerRequest`]: the pages were read in
    /// separate requests, so the snapshot is not a point-in-time copy.
    pub consistency: Consistency,
    /// When the first page was received, in milliseconds since the Unix
    /// epoch.
    pub first_observed_at_ms: u64,
    /// When the last page was received.
    pub last_observed_at_ms: u64,
    /// The number of requests the build sent.
    pub pages: u32,
    /// The response-body bytes the build copied.
    pub bytes: u64,
}

/// Holds the current complete snapshot of one collection, if any.
///
/// [`SnapshotStore::rebuild`] replaces it only when a whole build
/// succeeded. A failed or oversized build leaves it untouched (§5.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotStore<Row> {
    relation: String,
    current: Option<Snapshot<Row>>,
}

impl<Row> SnapshotStore<Row> {
    /// An empty store for one collection.
    pub fn new(relation: &str) -> Self {
        Self {
            relation: relation.to_owned(),
            current: None,
        }
    }

    /// The collection this store holds.
    pub const fn relation(&self) -> &str {
        self.relation.as_str()
    }

    /// The last complete snapshot, if a build has ever succeeded.
    pub const fn current(&self) -> Option<&Snapshot<Row>> {
        self.current.as_ref()
    }

    /// Build a fresh snapshot by keyset paging over the whole allowed
    /// set, then swap it in.
    ///
    /// Each page's row count and body size are checked against `caps`
    /// before its rows are copied; exceeding either is
    /// [`ReadError::SnapshotTooLarge`]. Any error discards the partial
    /// build and keeps the previous snapshot.
    pub fn rebuild<T: Transport, D: PageDecoder<Row = Row>>(
        &mut self,
        endpoint: &RegisteredEndpoint,
        modes: &ModeChoices,
        transport: &mut T,
        decoder: &D,
        caps: SnapshotCaps,
    ) -> Result<&Snapshot<Row>, ReadError> {
        let snapshot = build_snapshot(endpoint, modes, transport, decoder, &self.relation, caps)?;
        Ok(self.current.insert(snapshot))
    }
}

/// Page through the whole collection into a new snapshot, or fail.
fn build_snapshot<T: Transport, D: PageDecoder>(
    endpoint: &RegisteredEndpoint,
    modes: &ModeChoices,
    transport: &mut T,
    decoder: &D,
    relation: &str,
    caps: SnapshotCaps,
) -> Result<Snapshot<D::Row>, ReadError> {
    let translator = endpoint.translator();
    let mut query = Query::read(relation);
    let mut request = translator.translate(&query)?;
    let collection = collection_for(endpoint, &query)?;
    match modes.mode_for(&collection.relation)? {
        ReadMode::Materialized => {}
        chosen @ ReadMode::Live => return Err(ReadError::WrongMode { chosen }),
    }
    let limit = collection.effective_max_limit();
    let ordering = effective_ordering(collection, &query.order);

    let mut rows: Vec<D::Row> = Vec::new();
    let mut row_count = 0_u64;
    let mut bytes = 0_u64;
    let mut pages = 0_u32;
    let mut first_observed_at_ms = None;
    loop {
        let page = fetch_page(endpoint, transport, decoder, &request, limit)?;
        pages = pages.saturating_add(1);
        let page_rows = u64::try_from(page.rows.len())
            .ok()
            .ok_or(ReadError::MalformedPage)?;
        // The caps are checked before this page's rows are copied.
        let next_rows = row_count.saturating_add(page_rows);
        let next_bytes = bytes.saturating_add(page.body_bytes);
        if next_rows > caps.max_rows || next_bytes > caps.max_bytes {
            return Err(ReadError::SnapshotTooLarge);
        }
        row_count = next_rows;
        bytes = next_bytes;
        let first = *first_observed_at_ms.get_or_insert(page.observed_at_ms);
        let complete = page.complete;
        let last_observed_at_ms = page.observed_at_ms;
        rows.extend(page.rows);
        if complete {
            return Ok(Snapshot {
                rows,
                consistency: Consistency::PerRequest,
                first_observed_at_ms: first,
                last_observed_at_ms,
                pages,
                bytes,
            });
        }
        // An incomplete page always holds at least one row here, because a
        // registered collection's limit is at least 1; anything else is a
        // protocol break, not an empty success.
        let last = rows.last().ok_or(ReadError::MalformedPage)?;
        query = next_query(&query, decoder, last, &ordering)?;
        request = translator.translate(&query)?;
    }
}
