//! The native one-shot HTTPS legs of the hosted signed-object
//! bridge (#3818): `GET /v1/bridge/challenge`,
//! `GET /v1/bridge/heads`, `POST /v1/bridge/push`,
//! `POST /v1/bridge/pull`.
//!
//! The WebSocket session ([`crate::bridge`]) is the bridge's sync
//! protocol; these are its one-shot counterparts, the routes the
//! hosted worker's `user-do.ts` serves beside it. Each leg is a
//! single call: `heads` reads the hosted store's chain heads,
//! `push` ingests one bounded batch and reports its outcomes,
//! `pull` reads the sync set beyond the caller's per-author
//! positions. A caller that only needs to publish a few events, or
//! to catch up without holding a session open, uses these; the
//! session remains the way to reconcile two live stores.
//!
//! Every leg but `challenge` carries the ID-key proof of the
//! hosted `bridge-auth.ts`: the caller first fetches a fresh
//! single-use nonce from `challenge`, then signs the bridge-auth
//! transcript — binding, nonce, leg, BLAKE3 of the exact body —
//! and sends the nonce and signature as the `x-id-nonce` and
//! `x-id-proof` headers. The nonce is consumed on first use,
//! success or failure, so a proof authorizes exactly one call;
//! this client fetches a fresh one per leg, immediately before
//! signing. The account layer that routes a call to the caller's
//! binding (the front door) is the caller's concern, exactly as
//! with the session's [`crate::bridge::connect`]: a
//! [`BridgeHttp`] implementation may attach whatever headers that
//! layer needs.
//!
//! [`BridgeHttp`] is the transport seam: one request, one
//! response, whole bodies. Production speaks through
//! [`ReqwestHttp`]; tests speak to anything that answers.

use std::future::Future;

use anyhow::{Context, Result, bail, ensure};
use ed25519_dalek::{Signer as _, SigningKey};
use serde::{Deserialize, Serialize};

use crate::bridge::{
    BridgeLimits, Head, auth_transcript, body_hash_hex, decode_batch, encode_batch,
};
use crate::hex::hex_encode;

/// One HTTP request: a method, a full URL, headers, and a whole
/// body. The bridge's calls are small and bounded, so nothing
/// streams.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpRequest {
    /// The HTTP method (`GET` or `POST` for the bridge legs, `PUT`
    /// for a pkarr relay publish).
    pub method: String,
    /// The full URL the request goes to.
    pub url: String,
    /// The request headers, in send order.
    pub headers: Vec<(String, String)>,
    /// The exact request body bytes (the proof signs these).
    pub body: Vec<u8>,
}

/// One HTTP response: a status, headers, and a whole body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpResponse {
    /// The HTTP status code.
    pub status: u16,
    /// The response headers, as received.
    pub headers: Vec<(String, String)>,
    /// The exact response body bytes.
    pub body: Vec<u8>,
}

impl HttpResponse {
    /// One header's value, matched case-insensitively, if sent.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// The bridge's HTTP transport seam: carries one whole request
/// and returns the whole response. A transport failure (no route,
/// a reset connection) is an error; an HTTP refusal is a response.
pub trait BridgeHttp {
    /// Carries one request to its response.
    fn request(
        &mut self,
        request: HttpRequest,
    ) -> impl Future<Output = Result<HttpResponse>> + Send;
}

/// The production [`BridgeHttp`]: reqwest over rustls, the same
/// stack the workspace's other HTTPS client (id-worlds) uses.
#[derive(Debug)]
pub struct ReqwestHttp {
    client: reqwest::Client,
}

impl ReqwestHttp {
    /// A transport on a default reqwest client.
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .build()
            .context("bridge http client")?;
        Ok(Self { client })
    }
}

impl BridgeHttp for ReqwestHttp {
    async fn request(&mut self, request: HttpRequest) -> Result<HttpResponse> {
        let mut builder = match request.method.as_str() {
            "GET" => self.client.get(&request.url),
            "POST" => self.client.post(&request.url),
            // The pkarr relay sinks publish with PUT (#10706).
            "PUT" => self.client.put(&request.url),
            other => bail!("bridge http method {other} is not one the legs use"),
        };
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        let response = builder
            .body(request.body)
            .send()
            .await
            .context("bridge http request")?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_owned(),
                    value.to_str().unwrap_or_default().to_owned(),
                )
            })
            .collect();
        let body = response
            .bytes()
            .await
            .context("bridge http response body")?
            .to_vec();
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }
}

/// A refusal the hosted side named: the leg's HTTP status and the
/// `{error, message}` body every bridge route fails closed with.
#[derive(Debug, thiserror::Error)]
pub enum BridgeHttpError {
    /// The server answered with a non-success status.
    #[error("bridge call refused with HTTP {status} ({code}): {message}")]
    Refused {
        /// The HTTP status code.
        status: u16,
        /// The failure's code (e.g. `bridge_proof_invalid`).
        code: String,
        /// The failure's human-readable message.
        message: String,
    },
}

#[derive(Deserialize)]
struct HttpErrorBody {
    error: String,
    message: String,
}

/// Returns the response when it succeeded, or its named refusal.
fn ensure_success(response: &HttpResponse) -> Result<()> {
    if (200..300).contains(&response.status) {
        return Ok(());
    }
    let (code, message) = match serde_json::from_slice::<HttpErrorBody>(&response.body) {
        Ok(body) => (body.error, body.message),
        Err(_) => (
            "unparseable_error".to_owned(),
            String::from_utf8_lossy(&response.body).into_owned(),
        ),
    };
    Err(BridgeHttpError::Refused {
        status: response.status,
        code,
        message,
    }
    .into())
}

/// A minted challenge nonce and its expiry (the server's ISO
/// timestamp, kept as sent — a caller uses the nonce immediately
/// and never needs to read the expiry back).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Challenge {
    /// The single-use nonce, hex.
    pub nonce: String,
    /// When the nonce stops verifying, as an ISO timestamp.
    pub expires_at: String,
}

/// One author's pull position: the sequence number its next
/// wanted event takes (zero pulls from the start of the chain).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullPosition {
    /// The author's key, hex.
    pub author: String,
    /// The sequence number the author's next event takes.
    pub next_seq: u64,
}

impl From<&Head> for PullPosition {
    fn from(head: &Head) -> Self {
        Self {
            author: head.author.clone(),
            next_seq: head.next_seq,
        }
    }
}

/// One pushed event's outcome, as the push leg reports it — the
/// hosted store's ingest record, JSON-shaped (`serializeOutcome`
/// in `user-do.ts`).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PushOutcome {
    /// Stored and applied; `released` lists the waiting events
    /// that became applied because of it, in application order.
    Accepted {
        /// The event's ID, hex.
        id: String,
        /// The released waiting events' IDs, hex.
        released: Vec<String>,
    },
    /// Already held; nothing changed.
    Duplicate {
        /// The event's ID, hex.
        id: String,
    },
    /// The author signed two different events at one sequence
    /// number; the chain is frozen there.
    Fork {
        /// The author's key, hex.
        author: String,
        /// The fork point's sequence number.
        seq: u64,
    },
    /// Held for a dependency; not stored.
    Waiting {
        /// The event's ID, hex.
        id: String,
    },
    /// Refused; nothing changed.
    Refused {
        /// The refusal's reason.
        reason: String,
        /// The refusal's human-readable message.
        message: String,
    },
}

/// What one push did: every event's outcome, in batch order, and
/// the hosted store's heads after the batch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushResult {
    /// The batch's outcomes, in the order the events were sent.
    pub outcomes: Vec<PushOutcome>,
    /// The hosted store's chain heads after the ingest.
    pub heads: Vec<Head>,
}

/// What one pull read: one bounded batch of the sync set beyond
/// the caller's positions, whether more remains, and the hosted
/// store's heads as the batch was cut.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullResult {
    /// The events, in (author, sequence) order.
    pub events: Vec<Vec<u8>>,
    /// More of the sync set remains beyond this batch.
    pub more: bool,
    /// The hosted store's chain heads.
    pub heads: Vec<Head>,
}

#[derive(Deserialize)]
struct HeadsBody {
    heads: Vec<Head>,
}

#[derive(Deserialize)]
struct PushBody {
    outcomes: Vec<PushOutcome>,
    heads: Vec<Head>,
}

#[derive(Serialize)]
struct PullBody<'a> {
    positions: &'a [PullPosition],
    #[serde(skip_serializing_if = "Option::is_none")]
    max_events: Option<u32>,
}

/// The native client for the bridge's one-shot HTTPS legs: one
/// binding, one ID key, one hosted store's front-door base URL.
#[derive(Clone)]
pub struct BridgeHttpClient {
    binding_id: String,
    key: SigningKey,
    base: String,
    limits: BridgeLimits,
}

impl std::fmt::Debug for BridgeHttpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The key never prints.
        f.debug_struct("BridgeHttpClient")
            .field("binding_id", &self.binding_id)
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

impl BridgeHttpClient {
    /// A client for `binding_id` at the front door's `base_url`
    /// (its origin, e.g. `https://host.example`), proving with
    /// `key` — the ID key the binding record names.
    pub fn new(
        binding_id: impl Into<String>,
        key: SigningKey,
        base_url: impl Into<String>,
    ) -> Self {
        let base = base_url.into();
        Self {
            binding_id: binding_id.into(),
            key,
            base: base.trim_end_matches('/').to_owned(),
            limits: BridgeLimits::default(),
        }
    }

    /// Sets the receipt bounds (the push leg also enforces them
    /// on the send side, so an over-bound batch never leaves).
    #[must_use]
    pub fn with_limits(mut self, limits: BridgeLimits) -> Self {
        self.limits = limits;
        self
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    /// The challenge leg: mints the single-use nonce the next
    /// leg's proof signs over. Minting replaces the binding's
    /// previous live nonce, and a nonce is consumed by the first
    /// leg that presents it — the legs below each fetch their own,
    /// immediately before signing.
    pub async fn challenge<H: BridgeHttp>(&self, http: &mut H) -> Result<Challenge> {
        let response = http
            .request(HttpRequest {
                method: "GET".to_owned(),
                url: self.url("/v1/bridge/challenge"),
                headers: Vec::new(),
                body: Vec::new(),
            })
            .await?;
        ensure_success(&response)?;
        serde_json::from_slice(&response.body).context("the bridge challenge is not its JSON")
    }

    /// The proof headers for one leg over `body`: a fresh nonce,
    /// and the ID key's signature over the bridge-auth transcript
    /// naming this leg.
    async fn proof_headers<H: BridgeHttp>(
        &self,
        http: &mut H,
        context: &str,
        body: &[u8],
    ) -> Result<Vec<(String, String)>> {
        let challenge = self.challenge(http).await?;
        let transcript = auth_transcript(
            &self.binding_id,
            &challenge.nonce,
            context,
            &body_hash_hex(body),
        );
        let signature = self.key.sign(&transcript);
        Ok(vec![
            ("x-id-nonce".to_owned(), challenge.nonce),
            ("x-id-proof".to_owned(), hex_encode(&signature.to_bytes())),
        ])
    }

    /// The heads leg: the hosted store's chain heads, proven over
    /// an empty body.
    pub async fn heads<H: BridgeHttp>(&self, http: &mut H) -> Result<Vec<Head>> {
        let headers = self.proof_headers(http, "heads", &[]).await?;
        let response = http
            .request(HttpRequest {
                method: "GET".to_owned(),
                url: self.url("/v1/bridge/heads"),
                headers,
                body: Vec::new(),
            })
            .await?;
        ensure_success(&response)?;
        let body: HeadsBody = serde_json::from_slice(&response.body)
            .context("the bridge heads are not their JSON")?;
        Ok(body.heads)
    }

    /// The push leg: ingests one batch of events into the hosted
    /// store and reports every event's outcome. The batch is
    /// bound-checked here first, in the hosted codec's terms, so
    /// an over-bound batch is a local error, never a sent call.
    pub async fn push<H: BridgeHttp>(
        &self,
        http: &mut H,
        events: &[Vec<u8>],
    ) -> Result<PushResult> {
        ensure!(
            events.len() <= self.limits.max_batch_events,
            "batch of {} events is over the {}-event bound",
            events.len(),
            self.limits.max_batch_events
        );
        for event in events {
            ensure!(
                event.len() <= self.limits.max_event_bytes,
                "event of {} bytes is over the {}-byte bound",
                event.len(),
                self.limits.max_event_bytes
            );
        }
        let body = encode_batch(events);
        ensure!(
            body.len() <= self.limits.max_batch_bytes,
            "batch of {} bytes is over the {}-byte bound",
            body.len(),
            self.limits.max_batch_bytes
        );
        let headers = self.proof_headers(http, "push", &body).await?;
        let response = http
            .request(HttpRequest {
                method: "POST".to_owned(),
                url: self.url("/v1/bridge/push"),
                headers,
                body,
            })
            .await?;
        ensure_success(&response)?;
        let body: PushBody = serde_json::from_slice(&response.body)
            .context("the bridge push answer is not its JSON")?;
        Ok(PushResult {
            outcomes: body.outcomes,
            heads: body.heads,
        })
    }

    /// The pull leg: reads one bounded batch of the hosted
    /// store's sync set beyond `positions` (an author with no
    /// position is pulled from zero). `max_events` narrows the
    /// batch below the receipt bound when given; the answer's
    /// `more` says whether another pull would read further.
    pub async fn pull<H: BridgeHttp>(
        &self,
        http: &mut H,
        positions: &[PullPosition],
        max_events: Option<u32>,
    ) -> Result<PullResult> {
        let body = serde_json::to_vec(&PullBody {
            positions,
            max_events,
        })
        .context("the bridge pull body does not serialize")?;
        let headers = self.proof_headers(http, "pull", &body).await?;
        let response = http
            .request(HttpRequest {
                method: "POST".to_owned(),
                url: self.url("/v1/bridge/pull"),
                headers,
                body,
            })
            .await?;
        ensure_success(&response)?;
        let heads: Vec<Head> = match response.header("x-bridge-heads") {
            Some(raw) => {
                serde_json::from_str(raw).context("the bridge pull heads header is not its JSON")?
            }
            None => bail!("the bridge pull answer carries no heads header"),
        };
        let more = response.header("x-bridge-more") == Some("1");
        let events = decode_batch(&response.body, &self.limits)?;
        Ok(PullResult {
            events,
            more,
            heads,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    use ed25519_dalek::{Signature as DalekSignature, Verifier as _, VerifyingKey};
    use p2panda_core::SigningKey as PandaSigningKey;

    use id_core::waiting::WaitingLimits;
    use id_events::event_store::EventStore;
    use id_store::chain::{ChainExt, sign};
    use id_store::event_types::{Ingest, Refusal};

    use crate::bridge::BridgeStore as _;
    use crate::hex::hex_to_array;

    const BINDING: &str = "binding-1";
    const BASE: &str = "http://mirror";
    const NOW_MS: u64 = 0;
    const NONCE_TTL_MS: u64 = 5 * 60 * 1000;

    // ----- fixtures (the bridge.rs fixtures' shape) --------------

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
            thread: None,
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

    fn client(seed: u8) -> BridgeHttpClient {
        BridgeHttpClient::new(BINDING, dalek_key(seed), BASE)
    }

    // ----- the mirror: the hosted routes, in memory ---------------
    //
    // The user-do bridge routes transcribed against a native
    // EventStore: the challenge mints one live nonce, every other
    // leg verifies and consumes it before acting, push ingests
    // through the same store seam the dialer uses, pull cuts the
    // sync set at the caller's positions. Outcomes serialize
    // exactly as `serializeOutcome` shapes them.

    struct MirrorDo {
        store: EventStore,
        limits: BridgeLimits,
        bound_key: VerifyingKey,
        nonce: Option<(String, u64)>,
        nonce_counter: u64,
        seen: Vec<String>,
        _dir: tempfile::TempDir,
    }

    impl MirrorDo {
        fn new(bound_seed: u8) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let store = EventStore::open(dir.path(), WaitingLimits::default(), NOW_MS).unwrap();
            Self {
                store,
                limits: BridgeLimits::default(),
                bound_key: dalek_key(bound_seed).verifying_key(),
                nonce: None,
                nonce_counter: 0,
                seen: Vec::new(),
                _dir: dir,
            }
        }

        fn json(status: u16, value: serde_json::Value) -> HttpResponse {
            HttpResponse {
                status,
                headers: vec![("content-type".to_owned(), "application/json".to_owned())],
                body: serde_json::to_vec(&value).unwrap(),
            }
        }

        fn refusal(status: u16, code: &str, message: &str) -> HttpResponse {
            Self::json(
                status,
                serde_json::json!({ "error": code, "message": message }),
            )
        }

        /// The bridgeProof gate: the nonce is consumed either way;
        /// only a fresh nonce and a signature by the bound key over
        /// the exact transcript for this leg and body pass.
        fn verify(
            &mut self,
            request: &HttpRequest,
            context: &str,
            body: &[u8],
        ) -> Result<(), HttpResponse> {
            let live = self.nonce.take();
            let nonce = request
                .headers
                .iter()
                .find(|(name, _)| name == "x-id-nonce")
                .map(|(_, value)| value.clone());
            let proof = request
                .headers
                .iter()
                .find(|(name, _)| name == "x-id-proof")
                .map(|(_, value)| value.clone());
            let (Some(nonce), Some(proof)) = (nonce, proof) else {
                return Err(Self::refusal(
                    401,
                    "bridge_proof_required",
                    "bridge calls carry an x-id-nonce and x-id-proof header",
                ));
            };
            let verified = live.is_some_and(|(minted, expires_at)| {
                minted == nonce
                    && NOW_MS <= expires_at
                    && hex_to_array::<64>(&proof).is_some_and(|bytes| {
                        let signature = DalekSignature::from_bytes(&bytes);
                        let transcript =
                            auth_transcript(BINDING, &nonce, context, &body_hash_hex(body));
                        self.bound_key.verify(&transcript, &signature).is_ok()
                    })
            });
            if verified {
                Ok(())
            } else {
                Err(Self::refusal(
                    403,
                    "bridge_proof_invalid",
                    "the ID-key proof did not verify",
                ))
            }
        }

        fn heads_json(&self) -> serde_json::Value {
            serde_json::to_value(self.store.bridge_heads().unwrap()).unwrap()
        }
    }

    /// One pushed event's outcome in the push leg's JSON shape.
    fn outcome_json(outcome: &Ingest) -> serde_json::Value {
        match outcome {
            Ingest::Accepted { id, released } | Ingest::Voided { id, released } => {
                serde_json::json!({
                    "kind": "accepted",
                    "id": hex_encode(id.as_bytes()),
                    "released": released.iter().map(|id| hex_encode(id.as_bytes())).collect::<Vec<_>>(),
                })
            }
            Ingest::Duplicate { id } => {
                serde_json::json!({ "kind": "duplicate", "id": hex_encode(id.as_bytes()) })
            }
            Ingest::Fork { author, seq } => {
                serde_json::json!({
                    "kind": "fork",
                    "author": hex_encode(author),
                    "seq": u64::from(*seq),
                })
            }
            Ingest::Waiting { id } => {
                serde_json::json!({ "kind": "waiting", "id": hex_encode(id.as_bytes()) })
            }
            Ingest::Refused(refusal) => {
                let (reason, message) = match refusal {
                    Refusal::Malformed(message) => ("malformed", message.clone()),
                    Refusal::Frozen => (
                        "frozen",
                        "the event is above its author's fork point".to_owned(),
                    ),
                    Refusal::WaitingFull => (
                        "waiting_full",
                        "the author holds as many waiting events as the caps allow".to_owned(),
                    ),
                };
                serde_json::json!({ "kind": "refused", "reason": reason, "message": message })
            }
        }
    }

    impl BridgeHttp for MirrorDo {
        async fn request(&mut self, request: HttpRequest) -> Result<HttpResponse> {
            let path = request
                .url
                .strip_prefix(BASE)
                .unwrap_or_default()
                .to_owned();
            self.seen.push(format!("{} {path}", request.method));
            let limits = self.limits;
            match (request.method.as_str(), path.as_str()) {
                ("GET", "/v1/bridge/challenge") => {
                    self.nonce_counter += 1;
                    let nonce = format!("{:064x}", self.nonce_counter);
                    self.nonce = Some((nonce.clone(), NOW_MS + NONCE_TTL_MS));
                    Ok(Self::json(
                        200,
                        serde_json::json!({ "nonce": nonce, "expires_at": "1970-01-01T00:05:00.000Z" }),
                    ))
                }
                ("GET", "/v1/bridge/heads") => {
                    if let Err(refusal) = self.verify(&request, "heads", &[]) {
                        return Ok(refusal);
                    }
                    Ok(Self::json(
                        200,
                        serde_json::json!({ "heads": self.heads_json() }),
                    ))
                }
                ("POST", "/v1/bridge/push") => {
                    if request.body.len() > limits.max_batch_bytes {
                        return Ok(Self::refusal(
                            413,
                            "batch_too_large",
                            "push body is too large",
                        ));
                    }
                    if let Err(refusal) = self.verify(&request, "push", &request.body) {
                        return Ok(refusal);
                    }
                    let events = match decode_batch(&request.body, &limits) {
                        Ok(events) => events,
                        Err(err) => {
                            return Ok(Self::refusal(400, "invalid_batch", &format!("{err}")));
                        }
                    };
                    let outcomes = self.store.bridge_ingest(&events, NOW_MS).unwrap();
                    Ok(Self::json(
                        200,
                        serde_json::json!({
                            "outcomes": outcomes.iter().map(outcome_json).collect::<Vec<_>>(),
                            "heads": self.heads_json(),
                        }),
                    ))
                }
                ("POST", "/v1/bridge/pull") => {
                    if let Err(refusal) = self.verify(&request, "pull", &request.body) {
                        return Ok(refusal);
                    }
                    #[derive(Deserialize)]
                    struct PullIn {
                        #[serde(default)]
                        positions: Vec<PullPosition>,
                        max_events: Option<u32>,
                    }
                    let parsed: PullIn = match serde_json::from_slice(&request.body) {
                        Ok(parsed) => parsed,
                        Err(_) => {
                            return Ok(Self::refusal(
                                400,
                                "invalid_pull",
                                "pull body must be JSON positions",
                            ));
                        }
                    };
                    let max_events = parsed
                        .max_events
                        .map_or(limits.max_batch_events, |max| (max as usize).clamp(1, 512));
                    let mut batch: Vec<Vec<u8>> = Vec::new();
                    // The encoded size so far: the 4-byte count
                    // prefix, then each event's 4-byte length
                    // prefix and payload (#14644).
                    let mut batch_bytes = 4usize;
                    let mut more = false;
                    // The sync set is key-ordered: author, then
                    // sequence — the hosted pull's sort.
                    for (key, id) in self.store.bridge_sync_entries().unwrap() {
                        let author = hex_encode(&key[0..32]);
                        let seq = u64::from_be_bytes(key[32..40].try_into().unwrap());
                        let wanted_from = parsed
                            .positions
                            .iter()
                            .find(|position| position.author == author)
                            .map_or(0, |position| position.next_seq);
                        if seq < wanted_from {
                            continue;
                        }
                        let Some(bytes) = self.store.bridge_event_bytes(&id).unwrap() else {
                            continue;
                        };
                        if batch.len() >= max_events
                            || batch_bytes + 4 + bytes.len() > limits.max_batch_bytes
                        {
                            if batch.is_empty() {
                                // The first event alone cannot fit
                                // any batch under the bound: refuse
                                // by name rather than answer with an
                                // empty batch the caller would retry
                                // forever (#14644).
                                return Ok(Self::refusal(
                                    413,
                                    "event_too_large",
                                    &format!(
                                        "event of {} bytes cannot fit the {}-byte batch bound",
                                        bytes.len(),
                                        limits.max_batch_bytes
                                    ),
                                ));
                            }
                            more = true;
                            break;
                        }
                        batch_bytes += 4 + bytes.len();
                        batch.push(bytes);
                    }
                    Ok(HttpResponse {
                        status: 200,
                        headers: vec![
                            (
                                "x-bridge-more".to_owned(),
                                if more { "1" } else { "0" }.to_owned(),
                            ),
                            (
                                "x-bridge-heads".to_owned(),
                                serde_json::to_string(&self.store.bridge_heads().unwrap()).unwrap(),
                            ),
                        ],
                        body: encode_batch(&batch),
                    })
                }
                _ => Ok(Self::refusal(404, "not_found", "no such bridge route")),
            }
        }
    }

    // ----- the legs -------------------------------------------------

    #[tokio::test]
    async fn heads_on_an_empty_store() {
        let mut mirror = MirrorDo::new(1);
        let heads = client(1).heads(&mut mirror).await.unwrap();
        assert!(heads.is_empty());
        assert_eq!(
            mirror.seen,
            ["GET /v1/bridge/challenge", "GET /v1/bridge/heads"]
        );
    }

    #[tokio::test]
    async fn a_wrong_key_proof_is_refused_by_name() {
        let mut mirror = MirrorDo::new(1);
        let err = client(2).heads(&mut mirror).await.unwrap_err();
        let refused = err.downcast_ref::<BridgeHttpError>().unwrap();
        assert!(matches!(
            refused,
            BridgeHttpError::Refused { status: 403, code, .. } if code == "bridge_proof_invalid"
        ));
    }

    #[tokio::test]
    async fn a_nonce_proves_exactly_one_call() {
        let mut mirror = MirrorDo::new(1);
        let client = client(1);
        let challenge = client.challenge(&mut mirror).await.unwrap();
        let transcript = auth_transcript(BINDING, &challenge.nonce, "heads", &body_hash_hex(&[]));
        let signature = client.key.sign(&transcript);
        let request = HttpRequest {
            method: "GET".to_owned(),
            url: format!("{BASE}/v1/bridge/heads"),
            headers: vec![
                ("x-id-nonce".to_owned(), challenge.nonce.clone()),
                ("x-id-proof".to_owned(), hex_encode(&signature.to_bytes())),
            ],
            body: Vec::new(),
        };
        let first = mirror.request(request.clone()).await.unwrap();
        assert_eq!(first.status, 200);
        // The nonce was consumed by the first call, success or
        // failure — the identical proof cannot be replayed.
        let replay = mirror.request(request).await.unwrap();
        assert_eq!(replay.status, 403);
    }

    #[tokio::test]
    async fn push_heads_and_pull_round_trip() {
        let mut mirror = MirrorDo::new(1);
        let client = client(1);
        let events = make_events(7, 3);

        let pushed = client.push(&mut mirror, &events).await.unwrap();
        assert_eq!(pushed.outcomes.len(), 3);
        assert!(
            pushed
                .outcomes
                .iter()
                .all(|outcome| matches!(outcome, PushOutcome::Accepted { released, .. } if released.is_empty()))
        );
        assert_eq!(
            pushed.heads,
            vec![Head {
                author: author_hex(7),
                next_seq: 3,
                head: pushed.heads[0].head.clone(),
            }]
        );

        // A re-push is all duplicates: the sender retries exactly
        // what was refused, and nothing here was.
        let repushed = client.push(&mut mirror, &events).await.unwrap();
        assert!(
            repushed
                .outcomes
                .iter()
                .all(|outcome| matches!(outcome, PushOutcome::Duplicate { .. }))
        );

        let heads = client.heads(&mut mirror).await.unwrap();
        assert_eq!(heads, pushed.heads);

        let pulled = client.pull(&mut mirror, &[], None).await.unwrap();
        assert_eq!(pulled.events, events);
        assert!(!pulled.more);
        assert_eq!(pulled.heads, heads);

        // A caller already at the head pulls nothing.
        let positions: Vec<PullPosition> = heads.iter().map(PullPosition::from).collect();
        let caught_up = client.pull(&mut mirror, &positions, None).await.unwrap();
        assert!(caught_up.events.is_empty());
        assert!(!caught_up.more);
    }

    #[tokio::test]
    async fn pull_paginates_at_max_events() {
        let mut mirror = MirrorDo::new(1);
        let client = client(1);
        let events = make_events(7, 3);
        client.push(&mut mirror, &events).await.unwrap();

        let mut position = PullPosition {
            author: author_hex(7),
            next_seq: 0,
        };
        let mut collected: Vec<Vec<u8>> = Vec::new();
        loop {
            let page = client
                .pull(&mut mirror, std::slice::from_ref(&position), Some(1))
                .await
                .unwrap();
            assert!(page.events.len() <= 1);
            collected.extend(page.events);
            position.next_seq += 1;
            if !page.more {
                break;
            }
        }
        assert_eq!(collected, events);
        assert_eq!(position.next_seq, 3);
    }

    #[tokio::test]
    async fn pull_paginates_at_the_encoded_byte_bound() {
        // The mirror's pull budgets the encoded batch (4-byte count
        // + 4-byte length per event), not the payload sum (#14644):
        // with room for exactly one framed event per page, three
        // events arrive one per pull, in order.
        let mut mirror = MirrorDo::new(1);
        let events = make_events(7, 3);
        // Room for exactly one framed event: the count prefix plus
        // the longest event's length prefix and payload (seq 0 is
        // shorter — it carries no backlink).
        let longest = events.iter().map(|e| e.len()).max().unwrap();
        mirror.limits = BridgeLimits {
            max_batch_bytes: 4 + 4 + longest,
            ..BridgeLimits::default()
        };
        let client = client(1).with_limits(mirror.limits);
        for event in &events {
            client
                .push(&mut mirror, std::slice::from_ref(event))
                .await
                .unwrap();
        }
        let mut position = PullPosition {
            author: author_hex(7),
            next_seq: 0,
        };
        let mut collected: Vec<Vec<u8>> = Vec::new();
        loop {
            let page = client
                .pull(&mut mirror, std::slice::from_ref(&position), None)
                .await
                .unwrap();
            assert_eq!(page.events.len(), 1);
            collected.extend(page.events);
            position.next_seq += 1;
            if !page.more {
                break;
            }
        }
        assert_eq!(collected, events);
    }

    #[tokio::test]
    async fn pull_refuses_an_event_that_cannot_fit_any_batch() {
        let mut mirror = MirrorDo::new(1);
        let events = make_events(7, 1);
        client(1).push(&mut mirror, &events).await.unwrap();
        // One byte short of a single framed event.
        mirror.limits = BridgeLimits {
            max_batch_bytes: 4 + 4 + events[0].len() - 1,
            ..BridgeLimits::default()
        };
        let tight = client(1).with_limits(mirror.limits);
        let position = PullPosition {
            author: author_hex(7),
            next_seq: 0,
        };
        let err = tight
            .pull(&mut mirror, &[position], None)
            .await
            .unwrap_err();
        assert!(format!("{err:#}").contains("event_too_large"), "{err:#}");
    }

    #[tokio::test]
    async fn a_malformed_event_is_a_refused_outcome() {
        let mut mirror = MirrorDo::new(1);
        let pushed = client(1).push(&mut mirror, &[vec![1, 2, 3]]).await.unwrap();
        assert!(matches!(
            pushed.outcomes.as_slice(),
            [PushOutcome::Refused { .. }]
        ));
    }

    #[tokio::test]
    async fn an_over_bound_batch_never_leaves_the_client() {
        let mut mirror = MirrorDo::new(1);
        let oversize = vec![0u8; BridgeLimits::default().max_event_bytes + 1];
        let err = client(1).push(&mut mirror, &[oversize]).await.unwrap_err();
        assert!(format!("{err}").contains("over the"));
        // Not even the challenge was fetched: the bound is local.
        assert!(mirror.seen.is_empty());
    }
}
