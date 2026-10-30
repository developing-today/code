//! `LogSync` between two stores, over iroh (P3.2).
//!
//! A session is one stream pair on [`SYNC_ALPN`]. The initiator sends a [`Request`] with the topic
//! and the method it chose ([`choose`]), and the responder answers with an [`Answer`]. If both agree
//! on `LogSync`, each side runs `p2panda-sync`'s `LogSync` on its [`SharedEventStore`] over the stream;
//! if both agree on range sync, each side runs a range session (`event_store::sync`) on the same
//! stream pair, over the same store (#179).
//!
//! Three decisions shape the session:
//!
//! - **Arrival order is the write path.** Each operation is handed to an ingest worker, which verifies
//!   and stores the arrivals in order, in batches, with [`SharedEventStore::ingest_batch`]. A batch is
//!   one write transaction, so one fsync, and the worker runs on a blocking thread so a write does not
//!   hold the runtime. The events are not read back from `p2panda-sync`'s broadcast channel. A slow
//!   reader of that channel loses events without an error, so the events are never taken from it.
//! - **A refused event does not end the session.** A refusal (past a fork point, or over the waiting
//!   caps) is counted in [`Tally`] and the batch goes on. One conflict must not drop the events after
//!   it, which is the failure that range sync had (P3.1a).
//! - **`LogSync` sends each author's applied prefix, and no fork proof.** A store that has forked an
//!   author sends the prefix up to the fork point. `LogSync` compares heights, not heads, so it cannot
//!   see a fork, and the receiver never learns of one from it. Range sync and the proof exchange
//!   (P3.1) carry proofs.
//!
//! Every stage runs under one deadline, so a silent peer ends the session with [`SyncError::Timeout`].
//! A task of its own reads the stream, because `LogSync` selects between reading and writing and a
//! read cancelled midway would lose bytes of a frame.

use std::sync::Arc;
use std::time::Duration;

use futures_lite::{StreamExt, stream};
use futures_util::sink;
use iroh::endpoint::{Connection, RecvStream, SendStream};
use iroh::protocol::{AcceptError, ProtocolHandler};
use iroh::{Endpoint, EndpointAddr};
use iroh_base::EndpointId;
use p2panda_core::Hash;
use p2panda_store::topics::TopicStore;
use p2panda_sync::protocols::{LogSync, LogSyncEvent, LogSyncMessage, LogSyncMetrics};
use p2panda_sync::traits::Protocol;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::{broadcast, mpsc};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::timeout;

use crate::chain::ChainExt;
use crate::connectivity::connect;
use crate::event_store::Ingest;
use crate::frames::{read_frame, write_frame};
use crate::pull_policy::PullGate;
use crate::store_traits::{ChainLog, SharedEventStore};
use crate::sync_method::{Answer, Request, SyncMethod, SyncStats, SyncSupport, choose};
use crate::time::unix_ms;

/// The ALPN for sync sessions. Range sync joins as a method of the same session, and the
/// first frame of every session is the [`Request`]: one session shape on this ALPN.
/// `serve`'s own range sessions (P3.1) run on their own ALPN, `/id-range-sync/1`
/// ([`crate::event_store::sync::ALPN`]) — a deliberate, documented split (#179), so no
/// second, unlabelled session shape shares this one either.
pub const SYNC_ALPN: &[u8] = b"/id-sync/1";

/// How long one side may spend on a session. A peer that stops answering ends it at this deadline.
pub const SESSION_DEADLINE: Duration = Duration::from_secs(60);

/// Events the broadcast channel holds. Nothing reads them (see the module docs), so this only has
/// to be nonzero.
const EVENT_BUFFER: usize = 16;

/// Operations the ingest worker takes in one write transaction, at most.
const INGEST_BATCH: usize = 256;

/// Total arrivals one batch may hold, refusals included. [`INGEST_BATCH`] caps the headers a
/// batch stores, but a refusal is not stored, so a header-only cap never fires on a refused
/// stream: every arrival restarts the batch's idle grace, and a peer streaming refusals spaced
/// inside the grace would hold one batch open forever — nothing commits, and the refusals are
/// never tallied (#1537). Counting every arrival towards this cap ends such a batch; the
/// refusals are tallied and any headers queued with them commit.
const BATCH_MAX_ARRIVALS: usize = INGEST_BATCH;

/// How long the ingest worker waits for more arrivals before committing a batch that is not
/// full. A batch is one write transaction, so a batch per arrival pays one commit (and one
/// fsync) per operation. Arrivals are paced by the peer's page reads and the stream, not by
/// this worker, so without a grace the batch collapses to whatever has arrived in the instant
/// the worker wakes, and a bulk session pays a commit for nearly every operation.
const BATCH_GRACE: Duration = Duration::from_millis(5);

/// How often the ingest worker re-checks the arrivals channel while it waits out
/// [`BATCH_GRACE`]. The worker runs on a blocking thread, so sleeping is safe.
const BATCH_POLL: Duration = Duration::from_micros(250);

/// Operations the stream may queue for the ingest worker before it waits for the worker.
const ARRIVAL_BUFFER: usize = 256;

/// Why a sync session failed. A refused event is not an error: it is counted in [`Tally`].
#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    /// The session did not finish within its deadline.
    #[error("the sync session did not finish within {0:?}")]
    Timeout(Duration),
    /// The peer could not be reached.
    #[error("connecting to the peer: {0}")]
    Connect(String),
    /// A stream failed, or closed before the session finished.
    #[error("the sync stream: {0}")]
    Stream(String),
    /// A frame does not decode.
    #[error("a sync frame does not decode: {0}")]
    Frame(String),
    /// The responder refused the method.
    #[error("the peer refused {0:?}")]
    Refused(SyncMethod),
    /// The pull policy refuses this peer. The responder answers `Refuse`, and no event is sent.
    #[error("the pull policy refuses this peer")]
    PullRefused,
    /// The responder answered a different method from the one requested.
    #[error("the peer answered {answered:?} to a request for {requested:?}")]
    Mismatch {
        /// The method the initiator asked for.
        requested: SyncMethod,
        /// The method the responder answered.
        answered: SyncMethod,
    },
    /// Range sync, the agreed [`SyncMethod::Range`], failed.
    #[error("range sync: {0}")]
    Range(String),
    /// The event store failed.
    #[error("the event store: {0}")]
    Store(String),
    /// `LogSync` failed.
    #[error("LogSync: {0}")]
    LogSync(String),
}

/// What one side did with the events it received in a session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tally {
    /// Events stored and applied.
    pub accepted: u32,
    /// Events the store already had. A session that sends only what the other side lacks has none.
    pub duplicates: u32,
    /// Events held for a dependency that has not arrived.
    pub held: u32,
    /// Events the store refused: past a fork point, over the waiting caps, or malformed. Each is
    /// counted, and the session goes on.
    pub refused: u32,
    /// Events that named a fork. The conflicting headers are stored as the proof.
    pub forks: u32,
    /// Events stored but voided by the membership layer. They do not apply, and they still count
    /// as present.
    pub voided: u32,
}

impl Tally {
    const fn record(&mut self, outcome: &Ingest) {
        match outcome {
            Ingest::Accepted { .. } => self.accepted += 1,
            Ingest::Duplicate { .. } => self.duplicates += 1,
            Ingest::Fork { .. } => self.forks += 1,
            Ingest::Waiting { .. } => self.held += 1,
            Ingest::Refused(_) => self.refused += 1,
            Ingest::Voided { .. } => self.voided += 1,
        }
    }
}

/// The result of one session, on one side.
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    /// The method both sides ran.
    pub method: SyncMethod,
    /// What this side stored.
    pub received: Tally,
    /// `LogSync`'s counts for the session. `sent_operations` is what this side sent.
    pub metrics: LogSyncMetrics,
}

/// Serves sync sessions on [`SYNC_ALPN`]. Register it with
/// `Router::builder(..).accept(SYNC_ALPN, SyncProtocol::new(store, support, gate))`.
#[derive(Clone, Debug)]
pub struct SyncProtocol {
    store: SharedEventStore,
    support: Arc<dyn SyncSupport>,
    gate: PullGate,
}

impl SyncProtocol {
    /// Serves `store`, running the methods that `support` allows, to the peers `gate` admits.
    #[must_use]
    pub fn new(
        store: SharedEventStore,
        support: impl SyncSupport + 'static,
        gate: PullGate,
    ) -> Self {
        Self {
            store,
            support: Arc::new(support),
            gate,
        }
    }
}

impl ProtocolHandler for SyncProtocol {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let mut sessions = JoinSet::new();
        let peer = conn.remote_id();
        while let Ok((send, recv)) = conn.accept_bi().await {
            let store = self.store.clone();
            let support = Arc::clone(&self.support);
            let gate = self.gate.clone();
            sessions.spawn(async move {
                if let Err(error) = answer(
                    &store,
                    support.as_ref(),
                    &gate,
                    peer,
                    send,
                    recv,
                    SESSION_DEADLINE,
                )
                .await
                {
                    tracing::debug!("sync: session ended: {error}");
                }
            });
        }
        sessions.shutdown().await;
        Ok(())
    }
}

/// Syncs `topic` with `peer`, within `deadline`. The method is chosen from `stats` by [`choose`].
/// The session runs only if the responder agrees to that method.
///
/// # Errors
///
/// Returns [`SyncError::Timeout`] if the session does not finish within `deadline`, and an error
/// for a refused, mismatched or failed step.
pub async fn sync_with(
    endpoint: &Endpoint,
    peer: impl Into<EndpointAddr>,
    store: &SharedEventStore,
    topic: Hash,
    stats: &impl SyncStats,
    deadline: Duration,
) -> Result<Outcome, SyncError> {
    let method = choose(stats);
    let session = async {
        let conn = connect(endpoint, peer, SYNC_ALPN)
            .await
            .map_err(|error| SyncError::Connect(format!("{error:#}")))?;
        let (mut send, mut recv) = conn
            .open_bi()
            .await
            .map_err(|error| SyncError::Stream(error.to_string()))?;
        send_message(&mut send, &Request { topic, method }).await?;
        match next_message::<Answer>(&mut recv).await? {
            Answer::Agree(answered) if answered == method => {}
            Answer::Agree(answered) => {
                return Err(SyncError::Mismatch {
                    requested: method,
                    answered,
                });
            }
            Answer::Refuse => return Err(SyncError::Refused(method)),
        }
        let outcome = run_agreed(store, topic, method, send, recv, true).await?;
        conn.close(0_u32.into(), b"done");
        Ok(outcome)
    };
    match timeout(deadline, session).await {
        Ok(result) => result,
        Err(_) => Err(SyncError::Timeout(deadline)),
    }
}

/// Answers one session on `send` and `recv`: reads the request, agrees or refuses, and runs the
/// agreed method.
///
/// The responder does not choose the method. It runs the one the initiator asked for. `gate`
/// decides whether `peer` may pull, and is checked before the method is agreed.
///
/// # Errors
///
/// Returns [`SyncError::Timeout`] past `deadline`, [`SyncError::Refused`] when `support` does not
/// allow the method, [`SyncError::PullRefused`] when `gate` refuses `peer`, and an error for a
/// failed step.
pub async fn answer(
    store: &SharedEventStore,
    support: &dyn SyncSupport,
    gate: &PullGate,
    peer: EndpointId,
    mut send: SendStream,
    mut recv: RecvStream,
    deadline: Duration,
) -> Result<Outcome, SyncError> {
    let session = async {
        let request: Request = next_message(&mut recv).await?;
        // The pull policy decides before the method is agreed, so a refused peer gets no event.
        if !gate.admits(&peer) {
            send_message(&mut send, &Answer::Refuse).await?;
            finish_stream(&mut send, &mut recv).await;
            return Err(SyncError::PullRefused);
        }
        if !support.supports(request.method) {
            send_message(&mut send, &Answer::Refuse).await?;
            finish_stream(&mut send, &mut recv).await;
            return Err(SyncError::Refused(request.method));
        }
        send_message(&mut send, &Answer::Agree(request.method)).await?;
        run_agreed(store, request.topic, request.method, send, recv, false).await
    };
    match timeout(deadline, session).await {
        Ok(result) => result,
        Err(_) => Err(SyncError::Timeout(deadline)),
    }
}

/// Runs the method both sides agreed to, on the stream pair the handshake just used.
///
/// The handshake runs on the raw streams, before any `Frames` reader task exists: it is
/// one frame each way, strictly in turn, so no read is ever selected against a write and
/// a cancelled read cannot lose a frame's bytes. `LogSync` does select between reading
/// and writing, so the `Log` arm hands the streams to [`Frames`] (whose own task reads),
/// while the `Range` arm keeps the raw streams: a range session sends and receives at
/// once through its own halves, and reads each frame to its end before answering it.
async fn run_agreed(
    store: &SharedEventStore,
    topic: Hash,
    method: SyncMethod,
    send: SendStream,
    recv: RecvStream,
    initiator: bool,
) -> Result<Outcome, SyncError> {
    match method {
        SyncMethod::Log => {
            let mut frames = Frames::new(send, recv);
            let outcome = run_log_sync(store, topic, &mut frames).await?;
            frames.finish().await;
            Ok(outcome)
        }
        SyncMethod::Range => {
            let (mut send, mut recv) = (send, recv);
            let ranged = run_range_sync(store, &mut send, &mut recv, initiator).await;
            finish_stream(&mut send, &mut recv).await;
            ranged
        }
    }
}

/// Runs one range session under the handshake, on the stream pair the handshake used.
///
/// The store seam is the session's only design choice (#179): the range session takes
/// the store one step at a time through `event_store::sync::RangeStore`, whose
/// [`SharedEventStore`] implementation locks per step and never across an await, so a
/// session never blocks the store's other users behind a network wait. The range sync
/// set is the store's whole set: range sync has no topic subset, so `topic` names the
/// session for the `Log` arm only. The deadline is the caller's, as for `Log`.
async fn run_range_sync(
    store: &SharedEventStore,
    send: &mut SendStream,
    recv: &mut RecvStream,
    initiator: bool,
) -> Result<Outcome, SyncError> {
    let mut store = store.clone();
    let ranged = crate::event_store::sync::sync_on_streams(
        &mut store,
        send,
        recv,
        initiator,
        crate::event_store::range::Params::default(),
        unix_ms(),
    )
    .await
    .map_err(|error| SyncError::Range(format!("{error:#}")))?;
    let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    Ok(Outcome {
        method: SyncMethod::Range,
        received: Tally {
            accepted: count(ranged.accepted),
            duplicates: count(ranged.duplicates),
            held: count(ranged.waiting),
            refused: count(ranged.refused),
            forks: count(ranged.forks),
            voided: count(ranged.voided),
        },
        metrics: LogSyncMetrics {
            sent_operations: count(ranged.sent),
            received_operations: count(ranged.received),
            ..LogSyncMetrics::default()
        },
    })
}

/// Writes one handshake message as a frame on the raw stream.
async fn send_message<T: Serialize>(send: &mut SendStream, message: &T) -> Result<(), SyncError> {
    let bytes =
        postcard::to_allocvec(message).map_err(|error| SyncError::Frame(error.to_string()))?;
    write_frame(send, &bytes)
        .await
        .map_err(|error| SyncError::Stream(format!("{error:#}")))
}

/// Reads one handshake message from the raw stream. A first frame that does not decode
/// as the expected message is a [`SyncError::Frame`]: the session is refused, never
/// reinterpreted as another protocol.
async fn next_message<T: DeserializeOwned>(recv: &mut RecvStream) -> Result<T, SyncError> {
    let frame = read_frame(recv)
        .await
        .map_err(|error| SyncError::Stream(format!("{error:#}")))?
        .ok_or_else(|| SyncError::Stream("the stream ended during the handshake".to_owned()))?;
    postcard::from_bytes(&frame).map_err(|error| SyncError::Frame(error.to_string()))
}

/// Ends our side of a raw stream pair, then reads to the peer's end, so nothing sent is
/// cut off by closing the connection. The raw-stream counterpart of [`Frames::finish`].
async fn finish_stream(send: &mut SendStream, recv: &mut RecvStream) {
    let _ = send.finish();
    while let Ok(Some(_frame)) = read_frame(recv).await {}
}

/// Runs `p2panda-sync`'s `LogSync` for the logs of `topic`, on `frames`.
///
/// The logs are the topic's authors on this side, as `TopicStore::resolve` gives them. An author
/// the topic does not name here is sent in full by the other side, and the duplicates are counted.
async fn run_log_sync(
    store: &SharedEventStore,
    topic: Hash,
    frames: &mut Frames,
) -> Result<Outcome, SyncError> {
    let logs = store
        .resolve(&topic)
        .await
        .map_err(|error| SyncError::Store(error.to_string()))?;
    // Nothing reads the broadcast events. The sender fails only when no receiver exists, so this
    // receiver is kept alive until the session ends.
    let (events_tx, _events_rx) = broadcast::channel::<LogSyncEvent<ChainExt>>(EVENT_BUFFER);
    let protocol: LogSync<ChainLog, ChainExt, SharedEventStore, LogSyncEvent<ChainExt>> =
        LogSync::new(store.clone(), logs, events_tx);

    // The operations are stored by a worker on a thread that may block, so a write does not hold the
    // runtime. The stream sends each operation to it, and waits when it is full.
    let (arrivals_tx, arrivals_rx) = mpsc::channel::<Arrival>(ARRIVAL_BUFFER);
    let worker = {
        let store = store.clone();
        tokio::task::spawn_blocking(move || ingest_arrivals(&store, arrivals_rx))
    };

    let inbound = stream::unfold(&mut frames.inbound, |rx| async move {
        let item = rx.recv().await?;
        Some((item, rx))
    });
    let mut inbound = Box::pin(inbound.then(|frame| {
        let arrivals = &arrivals_tx;
        async move {
            let frame = frame.map_err(SyncError::Stream)?;
            let message: LogSyncMessage<ChainLog> = postcard::from_bytes(&frame)
                .map_err(|error| SyncError::Frame(error.to_string()))?;
            if let LogSyncMessage::Operation((header, body)) = &message {
                // Events carry no body here, and a body is not stored, so a body is refused.
                let arrival = if body.is_some() {
                    Arrival::Refused
                } else {
                    Arrival::Header(header.clone())
                };
                arrivals
                    .send(arrival)
                    .await
                    .map_err(|_closed| SyncError::Store("the ingest worker stopped".to_owned()))?;
            }
            Ok::<_, SyncError>(message)
        }
    }));
    let mut outbound = Box::pin(sink::unfold(
        &mut frames.send,
        |send, message: LogSyncMessage<ChainLog>| async move {
            let bytes = postcard::to_allocvec(&message)
                .map_err(|error| SyncError::Frame(error.to_string()))?;
            write_frame(&mut *send, &bytes)
                .await
                .map_err(|error| SyncError::Stream(format!("{error:#}")))?;
            Ok::<_, SyncError>(send)
        },
    ));

    let result = protocol.run(&mut outbound, &mut inbound).await;
    // The streams hold the sender. Release them, then the worker ends once it has stored the rest.
    drop((outbound, inbound));
    drop(arrivals_tx);
    let stored = worker
        .await
        .map_err(|error| SyncError::Store(error.to_string()))?;
    // A store error ends the worker, and the stream then fails too. The store's error is the cause.
    let tally = stored?;
    let (_dedup, metrics) = result.map_err(|error| SyncError::LogSync(error.to_string()))?;
    Ok(Outcome {
        method: SyncMethod::Log,
        received: tally,
        metrics,
    })
}

/// One thing the stream hands the ingest worker.
enum Arrival {
    /// An operation's header, to verify and store.
    Header(Vec<u8>),
    /// An operation with a body. It is refused, and counted.
    Refused,
}

/// Stores a session's arrivals, in order, on a thread that may block. The worker takes what has
/// arrived, up to [`INGEST_BATCH`] operations, and stores it in one write transaction, so the
/// session pays one commit, and one fsync, per batch rather than per operation. It returns the
/// tally once the stream has closed.
///
/// # Errors
///
/// Returns [`SyncError::Store`] for the first batch the store refuses. Nothing of that batch is
/// stored, and the arrivals after it are not read.
fn ingest_arrivals(
    store: &SharedEventStore,
    mut arrivals: mpsc::Receiver<Arrival>,
) -> Result<Tally, SyncError> {
    let mut tally = Tally::default();
    while let Some(first) = arrivals.blocking_recv() {
        let mut batch: Vec<Vec<u8>> = Vec::new();
        for arrival in collect_batch(first, &mut arrivals) {
            match arrival {
                Arrival::Header(bytes) => batch.push(bytes),
                Arrival::Refused => tally.refused += 1,
            }
        }
        if batch.is_empty() {
            continue;
        }
        let outcomes = store
            .ingest_batch(&batch, unix_ms())
            .map_err(|error| SyncError::Store(error.to_string()))?;
        for outcome in &outcomes {
            tally.record(outcome);
        }
    }
    Ok(tally)
}

/// Collects one batch of arrivals, starting from `first`. The batch holds at most
/// [`INGEST_BATCH`] headers and at most [`BATCH_MAX_ARRIVALS`] arrivals in total, refusals
/// included, so a refused stream cannot hold the batch open (see [`BATCH_MAX_ARRIVALS`]).
/// Everything already queued is taken at once; then, while the batch is not full, the worker
/// waits out an idle [`BATCH_GRACE`] for more arrivals before giving up on the batch — each
/// arrival restarts the wait — so arrivals paced by the stream still land in the same write
/// transaction, however long their run spans. The wait ends early when the stream
/// closes (every sender dropped), so a session's last, partial batch is not held back.
fn collect_batch(first: Arrival, arrivals: &mut mpsc::Receiver<Arrival>) -> Vec<Arrival> {
    use tokio::sync::mpsc::error::TryRecvError;

    let mut batch = Vec::new();
    let mut headers = 0_usize;
    let mut next = Some(first);
    let mut grace_end = None;
    while let Some(arrival) = next {
        if matches!(arrival, Arrival::Header(_)) {
            headers += 1;
        }
        batch.push(arrival);
        if headers >= INGEST_BATCH || batch.len() >= BATCH_MAX_ARRIVALS {
            break;
        }
        next = loop {
            match arrivals.try_recv() {
                Ok(arrival) => {
                    // The grace is an idle wait: an arrival restarts it, so a stream that
                    // keeps arriving inside the grace keeps sharing this batch, however
                    // long the whole run spans.
                    grace_end = None;
                    break Some(arrival);
                }
                Err(TryRecvError::Disconnected) => break None,
                Err(TryRecvError::Empty) => {
                    let end =
                        *grace_end.get_or_insert_with(|| std::time::Instant::now() + BATCH_GRACE);
                    if std::time::Instant::now() >= end {
                        break None;
                    }
                    std::thread::sleep(BATCH_POLL);
                }
            }
        };
    }
    batch
}

/// One stream pair. A task of its own reads the receive half and forwards whole frames over a
/// channel, so a read is never cancelled midway, whatever the caller selects over.
struct Frames {
    send: SendStream,
    inbound: mpsc::Receiver<Result<Vec<u8>, String>>,
    reader: JoinHandle<()>,
}

impl Frames {
    fn new(send: SendStream, mut recv: RecvStream) -> Self {
        let (tx, inbound) = mpsc::channel(8);
        let reader = tokio::spawn(async move {
            loop {
                let frame = match read_frame(&mut recv).await {
                    Ok(Some(frame)) => Ok(frame),
                    // The peer finished its side. Dropping `tx` ends the inbound stream.
                    Ok(None) => break,
                    Err(error) => Err(format!("{error:#}")),
                };
                let failed = frame.is_err();
                if tx.send(frame).await.is_err() || failed {
                    break;
                }
            }
        });
        Self {
            send,
            inbound,
            reader,
        }
    }

    /// Ends our side, then waits for the peer's end, so that nothing sent is cut off by closing the
    /// connection. This runs after `LogSync` has finished on both sides, so everything each side needs
    /// has already arrived. A read error here is the peer closing the connection, which is not a
    /// failure of the session.
    async fn finish(mut self) {
        let _ = self.send.finish();
        while let Some(frame) = self.inbound.recv().await {
            if frame.is_err() {
                break;
            }
        }
    }
}

impl Drop for Frames {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddr};

    use iroh::endpoint::{Connection, RelayMode, presets};
    use iroh::protocol::Router;
    use p2panda_core::{SeqNum, SigningKey};

    use super::*;
    use crate::chain::{Event, sign};
    use crate::event_store::EventStore;
    use crate::pull_policy::{PullGate, PullPolicy, testing};
    use crate::sync_method::{Estimate, LogAndRange, LogSyncOnly};
    use crate::waiting::WaitingLimits;
    use iroh_base::SecretKey;

    // Keep test sessions aligned with production; CI can run the bulk page-transfer test under
    // substantially more contention than a focused local invocation.
    const DEADLINE: Duration = SESSION_DEADLINE;

    fn key(n: u8) -> SigningKey {
        SigningKey::from_bytes(&[n; 32])
    }

    fn group(n: u8) -> Hash {
        Hash::digest([n])
    }

    /// No chains and no sync yet: `LogSync`, by the rule.
    fn bootstrap() -> Estimate {
        Estimate {
            chains: 0,
            difference: 0,
            last_completed_sync_ms: None,
        }
    }

    /// The first `count` events of `signer`'s chain, all in `group`, each naming the one before.
    fn chain_of(signer: &SigningKey, count: SeqNum, group: Hash) -> Vec<Event> {
        let mut events: Vec<Event> = Vec::new();
        for seq in 0..count {
            let backlink = events.last().map(|event| event.id);
            let ext = ChainExt {
                kind: "message".to_owned(),
                group: Some(group),
                after: Vec::new(),
                tag: None,
                document: None,
                removes: None,
            };
            events.push(sign(signer, seq, backlink, None, ext).unwrap());
        }
        events
    }

    fn open() -> (tempfile::TempDir, SharedEventStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = EventStore::open(dir.path(), WaitingLimits::default(), 0).unwrap();
        (dir, SharedEventStore::new(store))
    }

    fn ingest_all(store: &SharedEventStore, events: &[Event]) {
        for event in events {
            let outcome = store.ingest(&event.bytes, None, 0).unwrap();
            assert!(
                matches!(outcome, Ingest::Accepted { .. }),
                "expected an accepted event, got {outcome:?}"
            );
        }
    }

    /// The bytes of every position `0..=max_seq` of each author, as the store applies them. `None`
    /// is an unapplied position.
    fn applied(
        store: &SharedEventStore,
        authors: &[&SigningKey],
        max_seq: SeqNum,
    ) -> Vec<Option<Vec<u8>>> {
        let mut positions = Vec::new();
        for author in authors {
            for seq in 0..=max_seq {
                let who = *author.verifying_key().as_bytes();
                let bytes = store
                    .read(|s| Ok(s.event(&who, seq)?.map(|event| event.bytes)))
                    .unwrap();
                positions.push(bytes);
            }
        }
        positions
    }

    async fn endpoint() -> Endpoint {
        Endpoint::builder(presets::Minimal)
            .relay_mode(RelayMode::Disabled)
            .bind()
            .await
            .unwrap()
    }

    /// As [`endpoint`], under `key`, so a test knows the identity before it dials.
    async fn endpoint_as(key: SecretKey) -> Endpoint {
        Endpoint::builder(presets::Minimal)
            .secret_key(key)
            .relay_mode(RelayMode::Disabled)
            .bind()
            .await
            .unwrap()
    }

    /// `ep`'s address on its loopback sockets, so a peer dials it without a relay or discovery.
    fn loopback(ep: &Endpoint) -> EndpointAddr {
        EndpointAddr::from_parts(
            ep.id(),
            ep.bound_sockets().iter().map(|addr| {
                let ip = if addr.ip().is_unspecified() {
                    Ipv4Addr::LOCALHOST.into()
                } else {
                    addr.ip()
                };
                iroh::TransportAddr::Ip(SocketAddr::new(ip, addr.port()))
            }),
        )
    }

    /// The responder: the production [`answer`], with each session's result sent to the test.
    #[derive(Debug)]
    struct Capture {
        store: SharedEventStore,
        support: Arc<dyn SyncSupport>,
        gate: PullGate,
        results: mpsc::UnboundedSender<Result<Outcome, SyncError>>,
    }

    impl ProtocolHandler for Capture {
        async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
            while let Ok((send, recv)) = conn.accept_bi().await {
                let result = answer(
                    &self.store,
                    self.support.as_ref(),
                    &self.gate,
                    conn.remote_id(),
                    send,
                    recv,
                    DEADLINE,
                )
                .await;
                let _ = self.results.send(result);
            }
            Ok(())
        }
    }

    /// Serves `store` on loopback, to every peer. The receiver gets the responder's result for
    /// each session.
    async fn responder(
        store: &SharedEventStore,
    ) -> (
        Endpoint,
        Router,
        mpsc::UnboundedReceiver<Result<Outcome, SyncError>>,
    ) {
        responder_gated(store, testing::open_gate()).await
    }

    /// Serves `store` on loopback under `gate`.
    async fn responder_gated(
        store: &SharedEventStore,
        gate: PullGate,
    ) -> (
        Endpoint,
        Router,
        mpsc::UnboundedReceiver<Result<Outcome, SyncError>>,
    ) {
        let ep = endpoint().await;
        let (results, rx) = mpsc::unbounded_channel();
        let handler = Capture {
            store: store.clone(),
            support: Arc::new(LogSyncOnly),
            gate,
            results,
        };
        let router = Router::builder(ep.clone())
            .accept(SYNC_ALPN, handler)
            .spawn();
        (ep, router, rx)
    }

    /// Serves `store` on loopback, running the methods `support` allows, to every peer.
    async fn responder_support(
        store: &SharedEventStore,
        support: Arc<dyn SyncSupport>,
    ) -> (
        Endpoint,
        Router,
        mpsc::UnboundedReceiver<Result<Outcome, SyncError>>,
    ) {
        let ep = endpoint().await;
        let (results, rx) = mpsc::unbounded_channel();
        let handler = Capture {
            store: store.clone(),
            support,
            gate: testing::open_gate(),
            results,
        };
        let router = Router::builder(ep.clone())
            .accept(SYNC_ALPN, handler)
            .spawn();
        (ep, router, rx)
    }

    #[tokio::test]
    async fn test_two_stores_end_equal_after_one_session_with_no_duplicates() {
        let topic = group(1);
        let (alice, bob, carol) = (key(1), key(2), key(3));
        let alice_events = chain_of(&alice, 5, topic);
        let bob_events = chain_of(&bob, 3, topic);
        let carol_events = chain_of(&carol, 5, topic);
        let (_da, store_a) = open();
        let (_db, store_b) = open();
        // A has alice and bob. B has the first three of alice, which A also has, and carol.
        ingest_all(&store_a, &alice_events);
        ingest_all(&store_a, &bob_events);
        ingest_all(&store_b, &alice_events[..3]);
        ingest_all(&store_b, &carol_events);

        let (server, _router, mut results) = responder(&store_a).await;
        let client = endpoint().await;
        let outcome = sync_with(
            &client,
            loopback(&server),
            &store_b,
            topic,
            &bootstrap(),
            DEADLINE,
        )
        .await
        .unwrap();
        let answered = results.recv().await.unwrap().unwrap();

        assert_eq!(outcome.method, SyncMethod::Log);
        assert_eq!(answered.method, SyncMethod::Log, "both sides ran LogSync");
        // B lacked alice 3 and 4, and bob 0 to 2. A lacked carol 0 to 4.
        assert_eq!(outcome.received.accepted, 5);
        assert_eq!(answered.received.accepted, 5);
        assert_eq!(outcome.received.duplicates, 0, "no event is inserted twice");
        assert_eq!(
            answered.received.duplicates, 0,
            "no event is inserted twice"
        );
        assert_eq!(outcome.received.refused + answered.received.refused, 0);
        assert_eq!(outcome.metrics.sent_operations, 5, "B sent carol 0 to 4");

        let all = [&alice, &bob, &carol];
        let (a_state, b_state) = (applied(&store_a, &all, 4), applied(&store_b, &all, 4));
        assert_eq!(a_state, b_state, "both stores hold the same applied events");
        // Alice 0 to 4, bob 0 to 2, and carol 0 to 4: thirteen events. Bob 3 and 4 do not exist.
        assert_eq!(
            a_state.iter().flatten().count(),
            13,
            "the union is complete"
        );
        store_a.check_invariants().unwrap();
        store_b.check_invariants().unwrap();

        // A second session finds nothing to send, and inserts nothing.
        let again = sync_with(
            &client,
            loopback(&server),
            &store_b,
            topic,
            &bootstrap(),
            DEADLINE,
        )
        .await
        .unwrap();
        let again_answered = results.recv().await.unwrap().unwrap();
        assert_eq!(again.received, Tally::default());
        assert_eq!(again_answered.received, Tally::default());
        assert_eq!(again.metrics.sent_operations, 0);
    }

    /// Issue #1302: on a failed bulk session, read the responder's own
    /// session result (bounded, so a still-running responder cannot hang
    /// the test) and render it next to the client's error. `SyncError`
    /// rendering distinguishes the verdicts for #241: a responder
    /// `Timeout` means its session deadline dropped it; a transport
    /// error on both sides points at the connection itself.
    async fn drain_responder_result(
        results: &mut mpsc::UnboundedReceiver<Result<Outcome, SyncError>>,
    ) -> String {
        match tokio::time::timeout(std::time::Duration::from_secs(10), results.recv()).await {
            Ok(Some(Ok(outcome))) => format!("Ok(received={:?})", outcome.received),
            Ok(Some(Err(error))) => format!("Err({error:#})"),
            Ok(None) => "channel closed without a result".to_owned(),
            Err(_) => "no result within 10s of the client error".to_owned(),
        }
    }

    #[tokio::test]
    async fn test_a_bulk_session_moves_every_event_across_pages() {
        let topic = group(2);
        let authors = [key(4), key(5), key(6)];
        // 70 events an author is more than one page of 64, so the stream crosses a page boundary.
        let events: Vec<Vec<Event>> = authors.iter().map(|who| chain_of(who, 70, topic)).collect();
        let (_da, store_a) = open();
        let (_db, store_b) = open();
        for chain in &events {
            ingest_all(&store_a, chain);
        }

        // Under full-suite load a loopback session can be reset by the peer
        // mid-stream (issue #188). Sync is idempotent, so retry with a fresh
        // session and assert the converged state of the successful one.
        let mut attempt = 0;
        let (outcome, answered) = loop {
            attempt += 1;
            let (server, router, mut results) = responder(&store_a).await;
            let client = endpoint().await;
            match sync_with(
                &client,
                loopback(&server),
                &store_b,
                topic,
                &bootstrap(),
                DEADLINE,
            )
            .await
            {
                Ok(outcome) => {
                    let answered = results.recv().await.unwrap().unwrap();
                    client.close().await;
                    router.shutdown().await.unwrap();
                    break (outcome, answered);
                }
                Err(error) if attempt < 3 => {
                    // Issue #1302: the responder's own session result is the
                    // decisive evidence for a mid-stream reset (#241) — drain
                    // it before shutting anything down, next to the client's
                    // error. Closing the client first is what makes the
                    // responder's `answer` return promptly on this path.
                    client.close().await;
                    let responder = drain_responder_result(&mut results).await;
                    eprintln!(
                        "retrying the bulk session after a transport error: {error:#}\n\
                         responder session result: {responder}"
                    );
                    let _ = router.shutdown().await;
                }
                Err(error) => {
                    client.close().await;
                    let responder = drain_responder_result(&mut results).await;
                    panic!(
                        "bulk session failed after {attempt} attempts: {error:#}\n\
                         responder session result: {responder}"
                    );
                }
            }
        };

        assert_eq!(outcome.received.accepted, 210);
        assert_eq!(outcome.received.duplicates, 0);
        assert_eq!(outcome.received.refused, 0);
        let _ = answered;
        let refs: Vec<&SigningKey> = authors.iter().collect();
        assert_eq!(applied(&store_a, &refs, 69), applied(&store_b, &refs, 69));
        store_b.check_invariants().unwrap();
    }

    /// Arrivals paced a few milliseconds apart — as a bulk stream paces them — still land in
    /// one batch, so the session pays one write transaction for them, not one per operation.
    #[test]
    fn test_batch_collection_coalesces_paced_arrivals() {
        let (tx, mut rx) = mpsc::channel::<Arrival>(16);
        let pacer = std::thread::spawn(move || {
            for n in 0..3_u8 {
                tx.try_send(Arrival::Header(vec![n])).unwrap();
                std::thread::sleep(Duration::from_millis(2));
            }
        });
        // The first arrival, however it gets here, starts the batch.
        let first = loop {
            match rx.try_recv() {
                Ok(arrival) => break arrival,
                Err(mpsc::error::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(error) => panic!("the pacer went away: {error}"),
            }
        };
        let batch = collect_batch(first, &mut rx);
        pacer.join().unwrap();

        assert_eq!(batch.len(), 3, "paced arrivals share one batch");
        assert!(
            batch
                .iter()
                .all(|arrival| matches!(arrival, Arrival::Header(_)))
        );
    }

    /// The grace is an idle wait, not a total window: arrivals that keep coming, each within
    /// the grace of the one before, share one batch even when the whole run spans far longer
    /// than [`BATCH_GRACE`]. A pacing peer must not be cut back to one transaction per
    /// operation just because its stream outlasts the first grace window.
    #[test]
    fn test_batch_collection_grace_restarts_on_each_arrival() {
        const PACED: usize = 8;
        let (tx, mut rx) = mpsc::channel::<Arrival>(16);
        let pacer = std::thread::spawn(move || {
            for n in 0..PACED {
                tx.try_send(Arrival::Header(vec![n as u8])).unwrap();
                std::thread::sleep(Duration::from_millis(2));
            }
        });
        let first = loop {
            match rx.try_recv() {
                Ok(arrival) => break arrival,
                Err(mpsc::error::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(error) => panic!("the pacer went away: {error}"),
            }
        };
        let batch = collect_batch(first, &mut rx);
        pacer.join().unwrap();

        assert_eq!(
            batch.len(),
            PACED,
            "arrivals each inside the grace of the previous one share one batch"
        );
    }

    /// A lone arrival is committed after the grace, not held for a full batch that may never
    /// come: a small session's last batch is not delayed.
    #[test]
    fn test_batch_collection_releases_a_lone_arrival() {
        let (tx, mut rx) = mpsc::channel::<Arrival>(16);
        tx.try_send(Arrival::Header(vec![7])).unwrap();
        let first = rx.try_recv().unwrap();
        let started = std::time::Instant::now();
        let batch = collect_batch(first, &mut rx);

        assert_eq!(batch.len(), 1);
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "a lone arrival waits out the grace only, took {:?}",
            started.elapsed()
        );
    }

    /// A full batch is capped at [`INGEST_BATCH`] headers, however much is queued behind it.
    #[test]
    fn test_batch_collection_stops_at_a_full_batch() {
        let (tx, mut rx) = mpsc::channel::<Arrival>(INGEST_BATCH + 16);
        for n in 0..INGEST_BATCH + 10 {
            tx.try_send(Arrival::Header(vec![(n % 251) as u8])).unwrap();
        }
        let first = rx.try_recv().unwrap();
        let batch = collect_batch(first, &mut rx);

        assert_eq!(batch.len(), INGEST_BATCH, "the batch is capped");
        let mut left = 0;
        while rx.try_recv().is_ok() {
            left += 1;
        }
        assert_eq!(left, 10, "the rest stays queued for the next batch");
    }

    /// A stream of refusals spaced inside the grace must not hold a batch open forever
    /// (#1537): refusals count towards the batch's total-arrival cap, so the batch ends at
    /// the cap while the stream is still flowing. Before the cap, each refusal restarted
    /// the idle grace and `collect_batch` returned only once the stream closed.
    #[test]
    fn test_batch_collection_refused_stream_ends_at_the_total_cap() {
        const SENT: usize = BATCH_MAX_ARRIVALS * 3;
        let (tx, mut rx) = mpsc::channel::<Arrival>(SENT + 16);
        let pacer = std::thread::spawn(move || {
            for _ in 0..SENT {
                tx.try_send(Arrival::Refused).unwrap();
                std::thread::sleep(Duration::from_millis(2));
            }
        });
        let first = loop {
            match rx.try_recv() {
                Ok(arrival) => break arrival,
                Err(mpsc::error::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(error) => panic!("the pacer went away: {error}"),
            }
        };
        let started = std::time::Instant::now();
        let batch = collect_batch(first, &mut rx);
        let elapsed = started.elapsed();
        pacer.join().unwrap();

        assert_eq!(
            batch.len(),
            BATCH_MAX_ARRIVALS,
            "a refused stream ends the batch at the total-arrival cap"
        );
        assert!(
            batch
                .iter()
                .all(|arrival| matches!(arrival, Arrival::Refused)),
            "the capped batch holds the refusals it collected"
        );
        assert!(
            elapsed < Duration::from_millis((SENT * 2) as u64),
            "the batch ended while the stream was still flowing, took {elapsed:?}"
        );
    }

    /// The ingest worker tallies a refused stream and commits the headers queued with it
    /// (#1537): the refusals are counted, not held uncommitted behind a batch that never
    /// ends, and the one real header among them is stored.
    #[test]
    fn test_ingest_tallies_refusals_and_commits_queued_headers() {
        let (_dir, store) = open();
        let signer = key(31);
        let events = chain_of(&signer, 1, group(31));
        let header = events[0].bytes.clone();

        const REFUSALS: usize = BATCH_MAX_ARRIVALS + 10;
        let (tx, rx) = mpsc::channel::<Arrival>(REFUSALS + 16);
        let producer = std::thread::spawn(move || {
            for _ in 0..REFUSALS {
                tx.try_send(Arrival::Refused).unwrap();
                std::thread::sleep(Duration::from_millis(1));
            }
            tx.try_send(Arrival::Header(header)).unwrap();
        });
        let tally = ingest_arrivals(&store, rx).unwrap();
        producer.join().unwrap();

        assert_eq!(tally.refused, REFUSALS as u32, "every refusal is tallied");
        assert_eq!(tally.accepted, 1, "the queued header is committed");
        let refs: Vec<&SigningKey> = vec![&signer];
        assert_eq!(
            applied(&store, &refs, 0)[0].as_deref(),
            Some(events[0].bytes.as_slice())
        );
    }

    /// A store that has forked `alice`. It holds the first `held` events of the honest chain, which
    /// has three. Its fork proof is the honest and rival headers at seq 1, so its applied prefix is
    /// seq 0. Returns the store, the whole honest chain, and the rival.
    fn forked_store(
        alice: &SigningKey,
        topic: Hash,
        held: usize,
    ) -> (tempfile::TempDir, SharedEventStore, Vec<Event>, Event) {
        let (dir, store) = open();
        let honest = chain_of(alice, 3, topic);
        // The rival is another seq 1, with the same backlink as the honest seq 1.
        ingest_all(&store, &honest[..held]);
        let rival_ext = ChainExt {
            kind: "rival".to_owned(),
            group: Some(group(9)),
            after: Vec::new(),
            tag: None,
            document: None,
            removes: None,
        };
        let rival = sign(alice, 1, Some(honest[0].id), None, rival_ext).unwrap();
        assert!(matches!(
            store.ingest(&rival.bytes, None, 0).unwrap(),
            Ingest::Fork { seq: 1, .. }
        ));
        (dir, store, honest, rival)
    }

    #[tokio::test]
    async fn test_fork_proofs_are_not_carried_and_the_log_is_the_applied_prefix() {
        let topic = group(3);
        let alice = key(7);
        let who = alice.verifying_key();
        let (_da, store_a, _honest, rival) = forked_store(&alice, topic, 3);
        let (_db, store_b) = open();
        // A's view, before the sync: seq 0 applied, nothing past it.
        assert_eq!(
            store_a
                .read(|s| Ok(s.chain(who.as_bytes())?.map(|c| c.next_seq)))
                .unwrap(),
            Some(1),
            "the applied prefix stops at the fork point"
        );

        let (server, _router, mut results) = responder(&store_a).await;
        let client = endpoint().await;
        let outcome = sync_with(
            &client,
            loopback(&server),
            &store_b,
            topic,
            &bootstrap(),
            DEADLINE,
        )
        .await
        .unwrap();
        results.recv().await.unwrap().unwrap();

        // B receives the applied prefix, seq 0, and nothing else from alice.
        assert_eq!(outcome.received.accepted, 1);
        assert_eq!(
            applied(&store_b, &[&alice], 2),
            vec![applied(&store_a, &[&alice], 0)[0].clone(), None, None,],
            "B holds seq 0 only"
        );
        // B has no fork proof, and never saw the rival. A keeps its own proof.
        assert!(
            store_b
                .read(|s| Ok(s.fork_proof(who.as_bytes())?))
                .unwrap()
                .is_none()
        );
        assert!(
            store_b
                .read(|s| Ok(s.event_by_id(&rival.id)?))
                .unwrap()
                .is_none()
        );
        assert!(
            store_a
                .read(|s| Ok(s.fork_proof(who.as_bytes())?))
                .unwrap()
                .is_some()
        );
        store_b.check_invariants().unwrap();
    }

    #[tokio::test]
    async fn test_a_refused_event_does_not_abort_the_batch_after_it() {
        // Two authors. The forked one sorts first, so its refused events come before the other
        // author's events in the batch.
        let topic = group(4);
        let (first, second) = (key(8), key(9));
        let (x, y) = if first.verifying_key() < second.verifying_key() {
            (first, second)
        } else {
            (second, first)
        };
        // A holds seq 0 and seq 1 of x, so seq 2 of x is past the fork point when it arrives.
        let (_da, store_a, honest_x, _rival) = forked_store(&x, topic, 2);
        let y_events = chain_of(&y, 5, topic);
        // B has x's honest chain, through seq 2, and y's five events. A has only the prefix up to
        // x's fork point, and none of y's events.
        let (_db, store_b) = open();
        ingest_all(&store_b, &honest_x);
        ingest_all(&store_b, &y_events);

        let (server, _router, mut results) = responder(&store_a).await;
        let client = endpoint().await;
        let outcome = sync_with(
            &client,
            loopback(&server),
            &store_b,
            topic,
            &bootstrap(),
            DEADLINE,
        )
        .await
        .unwrap();
        let answered = results.recv().await.unwrap().unwrap();

        // The batch is seq 1 and seq 2 of x, then five events of y. Seq 1 is already held as a proof
        // header, and seq 2 is above the fork point. Both are counted, and y's five still arrive.
        assert_eq!(answered.received.duplicates, 1, "seq 1 is a proof header");
        assert_eq!(answered.received.refused, 1, "seq 2 is past the fork point");
        assert_eq!(answered.received.accepted, 5, "every event of y arrives");
        assert_eq!(outcome.metrics.sent_operations, 7);
        let refs = [&y];
        assert_eq!(applied(&store_a, &refs, 4), applied(&store_b, &refs, 4));
        assert!(
            store_a
                .read(|s| Ok(s.chain(x.verifying_key().as_bytes())?.map(|c| c.next_seq)))
                .unwrap()
                == Some(1),
            "x stays at its fork point"
        );
        store_a.check_invariants().unwrap();
    }

    /// A peer that accepts and never answers. A session with it must end at its deadline.
    #[derive(Debug)]
    struct Silent;

    impl ProtocolHandler for Silent {
        async fn accept(&self, _conn: Connection) -> Result<(), AcceptError> {
            std::future::pending::<()>().await;
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_a_silent_peer_times_out() {
        let (_d, store) = open();
        let server = endpoint().await;
        let _router = Router::builder(server.clone())
            .accept(SYNC_ALPN, Silent)
            .spawn();
        let client = endpoint().await;
        let started = std::time::Instant::now();
        let error = sync_with(
            &client,
            loopback(&server),
            &store,
            group(5),
            &bootstrap(),
            Duration::from_millis(300),
        )
        .await
        .unwrap_err();

        assert!(
            matches!(error, SyncError::Timeout(_)),
            "a silent peer ends the session at its deadline, got {error}"
        );
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    /// The initiator asked for `LogSync`, and a peer answers range. The initiator must refuse it.
    #[derive(Debug)]
    struct Misanswers;

    impl ProtocolHandler for Misanswers {
        async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
            let Ok((mut send, mut recv)) = conn.accept_bi().await else {
                return Ok(());
            };
            let _ = read_frame(&mut recv).await;
            let answer = postcard::to_allocvec(&Answer::Agree(SyncMethod::Range)).unwrap();
            let _ = write_frame(&mut send, &answer).await;
            std::future::pending::<()>().await;
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_an_answer_for_another_method_is_refused_by_the_initiator() {
        let (_d, store) = open();
        let server = endpoint().await;
        let _router = Router::builder(server.clone())
            .accept(SYNC_ALPN, Misanswers)
            .spawn();
        let client = endpoint().await;
        let error = sync_with(
            &client,
            loopback(&server),
            &store,
            group(6),
            &bootstrap(),
            Duration::from_secs(10),
        )
        .await
        .unwrap_err();

        assert!(
            matches!(
                error,
                SyncError::Mismatch {
                    requested: SyncMethod::Log,
                    answered: SyncMethod::Range,
                }
            ),
            "got {error}"
        );
    }

    #[tokio::test]
    async fn test_the_initiator_chooses_and_a_responder_that_cannot_run_it_refuses_on_both_sides() {
        let topic = group(7);
        let alice = key(10);
        let events = chain_of(&alice, 3, topic);
        let (_da, store_a) = open();
        let (_db, store_b) = open();
        ingest_all(&store_a, &events);
        let before = applied(&store_b, &[&alice], 2);

        let (server, _router, mut results) = responder(&store_a).await;
        let client = endpoint().await;
        // Synced before, with no difference: the rule picks range, which a LogSyncOnly responder does not run.
        let synced = Estimate {
            chains: 1,
            difference: 0,
            last_completed_sync_ms: Some(1),
        };
        assert_eq!(choose(&synced), SyncMethod::Range);
        let error = sync_with(
            &client,
            loopback(&server),
            &store_b,
            topic,
            &synced,
            DEADLINE,
        )
        .await
        .unwrap_err();
        let answered = results.recv().await.unwrap();

        assert!(
            matches!(error, SyncError::Refused(SyncMethod::Range)),
            "got {error}"
        );
        assert!(
            matches!(answered, Err(SyncError::Refused(SyncMethod::Range))),
            "the responder refused the same method, got {answered:?}"
        );
        assert_eq!(applied(&store_b, &[&alice], 2), before, "nothing moved");
    }

    /// #179: one peer/store pair syncs under the one handshake by Log first and by
    /// Range second, and the two stores end equal after each.
    #[tokio::test]
    async fn test_one_pair_syncs_by_log_then_by_range_under_one_handshake() {
        let topic = group(8);
        let (alice, bob) = (key(11), key(12));
        let alice_events = chain_of(&alice, 6, topic);
        let bob_events = chain_of(&bob, 4, topic);
        let (_da, store_a) = open();
        let (_db, store_b) = open();
        // A has alice 0 to 4. B has alice 0 to 2.
        ingest_all(&store_a, &alice_events[..5]);
        ingest_all(&store_b, &alice_events[..3]);

        let (server, _router, mut results) =
            responder_support(&store_a, Arc::new(LogAndRange)).await;
        let client = endpoint().await;

        // First sync: unbootstrapped, so the rule picks Log.
        let logged = sync_with(
            &client,
            loopback(&server),
            &store_b,
            topic,
            &bootstrap(),
            DEADLINE,
        )
        .await
        .unwrap();
        let logged_answered = results.recv().await.unwrap().unwrap();
        assert_eq!(logged.method, SyncMethod::Log);
        assert_eq!(logged_answered.method, SyncMethod::Log);
        assert_eq!(logged.received.accepted, 2, "B lacked alice 3 and 4");
        assert_eq!(
            applied(&store_a, &[&alice], 5),
            applied(&store_b, &[&alice], 5),
            "the Log session converged the pair"
        );

        // A gains bob's chain and alice's next event. Second sync: bootstrapped with
        // a small difference, so the rule picks Range — over the same handshake.
        ingest_all(&store_a, &bob_events);
        ingest_all(&store_a, &alice_events[5..]);
        let synced = Estimate {
            chains: 2,
            difference: 0,
            last_completed_sync_ms: Some(1),
        };
        assert_eq!(choose(&synced), SyncMethod::Range);
        let ranged = sync_with(
            &client,
            loopback(&server),
            &store_b,
            topic,
            &synced,
            DEADLINE,
        )
        .await
        .unwrap();
        let ranged_answered = results.recv().await.unwrap().unwrap();
        assert_eq!(ranged.method, SyncMethod::Range);
        assert_eq!(
            ranged_answered.method,
            SyncMethod::Range,
            "both sides ran Range"
        );
        assert_eq!(
            ranged.received.accepted, 5,
            "B lacked bob 0 to 3 and alice 5"
        );
        assert_eq!(ranged_answered.received.accepted, 0, "A lacked nothing");
        let all = [&alice, &bob];
        assert_eq!(
            applied(&store_a, &all, 5),
            applied(&store_b, &all, 5),
            "the Range session converged the pair"
        );
        store_a.check_invariants().unwrap();
        store_b.check_invariants().unwrap();
    }

    /// #179: a first frame that is not a `Request` is refused as a frame error, never
    /// reinterpreted as another session shape.
    #[tokio::test]
    async fn test_a_first_frame_that_is_not_a_request_is_refused() {
        let (_d, store) = open();
        let (server, _router, mut results) = responder(&store).await;
        let client = endpoint().await;
        let conn = connect(&client, loopback(&server), SYNC_ALPN)
            .await
            .unwrap();
        let (mut send, _recv) = conn.open_bi().await.unwrap();
        write_frame(&mut send, &[0xff, 0xfe, 0xfd]).await.unwrap();
        send.finish().unwrap();
        let answered = results.recv().await.unwrap();
        assert!(
            matches!(answered, Err(SyncError::Frame(_))),
            "a foreign first frame is a frame error, got {answered:?}"
        );
    }

    /// P6.5: open pull is today's behaviour. A stranger pulls every event of the topic.
    #[tokio::test]
    async fn test_pull_policy_open_log_session_serves_a_non_member() {
        let topic = group(10);
        let alice = key(20);
        let (_da, store_a) = open();
        let (_db, store_b) = open();
        ingest_all(&store_a, &chain_of(&alice, 4, topic));
        let (server, _router, mut results) = responder(&store_a).await;
        let client = endpoint_as(testing::key(0xd9)).await;
        let outcome = sync_with(
            &client,
            loopback(&server),
            &store_b,
            topic,
            &bootstrap(),
            DEADLINE,
        )
        .await
        .unwrap();
        results.recv().await.unwrap().unwrap();

        assert_eq!(
            outcome.received.accepted, 4,
            "a stranger pulled every event"
        );
        assert_eq!(
            applied(&store_a, &[&alice], 3),
            applied(&store_b, &[&alice], 3)
        );
    }

    /// P6.5: the members option refuses a stranger before any event is sent. The stranger gets
    /// `Refuse`, the responder reports the refusal, and the stranger's store does not change.
    #[tokio::test]
    async fn test_pull_policy_members_log_refuses_a_non_member_and_sends_no_event() {
        let topic = group(11);
        let alice = key(22);
        let (_da, store_a) = open();
        let (_db, store_b) = open();
        ingest_all(&store_a, &chain_of(&alice, 4, topic));
        let before = applied(&store_b, &[&alice], 3);
        // The founder is the only member. The dialer is a stranger.
        let world = testing::docs_world(
            testing::key(0xd0).public(),
            testing::key(0xd2).public(),
            false,
        );
        let gate = PullGate::new(PullPolicy::Members, testing::WORLD, testing::live(world));
        let (server, _router, mut results) = responder_gated(&store_a, gate).await;
        let client = endpoint_as(testing::key(0xd1)).await;
        let error = sync_with(
            &client,
            loopback(&server),
            &store_b,
            topic,
            &bootstrap(),
            DEADLINE,
        )
        .await
        .unwrap_err();
        let answered = results.recv().await.unwrap();

        assert!(
            matches!(error, SyncError::Refused(SyncMethod::Log)),
            "got {error}"
        );
        assert!(
            matches!(answered, Err(SyncError::PullRefused)),
            "got {answered:?}"
        );
        assert_eq!(
            applied(&store_b, &[&alice], 3),
            before,
            "no event crossed to the stranger"
        );
    }

    /// P6.5: a member gets every event under the members option.
    #[tokio::test]
    async fn test_pull_policy_members_log_serves_a_member_everything() {
        let topic = group(12);
        let alice = key(23);
        let (_da, store_a) = open();
        let (_db, store_b) = open();
        ingest_all(&store_a, &chain_of(&alice, 4, topic));
        let world = testing::docs_world(
            testing::key(0xd0).public(),
            testing::key(0xd3).public(),
            false,
        );
        let gate = PullGate::new(PullPolicy::Members, testing::WORLD, testing::live(world));
        let (server, _router, mut results) = responder_gated(&store_a, gate).await;
        let client = endpoint_as(testing::key(0xd3)).await;
        let outcome = sync_with(
            &client,
            loopback(&server),
            &store_b,
            topic,
            &bootstrap(),
            DEADLINE,
        )
        .await
        .unwrap();
        results.recv().await.unwrap().unwrap();

        assert_eq!(
            outcome.received.accepted, 4,
            "the member pulled every event"
        );
        assert_eq!(
            applied(&store_a, &[&alice], 3),
            applied(&store_b, &[&alice], 3)
        );
    }

    /// P6.5: membership is read when each session starts. A member removed from the fold is refused
    /// on its next session, and the events it pulled before stay. Nothing new crosses.
    #[tokio::test]
    async fn test_pull_policy_members_log_refuses_a_removed_member_on_the_next_session() {
        let topic = group(13);
        let (founder, member) = (testing::key(0xd0).public(), testing::key(0xd4).public());
        let access = testing::live(testing::docs_world(founder, member, false));
        let gate = PullGate::new(PullPolicy::Members, testing::WORLD, Arc::clone(&access));
        let alice = key(24);
        let (_da, store_a) = open();
        let (_db, store_b) = open();
        ingest_all(&store_a, &chain_of(&alice, 2, topic));
        let (server, _router, mut results) = responder_gated(&store_a, gate).await;
        let client = endpoint_as(testing::key(0xd4)).await;
        sync_with(
            &client,
            loopback(&server),
            &store_b,
            topic,
            &bootstrap(),
            DEADLINE,
        )
        .await
        .unwrap();
        results.recv().await.unwrap().unwrap();
        assert_eq!(
            applied(&store_a, &[&alice], 1),
            applied(&store_b, &[&alice], 1),
            "the member pulled while it was a member"
        );

        // The removal lands, and the server gets new events for the topic.
        access.set(testing::docs_world(founder, member, true));
        let late = key(25);
        ingest_all(&store_a, &chain_of(&late, 2, topic));
        let before = applied(&store_b, &[&late], 1);
        let error = sync_with(
            &client,
            loopback(&server),
            &store_b,
            topic,
            &bootstrap(),
            DEADLINE,
        )
        .await
        .unwrap_err();
        let answered = results.recv().await.unwrap();

        assert!(
            matches!(error, SyncError::Refused(SyncMethod::Log)),
            "got {error}"
        );
        assert!(
            matches!(answered, Err(SyncError::PullRefused)),
            "got {answered:?}"
        );
        assert_eq!(
            applied(&store_b, &[&late], 1),
            before,
            "the refused pull sent nothing"
        );
    }
}
