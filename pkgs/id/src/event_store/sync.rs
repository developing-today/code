//! Range sync over the event store: the method that `/id-sync/1` runs after the handshake.
//!
//! The handshake is [`crate::log_sync`]'s: the initiator sends a `Request` with the topic and the
//! method it chose, and the responder answers. When both agree on range, [`run_range`] runs on the
//! stream they share. The sync set is the topic's authors' events, the same authors LogSync
//! resolves (see [`SharedEventStore::range_entries`]): each stored event and each fork proof header,
//! one key each (see [`super::range`]). A session has three phases:
//!
//! 1. **Reconciliation.** Messages alternate as flights of frames until one side answers with only
//!    skips. The initiator sends the full range's fingerprint first.
//! 2. **Data.** Each side knows the symmetric difference, so it knows what the peer lacks. Each
//!    side sends the events it has that the peer lacks, in key order: a count frame, then one
//!    frame per event. Both sides send at once.
//! 3. **Apply.** The receiver verifies each event, checks that it was expected, and decides the
//!    events in batches. A batch is one write transaction, and a fork in it is an outcome like any
//!    other, so the events after it are decided too (P3.1a).
//!
//! K, T, and the frame budget are local. The peer never sees them.

use std::collections::HashSet;

use anyhow::{Context, Result, anyhow, bail, ensure};
use iroh::endpoint::SendStream;
use p2panda_core::Hash;
use redb::{ReadableDatabase, ReadableTable};
use tokio::sync::mpsc;

use super::range::{
    self, Entry, Id, Key, Params, Party, all_skip, check_bounds, decode_frame, frames,
};
use super::{
    EVENTS, EventStore, FORKS, HASHES, Ingest, PROOF_IDS, StoreError, corrupt, decode_proof,
    position_key,
};
use crate::chain::{self, Event};
use crate::store_traits::SharedEventStore;
use crate::waiting::AuthorKey;

/// Bytes in a position key: the author, then the sequence number.
const POSITION_LEN: usize = 40;

/// The receiving half of a sync stream, as the handshake's reader task delivers it: whole frames,
/// or the error that ended the stream.
pub(crate) type Inbound = mpsc::Receiver<Result<Vec<u8>, String>>;

/// The most messages one side writes in reconciliation before it gives up.
const MAX_FLIGHTS: u32 = 10_000;

/// Events the receiver decides in one write transaction.
const BATCH_EVENTS: usize = 256;

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
    /// Received events that were stored: applied, or voided.
    pub accepted: usize,
    /// Received events the store already held.
    pub duplicates: usize,
    /// Received events held for a dependency.
    pub waiting: usize,
    /// Received events refused.
    pub refused: usize,
    /// Forks that received events revealed.
    pub forks: usize,
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
}

impl Tally {
    const fn add(&mut self, outcome: &Ingest) {
        match outcome {
            Ingest::Accepted { .. } => self.accepted += 1,
            Ingest::Duplicate { .. } => self.duplicates += 1,
            Ingest::Fork { .. } => self.forks += 1,
            Ingest::Waiting { .. } => self.waiting += 1,
            Ingest::Refused(_) => self.refused += 1,
        }
    }
}

/// Where to cut a session, for the interruption test. A cut finishes this side's send half early,
/// so the peer reads what was sent and then sees the stream end, and this session returns an error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cut {
    /// No cut.
    Never,
    /// Cut before this side writes message `n + 1`.
    AfterFlights(u32),
    /// Cut before this side writes event `n + 1`.
    AfterRecords(usize),
}

/// Runs one range session on the stream a `Request` agreed on. `initiator` sends the first
/// message. `now_ms` is the clock the store's waiting rules use. The session syncs the events of
/// `topic`'s authors on this side, as LogSync does.
///
/// # Errors
///
/// Returns an error if the peer breaks the protocol, a frame is malformed or too large, an event
/// fails verification, the session does not end within the flight limit, or the stream fails.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_range(
    store: &SharedEventStore,
    topic: Hash,
    params: Params,
    now_ms: u64,
    initiator: bool,
    send: &mut SendStream,
    inbound: &mut Inbound,
    cut: Cut,
) -> Result<Outcome> {
    let party = Party::new(store.range_entries(&topic)?)?;
    let mut out = Outcome::default();

    // 1. Reconciliation, the same loop as S7's session, with each message as frames.
    let mut learned: Vec<Id> = Vec::new();
    if initiator {
        write_flight(send, &mut out, &[party.full_range()], params, cut).await?;
    }
    loop {
        let incoming = read_flight(inbound).await?;
        if all_skip(&incoming) {
            break;
        }
        let reply = party.respond(&incoming, params, &mut learned);
        let last = all_skip(&reply);
        write_flight(send, &mut out, &reply, params, cut).await?;
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

    let sending = async {
        let count = u32::try_from(offers.len())?;
        let mut bytes = send_frame(send, &count.to_be_bytes()).await?;
        for (sent, id) in offers.iter().enumerate() {
            if cut == Cut::AfterRecords(sent) {
                end_early(send);
                bail!("cut before event {}", sent + 1);
            }
            let record = store
                .sync_bytes(id)?
                .context("an offered event is no longer stored")?;
            bytes += send_frame(send, &record).await?;
        }
        anyhow::Ok(bytes)
    };

    let receiving = async {
        let count_frame = next_frame(inbound).await?;
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
        let mut tally = Tally::default();
        for _ in 0..count {
            let record = next_frame(inbound).await?;
            let event = chain::decode(&record)?;
            ensure!(
                need.remove(event.id.as_bytes()),
                "an event arrived that was not expected, or arrived twice"
            );
            pending.push(event);
            if pending.len() == BATCH_EVENTS {
                apply_batch(store, std::mem::take(&mut pending), now_ms, &mut tally)?;
            }
        }
        apply_batch(store, pending, now_ms, &mut tally)?;
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
    Ok(out)
}

/// Decides one batch in one write transaction, and tallies each outcome. A fork is an outcome
/// like any other: it is recorded, and the events after it are still decided. Only an error from
/// the store ends the batch, and then nothing in it commits.
fn apply_batch(
    store: &SharedEventStore,
    events: Vec<Event>,
    now_ms: u64,
    tally: &mut Tally,
) -> Result<()> {
    for outcome in store.ingest_events(events, now_ms)? {
        tally.add(&outcome);
    }
    Ok(())
}

/// Writes one message as frames, or finishes the stream early if the cut is due.
async fn write_flight(
    send: &mut SendStream,
    out: &mut Outcome,
    msg: &[Entry],
    params: Params,
    cut: Cut,
) -> Result<()> {
    if cut == Cut::AfterFlights(out.flights) {
        end_early(send);
        bail!("cut before message {}", out.flights + 1);
    }
    for frame in frames(msg, params.frame_budget())? {
        out.control_bytes += send_frame(send, &frame).await?;
    }
    out.flights += 1;
    Ok(())
}

/// Reads frames until one without the `more` flag, and returns the message's entries.
async fn read_flight(inbound: &mut Inbound) -> Result<Vec<Entry>> {
    let mut entries = Vec::new();
    loop {
        let (more, mut part) = decode_frame(&next_frame(inbound).await?)?;
        entries.append(&mut part);
        if !more {
            break;
        }
    }
    check_bounds(&entries)?;
    Ok(entries)
}

/// Writes one frame, and returns its size on the wire, with the length prefix. The frame is one
/// write, so a cancelled caller never leaves half of it on the stream.
async fn send_frame(send: &mut SendStream, payload: &[u8]) -> Result<usize> {
    ensure!(
        payload.len() <= range::FRAME_CAP,
        "frame of {} bytes is over the cap",
        payload.len()
    );
    let len = u32::try_from(payload.len())?;
    let mut framed = Vec::with_capacity(4 + payload.len());
    framed.extend_from_slice(&len.to_be_bytes());
    framed.extend_from_slice(payload);
    send.write_all(&framed).await?;
    Ok(framed.len())
}

/// The next whole frame from the peer.
async fn next_frame(inbound: &mut Inbound) -> Result<Vec<u8>> {
    match inbound.recv().await {
        Some(Ok(frame)) => Ok(frame),
        Some(Err(error)) => Err(anyhow!(error)),
        None => Err(anyhow!("the sync stream ended")),
    }
}

/// Finishes this side's send half early. The peer reads what was sent, then the stream ends, as
/// it would after an interruption. A reset would be wrong here: QUIC discards the data a reader
/// has not yet taken when a stream is reset.
fn end_early(send: &mut SendStream) {
    let _ = send.finish();
}

impl SharedEventStore {
    /// The bytes of the event with `id`, if the store holds it as an event or a fork proof header.
    pub(crate) fn sync_bytes(&self, id: &Id) -> Result<Option<Vec<u8>>> {
        Ok(self.read(|store| Ok(store.sync_bytes(id)?))?)
    }
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
        let forks = txn.open_table(FORKS)?;
        let mut entries: Vec<(Key, Id)> = Vec::new();
        for row in hashes.iter()? {
            let (id, position) = row?;
            let id = id_of(id.value())?;
            entries.push((sync_key(position.value(), &id)?, id));
        }
        for row in forks.iter()? {
            let (_author, proof) = row?;
            let proof = decode_proof(proof.value())?;
            for header in [&proof.first, &proof.second] {
                let author = *header.header.verifying_key.as_bytes();
                let position = position_key(&author, header.header.seq_num);
                let id = *header.id.as_bytes();
                entries.push((sync_key(&position, &id)?, id));
            }
        }
        entries.sort_unstable_by_key(|e| e.0);
        entries.dedup_by(|a, b| a.0 == b.0);
        Ok(entries)
    }

    /// The encoded bytes of the event with this ID, if the store holds it as a stored event or as
    /// a current fork proof header.
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
        let proof_ids = txn.open_table(PROOF_IDS)?;
        let Some(author) = proof_ids.get(id.as_slice())? else {
            return Ok(None);
        };
        let author: AuthorKey = author
            .value()
            .try_into()
            .map_err(|_err| corrupt("a proof ID names no author"))?;
        let forks = txn.open_table(FORKS)?;
        let Some(row) = forks.get(author.as_slice())? else {
            return Ok(None);
        };
        let proof = decode_proof(row.value())?;
        Ok([proof.first, proof.second]
            .into_iter()
            .find(|header| header.id.as_bytes() == id)
            .map(|header| header.bytes))
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::net::Ipv4Addr;
    use std::time::Duration;

    use iroh::endpoint::{Endpoint, presets};
    use iroh::{EndpointAddr, RelayMode, TransportAddr};
    use p2panda_core::SigningKey;

    use super::*;
    use crate::chain::ChainExt;
    use crate::event_store::EventStore;
    use crate::log_sync::Frames;
    use crate::waiting::WaitingLimits;

    const NOW: u64 = 1_000_000_000;
    /// The topic every test event belongs to, so each one is in the sync set.
    const TOPIC_BYTES: [u8; 1] = [1];

    fn topic() -> Hash {
        Hash::digest(TOPIC_BYTES)
    }

    fn open() -> (tempfile::TempDir, SharedEventStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = EventStore::open(dir.path(), WaitingLimits::default(), NOW).unwrap();
        (dir, SharedEventStore::new(store))
    }

    fn signer(n: u64) -> SigningKey {
        SigningKey::from_bytes(Hash::digest(n.to_be_bytes()).as_bytes())
    }

    /// The first `len` events of chain `n`, in the topic. Each names the one before, and carries
    /// `body`.
    fn chain_of(n: u64, len: u32, body: &[u8]) -> Vec<Event> {
        let key = signer(n);
        let mut events: Vec<Event> = Vec::new();
        for seq in 0..len {
            let backlink = events.last().map(|e| e.id);
            let ext = ChainExt {
                kind: "message".to_owned(),
                group: Some(topic()),
                after: Vec::new(),
            };
            events.push(chain::sign(&key, seq, backlink, Some(body), ext).unwrap());
        }
        events
    }

    /// Stores `events` in order, and checks each one is applied.
    fn fill(store: &SharedEventStore, events: &[Event]) {
        let outcomes = store.ingest_events(events.to_vec(), NOW).unwrap();
        assert!(
            outcomes
                .iter()
                .all(|o| matches!(o, Ingest::Accepted { .. })),
            "{outcomes:?}"
        );
    }

    /// The chains of `lengths` (chain number, length), each with a prefix stored.
    fn populate(store: &SharedEventStore, lengths: &[(u64, u32)]) {
        for &(n, len) in lengths {
            fill(store, &chain_of(n, len, b"body"));
        }
    }

    /// Every sync key and ID the store holds, whatever the topic.
    fn ids(store: &SharedEventStore) -> HashSet<Id> {
        store
            .read(|s| Ok(s.sync_entries()?))
            .unwrap()
            .into_iter()
            .map(|entry| entry.1)
            .collect()
    }

    /// The symmetric difference of two stores' IDs.
    fn difference(a: &SharedEventStore, b: &SharedEventStore) -> HashSet<Id> {
        let (a, b) = (ids(a), ids(b));
        a.symmetric_difference(&b).copied().collect()
    }

    async fn endpoint(alpns: Vec<Vec<u8>>) -> Result<Endpoint> {
        Ok(Endpoint::builder(presets::Minimal)
            .alpns(alpns)
            .relay_mode(RelayMode::Disabled)
            .bind()
            .await?)
    }

    /// The address of `ep` on localhost, so a peer dials it without a relay or discovery.
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

    /// Runs a range session on each side of a loopback stream. `a` dials and `b` accepts. Both run
    /// in this task, each store borrowed only by its own session, and the whole exchange has a
    /// deadline, so it cannot hang.
    async fn loopback(
        a: &SharedEventStore,
        b: &SharedEventStore,
        pa: Params,
        pb: Params,
        cut: Cut,
    ) -> Result<(Result<Outcome>, Result<Outcome>)> {
        let server = endpoint(vec![crate::log_sync::SYNC_ALPN.to_vec()]).await?;
        let client = endpoint(vec![]).await?;
        let target = loopback_addr(&server);
        let accepting = async {
            let incoming = server
                .accept()
                .await
                .context("endpoint closed before a connection")?;
            let conn = incoming.await?;
            let (send, recv) = conn.accept_bi().await?;
            let mut frames = Frames::new(send, recv);
            let out = run_range(
                b,
                topic(),
                pb,
                NOW,
                false,
                &mut frames.send,
                &mut frames.inbound,
                Cut::Never,
            )
            .await;
            frames.finish().await;
            anyhow::Ok((out, conn))
        };
        let dialing = async {
            let conn = client.connect(target, crate::log_sync::SYNC_ALPN).await?;
            let (send, recv) = conn.open_bi().await?;
            let mut frames = Frames::new(send, recv);
            let out = run_range(
                a,
                topic(),
                pa,
                NOW,
                true,
                &mut frames.send,
                &mut frames.inbound,
                cut,
            )
            .await;
            frames.finish().await;
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

    #[tokio::test(flavor = "multi_thread")]
    async fn test_empty_store_is_readable_and_two_empty_stores_agree() {
        let (_da, a) = open();
        let (_db, b) = open();
        assert!(ids(&a).is_empty(), "a fresh store reads as empty");
        let (da, db) = loopback(&a, &b, Params::default(), Params::default(), Cut::Never)
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
        let (_d, store) = open();
        let events = chain_of(1, 3, b"x");
        fill(&store, &events);
        // A rival at sequence 1: a different body, so a different ID, at a stored position.
        let rival = chain::sign(
            &signer(1),
            1,
            Some(events[0].id),
            Some(b"other"),
            ChainExt {
                kind: "message".to_owned(),
                group: Some(topic()),
                after: Vec::new(),
            },
        )
        .unwrap();
        let outcome = store.ingest_events(vec![rival.clone()], NOW).unwrap();
        assert!(matches!(outcome[0], Ingest::Fork { .. }), "{outcome:?}");
        // The stored events (0, 1, and 2 voided above the fork point), and the rival, which is
        // now a proof header. The stored event at sequence 1 is also a proof header, so it is
        // listed once.
        assert_eq!(ids(&store).len(), 4);
        assert!(ids(&store).contains(rival.id.as_bytes()));
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
        let (_d, store) = open();
        fill(&store, &chain_of(1, 1, b"x"));
        let rival = chain::sign(
            &signer(1),
            0,
            None,
            Some(b"rival"),
            ChainExt {
                kind: "message".to_owned(),
                group: Some(topic()),
                after: Vec::new(),
            },
        )
        .unwrap();
        let mut batch = vec![rival];
        for n in 2..22 {
            batch.extend(chain_of(n, 1, b"unrelated"));
        }
        let mut tally = Tally::default();
        apply_batch(&store, batch, NOW, &mut tally).unwrap();
        assert_eq!(tally.forks, 1);
        assert_eq!(
            tally.accepted, 20,
            "the 20 unrelated events survive the fork"
        );
        // The original, the rival that proves the fork, and the 20 unrelated events.
        assert_eq!(ids(&store).len(), 22);
    }

    /// Only the topic's authors are offered: an author with no event in the topic stays home.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_range_offers_only_the_topics_authors() {
        let (_da, a) = open();
        let (_db, b) = open();
        fill(&a, &chain_of(1, 3, b"in"));
        // Chain 2 is in a different group, so it is not in the topic.
        let other = Hash::digest([2_u8]);
        let key = signer(2);
        let mut outside: Vec<Event> = Vec::new();
        for seq in 0..3 {
            let backlink = outside.last().map(|e| e.id);
            let ext = ChainExt {
                kind: "message".to_owned(),
                group: Some(other),
                after: Vec::new(),
            };
            outside.push(chain::sign(&key, seq, backlink, None, ext).unwrap());
        }
        fill(&a, &outside);
        let (da, _db) = loopback(&a, &b, Params::default(), Params::default(), Cut::Never)
            .await
            .unwrap();
        let da = da.unwrap();
        assert_eq!(da.sent, 3, "only the topic's chain is offered");
        assert_eq!(ids(&b).len(), 3);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_loopback_converges_for_mixed_k_and_t() {
        let cases = [
            (16usize, 16usize, 16usize, 16usize),
            (2, 1, 32, 4),
            (4, 8, 16, 16),
            (32, 1, 2, 4),
        ];
        for (seed, (ka, ta, kb, tb)) in cases.into_iter().enumerate() {
            // Prefix lengths of each side's chains, so each side holds events the other lacks.
            let seed = seed as u32;
            let la: Vec<(u64, u32)> = (1..=24u64)
                .map(|c| (c, (c as u32 * (7 + seed)) % 19))
                .collect();
            let lb: Vec<(u64, u32)> = (1..=24u64)
                .map(|c| (c, (c as u32 * (5 + seed) + 3) % 23))
                .collect();
            let (a, b) = (open().1, open().1);
            populate(&a, &la);
            populate(&b, &lb);
            let truth = difference(&a, &b);
            let union: HashSet<Id> = ids(&a).union(&ids(&b)).copied().collect();
            let label = format!("A K={ka} T={ta}, B K={kb} T={tb}");

            let (da, db) = loopback(
                &a,
                &b,
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

    /// An interrupted session commits whole batches only. A restart from scratch sends only the
    /// missing events, with no duplicate.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_an_interrupted_session_restarts_without_duplicates() {
        let (_da, a) = open();
        let (_db, b) = open();
        populate(&a, &[(1, 300), (2, 300)]);
        let total = ids(&a);

        // A cuts after 400 events. B has decided one full batch of 256, and the other 144 died
        // with the session.
        let (first_a, first_b) = loopback(
            &a,
            &b,
            Params::default(),
            Params::default(),
            Cut::AfterRecords(400),
        )
        .await
        .unwrap();
        assert!(first_a.is_err(), "the cut ends the dialer's session");
        assert!(first_b.is_err(), "the acceptor sees the stream end");
        assert_eq!(
            ids(&b).len(),
            BATCH_EVENTS,
            "only the whole batch was committed"
        );

        let (second_a, second_b) =
            loopback(&a, &b, Params::default(), Params::default(), Cut::Never)
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

    /// Bootstrap over many frames, in both directions: a 4 KiB budget splits every ID reply.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_bootstrap_over_many_frames_in_both_directions_on_real_stores() {
        let budget = Params::default().with_frame_budget(4096).unwrap();
        let chains: Vec<(u64, u32)> = (1..=30).map(|c| (c, 40)).collect();
        for full_dials in [true, false] {
            let (_df, full) = open();
            let (_de, empty) = open();
            populate(&full, &chains);
            let total = ids(&full);
            let (dialer, acceptor) = if full_dials {
                loopback(&full, &empty, budget, budget, Cut::Never)
                    .await
                    .unwrap()
            } else {
                loopback(&empty, &full, budget, budget, Cut::Never)
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
            // Chunking adds frames, not messages, so the control bytes show the split: 1,200 IDs
            // are 38 KB, which is many 4 KiB frames.
            assert!(
                dialer.control_bytes + acceptor.control_bytes > 4 * 4096,
                "{label}: the ID replies were split into frames"
            );
            assert_eq!(dialer.duplicates + acceptor.duplicates, 0, "{label}");
        }
    }

    /// P3.1c on a real store: a store above the 524,288-event limit bootstraps into an empty one
    /// in both directions. The ID reply alone is 17.6 MB, far over one frame, so it has to be
    /// chunked. Signing 550,000 events is slow in debug builds, so run it on demand:
    /// `cargo test --release -- --ignored test_bootstrap_above_the_frame_limit`.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "signs 550,000 events; run with --release --ignored"]
    async fn test_bootstrap_above_the_frame_limit_on_a_real_store() {
        let chains: Vec<(u64, u32)> = (1..=1100).map(|c| (c, 500)).collect();
        for full_dials in [true, false] {
            let (_df, full) = open();
            let (_de, empty) = open();
            populate(&full, &chains);
            let total = ids(&full);
            assert_eq!(total.len(), 550_000);
            let params = Params::default();
            let (dialer, acceptor) = if full_dials {
                loopback(&full, &empty, params, params, Cut::Never)
                    .await
                    .unwrap()
            } else {
                loopback(&empty, &full, params, params, Cut::Never)
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
}
