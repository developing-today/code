//! Versioned, deterministic projections over an already-authorized name
//! candidate set (A-9121.2; slice A9121-lenses-groups).
//!
//! This module does not verify receipts, signatures, or namespace policy.
//! Its caller supplies the authorized candidate/evidence inputs. Evidence
//! values are deliberately store-local: id-store depends on id-core, while
//! the full B19 federation receipt implementation is in id-sync, which
//! depends on id-store. Adapters must turn verified B19 presence into these
//! neutral values; unsigned inventory observations stay a distinct variant.

use std::collections::{BTreeMap, BTreeSet};

use id_core::id_shape::ContentId;

use crate::name_candidates::{NameCandidate, NameCandidateError};

/// The version of the ranking and tie-breaking rules in this module.
pub const LENS_POLICY_VERSION: u64 = 1;

/// The selected projection over the authorized candidate multiset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lens {
    /// Prefer the candidate with the greatest explicit local override.
    LocalOverrides,
    /// Rank by authorized aggregate popularity.
    TotalPopularity,
    /// Keep candidates with authorized interaction evidence, then rank by it.
    InteractionCohort,
    /// Rank candidates with a currently valid, in-scope standing claim first.
    StandingCohort,
    /// Keep only candidates curated into this voluntary group.
    AuthorityGroup(String),
}

/// The policy context already authenticated by the caller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LensContext {
    /// The exact namespace/tenant scope of this projection.
    pub scope: String,
    /// Current logical time used for expiry checks.
    pub now: u64,
    /// Signers whose candidate assertions are authorized for this query.
    pub authorized_signers: BTreeSet<[u8; 32]>,
    /// Issuers whose projection evidence is authorized for this query.
    pub authorized_evidence_issuers: BTreeSet<[u8; 32]>,
}

/// Policy evidence consumed by a lens. A stored-unverified observation is
/// represented but never treated as standing or as verified presence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LensEvidence {
    /// A B19 `verified_present` result already signature- and
    /// validity-checked by its adapter; this is not a wire receipt type.
    VerifiedPresence {
        /// Digest of the content whose presence was verified.
        subject_digest: [u8; 32],
        /// Exact scope bound by the receipt.
        scope: String,
        /// Logical time at which presence was observed.
        observed_at: u64,
        /// Inclusive expiry of the verified result.
        valid_until: u64,
    },
    /// An opaque standing assertion, interpreted only by issuer/scope/expiry.
    Standing {
        /// Scope for which the issuer made the claim.
        scope: String,
        /// Inclusive last logical time for which it is valid.
        expires_at: u64,
    },
    /// Inventory-only observation, never equivalent to a verified receipt.
    StoredUnverified,
}

/// Authorized evidence associated with one candidate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandidateEvidence {
    /// Candidate this evidence describes.
    pub candidate_id: ContentId,
    /// Issuer that supplied this projection evidence.
    pub issuer: [u8; 32],
    /// Namespace/tenant scope authorized for this evidence.
    pub scope: String,
    /// Aggregate popularity value declared by policy.
    pub popularity: u64,
    /// Whether this candidate belongs to the caller's interaction cohort.
    pub interacted: bool,
    /// Explicit caller-local preference; larger values rank first.
    pub local_override: Option<u64>,
    /// Standing or inventory-only evidence, if supplied.
    pub standing: Option<LensEvidence>,
    /// Voluntary, potentially overlapping curation groups.
    pub authority_groups: BTreeSet<String>,
}

impl CandidateEvidence {
    /// Associate an issuer-scoped standing value with this evidence.
    #[must_use]
    pub fn with_standing(mut self, issuer: [u8; 32], standing: LensEvidence) -> Self {
        self.issuer = issuer;
        self.standing = Some(standing);
        self
    }

    /// Set the explicit local override score.
    #[must_use]
    pub const fn with_local_override(mut self, score: Option<u64>) -> Self {
        self.local_override = score;
        self
    }

    /// Mark membership in the caller's interaction cohort.
    #[must_use]
    pub const fn with_interacted(mut self, interacted: bool) -> Self {
        self.interacted = interacted;
        self
    }
}

impl LensEvidence {
    /// Whether this value represents verified presence that remains valid
    /// for the supplied scope and logical time. The observation must have
    /// already happened (`observed_at <= now`) and remain unexpired.
    #[must_use]
    pub fn proves_present_in(&self, scope: &str, now: u64) -> bool {
        matches!(
            self,
            Self::VerifiedPresence { scope: evidence_scope, observed_at, valid_until, .. }
                if evidence_scope == scope && *observed_at <= now && now <= *valid_until
        )
    }
}

/// Apply one lens to candidates and evidence. Candidates outside the
/// caller's authorized signer set or exact scope are removed before evidence
/// aggregation; unauthorized evidence is indistinguishable from absent data.
/// The returned candidates are borrowed and are never rewritten or collapsed.
///
/// Ordering ties are broken by signer bytes, then candidate ID digest.
///
/// # Errors
///
/// Returns a candidate identity/encoding error if a candidate ID cannot be
/// computed.
pub fn project<'a>(
    candidates: &'a [NameCandidate],
    evidence: &[CandidateEvidence],
    context: &LensContext,
    lens: &Lens,
) -> Result<Vec<&'a NameCandidate>, NameCandidateError> {
    let mut evidence_by_candidate = BTreeMap::<ContentId, Vec<&CandidateEvidence>>::new();
    for item in evidence {
        if context.authorized_evidence_issuers.contains(&item.issuer) && item.scope == context.scope
        {
            evidence_by_candidate
                .entry(item.candidate_id)
                .or_default()
                .push(item);
        }
    }

    let mut ranked = Vec::new();
    for candidate in candidates {
        if candidate.scope() != context.scope
            || !context.authorized_signers.contains(candidate.signer())
        {
            continue;
        }
        let candidate_id = candidate.candidate_id()?;
        let records = evidence_by_candidate
            .get(&candidate_id)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let score = match lens {
            Lens::LocalOverrides => records
                .iter()
                .filter_map(|record| record.local_override)
                .max()
                .unwrap_or(0),
            Lens::TotalPopularity => records
                .iter()
                .map(|record| record.popularity)
                .max()
                .unwrap_or(0),
            Lens::InteractionCohort => {
                if !records.iter().any(|record| record.interacted) {
                    continue;
                }
                records.iter().filter(|record| record.interacted).count() as u64
            }
            Lens::StandingCohort => u64::from(records.iter().any(|record| {
                matches!(
                    record.standing.as_ref(),
                    Some(LensEvidence::Standing { scope, expires_at })
                        if scope == &context.scope && context.now <= *expires_at
                )
            })),
            Lens::AuthorityGroup(group_id) => {
                if !records
                    .iter()
                    .any(|record| record.authority_groups.contains(group_id))
                {
                    continue;
                }
                0
            }
        };
        ranked.push((
            score,
            *candidate.signer(),
            *candidate_id.digest(),
            candidate,
        ));
    }

    ranked.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| left.2.cmp(&right.2))
    });
    Ok(ranked.into_iter().map(|entry| entry.3).collect())
}
