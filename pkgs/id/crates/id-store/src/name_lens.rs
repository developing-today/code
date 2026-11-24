//! Explicit name lenses and voluntary authority-group filters
//! (slice A9121-lenses-groups, issue #10281; parent #9121; settled
//! design: `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/
//! part-a-addressing/versioned-design.md`, section "#9121 —
//! String-path names, lenses and authority groups", A-9121.2;
//! statements S4.53–S4.63).
//!
//! A lens is a named, versioned projection function over authorized
//! evidence. The input is one name's authorized candidate set — the
//! set [`NameCandidateStore::query`] admits, where every candidate
//! is a signed assertion by an authorized signer and collisions are
//! data, never a hidden winner — plus the evidence model below. The
//! output is that same candidate set in a deterministic order, with
//! one exception: the authority-group lens is a *filter* and
//! projects only the candidates the selected group has curated.
//!
//! Ordering is deterministic for a fixed policy version: declared
//! score descending, then signer ID ascending, then candidate ID
//! ascending; exact ties (duplicate candidates) keep input order.
//! The projection is a pure function of its inputs: it keeps no
//! state and consults no clock, store or network.
//!
//! The five lenses of A-9121.2:
//!
//! - [`Lens::LocalOverrides`]: candidates signed by one of the
//!   client's own local signers outrank all others.
//! - [`Lens::TotalPopularity`]: candidates are scored by their
//!   total attested interaction count across all observers.
//! - [`Lens::InteractionCohort`]: the popularity score counts only
//!   attestations from observers inside a selected cohort.
//! - [`Lens::StandingCohort`]: standing is an opaque, issuer-scoped
//!   claim queried through policy, never a global numeric trust
//!   metric. A claim is an (issuer, reference) pair; the lens never
//!   interprets the reference. Cohort membership is binary: the
//!   candidate's signer holds at least one claim from an issuer the
//!   policy accepts, or it does not.
//! - [`Lens::AuthorityGroup`]: groups sign curation assertions
//!   ([`GroupCuration`]) and are voluntary; several groups may
//!   curate the same candidate, groups may overlap, and selecting a
//!   group for one projection grants it no authority anywhere else.
//!   A curation counts only when it verifies and its signer is the
//!   group's registered key. Write authority is untouched by all of
//!   this: it remains namespace policy and signer authorization.
//!
//! Existence evidence follows the B19 binding of
//! A9115-existence-receipt (#10256). A candidate's presence is
//! [`PresenceEvidence::VerifiedPresent`] only when a B19
//! `verified_present` ack — signature-checked by the existence gate
//! and presence-checked against the candidate's canonical bytes —
//! backs it; the variant carries exactly the fields that ack binds.
//! A `stored_unverified` inventory observation is carried as
//! [`PresenceEvidence::StoredUnverified`] and never proves presence
//! ([`PresenceEvidence::proves_present`] is false for it), under
//! every lens. Presence is reported per projected candidate; it
//! never changes any lens's ordering.
//!
//! The crate-root export belongs to the wiring slice (9110-02
//! line); until it names this module, the integration test includes
//! it by path, as the neighbouring ID slices do.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use id_core::canonical_cbor::{CborError, Value, encode_canonical};

use crate::name_candidates::{NameCandidate, NameCandidateError};

/// Version of the lens definitions in this module. Every projection
/// stamps the selected lens and this version beside its entries.
pub const LENS_VERSION: u64 = 1;
/// Versioned signing domain for group curation assertions.
pub const CURATION_DOMAIN: &str = "id.name-curation/v1";
/// Record version written by this module for curation assertions.
pub const CURATION_RECORD_VERSION: u64 = 1;
/// Maximum byte length of a group id label.
pub const MAX_GROUP_ID_BYTES: usize = 256;

/// An Ed25519 signer identity (verifying-key bytes).
pub type SignerId = [u8; 32];
/// A candidate's identity: the digest bytes of its
/// [`NameCandidate::candidate_id`].
pub type CandidateDigest = [u8; 32];
/// A SHA-256 digest: a B19 subject digest.
pub type Digest32 = [u8; 32];

/// One of the five named lenses of A-9121.2. The selection is part
/// of the projection it produces: search results name the lens they
/// were projected through.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lens {
    /// Candidates signed by a local signer outrank all others.
    LocalOverrides,
    /// Score is the total attested interaction count.
    TotalPopularity,
    /// Score counts only attestations from observers in `cohort`.
    InteractionCohort {
        /// The observer identities whose attestations count.
        cohort: BTreeSet<SignerId>,
    },
    /// Score is cohort membership: the candidate's signer holds a
    /// standing claim from at least one accepted issuer.
    StandingCohort {
        /// The issuers whose standing claims the policy accepts.
        accepted_issuers: BTreeSet<SignerId>,
    },
    /// Filter: only candidates the named group has curated are
    /// projected, scored by the group's declared rank.
    AuthorityGroup {
        /// The selected group's id.
        group_id: String,
    },
}

impl Lens {
    /// The lens's published name.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::LocalOverrides => "local_overrides",
            Self::TotalPopularity => "total_popularity",
            Self::InteractionCohort { .. } => "interaction_cohort",
            Self::StandingCohort { .. } => "standing_cohort",
            Self::AuthorityGroup { .. } => "authority_group",
        }
    }

    /// The lens definition version.
    #[must_use]
    pub const fn version(&self) -> u64 {
        LENS_VERSION
    }
}

/// The policy a projection runs under. Ordering is deterministic
/// for a fixed policy version and fixed inputs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LensPolicy {
    /// The policy version, stamped onto the projection.
    pub version: u64,
    /// The client's own signers, whose candidates are the local
    /// overrides under [`Lens::LocalOverrides`].
    pub local_signers: BTreeSet<SignerId>,
}

/// A candidate's existence evidence, classified by the existence
/// gate under the B19 binding (A9115-existence-receipt). The lens
/// preserves the classification exactly; it never upgrades an
/// observation into proof.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum PresenceEvidence {
    /// No existence evidence accompanies the candidate.
    #[default]
    None,
    /// A `stored_unverified` inventory observation: some bytes were
    /// seen in storage. Unsigned, inventory-only, and never proof
    /// that the content was accepted or validated.
    StoredUnverified {
        /// The digest of the bytes seen.
        subject_digest: Digest32,
        /// When they were seen (logical time).
        observed_at: u64,
        /// Where they were seen.
        location: String,
    },
    /// A B19 `verified_present` ack, admitted only after the
    /// existence gate verified its signature and its presence
    /// response against the candidate's canonical bytes. Carries
    /// the fields the ack binds.
    VerifiedPresent {
        /// The digest of the candidate's canonical bytes.
        subject_digest: Digest32,
        /// The responder identity the ack binds.
        responder: String,
        /// The key the responder answered with.
        responder_key: SignerId,
        /// The tenant/namespace scope the assertion holds in.
        scope: String,
        /// The responder's policy epoch at observation.
        epoch: u64,
        /// When the responder observed the content (logical time).
        observed_at: u64,
    },
}

impl PresenceEvidence {
    /// Whether this evidence proves the candidate present: only a
    /// verified ack does. An unverified storage observation never
    /// does.
    #[must_use]
    pub const fn proves_present(&self) -> bool {
        matches!(self, Self::VerifiedPresent { .. })
    }

    /// The subject digest the evidence is about, when it carries
    /// one.
    #[must_use]
    pub const fn subject_digest(&self) -> Option<&Digest32> {
        match self {
            Self::None => None,
            Self::StoredUnverified { subject_digest, .. }
            | Self::VerifiedPresent { subject_digest, .. } => Some(subject_digest),
        }
    }
}

/// One observer's attested interaction count for a candidate.
/// Attestations accumulate: repeated attestations from one observer
/// sum.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InteractionAttestation {
    /// The observing signer.
    pub observer: SignerId,
    /// The interaction count this observer attests.
    pub interactions: u64,
}

/// An opaque, issuer-scoped standing claim about a candidate's
/// signer. The reference is issuer-scoped data: the lens compares
/// issuers only and never interprets the reference, so standing can
/// never be reduced to a numeric trust metric here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StandingClaim {
    /// The claim's issuer.
    pub issuer: SignerId,
    /// The opaque issuer-scoped claim reference.
    pub reference: String,
}

/// The evidence model for one candidate.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CandidateEvidence {
    /// The candidate's classified existence evidence.
    pub presence: PresenceEvidence,
    /// Interaction attestations naming this candidate.
    pub interactions: Vec<InteractionAttestation>,
    /// Standing claims about this candidate's signer.
    pub standing: Vec<StandingClaim>,
}

/// Evidence for a candidate set, keyed by candidate digest.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LensEvidence {
    by_candidate: BTreeMap<CandidateDigest, CandidateEvidence>,
}

impl LensEvidence {
    /// An empty evidence model.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Attach evidence to one candidate, replacing any prior entry.
    pub fn insert(&mut self, candidate: CandidateDigest, evidence: CandidateEvidence) {
        self.by_candidate.insert(candidate, evidence);
    }

    /// The evidence attached to one candidate, if any.
    #[must_use]
    pub fn get(&self, candidate: &CandidateDigest) -> Option<&CandidateEvidence> {
        self.by_candidate.get(candidate)
    }
}

/// A voluntary authority group, identified by the key its
/// curations must be signed with. Registration is local policy: a
/// group registered here holds authority over nothing but the
/// projections that select it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorityGroup {
    /// The group's id, as curations and lens selections name it.
    pub group_id: String,
    /// The group's signing key.
    pub group_key: SignerId,
}

/// One group's signed curation assertion about one candidate: the
/// group declares a rank for the candidate inside its own curation.
/// Assertions are not deduplicated across groups; several groups may
/// curate the same candidate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupCuration {
    group_id: String,
    candidate: CandidateDigest,
    declared_rank: u64,
    signer: SignerId,
    signature: [u8; 64],
}

impl GroupCuration {
    /// Sign a curation assertion with the group's key.
    ///
    /// # Errors
    ///
    /// [`NameLensError::Invalid`] when the group id is not a visible
    /// label; [`NameLensError::Cbor`] when encoding fails.
    pub fn sign(
        group_id: impl Into<String>,
        candidate: CandidateDigest,
        declared_rank: u64,
        key: &SigningKey,
    ) -> Result<Self, NameLensError> {
        let group_id = group_id.into();
        checked_group_id(&group_id)?;
        let signer = key.verifying_key().to_bytes();
        let unsigned = unsigned_value(&group_id, &candidate, declared_rank, &signer);
        let signature = key
            .sign(&signature_preimage(&encode_canonical(&unsigned)?))
            .to_bytes();
        Ok(Self {
            group_id,
            candidate,
            declared_rank,
            signer,
            signature,
        })
    }

    /// Verify the signature and all signed fields.
    ///
    /// # Errors
    ///
    /// [`NameLensError::Invalid`] when the group id is not a visible
    /// label; [`NameLensError::BadCurationKey`] when the signer
    /// bytes are not a valid Ed25519 key;
    /// [`NameLensError::BadCurationSignature`] when the signature
    /// does not verify.
    pub fn verify(&self) -> Result<(), NameLensError> {
        checked_group_id(&self.group_id)?;
        let key =
            VerifyingKey::from_bytes(&self.signer).map_err(|_| NameLensError::BadCurationKey)?;
        let unsigned = unsigned_value(
            &self.group_id,
            &self.candidate,
            self.declared_rank,
            &self.signer,
        );
        key.verify_strict(
            &signature_preimage(&encode_canonical(&unsigned)?),
            &Signature::from_bytes(&self.signature),
        )
        .map_err(|_| NameLensError::BadCurationSignature)
    }

    /// The group this curation speaks for.
    #[must_use]
    pub fn group_id(&self) -> &str {
        &self.group_id
    }

    /// The curated candidate's digest.
    #[must_use]
    pub const fn candidate(&self) -> &CandidateDigest {
        &self.candidate
    }

    /// The rank the group declares for the candidate.
    #[must_use]
    pub const fn declared_rank(&self) -> u64 {
        self.declared_rank
    }

    /// The signer public key bytes.
    #[must_use]
    pub const fn signer(&self) -> &SignerId {
        &self.signer
    }

    /// Canonical bytes for storage or transport.
    ///
    /// # Errors
    ///
    /// [`NameLensError::Cbor`] when encoding fails.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, NameLensError> {
        let mut entries = unsigned_entries(
            &self.group_id,
            &self.candidate,
            self.declared_rank,
            &self.signer,
        );
        entries.push((text("signature"), Value::Bytes(self.signature.to_vec())));
        Ok(encode_canonical(&Value::Map(entries))?)
    }
}

/// One candidate in a projection: the signed assertion itself, its
/// declared score under the selected lens, and its classified
/// existence evidence. Source, provenance, signer and scope are the
/// candidate's own signed fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectedCandidate {
    /// The signed candidate assertion.
    pub candidate: NameCandidate,
    /// The candidate's identity digest.
    pub candidate_digest: CandidateDigest,
    /// The declared score the selected lens assigns.
    pub score: u64,
    /// The candidate's classified existence evidence.
    pub presence: PresenceEvidence,
}

impl ProjectedCandidate {
    /// Whether verified existence evidence backs this candidate.
    #[must_use]
    pub fn proves_present(&self) -> bool {
        self.presence.proves_present()
    }
}

/// The result of projecting a candidate set through one lens: the
/// selection, the versions it ran under, and the ordered entries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LensProjection {
    /// The lens that produced this projection.
    pub lens: Lens,
    /// The lens definition version.
    pub lens_version: u64,
    /// The policy version the projection ran under.
    pub policy_version: u64,
    /// The projected candidates, in projection order.
    pub entries: Vec<ProjectedCandidate>,
}

/// Why a projection or a curation was rejected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NameLensError {
    /// A field violates the lens contract.
    Invalid(&'static str),
    /// Canonical encoding failed.
    Cbor(CborError),
    /// A candidate assertion failed verification.
    Candidate(NameCandidateError),
    /// The curation signer bytes are not a valid Ed25519 key.
    BadCurationKey,
    /// The curation signature does not verify.
    BadCurationSignature,
    /// A curation names the selected group but is signed by a key
    /// other than the group's registered key.
    CurationSignerMismatch,
    /// The selected authority group is not registered.
    UnknownGroup,
}

impl fmt::Display for NameLensError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(problem) => f.write_str(problem),
            Self::Cbor(error) => write!(f, "curation encoding: {error}"),
            Self::Candidate(error) => write!(f, "candidate assertion: {error}"),
            Self::BadCurationKey => f.write_str("curation signer key is invalid"),
            Self::BadCurationSignature => f.write_str("curation signature is invalid"),
            Self::CurationSignerMismatch => {
                f.write_str("curation is not signed by the selected group's key")
            }
            Self::UnknownGroup => f.write_str("selected authority group is not registered"),
        }
    }
}

impl std::error::Error for NameLensError {}

impl From<CborError> for NameLensError {
    fn from(error: CborError) -> Self {
        Self::Cbor(error)
    }
}

impl From<NameCandidateError> for NameLensError {
    fn from(error: NameCandidateError) -> Self {
        Self::Candidate(error)
    }
}

/// Project one name's authorized candidate set through `lens`.
///
/// Every candidate is verified before it is projected. Under the
/// four ordering lenses the candidate set is stable: every input
/// candidate appears exactly once. Under
/// [`Lens::AuthorityGroup`] the projection is a filter: only
/// candidates the selected group has curated appear, scored by the
/// group's declared rank (the highest rank when the group curated a
/// candidate more than once). A curation that names the selected
/// group but fails verification, or is signed by a key other than
/// the group's registered key, rejects the projection: forged
/// curation is never silently counted or silently dropped.
///
/// # Errors
///
/// [`NameLensError::Candidate`] when a candidate does not verify;
/// [`NameLensError::UnknownGroup`] when the selected group is not
/// registered (or is registered under conflicting keys); the
/// curation errors above when the selected group's curations do not
/// check out.
pub fn project(
    candidates: &[NameCandidate],
    evidence: &LensEvidence,
    curations: &[GroupCuration],
    groups: &[AuthorityGroup],
    lens: &Lens,
    policy: &LensPolicy,
) -> Result<LensProjection, NameLensError> {
    let mut resolved = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        candidate.verify()?;
        resolved.push((*candidate.candidate_id()?.digest(), candidate));
    }

    let mut entries: Vec<ProjectedCandidate> = Vec::new();
    match lens {
        Lens::LocalOverrides => {
            for (digest, candidate) in &resolved {
                let score = u64::from(policy.local_signers.contains(candidate.signer()));
                entries.push(projected(digest, candidate, score, evidence));
            }
        }
        Lens::TotalPopularity => {
            for (digest, candidate) in &resolved {
                let score = evidence_for(evidence, digest).map_or(0, |candidate_evidence| {
                    candidate_evidence
                        .interactions
                        .iter()
                        .fold(0u64, |total, attestation| {
                            total.saturating_add(attestation.interactions)
                        })
                });
                entries.push(projected(digest, candidate, score, evidence));
            }
        }
        Lens::InteractionCohort { cohort } => {
            for (digest, candidate) in &resolved {
                let score = evidence_for(evidence, digest).map_or(0, |candidate_evidence| {
                    candidate_evidence
                        .interactions
                        .iter()
                        .filter(|attestation| cohort.contains(&attestation.observer))
                        .fold(0u64, |total, attestation| {
                            total.saturating_add(attestation.interactions)
                        })
                });
                entries.push(projected(digest, candidate, score, evidence));
            }
        }
        Lens::StandingCohort { accepted_issuers } => {
            for (digest, candidate) in &resolved {
                let member = evidence_for(evidence, digest).is_some_and(|candidate_evidence| {
                    candidate_evidence
                        .standing
                        .iter()
                        .any(|claim| accepted_issuers.contains(&claim.issuer))
                });
                entries.push(projected(digest, candidate, u64::from(member), evidence));
            }
        }
        Lens::AuthorityGroup { group_id } => {
            let group = registered_group(groups, group_id)?;
            let mut ranks: BTreeMap<CandidateDigest, u64> = BTreeMap::new();
            for curation in curations {
                if curation.group_id() != group_id {
                    continue;
                }
                curation.verify()?;
                if curation.signer() != &group.group_key {
                    return Err(NameLensError::CurationSignerMismatch);
                }
                let rank = ranks.entry(*curation.candidate()).or_insert(0);
                *rank = (*rank).max(curation.declared_rank());
            }
            for (digest, candidate) in &resolved {
                if let Some(score) = ranks.get(digest) {
                    entries.push(projected(digest, candidate, *score, evidence));
                }
            }
        }
    }

    entries.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.candidate.signer().cmp(right.candidate.signer()))
            .then_with(|| left.candidate_digest.cmp(&right.candidate_digest))
    });

    Ok(LensProjection {
        lens: lens.clone(),
        lens_version: lens.version(),
        policy_version: policy.version,
        entries,
    })
}

fn evidence_for<'a>(
    evidence: &'a LensEvidence,
    digest: &CandidateDigest,
) -> Option<&'a CandidateEvidence> {
    evidence.get(digest)
}

fn projected(
    digest: &CandidateDigest,
    candidate: &NameCandidate,
    score: u64,
    evidence: &LensEvidence,
) -> ProjectedCandidate {
    ProjectedCandidate {
        candidate: candidate.clone(),
        candidate_digest: *digest,
        score,
        presence: evidence_for(evidence, digest)
            .map_or(PresenceEvidence::None, |candidate_evidence| {
                candidate_evidence.presence.clone()
            }),
    }
}

fn registered_group<'a>(
    groups: &'a [AuthorityGroup],
    group_id: &str,
) -> Result<&'a AuthorityGroup, NameLensError> {
    let mut matches = groups.iter().filter(|group| group.group_id == group_id);
    let Some(group) = matches.next() else {
        return Err(NameLensError::UnknownGroup);
    };
    if matches.any(|other| other.group_key != group.group_key) {
        return Err(NameLensError::Invalid(
            "authority group is registered under conflicting keys",
        ));
    }
    Ok(group)
}

fn checked_group_id(group_id: &str) -> Result<(), NameLensError> {
    if group_id.is_empty()
        || group_id.len() > MAX_GROUP_ID_BYTES
        || group_id.chars().any(char::is_control)
    {
        Err(NameLensError::Invalid(
            "group id must be a non-empty visible label",
        ))
    } else {
        Ok(())
    }
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

fn unsigned_entries(
    group_id: &str,
    candidate: &CandidateDigest,
    declared_rank: u64,
    signer: &SignerId,
) -> Vec<(Value, Value)> {
    vec![
        (text("candidate"), Value::Bytes(candidate.to_vec())),
        (text("group"), text(group_id)),
        (text("rank"), Value::UInt(declared_rank)),
        (text("signer"), Value::Bytes(signer.to_vec())),
        (text("v"), Value::UInt(CURATION_RECORD_VERSION)),
    ]
}

fn unsigned_value(
    group_id: &str,
    candidate: &CandidateDigest,
    declared_rank: u64,
    signer: &SignerId,
) -> Value {
    Value::Map(unsigned_entries(group_id, candidate, declared_rank, signer))
}

fn signature_preimage(unsigned: &[u8]) -> Vec<u8> {
    let mut preimage = Vec::with_capacity(CURATION_DOMAIN.len() + 1 + unsigned.len());
    preimage.extend_from_slice(CURATION_DOMAIN.as_bytes());
    preimage.push(0);
    preimage.extend_from_slice(unsigned);
    preimage
}
