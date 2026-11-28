//! One sync session over an iroh bidirectional stream, and the two-endpoint runner.
//!
//! Protocol `/id-sync/1`, one stream per session:
//! 1. Reconciliation. Flights alternate as in S1's `reconcile`. The dialer sends the full-range
//!    fingerprint, and each side answers. A message with only Skip entries ends the loop. Each
//!    message is one length-prefixed frame.
//! 2. Data. After reconciliation both sides hold the symmetric difference, so each knows what it
//!    lacks, and what it holds that the peer lacks. No request is needed. Each side sends its
//!    events in key order: a count frame, then one frame per record. Both sides send at once.
//!    The receiver verifies each event, checks that it was expected, and commits every `batch`
//!    events in one transaction.

use crate::chain::{self, split_record};
use crate::range::{self, Entry, Party, all_skip};
use crate::store::{Batch, Id, Item, Store, key_of};
use anyhow::{Context, Result, anyhow, bail, ensure};
use iroh::endpoint::{Connection, Endpoint, RecvStream, SendStream, presets};
use iroh::{EndpointAddr, RelayMode};
use std::collections::HashSet;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const ALPN: &[u8] = b"/id-sync/1";
const MAX_FRAME: usize = 16 << 20;

#[derive(Clone, Copy, Debug)]
pub struct Params {
    pub k: usize,
    pub t: usize,
    /// Events per commit on the receiving side.
    pub batch: usize,
}

/// Where to cut the connection, for the interruption test. The cut closes the connection.
/// Counts are this side's own writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cut {
    Never,
    /// Close before this side writes its reconciliation message `n + 1`.
    AfterFlights(u32),
    /// Close before this side writes event record `n + 1`.
    AfterRecords(usize),
}

#[derive(Debug, Default, Clone)]
pub struct Outcome {
    /// Reconciliation messages this side wrote.
    pub flights: u32,
    /// Bytes this side wrote in reconciliation, including 4-byte frame lengths.
    pub control_bytes: usize,
    /// Bytes this side wrote in the data phase: count frame, then records with frame lengths.
    pub data_bytes: usize,
    /// Event records this side wrote.
    pub sent: usize,
    pub received: usize,
    pub inserted: usize,
    pub duplicates: usize,
    /// IDs this side lacks (what it expected to receive).
    pub need: usize,
    /// Time spent in redb commits on this side (each one fsyncs).
    pub commit: Duration,
    /// Symmetric difference, as this side learned it.
    pub learned: HashSet<Id>,
}

async fn write_frame(send: &mut SendStream, payload: &[u8]) -> Result<usize> {
    ensure!(payload.len() <= MAX_FRAME, "frame of {} bytes", payload.len());
    send.write_all(&(payload.len() as u32).to_be_bytes()).await?;
    send.write_all(payload).await?;
    Ok(4 + payload.len())
}

async fn read_frame(recv: &mut RecvStream) -> Result<Vec<u8>> {
    let mut len = [0u8; 4];
    recv.read_exact(&mut len).await?;
    let len = u32::from_be_bytes(len) as usize;
    ensure!(len <= MAX_FRAME, "frame announces {len} bytes");
    let mut buf = vec![0u8; len];
    recv.read_exact(&mut buf).await?;
    Ok(buf)
}

/// Writes one reconciliation message, or closes the connection if the cut is due.
async fn flight(send: &mut SendStream, conn: &Connection, out: &mut Outcome, msg: &[Entry], cut: Cut) -> Result<()> {
    if cut == Cut::AfterFlights(out.flights) {
        conn.close(1u32.into(), b"cut");
        bail!("cut before flight {}", out.flights + 1);
    }
    out.control_bytes += write_frame(send, &range::encode(msg)).await?;
    out.flights += 1;
    Ok(())
}

/// Commits a batch, and returns how long the commit took (redb fsyncs each commit). Blocking
/// redb work runs in `block_in_place`, on the multi-thread runtime.
fn commit(store: &Store, pending: &mut Vec<Item>) -> Result<(Batch, Duration)> {
    let started = Instant::now();
    let batch = tokio::task::block_in_place(|| store.insert(pending))?;
    pending.clear();
    Ok((batch, started.elapsed()))
}

/// Runs one session on an open connection. `initiator` is the dialer.
pub async fn run(store: &Store, party: &Party, conn: &Connection, initiator: bool, p: Params, cut: Cut) -> Result<Outcome> {
    let (mut send, mut recv) = if initiator { conn.open_bi().await? } else { conn.accept_bi().await? };
    let mut out = Outcome::default();
    let mut learned = HashSet::new();

    // 1. Reconciliation, the same loop as S1's `reconcile`.
    if initiator {
        flight(&mut send, conn, &mut out, &[party.full_range()], cut).await?;
    }
    loop {
        let incoming = range::decode(&read_frame(&mut recv).await?)?;
        if all_skip(&incoming) {
            break;
        }
        let reply = party.respond(&incoming, p.t, p.k, &mut learned);
        let last = all_skip(&reply);
        flight(&mut send, conn, &mut out, &reply, cut).await?;
        if last {
            break;
        }
    }

    // 2. Data. `learned` is the symmetric difference, so both sides agree on it.
    let mut need: HashSet<Id> = HashSet::new();
    for id in &learned {
        if !store.has(id)? {
            need.insert(*id);
        }
    }
    let offers: Vec<usize> = (0..party.len()).filter(|&i| learned.contains(&party.ids()[i])).collect();

    let sending = async {
        let mut bytes = write_frame(&mut send, &(offers.len() as u32).to_be_bytes()).await?;
        for (sent, &i) in offers.iter().enumerate() {
            if cut == Cut::AfterRecords(sent) {
                conn.close(1u32.into(), b"cut");
                bail!("cut before record {}", sent + 1);
            }
            bytes += write_frame(&mut send, &store.record(&party.ids()[i])?).await?;
        }
        send.finish()?;
        anyhow::Ok(bytes)
    };

    let receiving = async {
        let count_frame = read_frame(&mut recv).await?;
        let count: [u8; 4] = count_frame.as_slice().try_into().map_err(|_| anyhow!("count frame is not 4 bytes"))?;
        let count = u32::from_be_bytes(count) as usize;
        ensure!(count == need.len(), "peer offers {count} events, and {} are missing here", need.len());
        let mut seen: HashSet<Id> = HashSet::with_capacity(count);
        let mut pending: Vec<Item> = Vec::with_capacity(p.batch);
        let mut total = Batch::default();
        let mut commit_time = Duration::ZERO;
        for _ in 0..count {
            let record = read_frame(&mut recv).await?;
            let (header, body) = split_record(&record)?;
            let v = chain::verify(header, body)?;
            ensure!(need.contains(&v.id), "event {} was not expected", hex::encode(v.id));
            ensure!(seen.insert(v.id), "event {} arrived twice", hex::encode(v.id));
            pending.push(Item { key: key_of(&v.author, v.seq), id: v.id, record });
            if pending.len() == p.batch {
                let (b, t) = commit(store, &mut pending)?;
                total.inserted += b.inserted;
                total.duplicates += b.duplicates;
                commit_time += t;
            }
        }
        if !pending.is_empty() {
            let (b, t) = commit(store, &mut pending)?;
            total.inserted += b.inserted;
            total.duplicates += b.duplicates;
            commit_time += t;
        }
        anyhow::Ok((count, total, commit_time))
    };

    let (sent_bytes, received) = tokio::join!(sending, receiving);
    out.data_bytes = sent_bytes?;
    out.sent = offers.len();
    let (count, total, commit_time) = received?;
    out.received = count;
    out.inserted = total.inserted;
    out.duplicates = total.duplicates;
    out.commit = commit_time;
    out.need = need.len();
    out.learned = learned;
    Ok(out)
}

/// An endpoint on localhost, with no relay and no address lookup. Dialled by address only.
pub async fn endpoint(alpns: Vec<Vec<u8>>) -> Result<Endpoint> {
    Ok(Endpoint::builder(presets::Minimal)
        .alpns(alpns)
        .relay_mode(RelayMode::Disabled)
        .clear_ip_transports()
        .bind_addr((Ipv4Addr::LOCALHOST, 0))?
        .bind()
        .await?)
}

/// One store and its index, shared with the accepting task.
#[derive(Clone)]
pub struct Side {
    pub store: Arc<Store>,
    pub party: Arc<Party>,
}

pub struct Run {
    pub dialer: Result<Outcome>,
    pub acceptor: Result<Outcome>,
    /// Connect to both sessions finishing. Indexes are built before the call.
    pub wall: Duration,
    /// QUIC bytes in both directions on the dialer's connection, including headers and TLS.
    pub transport_bytes: u64,
}

/// Syncs `dialer` and `acceptor` over one connection on localhost, with `cut` applied to the
/// dialer. Each side returns only after it has read everything the other side sent, so
/// closing the connections afterwards loses nothing.
pub async fn sync_pair(dialer: Side, acceptor: Side, p: Params, cut: Cut) -> Result<Run> {
    let server = endpoint(vec![ALPN.to_vec()]).await?;
    let client = endpoint(vec![]).await?;
    let sock = *server.addr().ip_addrs().next().context("server has no IP address")?;
    let target = EndpointAddr::new(server.id()).with_ip_addr(sock);

    let accepting = tokio::spawn({
        let server = server.clone();
        async move {
            let incoming = server.accept().await.context("endpoint closed before a connection")?;
            let conn = incoming.await?;
            let out = run(&acceptor.store, &acceptor.party, &conn, false, p, Cut::Never).await?;
            anyhow::Ok((out, conn))
        }
    });

    let started = Instant::now();
    let conn = client.connect(target, ALPN).await.context("connect")?;
    let dialing = run(&dialer.store, &dialer.party, &conn, true, p, cut);
    let (dialer_result, accepted) = tokio::time::timeout(Duration::from_secs(120), async {
        tokio::join!(dialing, accepting)
    })
    .await
    .map_err(|_| anyhow!("sync timed out after 120 s"))?;
    let wall = started.elapsed();

    let transport_bytes = {
        let s = conn.stats();
        s.udp_tx.bytes + s.udp_rx.bytes
    };
    let (acceptor_result, acceptor_conn) = match accepted? {
        Ok((out, conn)) => (Ok(out), Some(conn)),
        Err(e) => (Err(e), None),
    };
    conn.close(0u32.into(), b"done");
    if let Some(c) = acceptor_conn {
        c.close(0u32.into(), b"done");
    }
    client.close().await;
    server.close().await;
    Ok(Run {
        dialer: dialer_result,
        acceptor: acceptor_result,
        wall,
        transport_bytes,
    })
}
