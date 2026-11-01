//! Range sync over the event store, on `/id-range-sync/1`.
//!
//! The ALPN is this protocol's alone (#179). Range sessions used to share `/id-sync/1`
//! with the `LogSync` handshake sessions in [`crate::log_sync`], whose first frame is a
//! `Request` - two different session shapes on one ALPN, so a peer opening with one
//! shape was misread by an acceptor expecting the other. The split is deliberate and
//! documented here rather than silent: `/id-sync/1` is the handshake session's, and a
//! range session also runs as a method of the handshake session (see
//! [`sync_on_streams`]), over the store-access seam ([`RangeStore`]) that lets
//! `log_sync`'s shared store lend one step at a time.
//! Both the dialer and the acceptor take the ALPN from [`ALPN`], so the two sides move
//! together; a peer running an older build does not serve `/id-range-sync/1`, its dial
//! fails, and `serve` retries on its next interval.
//!
//! The sync set is every stored event and every fork proof header, one key each: the author,
//! the sequence number, and the ID (see [`super::range`]). A session has four phases:
//!
//! 1. **Reconciliation.** Messages alternate as flights of frames until one side answers with
//!    only skips. The dialer opens the stream and sends the full range's fingerprint.
//! 2. **Data.** Each side knows the symmetric difference, so it knows what the peer lacks and
//!    what it holds that the peer lacks. Each side sends its events in key order: a count
//!    frame, then one frame per event. Both sides send at once.
//! 3. **Apply.** The receiver verifies each event, checks it was expected, and decides the
//!    events in batches, in one write transaction per batch, as the store's ingest does.
//! 4. **Completion.** Each side, once it has applied what it received, sends one completion
//!    frame (see [`APPLIED_FRAME`]), and ends the session only after reading the peer's.
//!    A side that has returned therefore knows the peer applied what it sent: closing the
//!    connection at that point cannot discard events the peer has received into its stream
//!    buffers but not yet decided. Without the frame, the side that finishes first can
//!    close while its last events are still in flight, and the peer's session dies with
//!    the connection — the intermittent replication loss #792's two-node test caught.
//!
//! K, T, the frame budget and the session budget are local. The frame budget only has to stay
//! under the frame cap. A session reads at most its session budget in reconciliation, and ends
//! with [`BudgetExceeded`] past it, before the store is written.
//!
//! Before it writes, each side checks the peer against its own [`PullGate`]. A refused peer gets no
//! event: the connection closes with an application code, and the peer's next read fails.

use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail, ensure};
use iroh::endpoint::{Connection, RecvStream, SendStream};
use redb::{ReadableDatabase, ReadableTable};

use super::range::{
    self, Entry, Id, Key, Params, Party, all_skip, check_bounds, decode_frame, frames,
};
use super::{
    EVENTS, EventStore, HASHES, Ingest, PROOF_HEADERS, StoreError, corrupt, decode_stored,
    position_key,
};
use crate::chain::{self, Event};
use crate::pull_policy::{PullGate, PullRefused};
use crate::store_traits::SharedEventStore;

/// The protocol ID. The wire version only; K and T are not in it. Distinct from
/// `log_sync`'s `/id-sync/1` since #179 (see the module docs): the two session shapes
/// no longer share an ALPN.
pub const ALPN: &[u8] = b"/id-range-sync/1";

/// The most messages one side writes in reconciliation before it gives up.
const MAX_FLIGHTS: u32 = 10_000;

/// Events the receiver decides in one write transaction.
const BATCH_EVENTS: usize = 256;

/// Encoded bytes the receiver decides in one write transaction. A batch fills on
/// whichever bound it reaches first, this or [`BATCH_EVENTS`]: one frame may carry
/// up to [`range::FRAME_CAP`] bytes, so a count-only bound could buffer 256 maximum
/// frames into a single transaction. The frame budget is the unit the rest of the
/// module already reasons in, so a batch holds at most about one frame budget of
/// payload. A single event larger than this bound is decided alone: it is never
/// split, and its size alone is never an error.
const BATCH_BYTES: usize = range::DEFAULT_FRAME_BUDGET;

/// Bytes in a position key: the author, then the sequence number.
const POSITION_LEN: usize = 40;

/// The application close code when the pull policy refuses the peer. [`Cut`] closes with 1.
const CLOSE_PULL_REFUSED: u32 = 2;

/// The payload of the completion frame (phase 4): a side writes it once it has applied
/// everything it received, and reads the peer's before ending the session.
const APPLIED_FRAME: &[u8] = b"applied";

/// A range session read past its session budget during reconciliation.
///
/// It is raised before the frame that would pass the budget is buffered, so `read` never exceeds
/// `budget`. The store is untouched: reconciliation writes nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the range session would read past its {budget}-byte budget ({read} bytes read so far)")]
pub struct BudgetExceeded {
    /// The session's budget, from [`Params::session_budget`].
    pub budget: usize,
    /// Bytes the session had read when it refused the frame.
    pub read: usize,
}

/// What one session did on this side.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// Messages this side wrote in reconciliation.
    pub flights: u32,
    /// Bytes this side wrote in reconciliation, with each frame's length prefix.
    pub control_bytes: usize,
    /// Bytes this side wrote in the data phase, with each frame's length prefix.
    pub data_bytes: usize,
    /// Events this side sent.
    pub sent: usize,
    /// Events this side received.
    pub received: usize,
    /// Received events that were stored and applied.
    pub accepted: usize,
    /// Received events the store already held.
    pub duplicates: usize,
    /// Received events held for a dependency.
    pub waiting: usize,
    /// Received events refused.
    pub refused: usize,
    /// Forks that received events revealed.
    pub forks: usize,
    /// Received events that were stored but voided: they count as present, but do not
    /// apply. Counted separately from the accepted count, mirroring the log sync tally.
    pub voided: usize,
    /// The symmetric difference, as this side learned it.
    pub learned: HashSet<Id>,
}

/// How a receiver's batches were decided.
#[derive(Debug, Default)]
struct Tally {
    accepted: usize,
    duplicates: usize,
    waiting: usize,
    refused: usize,
    forks: usize,
    voided: usize,
}

impl Tally {
    fn add(&mut self, outcome: &Ingest) {
        match outcome {
            Ingest::Accepted { .. } => self.accepted += 1,
            Ingest::Duplicate { .. } => self.duplicates += 1,
            Ingest::Fork { .. } => self.forks += 1,
            Ingest::Waiting { .. } => self.waiting += 1,
            Ingest::Refused(_) => self.refused += 1,
            Ingest::Voided { .. } => self.voided += 1,
        }
    }
}

/// The events one side has committed, shared between the two sides of a loopback session so
/// a cut can wait for a commit instead of guessing from a record count. Test plumbing only:
/// production sessions ([`sync_over`]) never create one.
#[derive(Clone, Debug, Default)]
pub(crate) struct CommitSignal {
    count: Arc<AtomicUsize>,
}

impl CommitSignal {
    /// Records that `events` more events were committed, in one batch.
    fn note(&self, events: usize) {
        self.count.fetch_add(events, Ordering::SeqCst);
    }

    /// How many events have been committed so far.
    fn committed(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }
}

/// Where to cut the connection, for the interruption test. The cut closes the connection.
///
/// Some variants are not constructed by any current test: they are the rest of the
/// interruption toolkit, kept for the next test that needs a flight- or record-counted
/// cut. (Until the commit-waiting variant landed, the derived `PartialEq` was what kept
/// them from reading as dead.)
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) enum Cut {
    /// No cut.
    Never,
    /// Close before this side writes message `n + 1`.
    AfterFlights(u32),
    /// Close before this side writes event `n + 1`.
    AfterRecords(usize),
    /// Close before this side writes event `records + 1`, but only once the peer has
    /// committed `committed` events, as reported through `signal`.
    ///
    /// Closing a connection discards the stream data the peer has not read yet, so a cut
    /// counted in records alone can land before the peer has read a whole batch: under
    /// load the batch then never commits, and the interruption test sees 0 committed
    /// events instead of one batch. Waiting for the commit makes the cut deterministic.
    /// The wait is capped (see `wait_for_commit`): if the commit never comes, the cut
    /// happens anyway and the test's assertions report the state the peer reached.
    AfterRecordsWhenCommitted {
        /// Events to write before the cut, as in [`Cut::AfterRecords`].
        records: usize,
        /// The commits the peer reports as it flushes each batch.
        signal: CommitSignal,
        /// How many committed events to wait for before cutting.
        committed: usize,
    },
}

/// Runs one range session over an open connection. The dialer opens the stream and passes
/// `initiator = true`. `now_ms` is the clock the store's waiting rules use. `deadline` bounds
/// the whole session, from opening the stream to the last event, as LogSync's does.
///
/// # Errors
///
/// Returns an error if the peer breaks the protocol, a frame is malformed or too large, an
/// event fails verification, the session does not end within the flight limit or within
/// `deadline`, the connection fails, or `gate` refuses the peer, with [`PullRefused`].
pub async fn sync_over(
    store: &mut EventStore,
    conn: &Connection,
    initiator: bool,
    params: Params,
    now_ms: u64,
    deadline: Duration,
    gate: &PullGate,
) -> Result<Outcome> {
    match tokio::time::timeout(
        deadline,
        run(
            store,
            conn,
            initiator,
            params,
            now_ms,
            Cut::Never,
            gate,
            None,
            BATCH_BYTES,
        ),
    )
    .await
    {
        Ok(result) => result,
        Err(_elapsed) => bail!("the range session did not finish within {deadline:?}"),
    }
}

/// The store access one range session needs, one step at a time.
///
/// A session touches its store in short steps — the sync set once at the start, one
/// event's bytes per offer, one ingest batch per flush — and never needs the store
/// across an await: [`run_session`] guards the store with a `Mutex` it locks per step.
/// That is the seam that lets a session run over [`SharedEventStore`] (#179): its
/// implementation takes the shared `RwLock` per step, read for the queries and write
/// for one batch, instead of holding a write guard for the session's whole life, which
/// would block every other user of the store behind a network wait.
pub(crate) trait RangeStore {
    /// The sync set, as [`EventStore::sync_entries`].
    fn sync_entries(&self) -> Result<Vec<(Key, Id)>>;

    /// The bytes of one event, as [`EventStore::sync_bytes`].
    fn sync_bytes(&self, id: &Id) -> Result<Option<Vec<u8>>>;

    /// Decides one batch in one write transaction, as [`EventStore::ingest_events`].
    fn ingest_events(&mut self, events: Vec<Event>, now_ms: u64) -> Result<Vec<Ingest>>;
}

impl RangeStore for EventStore {
    fn sync_entries(&self) -> Result<Vec<(Key, Id)>> {
        Ok(EventStore::sync_entries(self)?)
    }

    fn sync_bytes(&self, id: &Id) -> Result<Option<Vec<u8>>> {
        Ok(EventStore::sync_bytes(self, id)?)
    }

    fn ingest_events(&mut self, events: Vec<Event>, now_ms: u64) -> Result<Vec<Ingest>> {
        Ok(EventStore::ingest_events(self, events, now_ms)?)
    }
}

impl RangeStore for SharedEventStore {
    fn sync_entries(&self) -> Result<Vec<(Key, Id)>> {
        Ok(self.range_entries()?)
    }

    fn sync_bytes(&self, id: &Id) -> Result<Option<Vec<u8>>> {
        Ok(self.range_bytes(id)?)
    }

    fn ingest_events(&mut self, events: Vec<Event>, now_ms: u64) -> Result<Vec<Ingest>> {
        Ok(self.range_ingest_events(events, now_ms)?)
    }
}

/// Runs one range session on an already-open stream pair, as a method agreed under
/// `log_sync`'s `Request`/`Answer` handshake (#179). The caller owns the handshake, the
/// pull-gate check and the deadline: the responder's gate already decided in
/// `log_sync::answer`, exactly as for a `Log` session, and the caller's deadline bounds
/// this call. The sync set is the store's whole set, as in [`sync_over`]: range sync
/// has no topic subset.
pub(crate) async fn sync_on_streams<S: RangeStore>(
    store: &mut S,
    send: &mut SendStream,
    recv: &mut RecvStream,
    initiator: bool,
    params: Params,
    now_ms: u64,
) -> Result<Outcome> {
    run_session(
        store,
        send,
        recv,
        initiator,
        params,
        now_ms,
        Cut::Never,
        None,
        None,
        BATCH_BYTES,
    )
    .await
}

impl EventStore {
    /// The sync set, sorted by key: each stored event, and each fork proof header, once.
    ///
    /// # Errors
    ///
    /// Returns a redb error if the read fails, or [`StoreError::Corrupt`].
    pub(crate) fn sync_entries(&self) -> Result<Vec<(Key, Id)>, StoreError> {
        let txn = self.db.begin_read()?;
        let hashes = txn.open_table(HASHES)?;
        let headers = txn.open_table(PROOF_HEADERS)?;
        let mut entries: Vec<(Key, Id)> = Vec::new();
        for row in hashes.iter()? {
            let (id, position) = row?;
            let id = id_of(id.value())?;
            entries.push((sync_key(position.value(), &id)?, id));
        }
        for row in headers.iter()? {
            let (_id, bytes) = row?;
            let header = decode_stored(bytes.value())?;
            let author = *header.header.verifying_key.as_bytes();
            let position = position_key(&author, header.header.seq_num);
            let id = *header.id.as_bytes();
            entries.push((sync_key(&position, &id)?, id));
        }
        entries.sort_unstable_by_key(|e| e.0);
        entries.dedup_by(|a, b| a.0 == b.0);
        Ok(entries)
    }

    /// The encoded bytes of the event with this ID, if the store holds it as a stored event or as
    /// a fork proof header, whichever proof names it now.
    ///
    /// # Errors
    ///
    /// Returns a redb error if the read fails, or [`StoreError::Corrupt`].
    pub(crate) fn sync_bytes(&self, id: &Id) -> Result<Option<Vec<u8>>, StoreError> {
        let txn = self.db.begin_read()?;
        let hashes = txn.open_table(HASHES)?;
        if let Some(position) = hashes.get(id.as_slice())? {
            let events = txn.open_table(EVENTS)?;
            let stored = events.get(position.value())?;
            return Ok(stored.map(|bytes| bytes.value().to_vec()));
        }
        let headers = txn.open_table(PROOF_HEADERS)?;
        let kept = headers.get(id.as_slice())?;
        Ok(kept.map(|bytes| bytes.value().to_vec()))
    }
}

fn id_of(bytes: &[u8]) -> Result<Id, StoreError> {
    bytes
        .try_into()
        .map_err(|_err| corrupt("an event ID is not 32 bytes"))
}

/// The sync key of an event: its position (author and sequence number), then its ID.
fn sync_key(position: &[u8], id: &Id) -> Result<Key, StoreError> {
    if position.len() != POSITION_LEN {
        return Err(corrupt("a position key has the wrong length"));
    }
    let mut key = [0u8; range::KEY_LEN];
    key[..POSITION_LEN].copy_from_slice(position);
    key[POSITION_LEN..].copy_from_slice(id);
    Ok(key)
}

async fn write_frame(send: &mut SendStream, payload: &[u8]) -> Result<usize> {
    ensure!(
        payload.len() <= range::FRAME_CAP,
        "frame of {} bytes is over the cap",
        payload.len()
    );
    let len = u32::try_from(payload.len())?;
    send.write_all(&len.to_be_bytes()).await?;
    send.write_all(payload).await?;
    Ok(4 + payload.len())
}

async fn read_frame(recv: &mut RecvStream) -> Result<Vec<u8>> {
    let mut len = [0u8; 4];
    recv.read_exact(&mut len).await?;
    let len = u32::from_be_bytes(len) as usize;
    ensure!(len <= range::FRAME_CAP, "frame announces {len} bytes");
    let mut buf = vec![0u8; len];
    recv.read_exact(&mut buf).await?;
    Ok(buf)
}

/// Waits until `signal` reports at least `committed` events, capped at 30 s. The cap keeps
/// a peer that never commits from hanging the session: the cut then happens anyway, and
/// the caller's assertions report the state the peer actually reached.
async fn wait_for_commit(signal: &CommitSignal, committed: usize) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while signal.committed() < committed {
        if tokio::time::Instant::now() >= deadline {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Writes one message as frames, or closes the connection if the cut is due.
async fn write_flight(
    send: &mut SendStream,
    conn: Option<&Connection>,
    out: &mut Outcome,
    msg: &[Entry],
    params: Params,
    cut: &Cut,
) -> Result<()> {
    if matches!(cut, Cut::AfterFlights(flights) if *flights == out.flights) {
        if let Some(conn) = conn {
            conn.close(1u32.into(), b"cut");
        }
        bail!("cut before flight {}", out.flights + 1);
    }
    for frame in frames(msg, params.frame_budget())? {
        out.control_bytes += write_frame(send, &frame).await?;
    }
    out.flights += 1;
    Ok(())
}

/// The bytes one session has read in reconciliation, against its budget.
#[derive(Debug)]
struct ReadBudget {
    budget: usize,
    read: usize,
}

impl ReadBudget {
    /// Charges `cost` bytes. A charge that would pass the budget is refused, and nothing is
    /// charged.
    fn charge(&mut self, cost: usize) -> Result<(), BudgetExceeded> {
        let total = self.read.saturating_add(cost);
        if total > self.budget {
            return Err(BudgetExceeded {
                budget: self.budget,
                read: self.read,
            });
        }
        self.read = total;
        Ok(())
    }
}

/// Reads one frame of a flight. Its wire bytes are charged before its body is buffered, and its
/// entries before it is decoded, so a frame that would pass the budget is refused unbuffered.
async fn read_flight_frame(
    recv: &mut RecvStream,
    budget: &mut ReadBudget,
) -> Result<(bool, Vec<Entry>)> {
    let mut len = [0u8; 4];
    recv.read_exact(&mut len).await?;
    let len = u32::from_be_bytes(len) as usize;
    ensure!(len <= range::FRAME_CAP, "frame announces {len} bytes");
    budget.charge(4 + len)?;
    let mut buf = vec![0u8; len];
    recv.read_exact(&mut buf).await?;
    let entries = range::frame_entry_count(&buf)?;
    budget.charge(entries.saturating_mul(range::ENTRY_CHARGE))?;
    decode_frame(&buf)
}

/// Reads frames until one without the `more` flag, and returns the message's entries. The
/// flight is held whole, so the session's budget bounds it.
async fn read_flight(recv: &mut RecvStream, budget: &mut ReadBudget) -> Result<Vec<Entry>> {
    let mut entries = Vec::new();
    loop {
        let (more, mut part) = read_flight_frame(recv, budget).await?;
        entries.append(&mut part);
        if !more {
            break;
        }
    }
    check_bounds(&entries)?;
    Ok(entries)
}

/// Locks the store for one step. The lock is never held across an await.
fn lock<'a, 'b, S: RangeStore>(store: &'a Mutex<&'b mut S>) -> Result<MutexGuard<'a, &'b mut S>> {
    store
        .lock()
        .map_err(|_err| anyhow!("the event store lock is poisoned"))
}

/// Decides one batch in one write transaction, and tallies each outcome. A fork is an outcome
/// like any other: it is recorded, and the events after it in the batch are still decided. Only
/// an error from the store ends the batch, and then nothing in it commits.
fn apply_batch<S: RangeStore>(
    store: &mut S,
    events: Vec<Event>,
    now_ms: u64,
    tally: &mut Tally,
) -> Result<()> {
    for outcome in store.ingest_events(events, now_ms)? {
        tally.add(&outcome);
    }
    Ok(())
}

async fn run(
    store: &mut EventStore,
    conn: &Connection,
    initiator: bool,
    params: Params,
    now_ms: u64,
    cut: Cut,
    gate: &PullGate,
    commits: Option<CommitSignal>,
    batch_bytes: usize,
) -> Result<Outcome> {
    let (mut send, mut recv) = if initiator {
        conn.open_bi().await?
    } else {
        conn.accept_bi().await?
    };
    // The pull policy decides before a byte is written, so a refused peer gets no event.
    if !gate.admits(&conn.remote_id()) {
        conn.close(CLOSE_PULL_REFUSED.into(), b"pull refused");
        return Err(PullRefused(gate.world().to_owned()).into());
    }
    run_session(
        store,
        &mut send,
        &mut recv,
        initiator,
        params,
        now_ms,
        cut,
        Some(conn),
        commits,
        batch_bytes,
    )
    .await
}

/// One range session on an open stream pair. `conn` is the session's connection when
/// the caller has one (the cut closes it); a session running under `log_sync`'s
/// handshake passes `None` — its cuts are `Cut::Never`, so only the close is skipped,
/// and dropping the stream ends that side anyway.
async fn run_session<S: RangeStore>(
    store: &mut S,
    send: &mut SendStream,
    recv: &mut RecvStream,
    initiator: bool,
    params: Params,
    now_ms: u64,
    cut: Cut,
    conn: Option<&Connection>,
    commits: Option<CommitSignal>,
    batch_bytes: usize,
) -> Result<Outcome> {
    let mut out = Outcome::default();
    let party = Party::new(store.sync_entries()?)?;
    let mut budget = ReadBudget {
        budget: params.session_budget(),
        read: 0,
    };

    // 1. Reconciliation. The same loop as S7's session, with each message as frames.
    let mut learned: Vec<Id> = Vec::new();
    if initiator {
        write_flight(send, conn, &mut out, &[party.full_range()], params, &cut).await?;
    }
    loop {
        let incoming = read_flight(recv, &mut budget).await?;
        if all_skip(&incoming) {
            break;
        }
        let reply = party.respond(&incoming, params, &mut learned);
        let last = all_skip(&reply);
        write_flight(send, conn, &mut out, &reply, params, &cut).await?;
        if last {
            break;
        }
        ensure!(
            out.flights < MAX_FLIGHTS,
            "reconciliation did not terminate"
        );
    }

    // 2. Data. Both sides hold the same symmetric difference.
    let learned: HashSet<Id> = learned.into_iter().collect();
    let mine: HashSet<Id> = party.ids().iter().copied().collect();
    let mut need: HashSet<Id> = learned
        .iter()
        .filter(|id| !mine.contains(*id))
        .copied()
        .collect();
    let offers: Vec<Id> = party
        .ids()
        .iter()
        .filter(|id| learned.contains(*id))
        .copied()
        .collect();
    out.learned = learned;
    let store = Mutex::new(store);

    // The record cut, resolved once: how many events to write before closing, and the
    // commit to wait for first, if the cut is one that waits.
    let cut_records: Option<(usize, Option<(CommitSignal, usize)>)> = match &cut {
        Cut::AfterRecords(records) => Some((*records, None)),
        Cut::AfterRecordsWhenCommitted {
            records,
            signal,
            committed,
        } => Some((*records, Some((signal.clone(), *committed)))),
        Cut::Never | Cut::AfterFlights(_) => None,
    };

    // The completion handshake (phase 4): the receiving half signals once it has
    // applied what it received; only then does the sending half write its completion
    // frame, and the receiving half ends the session only after reading the peer's.
    // `notify_one` stores its permit, so a signal sent before the sending half waits
    // is not lost.
    let applied = tokio::sync::Notify::new();

    let sending = async {
        let count = u32::try_from(offers.len())?;
        let mut bytes = write_frame(&mut *send, &count.to_be_bytes()).await?;
        for (sent, id) in offers.iter().enumerate() {
            if let Some((records, wait)) = &cut_records {
                if sent == *records {
                    if let Some((signal, committed)) = wait {
                        wait_for_commit(signal, *committed).await;
                    }
                    if let Some(conn) = conn {
                        conn.close(1u32.into(), b"cut");
                    }
                    bail!("cut before event {}", sent + 1);
                }
            }
            let record = lock(&store)?
                .sync_bytes(id)?
                .context("an offered event is no longer stored")?;
            bytes += write_frame(&mut *send, &record).await?;
        }
        applied.notified().await;
        bytes += write_frame(&mut *send, APPLIED_FRAME).await?;
        send.finish()?;
        anyhow::Ok(bytes)
    };

    let receiving = async {
        let received = receive_data(recv, &store, &mut need, now_ms, &commits, batch_bytes).await;
        // Signal on every exit, error included: the sending half waits for this
        // signal before it can finish, and a failed receive must not hold the
        // session open until its deadline.
        applied.notify_one();
        let (count, tally) = received?;
        // Applied: this side's completion frame is now released; wait for the
        // peer's — the session ends only once the peer has applied what this
        // side sent.
        let completion = read_frame(&mut *recv).await?;
        ensure!(
            completion == APPLIED_FRAME,
            "the peer ended its data phase without a completion frame"
        );
        anyhow::Ok((count, tally))
    };

    let (sent_bytes, received) = tokio::join!(sending, receiving);
    out.data_bytes = sent_bytes?;
    out.sent = offers.len();
    let (count, tally) = received?;
    out.received = count;
    out.accepted = tally.accepted;
    out.duplicates = tally.duplicates;
    out.waiting = tally.waiting;
    out.refused = tally.refused;
    out.forks = tally.forks;
    out.voided = tally.voided;
    Ok(out)
}

/// The receiving half of the data phase: reads the peer's count frame and that many
/// event frames, deciding each batch as it fills, and returns how many events arrived
/// and how they were decided. The caller owns the completion handshake (phase 4).
async fn receive_data<S: RangeStore>(
    recv: &mut RecvStream,
    store: &Mutex<&mut S>,
    need: &mut HashSet<Id>,
    now_ms: u64,
    commits: &Option<CommitSignal>,
    batch_bytes: usize,
) -> Result<(usize, Tally)> {
    let count_frame = read_frame(recv).await?;
    let count: [u8; 4] = count_frame
        .as_slice()
        .try_into()
        .map_err(|_err| anyhow!("count frame is not 4 bytes"))?;
    let count = u32::from_be_bytes(count) as usize;
    ensure!(
        count == need.len(),
        "peer offers {count} events, and {} are missing here",
        need.len()
    );
    let mut pending: Vec<Event> = Vec::with_capacity(BATCH_EVENTS);
    let mut pending_bytes: usize = 0;
    let mut tally = Tally::default();
    for _ in 0..count {
        let record = read_frame(recv).await?;
        let record_len = record.len();
        let event = chain::decode(&record)?;
        ensure!(
            need.remove(event.id.as_bytes()),
            "an event arrived that was not expected, or arrived twice"
        );
        // A batch is bounded by bytes as well as by count: flush before this
        // event if it would carry the batch past the byte bound. An empty batch
        // always takes the event, so one larger than the bound forms a batch of
        // its own rather than being refused.
        if !pending.is_empty() && pending_bytes + record_len > batch_bytes {
            flush(store, &mut pending, now_ms, &mut tally, commits)?;
            pending_bytes = 0;
        }
        pending.push(event);
        pending_bytes += record_len;
        if pending.len() == BATCH_EVENTS {
            flush(store, &mut pending, now_ms, &mut tally, commits)?;
            pending_bytes = 0;
        }
    }
    flush(store, &mut pending, now_ms, &mut tally, commits)?;
    Ok((count, tally))
}

fn flush<S: RangeStore>(
    store: &Mutex<&mut S>,
    pending: &mut Vec<Event>,
    now_ms: u64,
    tally: &mut Tally,
    commits: &Option<CommitSignal>,
) -> Result<()> {
    if pending.is_empty() {
        return Ok(());
    }
    let events = std::mem::take(pending);
    let committed = events.len();
    let mut guard = lock(store)?;
    apply_batch(&mut **guard, events, now_ms, tally)?;
    // Only a batch that committed is reported: an error above ends the batch, and
    // nothing in it commits.
    if let Some(signal) = commits {
        signal.note(committed);
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::chain::ChainExt;
    use crate::pull_policy::{PullPolicy, PullRefused, testing};
    use crate::waiting::WaitingLimits;
    use p2panda_core::{Hash, SigningKey};
    use std::sync::Arc;

    use iroh::endpoint::{Endpoint, presets};
    use iroh::{EndpointAddr, RelayMode, TransportAddr};
    use iroh_base::SecretKey;
    use std::net::Ipv4Addr;

    const NOW: u64 = 1_000_000_000;
    /// An endpoint on localhost, with no relay and no address lookup. Dialed by address only.
    async fn endpoint(alpns: Vec<Vec<u8>>) -> Result<Endpoint> {
        Ok(Endpoint::builder(presets::Minimal)
            .alpns(alpns)
            .relay_mode(RelayMode::Disabled)
            .bind()
            .await?)
    }

    /// As [`endpoint`], under `key`, so a test knows the identity before it dials.
    async fn endpoint_as(key: SecretKey, alpns: Vec<Vec<u8>>) -> Result<Endpoint> {
        Ok(Endpoint::builder(presets::Minimal)
            .secret_key(key)
            .alpns(alpns)
            .relay_mode(RelayMode::Disabled)
            .bind()
            .await?)
    }

    /// The address of `ep` on localhost: every bound socket, with an unspecified IP replaced by
    /// the loopback address.
    fn loopback_addr(ep: &Endpoint) -> EndpointAddr {
        EndpointAddr::from_parts(
            ep.id(),
            ep.bound_sockets().iter().map(|addr| {
                let ip = if addr.ip().is_unspecified() {
                    Ipv4Addr::LOCALHOST.into()
                } else {
                    addr.ip()
                };
                TransportAddr::Ip(std::net::SocketAddr::new(ip, addr.port()))
            }),
        )
    }

    fn open() -> (tempfile::TempDir, EventStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = EventStore::open(dir.path(), WaitingLimits::default(), NOW).unwrap();
        (dir, store)
    }

    fn signer(n: u64) -> SigningKey {
        SigningKey::from_bytes(Hash::digest(n.to_be_bytes()).as_bytes())
    }

    /// The first `len` events of chain `n`. Each names the one before, and has a body if `body`.
    fn chain_of(n: u64, len: u32, body: &[u8]) -> Vec<Event> {
        let key = signer(n);
        let mut events: Vec<Event> = Vec::new();
        for seq in 0..len {
            let backlink = events.last().map(|e| e.id);
            let ext = ChainExt {
                kind: "message".to_owned(),
                group: None,
                after: Vec::new(),
                tag: None,
                document: None,
                removes: None,
                chat: None,
            };
            events.push(chain::sign(&key, seq, backlink, Some(body), ext).unwrap());
        }
        events
    }

    /// Stores `events` in order, and checks each one is applied.
    fn fill(store: &mut EventStore, events: &[Event]) {
        let outcomes = store.ingest_events(events.to_vec(), NOW).unwrap();
        assert!(
            outcomes
                .iter()
                .all(|o| matches!(o, Ingest::Accepted { .. })),
            "{outcomes:?}"
        );
    }

    /// The chains of `lengths` (chain number, length), with a prefix of each chain stored.
    fn populate(store: &mut EventStore, lengths: &[(u64, u32)]) {
        for &(n, len) in lengths {
            fill(store, &chain_of(n, len, b"body"));
        }
    }

    fn ids(store: &EventStore) -> HashSet<Id> {
        store
            .sync_entries()
            .unwrap()
            .into_iter()
            .map(|e| e.1)
            .collect()
    }

    /// Runs one session on each side of a loopback connection. The dialer is `a`. Both sessions
    /// run in this task, so each store is borrowed only by its own session.
    async fn loopback(
        a: &mut EventStore,
        b: &mut EventStore,
        pa: Params,
        pb: Params,
        cut: Cut,
    ) -> Result<(Result<Outcome>, Result<Outcome>)> {
        loopback_gated(a, b, pa, pb, cut, &testing::open_gate(), testing::key(0xd9)).await
    }

    /// [`loopback`] with the acceptor's pull gate, and the dialer under `dialer`, so a test can name
    /// the dialer's identity in the world. The dialer's own gate is open.
    async fn loopback_gated(
        a: &mut EventStore,
        b: &mut EventStore,
        pa: Params,
        pb: Params,
        cut: Cut,
        gate: &PullGate,
        dialer: SecretKey,
    ) -> Result<(Result<Outcome>, Result<Outcome>)> {
        loopback_gated_bounded(a, b, pa, pb, cut, gate, dialer, BATCH_BYTES).await
    }

    /// loopback_gated with the receiver byte bound named, so a test can set the
    /// bound a batch fills at to the size of the small events a fixture can build.
    /// Production sessions always run with BATCH_BYTES; the largest valid event a
    /// fixture can sign is about 10 KiB (a tag.clear naming 256 claims), so the
    /// production bound cannot be reached by signed fixtures at all.
    async fn loopback_gated_bounded(
        a: &mut EventStore,
        b: &mut EventStore,
        pa: Params,
        pb: Params,
        cut: Cut,
        gate: &PullGate,
        dialer: SecretKey,
        batch_bytes: usize,
    ) -> Result<(Result<Outcome>, Result<Outcome>)> {
        let open = testing::open_gate();
        let server = endpoint(vec![ALPN.to_vec()]).await?;
        let client = endpoint_as(dialer, vec![]).await?;
        let target = loopback_addr(&server);
        // A cut that waits for a commit is fed by the acceptor: it reports each batch it
        // commits into the cut's signal, so the dialer cuts on committed state.
        let acceptor_commits = match &cut {
            Cut::AfterRecordsWhenCommitted { signal, .. } => Some(signal.clone()),
            _ => None,
        };
        let accepting = async {
            let incoming = server
                .accept()
                .await
                .context("endpoint closed before a connection")?;
            let conn = incoming.await?;
            let out = run(
                b,
                &conn,
                false,
                pb,
                NOW,
                Cut::Never,
                gate,
                acceptor_commits,
                batch_bytes,
            )
            .await;
            anyhow::Ok((out, conn))
        };
        let dialing = async {
            let conn = client.connect(target, ALPN).await?;
            let out = run(a, &conn, true, pa, NOW, cut, &open, None, batch_bytes).await;
            anyhow::Ok((out, conn))
        };
        let (dialed, accepted) = tokio::time::timeout(Duration::from_secs(300), async {
            tokio::join!(dialing, accepting)
        })
        .await
        .context("loopback sync timed out after 300 s")?;
        let (dialer_out, dial_conn) = dialed?;
        let (acceptor_out, accept_conn) = accepted?;
        dial_conn.close(0u32.into(), b"done");
        accept_conn.close(0u32.into(), b"done");
        client.close().await;
        server.close().await;
        Ok((dialer_out, acceptor_out))
    }

    /// The symmetric difference of two stores' IDs.
    fn difference(a: &EventStore, b: &EventStore) -> HashSet<Id> {
        let (a, b) = (ids(a), ids(b));
        a.symmetric_difference(&b).copied().collect()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_empty_store_is_readable_and_two_empty_stores_agree() {
        let (_da, mut a) = open();
        let (_db, mut b) = open();
        assert!(
            a.sync_entries().unwrap().is_empty(),
            "a fresh store reads as empty"
        );
        let (da, db) = loopback(
            &mut a,
            &mut b,
            Params::default(),
            Params::default(),
            Cut::Never,
        )
        .await
        .unwrap();
        let (da, db) = (da.unwrap(), db.unwrap());
        assert_eq!(da.flights, 1);
        assert_eq!(db.flights, 1);
        assert!(da.learned.is_empty() && db.learned.is_empty());
        assert_eq!(da.received + db.received, 0);
    }

    #[test]
    fn test_sync_set_holds_stored_events_and_fork_proof_headers_once() {
        let (_d, mut store) = open();
        let events = chain_of(1, 3, b"x");
        fill(&mut store, &events);
        // A rival at sequence 1: a different body, so a different ID, at a stored position.
        let rival = chain::sign(
            &signer(1),
            1,
            Some(events[0].id),
            Some(b"other"),
            ChainExt {
                kind: "message".to_owned(),
                group: None,
                after: Vec::new(),
                tag: None,
                document: None,
                removes: None,
                chat: None,
            },
        )
        .unwrap();
        let outcome = store.ingest_events(vec![rival.clone()], NOW).unwrap();
        assert!(matches!(outcome[0], Ingest::Fork { .. }), "{outcome:?}");
        let sync = store.sync_entries().unwrap();
        // The stored events (0, 1, and 2 voided above the fork point), and the rival, which is
        // now a proof header. The stored event at sequence 1 is also a proof header, so it is
        // listed once.
        let set: HashSet<Id> = sync.iter().map(|e| e.1).collect();
        assert_eq!(set.len(), 4);
        assert!(set.contains(rival.id.as_bytes()));
        assert_eq!(
            store.sync_bytes(rival.id.as_bytes()).unwrap().unwrap(),
            rival.bytes
        );
        assert_eq!(
            store.sync_bytes(events[2].id.as_bytes()).unwrap().unwrap(),
            events[2].bytes
        );
        assert!(store.sync_bytes(&[9; 32]).unwrap().is_none());
    }

    /// P3.1a: a fork in a batch does not end the batch. The 20 unrelated events after it are
    /// decided, and the fork is recorded.
    #[test]
    fn test_a_fork_in_a_batch_is_isolated() {
        let (_d, mut store) = open();
        let original = chain_of(1, 1, b"x");
        fill(&mut store, &original);
        let rival = chain::sign(
            &signer(1),
            0,
            None,
            Some(b"rival"),
            ChainExt {
                kind: "message".to_owned(),
                group: None,
                after: Vec::new(),
                tag: None,
                document: None,
                removes: None,
                chat: None,
            },
        )
        .unwrap();
        let mut batch = vec![rival];
        for n in 2..22 {
            batch.extend(chain_of(n, 1, b"unrelated"));
        }
        let mut tally = Tally::default();
        apply_batch(&mut store, batch, NOW, &mut tally).unwrap();
        assert_eq!(tally.forks, 1);
        assert_eq!(
            tally.accepted, 20,
            "the 20 unrelated events survive the fork"
        );
        // The original, the rival that proves the fork, and the 20 unrelated events.
        assert_eq!(store.sync_entries().unwrap().len(), 22);
    }

    /// A range session counts every disposition it decides, including the arrivals it does
    /// not store (Codex finding on PR 2214: the tally had a voided count the outcome dropped).
    ///
    /// The shape matters. An event above its author's fork point that arrives *after* the
    /// fork is refused as frozen (`ingest.rs`, "Known limit: a rival refused as frozen"), so
    /// it is counted as refused, and the voided count stays zero: voided labels an event the
    /// store keeps without applying, not an arrival it turns away.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_a_range_session_counts_frozen_arrivals_as_refused() {
        let (_da, mut a) = open();
        let (_db, mut b) = open();
        let events = chain_of(1, 5, b"x");
        fill(&mut a, &events);
        fill(&mut b, &events[..3]);
        // B holds a rival at sequence 1, so author 1 has forked in B and the
        // events from sequence 1 on are voided there.
        let rival = chain::sign(
            &signer(1),
            1,
            Some(events[0].id),
            Some(b"other"),
            ChainExt {
                kind: "message".to_owned(),
                group: None,
                after: Vec::new(),
                tag: None,
                document: None,
                removes: None,
                chat: None,
            },
        )
        .unwrap();
        let outcome = b.ingest_events(vec![rival], NOW).unwrap();
        assert!(matches!(outcome[0], Ingest::Fork { .. }), "{outcome:?}");
        let (da, db) = loopback(
            &mut a,
            &mut b,
            Params::default(),
            Params::default(),
            Cut::Never,
        )
        .await
        .unwrap();
        let (da, db) = (da.unwrap(), db.unwrap());
        assert_eq!(da.sent, 2, "events 4 and 5 are missing on B's side");
        assert_eq!(
            db.received, 2,
            "both above the fork point are received and turned away: {db:?}"
        );
        assert_eq!(
            db.refused, 2,
            "events 4 and 5 arrive above the fork point, so they are frozen: {db:?}"
        );
        assert_eq!(db.accepted, 0, "a frozen arrival is not accepted: {db:?}");
        assert_eq!(
            db.voided, 0,
            "voided counts stored-but-unapplied events, not refused arrivals: {db:?}"
        );
    }

    /// Prefix lengths of two sides' chains, so each side holds some events the other lacks.
    fn lengths(seed: u32) -> (Vec<(u64, u32)>, Vec<(u64, u32)>) {
        let a = (1..=24u64)
            .map(|c| (c, (c as u32 * (7 + seed)) % 19))
            .collect();
        let b = (1..=24u64)
            .map(|c| (c, (c as u32 * (5 + seed) + 3) % 23))
            .collect();
        (a, b)
    }

    /// Loopback sessions over iroh, with K and T on each side. Both stores end equal, and each
    /// side learns the exact symmetric difference, with no duplicate and no refusal.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_loopback_converges_for_mixed_k_and_t() {
        let cases = [
            (16usize, 16usize, 16usize, 16usize),
            (2, 1, 32, 4),
            (4, 8, 16, 16),
            (32, 1, 2, 4),
        ];
        for (seed, (ka, ta, kb, tb)) in cases.into_iter().enumerate() {
            let (la, lb) = lengths(seed as u32);
            let (_da, mut a) = open();
            let (_db, mut b) = open();
            populate(&mut a, &la);
            populate(&mut b, &lb);
            let truth = difference(&a, &b);
            let union: HashSet<Id> = ids(&a).union(&ids(&b)).copied().collect();
            let label = format!("A K={ka} T={ta}, B K={kb} T={tb}");

            let (da, db) = loopback(
                &mut a,
                &mut b,
                Params::new(ka, ta).unwrap(),
                Params::new(kb, tb).unwrap(),
                Cut::Never,
            )
            .await
            .unwrap();
            let (da, db) = (da.unwrap(), db.unwrap());

            assert_eq!(ids(&a), union, "{label}: A holds the union");
            assert_eq!(ids(&b), union, "{label}: B holds the union");
            assert_eq!(da.learned, truth, "{label}: A learns the exact difference");
            assert_eq!(db.learned, truth, "{label}: B learns the exact difference");
            assert_eq!(da.accepted, db.sent, "{label}");
            assert_eq!(db.accepted, da.sent, "{label}");
            assert_eq!(
                da.duplicates + db.duplicates,
                0,
                "{label}: no duplicate inserts"
            );
            assert_eq!(
                da.refused + db.refused + da.waiting + db.waiting,
                0,
                "{label}"
            );
            assert_eq!(da.forks + db.forks, 0, "{label}");
        }
    }

    /// The commit signal is shared between the two sides of a loopback session: every clone
    /// sees the batches each clone reports.
    #[test]
    fn test_commit_signal_counts_the_batches_each_clone_reports() {
        let signal = CommitSignal::default();
        let clone = signal.clone();
        assert_eq!(signal.committed(), 0);
        clone.note(BATCH_EVENTS);
        signal.note(144);
        assert_eq!(signal.committed(), BATCH_EVENTS + 144);
        assert_eq!(clone.committed(), BATCH_EVENTS + 144);
    }

    /// A cut that waits for a commit is released by the commit it waits for, and a commit
    /// that already happened releases it at once.
    #[tokio::test]
    async fn test_wait_for_commit_is_released_by_the_commit_it_waits_for() {
        let signal = CommitSignal::default();
        let waiting = {
            let signal = signal.clone();
            tokio::spawn(async move {
                wait_for_commit(&signal, BATCH_EVENTS).await;
            })
        };
        signal.note(BATCH_EVENTS);
        tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .unwrap()
            .unwrap();
        let ready = CommitSignal::default();
        ready.note(BATCH_EVENTS);
        tokio::time::timeout(Duration::from_secs(5), async {
            wait_for_commit(&ready, BATCH_EVENTS).await;
        })
        .await
        .unwrap();
    }

    /// An interrupted session commits whole batches only. A restart from scratch sends only the
    /// missing events, with no duplicate.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_an_interrupted_session_restarts_without_duplicates() {
        let (_da, mut a) = open();
        let (_db, mut b) = open();
        populate(&mut a, &[(1, 300), (2, 300)]);
        let total = ids(&a);

        // A cuts after 400 events, but the cut waits until B has committed its first
        // batch: closing the connection discards whatever B has not read yet, so a cut
        // counted in records alone can land before B reads 256 events under load, and
        // the batch then never commits (#188). B has decided one full batch of 256, and
        // the other 144 died with the session.
        let (first_a, first_b) = loopback(
            &mut a,
            &mut b,
            Params::default(),
            Params::default(),
            Cut::AfterRecordsWhenCommitted {
                records: 400,
                signal: CommitSignal::default(),
                committed: BATCH_EVENTS,
            },
        )
        .await
        .unwrap();
        assert!(first_a.is_err(), "the cut ends the dialer's session");
        assert!(first_b.is_err(), "the acceptor sees the connection close");
        assert_eq!(
            ids(&b).len(),
            BATCH_EVENTS,
            "only the whole batch was committed"
        );

        let (second_a, second_b) = loopback(
            &mut a,
            &mut b,
            Params::default(),
            Params::default(),
            Cut::Never,
        )
        .await
        .unwrap();
        let (second_a, second_b) = (second_a.unwrap(), second_b.unwrap());
        assert_eq!(ids(&a), total);
        assert_eq!(ids(&b), total, "the restart completes the store");
        assert_eq!(
            second_b.accepted,
            total.len() - BATCH_EVENTS,
            "only the missing events are sent"
        );
        assert_eq!(second_b.duplicates + second_a.duplicates, 0, "no duplicate");
        assert_eq!(second_a.accepted, 0);
    }

    /// #180: the data phase decides batches bounded by bytes as well as by count.
    /// A signed event's record is small: the body is committed by hash, not
    /// carried, and the largest record a valid fixture can build is about
    /// 10 KiB, far under the production byte bound -- so this test names a
    /// bound of two records through the test plumbing. Batches then fill
    /// after two events while the count bound (256) is nowhere near. A batch
    /// flushes when the record that would overflow it arrives, so the cut
    /// comes after five records: the fifth record's arrival flushes the
    /// second batch, and the cut, waiting until the receiver has committed
    /// four, leaves exactly those four in the receiving store -- two whole
    /// byte-bound batches, where a count-only receiver would have committed
    /// nothing at all (its one partial batch dies with the session, as in
    /// the interruption test above).
    #[tokio::test(flavor = "multi_thread")]
    async fn test_data_phase_batches_are_bounded_by_bytes_as_well_as_count() {
        let (_da, mut a) = open();
        let (_db, mut b) = open();
        for n in 1..=6u64 {
            fill(&mut a, &chain_of(n, 1, b"body"));
        }
        let record_len = chain_of(1, 1, b"body")[0].bytes.len();
        let (first_a, first_b) = loopback_gated_bounded(
            &mut a,
            &mut b,
            Params::default(),
            Params::default(),
            Cut::AfterRecordsWhenCommitted {
                records: 5,
                signal: CommitSignal::default(),
                committed: 4,
            },
            &testing::open_gate(),
            testing::key(0xd9),
            2 * record_len,
        )
        .await
        .unwrap();
        assert!(first_a.is_err(), "the cut ends the dialer's session");
        assert!(first_b.is_err(), "the acceptor sees the connection close");
        assert_eq!(
            ids(&b).len(),
            4,
            "two byte-bound batches of two events each were committed"
        );
    }

    /// #180: a single event larger than the byte bound is decided alone and
    /// does not error: with a bound smaller than one record, every event
    /// forms a solo batch, the session completes, and every event arrives.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_an_event_larger_than_the_byte_bound_is_decided_alone() {
        let (_da, mut a) = open();
        let (_db, mut b) = open();
        populate(&mut a, &[(1, 2), (2, 2)]);
        let total = ids(&a);
        let record_len = chain_of(1, 1, b"body")[0].bytes.len();
        let (out_a, out_b) = loopback_gated_bounded(
            &mut a,
            &mut b,
            Params::default(),
            Params::default(),
            Cut::Never,
            &testing::open_gate(),
            testing::key(0xd9),
            record_len - 1,
        )
        .await
        .unwrap();
        out_a.unwrap();
        let out_b = out_b.unwrap();
        assert_eq!(ids(&b), total, "every oversize event arrives");
        assert_eq!(out_b.accepted, total.len());
    }

    /// P3.1c: a peer that never clears `more` must not make the reader buffer without bound. This
    /// peer sends 4 MiB of flights with `more` set, then goes silent. Against a 256 KiB session
    /// budget, the session must end with [`BudgetExceeded`] before it buffers past the budget, and
    /// the store must be unchanged.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_an_unbounded_flight_ends_at_the_session_budget() {
        const PEER_BYTES: usize = 4 << 20;
        const BUDGET: usize = 256 << 10;
        let (_dir, mut store) = open();
        populate(&mut store, &[(1, 3)]);
        let before = ids(&store);
        let server = endpoint(vec![ALPN.to_vec()]).await.unwrap();
        let client = endpoint(vec![]).await.unwrap();
        let target = loopback_addr(&server);
        // One frame with `more` set and 1,000 unbounded skip entries: two bytes each on the wire.
        let mut frame = vec![1u8, 0xE8, 0x07];
        frame.extend(std::iter::repeat_n(0u8, 2000));
        let peer = tokio::spawn(async move {
            let incoming = server.accept().await.unwrap();
            let conn = incoming.await.unwrap();
            let (mut send, _recv) = conn.accept_bi().await.unwrap();
            let mut written = 0;
            while written < PEER_BYTES {
                let len = u32::try_from(frame.len()).unwrap().to_be_bytes();
                if send.write_all(&len).await.is_err() || send.write_all(&frame).await.is_err() {
                    return;
                }
                written += 4 + frame.len();
            }
            // Silent from here on. The connection stays open until the session gives up.
            tokio::time::sleep(Duration::from_secs(120)).await;
            drop(conn);
        });
        let conn = client.connect(target, ALPN).await.unwrap();
        let params = Params::default().with_session_budget(BUDGET).unwrap();
        let outcome = sync_over(
            &mut store,
            &conn,
            true,
            params,
            NOW,
            Duration::from_secs(5),
            &testing::open_gate(),
        )
        .await;
        conn.close(0u32.into(), b"done");
        peer.abort();
        client.close().await;
        let err = outcome.expect_err("an unbounded flight gave a session");
        let budget = err
            .downcast_ref::<BudgetExceeded>()
            .copied()
            .expect("the error is not a BudgetExceeded");
        assert_eq!(budget.budget, BUDGET);
        assert!(
            budget.read <= BUDGET,
            "the reader buffered {} bytes past its budget",
            budget.read
        );
        assert_eq!(
            ids(&store),
            before,
            "a refused session leaves the store unchanged"
        );
    }

    /// A charge that passes the budget is refused and charges nothing, so `read` stays at or under
    /// the budget. A charge that lands exactly on the budget is accepted.
    #[test]
    fn test_read_budget_refuses_the_charge_that_passes_the_budget() {
        let mut budget = ReadBudget {
            budget: 100,
            read: 0,
        };
        assert!(budget.charge(60).is_ok());
        assert!(
            budget.charge(40).is_ok(),
            "landing on the budget is allowed"
        );
        let err = budget.charge(1).unwrap_err();
        assert_eq!(
            err,
            BudgetExceeded {
                budget: 100,
                read: 100
            }
        );
        assert_eq!(budget.read, 100, "a refused charge is not counted");
    }

    /// A peer that opens a connection and then says nothing holds a range session forever, unless
    /// the session has a deadline of its own. The session must end at that deadline.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_a_silent_peer_ends_the_session_at_its_deadline() {
        let (_dir, mut store) = open();
        let server = endpoint(vec![ALPN.to_vec()]).await.unwrap();
        let client = endpoint(vec![]).await.unwrap();
        let target = loopback_addr(&server);
        // The peer accepts, then holds the connection open and never reads or writes.
        let silent = tokio::spawn(async move {
            let incoming = server.accept().await.unwrap();
            let conn = incoming.await.unwrap();
            tokio::time::sleep(Duration::from_secs(120)).await;
            drop(conn);
        });
        let conn = client.connect(target, ALPN).await.unwrap();
        let gate = testing::open_gate();
        let session = sync_over(
            &mut store,
            &conn,
            true,
            Params::default(),
            NOW,
            Duration::from_secs(2),
            &gate,
        );
        let outcome = tokio::time::timeout(Duration::from_secs(10), session)
            .await
            .expect("the session had no deadline of its own");
        assert!(
            outcome.is_err(),
            "a silent peer gave a session: {outcome:?}"
        );
        conn.close(0u32.into(), b"done");
        silent.abort();
        client.close().await;
    }

    /// P3.1c on a real store: a store above the 524,288-event limit bootstraps into an empty one
    /// in both directions. The ID reply alone is 17.6 MB, over the 16 MiB frame cap, so it has to
    /// be chunked. Signing 550,000 events is slow in debug builds, so run it on demand:
    /// `cargo test --release -- --ignored test_bootstrap_above_the_frame_limit`.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "signs 550,000 events; run with --release --ignored"]
    async fn test_bootstrap_above_the_frame_limit_on_a_real_store() {
        let chains: Vec<(u64, u32)> = (1..=1100).map(|c| (c, 500)).collect();
        for full_dials in [true, false] {
            let (_df, mut full) = open();
            let (_de, mut empty) = open();
            populate(&mut full, &chains);
            let total = ids(&full);
            assert_eq!(total.len(), 550_000);
            let params = Params::default();
            let (dialer, acceptor) = if full_dials {
                loopback(&mut full, &mut empty, params, params, Cut::Never)
                    .await
                    .unwrap()
            } else {
                loopback(&mut empty, &mut full, params, params, Cut::Never)
                    .await
                    .unwrap()
            };
            let (dialer, acceptor) = (dialer.unwrap(), acceptor.unwrap());
            let label = if full_dials {
                "full dials empty"
            } else {
                "empty dials full"
            };
            assert_eq!(ids(&empty), total, "{label}");
            assert_eq!(ids(&full), total, "{label}");
            assert!(
                dialer.control_bytes + acceptor.control_bytes > range::FRAME_CAP,
                "{label}: the reply is above one frame"
            );
            assert_eq!(dialer.duplicates + acceptor.duplicates, 0, "{label}");
        }
    }

    /// Bootstraps an empty store from a full one, and the other way round, under `budget`. Each
    /// direction must converge, and the ID replies must be split into more than `frames` bytes of
    /// frames, so the chunking is exercised.
    async fn bootstrap_both_ways(budget: Params, chains: &[(u64, u32)], split_above: usize) {
        for full_dials in [true, false] {
            let (_df, mut full) = open();
            let (_de, mut empty) = open();
            populate(&mut full, chains);
            let total = ids(&full);
            let (dialer, acceptor) = if full_dials {
                loopback(&mut full, &mut empty, budget, budget, Cut::Never)
                    .await
                    .unwrap()
            } else {
                loopback(&mut empty, &mut full, budget, budget, Cut::Never)
                    .await
                    .unwrap()
            };
            let (dialer, acceptor) = (dialer.unwrap(), acceptor.unwrap());
            let label = if full_dials {
                "full dials empty"
            } else {
                "empty dials full"
            };
            assert_eq!(ids(&empty), total, "{label}");
            assert_eq!(ids(&full), total, "{label}");
            // Chunking adds frames, not messages, so the control bytes show the split.
            assert!(
                dialer.control_bytes + acceptor.control_bytes > split_above,
                "{label}: the ID replies were split into frames"
            );
            assert_eq!(dialer.duplicates + acceptor.duplicates, 0, "{label}");
        }
    }

    /// Bootstrap of a store whose sync set needs many frames: 1,200 events under a 4 KiB budget. The
    /// ID replies are 38 KB, which is many 4 KiB frames. This takes about 100 s in a debug build,
    /// so it runs on demand. `test_bootstrap_over_many_frames_on_small_stores` is the always-on
    /// version of the same check.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "1,200 events take about 100 s in debug; run with: cargo test --release --lib -- --ignored test_bootstrap_over_many_frames_in_both_directions_on_real_stores"]
    async fn test_bootstrap_over_many_frames_in_both_directions_on_real_stores() {
        let budget = Params::default().with_frame_budget(4096).unwrap();
        let chains: Vec<(u64, u32)> = (1..=30).map(|c| (c, 40)).collect();
        bootstrap_both_ways(budget, &chains, 4 * 4096).await;
    }

    /// The always-on version of the test above: 60 events under a 1 KiB budget. Each ID reply is
    /// still several frames, and the run takes a few seconds in a debug build.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_bootstrap_over_many_frames_on_small_stores() {
        let budget = Params::default().with_frame_budget(1024).unwrap();
        let chains: Vec<(u64, u32)> = (1..=2).map(|c| (c, 30)).collect();
        bootstrap_both_ways(budget, &chains, 2 * 1024).await;
    }

    /// P6.5: open pull is today's behaviour. A peer outside the world still pulls every event.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_pull_policy_open_range_session_serves_a_non_member() {
        let (_da, mut a) = open();
        let (_db, mut b) = open();
        populate(&mut a, &[(1, 4), (2, 3)]);
        let (dialer, acceptor) = loopback_gated(
            &mut a,
            &mut b,
            Params::default(),
            Params::default(),
            Cut::Never,
            &testing::open_gate(),
            testing::key(0xd9),
        )
        .await
        .unwrap();
        dialer.unwrap();
        acceptor.unwrap();
        assert_eq!(ids(&b), ids(&a), "a stranger pulled every event");
    }

    /// P6.5: the members option refuses a stranger before any event is sent. Both sides end with
    /// an error, and neither store changes.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_pull_policy_members_range_refuses_a_non_member_and_sends_no_event() {
        let (_da, mut a) = open();
        let (_db, mut b) = open();
        populate(&mut a, &[(1, 4)]);
        let (before_a, before_b) = (ids(&a), ids(&b));
        // The founder is the only member. The dialer is a stranger.
        let world = testing::docs_world(
            testing::key(0xd0).public(),
            testing::key(0xd2).public(),
            false,
        );
        let gate = PullGate::new(PullPolicy::Members, testing::WORLD, testing::live(world));
        let (dialer, acceptor) = loopback_gated(
            &mut a,
            &mut b,
            Params::default(),
            Params::default(),
            Cut::Never,
            &gate,
            testing::key(0xd1),
        )
        .await
        .unwrap();
        let refused = acceptor.expect_err("a stranger was served events");
        assert!(
            refused.downcast_ref::<PullRefused>().is_some(),
            "{refused:#}"
        );
        assert!(
            dialer.is_err(),
            "the dialer's session ends with the refusal"
        );
        assert_eq!(ids(&b), before_b, "no event crossed to the stranger");
        assert_eq!(ids(&a), before_a, "the server's store is unchanged");
    }

    /// P6.5: a member gets every event under the members option.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_pull_policy_members_range_serves_a_member_everything() {
        let (_da, mut a) = open();
        let (_db, mut b) = open();
        populate(&mut a, &[(1, 4), (2, 3)]);
        let world = testing::docs_world(
            testing::key(0xd0).public(),
            testing::key(0xd1).public(),
            false,
        );
        let gate = PullGate::new(PullPolicy::Members, testing::WORLD, testing::live(world));
        let (dialer, acceptor) = loopback_gated(
            &mut a,
            &mut b,
            Params::default(),
            Params::default(),
            Cut::Never,
            &gate,
            testing::key(0xd1),
        )
        .await
        .unwrap();
        dialer.unwrap();
        acceptor.unwrap();
        assert_eq!(ids(&b), ids(&a), "the member pulled every event");
    }

    /// P6.5: membership is read when each session starts. A member removed from the fold is refused
    /// on the next pull, and the events it pulled before stay.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_pull_policy_members_range_refuses_a_removed_member_on_the_next_pull() {
        let (founder, member) = (testing::key(0xd0).public(), testing::key(0xd1).public());
        let access = testing::live(testing::docs_world(founder, member, false));
        let gate = PullGate::new(PullPolicy::Members, testing::WORLD, Arc::clone(&access));
        let (_da, mut a) = open();
        let (_db, mut b) = open();
        populate(&mut a, &[(1, 3)]);
        let (dialer, acceptor) = loopback_gated(
            &mut a,
            &mut b,
            Params::default(),
            Params::default(),
            Cut::Never,
            &gate,
            testing::key(0xd1),
        )
        .await
        .unwrap();
        dialer.unwrap();
        acceptor.unwrap();
        assert_eq!(ids(&b), ids(&a), "the member pulled while it was a member");

        // The removal lands, and the server has new events for the member to pull.
        access.set(testing::docs_world(founder, member, true));
        populate(&mut a, &[(2, 3)]);
        let before = ids(&b);
        let (_dialer, acceptor) = loopback_gated(
            &mut a,
            &mut b,
            Params::default(),
            Params::default(),
            Cut::Never,
            &gate,
            testing::key(0xd1),
        )
        .await
        .unwrap();
        let refused = acceptor.expect_err("the removed member was served again");
        assert!(
            refused.downcast_ref::<PullRefused>().is_some(),
            "{refused:#}"
        );
        assert_eq!(ids(&b), before, "the refused pull sent nothing");
    }
}
