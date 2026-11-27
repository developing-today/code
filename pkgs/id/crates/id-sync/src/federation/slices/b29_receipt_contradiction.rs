//! B29: ack/recipient-list contradiction detection, evidence retention,
//! collusion threshold and public-log integration (#10266, statements
//! S1.105-S1.108; Part B "V4 acknowledgment and recipient-list
//! evidence").
//!
//! A recipient-list commitment binds a transfer id, the exact recipient
//! set (or a Merkle root plus an inclusion proof), the payload digest
//! and the time; a receiver's `accepted` receipt (B19) independently
//! proves admission. The contradiction evaluator reports a positive
//! finding only when the same object/digest, interval, scope, policy
//! and authorization line up under exactly matching claim semantics
//! (admission completeness here); anything else — a mismatched
//! dimension, a receipt of another assertion kind, or simply no
//! counterpart on record — is `insufficient_evidence`. Withheld
//! evidence is never proof of omission, and a missing result proves
//! nothing.
//!
//! Findings carry their retention policy digest and expiry, so evidence
//! retention is a field on the record rather than an afterthought, and
//! the collusion counter below deduplicates by accountable peer and
//! object inside a finite window before any threshold is consulted. A
//! finding converts to a B27 public-log claim whose reputation delta
//! is zero by construction: the log publishes the contradiction, it
//! never enacts a verdict.
//!
//! This module is deliberately self-contained: the federation roots
//! record slice paths without declaring them (slice 9110-02), so the
//! slice's integration test includes this file directly and no shared
//! root names it yet; #9110-09 wires versioned public exports after
//! module outputs land.

use std::collections::BTreeSet;

use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};

use super::b05_signer_binding::{
    BindingError, BindingRegistry, ED25519_SIGNATURE_LEN, VerifiedIdentity, verify_bound_signature,
};
use super::b19_signed_receipt::{Assertion, Digest32, KeyBytes, VerifiedReceipt};
use super::b27_claim_log::{
    ClaimError, ClaimLog, EventBody, EventId, EventKind, MAX_STATEMENT_BYTES, sign_event,
};
use super::b32_ingest_gate::LogicalTime;

/// A transfer's stable identity.
pub type TransferId = [u8; 32];
/// The authorization (grant chain / policy exception) a commitment
/// claims to act under, as a digest.
pub type AuthorizationDigest = [u8; 32];

/// Domain prefix of a recipient-list commitment's canonical bytes.
pub const COMMITMENT_DOMAIN: &[u8] = b"id-sync/b29-commitment/v1";
/// Domain prefix of a contradiction finding's claim statement.
pub const FINDING_STATEMENT_DOMAIN: &[u8] = b"id-sync/b29-finding/v1";
/// The most recipient keys one exact set may name. Bounds the evidence
/// a single commitment can force every checker to hold.
pub const MAX_RECIPIENTS: usize = 1024;
/// Proposed starting default from the design: this many distinct
/// attributable contradictions for one peer inside [`COLLUSION_WINDOW`]
/// meet the collusion threshold. A starting default, not an externally
/// verified universal weight.
pub const DEFAULT_COLLUSION_THRESHOLD: u32 = 3;
/// The window distinct attributable contradictions must fall in to
/// count toward one threshold decision: ten minutes.
pub const COLLUSION_WINDOW: LogicalTime = 600;

const TAG_EXACT: u8 = 1;
const TAG_ROOT: u8 = 2;

/// A membership witness for a root commitment: the key claimed to be a
/// member, and the sibling hashes from the leaf up to the root.
/// Each pair is the sibling hash and whether that sibling is the left
/// child (`true`) or the right child (`false`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MembershipProof {
    /// The key claimed to be in the committed set.
    pub leaf_key: KeyBytes,
    /// Sibling hashes from the leaf level up, with their side.
    pub path: Vec<(Digest32, bool)>,
}

/// The recipient set a commitment binds: either the exact keys, or a
/// Merkle root the checker resolves through [`MembershipProof`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecipientSet {
    /// Every recipient key, in any order; canonical bytes sort them.
    Exact(Vec<KeyBytes>),
    /// A Merkle root over the set plus one membership witness.
    Root {
        /// The committed set root.
        root: Digest32,
        /// The witness for the key under test.
        proof: MembershipProof,
    },
}

fn leaf_hash(key: &KeyBytes) -> Digest32 {
    let mut hasher = Sha256::new();
    hasher.update([0x00]);
    hasher.update(key);
    hasher.finalize().into()
}

fn node_hash(left: &Digest32, right: &Digest32) -> Digest32 {
    let mut hasher = Sha256::new();
    hasher.update([0x01]);
    hasher.update(left);
    hasher.update(right);
    hasher.finalize().into()
}

fn merkle_root(sorted_keys: &[KeyBytes]) -> Digest32 {
    let mut level: Vec<Digest32> = sorted_keys.iter().map(leaf_hash).collect();
    if level.is_empty() {
        return [0u8; 32];
    }
    while level.len() > 1 {
        let mut next = Vec::with_capacity(level.len().div_ceil(2));
        let mut i = 0;
        while i < level.len() {
            let left = level[i];
            let right = if i + 1 < level.len() {
                level[i + 1]
            } else {
                left
            };
            next.push(node_hash(&left, &right));
            i += 2;
        }
        level = next;
    }
    level[0]
}

fn verify_proof(root: &Digest32, proof: &MembershipProof) -> bool {
    let mut current = leaf_hash(&proof.leaf_key);
    for (sibling, sibling_is_left) in &proof.path {
        current = if *sibling_is_left {
            node_hash(sibling, &current)
        } else {
            node_hash(&current, sibling)
        };
    }
    &current == root
}

fn canonical_keys(keys: &[KeyBytes]) -> Vec<KeyBytes> {
    let mut out = keys.to_vec();
    out.sort_unstable();
    out.dedup();
    out
}

impl RecipientSet {
    /// The canonical root of the committed set: the Merkle root over
    /// the sorted exact keys, or the stored root.
    pub fn root(&self) -> Digest32 {
        match self {
            Self::Exact(keys) => merkle_root(&canonical_keys(keys)),
            Self::Root { root, .. } => *root,
        }
    }

    /// Whether `key` is provably in the committed set: exact
    /// membership, or a valid inclusion proof against the root.
    pub fn contains(&self, key: &KeyBytes) -> bool {
        match self {
            Self::Exact(keys) => canonical_keys(keys).contains(key),
            Self::Root { root, proof } => &proof.leaf_key == key && verify_proof(root, proof),
        }
    }
}

/// The signed fields of a recipient-list commitment: an admission
/// completeness claim. It asserts that exactly the bound set was
/// admitted for the transfer — nothing more, nothing less.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitmentBody {
    /// The transfer this commits the recipient set for.
    pub transfer_id: TransferId,
    /// The payload digest admitted.
    pub payload_digest: Digest32,
    /// The exact set, or a root plus witness.
    pub recipients: RecipientSet,
    /// The scope the admission holds in.
    pub scope: String,
    /// The policy in force at commit time.
    pub policy_digest: Digest32,
    /// The authorization this transfer acts under.
    pub authorization: AuthorizationDigest,
    /// When the commitment was issued (the signer's claim).
    pub issued_at: LogicalTime,
    /// The commitment is valid from this time.
    pub valid_from: LogicalTime,
    /// The commitment is valid through this time.
    pub valid_until: LogicalTime,
    /// The retention policy in force where this evidence is held.
    pub retention_policy: Digest32,
    /// When the holder's retention of this evidence expires.
    pub retention_expires: LogicalTime,
    /// The signer's claimed identity.
    pub signer: String,
    /// The key that signs the commitment.
    pub signer_key: KeyBytes,
}

fn push_field(out: &mut Vec<u8>, field: &[u8]) {
    out.extend_from_slice(&(field.len() as u64).to_be_bytes());
    out.extend_from_slice(field);
}

fn push_proof(out: &mut Vec<u8>, proof: &MembershipProof) {
    out.extend_from_slice(&proof.leaf_key);
    out.extend_from_slice(&(proof.path.len() as u64).to_be_bytes());
    for (sibling, sibling_is_left) in &proof.path {
        out.extend_from_slice(sibling);
        out.push(u8::from(*sibling_is_left));
    }
}

impl CommitmentBody {
    /// The canonical bytes the signature covers.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = COMMITMENT_DOMAIN.to_vec();
        out.extend_from_slice(&self.transfer_id);
        out.extend_from_slice(&self.payload_digest);
        match &self.recipients {
            RecipientSet::Exact(keys) => {
                out.push(TAG_EXACT);
                let canonical = canonical_keys(keys);
                out.extend_from_slice(&(canonical.len() as u64).to_be_bytes());
                for key in &canonical {
                    out.extend_from_slice(key);
                }
            }
            RecipientSet::Root { root, proof } => {
                out.push(TAG_ROOT);
                out.extend_from_slice(root);
                push_proof(&mut out, proof);
            }
        }
        push_field(&mut out, self.scope.as_bytes());
        out.extend_from_slice(&self.policy_digest);
        out.extend_from_slice(&self.authorization);
        out.extend_from_slice(&self.issued_at.to_be_bytes());
        out.extend_from_slice(&self.valid_from.to_be_bytes());
        out.extend_from_slice(&self.valid_until.to_be_bytes());
        out.extend_from_slice(&self.retention_policy);
        out.extend_from_slice(&self.retention_expires.to_be_bytes());
        push_field(&mut out, self.signer.as_bytes());
        out.extend_from_slice(&self.signer_key);
        out
    }
}

/// A commitment and its signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedCommitment {
    /// The signed fields.
    pub body: CommitmentBody,
    /// Ed25519 over [`CommitmentBody::canonical_bytes`].
    pub signature: [u8; ED25519_SIGNATURE_LEN],
}

/// Sign a commitment body. The caller owns key management; the
/// signature covers exactly [`CommitmentBody::canonical_bytes`].
pub fn sign_commitment(body: &CommitmentBody, signing: &SigningKey) -> SignedCommitment {
    SignedCommitment {
        body: body.clone(),
        signature: signing.sign(&body.canonical_bytes()).to_bytes(),
    }
}

/// Why a commitment could not be trusted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitmentError {
    /// The recipient set is empty: a completeness claim over nobody
    /// admits nothing and proves nothing.
    EmptyRecipients,
    /// The recipient set names more keys than [`MAX_RECIPIENTS`].
    TooManyRecipients {
        /// How many keys were named.
        len: usize,
    },
    /// The validity interval is empty.
    InvalidInterval,
    /// The signer's key binding failed: unbound key, unknown identity,
    /// or a key bound to someone else.
    Binding(BindingError),
    /// The key is bound to the signer, but the signature is invalid.
    BadSignature,
    /// The commitment is not yet valid.
    NotYetValid,
    /// The commitment's validity has ended.
    Expired,
    /// A finding statement does not fit the public-log bound.
    StatementTooLarge {
        /// The statement's length in bytes.
        len: usize,
    },
}

impl std::fmt::Display for CommitmentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyRecipients => write!(f, "recipient set is empty"),
            Self::TooManyRecipients { len } => {
                write!(f, "{len} recipients exceed the {MAX_RECIPIENTS} limit")
            }
            Self::InvalidInterval => write!(f, "validity interval is empty"),
            Self::Binding(source) => write!(f, "signer binding failed: {source}"),
            Self::BadSignature => write!(f, "signature is invalid"),
            Self::NotYetValid => write!(f, "commitment is not yet valid"),
            Self::Expired => write!(f, "commitment validity has ended"),
            Self::StatementTooLarge { len } => {
                write!(f, "finding statement of {len} bytes exceeds the log bound")
            }
        }
    }
}

impl std::error::Error for CommitmentError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Binding(source) => Some(source),
            _ => None,
        }
    }
}

impl From<BindingError> for CommitmentError {
    fn from(source: BindingError) -> Self {
        Self::Binding(source)
    }
}

/// A commitment whose signer binding, signature, shape and validity
/// checked out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedCommitment {
    /// The commitment.
    pub commitment: SignedCommitment,
    /// The bound identity that signed it.
    pub identity: VerifiedIdentity,
}

/// Verify a commitment at `now`: the set is non-empty and bounded, the
/// interval is well formed, the signing key is bound to the claimed
/// signer (checked before the signature), the signature covers the
/// canonical bytes, and `now` is inside the validity interval.
pub fn verify_commitment(
    registry: &BindingRegistry,
    commitment: &SignedCommitment,
    now: LogicalTime,
) -> Result<VerifiedCommitment, CommitmentError> {
    match &commitment.body.recipients {
        RecipientSet::Exact(keys) if canonical_keys(keys).is_empty() => {
            return Err(CommitmentError::EmptyRecipients);
        }
        RecipientSet::Exact(keys) if canonical_keys(keys).len() > MAX_RECIPIENTS => {
            return Err(CommitmentError::TooManyRecipients {
                len: canonical_keys(keys).len(),
            });
        }
        _ => {}
    }
    let body = &commitment.body;
    if body.valid_from > body.valid_until {
        return Err(CommitmentError::InvalidInterval);
    }
    let identity = verify_bound_signature(
        registry,
        &body.signer,
        &body.signer_key,
        &body.canonical_bytes(),
        &commitment.signature,
    )
    .map_err(|error| match error {
        BindingError::BadSignature => CommitmentError::BadSignature,
        other => CommitmentError::Binding(other),
    })?;
    if now < body.valid_from {
        return Err(CommitmentError::NotYetValid);
    }
    if now > body.valid_until {
        return Err(CommitmentError::Expired);
    }
    Ok(VerifiedCommitment {
        commitment: commitment.clone(),
        identity,
    })
}

/// One binding dimension a positive finding requires to match. Any
/// mismatch — or any missing counterpart — makes the result
/// `insufficient_evidence`, never a contradiction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MismatchDimension {
    /// Different transfer: a different object. A different payload
    /// digest under the same transfer, with every other binding
    /// matching, is [`ContradictionCase::DivergentCommitments`], not a
    /// mismatch.
    ObjectDigest,
    /// Different scope.
    Scope,
    /// Different policy digest. A receipt's authorization is the B32
    /// gate admission it records, so the receipt side matches on the
    /// policy that admission ran under.
    Policy,
    /// Validity intervals do not overlap: different times.
    Interval,
    /// Different authorization digest (commitment against commitment).
    Authorization,
    /// A receipt of another assertion kind cannot corroborate or
    /// contradict an admission completeness claim: only `accepted`
    /// receipts speak to who was admitted.
    ClaimSemantics,
}

/// Why the evaluator could not report a contradiction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsufficientReason {
    /// Every dimension matched and no conflict was found: the evidence
    /// is consistent, which is itself a checkable outcome.
    Consistent,
    /// One binding dimension did not match.
    Dimension(MismatchDimension),
    /// There was no counterpart to check against: no accepted receipt
    /// on record, or no second commitment. Withheld evidence is not
    /// proof of omission.
    NoCounterpart,
}

/// How two signed assertions can irreconcilably disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContradictionCase {
    /// An exact-set commitment names the full admitted set, but a
    /// verified `accepted` receipt proves admission of the same digest
    /// to a key outside it.
    NonMemberAdmission,
    /// Two verified commitments bind the same transfer under the same
    /// scope, policy, authorization and overlapping interval, but name
    /// different digests or different sets.
    DivergentCommitments,
}

/// A positive contradiction finding. The accountable peer is the
/// identity that signed the commitment the finding holds against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contradiction {
    /// How the assertions disagree.
    pub case: ContradictionCase,
    /// The transfer the finding is about.
    pub transfer_id: TransferId,
    /// The payload digest the finding is about.
    pub payload_digest: Digest32,
    /// The accountable peer: the committer's bound identity.
    pub peer: String,
    /// When the evaluation ran.
    pub at: LogicalTime,
    /// The retention policy in force where this evidence is held.
    pub retention_policy: Digest32,
    /// When the holder's retention of this evidence expires. Expired
    /// findings are excluded from threshold counts.
    pub retention_expires: LogicalTime,
}

impl Contradiction {
    /// Whether this finding is still retained at `now`.
    pub const fn is_live(&self, now: LogicalTime) -> bool {
        now <= self.retention_expires
    }
}

/// The evaluator's verdict: a positive contradiction, or
/// insufficient evidence with the reason recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The assertions irreconcilably disagree on every binding
    /// dimension.
    Contradiction(ContradictionCase),
    /// Anything else: mismatch, wrong semantics, or nothing to check.
    InsufficientEvidence(InsufficientReason),
}

fn intervals_overlap(
    a_from: LogicalTime,
    a_until: LogicalTime,
    b_from: LogicalTime,
    b_until: LogicalTime,
) -> bool {
    a_from <= b_until && b_from <= a_until
}

/// Check one receipt against a verified exact-set commitment. Only
/// `accepted` receipts participate: a receipt of another assertion
/// kind is a semantics mismatch, and a receipt that fails any binding
/// dimension is insufficient evidence on that dimension. A receipt for
/// a member key is consistent; a receipt for a non-member key with
/// every dimension matching is a contradiction.
pub fn evaluate_admission(commitment: &VerifiedCommitment, receipt: &VerifiedReceipt) -> Verdict {
    let body = &commitment.commitment.body;
    let other = &receipt.receipt.body;
    if receipt.assertion() != Assertion::Accepted {
        return Verdict::InsufficientEvidence(InsufficientReason::Dimension(
            MismatchDimension::ClaimSemantics,
        ));
    }
    if other.payload_digest != body.payload_digest {
        return Verdict::InsufficientEvidence(InsufficientReason::Dimension(
            MismatchDimension::ObjectDigest,
        ));
    }
    if other.scope != body.scope {
        return Verdict::InsufficientEvidence(InsufficientReason::Dimension(
            MismatchDimension::Scope,
        ));
    }
    if other.policy_digest != body.policy_digest {
        return Verdict::InsufficientEvidence(InsufficientReason::Dimension(
            MismatchDimension::Policy,
        ));
    }
    if !intervals_overlap(
        body.valid_from,
        body.valid_until,
        other.valid_from,
        other.valid_until,
    ) {
        return Verdict::InsufficientEvidence(InsufficientReason::Dimension(
            MismatchDimension::Interval,
        ));
    }
    if body.recipients.contains(&other.recipient_key) {
        return Verdict::InsufficientEvidence(InsufficientReason::Consistent);
    }
    Verdict::Contradiction(ContradictionCase::NonMemberAdmission)
}

/// Check a set of receipts against one commitment and keep the
/// strongest outcome: a contradiction beats consistency, and
/// consistency beats having nothing to check at all.
pub fn evaluate_receipts(
    commitment: &VerifiedCommitment,
    receipts: &[VerifiedReceipt],
) -> (Verdict, Option<Contradiction>) {
    let mut seen_accepted = false;
    let mut consistent = false;
    for receipt in receipts {
        if receipt.assertion() == Assertion::Accepted {
            seen_accepted = true;
        }
        match evaluate_admission(commitment, receipt) {
            Verdict::Contradiction(case) => {
                let body = &commitment.commitment.body;
                return (
                    Verdict::Contradiction(case),
                    Some(Contradiction {
                        case,
                        transfer_id: body.transfer_id,
                        payload_digest: body.payload_digest,
                        peer: body.signer.clone(),
                        at: body.issued_at,
                        retention_policy: body.retention_policy,
                        retention_expires: body.retention_expires,
                    }),
                );
            }
            Verdict::InsufficientEvidence(InsufficientReason::Consistent) => {
                consistent = true;
            }
            Verdict::InsufficientEvidence(_) => {}
        }
    }
    if consistent || seen_accepted {
        (
            Verdict::InsufficientEvidence(InsufficientReason::Consistent),
            None,
        )
    } else {
        (
            Verdict::InsufficientEvidence(InsufficientReason::NoCounterpart),
            None,
        )
    }
}

/// Check two verified commitments against each other. A positive
/// finding needs the same transfer, scope, policy and authorization
/// with overlapping validity, and then a real disagreement: different
/// digests or different committed sets. Anything else is insufficient
/// evidence on the dimension that differs — including two identical
/// commitments, which are consistent.
pub fn evaluate_commitments(
    first: &VerifiedCommitment,
    second: &VerifiedCommitment,
) -> (Verdict, Option<Contradiction>) {
    let a = &first.commitment.body;
    let b = &second.commitment.body;
    if a.transfer_id != b.transfer_id {
        return (
            Verdict::InsufficientEvidence(InsufficientReason::Dimension(
                MismatchDimension::ObjectDigest,
            )),
            None,
        );
    }
    if a.scope != b.scope {
        return (
            Verdict::InsufficientEvidence(InsufficientReason::Dimension(MismatchDimension::Scope)),
            None,
        );
    }
    if a.policy_digest != b.policy_digest {
        return (
            Verdict::InsufficientEvidence(InsufficientReason::Dimension(MismatchDimension::Policy)),
            None,
        );
    }
    if a.authorization != b.authorization {
        return (
            Verdict::InsufficientEvidence(InsufficientReason::Dimension(
                MismatchDimension::Authorization,
            )),
            None,
        );
    }
    if !intervals_overlap(a.valid_from, a.valid_until, b.valid_from, b.valid_until) {
        return (
            Verdict::InsufficientEvidence(InsufficientReason::Dimension(
                MismatchDimension::Interval,
            )),
            None,
        );
    }
    if a.payload_digest != b.payload_digest || a.recipients.root() != b.recipients.root() {
        let case = ContradictionCase::DivergentCommitments;
        return (
            Verdict::Contradiction(case),
            Some(Contradiction {
                case,
                transfer_id: a.transfer_id,
                payload_digest: a.payload_digest,
                peer: a.signer.clone(),
                at: a.issued_at,
                retention_policy: a.retention_policy,
                retention_expires: a.retention_expires,
            }),
        );
    }
    (
        Verdict::InsufficientEvidence(InsufficientReason::Consistent),
        None,
    )
}

/// A retained contradiction finding with its contributor, for
/// threshold counting. Retries of one event count once: identity is
/// the accountable peer plus the object.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StoredFinding {
    peer: String,
    transfer_id: TransferId,
    payload_digest: Digest32,
    retention_expires: LogicalTime,
}

/// Counts distinct attributable contradictions per peer inside a
/// finite window. Evidence IDs deduplicate (one object counts once
/// per peer no matter how often it is re-reported), per-identity
/// contribution is capped by the threshold query itself, and expired
/// findings never count.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ContradictionCounter {
    findings: Vec<StoredFinding>,
}

impl ContradictionCounter {
    /// An empty counter.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a finding. Recording is unconditional; expiry and
    /// attribution are decided at count time so a later retention
    /// change cannot rewrite history.
    pub fn record(&mut self, finding: &Contradiction) {
        self.findings.push(StoredFinding {
            peer: finding.peer.clone(),
            transfer_id: finding.transfer_id,
            payload_digest: finding.payload_digest,
            retention_expires: finding.retention_expires,
        });
    }

    /// How many distinct live objects with findings name `peer` at
    /// `now`. Expired findings are excluded; re-reports of one object
    /// count once.
    pub fn count_attributable(&self, peer: &str, now: LogicalTime) -> u32 {
        let mut objects = BTreeSet::new();
        for finding in &self.findings {
            if finding.peer != peer || now > finding.retention_expires {
                continue;
            }
            objects.insert((finding.transfer_id, finding.payload_digest));
        }
        objects.len() as u32
    }

    /// Whether `peer` meets `threshold` distinct attributable live
    /// findings at `now`.
    pub fn meets_threshold(&self, peer: &str, threshold: u32, now: LogicalTime) -> bool {
        self.count_attributable(peer, now) >= threshold
    }
}

/// Render a finding as a public-log claim statement: domain-separated,
/// fixed-field, and bounded so it always fits the log. The statement
/// carries the finding's retention fields so the log record and the
/// evidence agree on how long this may be held.
pub fn finding_statement(finding: &Contradiction) -> Result<Vec<u8>, CommitmentError> {
    let mut out = FINDING_STATEMENT_DOMAIN.to_vec();
    out.extend_from_slice(match finding.case {
        ContradictionCase::NonMemberAdmission => b"non-member-admission",
        ContradictionCase::DivergentCommitments => b"divergent-commitments",
    });
    out.extend_from_slice(&finding.transfer_id);
    out.extend_from_slice(&finding.payload_digest);
    push_field(&mut out, finding.peer.as_bytes());
    out.extend_from_slice(&finding.at.to_be_bytes());
    out.extend_from_slice(&finding.retention_policy);
    out.extend_from_slice(&finding.retention_expires.to_be_bytes());
    if out.len() > MAX_STATEMENT_BYTES {
        return Err(CommitmentError::StatementTooLarge { len: out.len() });
    }
    Ok(out)
}

/// Build the log claim body that publishes a finding. The subject is
/// the accountable peer and the statement is
/// [`finding_statement`]; the caller signs with [`sign_event`] and
/// appends with [`ClaimLog::append`]. Publication never moves a
/// reputation score: B27 evidence from a unilateral claim alters
/// nothing, and a contradiction finding is a claim, not a ruling.
pub fn finding_claim_body(
    finding: &Contradiction,
    event_id: EventId,
    author: &str,
    author_key: &KeyBytes,
    issued_at: LogicalTime,
) -> Result<EventBody, CommitmentError> {
    Ok(EventBody {
        event_id,
        kind: EventKind::Claim,
        author: author.to_string(),
        author_key: *author_key,
        subject: finding.peer.clone(),
        in_reply_to: None,
        statement: finding_statement(finding)?,
        issued_at,
        policy_digest: finding.retention_policy,
        expires_at: finding.retention_expires,
    })
}

/// Publish a finding to a public log and return the log's evidence
/// view of it. A convenience for the integration path the slice owns:
/// sign, append, then read back the evidence the log hands to
/// reputation.
pub fn publish_finding(
    log: &mut ClaimLog,
    registry: &BindingRegistry,
    finding: &Contradiction,
    event_id: EventId,
    author: &str,
    signing: &SigningKey,
    issued_at: LogicalTime,
) -> Result<EventId, ClaimError> {
    let body = finding_claim_body(
        finding,
        event_id,
        author,
        &signing.verifying_key().to_bytes(),
        issued_at,
    )
    .map_err(|_| ClaimError::EmptyStatement)?;
    let event = sign_event(&body, signing);
    log.append(registry, &event.encode())
}
