//! Range sync over the event store, on `/id-sync/1`.
//!
//! The sync set is every stored event and every fork proof header, one key each: the author,
//! the sequence number, and the ID (see [`super::range`]). A session has three phases:
//!
//! 1. **Reconciliation.** Messages alternate as flights of frames until one side answers with
//!    only skips. The dialer opens the stream and sends the full range's fingerprint.
//! 2. **Data.** Each side knows the symmetric difference, so it knows what the peer lacks and
//!    what it holds that the peer lacks. Each side sends its events in key order: a count
//!    frame, then one frame per event. Both sides send at once.
//! 3. **Apply.** The receiver verifies each event, checks it was expected, and decides the
//!    events in batches, in one write transaction per batch, as the store's ingest does.
//!
//! K and T are local. The frame budget is local too: it only has to stay under the frame cap.

use std::collections::HashSet;
use std::sync::{Mutex, MutexGuard};

use anyhow::{Context, Result, anyhow, bail, ensure};
use iroh::endpoint::{Connection, RecvStream, SendStream};
use redb::{ReadableDatabase, ReadableTable};

use super::range::{
    self, Entry, Id, Key, Params, Party, all_skip, check_bounds, decode_frame, frames,
};
use super::{
    EVENTS, EventStore, FORKS, HASHES, Ingest, PROOF_IDS, StoreError, corrupt, decode_proof,
    position_key,
};
use crate::chain::{self, Event};
use crate::waiting::AuthorKey;

/// The protocol ID. The wire version only; K and T are not in it.
pub const ALPN: &[u8] = b"/id-sync/1";

/// The most messages one side writes in reconciliation before it gives up.
const MAX_FLIGHTS: u32 = 10_000;

/// Events the receiver decides in one write transaction.
const BATCH_EVENTS: usize = 256;

/// Bytes in a position key: the author, then the sequence number.
const POSITION_LEN: usize = 40;

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

/// Where to cut the connection, for the interruption test. The cut closes the connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cut {
    /// No cut.
    Never,
    /// Close before this side writes message `n + 1`.
    AfterFlights(u32),
    /// Close before this side writes event `n + 1`.
    AfterRecords(usize),
}

/// Runs one range session over an open connection. The dialer opens the stream and passes
/// `initiator = true`. `now_ms` is the clock the store's waiting rules use.
///
/// # Errors
///
/// Returns an error if the peer breaks the protocol, a frame is malformed or too large, an
/// event fails verification, the session does not end within the flight limit, or the
/// connection fails.
pub async fn sync_over(
    store: &mut EventStore,
    conn: &Connection,
    initiator: bool,
    params: Params,
    now_ms: u64,
) -> Result<Outcome> {
    run(store, conn, initiator, params, now_ms, Cut::Never).await
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

/// Writes one message as frames, or closes the connection if the cut is due.
async fn write_flight(
    send: &mut SendStream,
    conn: &Connection,
    out: &mut Outcome,
    msg: &[Entry],
    params: Params,
    cut: Cut,
) -> Result<()> {
    if cut == Cut::AfterFlights(out.flights) {
        conn.close(1u32.into(), b"cut");
        bail!("cut before flight {}", out.flights + 1);
    }
    for frame in frames(msg, params.frame_budget())? {
        out.control_bytes += write_frame(send, &frame).await?;
    }
    out.flights += 1;
    Ok(())
}

/// Reads frames until one without the `more` flag, and returns the message's entries.
async fn read_flight(recv: &mut RecvStream) -> Result<Vec<Entry>> {
    let mut entries = Vec::new();
    loop {
        let (more, mut part) = decode_frame(&read_frame(recv).await?)?;
        entries.append(&mut part);
        if !more {
            break;
        }
    }
    check_bounds(&entries)?;
    Ok(entries)
}

/// Locks the store for one step. The lock is never held across an await.
fn lock<'a, 'b>(
    store: &'a Mutex<&'b mut EventStore>,
) -> Result<MutexGuard<'a, &'b mut EventStore>> {
    store
        .lock()
        .map_err(|_err| anyhow!("the event store lock is poisoned"))
}

/// Decides one batch in one write transaction, and tallies each outcome. A fork is an outcome
/// like any other: it is recorded, and the events after it in the batch are still decided. Only
/// an error from the store ends the batch, and then nothing in it commits.
fn apply_batch(
    store: &mut EventStore,
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
) -> Result<Outcome> {
    let (mut send, mut recv) = if initiator {
        conn.open_bi().await?
    } else {
        conn.accept_bi().await?
    };
    let mut out = Outcome::default();
    let party = Party::new(store.sync_entries()?)?;

    // 1. Reconciliation. The same loop as S7's session, with each message as frames.
    let mut learned: Vec<Id> = Vec::new();
    if initiator {
        write_flight(
            &mut send,
            conn,
            &mut out,
            &[party.full_range()],
            params,
            cut,
        )
        .await?;
    }
    loop {
        let incoming = read_flight(&mut recv).await?;
        if all_skip(&incoming) {
            break;
        }
        let reply = party.respond(&incoming, params, &mut learned);
        let last = all_skip(&reply);
        write_flight(&mut send, conn, &mut out, &reply, params, cut).await?;
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

    let sending = async {
        let count = u32::try_from(offers.len())?;
        let mut bytes = write_frame(&mut send, &count.to_be_bytes()).await?;
        for (sent, id) in offers.iter().enumerate() {
            if cut == Cut::AfterRecords(sent) {
                conn.close(1u32.into(), b"cut");
                bail!("cut before event {}", sent + 1);
            }
            let record = lock(&store)?
                .sync_bytes(id)?
                .context("an offered event is no longer stored")?;
            bytes += write_frame(&mut send, &record).await?;
        }
        send.finish()?;
        anyhow::Ok(bytes)
    };

    let receiving = async {
        let count_frame = read_frame(&mut recv).await?;
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
            let record = read_frame(&mut recv).await?;
            let event = chain::decode(&record)?;
            ensure!(
                need.remove(event.id.as_bytes()),
                "an event arrived that was not expected, or arrived twice"
            );
            pending.push(event);
            if pending.len() == BATCH_EVENTS {
                flush(&store, &mut pending, now_ms, &mut tally)?;
            }
        }
        flush(&store, &mut pending, now_ms, &mut tally)?;
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

fn flush(
    store: &Mutex<&mut EventStore>,
    pending: &mut Vec<Event>,
    now_ms: u64,
    tally: &mut Tally,
) -> Result<()> {
    if pending.is_empty() {
        return Ok(());
    }
    let events = std::mem::take(pending);
    let mut guard = lock(store)?;
    apply_batch(&mut guard, events, now_ms, tally)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::chain::ChainExt;
    use crate::waiting::WaitingLimits;
    use p2panda_core::{Hash, SigningKey};

    use iroh::endpoint::{Endpoint, presets};
    use iroh::{EndpointAddr, RelayMode, TransportAddr};
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
        let server = endpoint(vec![ALPN.to_vec()]).await?;
        let client = endpoint(vec![]).await?;
        let target = loopback_addr(&server);
        let accepting = async {
            let incoming = server
                .accept()
                .await
                .context("endpoint closed before a connection")?;
            let conn = incoming.await?;
            let out = run(b, &conn, false, pb, NOW, Cut::Never).await;
            anyhow::Ok((out, conn))
        };
        let dialing = async {
            let conn = client.connect(target, ALPN).await?;
            let out = run(a, &conn, true, pa, NOW, cut).await;
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

    /// An interrupted session commits whole batches only. A restart from scratch sends only the
    /// missing events, with no duplicate.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_an_interrupted_session_restarts_without_duplicates() {
        let (_da, mut a) = open();
        let (_db, mut b) = open();
        populate(&mut a, &[(1, 300), (2, 300)]);
        let total = ids(&a);

        // A cuts after 400 events. B has decided one full batch of 256, and the other 144 died
        // with the session.
        let (first_a, first_b) = loopback(
            &mut a,
            &mut b,
            Params::default(),
            Params::default(),
            Cut::AfterRecords(400),
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

    /// Bootstrap of a store whose sync set needs many frames: a 4 KiB budget splits every ID
    /// reply, and each direction converges.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_bootstrap_over_many_frames_in_both_directions_on_real_stores() {
        let budget = Params::default().with_frame_budget(4096).unwrap();
        let chains: Vec<(u64, u32)> = (1..=30).map(|c| (c, 40)).collect();
        for full_dials in [true, false] {
            let (_df, mut full) = open();
            let (_de, mut empty) = open();
            populate(&mut full, &chains);
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
            // Chunking adds frames, not messages, so the control bytes show the split: 1,200 IDs
            // are 38 KB, which is many 4 KiB frames.
            assert!(
                dialer.control_bytes + acceptor.control_bytes > 4 * 4096,
                "{label}: the ID replies were split into frames"
            );
            assert_eq!(dialer.duplicates + acceptor.duplicates, 0, "{label}");
        }
    }
}
