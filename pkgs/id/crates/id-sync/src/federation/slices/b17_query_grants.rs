//! B17: signed queries, relay grants and end-to-end responder
//! provenance (#10249, statements S1.58-S1.62; Part B "Query
//! contract", fan-out design #9144 sections 5 and 6).
//!
//! Three signed artifacts carry a federated query and its answer, and
//! every hop checks all of them before it uses or forwards anything
//! (S1.58):
//!
//! - [`OriginQuery`] is signed once by the origin and never changes. Its
//!   request id is the B20 id derived from the origin key, a fresh nonce
//!   and the immutable binding, so it is the same on every copy and
//!   every relay (S1.62), and a verifier recomputes it.
//! - [`RelayGrant`] is signed by the relay that forwards the query to
//!   one child. It names the origin query by digest and its parent grant
//!   by digest, so the grants form a chain from the first relay down;
//!   it carries the path of relays so far and a hop index, and the chain
//!   is refused once the origin's hop bound would be exceeded (S1.56,
//!   S1.57). A grant can only narrow the expiry it inherits.
//! - [`SignedResponse`] is signed by the peer that answered. Relays pass
//!   its bytes and signature back unchanged and add a
//!   [`ForwardingReceipt`] each, signed by the relay, naming the grant it
//!   issued and the response's digest. The requester checks the
//!   responder's signature directly, with the same rules as a direct
//!   response (S1.60), and checks that the responder is the peer the
//!   chain granted last, so no hop can answer in the responder's name
//!   (S1.59). It trusts the relays only for the path, which the
//!   receipts and grants prove.
//!
//! Peer, origin and responder identities are Ed25519 verifying keys, as
//! in B20, so a signature under a peer's id is that peer's signature.
//! Bounding breadth and total work (S1.61) is B16's budget vector; this
//! slice bounds depth. Whether a path that revisits a peer is a loop is
//! B22's call: this slice verifies the signed path, it does not judge
//! it.
//!
//! The digest of any signed artifact here is SHA-256 of its preimage
//! followed by its signature. Preimages, with integers big-endian:
//!
//! ```text
//! OriginQuery       "id-sync/b17-origin-query/v1" request_id origin nonce
//!                   binding (B20 canonical bytes)
//! RelayGrant        "id-sync/b17-relay-grant/v1" request_id origin_query_digest
//!                   parent (0x00, or 0x01 and a 32-byte grant digest)
//!                   relay_id child_id hop_index:u32 path_len:u32 path[]
//!                   expires_at:u64
//! SignedResponse    "id-sync/b17-response/v1" request_id responder_id
//!                   scope_len:u32 scope policy_digest completeness:u8
//!                   result_count:u32 (result_len:u32 result)* generated_at:u64
//! ForwardingReceipt "id-sync/b17-forwarding-receipt/v1" request_id relay_id
//!                   grant_digest response_digest forwarded_at:u64
//! ```
//!
//! Completeness is 0 (unstated), 1 (partial) or 2 (complete).
//!
//! Rollback is disabling relay forwarding: origin-query validation and
//! direct responses keep working without any grant.
//!
//! This module is deliberately self-contained: the federation roots
//! record slice paths without declaring them (slice 9110-02), so the
//! slice's integration test includes this file directly and no shared
//! root names it yet; #9110-09 wires versioned public exports after
//! module outputs land.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};

use super::b20_request_identity::{OriginKey, OriginNonce, PeerKey, QueryBinding, RequestId};

/// The most hops any origin may ask for.
pub const HOP_CEILING: u32 = 8;

/// Domain prefix of an origin query preimage.
pub const ORIGIN_QUERY_DOMAIN: &[u8] = b"id-sync/b17-origin-query/v1";
/// Domain prefix of a relay grant preimage.
pub const RELAY_GRANT_DOMAIN: &[u8] = b"id-sync/b17-relay-grant/v1";
/// Domain prefix of a response preimage.
pub const RESPONSE_DOMAIN: &[u8] = b"id-sync/b17-response/v1";
/// Domain prefix of a forwarding receipt preimage.
pub const RECEIPT_DOMAIN: &[u8] = b"id-sync/b17-forwarding-receipt/v1";

/// A SHA-256 digest.
pub type Digest32 = [u8; 32];
/// An Ed25519 signature.
pub type SignatureBytes = [u8; 64];

fn digest_of(preimage: &[u8], signature: &SignatureBytes) -> Digest32 {
    let mut hasher = Sha256::new();
    hasher.update(preimage);
    hasher.update(signature);
    hasher.finalize().into()
}

fn sign(key: &SigningKey, preimage: &[u8]) -> SignatureBytes {
    key.sign(preimage).to_bytes()
}

fn verifies(signer: &[u8; 32], preimage: &[u8], signature: &SignatureBytes) -> bool {
    VerifyingKey::from_bytes(signer).is_ok_and(|key| {
        key.verify_strict(preimage, &Signature::from_bytes(signature))
            .is_ok()
    })
}

fn push_len(out: &mut Vec<u8>, len: usize) {
    out.extend_from_slice(&u32::try_from(len).unwrap_or(u32::MAX).to_be_bytes());
}

/// The origin's signed, immutable query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OriginQuery {
    /// The B20 request id, derived from the fields below.
    pub request_id: RequestId,
    /// The origin's key.
    pub origin: OriginKey,
    /// The origin's fresh nonce.
    pub nonce: OriginNonce,
    /// The immutable binding (query digest, scope, times, hop bound).
    pub binding: QueryBinding,
    /// The origin's signature over the preimage.
    pub signature: SignatureBytes,
}

/// Why an origin query was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryError {
    /// The request id does not derive from the origin, nonce and binding.
    RequestIdMismatch,
    /// The origin's signature does not verify.
    BadSignature,
    /// The hop bound is zero or above [`HOP_CEILING`].
    HopBound,
    /// The query has expired.
    Expired,
}

impl OriginQuery {
    /// Sign a new query as the origin.
    pub fn sign(key: &SigningKey, nonce: OriginNonce, binding: QueryBinding) -> Self {
        let origin = key.verifying_key().to_bytes();
        let mut query = Self {
            request_id: RequestId::derive(&origin, &nonce, &binding),
            origin,
            nonce,
            binding,
            signature: [0; 64],
        };
        query.signature = sign(key, &query.preimage());
        query
    }

    /// The signed preimage.
    pub fn preimage(&self) -> Vec<u8> {
        let mut out = ORIGIN_QUERY_DOMAIN.to_vec();
        out.extend_from_slice(&self.request_id.0);
        out.extend_from_slice(&self.origin);
        out.extend_from_slice(&self.nonce);
        out.extend_from_slice(&self.binding.canonical_bytes());
        out
    }

    /// SHA-256 of the preimage and signature.
    pub fn digest(&self) -> Digest32 {
        digest_of(&self.preimage(), &self.signature)
    }

    /// Check the query at `now`: id, signature, hop bound, expiry.
    pub fn verify(&self, now: u64) -> Result<(), QueryError> {
        if RequestId::derive(&self.origin, &self.nonce, &self.binding) != self.request_id {
            return Err(QueryError::RequestIdMismatch);
        }
        if !verifies(&self.origin, &self.preimage(), &self.signature) {
            return Err(QueryError::BadSignature);
        }
        if self.binding.initial_max_hops == 0 || self.binding.initial_max_hops > HOP_CEILING {
            return Err(QueryError::HopBound);
        }
        if now >= self.binding.expires_at {
            return Err(QueryError::Expired);
        }
        Ok(())
    }
}

/// One relay's signed permission for one child to work on the query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayGrant {
    /// The query's request id.
    pub request_id: RequestId,
    /// The digest of the origin query.
    pub origin_query_digest: Digest32,
    /// The digest of the grant this relay holds, if it was itself relayed.
    pub parent_grant_digest: Option<Digest32>,
    /// The relay that signs this grant.
    pub relay_id: PeerKey,
    /// The peer it grants.
    pub child_id: PeerKey,
    /// How many relays the path holds, this one included.
    pub hop_index: u32,
    /// The relays from the first one to this one.
    pub path: Vec<PeerKey>,
    /// When the grant expires; never later than what it inherits.
    pub expires_at: u64,
    /// The relay's signature.
    pub signature: SignatureBytes,
}

impl RelayGrant {
    /// Grant `child` under `query`, as the relay holding `parent` (or as
    /// the origin's direct target when `parent` is `None`). The fields
    /// follow the chain rules; [`verify_chain`] still decides.
    pub fn issue(
        key: &SigningKey,
        query: &OriginQuery,
        parent: Option<&Self>,
        child: PeerKey,
        expires_at: u64,
    ) -> Self {
        let relay_id = key.verifying_key().to_bytes();
        let mut path = parent.map(|p| p.path.clone()).unwrap_or_default();
        path.push(relay_id);
        let mut grant = Self {
            request_id: query.request_id,
            origin_query_digest: query.digest(),
            parent_grant_digest: parent.map(Self::digest),
            relay_id,
            child_id: child,
            hop_index: u32::try_from(path.len()).unwrap_or(u32::MAX),
            path,
            expires_at,
            signature: [0; 64],
        };
        grant.sign(key);
        grant
    }

    /// Sign (or re-sign) the grant's current fields.
    pub fn sign(&mut self, key: &SigningKey) {
        self.signature = sign(key, &self.preimage());
    }

    /// The signed preimage.
    pub fn preimage(&self) -> Vec<u8> {
        let mut out = RELAY_GRANT_DOMAIN.to_vec();
        out.extend_from_slice(&self.request_id.0);
        out.extend_from_slice(&self.origin_query_digest);
        match self.parent_grant_digest {
            None => out.push(0),
            Some(parent) => {
                out.push(1);
                out.extend_from_slice(&parent);
            }
        }
        out.extend_from_slice(&self.relay_id);
        out.extend_from_slice(&self.child_id);
        out.extend_from_slice(&self.hop_index.to_be_bytes());
        push_len(&mut out, self.path.len());
        for peer in &self.path {
            out.extend_from_slice(peer);
        }
        out.extend_from_slice(&self.expires_at.to_be_bytes());
        out
    }

    /// SHA-256 of the preimage and signature.
    pub fn digest(&self) -> Digest32 {
        digest_of(&self.preimage(), &self.signature)
    }
}

/// Why a grant chain was refused. `index` is the grant's position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainError {
    /// The origin query itself was refused.
    Query(QueryError),
    /// The grant is for another request.
    WrongRequest {
        /// The grant's position.
        index: usize,
    },
    /// The grant names another origin query.
    WrongOriginQuery {
        /// The grant's position.
        index: usize,
    },
    /// The grant does not name the previous grant as its parent.
    BrokenLink {
        /// The grant's position.
        index: usize,
    },
    /// The grant's signer is not the child the previous grant named.
    RelayNotGranted {
        /// The grant's position.
        index: usize,
    },
    /// The hop index is not the grant's depth in the chain.
    HopIndex {
        /// The grant's position.
        index: usize,
    },
    /// The path is not the previous path plus this relay.
    PathMismatch {
        /// The grant's position.
        index: usize,
    },
    /// The child would sit beyond the origin's hop bound.
    HopBoundExceeded {
        /// The grant's position.
        index: usize,
    },
    /// The grant expires later than what it inherits.
    ExpiryWidened {
        /// The grant's position.
        index: usize,
    },
    /// The grant has expired.
    Expired {
        /// The grant's position.
        index: usize,
    },
    /// The relay's signature does not verify.
    BadSignature {
        /// The grant's position.
        index: usize,
    },
}

/// A verified chain: the relays in order and the peer holding the last
/// grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantPath {
    /// The relays, first to last.
    pub relays: Vec<PeerKey>,
    /// The child of the last grant, if there is any grant.
    pub holder: Option<PeerKey>,
    /// Hops left for the holder to grant onward.
    pub remaining_hops: u32,
}

impl GrantPath {
    /// Whether `peer` signed a grant on this path.
    pub fn visits(&self, peer: &PeerKey) -> bool {
        self.relays.contains(peer)
    }
}

/// Verify the origin query and a grant chain at `now`.
pub fn verify_chain(
    query: &OriginQuery,
    grants: &[RelayGrant],
    now: u64,
) -> Result<GrantPath, ChainError> {
    query.verify(now).map_err(ChainError::Query)?;
    let query_digest = query.digest();
    let max_hops = query.binding.initial_max_hops;
    let mut previous: Option<&RelayGrant> = None;
    for (index, grant) in grants.iter().enumerate() {
        if grant.request_id != query.request_id {
            return Err(ChainError::WrongRequest { index });
        }
        if grant.origin_query_digest != query_digest {
            return Err(ChainError::WrongOriginQuery { index });
        }
        if grant.parent_grant_digest != previous.map(RelayGrant::digest) {
            return Err(ChainError::BrokenLink { index });
        }
        if previous.is_some_and(|p| p.child_id != grant.relay_id) {
            return Err(ChainError::RelayNotGranted { index });
        }
        if usize::try_from(grant.hop_index).ok() != Some(index + 1) {
            return Err(ChainError::HopIndex { index });
        }
        let expected_path = previous
            .map(|p| p.path.as_slice())
            .unwrap_or_default()
            .iter()
            .chain(std::iter::once(&grant.relay_id));
        if !grant.path.iter().eq(expected_path) {
            return Err(ChainError::PathMismatch { index });
        }
        if grant.hop_index >= max_hops {
            return Err(ChainError::HopBoundExceeded { index });
        }
        let inherited = previous.map_or(query.binding.expires_at, |p| p.expires_at);
        if grant.expires_at > inherited {
            return Err(ChainError::ExpiryWidened { index });
        }
        if now >= grant.expires_at {
            return Err(ChainError::Expired { index });
        }
        if !verifies(&grant.relay_id, &grant.preimage(), &grant.signature) {
            return Err(ChainError::BadSignature { index });
        }
        previous = Some(grant);
    }
    let depth = u32::try_from(grants.len()).unwrap_or(u32::MAX);
    Ok(GrantPath {
        relays: grants.iter().map(|g| g.relay_id).collect(),
        holder: grants.last().map(|g| g.child_id),
        remaining_hops: max_hops.saturating_sub(depth.saturating_add(1)),
    })
}

/// How much of its scope a response claims to cover.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Completeness {
    /// No claim; absence of a result means nothing.
    Unstated,
    /// Explicitly partial.
    Partial,
    /// Explicitly complete for the stated scope and policy.
    Complete,
}

impl Completeness {
    const fn tag(self) -> u8 {
        match self {
            Self::Unstated => 0,
            Self::Partial => 1,
            Self::Complete => 2,
        }
    }
}

/// A responder's signed answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedResponse {
    /// The query's request id.
    pub request_id: RequestId,
    /// The peer that answered.
    pub responder_id: PeerKey,
    /// The scope it answered under.
    pub scope: Vec<u8>,
    /// The digest of the policy it answered under.
    pub policy_digest: Digest32,
    /// Its completeness claim.
    pub completeness: Completeness,
    /// The results, as opaque bytes.
    pub results: Vec<Vec<u8>>,
    /// When it answered, in its own clock.
    pub generated_at: u64,
    /// The responder's signature.
    pub signature: SignatureBytes,
}

impl SignedResponse {
    /// Sign (or re-sign) the response's current fields.
    pub fn sign(&mut self, key: &SigningKey) {
        self.signature = sign(key, &self.preimage());
    }

    /// The signed preimage.
    pub fn preimage(&self) -> Vec<u8> {
        let mut out = RESPONSE_DOMAIN.to_vec();
        out.extend_from_slice(&self.request_id.0);
        out.extend_from_slice(&self.responder_id);
        push_len(&mut out, self.scope.len());
        out.extend_from_slice(&self.scope);
        out.extend_from_slice(&self.policy_digest);
        out.push(self.completeness.tag());
        push_len(&mut out, self.results.len());
        for result in &self.results {
            push_len(&mut out, result.len());
            out.extend_from_slice(result);
        }
        out.extend_from_slice(&self.generated_at.to_be_bytes());
        out
    }

    /// SHA-256 of the preimage and signature.
    pub fn digest(&self) -> Digest32 {
        digest_of(&self.preimage(), &self.signature)
    }
}

/// A relay's signed statement that it forwarded one response it got
/// under the grant it issued.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardingReceipt {
    /// The query's request id.
    pub request_id: RequestId,
    /// The forwarding relay.
    pub relay_id: PeerKey,
    /// The digest of the grant the relay issued toward the responder.
    pub grant_digest: Digest32,
    /// The digest of the response forwarded, unchanged.
    pub response_digest: Digest32,
    /// When it forwarded, in its own clock.
    pub forwarded_at: u64,
    /// The relay's signature.
    pub signature: SignatureBytes,
}

impl ForwardingReceipt {
    /// Sign a receipt for forwarding `response`, received under `grant`.
    pub fn issue(
        key: &SigningKey,
        grant: &RelayGrant,
        response: &SignedResponse,
        forwarded_at: u64,
    ) -> Self {
        let mut receipt = Self {
            request_id: grant.request_id,
            relay_id: key.verifying_key().to_bytes(),
            grant_digest: grant.digest(),
            response_digest: response.digest(),
            forwarded_at,
            signature: [0; 64],
        };
        receipt.sign(key);
        receipt
    }

    /// Sign (or re-sign) the receipt's current fields.
    pub fn sign(&mut self, key: &SigningKey) {
        self.signature = sign(key, &self.preimage());
    }

    /// The signed preimage.
    pub fn preimage(&self) -> Vec<u8> {
        let mut out = RECEIPT_DOMAIN.to_vec();
        out.extend_from_slice(&self.request_id.0);
        out.extend_from_slice(&self.relay_id);
        out.extend_from_slice(&self.grant_digest);
        out.extend_from_slice(&self.response_digest);
        out.extend_from_slice(&self.forwarded_at.to_be_bytes());
        out
    }

    /// SHA-256 of the preimage and signature.
    pub fn digest(&self) -> Digest32 {
        digest_of(&self.preimage(), &self.signature)
    }
}

/// A response as it reaches the requester: the responder's bytes, the
/// grants down to the responder, and one receipt per grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayedResponse {
    /// The responder's signed answer, unchanged.
    pub response: SignedResponse,
    /// The grants from the first relay to the responder.
    pub grants: Vec<RelayGrant>,
    /// One receipt per grant, in the same order.
    pub receipts: Vec<ForwardingReceipt>,
}

/// Why a relayed response was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseError {
    /// The query or the grant chain was refused.
    Chain(ChainError),
    /// The first grant was not signed by the peer the requester asked.
    NotTheAskedPeer,
    /// The responder is not the peer the chain granted last (or, with
    /// no grant, the peer asked).
    ResponderMismatch,
    /// The response is for another request.
    WrongRequest,
    /// The response answers another scope.
    ScopeMismatch,
    /// The responder's signature does not verify.
    BadResponderSignature,
    /// There is not exactly one receipt per grant.
    ReceiptCount,
    /// A receipt names another relay, grant, request or response.
    ReceiptMismatch {
        /// The receipt's position.
        index: usize,
    },
    /// A receipt's signature does not verify.
    BadReceiptSignature {
        /// The receipt's position.
        index: usize,
    },
}

/// A response the requester can use, with the path that carried it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedResponse {
    /// The peer that answered and signed.
    pub responder: PeerKey,
    /// The relays the response passed through, first to last.
    pub relays: Vec<PeerKey>,
    /// The digest of the responder's unchanged response.
    pub response_digest: Digest32,
}

/// Verify a response received after asking `asked`, to the same
/// standard whether it came directly or through relays.
pub fn verify_response(
    query: &OriginQuery,
    asked: &PeerKey,
    relayed: &RelayedResponse,
    now: u64,
) -> Result<VerifiedResponse, ResponseError> {
    let path = verify_chain(query, &relayed.grants, now).map_err(ResponseError::Chain)?;
    if relayed.grants.first().is_some_and(|g| g.relay_id != *asked) {
        return Err(ResponseError::NotTheAskedPeer);
    }
    let response = &relayed.response;
    if response.responder_id != path.holder.unwrap_or(*asked) {
        return Err(ResponseError::ResponderMismatch);
    }
    if response.request_id != query.request_id {
        return Err(ResponseError::WrongRequest);
    }
    if response.scope != query.binding.scope {
        return Err(ResponseError::ScopeMismatch);
    }
    if !verifies(
        &response.responder_id,
        &response.preimage(),
        &response.signature,
    ) {
        return Err(ResponseError::BadResponderSignature);
    }
    if relayed.receipts.len() != relayed.grants.len() {
        return Err(ResponseError::ReceiptCount);
    }
    let response_digest = response.digest();
    for (index, (receipt, grant)) in relayed.receipts.iter().zip(&relayed.grants).enumerate() {
        if receipt.request_id != query.request_id
            || receipt.relay_id != grant.relay_id
            || receipt.grant_digest != grant.digest()
            || receipt.response_digest != response_digest
        {
            return Err(ResponseError::ReceiptMismatch { index });
        }
        if !verifies(&receipt.relay_id, &receipt.preimage(), &receipt.signature) {
            return Err(ResponseError::BadReceiptSignature { index });
        }
    }
    Ok(VerifiedResponse {
        responder: response.responder_id,
        relays: path.relays,
        response_digest,
    })
}
