//! `LogSync` between two stores, over iroh (P3.2).
//!
//! A session is one stream pair on [`SYNC_ALPN`]. The initiator sends a [`Request`] with the topic
//! and the method it chose ([`choose`]), and the responder answers with an [`Answer`]. If both agree
//! on `LogSync`, each side runs `p2panda-sync`'s `LogSync` on its [`SharedEventStore`] over the stream.
//!
//! Three decisions shape the session:
//!
//! - **Arrival order is the write path.** Each operation is verified and stored by
//!   [`SharedEventStore::ingest`] before `LogSync` sees it. The events are not read back from
//!   `p2panda-sync`'s broadcast channel. A slow reader of that channel loses events without an error,
//!   so the events are never taken from it.
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
use crate::event_store::range::Params;
use crate::event_store::sync::{Cut, run_range};
use crate::store_traits::{ChainLog, SharedEventStore};
use crate::sync_method::{Answer, Request, SyncMethod, SyncStats, SyncSupport, choose};
use crate::world::unix_ms;
use crate::world_net::{read_frame, write_frame};

/// The ALPN for sync sessions. This is P3.1's ID, so range sync joins as a method of the same
/// session, and the first frame of every session is the [`Request`].
pub const SYNC_ALPN: &[u8] = b"/id-sync/1";

/// How long one side may spend on a session. A peer that stops answering ends it at this deadline.
pub const SESSION_DEADLINE: Duration = Duration::from_secs(60);

/// Events the broadcast channel holds. Nothing reads them (see the module docs), so this only has
/// to be nonzero.
const EVENT_BUFFER: usize = 16;

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
    /// The responder answered a different method from the one requested.
    #[error("the peer answered {answered:?} to a request for {requested:?}")]
    Mismatch {
        /// The method the initiator asked for.
        requested: SyncMethod,
        /// The method the responder answered.
        answered: SyncMethod,
    },
    /// Both sides agreed on a method that this build does not run.
    #[error("{0:?} is agreed, but this build does not run it")]
    NotWired(SyncMethod),
    /// The event store failed.
    #[error("the event store: {0}")]
    Store(String),
    /// `LogSync` failed.
    #[error("LogSync: {0}")]
    LogSync(String),
    /// Range sync failed.
    #[error("range sync: {0}")]
    Range(String),
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
}

impl Tally {
    const fn record(&mut self, outcome: &Ingest) {
        match outcome {
            Ingest::Accepted { .. } => self.accepted += 1,
            Ingest::Duplicate { .. } => self.duplicates += 1,
            Ingest::Fork { .. } => self.forks += 1,
            Ingest::Waiting { .. } => self.held += 1,
            Ingest::Refused(_) => self.refused += 1,
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
    /// `LogSync`'s counts for the session. `sent_operations` is what this side sent. `None` for
    /// range sync, which has its own counts in [`crate::event_store::sync::Outcome`].
    pub metrics: Option<LogSyncMetrics>,
}

/// Serves sync sessions on [`SYNC_ALPN`]. Register it with
/// `Router::builder(..).accept(SYNC_ALPN, SyncProtocol::new(store, support))`.
#[derive(Clone, Debug)]
pub struct SyncProtocol {
    store: SharedEventStore,
    support: Arc<dyn SyncSupport>,
}

impl SyncProtocol {
    /// Serves `store`, running the methods that `support` allows.
    #[must_use]
    pub fn new(store: SharedEventStore, support: impl SyncSupport + 'static) -> Self {
        Self {
            store,
            support: Arc::new(support),
        }
    }
}

impl ProtocolHandler for SyncProtocol {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let mut sessions = JoinSet::new();
        while let Ok((send, recv)) = conn.accept_bi().await {
            let store = self.store.clone();
            let support = Arc::clone(&self.support);
            sessions.spawn(async move {
                if let Err(error) =
                    answer(&store, support.as_ref(), send, recv, SESSION_DEADLINE).await
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
        let (send, recv) = conn
            .open_bi()
            .await
            .map_err(|error| SyncError::Stream(error.to_string()))?;
        let mut frames = Frames::new(send, recv);
        frames.send_message(&Request { topic, method }).await?;
        match frames.next_message::<Answer>().await? {
            Answer::Agree(answered) if answered == method => {}
            Answer::Agree(answered) => {
                return Err(SyncError::Mismatch {
                    requested: method,
                    answered,
                });
            }
            Answer::Refuse => return Err(SyncError::Refused(method)),
        }
        let outcome = run_agreed(store, topic, method, &mut frames, true).await?;
        frames.finish().await;
        conn.close(0_u32.into(), b"done");
        Ok(outcome)
    };
    match timeout(deadline, session).await {
        Ok(result) => result,
        Err(_) => Err(SyncError::Timeout(deadline)),
    }
}

/// Answers one session on `send` and `recv`: reads the request, agrees or refuses, and runs the
/// agreed method. The responder does not choose the method. It runs the one the initiator asked for.
///
/// # Errors
///
/// Returns [`SyncError::Timeout`] past `deadline`, [`SyncError::Refused`] when `support` does not
/// allow the method, and an error for a failed step.
pub async fn answer(
    store: &SharedEventStore,
    support: &dyn SyncSupport,
    send: SendStream,
    recv: RecvStream,
    deadline: Duration,
) -> Result<Outcome, SyncError> {
    let mut frames = Frames::new(send, recv);
    let session = async {
        let request: Request = frames.next_message().await?;
        if !support.supports(request.method) {
            frames.send_message(&Answer::Refuse).await?;
            frames.finish().await;
            return Err(SyncError::Refused(request.method));
        }
        frames.send_message(&Answer::Agree(request.method)).await?;
        let outcome = run_agreed(store, request.topic, request.method, &mut frames, false).await?;
        frames.finish().await;
        Ok(outcome)
    };
    match timeout(deadline, session).await {
        Ok(result) => result,
        Err(_) => Err(SyncError::Timeout(deadline)),
    }
}

/// Runs the method both sides agreed to. `initiator` is true on the dialing side, which sends the
/// first message of a range session.
async fn run_agreed(
    store: &SharedEventStore,
    topic: Hash,
    method: SyncMethod,
    frames: &mut Frames,
    initiator: bool,
) -> Result<Outcome, SyncError> {
    match method {
        SyncMethod::Log => run_log_sync(store, topic, frames).await,
        SyncMethod::Range => run_range_session(store, topic, initiator, frames).await,
    }
}

/// Runs range sync (P3.1) on the agreed stream, with the local defaults for K, T, and the budget.
async fn run_range_session(
    store: &SharedEventStore,
    topic: Hash,
    initiator: bool,
    frames: &mut Frames,
) -> Result<Outcome, SyncError> {
    let outcome = run_range(
        store,
        topic,
        Params::default(),
        unix_ms(),
        initiator,
        &mut frames.send,
        &mut frames.inbound,
        Cut::Never,
    )
    .await
    .map_err(|error| SyncError::Range(format!("{error:#}")))?;
    let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    Ok(Outcome {
        method: SyncMethod::Range,
        received: Tally {
            accepted: count(outcome.accepted),
            duplicates: count(outcome.duplicates),
            held: count(outcome.waiting),
            refused: count(outcome.refused),
            forks: count(outcome.forks),
        },
        metrics: None,
    })
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

    let mut tally = Tally::default();
    let inbound = stream::unfold(&mut frames.inbound, |rx| async move {
        let item = rx.recv().await?;
        Some((item, rx))
    });
    let mut inbound = Box::pin(inbound.map(
        |frame| -> Result<LogSyncMessage<ChainLog>, SyncError> {
            let frame = frame.map_err(SyncError::Stream)?;
            let message: LogSyncMessage<ChainLog> = postcard::from_bytes(&frame)
                .map_err(|error| SyncError::Frame(error.to_string()))?;
            if let LogSyncMessage::Operation((header, body)) = &message {
                // Events carry no body here, and a body is not stored, so a body is refused.
                if body.is_some() {
                    tally.refused += 1;
                } else {
                    let outcome = store
                        .ingest(header, unix_ms())
                        .map_err(|error| SyncError::Store(error.to_string()))?;
                    tally.record(&outcome);
                }
            }
            Ok(message)
        },
    ));
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
    // The streams hold the tally and the frames. Release them before the tally is read.
    drop((outbound, inbound));
    let (_dedup, metrics) = result.map_err(|error| SyncError::LogSync(error.to_string()))?;
    Ok(Outcome {
        method: SyncMethod::Log,
        received: tally,
        metrics: Some(metrics),
    })
}

/// One stream pair. A task of its own reads the receive half and forwards whole frames over a
/// channel, so a read is never cancelled midway, whatever the caller selects over.
pub(crate) struct Frames {
    pub(crate) send: SendStream,
    pub(crate) inbound: mpsc::Receiver<Result<Vec<u8>, String>>,
    reader: JoinHandle<()>,
}

impl Frames {
    pub(crate) fn new(send: SendStream, mut recv: RecvStream) -> Self {
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

    /// Writes one handshake message as a frame.
    async fn send_message<T: Serialize>(&mut self, message: &T) -> Result<(), SyncError> {
        let bytes =
            postcard::to_allocvec(message).map_err(|error| SyncError::Frame(error.to_string()))?;
        write_frame(&mut self.send, &bytes)
            .await
            .map_err(|error| SyncError::Stream(format!("{error:#}")))
    }

    /// Reads one handshake message.
    async fn next_message<T: DeserializeOwned>(&mut self) -> Result<T, SyncError> {
        let frame = self
            .inbound
            .recv()
            .await
            .ok_or_else(|| SyncError::Stream("the stream ended during the handshake".to_owned()))?
            .map_err(SyncError::Stream)?;
        postcard::from_bytes(&frame).map_err(|error| SyncError::Frame(error.to_string()))
    }

    /// Ends our side, then waits for the peer's end, so that nothing sent is cut off by closing the
    /// connection. This runs after `LogSync` has finished on both sides, so everything each side needs
    /// has already arrived. A read error here is the peer closing the connection, which is not a
    /// failure of the session.
    pub(crate) async fn finish(mut self) {
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
    use crate::sync_method::{Estimate, LogAndRange, LogSyncOnly, PeerHistory};
    use crate::waiting::WaitingLimits;

    const DEADLINE: Duration = Duration::from_secs(30);

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
            let outcome = store.ingest(&event.bytes, 0).unwrap();
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
        results: mpsc::UnboundedSender<Result<Outcome, SyncError>>,
    }

    impl ProtocolHandler for Capture {
        async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
            while let Ok((send, recv)) = conn.accept_bi().await {
                let result = answer(&self.store, &LogSyncOnly, send, recv, DEADLINE).await;
                let _ = self.results.send(result);
            }
            Ok(())
        }
    }

    /// Serves `store` on loopback. The receiver gets the responder's result for each session.
    async fn responder(
        store: &SharedEventStore,
    ) -> (
        Endpoint,
        Router,
        mpsc::UnboundedReceiver<Result<Outcome, SyncError>>,
    ) {
        let ep = endpoint().await;
        let (results, rx) = mpsc::unbounded_channel();
        let handler = Capture {
            store: store.clone(),
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
        assert_eq!(
            outcome.metrics.clone().unwrap_or_default().sent_operations,
            5,
            "B sent carol 0 to 4"
        );

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
        assert_eq!(again.metrics.clone().unwrap_or_default().sent_operations, 0);
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

        assert_eq!(outcome.received.accepted, 210);
        assert_eq!(outcome.received.duplicates, 0);
        assert_eq!(outcome.received.refused, 0);
        let refs: Vec<&SigningKey> = authors.iter().collect();
        assert_eq!(applied(&store_a, &refs, 69), applied(&store_b, &refs, 69));
        store_b.check_invariants().unwrap();
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
        };
        let rival = sign(alice, 1, Some(honest[0].id), None, rival_ext).unwrap();
        assert!(matches!(
            store.ingest(&rival.bytes, 0).unwrap(),
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
        assert_eq!(
            outcome.metrics.clone().unwrap_or_default().sent_operations,
            7
        );
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
        // Synced before, with no difference: the rule picks range, which this build does not run.
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

    /// The responder for both methods: the production [`answer`], with each session's result sent
    /// to the test.
    #[derive(Debug)]
    struct CaptureAll {
        store: SharedEventStore,
        results: mpsc::UnboundedSender<Result<Outcome, SyncError>>,
    }

    impl ProtocolHandler for CaptureAll {
        async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
            while let Ok((send, recv)) = conn.accept_bi().await {
                let result = answer(&self.store, &LogAndRange, send, recv, DEADLINE).await;
                let _ = self.results.send(result);
            }
            Ok(())
        }
    }

    /// Serves `store` for both methods on loopback.
    async fn responder_for_both(
        store: &SharedEventStore,
    ) -> (
        Endpoint,
        Router,
        mpsc::UnboundedReceiver<Result<Outcome, SyncError>>,
    ) {
        let ep = endpoint().await;
        let (results, rx) = mpsc::unbounded_channel();
        let handler = CaptureAll {
            store: store.clone(),
            results,
        };
        let router = Router::builder(ep.clone())
            .accept(SYNC_ALPN, handler)
            .spawn();
        (ep, router, rx)
    }

    /// Thirty authors with three events each, all in `topic`. `a` has all of them. `b` has all
    /// but the last event of author 7, so the two stores differ by one event.
    fn thirty_authors(a: &SharedEventStore, b: &SharedEventStore, topic: Hash) {
        for n in 1..=30u8 {
            let events = chain_of(&key(n), 3, topic);
            ingest_all(a, &events);
            if n == 7 {
                ingest_all(b, &events[..2]);
            } else {
                ingest_all(b, &events);
            }
        }
    }

    /// The estimate for a peer that synced one event ago, with `difference` events apart.
    fn synced_once(difference: u64) -> PeerHistory {
        PeerHistory {
            last_completed_sync_ms: Some(1),
            changed_since_sync: difference,
        }
    }

    /// Thirty chains and one difference is range by the rule, so the choice is range. Two stores
    /// run range over the wire and converge, and each side answers with the method it was asked.
    #[tokio::test]
    async fn test_two_stores_choose_range_and_converge() {
        let topic = group(1);
        let (_da, store_a) = open();
        let (_db, store_b) = open();
        thirty_authors(&store_a, &store_b, topic);

        let estimate = store_b.estimate(&topic, &synced_once(1)).unwrap();
        assert_eq!(choose(&estimate), SyncMethod::Range);
        let (server, _router, mut results) = responder_for_both(&store_a).await;
        let client = endpoint().await;
        let outcome = tokio::time::timeout(
            DEADLINE,
            sync_with(
                &client,
                loopback(&server),
                &store_b,
                topic,
                &estimate,
                DEADLINE,
            ),
        )
        .await
        .expect("the session has a deadline")
        .unwrap();
        let answered = results.recv().await.unwrap().unwrap();

        assert_eq!(outcome.method, SyncMethod::Range);
        assert_eq!(
            answered.method,
            SyncMethod::Range,
            "both sides ran range sync"
        );
        assert_eq!(
            outcome.received.accepted, 1,
            "B lacked author 7's last event"
        );
        assert_eq!(answered.received.accepted, 0, "A lacked nothing");
        assert_eq!(
            outcome.received.duplicates + answered.received.duplicates,
            0
        );
        assert_eq!(outcome.received.refused + answered.received.refused, 0);
        assert_eq!(
            store_a.range_entries(&topic).unwrap(),
            store_b.range_entries(&topic).unwrap(),
            "both stores hold the same events"
        );
    }

    /// A peer never synced with is LogSync by the rule, so this session runs LogSync and converges
    /// the same stores.
    #[tokio::test]
    async fn test_two_stores_choose_log_and_converge() {
        let topic = group(1);
        let (_da, store_a) = open();
        let (_db, store_b) = open();
        thirty_authors(&store_a, &store_b, topic);

        let estimate = store_b.estimate(&topic, &PeerHistory::default()).unwrap();
        assert_eq!(choose(&estimate), SyncMethod::Log);
        let (server, _router, mut results) = responder_for_both(&store_a).await;
        let client = endpoint().await;
        let outcome = tokio::time::timeout(
            DEADLINE,
            sync_with(
                &client,
                loopback(&server),
                &store_b,
                topic,
                &estimate,
                DEADLINE,
            ),
        )
        .await
        .expect("the session has a deadline")
        .unwrap();
        let answered = results.recv().await.unwrap().unwrap();

        assert_eq!(outcome.method, SyncMethod::Log);
        assert_eq!(answered.method, SyncMethod::Log, "both sides ran LogSync");
        assert_eq!(outcome.received.accepted, 1);
        assert_eq!(outcome.received.duplicates, 0);
        assert_eq!(
            store_a.range_entries(&topic).unwrap(),
            store_b.range_entries(&topic).unwrap(),
            "both stores hold the same events"
        );
    }

    /// The selector reads the estimate, not a fixed rule: the same store chooses range for one
    /// difference and LogSync for two, and the chain count comes from the store.
    #[test]
    fn test_the_store_estimate_drives_the_choice() {
        let topic = group(1);
        let (_d, store) = open();
        for n in 1..=30u8 {
            ingest_all(&store, &chain_of(&key(n), 1, topic));
        }
        assert_eq!(
            store
                .estimate(&topic, &synced_once(1))
                .unwrap()
                .chain_count(),
            30
        );
        assert_eq!(
            choose(&store.estimate(&topic, &synced_once(1)).unwrap()),
            SyncMethod::Range,
            "one difference over thirty chains is range"
        );
        assert_eq!(
            choose(&store.estimate(&topic, &synced_once(2)).unwrap()),
            SyncMethod::Log,
            "two differences is above 0.035 of thirty chains"
        );
        assert_eq!(
            choose(&store.estimate(&topic, &PeerHistory::default()).unwrap()),
            SyncMethod::Log,
            "a peer never synced with is LogSync"
        );
    }

    /// The range sync frame cap matches the stream's: a range frame is never refused by the reader.
    #[test]
    fn test_the_range_frame_cap_is_the_sync_streams_cap() {
        assert_eq!(
            crate::event_store::range::FRAME_CAP,
            crate::world_session::MAX_FRAME_BYTES
        );
    }
}
