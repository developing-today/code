//! The host-owned client-provider broker (F13.7 step 3, design
//! `doc/2026-11-01_design_id789.md` sections 4 and 7; issue #5532).
//!
//! A client-side provider (`side: client` in the F12.4 registry, names
//! under the `client.` namespace) executes on the client that
//! originated a session — the host holds no such resource and never
//! executes it. This broker is the host's policy and correlation
//! point at that seam:
//!
//! - **Policy order** (design section 4, flow step 2): known manifest
//!   -> server/world ceiling -> actor scope -> explicit world grant
//!   -> provider/session availability. An over-ceiling or otherwise
//!   refused request is refused before any client frame is sent.
//!   Client providers never use grant-on-use: the world grant must
//!   already exist, and even then it is only a host policy gate — the
//!   local user is the grant authority over the device, enforced by
//!   the shell adapter on every request (step 4), never by this host
//!   code.
//! - **Pending state**: an allowed request binds the actor,
//!   participant, ephemeral session ID (#840), guest request ID,
//!   a host-minted nonce, the effective result bound (manifest bound
//!   and the protocol event bound, composed by minimum), and a
//!   deadline. The `provider_request` frame is broadcast for session
//!   routing and is only ever *sent* by the session whose ID it is
//!   bound to (see `world_session`), so a request is never routed to
//!   an arbitrary session.
//! - **Replies**: exactly one reply is accepted, only from the bound
//!   session, only before the deadline, and its bytes are validated
//!   against the effective bound. Unknown, duplicate, wrong-session,
//!   expired, and oversized replies fail closed; a client refusal and
//!   a malformed reply complete the request with a stable error.
//!   Timeout and session disconnect complete the request the same
//!   way — never by falling back to another client or to host
//!   execution.
//! - **Audit**: every host allow/deny/completion is recorded in the
//!   world's retained F12.6 log ([`WorldAuditLog`]) as a
//!   `client_provider_use` record carrying the provider name and
//!   version, the world, the actor, the outcome, the audit
//!   correlation ID, and an arguments digest (BLAKE3 of the canonical
//!   args JSON) — never raw arguments, file content, or local paths.
//! - **Delivery**: an accepted result becomes a broker completion;
//!   the world actor drains completions and feeds the ordinary
//!   capability result event to the guest (the only part that is
//!   journaled, via the existing `System` event). Pending entries,
//!   nonces, and offers are ephemeral and never journaled. After a
//!   reopen, a still-listed request has no acting session and is
//!   refused as unattributed rather than routed (design flow step 6).
//!
//! The adapter seam is the shell session protocol (offer / request /
//! reply frames, step 2). The tests at the bottom drive the broker
//! through a fake adapter implementing that seam, covering refusal,
//! timeout, disconnect, replay, malformed, and wrong-session replies.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::{Notify, broadcast};

use super::{AuditRecord, SessionId, WorldAuditLog, emit_audit_record, hex_encode, unix_ms};
use crate::world_caps::{
    ArgumentSchema, ArgumentType, CapError, ProviderBound, ProviderManifest, ProviderSide,
};

/// Default deadline for one provider request, in milliseconds. The
/// shell sees this exact value in its `provider_request` frame.
pub const DEFAULT_PROVIDER_DEADLINE_MS: u64 = 30_000;

/// Default maximum provider requests in flight per provider name.
/// Carries forward the Oct 25 design's starting in-flight limit of 2
/// for `client.file.read`; the manifest bound set has no in-flight
/// kind yet, so the broker enforces it as a protocol constant.
pub const MAX_PROVIDER_IN_FLIGHT: usize = 2;

/// One `provider_request` frame's routing payload. Broadcast by the
/// broker; the session layer sends it only on the session it is
/// bound to and drops every other copy.
#[derive(Clone, Debug)]
pub(crate) struct BrokerRequestFrame {
    /// The only session allowed to receive (and answer) this request.
    pub(crate) session: SessionId,
    /// Host-minted correlation nonce, hex-encoded.
    pub(crate) nonce: String,
    /// Provider name, under the `client.` namespace.
    pub(crate) provider: String,
    /// Request arguments, per the provider's manifest schema.
    pub(crate) args: serde_json::Value,
    /// The broker's deadline for the reply, in milliseconds.
    pub(crate) deadline_ms: u64,
}

/// A finished provider request, ready for the world actor to deliver
/// through the existing capability result event.
#[derive(Debug)]
pub(crate) struct BrokerCompletion {
    /// The guest's `wants` request ID this completion answers.
    pub(crate) guest_request_id: String,
    /// The bounded result value, or the stable refusal.
    pub(crate) outcome: Result<serde_json::Value, CapError>,
}

/// What one reply did at the broker. Session-layer callers ignore
/// this (a bad reply is never a session error); tests assert on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReplyOutcome {
    /// The reply was accepted and completed the request.
    Accepted,
    /// The bound client refused; the request completed with its code.
    ClientRefused,
    /// No pending entry carries this nonce (unknown or already
    /// completed — a replayed reply lands here). Fail closed.
    IgnoredUnknownNonce,
    /// The reply came from a session the nonce is not bound to. The
    /// pending entry survives until its deadline or disconnect.
    IgnoredWrongSession,
    /// The reply was malformed; the request completed with an error.
    Malformed,
    /// The reply arrived after the deadline; the request completed
    /// with a timeout.
    Expired,
}

/// The policy snapshot one request is decided against. The actor
/// supplies the ledger-derived facts; the broker owns the ordering
/// and never re-reads the ledger, so a client provider can never be
/// reached through grant-on-use or a host executor.
pub(crate) struct BeginRequest<'a> {
    /// The registry manifest for the requested provider.
    pub(crate) manifest: &'a ProviderManifest,
    /// Whether the server/world ceiling intersection allows the name.
    pub(crate) ceiling_allows: bool,
    /// Whether the acting participant's scopes permit the effect.
    pub(crate) actor_permits: bool,
    /// Whether the world has an explicit grant for the provider.
    pub(crate) granted: bool,
    /// Audit actor label (`participant:<id>` or `program`).
    pub(crate) actor: String,
    /// The acting participant, when the request is attributed.
    pub(crate) participant: Option<u64>,
    /// The session whose input caused the request, when attributed.
    pub(crate) session: Option<SessionId>,
    /// The guest's `wants` request ID.
    pub(crate) guest_request_id: String,
    /// The guest's request arguments.
    pub(crate) args: &'a serde_json::Value,
}

/// One in-flight provider request. Ephemeral by design: it is never
/// journaled, and a restart drops every entry (a re-listed guest
/// request then has no acting session and is refused unattributed).
struct PendingEntry {
    nonce: String,
    provider: String,
    provider_version: u32,
    actor: String,
    #[allow(dead_code)]
    participant: Option<u64>,
    session: SessionId,
    guest_request_id: String,
    args_digest: String,
    max_result_bytes: usize,
    deadline: Instant,
}

#[derive(Default)]
struct BrokerInner {
    /// Providers each live session currently offers.
    offers: HashMap<SessionId, BTreeSet<String>>,
    /// Pending requests by nonce.
    pending: HashMap<String, PendingEntry>,
    /// Completions waiting for the world actor to deliver.
    completions: Vec<BrokerCompletion>,
}

/// The client-provider broker for one world. Cheap to clone; shared
/// between the world handle, the world actor, and the session layer
/// (the same sharing pattern as [`WorldAuditLog`]).
#[derive(Clone)]
pub(crate) struct ProviderBroker {
    inner: Arc<Mutex<BrokerInner>>,
    changed: Arc<Notify>,
    requests: broadcast::Sender<BrokerRequestFrame>,
    world_id: String,
    audit: WorldAuditLog,
    deadline: Duration,
}

impl std::fmt::Debug for ProviderBroker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderBroker")
            .field("world_id", &self.world_id)
            .finish_non_exhaustive()
    }
}

impl ProviderBroker {
    /// A broker for one world, auditing into the world's retained
    /// F12.6 log.
    #[must_use]
    pub(crate) fn new(world_id: String, audit: WorldAuditLog) -> Self {
        let (requests, _) = broadcast::channel(64);
        Self {
            inner: Arc::new(Mutex::new(BrokerInner::default())),
            changed: Arc::new(Notify::new()),
            requests,
            world_id,
            audit,
            deadline: Duration::from_millis(DEFAULT_PROVIDER_DEADLINE_MS),
        }
    }

    /// Subscribe to provider-request routing frames. Every subscriber
    /// sees every frame; the session layer sends only frames bound
    /// to its own session.
    #[must_use]
    pub(crate) fn subscribe_requests(&self) -> broadcast::Receiver<BrokerRequestFrame> {
        self.requests.subscribe()
    }

    /// Resolves when broker state changes (a completion is queued, a
    /// pending entry appears or expires). The world actor waits on
    /// this so a reply is delivered promptly instead of at the next
    /// command.
    pub(crate) async fn wait_changed(&self) {
        self.changed.notified().await;
    }

    /// The earliest pending deadline, so the actor's sleep can wake
    /// to expire requests on time.
    #[must_use]
    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner.pending.values().map(|entry| entry.deadline).min()
    }

    /// Record the providers one session offers (its latest
    /// `provider_offer` frame replaces the previous set). An offer
    /// enables nothing by itself; it is the availability half of the
    /// policy check at [`begin`](Self::begin).
    pub(crate) fn register_offer(&self, session: SessionId, providers: Vec<String>) {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner
            .offers
            .insert(session, providers.into_iter().collect());
    }

    /// A session ended: drop its offers and fail every pending
    /// request bound to it. There is no fallback to another client.
    pub(crate) fn remove_session(&self, session: SessionId) {
        let mut completed = Vec::new();
        {
            let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            inner.offers.remove(&session);
            let nonces: Vec<String> = inner
                .pending
                .iter()
                .filter(|(_, entry)| entry.session == session)
                .map(|(nonce, _)| nonce.clone())
                .collect();
            for nonce in nonces {
                if let Some(entry) = inner.pending.remove(&nonce) {
                    completed.push(entry);
                }
            }
            for entry in &completed {
                inner.completions.push(BrokerCompletion {
                    guest_request_id: entry.guest_request_id.clone(),
                    outcome: Err(CapError::failed_with("provider_disconnected")),
                });
            }
        }
        for entry in &completed {
            self.audit(entry, "disconnected", None);
        }
        if !completed.is_empty() {
            self.changed.notify_one();
        }
    }

    /// Run the policy order for one guest request and, when allowed,
    /// register the pending entry and route the request frame to the
    /// originating session. A refusal is audited and returned before
    /// any frame is sent.
    ///
    /// # Errors
    ///
    /// Returns the stable [`CapError`] the guest sees for the first
    /// failed policy check, in design order.
    pub(crate) fn begin(&self, request: BeginRequest<'_>) -> Result<BrokerRequestFrame, CapError> {
        let manifest = request.manifest;
        let provider = manifest.name;
        let args_digest = args_digest(request.args);
        let audit_ctx = AuditCtx {
            provider,
            provider_version: manifest.version,
            actor: &request.actor,
            args_digest: &args_digest,
            nonce: None,
        };
        // 1. Known manifest, and it must be a client-side provider:
        //    the broker is never a host executor.
        if manifest.side != ProviderSide::Client {
            self.audit_ctx(&audit_ctx, "denied", Some("not_client_provider"));
            return Err(CapError::unknown());
        }
        // 2. Server/world ceiling.
        if !request.ceiling_allows {
            self.audit_ctx(&audit_ctx, "denied", Some("outside_ceiling"));
            return Err(CapError::outside_ceiling());
        }
        // 3. Actor scope.
        if !request.actor_permits {
            self.audit_ctx(&audit_ctx, "denied", Some("actor_denied"));
            return Err(CapError::actor_denied());
        }
        // 4. Explicit world grant. Grant-on-use never applies to a
        //    client provider (design section 6).
        if !request.granted {
            self.audit_ctx(&audit_ctx, "denied", Some("not_granted"));
            return Err(CapError::denied());
        }
        // 5a. Attribution: a request with no originating session is
        //     never routed to an arbitrary session (design flow 6).
        let Some(session) = request.session else {
            self.audit_ctx(&audit_ctx, "denied", Some("unattributed"));
            return Err(CapError::actor_denied_with("unattributed"));
        };
        // Argument shape and size, against the manifest.
        if let Err(error) = validate_args(manifest, request.args) {
            self.audit_ctx(&audit_ctx, "denied", Some("bad_args"));
            return Err(error);
        }
        let frame = {
            let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            // 5b. Availability: the *originating* session must have
            //     offered this provider. Another session's offer does
            //     not make the provider available to this request.
            let offered = inner
                .offers
                .get(&session)
                .is_some_and(|providers| providers.contains(provider));
            if !offered {
                drop(inner);
                self.audit_ctx(&audit_ctx, "unavailable", Some("provider_unavailable"));
                return Err(CapError::failed_with("provider_unavailable"));
            }
            let in_flight = inner
                .pending
                .values()
                .filter(|entry| entry.provider == provider)
                .count();
            if in_flight >= MAX_PROVIDER_IN_FLIGHT {
                drop(inner);
                self.audit_ctx(&audit_ctx, "denied", Some("provider_busy"));
                return Err(CapError::failed_with("provider_busy"));
            }
            let nonce = mint_nonce();
            let max_result_bytes = effective_result_bound(manifest);
            inner.pending.insert(
                nonce.clone(),
                PendingEntry {
                    nonce: nonce.clone(),
                    provider: provider.to_owned(),
                    provider_version: manifest.version,
                    actor: request.actor.clone(),
                    participant: request.participant,
                    session,
                    guest_request_id: request.guest_request_id,
                    args_digest: args_digest.clone(),
                    max_result_bytes,
                    deadline: Instant::now() + self.deadline,
                },
            );
            BrokerRequestFrame {
                session,
                nonce,
                provider: provider.to_owned(),
                args: request.args.clone(),
                deadline_ms: u64::try_from(self.deadline.as_millis()).unwrap_or(u64::MAX),
            }
        };
        let audit_ctx = AuditCtx {
            nonce: Some(&frame.nonce),
            ..audit_ctx
        };
        self.audit_ctx(&audit_ctx, "allowed", None);
        // Route the frame; the session layer sends it only on the
        // bound session. No subscriber (session already gone) is not
        // an error here — the pending entry then ends by deadline or
        // by the session's remove_session.
        let _ = self.requests.send(frame.clone());
        self.changed.notify_one();
        Ok(frame)
    }

    /// Accept (or fail closed on) one `provider_reply` from a joined
    /// session. Exactly one reply completes a pending request; the
    /// entry is removed by every terminal outcome, so a duplicate
    /// reply afterwards is an unknown nonce.
    pub(crate) fn accept_reply(
        &self,
        session: SessionId,
        nonce: &str,
        ok: bool,
        result: Option<Vec<u8>>,
        error: Option<String>,
    ) -> ReplyOutcome {
        enum Verdict {
            Expired(PendingEntry),
            Terminal(PendingEntry, Result<Vec<u8>, ReplyFailure>),
        }
        enum ReplyFailure {
            Refused(String),
            Malformed,
            Oversized,
        }
        let verdict = {
            let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            let Some(entry_session) = inner.pending.get(nonce).map(|entry| entry.session) else {
                return ReplyOutcome::IgnoredUnknownNonce;
            };
            if entry_session != session {
                return ReplyOutcome::IgnoredWrongSession;
            }
            // Checked just above under the same lock, so the entry is there.
            let Some(entry) = inner.pending.remove(nonce) else {
                return ReplyOutcome::IgnoredUnknownNonce;
            };
            if Instant::now() >= entry.deadline {
                inner.completions.push(BrokerCompletion {
                    guest_request_id: entry.guest_request_id.clone(),
                    outcome: Err(CapError::failed_with("provider_timeout")),
                });
                Verdict::Expired(entry)
            } else {
                let outcome = match (ok, result, error) {
                    (true, Some(bytes), None) if bytes.len() <= entry.max_result_bytes => Ok(bytes),
                    (true, Some(_), None) => Err(ReplyFailure::Oversized),
                    (false, None, Some(code)) => Err(ReplyFailure::Refused(code)),
                    _ => Err(ReplyFailure::Malformed),
                };
                let completion = match &outcome {
                    Ok(bytes) => BrokerCompletion {
                        guest_request_id: entry.guest_request_id.clone(),
                        outcome: Ok(serde_json::json!({
                            "provider": entry.provider,
                            "result_hex": hex_encode(bytes),
                        })),
                    },
                    Err(ReplyFailure::Refused(code)) => BrokerCompletion {
                        guest_request_id: entry.guest_request_id.clone(),
                        outcome: Err(CapError::denied_with(stable_refusal_code(code))),
                    },
                    Err(ReplyFailure::Malformed) => BrokerCompletion {
                        guest_request_id: entry.guest_request_id.clone(),
                        outcome: Err(CapError::failed_with("malformed_reply")),
                    },
                    Err(ReplyFailure::Oversized) => BrokerCompletion {
                        guest_request_id: entry.guest_request_id.clone(),
                        outcome: Err(CapError::result_too_large()),
                    },
                };
                inner.completions.push(completion);
                Verdict::Terminal(entry, outcome)
            }
        };
        match verdict {
            Verdict::Expired(entry) => {
                self.audit(&entry, "timeout", None);
                self.changed.notify_one();
                ReplyOutcome::Expired
            }
            Verdict::Terminal(entry, outcome) => {
                let (audit_outcome, reply_outcome) = match &outcome {
                    Ok(_) => ("completed", ReplyOutcome::Accepted),
                    Err(ReplyFailure::Refused(code)) => {
                        let _ = code;
                        ("refused", ReplyOutcome::ClientRefused)
                    }
                    Err(ReplyFailure::Malformed) => ("invalid_reply", ReplyOutcome::Malformed),
                    Err(ReplyFailure::Oversized) => ("oversized_reply", ReplyOutcome::Malformed),
                };
                self.audit(&entry, audit_outcome, None);
                self.changed.notify_one();
                reply_outcome
            }
        }
    }

    /// A reply whose result bytes could not even be decoded (bad hex
    /// in the frame). If it names a pending nonce bound to this
    /// session, the request completes as malformed; anything else is
    /// ignored, fail closed.
    pub(crate) fn note_malformed_reply(&self, session: SessionId, nonce: &str) {
        let entry = {
            let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            match inner.pending.get(nonce) {
                Some(entry) if entry.session == session => inner.pending.remove(nonce),
                _ => None,
            }
        };
        if let Some(entry) = entry {
            {
                let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
                inner.completions.push(BrokerCompletion {
                    guest_request_id: entry.guest_request_id.clone(),
                    outcome: Err(CapError::failed_with("malformed_reply")),
                });
            }
            self.audit(&entry, "invalid_reply", None);
            self.changed.notify_one();
        }
    }

    /// Record the world actor's restarted refusal of a client-provider
    /// request (design section 4, flow step 6): the request was already
    /// listed when the world reopened, so it is refused unattributed
    /// without reaching [`begin`](Self::begin). The refusal is still a
    /// host provider decision, so it belongs in the same F12.6 trail —
    /// outcome `denied`, reason `restarted`, arguments digest only —
    /// keeping the audit complete across a reopen.
    pub(crate) fn note_restarted_refusal(
        &self,
        manifest: &ProviderManifest,
        actor: &str,
        args: &serde_json::Value,
    ) {
        let args_digest = args_digest(args);
        let ctx = AuditCtx {
            provider: manifest.name,
            provider_version: manifest.version,
            actor,
            args_digest: &args_digest,
            nonce: None,
        };
        self.audit_ctx(&ctx, "denied", Some("restarted"));
    }

    /// Complete every pending request whose deadline has passed.
    /// Called by the world actor when its sleep wakes at
    /// [`next_deadline`](Self::next_deadline).
    pub(crate) fn collect_expired(&self) {
        let now = Instant::now();
        let mut expired = Vec::new();
        {
            let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            let nonces: Vec<String> = inner
                .pending
                .iter()
                .filter(|(_, entry)| entry.deadline <= now)
                .map(|(nonce, _)| nonce.clone())
                .collect();
            for nonce in nonces {
                if let Some(entry) = inner.pending.remove(&nonce) {
                    inner.completions.push(BrokerCompletion {
                        guest_request_id: entry.guest_request_id.clone(),
                        outcome: Err(CapError::failed_with("provider_timeout")),
                    });
                    expired.push(entry);
                }
            }
        }
        for entry in &expired {
            self.audit(entry, "timeout", None);
        }
        if !expired.is_empty() {
            self.changed.notify_one();
        }
    }

    /// Take every queued completion. The actor delivers each one to
    /// the guest as the ordinary capability result event.
    pub(crate) fn drain_completions(&self) -> Vec<BrokerCompletion> {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        std::mem::take(&mut inner.completions)
    }

    /// How many requests are currently in flight (test and metrics
    /// visibility).
    #[must_use]
    pub(crate) fn pending_count(&self) -> usize {
        let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner.pending.len()
    }

    fn audit(&self, entry: &PendingEntry, outcome: &str, reason: Option<&str>) {
        let ctx = AuditCtx {
            provider: &entry.provider,
            provider_version: entry.provider_version,
            actor: &entry.actor,
            args_digest: &entry.args_digest,
            nonce: Some(&entry.nonce),
        };
        self.audit_ctx(&ctx, outcome, reason);
    }

    fn audit_ctx(&self, ctx: &AuditCtx<'_>, outcome: &str, reason: Option<&str>) {
        let mut parameters = serde_json::json!({
            "provider": ctx.provider,
            "provider_version": ctx.provider_version,
            "capability": ctx.provider,
            "args_digest": ctx.args_digest,
        });
        if let Some(nonce) = ctx.nonce {
            parameters["nonce"] = serde_json::Value::String(nonce.to_owned());
        }
        if let Some(reason) = reason {
            parameters["reason"] = serde_json::Value::String(reason.to_owned());
        }
        let record = AuditRecord {
            schema_version: 2,
            timestamp_ms: unix_ms(),
            world_id: self.world_id.clone(),
            actor: ctx.actor.to_owned(),
            action: "client_provider_use".to_owned(),
            parameters,
            outcome: outcome.to_owned(),
            target: ctx.provider.to_owned(),
            correlation_id: super::next_audit_correlation_id(),
        };
        self.audit.record(record.clone());
        emit_audit_record(&record);
    }
}

/// The audit fields one broker decision shares.
struct AuditCtx<'a> {
    provider: &'a str,
    provider_version: u32,
    actor: &'a str,
    args_digest: &'a str,
    nonce: Option<&'a str>,
}

/// The refusal code a client reply may carry into the guest's
/// result event: a short stable token (lowercase `snake_case`, at
/// most 64 bytes) — the shape of every refusal constant the shell
/// adapter itself returns. A reply is client input at the host
/// boundary, and the code lands in the durable journal as the
/// result event's `reason`; an arbitrary string (a local path, file
/// content, a paragraph of text) must never be journaled as a
/// refusal reason, so anything outside the stable shape is replaced
/// by the generic `provider_refused`. The host audit outcome is
/// unaffected: it never carried the client code.
fn stable_refusal_code(code: &str) -> String {
    let stable = !code.is_empty()
        && code.len() <= 64
        && code.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
        && code
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    if stable {
        code.to_owned()
    } else {
        "provider_refused".to_owned()
    }
}

/// BLAKE3 of the canonical args JSON — the only args-derived value
/// the audit log ever carries.
fn args_digest(args: &serde_json::Value) -> String {
    let canonical = serde_json::to_string(args).unwrap_or_default();
    blake3::hash(canonical.as_bytes()).to_hex().to_string()
}

/// A host-minted nonce: 128 bits of host randomness, hex-encoded.
/// It is a correlation handle, not a credential — possession is never
/// sufficient, because the broker also binds the replying session.
fn mint_nonce() -> String {
    use rand::RngExt as _;
    let mut rng = rand::rng();
    let hi: u64 = rng.random();
    let lo: u64 = rng.random();
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&hi.to_be_bytes());
    bytes[8..].copy_from_slice(&lo.to_be_bytes());
    hex_encode(&bytes)
}

/// The effective result bound: the manifest's `ResponseBytes` bound
/// composed by minimum with the protocol's event bound (a serialized
/// result event must fit [`crate::world_caps::MAX_EVENT_BYTES`], and
/// the design's `result_too_large` guard is the last resort, not the
/// first).
fn effective_result_bound(manifest: &ProviderManifest) -> usize {
    let manifest_bound = manifest
        .bounds
        .iter()
        .filter_map(|bound| match bound {
            ProviderBound::ResponseBytes(bytes) => Some(*bytes),
            _ => None,
        })
        .min();
    let protocol_bound = crate::world_caps::MAX_EVENT_BYTES;
    manifest_bound.map_or(protocol_bound, |bound| bound.min(protocol_bound))
}

/// Validate the guest's arguments against the manifest: serialized
/// size against `ArgsBytes`, and — for a JSON-object schema — the
/// required fields' presence and value kinds. Unknown fields are
/// tolerated, matching the registry's existing validation posture;
/// the shell adapter re-checks everything locally regardless.
fn validate_args(manifest: &ProviderManifest, args: &serde_json::Value) -> Result<(), CapError> {
    for bound in manifest.bounds {
        if let ProviderBound::ArgsBytes(limit) = bound {
            let size = serde_json::to_string(args).map_or(0, |text| text.len());
            if size > *limit {
                return Err(CapError::bad_args_with("args_too_large"));
            }
        }
    }
    if let ArgumentSchema::JsonObject(fields) = manifest.argument_schema {
        let object = args
            .as_object()
            .ok_or_else(|| CapError::bad_args_with("args_not_object"))?;
        for field in fields {
            match object.get(field.name) {
                None if field.required => {
                    return Err(CapError::bad_args_with("missing_required_arg"));
                }
                Some(value) => {
                    let kind_ok = match field.value_type {
                        ArgumentType::String => value.is_string(),
                        ArgumentType::UnsignedInteger => value.is_u64(),
                        ArgumentType::Object => value.is_object(),
                    };
                    if !kind_ok {
                        return Err(CapError::bad_args_with("arg_type_mismatch"));
                    }
                }
                None => {}
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::world_caps::{
        ArgumentSchema, ArgumentType, DefaultCeiling, ProviderArgument, ProviderKind, RequiredScope,
    };

    /// The fake client provider the adapter tests run against: a
    /// client-side manifest with small, assertable bounds.
    const TEST_MANIFEST: ProviderManifest = ProviderManifest {
        name: "client.test.echo",
        version: 3,
        kind: ProviderKind::Request,
        argument_schema: ArgumentSchema::JsonObject(&[ProviderArgument {
            name: "path",
            value_type: ArgumentType::String,
            required: true,
        }]),
        bounds: &[
            ProviderBound::ArgsBytes(256),
            ProviderBound::ResponseBytes(64),
        ],
        required_scope: RequiredScope::Join,
        default_ceiling: DefaultCeiling::Excluded,
        side: ProviderSide::Client,
        class: id_caps::cap_class::CLASS_UNKNOWN,
    };

    const SESSION_A: SessionId = SessionId::new(41);
    const SESSION_B: SessionId = SessionId::new(42);

    fn broker() -> (ProviderBroker, WorldAuditLog) {
        let audit = WorldAuditLog::new();
        (
            ProviderBroker::new("world-test".to_owned(), audit.clone()),
            audit,
        )
    }

    static TEST_ARGS: std::sync::LazyLock<serde_json::Value> =
        std::sync::LazyLock::new(|| serde_json::json!({"path": "notes/readme.txt"}));

    fn args() -> &'static serde_json::Value {
        &TEST_ARGS
    }

    /// A begin request whose policy snapshot allows everything; a
    /// test flips exactly the fact it exercises.
    fn allowed_begin<'a>(
        broker: &ProviderBroker,
        manifest: &'a ProviderManifest,
        session: Option<SessionId>,
    ) -> BeginRequest<'a> {
        let _ = broker;
        BeginRequest {
            manifest,
            ceiling_allows: true,
            actor_permits: true,
            granted: true,
            actor: "participant:7".to_owned(),
            participant: Some(7),
            session,
            guest_request_id: "req-1".to_owned(),
            args: args(),
        }
    }

    /// What a fake shell adapter does with one routed request frame.
    /// This is the test implementation of the adapter seam: the same
    /// offer/request/reply protocol a real shell adapter satisfies,
    /// with each failure mode scripted.
    #[derive(Clone, Debug)]
    enum FakeBehaviour {
        /// Answer with these bytes.
        Ok(Vec<u8>),
        /// Refuse with a stable local code.
        Refuse(&'static str),
        /// Answer `ok` with no result at all.
        Malformed,
        /// Never answer (the request times out or disconnects).
        Never,
        /// End the session without answering.
        Disconnect,
    }

    struct FakeAdapter {
        session: SessionId,
        behaviour: FakeBehaviour,
    }

    impl FakeAdapter {
        /// Advertise the test provider for this adapter's session.
        fn offer(&self, broker: &ProviderBroker) {
            broker.register_offer(self.session, vec![TEST_MANIFEST.name.to_owned()]);
        }

        /// Consume one routed frame the way a shell would: check the
        /// frame is bound to this session, then act out the scripted
        /// behaviour against the broker.
        fn handle(
            &self,
            broker: &ProviderBroker,
            frame: &BrokerRequestFrame,
        ) -> Option<ReplyOutcome> {
            assert_eq!(
                frame.session, self.session,
                "frame routed to another session"
            );
            match &self.behaviour {
                FakeBehaviour::Ok(bytes) => Some(broker.accept_reply(
                    self.session,
                    &frame.nonce,
                    true,
                    Some(bytes.clone()),
                    None,
                )),
                FakeBehaviour::Refuse(code) => Some(broker.accept_reply(
                    self.session,
                    &frame.nonce,
                    false,
                    None,
                    Some((*code).to_owned()),
                )),
                FakeBehaviour::Malformed => {
                    Some(broker.accept_reply(self.session, &frame.nonce, true, None, None))
                }
                FakeBehaviour::Never => None,
                FakeBehaviour::Disconnect => {
                    broker.remove_session(self.session);
                    None
                }
            }
        }
    }

    fn begin_ok(broker: &ProviderBroker) -> BrokerRequestFrame {
        broker
            .begin(allowed_begin(broker, &TEST_MANIFEST, Some(SESSION_A)))
            .expect("allowed request begins")
    }

    fn drain_one(broker: &ProviderBroker) -> BrokerCompletion {
        let completions = broker.drain_completions();
        assert_eq!(completions.len(), 1, "exactly one completion");
        completions.into_iter().next().expect("one completion")
    }

    fn audit_outcomes(audit: &WorldAuditLog) -> Vec<String> {
        audit
            .records()
            .iter()
            .filter(|record| record.action == "client_provider_use")
            .map(|record| record.outcome.clone())
            .collect()
    }

    #[test]
    fn happy_path_completes_with_bounded_result_and_audit() {
        let (broker, audit) = broker();
        let adapter = FakeAdapter {
            session: SESSION_A,
            behaviour: FakeBehaviour::Ok(b"hello".to_vec()),
        };
        adapter.offer(&broker);
        let frame = begin_ok(&broker);
        assert_eq!(frame.provider, "client.test.echo");
        assert_eq!(
            adapter.handle(&broker, &frame),
            Some(ReplyOutcome::Accepted)
        );
        let completion = drain_one(&broker);
        assert_eq!(completion.guest_request_id, "req-1");
        let value = completion.outcome.expect("ok completion");
        assert_eq!(value["provider"], "client.test.echo");
        assert_eq!(value["result_hex"], hex_encode(b"hello"));
        assert_eq!(broker.pending_count(), 0);
        // Audit: allow then completion, with provider version and an
        // args digest — and never the raw args.
        assert_eq!(audit_outcomes(&audit), vec!["allowed", "completed"]);
        let records = audit.records();
        let first = &records[0];
        assert_eq!(first.parameters["provider_version"], 3);
        assert!(first.parameters["args_digest"].is_string());
        assert!(!first.parameters.to_string().contains("notes/readme.txt"));
        assert!(!first.correlation_id.is_empty());
    }

    #[test]
    fn ceiling_refusal_happens_before_any_frame() {
        let (broker, audit) = broker();
        FakeAdapter {
            session: SESSION_A,
            behaviour: FakeBehaviour::Never,
        }
        .offer(&broker);
        let mut request = allowed_begin(&broker, &TEST_MANIFEST, Some(SESSION_A));
        request.ceiling_allows = false;
        let error = broker.begin(request).expect_err("ceiling refuses");
        assert_eq!(error.code(), "outside_ceiling");
        assert_eq!(broker.pending_count(), 0);
        assert_eq!(audit_outcomes(&audit), vec!["denied"]);
    }

    #[test]
    fn actor_scope_and_missing_grant_refuse_in_order() {
        let (broker, audit) = broker();
        FakeAdapter {
            session: SESSION_A,
            behaviour: FakeBehaviour::Never,
        }
        .offer(&broker);
        let mut request = allowed_begin(&broker, &TEST_MANIFEST, Some(SESSION_A));
        request.actor_permits = false;
        assert_eq!(
            broker.begin(request).expect_err("scope refuses").code(),
            "actor_denied"
        );
        let mut request = allowed_begin(&broker, &TEST_MANIFEST, Some(SESSION_A));
        request.granted = false;
        // No grant-on-use for client providers: an ungranted use is a
        // plain denial even though every other gate allows it.
        assert_eq!(
            broker.begin(request).expect_err("grant refuses").code(),
            "cap_denied"
        );
        assert_eq!(audit_outcomes(&audit), vec!["denied", "denied"]);
    }

    #[test]
    fn unattributed_and_unoffered_requests_are_unavailable_or_refused() {
        let (broker, _audit) = broker();
        // No session at all: unattributed, never routed.
        let error = broker
            .begin(allowed_begin(&broker, &TEST_MANIFEST, None))
            .expect_err("unattributed refuses");
        assert_eq!(error.code(), "actor_denied");
        // A different session offered the provider; the originating
        // session did not. Availability is per originating session.
        FakeAdapter {
            session: SESSION_B,
            behaviour: FakeBehaviour::Never,
        }
        .offer(&broker);
        let error = broker
            .begin(allowed_begin(&broker, &TEST_MANIFEST, Some(SESSION_A)))
            .expect_err("unoffered refuses");
        assert_eq!(error.code(), "failed");
        assert_eq!(broker.pending_count(), 0);
    }

    #[test]
    fn client_refusal_completes_with_its_stable_code() {
        let (broker, audit) = broker();
        let adapter = FakeAdapter {
            session: SESSION_A,
            behaviour: FakeBehaviour::Refuse("local_denied"),
        };
        adapter.offer(&broker);
        let frame = begin_ok(&broker);
        assert_eq!(
            adapter.handle(&broker, &frame),
            Some(ReplyOutcome::ClientRefused)
        );
        let completion = drain_one(&broker);
        assert_eq!(
            completion.outcome.expect_err("refused").code(),
            "cap_denied"
        );
        assert_eq!(audit_outcomes(&audit), vec!["allowed", "refused"]);
    }

    #[test]
    fn timeout_completes_the_request_and_a_late_reply_is_ignored() {
        let (broker, audit) = broker();
        let mut broker = broker;
        broker.deadline = Duration::from_millis(20);
        let adapter = FakeAdapter {
            session: SESSION_A,
            behaviour: FakeBehaviour::Never,
        };
        adapter.offer(&broker);
        let frame = begin_ok(&broker);
        std::thread::sleep(Duration::from_millis(40));
        broker.collect_expired();
        let completion = drain_one(&broker);
        let error = completion.outcome.expect_err("timed out");
        assert_eq!(error.code(), "failed");
        // The adapter finally answers: the nonce is gone, fail closed.
        assert_eq!(
            broker.accept_reply(SESSION_A, &frame.nonce, true, Some(b"x".to_vec()), None),
            ReplyOutcome::IgnoredUnknownNonce
        );
        assert!(broker.drain_completions().is_empty());
        assert_eq!(audit_outcomes(&audit), vec!["allowed", "timeout"]);
    }

    #[test]
    fn disconnect_completes_the_request_without_fallback() {
        let (broker, audit) = broker();
        let adapter = FakeAdapter {
            session: SESSION_A,
            behaviour: FakeBehaviour::Disconnect,
        };
        adapter.offer(&broker);
        let frame = begin_ok(&broker);
        assert_eq!(adapter.handle(&broker, &frame), None);
        let completion = drain_one(&broker);
        assert_eq!(
            completion.outcome.expect_err("disconnected").code(),
            "failed"
        );
        // The offer is gone with the session.
        let error = broker
            .begin(allowed_begin(&broker, &TEST_MANIFEST, Some(SESSION_A)))
            .expect_err("disconnected session offers nothing");
        assert_eq!(error.code(), "failed");
        assert_eq!(
            audit_outcomes(&audit),
            vec!["allowed", "disconnected", "unavailable"]
        );
    }

    #[test]
    fn replayed_reply_cannot_complete_twice() {
        let (broker, _audit) = broker();
        let adapter = FakeAdapter {
            session: SESSION_A,
            behaviour: FakeBehaviour::Ok(b"once".to_vec()),
        };
        adapter.offer(&broker);
        let frame = begin_ok(&broker);
        assert_eq!(
            adapter.handle(&broker, &frame),
            Some(ReplyOutcome::Accepted)
        );
        // Replay: same session, same nonce, after completion.
        assert_eq!(
            broker.accept_reply(SESSION_A, &frame.nonce, true, Some(b"twice".to_vec()), None),
            ReplyOutcome::IgnoredUnknownNonce
        );
        let completion = drain_one(&broker);
        assert_eq!(
            completion.outcome.expect("ok")["result_hex"],
            hex_encode(b"once")
        );
        assert!(broker.drain_completions().is_empty());
    }

    #[test]
    fn wrong_session_reply_is_ignored_and_pending_survives() {
        let (broker, _audit) = broker();
        let adapter = FakeAdapter {
            session: SESSION_A,
            behaviour: FakeBehaviour::Ok(b"mine".to_vec()),
        };
        adapter.offer(&broker);
        let frame = begin_ok(&broker);
        assert_eq!(
            broker.accept_reply(
                SESSION_B,
                &frame.nonce,
                true,
                Some(b"stolen".to_vec()),
                None
            ),
            ReplyOutcome::IgnoredWrongSession
        );
        assert_eq!(broker.pending_count(), 1);
        assert!(broker.drain_completions().is_empty());
        // The bound session's own reply still completes it.
        assert_eq!(
            adapter.handle(&broker, &frame),
            Some(ReplyOutcome::Accepted)
        );
        assert_eq!(drain_one(&broker).guest_request_id, "req-1");
    }

    #[test]
    fn malformed_and_oversized_replies_fail_closed() {
        let (broker, audit) = broker();
        let adapter = FakeAdapter {
            session: SESSION_A,
            behaviour: FakeBehaviour::Malformed,
        };
        adapter.offer(&broker);
        let frame = begin_ok(&broker);
        assert_eq!(
            adapter.handle(&broker, &frame),
            Some(ReplyOutcome::Malformed)
        );
        assert_eq!(
            drain_one(&broker).outcome.expect_err("malformed").code(),
            "failed"
        );
        // Oversized: the manifest bound is 64 response bytes.
        let adapter = FakeAdapter {
            session: SESSION_A,
            behaviour: FakeBehaviour::Ok(vec![7u8; 65]),
        };
        let frame = begin_ok(&broker);
        assert_eq!(
            adapter.handle(&broker, &frame),
            Some(ReplyOutcome::Malformed)
        );
        assert_eq!(
            drain_one(&broker).outcome.expect_err("oversized").code(),
            "result_too_large"
        );
        assert_eq!(
            audit_outcomes(&audit),
            vec!["allowed", "invalid_reply", "allowed", "oversized_reply"]
        );
    }

    #[test]
    fn bad_args_refuse_at_begin() {
        let (broker, _audit) = broker();
        FakeAdapter {
            session: SESSION_A,
            behaviour: FakeBehaviour::Never,
        }
        .offer(&broker);
        let bad = serde_json::json!({"wrong": 1});
        let mut request = allowed_begin(&broker, &TEST_MANIFEST, Some(SESSION_A));
        request.args = &bad;
        assert_eq!(
            broker.begin(request).expect_err("schema refuses").code(),
            "bad_args"
        );
        assert_eq!(broker.pending_count(), 0);
    }

    #[test]
    fn host_side_manifest_is_never_brokered() {
        let (broker, _audit) = broker();
        let mut host_manifest = TEST_MANIFEST;
        host_manifest.side = ProviderSide::Host;
        host_manifest.name = "time.now";
        let error = broker
            .begin(allowed_begin(&broker, &host_manifest, Some(SESSION_A)))
            .expect_err("host provider refused");
        assert_eq!(error.code(), "unknown_cap");
    }

    // -- F13.7 step 6: the real client.file.read manifest through
    //    the broker, the refusal-code trust boundary, and the
    //    restarted-refusal audit record. ---------------------------

    #[test]
    fn real_manifest_args_are_validated_against_its_schema() {
        let (broker, _audit) = broker();
        let manifest = &crate::world_caps::CLIENT_FILE_READ_MANIFEST;
        broker.register_offer(SESSION_A, vec![manifest.name.to_owned()]);
        // Missing the manifest's one required argument.
        let missing = serde_json::json!({});
        let mut request = allowed_begin(&broker, manifest, Some(SESSION_A));
        request.args = &missing;
        let err = broker.begin(request).expect_err("missing path refuses");
        assert_eq!(err.code(), "bad_args");
        assert_eq!(
            err.reason().map(String::as_str),
            Some("missing_required_arg")
        );
        // The path must be a string.
        let wrong_type = serde_json::json!({ "path": 42 });
        let mut request = allowed_begin(&broker, manifest, Some(SESSION_A));
        request.args = &wrong_type;
        assert_eq!(
            broker
                .begin(request)
                .expect_err("non-string path refuses")
                .code(),
            "bad_args"
        );
        // Serialized args over the manifest's 8 KiB ArgsBytes bound
        // refuse whole, before any frame is dispatched.
        let big = serde_json::json!({ "path": "x".repeat(9000) });
        let mut request = allowed_begin(&broker, manifest, Some(SESSION_A));
        request.args = &big;
        assert_eq!(
            broker
                .begin(request)
                .expect_err("oversize args refuse")
                .code(),
            "bad_args"
        );
        assert_eq!(broker.pending_count(), 0);
        // A well-formed request begins. (allowed_begin's default args
        // are {"path": "notes/readme.txt"}.)
        assert!(
            broker
                .begin(allowed_begin(&broker, manifest, Some(SESSION_A)))
                .is_ok()
        );
    }

    #[test]
    fn real_manifest_result_bound_is_the_manifests_32_kib() {
        let (broker, _audit) = broker();
        let manifest = &crate::world_caps::CLIENT_FILE_READ_MANIFEST;
        broker.register_offer(SESSION_A, vec![manifest.name.to_owned()]);
        let frame = broker
            .begin(allowed_begin(&broker, manifest, Some(SESSION_A)))
            .expect("request begins");
        let outcome = broker.accept_reply(
            SESSION_A,
            &frame.nonce,
            true,
            Some(vec![7_u8; 32_768]),
            None,
        );
        assert_eq!(outcome, ReplyOutcome::Accepted);
        assert!(drain_one(&broker).outcome.is_ok());

        let frame = broker
            .begin(allowed_begin(&broker, manifest, Some(SESSION_A)))
            .expect("request begins");
        broker.accept_reply(
            SESSION_A,
            &frame.nonce,
            true,
            Some(vec![7_u8; 32_769]),
            None,
        );
        let completion = drain_one(&broker);
        assert_eq!(
            completion
                .outcome
                .expect_err("oversize result refuses")
                .code(),
            "result_too_large"
        );
    }

    #[test]
    fn client_refusal_codes_outside_the_stable_shape_are_replaced() {
        // A refusal code lands in the durable journal as the result
        // event's reason, so a client implementation must not be
        // able to journal arbitrary text — a local path, prose,
        // file content — through it. Anything that is not a short
        // lowercase snake_case token becomes `provider_refused`.
        let overlong: &'static str = Box::leak("x".repeat(65).into_boxed_str());
        for code in [
            "/home/user/notes.txt",
            "Has Spaces",
            "UPPER_CASE",
            "two words",
            "",
            overlong,
        ] {
            let (broker, _audit) = broker();
            let adapter = FakeAdapter {
                session: SESSION_A,
                behaviour: FakeBehaviour::Refuse(code),
            };
            adapter.offer(&broker);
            let frame = begin_ok(&broker);
            let outcome = adapter.handle(&broker, &frame).expect("reply handled");
            assert_eq!(outcome, ReplyOutcome::ClientRefused, "code {code:?}");
            let completion = drain_one(&broker);
            let event =
                crate::world_caps::result_event(&completion.guest_request_id, completion.outcome);
            let parsed: serde_json::Value = serde_json::from_str(&event).unwrap();
            assert_eq!(parsed["error"], "cap_denied", "code {code:?}");
            assert_eq!(parsed["reason"], "provider_refused", "code {code:?}");
        }
        // The shell adapter's own stable codes pass through untouched.
        let (broker, _audit) = broker();
        let adapter = FakeAdapter {
            session: SESSION_A,
            behaviour: FakeBehaviour::Refuse("file_read_invalid_path"),
        };
        adapter.offer(&broker);
        let frame = begin_ok(&broker);
        adapter.handle(&broker, &frame).expect("reply handled");
        let completion = drain_one(&broker);
        let event =
            crate::world_caps::result_event(&completion.guest_request_id, completion.outcome);
        let parsed: serde_json::Value = serde_json::from_str(&event).unwrap();
        assert_eq!(parsed["reason"], "file_read_invalid_path");
    }

    #[test]
    fn restarted_refusal_is_recorded_in_the_audit_trail() {
        let (broker, audit) = broker();
        broker.note_restarted_refusal(
            &crate::world_caps::CLIENT_FILE_READ_MANIFEST,
            "program",
            args(),
        );
        let records: Vec<_> = audit
            .records()
            .into_iter()
            .filter(|record| record.action == "client_provider_use")
            .collect();
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.outcome, "denied");
        assert_eq!(record.parameters["provider"], "client.file.read");
        assert_eq!(record.parameters["reason"], "restarted");
        assert!(
            !record.parameters.to_string().contains("notes/readme.txt"),
            "audit carries the args digest, never argument values: {record:?}"
        );
    }
}
