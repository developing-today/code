//! Resumable stream subscription and completed-target resolution
//! (slice A9123-stream-subscribe, issue #10282; parent #9123; design
//! `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md`
//! section "#9123 — addressable live streams", A-9123.1, statements
//! S4.79–S4.82).
//!
//! The write side — signed descriptor, chained signed chunk records,
//! the lifecycle machine — belongs to A9123-stream-records
//! (`stream_record`, issue #10246). This slice is the read side: a
//! consumer subscribes to a stream the chain makes addressable
//! before it finishes (S4.79), replays the retained chunks, follows
//! the live head, and resumes exactly where it left off.
//!
//! The rules this module pins:
//!
//! - The resume cursor is sequence + chain-record ID, never a
//!   timestamp. A [`StreamCursor`] at sequence `n` names chunk
//!   `n - 1`'s record ID; presenting it resumes with the exact
//!   suffix `n..head`. A cursor whose record ID does not match the
//!   chain at that sequence is rejected
//!   ([`SubscribeError::CursorMismatch`]) — it is a forgery or
//!   equivocation evidence, never silently re-anchored. A cursor
//!   past the head is [`SubscribeError::CursorAhead`]; a cursor
//!   below the retention floor is [`SubscribeError::CursorExpired`].
//! - Delivery is paged (the A9119 max-items discipline, issue
//!   #10274, applied to the chain): one [`StreamSubscription::poll`]
//!   returns at most `max_items` chunk events, reports `truncated`
//!   while more remain, and hands back the next cursor. At the head
//!   of a live stream a poll returns a heartbeat carrying the
//!   watermark cursor and the current state. Cancellation is
//!   observable: after [`StreamSubscription::cancel`], the next
//!   poll returns one terminal page whose outcome is
//!   [`TerminalOutcome::Cancelled`], never a silently short page.
//! - A terminal stream delivers its remaining chunks first; the
//!   terminal outcome (`completed` / `aborted` / `expired`) is the
//!   last event of the last page. Aborts and expiry claim no final
//!   content.
//! - A finished stream resolves via the typed
//!   [`ResolutionView::Completed`] (S4.80, S4.81): the final
//!   reference is data the caller explicitly chooses to follow —
//!   there is no redirect type in this module, so no implicit
//!   HTTP-style redirect can exist. Resolution re-checks the
//!   *current* authorization result on every call: the caller hands
//!   in the authorization its admission path currently computes for
//!   the subscriber context (#9083 — consumed here, never defined),
//!   and a revoked context is refused the final reference
//!   ([`SubscribeError::Unauthorized`]) even though it could read
//!   the stream while authorized. Authorization is likewise
//!   re-checked on subscribe, resume and every poll.
//! - The records a subscription folds are validated against the
//!   lifecycle machine's accepted chain before anything is
//!   delivered: each retained record must be the chain's record at
//!   its sequence (record ID, previous-record link, stream and
//!   tenant-scope binding all re-checked, signature re-verified).
//!   A record that does not fold is [`SubscribeError::ChainMismatch`]
//!   or [`SubscribeError::ScopeMismatch`], never a delivery.
//!
//! The crate-root export belongs to slice 9110-02 (which will point
//! the `crate::stream_record` import below at
//! `id_core::stream_record` when it wires the exports); until that
//! lands the integration test includes this module and its
//! dependencies by path.

use std::fmt;

use crate::canonical_cbor::{decode_canonical, encode_canonical, CborError, Value};
use crate::id_shape::{ContentId, HashProfile, IdShapeError};
use crate::signed_envelope::MAX_LABEL_BYTES;
use crate::stream_record::{
    ChunkRecord, ChunkRecordId, NamespaceId, StreamError, StreamId, StreamLifecycle, StreamState,
};

/// The subscription format version this module writes and reads
/// (cursor encodings carry it).
pub const SUBSCRIBE_FORMAT_VERSION: u64 = 1;

/// A resume cursor: a chain position named by sequence + the
/// record ID of the chunk just before it. The genesis cursor is
/// sequence 0 with no record ID. Cursors are the only resume
/// state; they are validated against the chain on every resume,
/// so a cursor is evidence, not authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StreamCursor {
    sequence: u64,
    record_id: Option<ChunkRecordId>,
}

impl StreamCursor {
    /// Builds a cursor, enforcing the genesis discipline: a
    /// record ID is present exactly when the sequence is past 0.
    ///
    /// # Errors
    ///
    /// [`SubscribeError::Malformed`] when the sequence and the
    /// record ID disagree about whether the cursor is genesis.
    pub fn new(sequence: u64, record_id: Option<ChunkRecordId>) -> Result<Self, SubscribeError> {
        if (sequence == 0) != record_id.is_none() {
            return Err(SubscribeError::malformed(
                "a cursor carries a record ID exactly when its sequence is past genesis",
            ));
        }
        Ok(Self {
            sequence,
            record_id,
        })
    }

    /// The genesis cursor: before chunk 0.
    #[must_use]
    pub const fn genesis() -> Self {
        Self {
            sequence: 0,
            record_id: None,
        }
    }

    /// The next sequence this cursor resumes at.
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// The record ID of the chunk just before the resume point;
    /// `None` only at genesis.
    #[must_use]
    pub const fn record_id(&self) -> Option<ChunkRecordId> {
        self.record_id
    }

    /// The cursor as a canonical CBOR value.
    pub fn to_value(&self) -> Value {
        let mut entries = vec![
            ("v", Value::UInt(SUBSCRIBE_FORMAT_VERSION)),
            ("sequence", Value::UInt(self.sequence)),
        ];
        if let Some(record_id) = &self.record_id {
            entries.push(("record_id", content_id_value(record_id.content_id())));
        }
        map(entries)
    }

    /// `canonical_cbor(StreamCursor)`.
    ///
    /// # Errors
    ///
    /// The encoder's rejection (never for a well-formed cursor).
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, SubscribeError> {
        Ok(encode_canonical(&self.to_value())?)
    }

    /// Reads a cursor from canonical bytes.
    ///
    /// # Errors
    ///
    /// [`SubscribeError`] for malformed bytes, an unknown field, a
    /// missing or mistyped field, an unsupported version, or a
    /// sequence/record-ID pair that breaks the genesis discipline.
    pub fn decode(bytes: &[u8]) -> Result<Self, SubscribeError> {
        Self::from_value(&decode_canonical(bytes)?)
    }

    /// Reads a cursor from a decoded canonical value.
    ///
    /// # Errors
    ///
    /// As [`StreamCursor::decode`].
    pub fn from_value(value: &Value) -> Result<Self, SubscribeError> {
        let Value::Map(entries) = value else {
            return Err(SubscribeError::malformed("cursor must be a map"));
        };
        let mut version = None;
        let mut sequence = None;
        let mut record_id = None;
        for (key, field) in entries {
            let Value::Text(name) = key else {
                return Err(SubscribeError::malformed("cursor keys must be text"));
            };
            match (name.as_str(), field) {
                ("v", Value::UInt(v)) => version = Some(*v),
                ("sequence", Value::UInt(n)) => sequence = Some(*n),
                ("record_id", value) => {
                    record_id = Some(ChunkRecordId::from_content_id(content_id_from_value(
                        value,
                    )?));
                }
                ("v" | "sequence" | "record_id", _) => {
                    return Err(SubscribeError::malformed(format!(
                        "cursor field {name:?} has the wrong type"
                    )));
                }
                _ => {
                    return Err(SubscribeError::malformed(format!(
                        "unknown critical field {name:?}"
                    )));
                }
            }
        }
        if version != Some(SUBSCRIBE_FORMAT_VERSION) {
            return Err(SubscribeError::malformed(
                "unsupported or missing cursor format version",
            ));
        }
        let sequence =
            sequence.ok_or_else(|| SubscribeError::malformed("cursor is missing its sequence"))?;
        Self::new(sequence, record_id)
    }
}

/// The authenticated subscriber context, consumed from the #9083
/// tenancy contract: the tenant scope every chunk of the stream
/// must bind, and the namespace the stream must live in. This
/// module never defines tenancy; it only refuses contexts that do
/// not match the stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriberContext {
    tenant_scope: Vec<u8>,
    namespace_id: NamespaceId,
}

impl SubscriberContext {
    /// Builds a context from its authenticated parts.
    ///
    /// # Errors
    ///
    /// [`SubscribeError::Malformed`] on an empty or over-bound
    /// tenant scope.
    pub fn new(tenant_scope: Vec<u8>, namespace_id: NamespaceId) -> Result<Self, SubscribeError> {
        if tenant_scope.is_empty() || tenant_scope.len() > MAX_LABEL_BYTES {
            return Err(SubscribeError::malformed(format!(
                "tenant scope must be 1..={MAX_LABEL_BYTES} bytes"
            )));
        }
        Ok(Self {
            tenant_scope,
            namespace_id,
        })
    }

    /// The tenant scope this context reads under.
    #[must_use]
    pub fn tenant_scope(&self) -> &[u8] {
        &self.tenant_scope
    }

    /// The namespace this context reads in.
    #[must_use]
    pub const fn namespace_id(&self) -> NamespaceId {
        self.namespace_id
    }
}

/// How a stream ended, for a subscriber. A completion carries the
/// final reference; aborts and expiry claim no final content, and
/// a cancellation is the consumer's own doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalOutcome {
    /// The stream completed; the final content/manifest reference.
    Completed {
        /// The final content/manifest reference.
        final_id: ContentId,
    },
    /// The stream was aborted.
    Aborted,
    /// The stream expired.
    Expired,
    /// The consumer cancelled the subscription.
    Cancelled,
}

/// One delivered item of a subscription page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivery {
    /// One retained chunk, in chain order.
    Chunk {
        /// The chunk's sequence.
        sequence: u64,
        /// The chunk record's ID.
        record_id: ChunkRecordId,
        /// Content identity of the chunk payload.
        payload_id: ContentId,
        /// The chunk's media type.
        media_type: String,
    },
    /// The subscription is at the head of a live stream: the
    /// watermark is the cursor to resume from after new chunks
    /// land, and the state is the stream's current state.
    Heartbeat {
        /// The head cursor.
        watermark: StreamCursor,
        /// The stream's current lifecycle state.
        state: StreamState,
    },
    /// The last event a subscription ever delivers.
    Terminal {
        /// How the stream (or the subscription) ended.
        outcome: TerminalOutcome,
    },
}

/// One page of a subscription: at most the `max_items` chunk
/// events the caller asked for, plus at most one heartbeat or
/// terminal event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscribePage {
    events: Vec<Delivery>,
    next_cursor: StreamCursor,
    truncated: bool,
    terminal: Option<TerminalOutcome>,
}

impl SubscribePage {
    /// The page's events, in delivery order.
    #[must_use]
    pub fn events(&self) -> &[Delivery] {
        &self.events
    }

    /// The cursor the next poll (or a later resume) continues
    /// from: the position just past the last delivered chunk.
    #[must_use]
    pub const fn next_cursor(&self) -> StreamCursor {
        self.next_cursor
    }

    /// Whether more chunk events remained undelivered when this
    /// page filled — the page was cut by the cap, never a silent
    /// drop.
    #[must_use]
    pub const fn truncated(&self) -> bool {
        self.truncated
    }

    /// The terminal outcome, present on the final page only.
    #[must_use]
    pub const fn terminal(&self) -> Option<TerminalOutcome> {
        self.terminal
    }
}

/// How a stream resolves for a reader right now, under the
/// current authorization result. The completed form is a typed
/// target: the caller decides whether to follow it. Nothing in
/// this module follows it for them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionView {
    /// Not completed: the live state and the current head cursor.
    Live {
        /// The current lifecycle state.
        state: StreamState,
        /// The chain head cursor.
        head: StreamCursor,
    },
    /// Completed: the typed final reference.
    Completed {
        /// The final content/manifest reference.
        final_id: ContentId,
    },
}

/// A validated snapshot of the lifecycle facts a subscription
/// folds: lifecycle state, final reference, chain head.
struct FoldedHead {
    state: StreamState,
    final_ref: Option<ContentId>,
    head: u64,
    head_record_id: Option<ChunkRecordId>,
}

/// A resumable subscription over one stream's validated retained
/// chain. Construct with [`StreamSubscription::subscribe`] (from
/// the retention floor) or [`StreamSubscription::resume`] (from a
/// cursor); drive it with [`StreamSubscription::refresh`] as the
/// stream grows and [`StreamSubscription::poll`] to consume.
#[derive(Debug)]
pub struct StreamSubscription {
    stream_id: StreamId,
    context: SubscriberContext,
    records: Vec<ChunkRecord>,
    retained_from: u64,
    state: StreamState,
    final_ref: Option<ContentId>,
    head: u64,
    head_record_id: Option<ChunkRecordId>,
    position: StreamCursor,
    cancelled: bool,
    terminal_delivered: bool,
    expired: bool,
}

impl StreamSubscription {
    /// Opens a subscription at the earliest retained position:
    /// the replay covers every retained chunk, and
    /// [`StreamSubscription::position`] starts at the retention
    /// floor.
    ///
    /// # Errors
    ///
    /// [`SubscribeError::Unauthorized`] when the current
    /// authorization result denies the context;
    /// [`SubscribeError::ScopeMismatch`] when the context does not
    /// match the stream's namespace or the records' tenant scope;
    /// [`SubscribeError::ChainMismatch`] when the retained records
    /// do not fold exactly onto the lifecycle's accepted chain.
    pub fn subscribe(
        lifecycle: &StreamLifecycle,
        records: &[ChunkRecord],
        retained_from: u64,
        context: SubscriberContext,
        authorized: bool,
    ) -> Result<Self, SubscribeError> {
        let chain = lifecycle.chain();
        let head = u64::try_from(chain.len())
            .map_err(|_| SubscribeError::malformed("chain longer than u64"))?;
        if retained_from > head {
            return Err(SubscribeError::CursorExpired {
                retained_from: head,
            });
        }
        let floor_cursor = StreamCursor::new(
            retained_from,
            if retained_from == 0 {
                None
            } else {
                Some(
                    chain[usize::try_from(retained_from - 1)
                        .map_err(|_| SubscribeError::malformed("retention floor out of range"))?],
                )
            },
        )?;
        Self::open(
            lifecycle,
            records,
            retained_from,
            context,
            authorized,
            floor_cursor,
        )
    }

    /// Opens a subscription resuming at `cursor`: the first chunk
    /// delivered is exactly `cursor.sequence()`.
    ///
    /// # Errors
    ///
    /// As [`StreamSubscription::subscribe`], plus
    /// [`SubscribeError::CursorAhead`] when the cursor is past
    /// the chain head, [`SubscribeError::CursorExpired`] when it
    /// is below the retention floor, and
    /// [`SubscribeError::CursorMismatch`] when its record ID does
    /// not match the chain at its sequence.
    pub fn resume(
        lifecycle: &StreamLifecycle,
        records: &[ChunkRecord],
        retained_from: u64,
        context: SubscriberContext,
        authorized: bool,
        cursor: StreamCursor,
    ) -> Result<Self, SubscribeError> {
        Self::validate_cursor(lifecycle, cursor.sequence(), retained_from)?;
        if cursor.sequence() > 0 {
            let index = usize::try_from(cursor.sequence() - 1).unwrap_or(usize::MAX);
            let expected = lifecycle.chain().get(index).copied();
            if expected.is_none() || cursor.record_id() != expected {
                return Err(SubscribeError::CursorMismatch {
                    sequence: cursor.sequence(),
                });
            }
        }
        Self::open(
            lifecycle,
            records,
            retained_from,
            context,
            authorized,
            cursor,
        )
    }

    fn open(
        lifecycle: &StreamLifecycle,
        records: &[ChunkRecord],
        retained_from: u64,
        context: SubscriberContext,
        authorized: bool,
        position: StreamCursor,
    ) -> Result<Self, SubscribeError> {
        if !authorized {
            return Err(SubscribeError::Unauthorized);
        }
        let folded = Self::fold(lifecycle, records, retained_from, &context)?;
        Ok(Self {
            stream_id: lifecycle.descriptor().stream_id,
            context,
            records: records.to_vec(),
            retained_from,
            state: folded.state,
            final_ref: folded.final_ref,
            head: folded.head,
            head_record_id: folded.head_record_id,
            position,
            cancelled: false,
            terminal_delivered: false,
            expired: false,
        })
    }

    /// Re-folds the subscription against the stream's current
    /// lifecycle and retained records as it grows (or is pruned).
    /// The previously validated chain prefix must be preserved —
    /// the fold checks it against the lifecycle's chain, which is
    /// append-only, so a rewritten history cannot fold.
    ///
    /// # Errors
    ///
    /// As the fold in [`StreamSubscription::subscribe`]. When the
    /// retention floor has moved past this subscription's
    /// position, refresh fails with
    /// [`SubscribeError::CursorExpired`] and the subscription is
    /// dead: later polls fail the same way.
    pub fn refresh(
        &mut self,
        lifecycle: &StreamLifecycle,
        records: &[ChunkRecord],
        retained_from: u64,
    ) -> Result<(), SubscribeError> {
        if lifecycle.descriptor().stream_id != self.stream_id {
            return Err(SubscribeError::Stream(StreamError::StreamMismatch));
        }
        let folded = Self::fold(lifecycle, records, retained_from, &self.context)?;
        self.records = records.to_vec();
        self.retained_from = retained_from;
        self.state = folded.state;
        self.final_ref = folded.final_ref;
        self.head = folded.head;
        self.head_record_id = folded.head_record_id;
        if self.position.sequence() < retained_from {
            self.expired = true;
            return Err(SubscribeError::CursorExpired { retained_from });
        }
        Ok(())
    }

    /// Delivers the next page: at most `max_items` chunk events
    /// from the current position, then — once the position
    /// reaches the head — a heartbeat (live stream) or the
    /// terminal event (terminal stream). The current
    /// authorization result is re-checked on every call.
    ///
    /// # Errors
    ///
    /// [`SubscribeError::Unauthorized`] when the current
    /// authorization result denies the context;
    /// [`SubscribeError::InvalidMaxItems`] when `max_items` is 0;
    /// [`SubscribeError::CursorExpired`] when a refresh found the
    /// position pruned away; [`SubscribeError::AlreadyTerminal`]
    /// after the terminal page has been delivered.
    pub fn poll(
        &mut self,
        max_items: usize,
        authorized: bool,
    ) -> Result<SubscribePage, SubscribeError> {
        if !authorized {
            return Err(SubscribeError::Unauthorized);
        }
        if max_items == 0 {
            return Err(SubscribeError::InvalidMaxItems);
        }
        if self.terminal_delivered {
            return Err(SubscribeError::AlreadyTerminal);
        }
        if self.expired {
            return Err(SubscribeError::CursorExpired {
                retained_from: self.retained_from,
            });
        }
        if self.cancelled {
            self.terminal_delivered = true;
            return Ok(SubscribePage {
                events: vec![Delivery::Terminal {
                    outcome: TerminalOutcome::Cancelled,
                }],
                next_cursor: self.position,
                truncated: false,
                terminal: Some(TerminalOutcome::Cancelled),
            });
        }
        let mut events = Vec::new();
        while (self.position.sequence() < self.head) && events.len() < max_items {
            let sequence = self.position.sequence();
            let index = usize::try_from(sequence - self.retained_from).map_err(|_| {
                SubscribeError::CursorExpired {
                    retained_from: self.retained_from,
                }
            })?;
            let record = &self.records[index];
            let record_id = record.record_id().map_err(SubscribeError::Stream)?;
            events.push(Delivery::Chunk {
                sequence,
                record_id,
                payload_id: record.payload_id,
                media_type: record.media_type.clone(),
            });
            self.position = StreamCursor {
                sequence: sequence + 1,
                record_id: Some(record_id),
            };
        }
        let truncated = self.position.sequence() < self.head;
        let mut terminal = None;
        if !truncated {
            if self.state.is_terminal() {
                let outcome = match self.state {
                    StreamState::Completed => TerminalOutcome::Completed {
                        final_id: self.final_ref.ok_or_else(|| {
                            SubscribeError::malformed(
                                "a completed stream without a final reference",
                            )
                        })?,
                    },
                    StreamState::Aborted => TerminalOutcome::Aborted,
                    _ => TerminalOutcome::Expired,
                };
                events.push(Delivery::Terminal { outcome });
                terminal = Some(outcome);
                self.terminal_delivered = true;
            } else {
                events.push(Delivery::Heartbeat {
                    watermark: self.position,
                    state: self.state,
                });
            }
        }
        Ok(SubscribePage {
            events,
            next_cursor: self.position,
            truncated,
            terminal,
        })
    }

    /// Cancels the subscription: the next poll returns the
    /// cancelled terminal page instead of further chunks.
    pub fn cancel(&mut self) {
        self.cancelled = true;
    }

    /// Resolves the stream under the *current* authorization
    /// result. A completed stream yields its typed final
    /// reference — data for the caller to follow explicitly,
    /// never an implicit redirect — and only to a context whose
    /// authorization currently holds.
    ///
    /// # Errors
    ///
    /// [`SubscribeError::Unauthorized`] when the current
    /// authorization result denies the context.
    pub fn resolve_target(&self, authorized: bool) -> Result<ResolutionView, SubscribeError> {
        if !authorized {
            return Err(SubscribeError::Unauthorized);
        }
        match (self.state, self.final_ref) {
            (StreamState::Completed, Some(final_id)) => Ok(ResolutionView::Completed { final_id }),
            (state, _) => Ok(ResolutionView::Live {
                state,
                head: StreamCursor {
                    sequence: self.head,
                    record_id: self.head_record_id,
                },
            }),
        }
    }

    /// The stream this subscription reads.
    #[must_use]
    pub const fn stream_id(&self) -> StreamId {
        self.stream_id
    }

    /// The cursor the next poll continues from.
    #[must_use]
    pub const fn position(&self) -> StreamCursor {
        self.position
    }

    /// The lifecycle state at the last fold.
    #[must_use]
    pub const fn state(&self) -> StreamState {
        self.state
    }

    fn validate_cursor(
        lifecycle: &StreamLifecycle,
        sequence: u64,
        retained_from: u64,
    ) -> Result<StreamCursor, SubscribeError> {
        let head = u64::try_from(lifecycle.chain().len())
            .map_err(|_| SubscribeError::malformed("chain longer than u64"))?;
        if sequence > head {
            return Err(SubscribeError::CursorAhead { head });
        }
        if sequence < retained_from {
            return Err(SubscribeError::CursorExpired { retained_from });
        }
        StreamCursor::new(
            sequence,
            if sequence == 0 {
                None
            } else {
                Some(
                    lifecycle.chain()[usize::try_from(sequence - 1)
                        .map_err(|_| SubscribeError::malformed("cursor sequence out of range"))?],
                )
            },
        )
    }

    /// Validates the retained records against the lifecycle's
    /// accepted chain and snapshots the head facts. `records[i]`
    /// must be the chain's record at sequence `retained_from + i`,
    /// covering the chain exactly to its head.
    fn fold(
        lifecycle: &StreamLifecycle,
        records: &[ChunkRecord],
        retained_from: u64,
        context: &SubscriberContext,
    ) -> Result<FoldedHead, SubscribeError> {
        let descriptor = lifecycle.descriptor();
        if descriptor.namespace_id != context.namespace_id {
            return Err(SubscribeError::ScopeMismatch);
        }
        let chain = lifecycle.chain();
        let head = u64::try_from(chain.len())
            .map_err(|_| SubscribeError::malformed("chain longer than u64"))?;
        if retained_from > head {
            return Err(SubscribeError::CursorExpired {
                retained_from: head,
            });
        }
        let floor = usize::try_from(retained_from)
            .map_err(|_| SubscribeError::malformed("retention floor out of range"))?;
        if records.len() != chain.len() - floor {
            return Err(SubscribeError::ChainMismatch {
                sequence: retained_from + u64::try_from(records.len()).unwrap_or(u64::MAX),
            });
        }
        for (offset, record) in records.iter().enumerate() {
            let sequence = retained_from + u64::try_from(offset).unwrap_or(u64::MAX);
            if record.stream_id != descriptor.stream_id || record.sequence != sequence {
                return Err(SubscribeError::ChainMismatch { sequence });
            }
            if record.tenant_scope != context.tenant_scope {
                return Err(SubscribeError::ScopeMismatch);
            }
            let expected_prev = if sequence == 0 {
                None
            } else {
                Some(
                    chain[usize::try_from(sequence - 1)
                        .map_err(|_| SubscribeError::malformed("chain sequence out of range"))?],
                )
            };
            if record.prev_record_id != expected_prev {
                return Err(SubscribeError::ChainMismatch { sequence });
            }
            let record_id = record.record_id().map_err(SubscribeError::Stream)?;
            if record_id != chain[offset + floor] {
                return Err(SubscribeError::ChainMismatch { sequence });
            }
            record.verify_signature().map_err(SubscribeError::Stream)?;
        }
        Ok(FoldedHead {
            state: descriptor.state,
            final_ref: descriptor.final_ref,
            head,
            head_record_id: chain.last().copied(),
        })
    }
}

/// Why a subscription operation was rejected. Every variant has a
/// stable failure class ([`SubscribeError::class`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubscribeError {
    /// The bytes or values were malformed.
    Malformed(String),
    /// A canonical-CBOR rejection.
    Cbor(CborError),
    /// An ID-shape rejection.
    Shape(IdShapeError),
    /// A stream-record rejection surfaced while folding or
    /// resolving (its class is preserved).
    Stream(StreamError),
    /// The current authorization result denies this context.
    Unauthorized,
    /// The context's namespace, or a record's tenant scope, does
    /// not match the stream.
    ScopeMismatch,
    /// `max_items` was 0: a page cap must admit at least one
    /// chunk, and a zero cap is a caller bug, not an empty page.
    InvalidMaxItems,
    /// The cursor's record ID does not match the chain at its
    /// sequence: forgery or equivocation evidence.
    CursorMismatch {
        /// The cursor's sequence.
        sequence: u64,
    },
    /// The cursor (or this subscription's position) is below the
    /// retention floor: the chunks it names are no longer
    /// retained.
    CursorExpired {
        /// The current retention floor.
        retained_from: u64,
    },
    /// The cursor is past the chain head.
    CursorAhead {
        /// The chain head at validation time.
        head: u64,
    },
    /// A retained record does not fold onto the accepted chain.
    ChainMismatch {
        /// The first sequence that does not fold.
        sequence: u64,
    },
    /// The terminal page has already been delivered.
    AlreadyTerminal,
}

impl SubscribeError {
    /// A malformed-input error with a human-readable detail.
    pub fn malformed(detail: impl Into<String>) -> Self {
        Self::Malformed(detail.into())
    }

    /// The stable failure class: `malformed`, `unauthorized`,
    /// `scope_mismatch`, `invalid_max_items`, `cursor_mismatch`,
    /// `cursor_expired`, `cursor_ahead`, `chain_mismatch`,
    /// `already_terminal`, or the wrapped stream error's class.
    #[must_use]
    pub fn class(&self) -> &'static str {
        match self {
            Self::Malformed(_) | Self::Cbor(_) | Self::Shape(_) => "malformed",
            Self::Stream(error) => error.class(),
            Self::Unauthorized => "unauthorized",
            Self::ScopeMismatch => "scope_mismatch",
            Self::InvalidMaxItems => "invalid_max_items",
            Self::CursorMismatch { .. } => "cursor_mismatch",
            Self::CursorExpired { .. } => "cursor_expired",
            Self::CursorAhead { .. } => "cursor_ahead",
            Self::ChainMismatch { .. } => "chain_mismatch",
            Self::AlreadyTerminal => "already_terminal",
        }
    }
}

impl fmt::Display for SubscribeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(detail) => write!(f, "malformed subscription input: {detail}"),
            Self::Cbor(error) => write!(f, "malformed subscription input: {error}"),
            Self::Shape(error) => write!(f, "malformed subscription input: {error}"),
            Self::Stream(error) => write!(f, "{error}"),
            Self::Unauthorized => {
                write!(f, "the current authorization result denies this context")
            }
            Self::ScopeMismatch => {
                write!(f, "the context or a record binds a different scope")
            }
            Self::InvalidMaxItems => write!(f, "max_items must be at least 1"),
            Self::CursorMismatch { sequence } => {
                write!(
                    f,
                    "cursor record ID does not match the chain at sequence {sequence}"
                )
            }
            Self::CursorExpired { retained_from } => {
                write!(
                    f,
                    "cursor_expired: chunks below {retained_from} are no longer retained"
                )
            }
            Self::CursorAhead { head } => {
                write!(f, "cursor is past the chain head {head}")
            }
            Self::ChainMismatch { sequence } => {
                write!(
                    f,
                    "retained record at sequence {sequence} does not fold onto the chain"
                )
            }
            Self::AlreadyTerminal => {
                write!(f, "the subscription already delivered its terminal page")
            }
        }
    }
}

impl std::error::Error for SubscribeError {}

impl From<CborError> for SubscribeError {
    fn from(error: CborError) -> Self {
        Self::Cbor(error)
    }
}

impl From<IdShapeError> for SubscribeError {
    fn from(error: IdShapeError) -> Self {
        Self::Shape(error)
    }
}

impl From<StreamError> for SubscribeError {
    fn from(error: StreamError) -> Self {
        Self::Stream(error)
    }
}

fn map(entries: Vec<(&str, Value)>) -> Value {
    Value::Map(
        entries
            .into_iter()
            .map(|(key, value)| (Value::Text(key.to_owned()), value))
            .collect(),
    )
}

fn content_id_value(id: &ContentId) -> Value {
    Value::Array(vec![
        Value::Text(id.profile().token().to_owned()),
        Value::Bytes(id.digest().to_vec()),
    ])
}

fn content_id_from_value(value: &Value) -> Result<ContentId, SubscribeError> {
    let Value::Array(parts) = value else {
        return Err(SubscribeError::malformed("content ID must be an array"));
    };
    let [Value::Text(token), Value::Bytes(digest)] = parts.as_slice() else {
        return Err(SubscribeError::malformed(
            "content ID must be [profile, digest]",
        ));
    };
    let profile = HashProfile::from_token(token)
        .ok_or_else(|| SubscribeError::malformed(format!("unknown hash profile {token:?}")))?;
    let digest: [u8; 32] = digest
        .clone()
        .try_into()
        .map_err(|_| SubscribeError::malformed("content ID digest must be 32 bytes"))?;
    Ok(ContentId::new(profile, digest))
}
