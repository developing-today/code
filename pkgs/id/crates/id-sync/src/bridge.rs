//! The native dialer for the hosted signed-object bridge (#3818).
//!
//! The hosted half of the bridge — the endpoint, the protocol core
//! and the store — lives in `pkgs/id/hosted-worker` (PR #12911).
//! This module is the other leg: a native node speaking the same
//! wire from Rust, the `/id-range-sync/2` peer role over the bridge
//! protocol instead of an Iroh stream.
//!
//! The wire is one ordered binary message channel (a WebSocket in
//! production; [`BridgeTransport`] is the seam). One-byte tags:
//! CHALLENGE/AUTH (the ID-key proof of the hosted `bridge-auth.ts`:
//! an Ed25519 signature by the binding record's Iroh root key over
//! `id.bridge-auth.v1\n<binding>\n<nonce>\n<context>\n<body hash>`),
//! HELLO/RESUME/RESUMED (chain heads — the frontier evidence),
//! RANGE flights (the proof-carrying range exchange of
//! [`id_events::event_store::range`], never a height comparison),
//! `WANT`/`BATCH`/`WANT_END`/`ACK` (bounded batches; every batch is
//! acknowledged with its ingest outcomes, so a sender retries
//! exactly what was refused and a re-sent event is a duplicate),
//! DONE/TOKEN.
//!
//! Withholding is detected, not silent: at session end the
//! announced heads are read against the applied frontier and the
//! delivered set, producing the evidence record ([`Withholding`]).
//!
//! The session logic mirrors the hosted worker's reference client
//! (`bridge-session.ts`) step for step, so the two implementations
//! are one protocol. Underneath it, the cross-language fixtures
//! (`pkgs/id/tests/bridge_fixtures.rs`) pin the event bytes and the
//! BLAKE3 agreement.

use std::collections::{BTreeSet, HashSet};
use std::future::Future;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use ed25519_dalek::{Signer as _, SigningKey};
use p2panda_core::Hash;
use serde::{Deserialize, Serialize};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

use id_events::event_store::EventStore;
use id_events::event_store::range::{
    ENTRY_CHARGE, Entry, FRAME_CAP, Mode, Params, Party, all_skip, check_bounds, decode_frame,
    frame_entry_count, frames,
};
use id_events::event_store::sync::RangeStore;
use id_store::event_types::Ingest;
use id_store::range::{Id, Key};

use crate::hex::{hex_encode, hex_to_array};

/// Message tag: the server's challenge nonce (UTF-8 payload).
pub const TAG_CHALLENGE: u8 = 0x01;
/// Message tag: the client's ID-key proof (JSON payload).
pub const TAG_AUTH: u8 = 0x02;
/// Message tag: chain heads, both directions (JSON payload).
pub const TAG_HELLO: u8 = 0x10;
/// Message tag: one range flight frame.
pub const TAG_RANGE: u8 = 0x11;
/// Message tag: ids one side asks the other for (JSON payload).
pub const TAG_WANT: u8 = 0x12;
/// Message tag: one bounded batch of events.
pub const TAG_BATCH: u8 = 0x13;
/// Message tag: the session is complete, both directions.
pub const TAG_DONE: u8 = 0x14;
/// Message tag: a returning client's resume offer (JSON payload).
pub const TAG_RESUME: u8 = 0x15;
/// Message tag: the server's answer to a resume offer (JSON).
pub const TAG_RESUMED: u8 = 0x16;
/// Message tag: the resume token for the next session (JSON).
pub const TAG_TOKEN: u8 = TAG_RESUMED + 1;
/// Message tag: a batch's ingest outcomes (JSON payload).
pub const TAG_ACK: u8 = 0x18;
/// Message tag: a side has answered a want in full.
pub const TAG_WANT_END: u8 = 0x19;
/// Message tag: a named session failure (JSON payload).
pub const TAG_ERROR: u8 = 0x1f;

/// The bridge's receipt bounds, enforced exactly as the hosted
/// session enforces them (`DEFAULT_BRIDGE_LIMITS` there).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BridgeLimits {
    /// The largest single event, in bytes.
    pub max_event_bytes: usize,
    /// The most events in one batch.
    pub max_batch_events: usize,
    /// The largest batch, in bytes.
    pub max_batch_bytes: usize,
    /// The most ids in one want.
    pub max_want_ids: usize,
}

impl Default for BridgeLimits {
    fn default() -> Self {
        Self {
            max_event_bytes: 64 * 1024,
            max_batch_events: 64,
            max_batch_bytes: 1024 * 1024,
            max_want_ids: 512,
        }
    }
}

/// One author's chain head: the frontier evidence a HELLO announces
/// and withholding detection reads.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Head {
    /// The author's key, hex.
    pub author: String,
    /// The sequence number the author's next event takes.
    pub next_seq: u64,
    /// The author's last applied event id, hex; `None` when none.
    pub head: Option<String>,
}

/// The withholding evidence record: an author announced a head the
/// session never delivered, so the events between this side's
/// applied frontier and the announcement are named as undelivered.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Withholding {
    /// The author whose head was announced.
    pub author: String,
    /// The announced frontier.
    pub announced_next_seq: u64,
    /// The announced head event id, hex.
    pub announced_head: Option<String>,
    /// This side's applied frontier for the author at session end.
    pub actual_next_seq: u64,
    /// The event ids, hex and sorted, that never became held here.
    pub undelivered: Vec<String>,
}

/// One batch's ingest outcomes, counted by kind. The wire shape of
/// the server's ACK; locally, of a batch this side ingested.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutcomeSummary {
    /// Events in the batch.
    pub received: u64,
    /// Stored and applied.
    pub accepted: u64,
    /// Already held.
    pub duplicate: u64,
    /// Revealed a fork.
    pub fork: u64,
    /// Held for a dependency.
    pub waiting: u64,
    /// Refused.
    pub refused: u64,
    /// Stored but voided by the membership layer. The hosted store
    /// never produces these (voiding is native-only), so the field
    /// is absent from the wire shape and defaults to zero.
    #[serde(default, skip_serializing)]
    pub voided: u64,
}

/// Counts one batch's native ingest outcomes.
pub fn summarize_outcomes(outcomes: &[Ingest]) -> OutcomeSummary {
    let mut summary = OutcomeSummary {
        received: outcomes.len() as u64,
        ..OutcomeSummary::default()
    };
    for outcome in outcomes {
        match outcome {
            Ingest::Accepted { .. } => summary.accepted += 1,
            Ingest::Duplicate { .. } => summary.duplicate += 1,
            Ingest::Fork { .. } => summary.fork += 1,
            Ingest::Waiting { .. } => summary.waiting += 1,
            Ingest::Refused(_) => summary.refused += 1,
            Ingest::Voided { .. } => summary.voided += 1,
        }
    }
    summary
}

/// Encodes one batch: a big-endian event count, then per event a
/// big-endian length and the event bytes. The hosted codec's shape.
pub fn encode_batch(events: &[Vec<u8>]) -> Vec<u8> {
    let mut size = 4usize;
    for event in events {
        size += 4 + event.len();
    }
    let mut out = Vec::with_capacity(size);
    // The wire count and length fields are u32. A batch anywhere
    // near that size is far past the byte bound every sender
    // enforces, so saturation is unreachable in practice; it keeps
    // the encoding total rather than truncating silently.
    out.extend_from_slice(
        &u32::try_from(events.len())
            .unwrap_or(u32::MAX)
            .to_be_bytes(),
    );
    for event in events {
        out.extend_from_slice(&u32::try_from(event.len()).unwrap_or(u32::MAX).to_be_bytes());
        out.extend_from_slice(event);
    }
    out
}

/// Decodes one batch, enforcing the receipt bounds in the hosted
/// codec's order: total bytes, count, then per-event length, with
/// truncation and trailing bytes refused.
pub fn decode_batch(bytes: &[u8], limits: &BridgeLimits) -> Result<Vec<Vec<u8>>> {
    ensure!(
        bytes.len() <= limits.max_batch_bytes,
        "batch of {} bytes is over the {}-byte bound",
        bytes.len(),
        limits.max_batch_bytes
    );
    ensure!(bytes.len() >= 4, "batch is truncated");
    let count = u32::from_be_bytes(bytes[0..4].try_into().unwrap_or([0; 4])) as usize;
    ensure!(
        count <= limits.max_batch_events,
        "batch of {count} events is over the {}-event bound",
        limits.max_batch_events
    );
    let mut events = Vec::with_capacity(count);
    let mut at = 4usize;
    for _ in 0..count {
        ensure!(at + 4 <= bytes.len(), "batch is truncated");
        let len = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap_or([0; 4])) as usize;
        at += 4;
        ensure!(
            len <= limits.max_event_bytes,
            "event of {len} bytes is over the {}-byte bound",
            limits.max_event_bytes
        );
        ensure!(at + len <= bytes.len(), "batch is truncated");
        events.push(bytes[at..at + len].to_vec());
        at += len;
    }
    ensure!(at == bytes.len(), "trailing bytes after the batch");
    Ok(events)
}

/// The bridge-auth transcript: the exact bytes the ID key signs for
/// one leg of one call (`id.bridge-auth.v1`, the binding, the
/// nonce, the leg, the body hash — one per line).
pub fn auth_transcript(
    binding_id: &str,
    nonce_hex: &str,
    context: &str,
    body_hash_hex: &str,
) -> Vec<u8> {
    format!("id.bridge-auth.v1\n{binding_id}\n{nonce_hex}\n{context}\n{body_hash_hex}").into_bytes()
}

/// BLAKE3 of a leg's body, hex — the transcript's last line. The
/// empty body hashes to the pinned `af1349b9…` value the fixtures
/// hold.
pub fn body_hash_hex(body: &[u8]) -> String {
    hex_encode(Hash::digest(body).as_bytes())
}

/// A protocol failure the session itself named.
#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    /// The peer sent an ERROR frame: a named, final refusal.
    #[error("bridge session error {code}: {message}")]
    Session {
        /// The failure's code (e.g. `auth_failed`, `resume_rejected`).
        code: String,
        /// The failure's human-readable message.
        message: String,
    },
    /// The transport closed before the session completed.
    #[error("the transport closed mid-session")]
    Closed,
    /// A frame arrived out of order.
    #[error("expected tag {expected}, got {got}")]
    UnexpectedTag {
        /// The tag the session waited for.
        expected: u8,
        /// The tag that arrived.
        got: u8,
    },
}

/// One ordered binary message channel: the bridge's transport seam.
/// A production node speaks over a WebSocket (the implementation
/// for [`WebSocketStream`] below); tests speak over anything that
/// carries whole messages in order.
pub trait BridgeTransport {
    /// Sends one whole message.
    fn send(&mut self, data: Vec<u8>) -> impl Future<Output = Result<()>> + Send;
    /// Receives the next whole message, or `None` when the channel
    /// has closed.
    fn recv(&mut self) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send;
}

impl<S> BridgeTransport for WebSocketStream<S>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
{
    async fn send(&mut self, data: Vec<u8>) -> Result<()> {
        futures_util::SinkExt::send(self, Message::Binary(data.into()))
            .await
            .context("bridge websocket send")?;
        Ok(())
    }

    async fn recv(&mut self) -> Result<Option<Vec<u8>>> {
        loop {
            match futures_util::StreamExt::next(self).await {
                None => return Ok(None),
                Some(Ok(Message::Binary(bytes))) => return Ok(Some(bytes.to_vec())),
                Some(Ok(Message::Close(_))) => return Ok(None),
                // Pings are answered by tungstenite itself; text is
                // not bridge traffic. Neither ends the session.
                Some(Ok(_)) => {}
                Some(Err(err)) => return Err(err).context("bridge websocket receive"),
            }
        }
    }
}

/// Dials the hosted bridge endpoint and returns the WebSocket the
/// session runs over. The URL is the front door's `wss://…` (or
/// `ws://…` on a local route) address of `/v1/bridge/ws`.
pub async fn connect(
    url: &str,
) -> Result<WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>> {
    let (stream, _response) = tokio_tungstenite::connect_async(url)
        .await
        .context("bridge websocket connect")?;
    Ok(stream)
}

/// The store access one bridge session needs, one step at a time —
/// the hosted `BridgeStore`'s native counterpart. Implemented for
/// the native [`EventStore`]; sessions touch the store in short
/// steps and never hold it across an await.
pub trait BridgeStore {
    /// Every author's applied head, in author order.
    fn bridge_heads(&self) -> Result<Vec<Head>>;
    /// The sync set (stored events and fork proof headers), as
    /// range keys and ids.
    fn bridge_sync_entries(&self) -> Result<Vec<(Key, Id)>>;
    /// The bytes of one sync-set event, if held in any form.
    fn bridge_event_bytes(&self, id: &Id) -> Result<Option<Vec<u8>>>;
    /// Decides one received batch, in order, in one transaction.
    fn bridge_ingest(&mut self, batch: &[Vec<u8>], now_ms: u64) -> Result<Vec<Ingest>>;
    /// The applied frontier of one author: the sequence number its
    /// next applied event takes, zero when it has none.
    fn bridge_chain_next_seq(&self, author: &[u8; 32]) -> Result<u64>;
}

impl BridgeStore for EventStore {
    fn bridge_heads(&self) -> Result<Vec<Head>> {
        let mut heads = Vec::new();
        for (author, chain) in self.chains()? {
            heads.push(Head {
                author: hex_encode(author.as_slice()),
                next_seq: u64::from(chain.next_seq),
                head: chain.head.map(|head| hex_encode(head.as_bytes())),
            });
        }
        Ok(heads)
    }

    fn bridge_sync_entries(&self) -> Result<Vec<(Key, Id)>> {
        RangeStore::sync_entries(self)
    }

    fn bridge_event_bytes(&self, id: &Id) -> Result<Option<Vec<u8>>> {
        RangeStore::sync_bytes(self, id)
    }

    fn bridge_ingest(&mut self, batch: &[Vec<u8>], now_ms: u64) -> Result<Vec<Ingest>> {
        Ok(EventStore::ingest_batch(self, batch, now_ms)?)
    }

    fn bridge_chain_next_seq(&self, author: &[u8; 32]) -> Result<u64> {
        for (candidate, chain) in self.chains()? {
            if candidate.as_slice() == author {
                return Ok(u64::from(chain.next_seq));
            }
        }
        Ok(0)
    }
}

/// What one completed session did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionResult {
    /// The resume token the store issued for the next session.
    pub token: Option<String>,
    /// Withholding evidence, one record per author whose announced
    /// head the session did not deliver.
    pub withholding: Vec<Withholding>,
    /// The ingest summaries of the batches this side received, in
    /// arrival order.
    pub acks: Vec<OutcomeSummary>,
    /// The heads the server announced.
    pub server_heads: Vec<Head>,
}

/// The native bridge dialer: the client half of one bridge
/// relationship — one binding, one ID key, one hosted store. Holds
/// the resume token between sessions so a returning sync resumes
/// instead of re-deriving from HELLO (the range exchange re-derives
/// the truth either way; positions are evidence, never authority).
#[derive(Clone)]
pub struct BridgeDialer {
    binding_id: String,
    key: SigningKey,
    params: Params,
    limits: BridgeLimits,
    resume_token: Option<String>,
}

impl std::fmt::Debug for BridgeDialer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The key never prints; the resume token is a bearer
        // credential and does not print either.
        f.debug_struct("BridgeDialer")
            .field("binding_id", &self.binding_id)
            .field(
                "resume_token",
                &self.resume_token.as_ref().map(|_| "<held>"),
            )
            .finish_non_exhaustive()
    }
}

impl BridgeDialer {
    /// A dialer for `binding_id`, proving with `key` — the ID key
    /// the binding record names (the account's Iroh root key).
    pub fn new(binding_id: impl Into<String>, key: SigningKey) -> Self {
        Self {
            binding_id: binding_id.into(),
            key,
            params: Params::default(),
            limits: BridgeLimits::default(),
            resume_token: None,
        }
    }

    /// Sets the range-reconciliation parameters (local policy; K
    /// and T never go on the wire).
    #[must_use]
    pub fn with_params(mut self, params: Params) -> Self {
        self.params = params;
        self
    }

    /// Sets the receipt bounds.
    #[must_use]
    pub fn with_limits(mut self, limits: BridgeLimits) -> Self {
        self.limits = limits;
        self
    }

    /// The resume token held for the next session, if any.
    pub fn resume_token(&self) -> Option<&str> {
        self.resume_token.as_deref()
    }

    /// Replaces the held resume token (a persisted token from a
    /// previous process, or `None` to forget it).
    pub fn set_resume_token(&mut self, token: Option<String>) {
        self.resume_token = token;
    }

    /// Runs one session over an open transport and adopts the
    /// resume token it issues.
    pub async fn sync<S: BridgeStore, T: BridgeTransport>(
        &mut self,
        store: &mut S,
        transport: &mut T,
        now_ms: u64,
    ) -> Result<SessionResult> {
        let result = self.run_session(store, transport, now_ms).await?;
        if result.token.is_some() {
            self.resume_token = result.token.clone();
        }
        Ok(result)
    }

    /// [`sync`](Self::sync) with retry and backoff around it, the
    /// hosted client's discipline: a dropped session resumes with
    /// the last token when one exists, and a rejected resume is not
    /// a failure of the sync itself — the token is stale, so the
    /// next attempt opens with HELLO and re-derives everything
    /// through the range exchange. Positions come from the store,
    /// so a retry never re-sends what already landed.
    pub async fn sync_with_retry<S, T, F, Fut>(
        &mut self,
        store: &mut S,
        mut make_transport: F,
        attempts: u32,
        base_delay_ms: u64,
        now_ms: u64,
    ) -> Result<SessionResult>
    where
        S: BridgeStore,
        T: BridgeTransport,
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        let mut last_error: Option<anyhow::Error> = None;
        for attempt in 0..attempts {
            let outcome = async {
                let mut transport = make_transport().await?;
                self.sync(store, &mut transport, now_ms).await
            }
            .await;
            match outcome {
                Ok(result) => return Ok(result),
                Err(err) => {
                    let resume_rejected = err.downcast_ref::<BridgeError>().is_some_and(|e| {
                        matches!(e, BridgeError::Session { code, .. } if code == "resume_rejected")
                    });
                    if resume_rejected {
                        self.resume_token = None;
                    }
                    last_error = Some(err);
                    if attempt + 1 < attempts {
                        let delay = base_delay_ms.saturating_mul(2u64.saturating_pow(attempt));
                        tokio::time::sleep(Duration::from_millis(delay)).await;
                    }
                }
            }
        }
        match last_error {
            Some(err) => Err(err),
            None => bail!("the bridge sync made no attempt"),
        }
    }

    async fn run_session<S: BridgeStore, T: BridgeTransport>(
        &self,
        store: &mut S,
        transport: &mut T,
        now_ms: u64,
    ) -> Result<SessionResult> {
        // AUTH: answer the server's challenge with the ID-key proof.
        // The WebSocket leg authorizes an empty body.
        let challenge = next_tag(transport, TAG_CHALLENGE).await?;
        let nonce = String::from_utf8(challenge).context("the bridge challenge is not UTF-8")?;
        let transcript = auth_transcript(&self.binding_id, &nonce, "ws", &body_hash_hex(&[]));
        let signature = self.key.sign(&transcript);
        send_json(
            transport,
            TAG_AUTH,
            &AuthBody {
                nonce: &nonce,
                signature: hex_encode(&signature.to_bytes()),
            },
        )
        .await?;

        // HELLO, or RESUME when a token is held.
        let server_heads: Vec<Head> = if let Some(token) = &self.resume_token {
            let positions = store.bridge_heads()?;
            send_json(transport, TAG_RESUME, &ResumeBody { token, positions }).await?;
            let resumed: ResumedBody = parse_json(&next_tag(transport, TAG_RESUMED).await?)?;
            resumed.positions
        } else {
            send_json(transport, TAG_HELLO, &store.bridge_heads()?).await?;
            parse_json(&next_tag(transport, TAG_HELLO).await?)?
        };

        // RANGE: ping-pong flights until both sides have sent
        // all-skip. The budget charges every received frame, as the
        // hosted session charges.
        let mut learned: Vec<Id> = Vec::new();
        let mut spent: usize = 0;
        let mut party = Party::new(store.bridge_sync_entries()?)?;
        let mut msg: Vec<Entry> = vec![party.full_range()];
        let mut sent_skip = false;
        let mut received_skip = false;
        for _round in 0..64 {
            if sent_skip && received_skip {
                break;
            }
            for wire in frames(&msg, self.params.frame_budget())? {
                send_frame(transport, TAG_RANGE, &wire).await?;
            }
            sent_skip = all_skip(&msg);
            let mut flight_frames: Vec<Vec<u8>> = Vec::new();
            loop {
                let payload = next_tag(transport, TAG_RANGE).await?;
                spent += payload.len() + frame_entry_count(&payload)? * ENTRY_CHARGE;
                ensure!(
                    spent <= self.params.session_budget(),
                    "range session budget exceeded"
                );
                let (more, _entries) = decode_frame(&payload)?;
                flight_frames.push(payload);
                if !more {
                    break;
                }
            }
            let flight = decode_flight(&flight_frames)?;
            received_skip = all_skip(&flight);
            if received_skip && sent_skip {
                break;
            }
            if received_skip {
                msg = vec![Entry {
                    upper: None,
                    mode: Mode::Skip,
                }];
                continue;
            }
            check_bounds(&flight)?;
            party = Party::new(store.bridge_sync_entries()?)?;
            msg = party.respond(&flight, self.params, &mut learned);
        }

        // WANT: ask for everything learned that the store does not
        // hold (bounded like any want; the rest stays missing and
        // withholding detection reads it below).
        let mut missing: Vec<Id> = Vec::new();
        for id in &learned {
            if !missing.contains(id) && store.bridge_event_bytes(id)?.is_none() {
                missing.push(*id);
            }
        }
        let want: Vec<String> = missing
            .iter()
            .take(self.limits.max_want_ids)
            .map(|id| hex_encode(id))
            .collect();
        send_json(transport, TAG_WANT, &want).await?;
        let mut acks: Vec<OutcomeSummary> = Vec::new();
        let mut received: HashSet<Id> = HashSet::new();
        let mut authored: Vec<(String, Id)> = Vec::new();
        loop {
            let (tag, payload) = next_frame(transport).await?;
            match tag {
                TAG_BATCH => {
                    let raw_events = decode_batch(&payload, &self.limits)?;
                    for raw in &raw_events {
                        if let Ok(event) = id_store::chain::decode(raw) {
                            received.insert(*event.id.as_bytes());
                            authored.push((
                                hex_encode(event.header.verifying_key.as_bytes()),
                                *event.id.as_bytes(),
                            ));
                        }
                    }
                    let outcomes = store.bridge_ingest(&raw_events, now_ms)?;
                    acks.push(summarize_outcomes(&outcomes));
                }
                TAG_WANT_END => break,
                TAG_ERROR => return Err(session_error(&payload).into()),
                other => bail!("unexpected tag {other} during transfer"),
            }
        }

        // The server's WANT: answer with batches, then WANT_END,
        // then DONE. The server's ACKs for the answer batches are
        // informational and drained while waiting for its DONE.
        let server_want: Vec<String> = parse_json(&next_tag(transport, TAG_WANT).await?)?;
        let mut answer: Vec<Vec<u8>> = Vec::new();
        for text in &server_want {
            if let Some(id) = hex_to_array::<32>(text) {
                if let Some(bytes) = store.bridge_event_bytes(&id)? {
                    answer.push(bytes);
                }
            }
        }
        send_batches(transport, &answer, &self.limits).await?;
        send_frame(transport, TAG_WANT_END, &[]).await?;
        send_frame(transport, TAG_DONE, &[]).await?;
        let mut token: Option<String> = None;
        loop {
            let (tag, payload) = next_frame(transport).await?;
            match tag {
                TAG_TOKEN => {
                    let body: TokenBody = parse_json(&payload)?;
                    token = Some(body.token);
                }
                TAG_DONE => break,
                TAG_ACK => {}
                TAG_ERROR => return Err(session_error(&payload).into()),
                other => bail!("unexpected tag {other} while closing"),
            }
        }

        // Withholding: what was wanted, never arrived, and is still
        // not held — read against the announced heads.
        let mut still_missing: HashSet<Id> = HashSet::new();
        for id in &missing {
            if !received.contains(id) && store.bridge_event_bytes(id)?.is_none() {
                still_missing.insert(*id);
            }
        }
        let withholding = detect_withholding(&server_heads, store, &authored, &still_missing)?;
        Ok(SessionResult {
            token,
            withholding,
            acks,
            server_heads,
        })
    }
}

/// The withholding evidence, the investigation's shape: for each
/// announced head this side cannot apply, the events the peer
/// authored (from what arrived, plus the announced head itself when
/// it is among the missing) that never became held here. An author
/// whose events arrived but wait on a withheld dependency is not
/// itself withholding — its own head applies or waits on another
/// author's gap, and the record names the author actually missing.
fn detect_withholding<S: BridgeStore>(
    announced: &[Head],
    store: &S,
    authored: &[(String, Id)],
    missing: &HashSet<Id>,
) -> Result<Vec<Withholding>> {
    let held: HashSet<Id> = store
        .bridge_sync_entries()?
        .into_iter()
        .map(|(_key, id)| id)
        .collect();
    let mut out = Vec::new();
    for head in announced {
        let actual_next_seq = match hex_to_array::<32>(&head.author) {
            Some(author) => store.bridge_chain_next_seq(&author)?,
            None => 0,
        };
        if actual_next_seq >= head.next_seq {
            continue;
        }
        let mut undelivered: BTreeSet<String> = BTreeSet::new();
        for (author, id) in authored {
            if *author == head.author && !held.contains(id) {
                undelivered.insert(hex_encode(id));
            }
        }
        if let Some(head_id) = &head.head {
            if let Some(id) = hex_to_array::<32>(head_id) {
                if missing.contains(&id) {
                    undelivered.insert(head_id.clone());
                }
            }
        }
        if !undelivered.is_empty() {
            out.push(Withholding {
                author: head.author.clone(),
                announced_next_seq: head.next_seq,
                announced_head: head.head.clone(),
                actual_next_seq,
                undelivered: undelivered.into_iter().collect(),
            });
        }
    }
    Ok(out)
}

fn frame(tag: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + payload.len());
    out.push(tag);
    out.extend_from_slice(payload);
    out
}

async fn send_frame<T: BridgeTransport>(transport: &mut T, tag: u8, payload: &[u8]) -> Result<()> {
    transport.send(frame(tag, payload)).await
}

async fn send_json<T: BridgeTransport, V: Serialize>(
    transport: &mut T,
    tag: u8,
    value: &V,
) -> Result<()> {
    let payload = serde_json::to_vec(value).context("a bridge frame is not serializable")?;
    send_frame(transport, tag, &payload).await
}

fn parse_json<V: serde::de::DeserializeOwned>(payload: &[u8]) -> Result<V> {
    serde_json::from_slice(payload).context("a bridge frame is not the expected JSON")
}

/// Sends events as bounded batches, the hosted sender's flush
/// discipline: a batch closes when the next event would cross the
/// count or the byte bound.
async fn send_batches<T: BridgeTransport>(
    transport: &mut T,
    events: &[Vec<u8>],
    limits: &BridgeLimits,
) -> Result<()> {
    let mut batch: Vec<Vec<u8>> = Vec::new();
    // The encoded size of the batch so far — the 4-byte count
    // prefix plus each event's 4-byte length prefix and payload.
    // decode_batch enforces max_batch_bytes against that encoded
    // size, not the payload sum (#14644).
    let mut batch_bytes: usize = 4;
    for event in events {
        if !batch.is_empty()
            && (batch.len() >= limits.max_batch_events
                || batch_bytes + 4 + event.len() > limits.max_batch_bytes)
        {
            send_frame(transport, TAG_BATCH, &encode_batch(&batch)).await?;
            batch.clear();
            batch_bytes = 4;
        }
        ensure!(
            batch_bytes + 4 + event.len() <= limits.max_batch_bytes,
            "event of {} bytes cannot fit the {}-byte batch bound",
            event.len(),
            limits.max_batch_bytes
        );
        batch.push(event.clone());
        batch_bytes += 4 + event.len();
    }
    if !batch.is_empty() {
        send_frame(transport, TAG_BATCH, &encode_batch(&batch)).await?;
    }
    Ok(())
}

/// Reads one frame and splits its tag. The frame cap is enforced
/// on receipt, as the hosted session enforces it.
async fn next_frame<T: BridgeTransport>(transport: &mut T) -> Result<(u8, Vec<u8>)> {
    let data = transport.recv().await?.ok_or(BridgeError::Closed)?;
    ensure!(!data.is_empty(), "an empty bridge frame");
    ensure!(
        data.len() <= FRAME_CAP,
        "a bridge frame of {} bytes is over the {FRAME_CAP}-byte cap",
        data.len()
    );
    Ok((data[0], data[1..].to_vec()))
}

/// Reads one frame of an expected tag; an ERROR frame is the
/// session's named failure, never a tag mismatch.
async fn next_tag<T: BridgeTransport>(transport: &mut T, tag: u8) -> Result<Vec<u8>> {
    let (got, payload) = next_frame(transport).await?;
    if got == TAG_ERROR {
        return Err(session_error(&payload).into());
    }
    if got != tag {
        return Err(BridgeError::UnexpectedTag { expected: tag, got }.into());
    }
    Ok(payload)
}

fn session_error(payload: &[u8]) -> BridgeError {
    match parse_json::<ErrorBody>(payload) {
        Ok(body) => BridgeError::Session {
            code: body.code,
            message: body.message,
        },
        Err(_) => BridgeError::Session {
            code: "session_error".to_owned(),
            message: "an undecodable error frame".to_owned(),
        },
    }
}

/// Decodes a whole flight (one or more frames) into its entries.
fn decode_flight(frames: &[Vec<u8>]) -> Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for frame in frames {
        let (_more, mut frame_entries) = decode_frame(frame)?;
        entries.append(&mut frame_entries);
    }
    Ok(entries)
}

#[derive(Serialize)]
struct AuthBody<'a> {
    nonce: &'a str,
    signature: String,
}

#[derive(Serialize)]
struct ResumeBody<'a> {
    token: &'a str,
    positions: Vec<Head>,
}

#[derive(Deserialize)]
struct ResumedBody {
    positions: Vec<Head>,
}

#[derive(Deserialize)]
struct TokenBody {
    token: String,
}

#[derive(Deserialize)]
struct ErrorBody {
    code: String,
    message: String,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use ed25519_dalek::{Signature as DalekSignature, Verifier as _, VerifyingKey};
    use p2panda_core::SigningKey as PandaSigningKey;
    use tokio::sync::mpsc;

    use id_core::waiting::WaitingLimits;
    use id_store::chain::{ChainExt, sign};

    const BINDING: &str = "binding-1";
    const NOW_MS: u64 = 0;

    // ----- fixtures -------------------------------------------------

    fn dalek_key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn message_ext() -> ChainExt {
        ChainExt {
            kind: "message".to_owned(),
            group: None,
            after: vec![],
            tag: None,
            document: None,
            removes: None,
            note: None,
            link: None,
            checkpoint: None,
            rotation: None,
            chat: None,
            anchor: None,
        }
    }

    /// A signed chain of `count` events by the author with `seed`.
    fn make_events(seed: u8, count: u64) -> Vec<Vec<u8>> {
        let key = PandaSigningKey::from_bytes(&[seed; 32]);
        let mut out = Vec::new();
        let mut backlink = None;
        for seq in 0..count {
            let event = sign(
                &key,
                p2panda_core::SeqNum::try_from(seq).unwrap(),
                backlink,
                None,
                message_ext(),
            )
            .unwrap();
            backlink = Some(event.id);
            out.push(event.bytes);
        }
        out
    }

    fn author_hex(seed: u8) -> String {
        let key = PandaSigningKey::from_bytes(&[seed; 32]);
        hex_encode(key.verifying_key().as_bytes())
    }

    fn open_store() -> (tempfile::TempDir, EventStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = EventStore::open(dir.path(), WaitingLimits::default(), NOW_MS).unwrap();
        (dir, store)
    }

    // ----- the in-memory transport pair ------------------------------

    struct PairTransport {
        tx: mpsc::UnboundedSender<Vec<u8>>,
        rx: mpsc::UnboundedReceiver<Vec<u8>>,
    }

    impl BridgeTransport for PairTransport {
        async fn send(&mut self, data: Vec<u8>) -> Result<()> {
            self.tx
                .send(data)
                .map_err(|_| anyhow::anyhow!("the peer is gone"))
        }

        async fn recv(&mut self) -> Result<Option<Vec<u8>>> {
            Ok(self.rx.recv().await)
        }
    }

    fn pair() -> (PairTransport, PairTransport) {
        let (a_tx, a_rx) = mpsc::unbounded_channel();
        let (b_tx, b_rx) = mpsc::unbounded_channel();
        (
            PairTransport { tx: a_tx, rx: b_rx },
            PairTransport { tx: b_tx, rx: a_rx },
        )
    }

    // ----- the test server: the hosted session, mirrored ------------

    #[derive(Deserialize)]
    struct AuthIn {
        nonce: String,
        signature: String,
    }

    #[derive(Deserialize)]
    struct ResumeIn {
        token: String,
        positions: Vec<Head>,
    }

    struct TestServer {
        store: EventStore,
        binding_id: String,
        bound_key: VerifyingKey,
        tokens: HashMap<String, String>,
        token_counter: u64,
        learned: Vec<Id>,
        withhold_all: bool,
        saw_hello: Arc<AtomicBool>,
        saw_resume: Arc<AtomicBool>,
    }

    impl TestServer {
        fn new(store: EventStore, bound_key: VerifyingKey, withhold_all: bool) -> Self {
            Self {
                store,
                binding_id: BINDING.to_owned(),
                bound_key,
                tokens: HashMap::new(),
                token_counter: 0,
                learned: Vec::new(),
                withhold_all,
                saw_hello: Arc::new(AtomicBool::new(false)),
                saw_resume: Arc::new(AtomicBool::new(false)),
            }
        }

        async fn fail(&self, transport: &mut PairTransport, code: &str) -> Result<()> {
            send_json(
                transport,
                TAG_ERROR,
                &serde_json::json!({ "code": code, "message": code }),
            )
            .await
        }

        async fn run(mut self, mut transport: PairTransport) -> Result<Self> {
            let nonce = "cd".repeat(32);
            send_frame(&mut transport, TAG_CHALLENGE, nonce.as_bytes()).await?;
            let mut authed = false;
            let mut flight_frames: Vec<Vec<u8>> = Vec::new();
            loop {
                let Some(data) = transport.recv().await? else {
                    return Ok(self);
                };
                if data.is_empty() {
                    continue;
                }
                let (tag, payload) = (data[0], data[1..].to_vec());
                if !authed {
                    if tag != TAG_AUTH {
                        self.fail(&mut transport, "auth_required").await?;
                        return Ok(self);
                    }
                    let body: AuthIn = serde_json::from_slice(&payload).unwrap();
                    let transcript =
                        auth_transcript(BINDING, &body.nonce, "ws", &body_hash_hex(&[]));
                    let signature =
                        DalekSignature::from_bytes(&hex_to_array::<64>(&body.signature).unwrap());
                    if body.nonce != nonce
                        || self.bound_key.verify(&transcript, &signature).is_err()
                    {
                        self.fail(&mut transport, "auth_failed").await?;
                        return Ok(self);
                    }
                    authed = true;
                    continue;
                }
                match tag {
                    TAG_HELLO => {
                        self.saw_hello.store(true, Ordering::SeqCst);
                        let heads = self.store.bridge_heads()?;
                        send_json(&mut transport, TAG_HELLO, &heads).await?;
                    }
                    TAG_RESUME => {
                        let body: ResumeIn = serde_json::from_slice(&payload).unwrap();
                        if self.tokens.get(&body.token) != Some(&self.binding_id) {
                            self.fail(&mut transport, "resume_rejected").await?;
                            return Ok(self);
                        }
                        self.saw_resume.store(true, Ordering::SeqCst);
                        let positions: Vec<Head> = self
                            .store
                            .bridge_heads()?
                            .into_iter()
                            .filter(|h| body.positions.iter().any(|p| p.author == h.author))
                            .collect();
                        send_json(
                            &mut transport,
                            TAG_RESUMED,
                            &serde_json::json!({ "positions": positions }),
                        )
                        .await?;
                    }
                    TAG_RANGE => {
                        flight_frames.push(payload);
                        let (more, _entries) =
                            decode_frame(&flight_frames.last().unwrap().clone())?;
                        if !more {
                            let flight = decode_flight(&flight_frames)?;
                            flight_frames.clear();
                            if all_skip(&flight) {
                                let skip = vec![Entry {
                                    upper: None,
                                    mode: Mode::Skip,
                                }];
                                let wires = frames(&skip, Params::default().frame_budget())?;
                                send_frame(&mut transport, TAG_RANGE, &wires[0]).await?;
                            } else {
                                check_bounds(&flight)?;
                                let party = Party::new(self.store.bridge_sync_entries()?)?;
                                let reply =
                                    party.respond(&flight, Params::default(), &mut self.learned);
                                for wire in frames(&reply, Params::default().frame_budget())? {
                                    send_frame(&mut transport, TAG_RANGE, &wire).await?;
                                }
                            }
                        }
                    }
                    TAG_WANT => {
                        let ids: Vec<String> = serde_json::from_slice(&payload).unwrap();
                        if !self.withhold_all {
                            let mut held = Vec::new();
                            for text in &ids {
                                if let Some(id) = hex_to_array::<32>(text) {
                                    if let Some(bytes) = self.store.bridge_event_bytes(&id)? {
                                        held.push(bytes);
                                    }
                                }
                            }
                            send_batches(&mut transport, &held, &BridgeLimits::default()).await?;
                        }
                        send_frame(&mut transport, TAG_WANT_END, &[]).await?;
                        let mut want: Vec<String> = Vec::new();
                        for id in &self.learned {
                            if self.store.bridge_event_bytes(id)?.is_none() {
                                let hex = hex_encode(id);
                                if !want.contains(&hex) {
                                    want.push(hex);
                                }
                            }
                        }
                        send_json(&mut transport, TAG_WANT, &want).await?;
                    }
                    TAG_BATCH => {
                        let events = decode_batch(&payload, &BridgeLimits::default())?;
                        let outcomes = self.store.bridge_ingest(&events, NOW_MS)?;
                        send_json(&mut transport, TAG_ACK, &summarize_outcomes(&outcomes)).await?;
                    }
                    TAG_WANT_END => {}
                    TAG_DONE => {
                        self.token_counter += 1;
                        let token = format!("{:064x}", self.token_counter);
                        self.tokens.insert(token.clone(), self.binding_id.clone());
                        send_json(
                            &mut transport,
                            TAG_TOKEN,
                            &serde_json::json!({ "token": token }),
                        )
                        .await?;
                        send_frame(&mut transport, TAG_DONE, &[]).await?;
                        return Ok(self);
                    }
                    _ => {
                        self.fail(&mut transport, "unknown_tag").await?;
                        return Ok(self);
                    }
                }
            }
        }
    }

    // ----- codec and transcript pins ---------------------------------

    #[test]
    fn batch_codec_round_trips_and_refuses() {
        let limits = BridgeLimits::default();
        let events = vec![vec![1u8; 10], vec![2u8; 3], Vec::new()];
        let encoded = encode_batch(&events);
        assert_eq!(decode_batch(&encoded, &limits).unwrap(), events);

        // Trailing bytes and truncation are refused.
        let mut trailed = encoded.clone();
        trailed.push(0);
        assert!(decode_batch(&trailed, &limits).is_err());
        assert!(decode_batch(&encoded[..encoded.len() - 1], &limits).is_err());

        // The count bound.
        let many = vec![Vec::new(); 65];
        assert!(decode_batch(&encode_batch(&many), &limits).is_err());

        // The per-event bound.
        let tight = BridgeLimits {
            max_event_bytes: 4,
            ..BridgeLimits::default()
        };
        assert!(decode_batch(&encode_batch(&[vec![0u8; 5]]), &tight).is_err());

        // The total bound.
        let small = BridgeLimits {
            max_batch_bytes: 8,
            ..BridgeLimits::default()
        };
        assert!(decode_batch(&encoded, &small).is_err());
    }

    #[tokio::test]
    async fn send_batches_budgets_the_encoded_size() {
        // The #14644 boundary at default limits: 16 x 64 KiB events
        // fit the payload sum (1 MiB) but not the encoded batch
        // (4-byte count + 4-byte length per event). Every emitted
        // batch must decode under the same limits, in order.
        let limits = BridgeLimits::default();
        let events: Vec<Vec<u8>> = (0..16u8).map(|i| vec![i; 65536]).collect();
        let (mut sender, mut receiver) = pair();
        send_batches(&mut sender, &events, &limits).await.unwrap();
        drop(sender);
        let mut frames = 0;
        let mut received: Vec<Vec<u8>> = Vec::new();
        while let Some(frame) = receiver.recv().await.unwrap() {
            frames += 1;
            assert_eq!(frame[0], TAG_BATCH);
            let decoded = decode_batch(&frame[1..], &limits).unwrap();
            received.extend(decoded);
        }
        assert_eq!(frames, 2);
        assert_eq!(received, events);
    }

    #[tokio::test]
    async fn send_batches_splits_at_the_first_overflow() {
        // Encoded accounting: 4 + (4 + 44) + (4 + 44) = 100 fits
        // exactly; the third event opens a new batch. Payload-only
        // accounting would also have paired two 48-byte events
        // (96 <= 100) whose encoded size (108) the decoder refuses.
        let limits = BridgeLimits {
            max_batch_bytes: 100,
            ..BridgeLimits::default()
        };
        let events: Vec<Vec<u8>> = vec![vec![1u8; 44], vec![2u8; 44], vec![3u8; 44]];
        let (mut sender, mut receiver) = pair();
        send_batches(&mut sender, &events, &limits).await.unwrap();
        drop(sender);
        let mut counts = Vec::new();
        while let Some(frame) = receiver.recv().await.unwrap() {
            counts.push(decode_batch(&frame[1..], &limits).unwrap().len());
        }
        assert_eq!(counts, vec![2, 1]);

        let events: Vec<Vec<u8>> = vec![vec![1u8; 48], vec![2u8; 48]];
        let (mut sender, mut receiver) = pair();
        send_batches(&mut sender, &events, &limits).await.unwrap();
        drop(sender);
        let mut counts = Vec::new();
        while let Some(frame) = receiver.recv().await.unwrap() {
            counts.push(decode_batch(&frame[1..], &limits).unwrap().len());
        }
        assert_eq!(counts, vec![1, 1]);
    }

    #[tokio::test]
    async fn send_batches_names_an_event_that_cannot_fit() {
        let limits = BridgeLimits {
            max_batch_bytes: 100,
            ..BridgeLimits::default()
        };
        let (mut sender, _receiver) = pair();
        let err = send_batches(&mut sender, &[vec![0u8; 200]], &limits)
            .await
            .unwrap_err();
        assert!(
            format!("{err:#}").contains("cannot fit the 100-byte batch bound"),
            "{err:#}"
        );
    }

    #[tokio::test]
    async fn send_batches_property_every_batch_decodes_under_the_same_limits() {
        // Oracle (#14644): for generated size sequences and limit
        // sets, every emitted batch decodes under the same limits
        // and the concatenation preserves the original sequence.
        let mut state = 0x2f6e_2b1u32;
        let mut rand = move |n: usize| {
            state = state.wrapping_mul(1103515245).wrapping_add(12345) & 0x7fff_ffff;
            (state as usize) % n
        };
        for _trial in 0..100 {
            let limits = BridgeLimits {
                max_event_bytes: 4096,
                max_batch_events: 1 + rand(8),
                max_batch_bytes: 64 + rand(2048),
                max_want_ids: 512,
            };
            let count = rand(24);
            let events: Vec<Vec<u8>> = (0..count)
                .map(|_| vec![0u8; 1 + rand(200)])
                .filter(|e| 4 + 4 + e.len() <= limits.max_batch_bytes)
                .collect();
            let (mut sender, mut receiver) = pair();
            send_batches(&mut sender, &events, &limits).await.unwrap();
            drop(sender);
            let mut received: Vec<Vec<u8>> = Vec::new();
            while let Some(frame) = receiver.recv().await.unwrap() {
                let decoded = decode_batch(&frame[1..], &limits).unwrap();
                assert!(decoded.len() <= limits.max_batch_events);
                received.extend(decoded);
            }
            assert_eq!(received, events);
        }
    }

    #[test]
    fn auth_transcript_and_body_hash_pins() {
        // The empty body hash is the BLAKE3 value the cross-language
        // fixtures pin for the hosted implementation.
        assert_eq!(
            body_hash_hex(&[]),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
        assert_eq!(
            auth_transcript("bind", "00ff", "ws", "af13"),
            b"id.bridge-auth.v1\nbind\n00ff\nws\naf13".to_vec()
        );
    }

    // ----- sessions ---------------------------------------------------

    #[tokio::test]
    async fn session_converges_both_directions_and_resumes() {
        let (_dir_a, mut client_store) = open_store();
        let (_dir_b, server_store) = open_store();
        let a_events = make_events(9, 3);
        let b_events = make_events(8, 3);
        client_store.bridge_ingest(&a_events, NOW_MS).unwrap();
        let mut server = TestServer::new(server_store, dalek_key(9).verifying_key(), false);
        server.store.bridge_ingest(&b_events, NOW_MS).unwrap();
        let saw_resume = server.saw_resume.clone();

        let (mut client_transport, server_transport) = pair();
        let handle = tokio::spawn(server.run(server_transport));
        let mut dialer = BridgeDialer::new(BINDING, dalek_key(9));
        let result = dialer
            .sync(&mut client_store, &mut client_transport, NOW_MS)
            .await
            .unwrap();
        assert!(
            result.withholding.is_empty(),
            "withholding: {:?}",
            result.withholding
        );
        assert!(result.token.is_some());
        // The want order is the range exchange's learning order, not
        // chain order: an event whose backlink is still in flight
        // lands as Waiting and is released by its predecessor, so
        // the batch tally varies. What the session guarantees is
        // that every learned event arrived, none was refused, and
        // the store converged.
        assert_eq!(result.acks.iter().map(|a| a.received).sum::<u64>(), 3);
        assert_eq!(result.acks.iter().map(|a| a.refused).sum::<u64>(), 0);
        assert_eq!(client_store.bridge_sync_entries().unwrap().len(), 6);
        let b_key = PandaSigningKey::from_bytes(&[8u8; 32]);
        assert_eq!(
            client_store
                .bridge_chain_next_seq(b_key.verifying_key().as_bytes())
                .unwrap(),
            3
        );
        assert_eq!(dialer.resume_token(), result.token.as_deref());

        // The server side converged too, and a second session
        // resumes on the token instead of opening with HELLO.
        let server = handle.await.unwrap().unwrap();
        assert_eq!(server.store.bridge_sync_entries().unwrap().len(), 6);
        let (mut client_transport, server_transport) = pair();
        let handle = tokio::spawn(server.run(server_transport));
        let result = dialer
            .sync(&mut client_store, &mut client_transport, NOW_MS)
            .await
            .unwrap();
        assert!(saw_resume.load(Ordering::SeqCst));
        assert!(result.withholding.is_empty());
        assert!(result.acks.is_empty());
        let _server = handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn session_detects_withholding() {
        let (_dir_a, mut client_store) = open_store();
        let (_dir_b, server_store) = open_store();
        let b_events = make_events(8, 3);
        let b_head = id_store::chain::decode(&b_events[2]).unwrap();
        let mut server = TestServer::new(server_store, dalek_key(9).verifying_key(), true);
        server.store.bridge_ingest(&b_events, NOW_MS).unwrap();

        let (mut client_transport, server_transport) = pair();
        let handle = tokio::spawn(server.run(server_transport));
        let mut dialer = BridgeDialer::new(BINDING, dalek_key(9));
        let result = dialer
            .sync(&mut client_store, &mut client_transport, NOW_MS)
            .await
            .unwrap();
        assert_eq!(client_store.bridge_sync_entries().unwrap().len(), 0);
        assert_eq!(result.withholding.len(), 1);
        let record = &result.withholding[0];
        assert_eq!(record.author, author_hex(8));
        assert_eq!(record.announced_next_seq, 3);
        assert_eq!(record.actual_next_seq, 0);
        assert_eq!(record.undelivered, vec![hex_encode(b_head.id.as_bytes())]);
        let _server = handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn session_refuses_a_wrong_key() {
        let (_dir_a, mut client_store) = open_store();
        let (_dir_b, server_store) = open_store();
        let server = TestServer::new(server_store, dalek_key(9).verifying_key(), false);
        let (mut client_transport, server_transport) = pair();
        let handle = tokio::spawn(server.run(server_transport));
        let mut dialer = BridgeDialer::new(BINDING, dalek_key(7));
        let err = dialer
            .sync(&mut client_store, &mut client_transport, NOW_MS)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("auth_failed"), "error: {err}");
        let _server = handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn rejected_resume_falls_back_to_hello() {
        let (_dir_a, mut client_store) = open_store();
        let b_events = make_events(8, 3);
        let saw_hello = Arc::new(AtomicBool::new(false));
        let saw = saw_hello.clone();
        let mut keepalive = Vec::new();
        let mut make_transport = move || {
            let (client_transport, server_transport) = pair();
            let (dir, mut store) = open_store();
            store.bridge_ingest(&b_events, NOW_MS).unwrap();
            let mut server = TestServer::new(store, dalek_key(9).verifying_key(), false);
            server.saw_hello = saw.clone();
            keepalive.push(dir);
            tokio::spawn(server.run(server_transport));
            std::future::ready(Ok(client_transport))
        };
        let mut dialer = BridgeDialer::new(BINDING, dalek_key(9));
        dialer.set_resume_token(Some("stale-token".to_owned()));
        let result = dialer
            .sync_with_retry(&mut client_store, &mut make_transport, 2, 1, NOW_MS)
            .await
            .unwrap();
        assert!(saw_hello.load(Ordering::SeqCst));
        assert!(result.withholding.is_empty());
        assert_eq!(client_store.bridge_sync_entries().unwrap().len(), 3);
        assert!(dialer.resume_token().is_some());
    }
}
