//! B22: tell the parties to a request about duplicates, and report a
//! loop only with the signed path that proves it (#10230, statements
//! S1.65, S1.66 and S1.69; design "Duplicate/replay/cycle state").
//!
//! A peer that receives a federated query runs it through a
//! [`LoopInformer`], which checks the B17 origin query and grant chain
//! first and then decides one of four things:
//!
//! - **Run** it: the first valid arrival at this peer.
//! - **Duplicate**: a valid repeat of a request this peer already has,
//!   in flight or answered, such as the same query arriving directly
//!   and through a relay (S1.64, normal and not an error). It is not
//!   run again (B20, B21), and the peer issues a signed duplicate
//!   [`Notice`] so the requester and the peer that sent it learn that
//!   it was not silently swallowed (S1.66). A completed duplicate's
//!   notice names the cached outcome.
//! - **Loop**: the verified grant chain shows that this peer already
//!   signed a grant for this request, so the query came back round.
//!   The loop notice carries that whole chain as evidence (S1.69).
//!   Repeating a request id, arriving twice, or a path where some
//!   other peer appears twice is never a loop at this peer; only its
//!   own signature earlier in the path is.
//! - **Refused**: the query or chain does not verify, the chain was not
//!   granted to this peer, the last grant is this peer granting itself,
//!   or B20 refuses the identity (replay, binding conflict, expiry, a
//!   full ledger). A refused arrival issues no notice, so an unverified
//!   path can never produce a loop report.
//!
//! Notices go to the origin and to the peer that delivered the request
//! (the last grant's relay; the origin itself for a direct arrival).
//! Each is signed by the reporting peer. [`verify_notice`] lets any
//! recipient check one: the signature, and for a loop, that the
//! evidence chain verified when it was observed, that it was granted
//! to the reporter by someone else, and that the reporter appears in
//! its path. A duplicate notice is the reporter's signed statement and
//! carries no evidence, since a repeat leaves no third-party proof.
//!
//! Rollback: [`LoopInformer::set_inform_duplicates`] turns duplicate
//! notices off. Duplicates are still recognized and never rerun, and
//! loops are still detected and reported.
//!
//! Notice preimage, with integers big-endian:
//!
//! ```text
//! "id-sync/b22-notice/v1" request_id origin reporter
//! sender (0x00, or 0x01 and a 32-byte peer)
//! kind (0x01 duplicate in flight; 0x02 duplicate completed and a
//!       32-byte outcome; 0x03 loop)
//! observed_at:u64 evidence_len:u32 grant digests[]
//! ```
//!
//! This module is deliberately self-contained: the federation roots
//! record slice paths without declaring them (slice 9110-02), so the
//! slice's integration test includes this file directly and no shared
//! root names it yet; #9110-09 wires versioned public exports after
//! module outputs land.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

use super::b17_query_grants::{ChainError, OriginQuery, RelayGrant, verify_chain};
use super::b20_request_identity::{
    Classification, DedupError, DedupLedger, OriginKey, OutcomeId, PeerKey, RequestId,
    RequestIdentity,
};

/// Domain prefix of a notice preimage.
pub const NOTICE_DOMAIN: &[u8] = b"id-sync/b22-notice/v1";

/// What a notice reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeKind {
    /// A repeat of a request this peer is still working on.
    DuplicateInFlight,
    /// A repeat of a request this peer already answered.
    DuplicateCompleted {
        /// The cached outcome the repeat is answered with.
        outcome: OutcomeId,
    },
    /// The request came back to a peer already on its signed path.
    Loop,
}

/// A reporting peer's signed statement about one arrival.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    /// The request.
    pub request_id: RequestId,
    /// The origin, always a recipient.
    pub origin: OriginKey,
    /// The peer that observed the arrival and signs the notice.
    pub reporter: PeerKey,
    /// The peer that delivered it, if a relay did.
    pub sender: Option<PeerKey>,
    /// What happened.
    pub kind: NoticeKind,
    /// When the reporter observed it, in its own clock.
    pub observed_at: u64,
    /// For a loop, the signed chain that arrived; empty otherwise.
    pub evidence: Vec<RelayGrant>,
    /// The reporter's signature.
    pub signature: [u8; 64],
}

impl Notice {
    /// The signed preimage.
    pub fn preimage(&self) -> Vec<u8> {
        let mut out = NOTICE_DOMAIN.to_vec();
        out.extend_from_slice(&self.request_id.0);
        out.extend_from_slice(&self.origin);
        out.extend_from_slice(&self.reporter);
        match self.sender {
            None => out.push(0),
            Some(sender) => {
                out.push(1);
                out.extend_from_slice(&sender);
            }
        }
        match self.kind {
            NoticeKind::DuplicateInFlight => out.push(1),
            NoticeKind::DuplicateCompleted { outcome } => {
                out.push(2);
                out.extend_from_slice(&outcome);
            }
            NoticeKind::Loop => out.push(3),
        }
        out.extend_from_slice(&self.observed_at.to_be_bytes());
        out.extend_from_slice(
            &u32::try_from(self.evidence.len())
                .unwrap_or(u32::MAX)
                .to_be_bytes(),
        );
        for grant in &self.evidence {
            out.extend_from_slice(&grant.digest());
        }
        out
    }

    /// Sign (or re-sign) the notice's current fields.
    pub fn sign(&mut self, key: &SigningKey) {
        self.signature = key.sign(&self.preimage()).to_bytes();
    }

    /// Who the notice goes to: the origin, then the sender if it is
    /// another peer.
    pub fn recipients(&self) -> Vec<PeerKey> {
        let mut out = vec![self.origin];
        if let Some(sender) = self.sender
            && sender != self.origin
        {
            out.push(sender);
        }
        out
    }
}

/// Why an arrival was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The origin query or the grant chain did not verify.
    Chain(ChainError),
    /// The chain was granted to another peer, or a direct arrival
    /// reached a peer the origin did not address.
    NotAddressed,
    /// The last grant is this peer granting itself.
    SelfGrant,
    /// B20: the origin nonce already minted another request.
    Replay,
    /// B20: the binding differs from the one registered under this id.
    BindingConflict,
    /// B20: the request has expired.
    Expired,
    /// B20: the ledger is full of live requests.
    LedgerFull,
}

/// What this peer does with one arrival.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arrival {
    /// First valid arrival: run it.
    Run,
    /// A repeat: do not run it again. `notice` is `None` only while
    /// duplicate notices are turned off.
    Duplicate {
        /// The cached outcome, when the request was already answered.
        completed: Option<OutcomeId>,
        /// The notice for the origin and the sender.
        notice: Option<Notice>,
    },
    /// The request came back round: do not run or forward it.
    Loop {
        /// The notice, with the chain that proves it.
        notice: Notice,
    },
    /// Not accepted, and nothing is reported.
    Refused(Refusal),
}

/// One peer's arrival handling for federated queries.
#[derive(Debug)]
pub struct LoopInformer {
    key: SigningKey,
    ledger: DedupLedger,
    inform_duplicates: bool,
}

impl LoopInformer {
    /// A peer signing with `key`, keeping at most `capacity` requests
    /// in its B20 ledger, each for `cleanup_grace_secs` past expiry.
    pub fn new(key: SigningKey, capacity: usize, cleanup_grace_secs: u64) -> Self {
        Self {
            key,
            ledger: DedupLedger::new(capacity, cleanup_grace_secs),
            inform_duplicates: true,
        }
    }

    /// This peer's id.
    pub fn peer(&self) -> PeerKey {
        self.key.verifying_key().to_bytes()
    }

    /// Turn duplicate notices on or off (the rollback switch).
    pub const fn set_inform_duplicates(&mut self, on: bool) {
        self.inform_duplicates = on;
    }

    /// Handle `query` arriving with `grants` at `now`. With no grants,
    /// the origin sent it directly, and `addressed` says whether the
    /// origin meant this peer.
    pub fn arrive(
        &mut self,
        query: &OriginQuery,
        grants: &[RelayGrant],
        addressed: bool,
        now: u64,
    ) -> Arrival {
        let path = match verify_chain(query, grants, now) {
            Ok(path) => path,
            Err(error) => return Arrival::Refused(Refusal::Chain(error)),
        };
        let local = self.peer();
        match path.holder {
            Some(holder) if holder != local => return Arrival::Refused(Refusal::NotAddressed),
            None if !addressed => return Arrival::Refused(Refusal::NotAddressed),
            _ => {}
        }
        let sender = grants.last().map(|g| g.relay_id);
        if sender == Some(local) {
            return Arrival::Refused(Refusal::SelfGrant);
        }
        let identity = RequestIdentity::new(query.origin, query.nonce, query.binding.clone());
        if path.visits(&local) {
            let notice = self.notice(query, sender, NoticeKind::Loop, now, grants.to_vec());
            return Arrival::Loop { notice };
        }
        let kind = match self.ledger.observe(&identity, now) {
            Classification::New => return Arrival::Run,
            Classification::DuplicateInFlight => NoticeKind::DuplicateInFlight,
            Classification::DuplicateCompleted => match self.ledger.cached_outcome(&identity.id) {
                Some(outcome) => NoticeKind::DuplicateCompleted { outcome },
                None => NoticeKind::DuplicateInFlight,
            },
            Classification::Replay => return Arrival::Refused(Refusal::Replay),
            Classification::BindingConflict => {
                return Arrival::Refused(Refusal::BindingConflict);
            }
            Classification::Expired | Classification::Cycle => {
                return Arrival::Refused(Refusal::Expired);
            }
            Classification::LedgerFull => return Arrival::Refused(Refusal::LedgerFull),
        };
        let completed = match kind {
            NoticeKind::DuplicateCompleted { outcome } => Some(outcome),
            NoticeKind::DuplicateInFlight | NoticeKind::Loop => None,
        };
        let notice = self
            .inform_duplicates
            .then(|| self.notice(query, sender, kind, now, Vec::new()));
        Arrival::Duplicate { completed, notice }
    }

    /// Record the outcome of a request this peer ran (B20).
    pub fn complete(&mut self, query: &OriginQuery, outcome: OutcomeId) -> Result<(), DedupError> {
        let identity = RequestIdentity::new(query.origin, query.nonce, query.binding.clone());
        self.ledger.complete(&identity, outcome)
    }

    fn notice(
        &self,
        query: &OriginQuery,
        sender: Option<PeerKey>,
        kind: NoticeKind,
        now: u64,
        evidence: Vec<RelayGrant>,
    ) -> Notice {
        let mut notice = Notice {
            request_id: query.request_id,
            origin: query.origin,
            reporter: self.peer(),
            sender,
            kind,
            observed_at: now,
            evidence,
            signature: [0; 64],
        };
        notice.sign(&self.key);
        notice
    }
}

/// Why a notice was refused by a recipient.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeError {
    /// The notice is about another request or origin.
    WrongRequest,
    /// The reporter's signature does not verify.
    BadSignature,
    /// A duplicate notice carries evidence, or a loop notice has none.
    EvidenceShape,
    /// The loop evidence did not verify when it was observed.
    Evidence(ChainError),
    /// The loop evidence was granted to someone else, or the sender
    /// named is not the last grant's relay.
    NotTheReporter,
    /// The last grant of the evidence is the reporter granting itself.
    SelfGrant,
    /// The evidence path does not show the reporter already visited.
    NoSelfVisit,
}

/// Check a notice about `query` as a recipient would.
pub fn verify_notice(query: &OriginQuery, notice: &Notice) -> Result<(), NoticeError> {
    if notice.request_id != query.request_id || notice.origin != query.origin {
        return Err(NoticeError::WrongRequest);
    }
    let verified = VerifyingKey::from_bytes(&notice.reporter).is_ok_and(|key| {
        key.verify_strict(
            &notice.preimage(),
            &Signature::from_bytes(&notice.signature),
        )
        .is_ok()
    });
    if !verified {
        return Err(NoticeError::BadSignature);
    }
    let is_loop = notice.kind == NoticeKind::Loop;
    if is_loop == notice.evidence.is_empty() {
        return Err(NoticeError::EvidenceShape);
    }
    if !is_loop {
        return Ok(());
    }
    let path =
        verify_chain(query, &notice.evidence, notice.observed_at).map_err(NoticeError::Evidence)?;
    let last = notice.evidence.last().map(|g| g.relay_id);
    if path.holder != Some(notice.reporter) || notice.sender != last {
        return Err(NoticeError::NotTheReporter);
    }
    if last == Some(notice.reporter) {
        return Err(NoticeError::SelfGrant);
    }
    if !path.visits(&notice.reporter) {
        return Err(NoticeError::NoSelfVisit);
    }
    Ok(())
}
