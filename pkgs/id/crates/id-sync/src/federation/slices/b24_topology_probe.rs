//! B24: nonce-bound topology probes (#10265, statements S1.77 and S1.78;
//! design "Routing validation and reputation policy").
//!
//! Routing state is an attack surface (S1.77): a forged "I reached that
//! peer" can redirect fan-out, hide results or manufacture loops. So a
//! node learns about its own links only from probes it sent and that
//! the probed peer answered under its own key:
//!
//! - A [`ProbeRequest`] is signed by the prober and names the target,
//!   the scope, a fresh nonce, when it was sent and when it times out.
//! - A [`ProbeResponse`] is signed by the responder and binds the
//!   responder, the prober, the scope, the nonce and the digest of the
//!   exact request it answers. [`respond`] answers only a request that
//!   is addressed to the responder, verifies under the prober's key and
//!   has not timed out.
//! - A [`Prober`] keeps one probe in flight per configured peer. It
//!   accepts a response only from the peer that probe went to, with the
//!   same nonce, scope and request digest, before the timeout, with a
//!   valid signature from that peer's configured key; anything else is
//!   refused and changes nothing. A probe answered in time marks the
//!   peer reachable; one that times out marks it unreachable. Either
//!   piece of evidence lasts a finite TTL.
//! - Probes run on a locally configured interval with jitter. The jitter
//!   is taken from the probe's own fresh nonce, so it is unpredictable
//!   to peers and reproducible in tests.
//! - [`Prober::observation`] turns current evidence into this node's own
//!   B23 [`RouteObservation`]: links from this node only, in its scope,
//!   valid only while every link's evidence is. It is partial by design
//!   (S1.78): peers never probed, or whose evidence expired, are simply
//!   absent. Probing changes only this node's local scoped state; it
//!   never writes anything about other observers.
//!
//! Rollback: [`Prober::disable`] stops probing, drops probes in flight
//! and expires all probe-derived evidence, while keeping the configured
//! peers and their keys.
//!
//! Signed bytes (integers big-endian, strings as a u32 length and UTF-8
//! bytes; the signature is Ed25519 over the whole preimage):
//!
//! ```text
//! request:  "id-sync/b24-probe-request/v1" prober target scope
//!           nonce:32 sent_at:u64 expires_at:u64
//! response: "id-sync/b24-probe-response/v1" responder prober scope
//!           nonce:32 request_digest:32 responded_at:u64
//! ```
//!
//! `request_digest` is SHA-256 over the request preimage followed by its
//! signature.
//!
//! This module is deliberately self-contained: the federation roots
//! record slice paths without declaring them (slice 9110-02), so the
//! slice's integration test includes this file directly and no shared
//! root names it yet; #9110-09 wires versioned public exports after
//! module outputs land.

use std::collections::BTreeMap;

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};

use super::b23_routing_state::{LinkStatus, PeerId, RouteObservation};

/// Domain prefix of a probe request preimage.
pub const REQUEST_DOMAIN: &[u8] = b"id-sync/b24-probe-request/v1";
/// Domain prefix of a probe response preimage.
pub const RESPONSE_DOMAIN: &[u8] = b"id-sync/b24-probe-response/v1";

/// A fresh probe nonce.
pub type Nonce = [u8; 32];

fn push_str(out: &mut Vec<u8>, value: &str) {
    out.extend_from_slice(&u32::try_from(value.len()).unwrap_or(u32::MAX).to_be_bytes());
    out.extend_from_slice(value.as_bytes());
}

fn verify(key: &VerifyingKey, preimage: &[u8], signature: &[u8; 64]) -> bool {
    key.verify_strict(preimage, &Signature::from_bytes(signature))
        .is_ok()
}

/// A signed probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeRequest {
    /// Who probes, and signs.
    pub prober: PeerId,
    /// Who is probed.
    pub target: PeerId,
    /// The routing scope.
    pub scope: String,
    /// Fresh per probe.
    pub nonce: Nonce,
    /// When it was sent.
    pub sent_at: u64,
    /// When it times out.
    pub expires_at: u64,
    /// The prober's signature over [`Self::preimage`].
    pub signature: [u8; 64],
}

impl ProbeRequest {
    /// The signed bytes.
    pub fn preimage(&self) -> Vec<u8> {
        let mut out = REQUEST_DOMAIN.to_vec();
        push_str(&mut out, &self.prober);
        push_str(&mut out, &self.target);
        push_str(&mut out, &self.scope);
        out.extend_from_slice(&self.nonce);
        out.extend_from_slice(&self.sent_at.to_be_bytes());
        out.extend_from_slice(&self.expires_at.to_be_bytes());
        out
    }

    /// SHA-256 over the preimage and the signature.
    pub fn digest(&self) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(self.preimage());
        h.update(self.signature);
        h.finalize().into()
    }
}

/// A signed answer to one probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeResponse {
    /// Who answers, and signs.
    pub responder: PeerId,
    /// Who sent the probe.
    pub prober: PeerId,
    /// The routing scope.
    pub scope: String,
    /// The probe's nonce.
    pub nonce: Nonce,
    /// The digest of the exact request answered.
    pub request_digest: [u8; 32],
    /// When it answered.
    pub responded_at: u64,
    /// The responder's signature over [`Self::preimage`].
    pub signature: [u8; 64],
}

impl ProbeResponse {
    /// The signed bytes.
    pub fn preimage(&self) -> Vec<u8> {
        let mut out = RESPONSE_DOMAIN.to_vec();
        push_str(&mut out, &self.responder);
        push_str(&mut out, &self.prober);
        push_str(&mut out, &self.scope);
        out.extend_from_slice(&self.nonce);
        out.extend_from_slice(&self.request_digest);
        out.extend_from_slice(&self.responded_at.to_be_bytes());
        out
    }
}

/// Why a responder will not answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RespondError {
    /// The probe is addressed to someone else.
    NotForMe,
    /// The probe's signature does not verify under the prober's key.
    BadSignature,
    /// The probe has timed out.
    Expired,
}

/// Answer `request` as `responder`, checking it under `prober_key`.
pub fn respond(
    key: &SigningKey,
    responder: &str,
    request: &ProbeRequest,
    prober_key: &VerifyingKey,
    now: u64,
) -> Result<ProbeResponse, RespondError> {
    if request.target != responder {
        return Err(RespondError::NotForMe);
    }
    if !verify(prober_key, &request.preimage(), &request.signature) {
        return Err(RespondError::BadSignature);
    }
    if now >= request.expires_at {
        return Err(RespondError::Expired);
    }
    let mut response = ProbeResponse {
        responder: responder.to_owned(),
        prober: request.prober.clone(),
        scope: request.scope.clone(),
        nonce: request.nonce,
        request_digest: request.digest(),
        responded_at: now,
        signature: [0; 64],
    };
    response.signature = key.sign(&response.preimage()).to_bytes();
    Ok(response)
}

/// Locally configured probe timing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbeConfig {
    /// Base time between probes of one peer.
    pub interval: u64,
    /// Up to this much is added to each interval.
    pub jitter: u64,
    /// How long a probe waits for its answer.
    pub timeout: u64,
    /// How long one probe's evidence lasts.
    pub ttl: u64,
}

/// Why a probe could not start or a response was refused. A refusal
/// changes nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeError {
    /// Probing is disabled.
    Disabled,
    /// The peer is not configured.
    UnknownPeer,
    /// A probe to this peer is already in flight.
    InFlight,
    /// No probe to this responder is in flight.
    Unsolicited,
    /// The response names another prober.
    WrongProber,
    /// The response is for another scope.
    WrongScope,
    /// The response carries another nonce.
    WrongNonce,
    /// The response answers another request.
    WrongRequest,
    /// The response claims a time before the probe was sent or after now.
    BadTime,
    /// The probe has timed out.
    Late,
    /// The signature does not verify under the responder's key.
    BadSignature,
}

/// Why a probe could not be configured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigError {
    /// A node does not probe itself.
    SelfProbe,
    /// Timeout and TTL must be positive.
    ZeroDuration,
}

/// One piece of probe evidence about a peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Evidence {
    /// What the probe showed.
    pub status: LinkStatus,
    /// When it showed it.
    pub observed_at: u64,
    /// When the evidence stops counting.
    pub expires_at: u64,
}

/// A node's prober for one scope.
#[derive(Debug, Clone)]
pub struct Prober {
    me: PeerId,
    key: SigningKey,
    scope: String,
    config: ProbeConfig,
    enabled: bool,
    peers: BTreeMap<PeerId, VerifyingKey>,
    next_due: BTreeMap<PeerId, u64>,
    in_flight: BTreeMap<PeerId, ProbeRequest>,
    evidence: BTreeMap<PeerId, Evidence>,
}

impl Prober {
    /// A prober for `peers`, each due at `now`.
    pub fn new(
        me: PeerId,
        key: SigningKey,
        scope: String,
        config: ProbeConfig,
        peers: BTreeMap<PeerId, VerifyingKey>,
        now: u64,
    ) -> Result<Self, ConfigError> {
        if peers.contains_key(&me) {
            return Err(ConfigError::SelfProbe);
        }
        if config.timeout == 0 || config.ttl == 0 {
            return Err(ConfigError::ZeroDuration);
        }
        let next_due = peers.keys().map(|p| (p.clone(), now)).collect();
        Ok(Self {
            me,
            key,
            scope,
            config,
            enabled: true,
            peers,
            next_due,
            in_flight: BTreeMap::new(),
            evidence: BTreeMap::new(),
        })
    }

    /// Configured peers due a probe at `now`, with none in flight.
    pub fn due(&self, now: u64) -> Vec<PeerId> {
        if !self.enabled {
            return Vec::new();
        }
        self.next_due
            .iter()
            .filter(|(peer, at)| **at <= now && !self.in_flight.contains_key(*peer))
            .map(|(peer, _)| peer.clone())
            .collect()
    }

    /// Sign and record a probe to `peer` with a fresh `nonce`.
    pub fn start(
        &mut self,
        peer: &str,
        nonce: Nonce,
        now: u64,
    ) -> Result<ProbeRequest, ProbeError> {
        if !self.enabled {
            return Err(ProbeError::Disabled);
        }
        if !self.peers.contains_key(peer) {
            return Err(ProbeError::UnknownPeer);
        }
        if self.in_flight.contains_key(peer) {
            return Err(ProbeError::InFlight);
        }
        let mut request = ProbeRequest {
            prober: self.me.clone(),
            target: peer.to_owned(),
            scope: self.scope.clone(),
            nonce,
            sent_at: now,
            expires_at: now.saturating_add(self.config.timeout),
            signature: [0; 64],
        };
        request.signature = self.key.sign(&request.preimage()).to_bytes();
        let mut head = [0; 8];
        head.copy_from_slice(&nonce[..8]);
        let jitter = u64::from_be_bytes(head) % self.config.jitter.saturating_add(1);
        self.next_due.insert(
            peer.to_owned(),
            now.saturating_add(self.config.interval)
                .saturating_add(jitter),
        );
        self.in_flight.insert(peer.to_owned(), request.clone());
        Ok(request)
    }

    /// Accept a response, marking its responder reachable.
    pub fn receive(&mut self, response: &ProbeResponse, now: u64) -> Result<(), ProbeError> {
        let request = self
            .in_flight
            .get(&response.responder)
            .ok_or(ProbeError::Unsolicited)?;
        if response.prober != self.me {
            return Err(ProbeError::WrongProber);
        }
        if response.scope != self.scope {
            return Err(ProbeError::WrongScope);
        }
        if response.nonce != request.nonce {
            return Err(ProbeError::WrongNonce);
        }
        if response.request_digest != request.digest() {
            return Err(ProbeError::WrongRequest);
        }
        if now >= request.expires_at {
            return Err(ProbeError::Late);
        }
        if response.responded_at < request.sent_at || response.responded_at > now {
            return Err(ProbeError::BadTime);
        }
        let key = self
            .peers
            .get(&response.responder)
            .ok_or(ProbeError::Unsolicited)?;
        if !verify(key, &response.preimage(), &response.signature) {
            return Err(ProbeError::BadSignature);
        }
        self.in_flight.remove(&response.responder);
        self.record(&response.responder, LinkStatus::Reachable, now);
        Ok(())
    }

    fn record(&mut self, peer: &str, status: LinkStatus, at: u64) {
        self.evidence.insert(
            peer.to_owned(),
            Evidence {
                status,
                observed_at: at,
                expires_at: at.saturating_add(self.config.ttl),
            },
        );
    }

    /// Mark every probe that timed out by `now` unreachable; returns the
    /// peers, in order.
    pub fn expire(&mut self, now: u64) -> Vec<PeerId> {
        let late: Vec<(PeerId, u64)> = self
            .in_flight
            .iter()
            .filter(|(_, r)| now >= r.expires_at)
            .map(|(p, r)| (p.clone(), r.expires_at))
            .collect();
        for (peer, at) in &late {
            self.in_flight.remove(peer);
            self.record(peer, LinkStatus::Unreachable, *at);
        }
        late.into_iter().map(|(p, _)| p).collect()
    }

    /// Current evidence about `peer`, if any.
    pub fn evidence(&self, peer: &str, now: u64) -> Option<Evidence> {
        self.evidence
            .get(peer)
            .copied()
            .filter(|e| now < e.expires_at)
    }

    /// This node's own observation from current evidence, or `None` when
    /// it has none.
    pub fn observation(&self, now: u64) -> Option<RouteObservation> {
        let current: Vec<(&PeerId, &Evidence)> = self
            .evidence
            .iter()
            .filter(|(_, e)| now < e.expires_at)
            .collect();
        let expires_at = current.iter().map(|(_, e)| e.expires_at).min()?;
        Some(RouteObservation {
            observer: self.me.clone(),
            scope: self.scope.clone(),
            observed_at: now,
            expires_at,
            links: current
                .into_iter()
                .map(|(peer, e)| (peer.clone(), e.status))
                .collect(),
        })
    }

    /// Stop probing and drop all probe-derived state; configured peers
    /// and keys stay.
    pub fn disable(&mut self) {
        self.enabled = false;
        self.in_flight.clear();
        self.evidence.clear();
    }

    /// Resume probing; every peer is due at `now`.
    pub fn enable(&mut self, now: u64) {
        self.enabled = true;
        for at in self.next_due.values_mut() {
            *at = now;
        }
    }

    /// Whether a probe to `peer` is in flight.
    pub fn in_flight(&self, peer: &str) -> bool {
        self.in_flight.contains_key(peer)
    }
}
