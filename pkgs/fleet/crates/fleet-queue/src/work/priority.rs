//! Versioned priority profile contract and pure validation /
//! comparison interface — PRI-01 (developing-today/code#9478; design
//! parent #9199, program #9196).
//!
//! Design of record:
//! `doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/components/9199-distributed-priority.md`
//! ("Taxonomy mapping; do not transplant the historic scale",
//! "Candidate order and rank semantics", "Triage mechanics") and the
//! architecture's "Protected owner priority policy" section.
//!
//! What this module fixes, stated once:
//!
//! * **Ratified values are exact.** Drew's #7030 bands
//!   (`expedite = 15`, `next-up = 25`) are first-class members of the
//!   queue profile. A value is either a member of its profile or a
//!   typed [`PriorityError::OffProfile`] — it is never snapped to the
//!   nearest scale value (the adapter defect the design names:
//!   `15 → 10`, `25 → 20`).
//! * **Profiles are versioned and never compared across.** A
//!   [`PriorityValue`] carries its profile identity and version;
//!   comparing values from different profiles is a typed
//!   [`PriorityError::ProfileMismatch`], not a numeric comparison.
//!   Translation between profiles happens only through an explicit,
//!   dated, *named-intent* [`ProfileMapping`] — never an affine
//!   conversion or rounding.
//! * **Order is total and deterministic.** [`compare_candidates`] is
//!   the V2 comparator tuple: urgency ascending (untriaged last),
//!   exact signed peer rank (legacy `None` reads as `0`), creation
//!   time ascending, then canonical `(authority_id, work_id)`
//!   identity — the final identity tie V1's claim order lacked.
//! * **Protected expedite membership is an owner-only write.**
//!   [`check_priority_write`] is the gate every priority-affecting
//!   path crosses. Only the authenticated configured owner principal
//!   acting as the owner may change a protected item's intrinsic
//!   value, peer placement, label intent or membership. Routine
//!   manual writes, aging, clump refinement, hysteresis, label
//!   normalisation or removal, imports, mobile and slash commands
//!   all deny with `OWNER_POLICY_PROTECTED`; a stale reference
//!   conflicts with `OWNER_POLICY_VERSION_CONFLICT`; a missing or
//!   unverifiable policy fails closed. Removing the mirror label
//!   does not unprotect an item.
//!
//! * **Aging reconsiders, it never just ages.** [`evaluate_aging`]
//!   (PRI-05a, #9481) turns dated evidence (deadline, pivot,
//!   freshness, effort, opportunity, value) into at most one bounded
//!   scale step per sweep, emitted as an `aging_reconsideration`
//!   triage hint. Elapsed time only makes an item due; age alone
//!   never moves it and never reaches Critical, and the protected
//!   owner set is skipped.
//! * **Opportunity boosts are temporary and separate.**
//!   [`OpportunityBoost`] (PRI-05b, #9482) is a derived factor with an
//!   explicit start, expiry and `expiry_action = retriage`.
//!   [`effective_priority`] applies the strongest active boost without
//!   touching intrinsic urgency; [`evaluate_boost_expiry`] removes only
//!   the boost and emits an `opportunity_expired` retriage request, so
//!   the item returns to its own value and stays in the searchable
//!   backlog instead of being buried. The protected owner set and the
//!   ratified bands are never boosted.
//! * **Hysteresis resists churn, never authority.**
//!   [`evaluate_hysteresis`] (PRI-05c, #9483) suppresses low-confidence,
//!   too-soon, multi-step or drifting automatic moves with an
//!   `hysteresis_suppressed` hint. An authorised manual change or a
//!   verified deadline/incident bypasses the resistance, but never the
//!   owner gate or the capability check.
//!
//! The module is pure: no I/O, no queue, no persisted state. It
//! changes no live priority and authorises no schema change — later
//! slices (PRI-02 … PRI-07) consume it.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

use super::contracts::{
    AuthenticatedPrincipal, ContractError, OwnerPriorityPolicy, WorkRef, is_canonical_decimal,
};

/// Wire schema identifier of a priority profile.
pub const PRIORITY_PROFILE_SCHEMA_VERSION: &str = "fleet.priority-profile/v1";
/// Wire schema identifier of a named-intent cross-profile mapping.
pub const PROFILE_MAPPING_SCHEMA_VERSION: &str = "fleet.priority-profile-mapping/v1";

/// Identity of the current queue/dispatch profile.
pub const QUEUE_PROFILE_ID: &str = "fleet.queue";
/// Version of the current queue/dispatch profile.
pub const QUEUE_PROFILE_VERSION: &str = "1";

/// #7030 ratified stored band for the `expedite` label.
pub const EXPEDITE_VALUE: i64 = 15;
/// #7030 ratified stored band for the `next-up` label.
pub const NEXT_UP_VALUE: i64 = 25;
/// Wire schema identifier of a temporary opportunity boost.
pub const OPPORTUNITY_BOOST_SCHEMA_VERSION: &str = "fleet.priority-opportunity-boost/v1";
/// The most scale steps one opportunity boost may move an item.
pub const MAX_BOOST_STEPS: u8 = 2;

/// Wire schema identifier of a hysteresis policy.
pub const HYSTERESIS_POLICY_SCHEMA_VERSION: &str = "fleet.priority-hysteresis-policy/v1";
/// The capability a manual priority change requires.
pub const PRIORITY_WRITE_CAPABILITY: &str = "priority.write";

/// Evidence for the ratified bands: the #7030 ratification comment
/// and the broker readback comment cited by the design.
pub const RATIFIED_BAND_EVIDENCE: [&str; 2] = [
    "https://github.com/developing-today/code/issues/7030#issuecomment-6715673367",
    "https://github.com/developing-today/code/issues/7030#issuecomment-6786217364",
];

/// A typed priority-contract failure. Every variant is
/// non-mutating: the check that produced it is the whole outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PriorityError {
    /// A profile or mapping schema version this contract does not
    /// support.
    UnsupportedVersion {
        /// The schema version that was presented.
        schema_version: String,
    },
    /// The profile or mapping document is malformed.
    InvalidProfile {
        /// What was wrong, in one line.
        detail: String,
    },
    /// The value is not a member of the profile. Never snapped.
    OffProfile {
        /// The profile the value was checked against.
        profile_id: String,
        /// The value that was presented.
        value: i64,
    },
    /// The name is not defined by the profile.
    UnknownName {
        /// The profile the name was looked up in.
        profile_id: String,
        /// The name that was presented.
        name: String,
    },
    /// Two values (or a value and a profile) from different
    /// profiles/versions were compared or combined.
    ProfileMismatch {
        /// `profile_id@version` that was expected.
        expected: String,
        /// `profile_id@version` that was found.
        found: String,
    },
    /// No named-intent entry maps this value across profiles.
    UnmappedAcrossProfiles {
        /// The mapping that was consulted.
        mapping_id: String,
        /// The source-profile name with no mapping entry.
        name: String,
    },
    /// The owner policy (or its membership snapshot) is missing or
    /// unverifiable for an affected mutation: fail closed.
    OwnerPolicyUnavailable,
    /// An opportunity boost document is malformed, targets other
    /// work, or replays a reference with different content.
    InvalidBoost {
        /// What was wrong, in one line.
        detail: String,
    },
    /// A shared-contract owner-policy failure
    /// (`OWNER_POLICY_PROTECTED`, `OWNER_POLICY_VERSION_CONFLICT`,
    /// `owner_policy_ref_missing`).
    OwnerPolicy(ContractError),
}

impl PriorityError {
    /// The stable wire error code for this failure.
    pub fn code(&self) -> &'static str {
        match self {
            PriorityError::UnsupportedVersion { .. } => "unsupported_version",
            PriorityError::InvalidProfile { .. } => "invalid_profile",
            PriorityError::OffProfile { .. } => "off_profile_value",
            PriorityError::UnknownName { .. } => "unknown_priority_name",
            PriorityError::ProfileMismatch { .. } => "profile_mismatch",
            PriorityError::UnmappedAcrossProfiles { .. } => "unmapped_across_profiles",
            PriorityError::OwnerPolicyUnavailable => "owner_policy_unavailable",
            PriorityError::InvalidBoost { .. } => "invalid_boost",
            PriorityError::OwnerPolicy(err) => err.code(),
        }
    }
}

impl fmt::Display for PriorityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PriorityError::UnsupportedVersion { schema_version } => {
                write!(f, "unsupported_version: {schema_version}")
            }
            PriorityError::InvalidProfile { detail } => write!(f, "invalid_profile: {detail}"),
            PriorityError::OffProfile { profile_id, value } => {
                write!(
                    f,
                    "off_profile_value: {value} is not a member of {profile_id}"
                )
            }
            PriorityError::UnknownName { profile_id, name } => {
                write!(
                    f,
                    "unknown_priority_name: {name:?} is not defined by {profile_id}"
                )
            }
            PriorityError::ProfileMismatch { expected, found } => {
                write!(f, "profile_mismatch: expected {expected}, found {found}")
            }
            PriorityError::UnmappedAcrossProfiles { mapping_id, name } => write!(
                f,
                "unmapped_across_profiles: {mapping_id} has no named entry for {name:?}"
            ),
            PriorityError::OwnerPolicyUnavailable => write!(
                f,
                "owner_policy_unavailable: missing or unverifiable owner policy fails closed"
            ),
            PriorityError::InvalidBoost { detail } => write!(f, "invalid_boost: {detail}"),
            PriorityError::OwnerPolicy(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for PriorityError {}

impl From<ContractError> for PriorityError {
    fn from(err: ContractError) -> Self {
        PriorityError::OwnerPolicy(err)
    }
}

/// Why a value is in a profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    /// A value on the profile's stated scale.
    Scale,
    /// A ratified stored band (#7030), deliberately off the stated
    /// scale and preserved exactly.
    RatifiedBand,
}

/// One member value of a profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileEntry {
    /// The exact stored value. Lower claims sooner.
    pub value: i64,
    /// The entry's name (unique within the profile).
    pub name: String,
    /// Scale value or ratified band.
    pub kind: EntryKind,
    /// Evidence for the entry, required for ratified bands.
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

/// A versioned priority profile (`fleet.priority-profile/v1`): the
/// closed set of values a producer may store and a consumer must
/// read back unchanged. Lower values claim sooner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriorityProfile {
    /// `fleet.priority-profile/v1`.
    pub schema_version: String,
    /// Stable profile identity, e.g. `fleet.queue`.
    pub profile_id: String,
    /// Profile version (canonical decimal string).
    pub version: String,
    /// The value an absent request priority resolves to, by name.
    /// Reads never fabricate it: an untriaged stored value stays
    /// `None`.
    #[serde(default)]
    pub default_name: Option<String>,
    /// Member values, ascending by value.
    pub entries: Vec<ProfileEntry>,
}

/// A value validated against a specific profile version.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PriorityValue {
    /// The profile the value belongs to.
    pub profile_id: String,
    /// The profile version the value was validated against.
    pub profile_version: String,
    /// The exact stored value.
    pub value: i64,
}

impl PriorityValue {
    fn profile_key(&self) -> String {
        format!("{}@{}", self.profile_id, self.profile_version)
    }

    /// Compare two values of the same profile version (lower is
    /// sooner, so `Less` means `self` claims first). Values from
    /// different profiles are never compared numerically.
    pub fn cmp_urgency(&self, other: &PriorityValue) -> Result<Ordering, PriorityError> {
        if self.profile_id != other.profile_id || self.profile_version != other.profile_version {
            return Err(PriorityError::ProfileMismatch {
                expected: self.profile_key(),
                found: other.profile_key(),
            });
        }
        Ok(self.value.cmp(&other.value))
    }
}

impl PriorityProfile {
    /// The current queue/dispatch profile: the stated scale
    /// `[10, 20, 30, 40, 50, 70, 90]` (names from
    /// `pm-stack/dispatch/src/validate.ts`) plus #7030's ratified
    /// `expedite = 15` and `next-up = 25` bands. Default `normal`.
    pub fn queue_v1() -> PriorityProfile {
        let scale = |value: i64, name: &str| ProfileEntry {
            value,
            name: name.to_string(),
            kind: EntryKind::Scale,
            evidence_refs: Vec::new(),
        };
        let band = |value: i64, name: &str| ProfileEntry {
            value,
            name: name.to_string(),
            kind: EntryKind::RatifiedBand,
            evidence_refs: RATIFIED_BAND_EVIDENCE
                .iter()
                .map(|s| s.to_string())
                .collect(),
        };
        PriorityProfile {
            schema_version: PRIORITY_PROFILE_SCHEMA_VERSION.to_string(),
            profile_id: QUEUE_PROFILE_ID.to_string(),
            version: QUEUE_PROFILE_VERSION.to_string(),
            default_name: Some("normal".to_string()),
            entries: vec![
                scale(10, "critical"),
                band(EXPEDITE_VALUE, "expedite"),
                scale(20, "high"),
                band(NEXT_UP_VALUE, "next-up"),
                scale(30, "scale-30"),
                scale(40, "normal"),
                scale(50, "scale-50"),
                scale(70, "low"),
                scale(90, "scale-90"),
            ],
        }
    }

    /// Validate the profile document: supported schema, canonical
    /// version, non-empty, strictly ascending unique values, unique
    /// non-empty names, evidence on every ratified band, and a
    /// default that names a member.
    pub fn validate(&self) -> Result<(), PriorityError> {
        if self.schema_version != PRIORITY_PROFILE_SCHEMA_VERSION {
            return Err(PriorityError::UnsupportedVersion {
                schema_version: self.schema_version.clone(),
            });
        }
        let invalid = |detail: String| Err(PriorityError::InvalidProfile { detail });
        if self.profile_id.is_empty() {
            return invalid("profile_id must not be empty".to_string());
        }
        if !is_canonical_decimal(&self.version) {
            return invalid(format!(
                "version is not a canonical decimal string: {:?}",
                self.version
            ));
        }
        if self.entries.is_empty() {
            return invalid("a profile needs at least one entry".to_string());
        }
        let mut names = BTreeSet::new();
        for (index, entry) in self.entries.iter().enumerate() {
            if entry.name.is_empty() {
                return invalid(format!("entry {} has an empty name", entry.value));
            }
            if !names.insert(entry.name.as_str()) {
                return invalid(format!("duplicate entry name {:?}", entry.name));
            }
            if index > 0 && self.entries[index - 1].value >= entry.value {
                return invalid(format!(
                    "entries must be strictly ascending by value: {} then {}",
                    self.entries[index - 1].value,
                    entry.value
                ));
            }
            if entry.kind == EntryKind::RatifiedBand && entry.evidence_refs.is_empty() {
                return invalid(format!(
                    "ratified band {:?} carries no evidence reference",
                    entry.name
                ));
            }
        }
        if let Some(name) = &self.default_name {
            if !names.contains(name.as_str()) {
                return invalid(format!("default_name {name:?} is not a profile entry"));
            }
        }
        Ok(())
    }

    /// Serialize the profile to its wire JSON form.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("profile serialization is infallible")
    }

    /// Parse and validate a profile from JSON text.
    pub fn from_json(text: &str) -> Result<PriorityProfile, PriorityError> {
        let profile: PriorityProfile =
            serde_json::from_str(text).map_err(|err| PriorityError::InvalidProfile {
                detail: format!("profile does not match {PRIORITY_PROFILE_SCHEMA_VERSION}: {err}"),
            })?;
        profile.validate()?;
        Ok(profile)
    }

    fn key(&self) -> String {
        format!("{}@{}", self.profile_id, self.version)
    }

    fn tag(&self, value: i64) -> PriorityValue {
        PriorityValue {
            profile_id: self.profile_id.clone(),
            profile_version: self.version.clone(),
            value,
        }
    }

    /// The entry holding `value`, if it is a member.
    pub fn entry(&self, value: i64) -> Option<&ProfileEntry> {
        self.entries.iter().find(|entry| entry.value == value)
    }

    /// Validate an exact stored or requested value. A member passes
    /// through unchanged; anything else is
    /// [`PriorityError::OffProfile`] — never snapped.
    pub fn validate_value(&self, value: i64) -> Result<PriorityValue, PriorityError> {
        match self.entry(value) {
            Some(_) => Ok(self.tag(value)),
            None => Err(PriorityError::OffProfile {
                profile_id: self.key(),
                value,
            }),
        }
    }

    /// Resolve a named value (case-insensitive, surrounding
    /// whitespace ignored). Only the profile's own names resolve.
    pub fn resolve_name(&self, name: &str) -> Result<PriorityValue, PriorityError> {
        let wanted = name.trim().to_ascii_lowercase();
        self.entries
            .iter()
            .find(|entry| entry.name == wanted)
            .map(|entry| self.tag(entry.value))
            .ok_or_else(|| PriorityError::UnknownName {
                profile_id: self.key(),
                name: name.to_string(),
            })
    }

    /// Resolve a request priority: absent → the profile default;
    /// otherwise a decimal integer (exact member) or a name.
    pub fn resolve_request(&self, input: Option<&str>) -> Result<PriorityValue, PriorityError> {
        match input.map(str::trim) {
            None | Some("") => match &self.default_name {
                Some(name) => self.resolve_name(name),
                None => Err(PriorityError::InvalidProfile {
                    detail: format!("{} defines no default", self.key()),
                }),
            },
            Some(text) => {
                let digits = text.strip_prefix('+').unwrap_or(text);
                if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
                    match digits.parse::<i64>() {
                        Ok(value) => self.validate_value(value),
                        Err(_) => Err(PriorityError::InvalidProfile {
                            detail: format!("priority {text:?} is out of range"),
                        }),
                    }
                } else {
                    self.resolve_name(text)
                }
            }
        }
    }

    /// Re-check that `value` was validated against this exact
    /// profile version and is still a member.
    pub fn check(&self, value: &PriorityValue) -> Result<(), PriorityError> {
        if value.profile_id != self.profile_id || value.profile_version != self.version {
            return Err(PriorityError::ProfileMismatch {
                expected: self.key(),
                found: value.profile_key(),
            });
        }
        self.validate_value(value.value).map(|_| ())
    }
}

/// One named-intent mapping entry: a source-profile name and the
/// target-profile name it means.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamedMappingEntry {
    /// Name in the source profile.
    pub from_name: String,
    /// Name in the target profile.
    pub to_name: String,
}

/// An explicit, dated, named-intent mapping between two profile
/// versions (`fleet.priority-profile-mapping/v1`). This is the only
/// way a value crosses profiles: there is no arithmetic or rounding
/// fallback, and a source name with no entry is unmapped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileMapping {
    /// `fleet.priority-profile-mapping/v1`.
    pub schema_version: String,
    /// Stable mapping identity.
    pub mapping_id: String,
    /// Mapping version (canonical decimal string).
    pub version: String,
    /// The dated policy record that establishes the mapping.
    pub policy_ref: String,
    /// Source profile identity.
    pub from_profile_id: String,
    /// Source profile version.
    pub from_profile_version: String,
    /// Target profile identity.
    pub to_profile_id: String,
    /// Target profile version.
    pub to_profile_version: String,
    /// Named-intent entries.
    pub entries: Vec<NamedMappingEntry>,
}

impl ProfileMapping {
    /// Validate the mapping against both profiles: supported schema,
    /// canonical version, a policy reference, matching profile
    /// identities, every name defined on its side, and no source
    /// name mapped twice.
    pub fn validate(
        &self,
        from: &PriorityProfile,
        to: &PriorityProfile,
    ) -> Result<(), PriorityError> {
        if self.schema_version != PROFILE_MAPPING_SCHEMA_VERSION {
            return Err(PriorityError::UnsupportedVersion {
                schema_version: self.schema_version.clone(),
            });
        }
        let invalid = |detail: String| Err(PriorityError::InvalidProfile { detail });
        if !is_canonical_decimal(&self.version) {
            return invalid(format!(
                "mapping version is not a canonical decimal string: {:?}",
                self.version
            ));
        }
        if self.policy_ref.is_empty() {
            return invalid("a cross-profile mapping needs a dated policy_ref".to_string());
        }
        for (side, id, version, profile) in [
            (
                "from",
                &self.from_profile_id,
                &self.from_profile_version,
                from,
            ),
            ("to", &self.to_profile_id, &self.to_profile_version, to),
        ] {
            if id != &profile.profile_id || version != &profile.version {
                return Err(PriorityError::ProfileMismatch {
                    expected: format!("{id}@{version} ({side})"),
                    found: profile.key(),
                });
            }
        }
        let mut seen = BTreeSet::new();
        for entry in &self.entries {
            if !seen.insert(entry.from_name.as_str()) {
                return invalid(format!("source name {:?} mapped twice", entry.from_name));
            }
            from.resolve_name(&entry.from_name)?;
            to.resolve_name(&entry.to_name)?;
        }
        Ok(())
    }

    /// Map a value by named intent. The value must belong to `from`;
    /// its entry's name must have a mapping entry; the result is the
    /// target profile's value for the mapped name.
    pub fn map_value(
        &self,
        from: &PriorityProfile,
        to: &PriorityProfile,
        value: &PriorityValue,
    ) -> Result<PriorityValue, PriorityError> {
        self.validate(from, to)?;
        from.check(value)?;
        let name = &from
            .entry(value.value)
            .expect("checked membership above")
            .name;
        match self.entries.iter().find(|entry| &entry.from_name == name) {
            Some(entry) => to.resolve_name(&entry.to_name),
            None => Err(PriorityError::UnmappedAcrossProfiles {
                mapping_id: self.mapping_id.clone(),
                name: name.clone(),
            }),
        }
    }
}

/// The comparator inputs of one candidate (V2 claim comparator,
/// #9199 "Candidate order and rank semantics"). Eligibility is a
/// gate applied before comparison, never a sortable score.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateKey {
    /// Canonical pair identity — the final tie-break.
    pub work: WorkRef,
    /// Effective urgency (or intrinsic urgency where the profile
    /// derives none). `None` is untriaged and sorts after every
    /// triaged candidate, matching V1's `(priority IS NULL)` first
    /// key.
    pub urgency: Option<PriorityValue>,
    /// Exact signed peer rank within equal urgency. A legacy `None`
    /// reads as the common rank `0`.
    pub peer_rank: Option<i64>,
    /// Stable creation time (any monotone integer clock, e.g. unix
    /// seconds), ascending.
    pub created_at: i64,
}

/// Compare two candidates: urgency ascending (untriaged last), peer
/// rank ascending (`None` = `0`), creation time ascending, canonical
/// identity ascending. Rank never crosses unequal urgency because
/// urgency is compared first. Mixed profiles are a typed error.
pub fn compare_candidates(a: &CandidateKey, b: &CandidateKey) -> Result<Ordering, PriorityError> {
    let urgency = match (&a.urgency, &b.urgency) {
        (Some(x), Some(y)) => x.cmp_urgency(y)?,
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    };
    Ok(urgency
        .then_with(|| a.peer_rank.unwrap_or(0).cmp(&b.peer_rank.unwrap_or(0)))
        .then_with(|| a.created_at.cmp(&b.created_at))
        .then_with(|| a.work.cmp(&b.work)))
}

/// Sort candidates into claim order with [`compare_candidates`].
/// Every triaged candidate must share one profile version; a mixed
/// set is rejected before sorting rather than partially ordered.
pub fn order_candidates(
    mut candidates: Vec<CandidateKey>,
) -> Result<Vec<CandidateKey>, PriorityError> {
    let mut profile: Option<&PriorityValue> = None;
    for candidate in &candidates {
        if let Some(value) = &candidate.urgency {
            match profile {
                None => profile = Some(value),
                Some(first) => {
                    first.cmp_urgency(value)?;
                }
            }
        }
    }
    candidates.sort_by(|a, b| compare_candidates(a, b).expect("profiles checked above"));
    Ok(candidates)
}

/// The owner-accepted protected-membership snapshot that a
/// [`OwnerPriorityPolicy`] references. Membership is policy, not a
/// mutable label or score: nothing here reads labels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtectedMembership {
    /// The policy's `protected_membership_ref` this snapshot is.
    pub membership_ref: String,
    /// The `policy_ref` the snapshot belongs to.
    pub policy_ref: String,
    /// Protected work, by canonical pair identity (so equal issue
    /// numbers in different repos are never conflated).
    pub members: BTreeSet<WorkRef>,
}

/// Which path is asking to change priority state. Every path
/// crosses the same gate; only [`WriteSource::OwnerAction`] from the
/// configured owner principal may touch protected state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteSource {
    /// The specifically authenticated owner amendment.
    OwnerAction,
    /// Routine manual `priority.write` (any maintainer or agent).
    ManualWrite,
    /// Automatic aging / de-escalation (PRI-05).
    Aging,
    /// Clump refinement.
    ClumpRefinement,
    /// A hysteresis bypass.
    HysteresisBypass,
    /// Priority-label normalisation.
    LabelNormalization,
    /// Removal of a mirror label.
    LabelRemoval,
    /// Import / reconciliation.
    Import,
    /// A mobile action.
    Mobile,
    /// A slash command.
    SlashCommand,
    /// A temporary opportunity boost or its expiry (PRI-05b).
    OpportunityBoost,
}

/// The protected field a write touches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PriorityField {
    /// Intrinsic urgency / stored value.
    IntrinsicUrgency,
    /// Peer rank / before-after placement.
    PeerPlacement,
    /// Priority-label intent.
    LabelIntent,
    /// Membership in the protected set (add or remove).
    Membership,
}

/// A proposed priority-affecting write, as the gate sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriorityWrite {
    /// Target work.
    pub work: WorkRef,
    /// The owner-policy reference the producer checked.
    pub owner_policy_ref: Option<String>,
    /// Which path is writing.
    pub source: WriteSource,
    /// Which field it touches.
    pub field: PriorityField,
}

/// The protected-owner gate (architecture "Protected owner priority
/// policy"). Checks, in order and atomically with the caller's
/// work-version mutation:
///
/// 1. a current, valid policy observation and a membership snapshot
///    belonging to it exist — otherwise fail closed
///    ([`PriorityError::OwnerPolicyUnavailable`]);
/// 2. the write cites `owner_policy_ref` — otherwise
///    `owner_policy_ref_missing`; a different reference is
///    `OWNER_POLICY_VERSION_CONFLICT`;
/// 3. a write to a protected item, or any membership change, is
///    allowed only as [`WriteSource::OwnerAction`] by the
///    authenticated principal bound as the policy's owner — every
///    other path is `OWNER_POLICY_PROTECTED`.
///
/// `actor_ref` is never consulted; only the authenticated principal
/// is. Ordinary (unprotected) items pass for non-membership writes.
pub fn check_priority_write(
    write: &PriorityWrite,
    policy: Option<&OwnerPriorityPolicy>,
    membership: Option<&ProtectedMembership>,
    principal: Option<&AuthenticatedPrincipal>,
) -> Result<(), PriorityError> {
    let policy = policy.ok_or(PriorityError::OwnerPolicyUnavailable)?;
    policy
        .validate()
        .map_err(|_| PriorityError::OwnerPolicyUnavailable)?;
    let membership = membership.ok_or(PriorityError::OwnerPolicyUnavailable)?;
    if membership.policy_ref != policy.policy_ref
        || membership.membership_ref != policy.protected_membership_ref
    {
        return Err(PriorityError::OwnerPolicyUnavailable);
    }
    match &write.owner_policy_ref {
        None => return Err(ContractError::MissingOwnerPolicyRef.into()),
        Some(reference) if reference != &policy.policy_ref => {
            return Err(ContractError::OwnerPolicyVersionConflict.into());
        }
        Some(_) => {}
    }
    let protected = membership.members.contains(&write.work);
    if !protected && write.field != PriorityField::Membership {
        return Ok(());
    }
    let is_owner = write.source == WriteSource::OwnerAction
        && principal.is_some_and(|p| p.principal_ref == policy.owner_principal_ref);
    if is_owner {
        Ok(())
    } else {
        Err(ContractError::OwnerPolicyProtected.into())
    }
}

/// Golden ordering of the queue profile, value → name, for fixtures
/// and adapters that must agree on the exact member set.
pub fn queue_v1_golden() -> BTreeMap<i64, &'static str> {
    BTreeMap::from([
        (10, "critical"),
        (15, "expedite"),
        (20, "high"),
        (25, "next-up"),
        (30, "scale-30"),
        (40, "normal"),
        (50, "scale-50"),
        (70, "low"),
        (90, "scale-90"),
    ])
}

// ---------------------------------------------------------------------------
// PRI-05a — evidence-based bidirectional aging and review windows
// (developing-today/code#9481; design parent #9199, slice PRI-05a).
// ---------------------------------------------------------------------------

/// Wire schema identifier of an aging policy.
pub const AGING_POLICY_SCHEMA_VERSION: &str = "fleet.priority-aging-policy/v1";

/// A versioned aging policy. Aging is a *reconsideration*: its
/// output is an `aging_reconsideration` triage hint (the #9200
/// registry permits that code a hint only), never a base write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgingPolicy {
    /// `fleet.priority-aging-policy/v1`.
    pub schema_version: String,
    /// The reference recommendations cite.
    pub policy_ref: String,
    /// Policy version (canonical decimal string).
    pub version: String,
    /// Seconds after the last meaningful triage before an item is
    /// due for reconsideration.
    pub review_interval_secs: i64,
    /// Evidence observed longer ago than this is stale and ignored.
    pub evidence_max_age_secs: i64,
}

impl AgingPolicy {
    /// Validate the policy document.
    pub fn validate(&self) -> Result<(), PriorityError> {
        if self.schema_version != AGING_POLICY_SCHEMA_VERSION {
            return Err(PriorityError::UnsupportedVersion {
                schema_version: self.schema_version.clone(),
            });
        }
        let invalid = |detail: String| Err(PriorityError::InvalidProfile { detail });
        if self.policy_ref.is_empty() {
            return invalid("aging policy_ref must not be empty".to_string());
        }
        if !is_canonical_decimal(&self.version) {
            return invalid(format!(
                "aging policy version is not a canonical decimal string: {:?}",
                self.version
            ));
        }
        if self.review_interval_secs <= 0 || self.evidence_max_age_secs <= 0 {
            return invalid("aging intervals must be positive".to_string());
        }
        Ok(())
    }
}

/// Which way a signal argues the item should move.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgingDirection {
    /// Claim sooner (a lower value).
    Raise,
    /// Claim later (a higher value).
    Lower,
}

/// The kind of evidence behind a signal. Elapsed time is not a
/// kind: age only makes an item *due* for review.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgingSignalKind {
    /// A verified deadline.
    Deadline,
    /// An authorised strategic pivot.
    Pivot,
    /// Fresh activity (or its absence) on the work.
    Freshness,
    /// Effort evidence (e.g. turned out small, or much larger).
    Effort,
    /// A time-boxed opportunity.
    Opportunity,
    /// Demonstrated value / usefulness.
    Value,
}

/// One piece of evidence for a reconsideration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgingSignal {
    /// What kind of evidence.
    pub kind: AgingSignalKind,
    /// Which way it argues.
    pub direction: AgingDirection,
    /// Where the evidence is; a signal without one is ignored.
    pub evidence_ref: String,
    /// Confidence, 0–100.
    pub confidence_pct: u8,
    /// When the evidence was observed (same clock as `as_of`).
    pub observed_at: i64,
}

/// The item under reconsideration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgingItem {
    /// Canonical identity.
    pub work: WorkRef,
    /// Current intrinsic value.
    pub current: PriorityValue,
    /// Last meaningful triage time; `None` means never triaged and
    /// always due.
    pub last_meaningful_triage_at: Option<i64>,
    /// The owner-policy reference the sweep read.
    pub owner_policy_ref: Option<String>,
}

/// A bounded, evidence-backed reconsideration hint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgingRecommendation {
    /// Target work.
    pub work: WorkRef,
    /// Always `aging_reconsideration`.
    pub reason_code: String,
    /// The effect class: always a hint.
    pub effect: String,
    /// Direction of the proposed move.
    pub direction: AgingDirection,
    /// Current value.
    pub from: PriorityValue,
    /// Proposed value: exactly one scale step away.
    pub to: PriorityValue,
    /// The evidence the move rests on (deduplicated, sorted).
    pub evidence_refs: Vec<String>,
    /// Lowest confidence among the supporting signals.
    pub confidence_pct: u8,
    /// Evaluation time.
    pub as_of: i64,
    /// Aging policy reference and version.
    pub policy_ref: String,
    /// Aging policy version.
    pub policy_version: String,
    /// When the item is next due for reconsideration.
    pub next_review_at: i64,
}

/// Why an evaluation produced no recommendation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgingHold {
    /// Inside the review window since the last meaningful triage.
    NotDue,
    /// Due, but no usable evidence: age alone never moves an item.
    AgeOnly,
    /// Usable evidence argues both ways equally.
    Balanced,
    /// Already at the profile edge in the argued direction.
    AtEdge,
    /// Raising into the most urgent value needs deadline evidence;
    /// age, freshness or value alone never reach Critical.
    CriticalNeedsDeadline,
    /// The item is in the protected owner set; aging never touches it.
    ProtectedExpedite,
}

/// The outcome of one aging evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgingOutcome {
    /// Recommend one bounded step.
    Recommend(AgingRecommendation),
    /// No move, with the reason and the next review time.
    Hold {
        /// Why nothing was recommended.
        hold: AgingHold,
        /// When to look again.
        next_review_at: i64,
    },
}

/// Evaluate one item for evidence-based aging (PRI-05a).
///
/// * The protected-owner gate runs first: a protected item is held
///   as [`AgingHold::ProtectedExpedite`]; a stale, missing or
///   unverifiable owner policy is an error (fail closed).
/// * An item inside its review window is [`AgingHold::NotDue`].
/// * Only signals with an evidence reference, observed no later
///   than `as_of` and no longer ago than the policy's evidence age,
///   count; a replayed evidence reference counts once.
/// * No usable evidence is [`AgingHold::AgeOnly`]: elapsed time
///   makes an item due, it never moves it.
/// * The net direction moves the item **one** scale step (ratified
///   bands are label intent and are never an aging target); raising
///   into the profile's most urgent scale value requires deadline
///   evidence.
#[allow(clippy::too_many_arguments)]
pub fn evaluate_aging(
    profile: &PriorityProfile,
    policy: &AgingPolicy,
    item: &AgingItem,
    signals: &[AgingSignal],
    as_of: i64,
    owner_policy: Option<&OwnerPriorityPolicy>,
    membership: Option<&ProtectedMembership>,
) -> Result<AgingOutcome, PriorityError> {
    profile.validate()?;
    policy.validate()?;
    profile.check(&item.current)?;

    let gate = PriorityWrite {
        work: item.work.clone(),
        owner_policy_ref: item.owner_policy_ref.clone(),
        source: WriteSource::Aging,
        field: PriorityField::IntrinsicUrgency,
    };
    let next_review_at = as_of.saturating_add(policy.review_interval_secs);
    match check_priority_write(&gate, owner_policy, membership, None) {
        Ok(()) => {}
        Err(PriorityError::OwnerPolicy(ContractError::OwnerPolicyProtected)) => {
            return Ok(AgingOutcome::Hold {
                hold: AgingHold::ProtectedExpedite,
                next_review_at,
            });
        }
        Err(err) => return Err(err),
    }

    if let Some(last) = item.last_meaningful_triage_at {
        let due_at = last.saturating_add(policy.review_interval_secs);
        if as_of < due_at {
            return Ok(AgingOutcome::Hold {
                hold: AgingHold::NotDue,
                next_review_at: due_at,
            });
        }
    }

    let oldest = as_of.saturating_sub(policy.evidence_max_age_secs);
    let mut seen = BTreeSet::new();
    let usable: Vec<&AgingSignal> = signals
        .iter()
        .filter(|s| !s.evidence_ref.trim().is_empty())
        .filter(|s| s.observed_at <= as_of && s.observed_at >= oldest)
        .filter(|s| s.confidence_pct <= 100)
        .filter(|s| seen.insert(s.evidence_ref.clone()))
        .collect();
    if usable.is_empty() {
        return Ok(AgingOutcome::Hold {
            hold: AgingHold::AgeOnly,
            next_review_at,
        });
    }
    let raises = usable
        .iter()
        .filter(|s| s.direction == AgingDirection::Raise)
        .count();
    let lowers = usable.len() - raises;
    let direction = match raises.cmp(&lowers) {
        Ordering::Greater => AgingDirection::Raise,
        Ordering::Less => AgingDirection::Lower,
        Ordering::Equal => {
            return Ok(AgingOutcome::Hold {
                hold: AgingHold::Balanced,
                next_review_at,
            });
        }
    };
    let supporting: Vec<&&AgingSignal> =
        usable.iter().filter(|s| s.direction == direction).collect();

    let scale: Vec<i64> = profile
        .entries
        .iter()
        .filter(|e| e.kind == EntryKind::Scale)
        .map(|e| e.value)
        .collect();
    let current = item.current.value;
    let target = match direction {
        AgingDirection::Raise => scale.iter().rev().find(|v| **v < current).copied(),
        AgingDirection::Lower => scale.iter().find(|v| **v > current).copied(),
    };
    let Some(target) = target else {
        return Ok(AgingOutcome::Hold {
            hold: AgingHold::AtEdge,
            next_review_at,
        });
    };
    if direction == AgingDirection::Raise
        && Some(&target) == scale.first()
        && !supporting
            .iter()
            .any(|s| s.kind == AgingSignalKind::Deadline)
    {
        return Ok(AgingOutcome::Hold {
            hold: AgingHold::CriticalNeedsDeadline,
            next_review_at,
        });
    }

    let reason = super::contracts::ReasonCode::AgingReconsideration;
    reason.check_field_effect(super::contracts::FieldEffectKind::Hint)?;
    let mut evidence_refs: Vec<String> =
        supporting.iter().map(|s| s.evidence_ref.clone()).collect();
    evidence_refs.sort();
    Ok(AgingOutcome::Recommend(AgingRecommendation {
        work: item.work.clone(),
        reason_code: reason.as_str().to_string(),
        effect: "hint".to_string(),
        direction,
        from: item.current.clone(),
        to: profile.validate_value(target)?,
        evidence_refs,
        confidence_pct: supporting
            .iter()
            .map(|s| s.confidence_pct)
            .min()
            .unwrap_or(0),
        as_of,
        policy_ref: policy.policy_ref.clone(),
        policy_version: policy.version.clone(),
        next_review_at,
    }))
}

/// What happens when an opportunity boost expires. The only
/// supported action is to retriage: the boost is removed and the
/// item is reassessed, never silently buried.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpiryAction {
    /// Remove the temporary factor and request a retriage.
    Retriage,
}

/// A temporary, time-boxed opportunity boost (PRI-05b). It is a
/// derived factor kept apart from intrinsic urgency: applying or
/// expiring it never writes the base value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpportunityBoost {
    /// `fleet.priority-opportunity-boost/v1`.
    pub schema_version: String,
    /// Stable reference of this boost; a replay with the same
    /// reference must carry the same content.
    pub boost_ref: String,
    /// The boosted work.
    pub work: WorkRef,
    /// Where the opportunity evidence is.
    pub opportunity_ref: String,
    /// When the boost starts to apply (inclusive).
    pub starts_at: i64,
    /// When the boost stops applying (exclusive).
    pub expires_at: i64,
    /// How many scale steps toward urgent, `1..=MAX_BOOST_STEPS`.
    pub steps: u8,
    /// Always [`ExpiryAction::Retriage`].
    pub expiry_action: ExpiryAction,
}

impl OpportunityBoost {
    /// Validate the boost document.
    pub fn validate(&self) -> Result<(), PriorityError> {
        if self.schema_version != OPPORTUNITY_BOOST_SCHEMA_VERSION {
            return Err(PriorityError::UnsupportedVersion {
                schema_version: self.schema_version.clone(),
            });
        }
        let invalid = |detail: String| Err(PriorityError::InvalidBoost { detail });
        if self.boost_ref.trim().is_empty() {
            return invalid("boost_ref must not be empty".to_string());
        }
        if self.opportunity_ref.trim().is_empty() {
            return invalid(format!(
                "boost {} has no opportunity evidence reference",
                self.boost_ref
            ));
        }
        if self.expires_at <= self.starts_at {
            return invalid(format!(
                "boost {} must expire after it starts",
                self.boost_ref
            ));
        }
        if self.steps == 0 || self.steps > MAX_BOOST_STEPS {
            return invalid(format!(
                "boost {} steps must be 1..={MAX_BOOST_STEPS}, got {}",
                self.boost_ref, self.steps
            ));
        }
        Ok(())
    }

    /// Whether the boost applies at `as_of`.
    pub fn is_active(&self, as_of: i64) -> bool {
        self.starts_at <= as_of && as_of < self.expires_at
    }
}

/// The item a boost is evaluated against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoostedItem {
    /// Canonical identity.
    pub work: WorkRef,
    /// Intrinsic urgency; a boost never writes it.
    pub intrinsic: PriorityValue,
    /// The owner-policy reference the evaluation read.
    pub owner_policy_ref: Option<String>,
}

/// Why an active boost had no effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoostSuppression {
    /// The item is in the protected owner set.
    ProtectedExpedite,
    /// The item sits on a ratified band (owner label intent).
    RatifiedBand,
}

/// Intrinsic urgency and the effective value a boost produces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectivePriority {
    /// Target work.
    pub work: WorkRef,
    /// Unchanged intrinsic urgency.
    pub intrinsic: PriorityValue,
    /// The value used for ordering at `as_of`.
    pub effective: PriorityValue,
    /// The boost that produced `effective`, if any applied.
    pub active_boost_ref: Option<String>,
    /// Why an active boost was ignored, if it was.
    pub suppressed: Option<BoostSuppression>,
    /// Evaluation time.
    pub as_of: i64,
}

/// A request to reassess an item whose opportunity boost expired.
/// It is an `opportunity_expired` effect: the derived boost is
/// removed and a retriage hint is raised; the base is untouched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetriageRequest {
    /// Target work.
    pub work: WorkRef,
    /// Always `opportunity_expired`.
    pub reason_code: String,
    /// The expired boost.
    pub boost_ref: String,
    /// How many steps the removed boost carried.
    pub removed_steps: u8,
    /// Intrinsic urgency, unchanged by the expiry.
    pub base: PriorityValue,
    /// The opportunity evidence the reassessment starts from.
    pub evidence_refs: Vec<String>,
    /// When the boost expired.
    pub expired_at: i64,
    /// When the retriage is due (the evaluation time).
    pub retriage_due_at: i64,
}

/// Where a boost stands at an evaluation time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoostState {
    /// Not started yet.
    Pending {
        /// When it starts.
        starts_at: i64,
    },
    /// Applying now.
    Active {
        /// When it expires.
        expires_at: i64,
    },
    /// Expired: remove it and retriage.
    Expired(RetriageRequest),
    /// Expired on a protected item: the boost never applied, so it is
    /// dropped and no automatic retriage is raised.
    ExpiredProtected {
        /// The dropped boost.
        boost_ref: String,
    },
}

/// Run the owner gate for a boost path. `Ok(true)` means the item is
/// protected; any other failure (missing, stale or unverifiable
/// policy) is an error, so the evaluation fails closed.
fn boost_gate_protected(
    item: &BoostedItem,
    owner_policy: Option<&OwnerPriorityPolicy>,
    membership: Option<&ProtectedMembership>,
) -> Result<bool, PriorityError> {
    let gate = PriorityWrite {
        work: item.work.clone(),
        owner_policy_ref: item.owner_policy_ref.clone(),
        source: WriteSource::OpportunityBoost,
        field: PriorityField::IntrinsicUrgency,
    };
    match check_priority_write(&gate, owner_policy, membership, None) {
        Ok(()) => Ok(false),
        Err(PriorityError::OwnerPolicy(ContractError::OwnerPolicyProtected)) => Ok(true),
        Err(err) => Err(err),
    }
}

/// Compute an item's effective value from its intrinsic urgency and
/// its opportunity boosts (PRI-05b).
///
/// * Boosts for other work are ignored; every boost for this work is
///   validated, and a replayed `boost_ref` with different content is
///   [`PriorityError::InvalidBoost`].
/// * Boosts never stack: the active boost with the most steps applies
///   (ties broken by `boost_ref`), so concurrent opportunities cannot
///   inflate an item.
/// * A boost moves the effective value up the scale (ratified bands
///   are skipped) and never into the most urgent scale value; reaching
///   Critical needs a deadline, not an opportunity.
/// * Protected owner items and ratified-band items are never boosted.
pub fn effective_priority(
    profile: &PriorityProfile,
    item: &BoostedItem,
    boosts: &[OpportunityBoost],
    as_of: i64,
    owner_policy: Option<&OwnerPriorityPolicy>,
    membership: Option<&ProtectedMembership>,
) -> Result<EffectivePriority, PriorityError> {
    profile.validate()?;
    profile.check(&item.intrinsic)?;
    let protected = boost_gate_protected(item, owner_policy, membership)?;

    let mut seen: BTreeMap<&str, &OpportunityBoost> = BTreeMap::new();
    for boost in boosts.iter().filter(|b| b.work == item.work) {
        boost.validate()?;
        let replayed = seen
            .insert(boost.boost_ref.as_str(), boost)
            .is_some_and(|prior| prior != boost);
        if replayed {
            return Err(PriorityError::InvalidBoost {
                detail: format!(
                    "boost_ref {} replayed with different content",
                    boost.boost_ref
                ),
            });
        }
    }
    let strongest = seen
        .values()
        .filter(|b| b.is_active(as_of))
        .max_by(|a, b| a.steps.cmp(&b.steps).then(b.boost_ref.cmp(&a.boost_ref)));

    let unchanged = |suppressed: Option<BoostSuppression>| EffectivePriority {
        work: item.work.clone(),
        intrinsic: item.intrinsic.clone(),
        effective: item.intrinsic.clone(),
        active_boost_ref: None,
        suppressed,
        as_of,
    };
    let Some(boost) = strongest else {
        return Ok(unchanged(None));
    };
    if protected {
        return Ok(unchanged(Some(BoostSuppression::ProtectedExpedite)));
    }
    let on_band = profile
        .entry(item.intrinsic.value)
        .is_some_and(|e| e.kind == EntryKind::RatifiedBand);
    if on_band {
        return Ok(unchanged(Some(BoostSuppression::RatifiedBand)));
    }

    let scale: Vec<i64> = profile
        .entries
        .iter()
        .filter(|e| e.kind == EntryKind::Scale)
        .map(|e| e.value)
        .collect();
    let floor = scale.get(1).copied().unwrap_or(item.intrinsic.value);
    let mut effective = item.intrinsic.value;
    for _ in 0..boost.steps {
        match scale.iter().rev().find(|v| **v < effective) {
            Some(next) if *next >= floor => effective = *next,
            _ => break,
        }
    }
    Ok(EffectivePriority {
        work: item.work.clone(),
        intrinsic: item.intrinsic.clone(),
        effective: profile.validate_value(effective)?,
        active_boost_ref: Some(boost.boost_ref.clone()),
        suppressed: None,
        as_of,
    })
}

/// Evaluate one opportunity boost at `as_of` (PRI-05b). On expiry the
/// boost is removed and an `opportunity_expired` retriage request is
/// returned carrying the unchanged base; the registry check confirms
/// the code permits only a derived removal and a hint, never a base
/// write. A protected item's expired boost is dropped with no
/// automatic retriage.
pub fn evaluate_boost_expiry(
    profile: &PriorityProfile,
    boost: &OpportunityBoost,
    item: &BoostedItem,
    as_of: i64,
    owner_policy: Option<&OwnerPriorityPolicy>,
    membership: Option<&ProtectedMembership>,
) -> Result<BoostState, PriorityError> {
    profile.validate()?;
    boost.validate()?;
    profile.check(&item.intrinsic)?;
    if boost.work != item.work {
        return Err(PriorityError::InvalidBoost {
            detail: format!("boost {} targets other work", boost.boost_ref),
        });
    }
    let protected = boost_gate_protected(item, owner_policy, membership)?;
    if as_of < boost.starts_at {
        return Ok(BoostState::Pending {
            starts_at: boost.starts_at,
        });
    }
    if as_of < boost.expires_at {
        return Ok(BoostState::Active {
            expires_at: boost.expires_at,
        });
    }
    if protected {
        return Ok(BoostState::ExpiredProtected {
            boost_ref: boost.boost_ref.clone(),
        });
    }
    let reason = super::contracts::ReasonCode::OpportunityExpired;
    reason.check_field_effect(super::contracts::FieldEffectKind::Derived)?;
    reason.check_field_effect(super::contracts::FieldEffectKind::Hint)?;
    if reason
        .check_field_effect(super::contracts::FieldEffectKind::Base)
        .is_ok()
    {
        return Err(PriorityError::InvalidBoost {
            detail: "opportunity_expired must never permit a base write".to_string(),
        });
    }
    match boost.expiry_action {
        ExpiryAction::Retriage => Ok(BoostState::Expired(RetriageRequest {
            work: item.work.clone(),
            reason_code: reason.as_str().to_string(),
            boost_ref: boost.boost_ref.clone(),
            removed_steps: boost.steps,
            base: item.intrinsic.clone(),
            evidence_refs: vec![boost.opportunity_ref.clone()],
            expired_at: boost.expires_at,
            retriage_due_at: as_of,
        })),
    }
}

/// A versioned hysteresis policy (PRI-05c): a bounded buffer around
/// every scale edge for automatic moves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HysteresisPolicy {
    /// `fleet.priority-hysteresis-policy/v1`.
    pub schema_version: String,
    /// The reference decisions cite.
    pub policy_ref: String,
    /// Policy version (canonical decimal string).
    pub version: String,
    /// Automatic moves below this confidence (0–100) are suppressed.
    pub min_confidence_pct: u8,
    /// No automatic move within this many seconds of the item's last
    /// applied move (manual or automatic).
    pub cooldown_secs: i64,
    /// The window over which automatic drift is bounded.
    pub drift_window_secs: i64,
    /// Most net automatic scale steps allowed inside the window.
    pub max_drift_steps: u8,
}

impl HysteresisPolicy {
    /// Validate the policy document.
    pub fn validate(&self) -> Result<(), PriorityError> {
        if self.schema_version != HYSTERESIS_POLICY_SCHEMA_VERSION {
            return Err(PriorityError::UnsupportedVersion {
                schema_version: self.schema_version.clone(),
            });
        }
        let invalid = |detail: String| Err(PriorityError::InvalidProfile { detail });
        if self.policy_ref.trim().is_empty() {
            return invalid("hysteresis policy_ref must not be empty".to_string());
        }
        if !is_canonical_decimal(&self.version) {
            return invalid(format!(
                "hysteresis policy version is not a canonical decimal string: {:?}",
                self.version
            ));
        }
        if self.min_confidence_pct > 100 {
            return invalid("min_confidence_pct must be 0..=100".to_string());
        }
        if self.cooldown_secs <= 0 || self.drift_window_secs <= 0 || self.max_drift_steps == 0 {
            return invalid("hysteresis windows and drift bound must be positive".to_string());
        }
        Ok(())
    }
}

/// Where a proposed move comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MoveOrigin {
    /// Automatic triage (aging, refinement, pressure).
    Automatic,
    /// A manual change by an authenticated principal holding
    /// `priority.write`.
    Manual,
    /// The authenticated owner acting as the owner.
    OwnerAction,
    /// A verified deadline.
    VerifiedDeadline,
    /// A verified incident.
    VerifiedIncident,
}

/// Which resistance-bypass let a move through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HysteresisBypass {
    /// Authorised manual or owner action.
    Manual,
    /// Verified deadline evidence.
    Deadline,
    /// Verified incident evidence.
    Incident,
}

/// Why an automatic move was held back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HysteresisHold {
    /// Confidence below the policy threshold.
    LowConfidence,
    /// Inside the cooldown after the item's last applied move.
    Cooldown,
    /// More than one scale step in one automatic move.
    OneStepOnly,
    /// Would exceed the net drift bound inside the window.
    DriftBound,
}

/// A move that was applied earlier, as the hysteresis check reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedMove {
    /// Target work.
    pub work: WorkRef,
    /// Value before.
    pub from: PriorityValue,
    /// Value after.
    pub to: PriorityValue,
    /// Where it came from.
    pub origin: MoveOrigin,
    /// When it was applied.
    pub applied_at: i64,
}

/// A proposed intrinsic-urgency move.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposedMove {
    /// Target work.
    pub work: WorkRef,
    /// Current value.
    pub from: PriorityValue,
    /// Proposed value.
    pub to: PriorityValue,
    /// Where it comes from.
    pub origin: MoveOrigin,
    /// Producer confidence, 0–100.
    pub confidence_pct: u8,
    /// Evidence; required for a verified deadline or incident.
    pub evidence_ref: Option<String>,
    /// The owner-policy reference the producer checked.
    pub owner_policy_ref: Option<String>,
    /// When the move is proposed.
    pub proposed_at: i64,
}

/// The hysteresis decision for one proposed move.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HysteresisDecision {
    /// The move may proceed (still subject to the caller's atomic
    /// work-version check).
    Allow {
        /// The bypass that skipped resistance, if any.
        bypass: Option<HysteresisBypass>,
    },
    /// The move is held back; a `hysteresis_suppressed` hint, no
    /// mutation.
    Suppress {
        /// Always `hysteresis_suppressed`.
        reason_code: String,
        /// Why.
        hold: HysteresisHold,
        /// When an automatic retry may succeed.
        retry_after: i64,
    },
}

fn scale_index(profile: &PriorityProfile, value: i64) -> Option<usize> {
    profile
        .entries
        .iter()
        .filter(|e| e.kind == EntryKind::Scale)
        .position(|e| e.value == value)
}

/// Signed scale-step distance (negative raises urgency). A ratified
/// band sits between its neighbours, so it is measured from the next
/// less urgent scale value.
fn scale_steps(profile: &PriorityProfile, from: i64, to: i64) -> i64 {
    let position = |value: i64| -> i64 {
        scale_index(profile, value).map_or_else(
            || {
                profile
                    .entries
                    .iter()
                    .filter(|e| e.kind == EntryKind::Scale && e.value > value)
                    .filter_map(|e| scale_index(profile, e.value))
                    .min()
                    .map_or(0, |i| i as i64)
            },
            |i| i as i64,
        )
    };
    position(to) - position(from)
}

/// Decide whether a proposed move passes hysteresis (PRI-05c).
///
/// Order, fail-closed:
/// 1. profile, policy and both values are valid; a no-op is refused;
/// 2. a manual move needs an authenticated principal with
///    `priority.write`; a verified deadline/incident needs evidence;
/// 3. the protected-owner gate runs for every origin — no bypass
///    skips it;
/// 4. manual, owner and verified moves bypass the resistance;
/// 5. an automatic move must be one scale step to a scale value, at
///    or above the confidence threshold, outside the cooldown since
///    the item's last applied move, and within the net drift bound.
pub fn evaluate_hysteresis(
    profile: &PriorityProfile,
    policy: &HysteresisPolicy,
    proposed: &ProposedMove,
    history: &[AppliedMove],
    owner_policy: Option<&OwnerPriorityPolicy>,
    membership: Option<&ProtectedMembership>,
    principal: Option<&AuthenticatedPrincipal>,
) -> Result<HysteresisDecision, PriorityError> {
    profile.validate()?;
    policy.validate()?;
    profile.check(&proposed.from)?;
    profile.check(&proposed.to)?;
    if proposed.from.value == proposed.to.value {
        return Err(PriorityError::InvalidProfile {
            detail: "a hysteresis move must change the value".to_string(),
        });
    }
    if proposed.confidence_pct > 100 {
        return Err(PriorityError::InvalidProfile {
            detail: "confidence must be 0..=100".to_string(),
        });
    }

    let source = match proposed.origin {
        MoveOrigin::Automatic => WriteSource::Aging,
        MoveOrigin::Manual => WriteSource::ManualWrite,
        MoveOrigin::OwnerAction => WriteSource::OwnerAction,
        MoveOrigin::VerifiedDeadline | MoveOrigin::VerifiedIncident => {
            WriteSource::HysteresisBypass
        }
    };
    if matches!(
        proposed.origin,
        MoveOrigin::Manual | MoveOrigin::OwnerAction
    ) {
        let authorized = principal.is_some_and(|p| {
            p.capabilities
                .iter()
                .any(|c| c == PRIORITY_WRITE_CAPABILITY)
        });
        if !authorized {
            return Err(ContractError::UnauthorizedInput.into());
        }
    }
    if matches!(
        proposed.origin,
        MoveOrigin::VerifiedDeadline | MoveOrigin::VerifiedIncident
    ) && proposed
        .evidence_ref
        .as_deref()
        .is_none_or(|e| e.trim().is_empty())
    {
        return Err(ContractError::UnauthorizedInput.into());
    }
    let gate = PriorityWrite {
        work: proposed.work.clone(),
        owner_policy_ref: proposed.owner_policy_ref.clone(),
        source,
        field: PriorityField::IntrinsicUrgency,
    };
    check_priority_write(&gate, owner_policy, membership, principal)?;

    let bypass = match proposed.origin {
        MoveOrigin::Manual | MoveOrigin::OwnerAction => Some(HysteresisBypass::Manual),
        MoveOrigin::VerifiedDeadline => Some(HysteresisBypass::Deadline),
        MoveOrigin::VerifiedIncident => Some(HysteresisBypass::Incident),
        MoveOrigin::Automatic => None,
    };
    if bypass.is_some() {
        return Ok(HysteresisDecision::Allow { bypass });
    }

    let reason = super::contracts::ReasonCode::HysteresisSuppressed;
    reason.check_field_effect(super::contracts::FieldEffectKind::Hint)?;
    let suppress = |hold: HysteresisHold, retry_after: i64| {
        Ok(HysteresisDecision::Suppress {
            reason_code: reason.as_str().to_string(),
            hold,
            retry_after,
        })
    };
    let now = proposed.proposed_at;
    let steps = scale_steps(profile, proposed.from.value, proposed.to.value);
    if scale_index(profile, proposed.to.value).is_none() || steps.abs() != 1 {
        return suppress(HysteresisHold::OneStepOnly, now);
    }
    if proposed.confidence_pct < policy.min_confidence_pct {
        return suppress(
            HysteresisHold::LowConfidence,
            now.saturating_add(policy.cooldown_secs),
        );
    }
    let mine: Vec<&AppliedMove> = history
        .iter()
        .filter(|m| m.work == proposed.work && m.applied_at <= now)
        .collect();
    if let Some(last) = mine.iter().map(|m| m.applied_at).max() {
        let ready_at = last.saturating_add(policy.cooldown_secs);
        if now < ready_at {
            return suppress(HysteresisHold::Cooldown, ready_at);
        }
    }
    let window_start = now.saturating_sub(policy.drift_window_secs);
    let in_window: Vec<&&AppliedMove> = mine
        .iter()
        .filter(|m| m.origin == MoveOrigin::Automatic && m.applied_at > window_start)
        .collect();
    let drift: i64 = in_window
        .iter()
        .map(|m| scale_steps(profile, m.from.value, m.to.value))
        .sum::<i64>()
        + steps;
    if drift.unsigned_abs() > u64::from(policy.max_drift_steps) {
        let retry_after = in_window
            .iter()
            .map(|m| m.applied_at)
            .min()
            .unwrap_or(now)
            .saturating_add(policy.drift_window_secs);
        return suppress(HysteresisHold::DriftBound, retry_after);
    }
    Ok(HysteresisDecision::Allow { bypass: None })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::work::contracts::OWNER_POLICY_SCHEMA_VERSION;

    fn queue() -> PriorityProfile {
        PriorityProfile::queue_v1()
    }

    fn work(id: &str) -> WorkRef {
        WorkRef::new("developing-today/code", id).unwrap()
    }

    fn charter_fixture() -> PriorityProfile {
        PriorityProfile {
            schema_version: PRIORITY_PROFILE_SCHEMA_VERSION.to_string(),
            profile_id: "charter.historical".to_string(),
            version: "1".to_string(),
            default_name: Some("ordinary".to_string()),
            entries: vec![
                ProfileEntry {
                    value: 450,
                    name: "high".to_string(),
                    kind: EntryKind::Scale,
                    evidence_refs: vec![],
                },
                ProfileEntry {
                    value: 1000,
                    name: "ordinary".to_string(),
                    kind: EntryKind::Scale,
                    evidence_refs: vec![],
                },
            ],
        }
    }

    fn charter_mapping(entries: Vec<(&str, &str)>) -> ProfileMapping {
        ProfileMapping {
            schema_version: PROFILE_MAPPING_SCHEMA_VERSION.to_string(),
            mapping_id: "charter-to-queue".to_string(),
            version: "1".to_string(),
            policy_ref: "fixture-dated-policy-2026-11-25".to_string(),
            from_profile_id: "charter.historical".to_string(),
            from_profile_version: "1".to_string(),
            to_profile_id: QUEUE_PROFILE_ID.to_string(),
            to_profile_version: QUEUE_PROFILE_VERSION.to_string(),
            entries: entries
                .into_iter()
                .map(|(f, t)| NamedMappingEntry {
                    from_name: f.to_string(),
                    to_name: t.to_string(),
                })
                .collect(),
        }
    }

    #[test]
    fn pmo_priority_profile_preserves_ratified_values() {
        let profile = queue();
        profile.validate().unwrap();

        // Every golden member passes through exactly, in order.
        let golden = queue_v1_golden();
        let members: Vec<(i64, &str)> = profile
            .entries
            .iter()
            .map(|e| (e.value, e.name.as_str()))
            .collect();
        let expected: Vec<(i64, &str)> = golden.iter().map(|(v, n)| (*v, *n)).collect();
        assert_eq!(members, expected);
        for value in golden.keys() {
            assert_eq!(profile.validate_value(*value).unwrap().value, *value);
        }

        // #7030 ratified bands are exact, never snapped to 10 / 20.
        assert_eq!(profile.validate_value(15).unwrap().value, EXPEDITE_VALUE);
        assert_eq!(profile.validate_value(25).unwrap().value, NEXT_UP_VALUE);
        assert_eq!(profile.resolve_name("expedite").unwrap().value, 15);
        assert_eq!(profile.resolve_name(" Next-Up ").unwrap().value, 25);
        assert_eq!(profile.resolve_request(Some("15")).unwrap().value, 15);
        assert_eq!(profile.resolve_request(Some("25")).unwrap().value, 25);
        for band in ["expedite", "next-up"] {
            let entry = profile.entries.iter().find(|e| e.name == band).unwrap();
            assert_eq!(entry.kind, EntryKind::RatifiedBand);
            assert!(!entry.evidence_refs.is_empty());
        }

        // Off-profile values are typed errors, never snapped.
        for off in [0, 11, 12, 14, 16, 24, 26, 35, 45, 60, 80, 100] {
            let err = profile.validate_value(off).unwrap_err();
            assert_eq!(err.code(), "off_profile_value", "value {off}");
        }
        assert_eq!(
            profile.resolve_request(Some("12")).unwrap_err().code(),
            "off_profile_value"
        );

        // The broker order is lower-sooner and keeps the bands between
        // their neighbours: 10 < 15 < 20 < 25 < 30.
        let v = |n| profile.validate_value(n).unwrap();
        assert_eq!(v(10).cmp_urgency(&v(15)).unwrap(), Ordering::Less);
        assert_eq!(v(15).cmp_urgency(&v(20)).unwrap(), Ordering::Less);
        assert_eq!(v(20).cmp_urgency(&v(25)).unwrap(), Ordering::Less);
        assert_eq!(v(25).cmp_urgency(&v(30)).unwrap(), Ordering::Less);

        // Default reads by name; absent requests resolve to normal.
        assert_eq!(profile.resolve_request(None).unwrap().value, 40);

        // Wire round-trip is lossless and re-validates.
        let parsed = PriorityProfile::from_json(&profile.to_json()).unwrap();
        assert_eq!(parsed, profile);

        // Cross-profile translation is named intent only.
        let charter = charter_fixture();
        let high_450 = charter.validate_value(450).unwrap();
        let mapping = charter_mapping(vec![("high", "high")]);
        assert_eq!(
            mapping
                .map_value(&charter, &profile, &high_450)
                .unwrap()
                .value,
            20
        );
        let ordinary = charter.validate_value(1000).unwrap();
        assert_eq!(
            mapping
                .map_value(&charter, &profile, &ordinary)
                .unwrap_err()
                .code(),
            "unmapped_across_profiles"
        );
        // A charter value is never compared numerically to a queue value.
        assert_eq!(
            high_450.cmp_urgency(&v(20)).unwrap_err().code(),
            "profile_mismatch"
        );
        // A charter number between bands (430) is not a member at all.
        assert_eq!(
            charter.validate_value(430).unwrap_err().code(),
            "off_profile_value"
        );
    }

    fn policy() -> OwnerPriorityPolicy {
        OwnerPriorityPolicy {
            schema_version: OWNER_POLICY_SCHEMA_VERSION.to_string(),
            policy_ref: "owner-policy:7".to_string(),
            version: "7".to_string(),
            evidence_refs: RATIFIED_BAND_EVIDENCE
                .iter()
                .map(|s| s.to_string())
                .collect(),
            owner_principal_ref: "principal:drew".to_string(),
            protected_membership_ref: "membership:7".to_string(),
            protected_fields: vec![
                "intrinsic_urgency".to_string(),
                "peer_placement".to_string(),
                "label_intent".to_string(),
                "membership".to_string(),
            ],
            protected_actions: vec!["priority.write".to_string()],
        }
    }

    fn membership() -> ProtectedMembership {
        ProtectedMembership {
            membership_ref: "membership:7".to_string(),
            policy_ref: "owner-policy:7".to_string(),
            members: BTreeSet::from([work("3447")]),
        }
    }

    fn principal(name: &str) -> AuthenticatedPrincipal {
        AuthenticatedPrincipal {
            principal_ref: name.to_string(),
            capabilities: vec!["priority.write".to_string()],
        }
    }

    fn write(id: &str, source: WriteSource, field: PriorityField) -> PriorityWrite {
        PriorityWrite {
            work: work(id),
            owner_policy_ref: Some("owner-policy:7".to_string()),
            source,
            field,
        }
    }

    #[test]
    fn pmo_priority_profile_preserves_protected_expedite_membership() {
        let policy = policy();
        let membership = membership();
        let drew = principal("principal:drew");
        let agent = principal("principal:agent");
        let gate = |w: &PriorityWrite, p: Option<&AuthenticatedPrincipal>| {
            check_priority_write(w, Some(&policy), Some(&membership), p)
        };

        let non_owner_sources = [
            WriteSource::ManualWrite,
            WriteSource::Aging,
            WriteSource::ClumpRefinement,
            WriteSource::HysteresisBypass,
            WriteSource::LabelNormalization,
            WriteSource::LabelRemoval,
            WriteSource::Import,
            WriteSource::Mobile,
            WriteSource::SlashCommand,
            WriteSource::OpportunityBoost,
        ];
        let fields = [
            PriorityField::IntrinsicUrgency,
            PriorityField::PeerPlacement,
            PriorityField::LabelIntent,
            PriorityField::Membership,
        ];

        // Every non-owner path is denied on a protected item, even
        // when the authenticated principal is the owner.
        for source in non_owner_sources {
            for field in fields {
                for who in [Some(&agent), Some(&drew), None] {
                    let err = gate(&write("3447", source, field), who).unwrap_err();
                    assert_eq!(err.code(), "OWNER_POLICY_PROTECTED", "{source:?} {field:?}");
                }
            }
        }

        // Label removal never unprotects: membership is unchanged.
        assert!(
            gate(
                &write("3447", WriteSource::LabelRemoval, PriorityField::Membership),
                Some(&agent)
            )
            .is_err()
        );
        assert!(membership.members.contains(&work("3447")));

        // The owner action needs the configured owner principal; an
        // owner-claiming non-owner principal is still denied.
        for field in fields {
            gate(&write("3447", WriteSource::OwnerAction, field), Some(&drew)).unwrap();
            assert_eq!(
                gate(
                    &write("3447", WriteSource::OwnerAction, field),
                    Some(&agent)
                )
                .unwrap_err()
                .code(),
                "OWNER_POLICY_PROTECTED"
            );
            assert_eq!(
                gate(&write("3447", WriteSource::OwnerAction, field), None)
                    .unwrap_err()
                    .code(),
                "OWNER_POLICY_PROTECTED"
            );
        }

        // Ordinary items take ordinary writes …
        for source in non_owner_sources {
            gate(
                &write("842", source, PriorityField::IntrinsicUrgency),
                Some(&agent),
            )
            .unwrap();
        }
        // … but promotion into the protected set is owner-only.
        assert_eq!(
            gate(
                &write("842", WriteSource::ManualWrite, PriorityField::Membership),
                Some(&agent)
            )
            .unwrap_err()
            .code(),
            "OWNER_POLICY_PROTECTED"
        );
        gate(
            &write("842", WriteSource::OwnerAction, PriorityField::Membership),
            Some(&drew),
        )
        .unwrap();

        // Equal work ids under another authority are not conflated.
        let other_repo = PriorityWrite {
            work: WorkRef::new("developing-today-forks/opencode", "3447").unwrap(),
            ..write(
                "3447",
                WriteSource::ManualWrite,
                PriorityField::IntrinsicUrgency,
            )
        };
        gate(&other_repo, Some(&agent)).unwrap();

        // Stale and missing policy references.
        let mut stale = write(
            "842",
            WriteSource::ManualWrite,
            PriorityField::IntrinsicUrgency,
        );
        stale.owner_policy_ref = Some("owner-policy:6".to_string());
        assert_eq!(
            gate(&stale, Some(&agent)).unwrap_err().code(),
            "OWNER_POLICY_VERSION_CONFLICT"
        );
        stale.owner_policy_ref = None;
        assert_eq!(
            gate(&stale, Some(&agent)).unwrap_err().code(),
            "owner_policy_ref_missing"
        );

        // Missing or unverifiable policy fails closed, even for the owner.
        let owner_write = write(
            "3447",
            WriteSource::OwnerAction,
            PriorityField::IntrinsicUrgency,
        );
        for (p, m) in [(None, Some(&membership)), (Some(&policy), None)] {
            assert_eq!(
                check_priority_write(&owner_write, p, m, Some(&drew))
                    .unwrap_err()
                    .code(),
                "owner_policy_unavailable"
            );
        }
        let mut foreign = membership.clone();
        foreign.policy_ref = "owner-policy:6".to_string();
        assert_eq!(
            check_priority_write(&owner_write, Some(&policy), Some(&foreign), Some(&drew))
                .unwrap_err()
                .code(),
            "owner_policy_unavailable"
        );
        let mut bad_version = policy.clone();
        bad_version.schema_version = "fleet.owner-priority-policy/v2".to_string();
        assert_eq!(
            check_priority_write(
                &owner_write,
                Some(&bad_version),
                Some(&membership),
                Some(&drew)
            )
            .unwrap_err()
            .code(),
            "owner_policy_unavailable"
        );
    }

    #[test]
    fn pmo_priority_comparator_is_total_and_deterministic() {
        let profile = queue();
        let key = |id: &str, urgency: Option<i64>, rank: Option<i64>, created: i64| CandidateKey {
            work: work(id),
            urgency: urgency.map(|u| profile.validate_value(u).unwrap()),
            peer_rank: rank,
            created_at: created,
        };
        let input = vec![
            key("9", None, None, 1),
            key("5", Some(40), Some(1), 1),
            key("4", Some(40), None, 5),
            key("3", Some(40), Some(0), 5),
            key("10", Some(15), Some(100), 9),
            key("2", Some(25), None, 1),
            key("1", Some(40), Some(-1), 9),
        ];
        let expected = ["10", "2", "1", "3", "4", "5", "9"];
        let mut permuted = input.clone();
        for round in 0..input.len() {
            permuted.rotate_left(1);
            if round % 2 == 0 {
                permuted.reverse();
            }
            let ordered: Vec<String> = order_candidates(permuted.clone())
                .unwrap()
                .into_iter()
                .map(|c| c.work.work_id)
                .collect();
            assert_eq!(ordered, expected);
        }
        // Rank never crosses urgency: rank 100 at 15 still beats rank -1 at 40.
        assert_eq!(
            compare_candidates(&input[4], &input[6]).unwrap(),
            Ordering::Less
        );
        // Canonical identity is the final tie, by integer value (9 < 10).
        let a = key("9", Some(40), None, 1);
        let b = key("10", Some(40), None, 1);
        assert_eq!(compare_candidates(&a, &b).unwrap(), Ordering::Less);

        // A mixed-profile set is rejected, not partially ordered.
        let charter = charter_fixture();
        let mut mixed = input.clone();
        mixed.push(CandidateKey {
            urgency: Some(charter.validate_value(450).unwrap()),
            ..key("77", None, None, 1)
        });
        assert_eq!(
            order_candidates(mixed).unwrap_err().code(),
            "profile_mismatch"
        );
    }

    #[test]
    fn pmo_priority_profile_rejects_malformed_documents() {
        let mut p = queue();
        p.schema_version = "fleet.priority-profile/v2".to_string();
        assert_eq!(p.validate().unwrap_err().code(), "unsupported_version");

        let mut p = queue();
        p.version = "01".to_string();
        assert_eq!(p.validate().unwrap_err().code(), "invalid_profile");

        let mut p = queue();
        p.entries.swap(0, 1);
        assert_eq!(p.validate().unwrap_err().code(), "invalid_profile");

        let mut p = queue();
        p.entries[1].evidence_refs.clear();
        assert_eq!(p.validate().unwrap_err().code(), "invalid_profile");

        let mut p = queue();
        p.default_name = Some("urgent".to_string());
        assert_eq!(p.validate().unwrap_err().code(), "invalid_profile");

        let mut p = queue();
        p.entries[2].name = "critical".to_string();
        assert_eq!(p.validate().unwrap_err().code(), "invalid_profile");

        assert_eq!(
            PriorityProfile::from_json("{\"schema_version\":1}")
                .unwrap_err()
                .code(),
            "invalid_profile"
        );
        assert_eq!(
            queue().resolve_request(Some("urgent")).unwrap_err().code(),
            "unknown_priority_name"
        );
        // A value validated against another profile version is rejected.
        let mut v2 = queue();
        v2.version = "2".to_string();
        let stale = queue().validate_value(15).unwrap();
        assert_eq!(v2.check(&stale).unwrap_err().code(), "profile_mismatch");

        // A mapping without a dated policy reference is invalid.
        let mut mapping = charter_mapping(vec![("high", "high")]);
        mapping.policy_ref.clear();
        assert_eq!(
            mapping
                .validate(&charter_fixture(), &queue())
                .unwrap_err()
                .code(),
            "invalid_profile"
        );
        // A mapping naming a target name the profile lacks is invalid.
        let mapping = charter_mapping(vec![("high", "urgent")]);
        assert_eq!(
            mapping
                .validate(&charter_fixture(), &queue())
                .unwrap_err()
                .code(),
            "unknown_priority_name"
        );
    }

    fn aging_policy() -> AgingPolicy {
        AgingPolicy {
            schema_version: AGING_POLICY_SCHEMA_VERSION.to_string(),
            policy_ref: "aging-policy:1".to_string(),
            version: "1".to_string(),
            review_interval_secs: 7 * 86_400,
            evidence_max_age_secs: 14 * 86_400,
        }
    }

    const NOW: i64 = 1_800_000_000;
    const DAY: i64 = 86_400;

    fn item(id: &str, value: i64, last_triage: Option<i64>) -> AgingItem {
        AgingItem {
            work: work(id),
            current: queue().validate_value(value).unwrap(),
            last_meaningful_triage_at: last_triage,
            owner_policy_ref: Some("owner-policy:7".to_string()),
        }
    }

    fn signal(kind: AgingSignalKind, direction: AgingDirection, evidence: &str) -> AgingSignal {
        AgingSignal {
            kind,
            direction,
            evidence_ref: evidence.to_string(),
            confidence_pct: 80,
            observed_at: NOW - DAY,
        }
    }

    fn age(item: &AgingItem, signals: &[AgingSignal]) -> AgingOutcome {
        evaluate_aging(
            &queue(),
            &aging_policy(),
            item,
            signals,
            NOW,
            Some(&policy()),
            Some(&membership()),
        )
        .unwrap()
    }

    fn recommended(outcome: AgingOutcome) -> AgingRecommendation {
        match outcome {
            AgingOutcome::Recommend(r) => r,
            other => panic!("expected a recommendation, got {other:?}"),
        }
    }

    fn held(outcome: AgingOutcome) -> AgingHold {
        match outcome {
            AgingOutcome::Hold { hold, .. } => hold,
            other => panic!("expected a hold, got {other:?}"),
        }
    }

    #[test]
    fn pmo_priority_aging_moves_both_directions_with_evidence() {
        use AgingDirection::*;
        use AgingSignalKind::*;
        let week_old = Some(NOW - 8 * DAY);

        // A week-old normal item with a verified deadline rises one step.
        let rise = recommended(age(
            &item("1", 40, week_old),
            &[signal(Deadline, Raise, "ev:deadline")],
        ));
        assert_eq!((rise.from.value, rise.to.value), (40, 30));
        assert_eq!(rise.direction, Raise);
        assert_eq!(rise.reason_code, "aging_reconsideration");
        assert_eq!(rise.effect, "hint");
        assert_eq!(rise.evidence_refs, vec!["ev:deadline".to_string()]);
        assert_eq!(rise.policy_ref, "aging-policy:1");
        assert_eq!(rise.policy_version, "1");
        assert_eq!(rise.as_of, NOW);
        assert_eq!(rise.next_review_at, NOW + 7 * DAY);

        // A post-pivot high item falls one step.
        let fall = recommended(age(
            &item("2", 20, week_old),
            &[signal(Pivot, Lower, "ev:pivot")],
        ));
        assert_eq!((fall.from.value, fall.to.value), (20, 30));
        assert_eq!(fall.direction, Lower);

        // Bounded: many signals still move exactly one scale step,
        // and the move skips the ratified bands (label intent).
        let many = [
            signal(Deadline, Raise, "ev:a"),
            signal(Value, Raise, "ev:b"),
            signal(Freshness, Raise, "ev:c"),
        ];
        let one = recommended(age(&item("3", 30, week_old), &many));
        assert_eq!(one.to.value, 20);
        assert_eq!(one.confidence_pct, 80);
        assert_eq!(one.evidence_refs.len(), 3);

        // Age alone never moves an item, however old.
        assert_eq!(
            held(age(&item("4", 70, Some(NOW - 400 * DAY)), &[])),
            AgingHold::AgeOnly
        );
        assert_eq!(held(age(&item("4", 70, None), &[])), AgingHold::AgeOnly);

        // Age-only paths never reach Critical: freshness/value evidence
        // cannot raise high (20) to critical (10); a deadline can.
        assert_eq!(
            held(age(
                &item("5", 20, week_old),
                &[signal(Value, Raise, "ev:v")]
            )),
            AgingHold::CriticalNeedsDeadline
        );
        let crit = recommended(age(
            &item("5", 20, week_old),
            &[signal(Deadline, Raise, "ev:d")],
        ));
        assert_eq!(crit.to.value, 10);

        // Inside the review window nothing moves.
        assert_eq!(
            held(age(
                &item("6", 40, Some(NOW - DAY)),
                &[signal(Deadline, Raise, "ev:d")]
            )),
            AgingHold::NotDue
        );

        // Unusable evidence is ignored: missing ref, future, stale.
        let mut future = signal(Deadline, Raise, "ev:future");
        future.observed_at = NOW + DAY;
        let mut stale = signal(Deadline, Raise, "ev:stale");
        stale.observed_at = NOW - 30 * DAY;
        let blank = signal(Deadline, Raise, "  ");
        assert_eq!(
            held(age(&item("7", 40, week_old), &[future, stale, blank])),
            AgingHold::AgeOnly
        );

        // A replayed evidence reference counts once; balanced evidence holds.
        let replay = [
            signal(Freshness, Raise, "ev:same"),
            signal(Freshness, Raise, "ev:same"),
            signal(Pivot, Lower, "ev:other"),
        ];
        assert_eq!(
            held(age(&item("8", 40, week_old), &replay)),
            AgingHold::Balanced
        );

        // Profile edges hold rather than leave the profile.
        assert_eq!(
            held(age(
                &item("9", 90, week_old),
                &[signal(Pivot, Lower, "ev:p")]
            )),
            AgingHold::AtEdge
        );
        assert_eq!(
            held(age(
                &item("10", 10, week_old),
                &[signal(Deadline, Raise, "ev:d")]
            )),
            AgingHold::AtEdge
        );

        // A value from another profile version is rejected.
        let mut foreign = item("11", 40, week_old);
        foreign.current.profile_version = "2".to_string();
        assert_eq!(
            evaluate_aging(
                &queue(),
                &aging_policy(),
                &foreign,
                &[],
                NOW,
                Some(&policy()),
                Some(&membership())
            )
            .unwrap_err()
            .code(),
            "profile_mismatch"
        );
    }

    #[test]
    fn pmo_priority_aging_skips_protected_expedite_set() {
        use AgingDirection::*;
        use AgingSignalKind::*;
        let week_old = Some(NOW - 8 * DAY);
        let strong = [
            signal(Pivot, Lower, "ev:pivot"),
            signal(Freshness, Lower, "ev:quiet"),
        ];

        // #3447 is in the protected set at the expedite band: aging
        // holds it, whatever the evidence says.
        assert_eq!(
            held(age(&item("3447", 15, week_old), &strong)),
            AgingHold::ProtectedExpedite
        );
        assert_eq!(
            held(age(
                &item("3447", 15, week_old),
                &[signal(Deadline, Raise, "ev:d")]
            )),
            AgingHold::ProtectedExpedite
        );

        // An unprotected item stored at 15 ages normally to the next
        // scale value, never onto the other ratified band.
        let ordinary = recommended(age(&item("842", 15, week_old), &strong));
        assert_eq!(ordinary.to.value, 20);

        // The owner gate fails closed for aging too.
        let mut stale = item("842", 40, week_old);
        stale.owner_policy_ref = Some("owner-policy:6".to_string());
        assert_eq!(
            evaluate_aging(
                &queue(),
                &aging_policy(),
                &stale,
                &strong,
                NOW,
                Some(&policy()),
                Some(&membership())
            )
            .unwrap_err()
            .code(),
            "OWNER_POLICY_VERSION_CONFLICT"
        );
        stale.owner_policy_ref = None;
        assert_eq!(
            evaluate_aging(
                &queue(),
                &aging_policy(),
                &stale,
                &strong,
                NOW,
                Some(&policy()),
                Some(&membership())
            )
            .unwrap_err()
            .code(),
            "owner_policy_ref_missing"
        );
        assert_eq!(
            evaluate_aging(
                &queue(),
                &aging_policy(),
                &item("842", 40, week_old),
                &strong,
                NOW,
                None,
                Some(&membership())
            )
            .unwrap_err()
            .code(),
            "owner_policy_unavailable"
        );

        // Malformed aging policy is rejected.
        let mut bad = aging_policy();
        bad.review_interval_secs = 0;
        assert_eq!(bad.validate().unwrap_err().code(), "invalid_profile");
        bad = aging_policy();
        bad.schema_version = "fleet.priority-aging-policy/v2".to_string();
        assert_eq!(bad.validate().unwrap_err().code(), "unsupported_version");
    }

    fn boost(
        id: &str,
        boost_ref: &str,
        steps: u8,
        starts_at: i64,
        expires_at: i64,
    ) -> OpportunityBoost {
        OpportunityBoost {
            schema_version: OPPORTUNITY_BOOST_SCHEMA_VERSION.to_string(),
            boost_ref: boost_ref.to_string(),
            work: work(id),
            opportunity_ref: format!("evidence:{boost_ref}"),
            starts_at,
            expires_at,
            steps,
            expiry_action: ExpiryAction::Retriage,
        }
    }

    fn boosted(id: &str, value: i64) -> BoostedItem {
        BoostedItem {
            work: work(id),
            intrinsic: queue().validate_value(value).unwrap(),
            owner_policy_ref: Some("owner-policy:7".to_string()),
        }
    }

    fn effective(item: &BoostedItem, boosts: &[OpportunityBoost], as_of: i64) -> EffectivePriority {
        effective_priority(
            &queue(),
            item,
            boosts,
            as_of,
            Some(&policy()),
            Some(&membership()),
        )
        .unwrap()
    }

    fn expiry(item: &BoostedItem, b: &OpportunityBoost, as_of: i64) -> BoostState {
        evaluate_boost_expiry(
            &queue(),
            b,
            item,
            as_of,
            Some(&policy()),
            Some(&membership()),
        )
        .unwrap()
    }

    #[test]
    fn pmo_priority_opportunity_expiry_retriages_without_base_mutation() {
        let cold = boosted("900", 70);
        let window = boost("900", "boost:conf", 2, NOW, NOW + 7 * DAY);

        // Before the window the boost is pending and does nothing.
        assert_eq!(
            expiry(&cold, &window, NOW - 1),
            BoostState::Pending { starts_at: NOW }
        );
        let before = effective(&cold, std::slice::from_ref(&window), NOW - 1);
        assert_eq!(before.effective.value, 70);
        assert_eq!(before.active_boost_ref, None);

        // Inside the window: two scale steps (70 → 50 → 40), base kept.
        let during = effective(&cold, std::slice::from_ref(&window), NOW + DAY);
        assert_eq!(during.effective.value, 40);
        assert_eq!(during.intrinsic.value, 70);
        assert_eq!(during.active_boost_ref.as_deref(), Some("boost:conf"));
        assert_eq!(
            expiry(&cold, &window, NOW + DAY),
            BoostState::Active {
                expires_at: NOW + 7 * DAY
            }
        );

        // Expiry (exclusive end) removes only the boost and retriages.
        let at_expiry = NOW + 7 * DAY;
        let BoostState::Expired(request) = expiry(&cold, &window, at_expiry) else {
            panic!("expected an expired boost");
        };
        assert_eq!(request.reason_code, "opportunity_expired");
        assert_eq!(request.base, cold.intrinsic);
        assert_eq!(request.base.value, 70);
        assert_eq!(request.removed_steps, 2);
        assert_eq!(
            request.evidence_refs,
            vec!["evidence:boost:conf".to_string()]
        );
        assert_eq!(request.expired_at, at_expiry);
        assert_eq!(request.retriage_due_at, at_expiry);
        let after = effective(&cold, std::slice::from_ref(&window), at_expiry + DAY);
        assert_eq!(after.effective, cold.intrinsic);
        assert_eq!(after.active_boost_ref, None);

        // The registry permits the expiry only a derived removal and a
        // hint; a base write under this code is refused.
        let code = crate::work::contracts::ReasonCode::OpportunityExpired;
        assert!(
            code.check_field_effect(crate::work::contracts::FieldEffectKind::Base)
                .is_err()
        );

        // Cold backlog stays searchable: the expired item is still in
        // the ordered set, back at its own value, not dropped or
        // buried below untouched cold work.
        let key = |id: &str, value: i64| CandidateKey {
            work: work(id),
            urgency: Some(queue().validate_value(value).unwrap()),
            peer_rank: None,
            created_at: 1,
        };
        let ordered = order_candidates(vec![
            key("901", 90),
            CandidateKey {
                work: work("900"),
                urgency: Some(after.effective.clone()),
                peer_rank: None,
                created_at: 1,
            },
            key("902", 40),
        ])
        .unwrap();
        let ids: Vec<&str> = ordered.iter().map(|c| c.work.work_id.as_str()).collect();
        assert_eq!(ids, vec!["902", "900", "901"]);

        // Boosts never stack: the strongest active one applies.
        let small = boost("900", "boost:small", 1, NOW, NOW + 7 * DAY);
        let both = effective(&cold, &[small.clone(), window.clone()], NOW + DAY);
        assert_eq!(both.effective.value, 40);
        assert_eq!(both.active_boost_ref.as_deref(), Some("boost:conf"));

        // An opportunity never reaches Critical.
        let high = boosted("903", 20);
        let capped = effective(&high, &[boost("903", "boost:hi", 2, NOW, NOW + DAY)], NOW);
        assert_eq!(capped.effective.value, 20);

        // A ratified band is owner label intent: never boosted.
        let next_up = boosted("904", NEXT_UP_VALUE);
        let band = effective(
            &next_up,
            &[boost("904", "boost:band", 1, NOW, NOW + DAY)],
            NOW,
        );
        assert_eq!(band.effective.value, NEXT_UP_VALUE);
        assert_eq!(band.suppressed, Some(BoostSuppression::RatifiedBand));

        // Boosts for other work are ignored.
        let other = effective(
            &cold,
            &[boost("905", "boost:other", 2, NOW, NOW + DAY)],
            NOW,
        );
        assert_eq!(other.effective.value, 70);

        // Replay: identical duplicates collapse, differing ones fail.
        let dup = effective(&cold, &[window.clone(), window.clone()], NOW + DAY);
        assert_eq!(dup.effective.value, 40);
        let mut forged = window.clone();
        forged.steps = 1;
        let err = effective_priority(
            &queue(),
            &cold,
            &[window.clone(), forged],
            NOW + DAY,
            Some(&policy()),
            Some(&membership()),
        )
        .unwrap_err();
        assert_eq!(err.code(), "invalid_boost");

        // Malformed and unknown-version boosts are refused.
        let mut bad = window.clone();
        bad.expires_at = bad.starts_at;
        assert_eq!(bad.validate().unwrap_err().code(), "invalid_boost");
        bad = window.clone();
        bad.steps = MAX_BOOST_STEPS + 1;
        assert_eq!(bad.validate().unwrap_err().code(), "invalid_boost");
        bad = window.clone();
        bad.opportunity_ref = " ".to_string();
        assert_eq!(bad.validate().unwrap_err().code(), "invalid_boost");
        bad = window.clone();
        bad.schema_version = "fleet.priority-opportunity-boost/v2".to_string();
        assert_eq!(bad.validate().unwrap_err().code(), "unsupported_version");
        assert!(
            serde_json::from_str::<ExpiryAction>("\"bury\"").is_err(),
            "only retriage is a valid expiry action"
        );

        // A boost for other work cannot be expired against this item.
        assert_eq!(
            evaluate_boost_expiry(
                &queue(),
                &boost("905", "boost:other", 1, NOW, NOW + DAY),
                &cold,
                NOW + 2 * DAY,
                Some(&policy()),
                Some(&membership())
            )
            .unwrap_err()
            .code(),
            "invalid_boost"
        );

        // The owner gate still fails closed for boosts.
        let mut stale = cold.clone();
        stale.owner_policy_ref = Some("owner-policy:6".to_string());
        assert_eq!(
            evaluate_boost_expiry(
                &queue(),
                &window,
                &stale,
                at_expiry,
                Some(&policy()),
                Some(&membership())
            )
            .unwrap_err()
            .code(),
            "OWNER_POLICY_VERSION_CONFLICT"
        );
        assert_eq!(
            effective_priority(
                &queue(),
                &cold,
                &[window.clone()],
                NOW,
                None,
                Some(&membership())
            )
            .unwrap_err()
            .code(),
            "owner_policy_unavailable"
        );
    }

    #[test]
    fn pmo_priority_opportunity_expiry_never_changes_protected_expedite() {
        let protected = boosted("3447", EXPEDITE_VALUE);
        let window = boost("3447", "boost:protected", 2, NOW, NOW + DAY);

        // While active, the protected item keeps its ratified value.
        let during = effective(&protected, std::slice::from_ref(&window), NOW);
        assert_eq!(during.effective.value, EXPEDITE_VALUE);
        assert_eq!(during.intrinsic.value, EXPEDITE_VALUE);
        assert_eq!(during.active_boost_ref, None);
        assert_eq!(during.suppressed, Some(BoostSuppression::ProtectedExpedite));

        // On expiry the boost is dropped with no automatic retriage
        // and no value change.
        assert_eq!(
            expiry(&protected, &window, NOW + DAY),
            BoostState::ExpiredProtected {
                boost_ref: "boost:protected".to_string()
            }
        );
        let after = effective(&protected, std::slice::from_ref(&window), NOW + 2 * DAY);
        assert_eq!(after.effective.value, EXPEDITE_VALUE);

        // The same item in another repository is a different item.
        let lookalike = BoostedItem {
            work: WorkRef::new("developing-today/other", "3447").unwrap(),
            intrinsic: queue().validate_value(70).unwrap(),
            owner_policy_ref: Some("owner-policy:7".to_string()),
        };
        let mut other_window = window.clone();
        other_window.work = lookalike.work.clone();
        let boosted_other = effective(&lookalike, &[other_window], NOW);
        assert_eq!(boosted_other.effective.value, 40);

        // A boost path is never an owner action, so the gate denies it.
        let denied = check_priority_write(
            &write(
                "3447",
                WriteSource::OpportunityBoost,
                PriorityField::IntrinsicUrgency,
            ),
            Some(&policy()),
            Some(&membership()),
            Some(&principal("principal:drew")),
        )
        .unwrap_err();
        assert_eq!(denied.code(), "OWNER_POLICY_PROTECTED");

        // A membership snapshot from another policy fails closed.
        let mut foreign = membership();
        foreign.policy_ref = "owner-policy:8".to_string();
        assert_eq!(
            effective_priority(
                &queue(),
                &protected,
                &[window.clone()],
                NOW,
                Some(&policy()),
                Some(&foreign)
            )
            .unwrap_err()
            .code(),
            "owner_policy_unavailable"
        );
    }

    fn hysteresis_policy() -> HysteresisPolicy {
        HysteresisPolicy {
            schema_version: HYSTERESIS_POLICY_SCHEMA_VERSION.to_string(),
            policy_ref: "hysteresis:1".to_string(),
            version: "1".to_string(),
            min_confidence_pct: 70,
            cooldown_secs: DAY,
            drift_window_secs: 30 * DAY,
            max_drift_steps: 2,
        }
    }

    fn proposed(id: &str, from: i64, to: i64, origin: MoveOrigin, confidence: u8) -> ProposedMove {
        ProposedMove {
            work: work(id),
            from: queue().validate_value(from).unwrap(),
            to: queue().validate_value(to).unwrap(),
            origin,
            confidence_pct: confidence,
            evidence_ref: None,
            owner_policy_ref: Some("owner-policy:7".to_string()),
            proposed_at: NOW,
        }
    }

    fn applied(id: &str, from: i64, to: i64, origin: MoveOrigin, at: i64) -> AppliedMove {
        AppliedMove {
            work: work(id),
            from: queue().validate_value(from).unwrap(),
            to: queue().validate_value(to).unwrap(),
            origin,
            applied_at: at,
        }
    }

    fn decide(
        mv: &ProposedMove,
        history: &[AppliedMove],
        who: Option<&AuthenticatedPrincipal>,
    ) -> Result<HysteresisDecision, PriorityError> {
        evaluate_hysteresis(
            &queue(),
            &hysteresis_policy(),
            mv,
            history,
            Some(&policy()),
            Some(&membership()),
            who,
        )
    }

    fn hold_of(decision: HysteresisDecision) -> (HysteresisHold, i64) {
        match decision {
            HysteresisDecision::Suppress {
                reason_code,
                hold,
                retry_after,
            } => {
                assert_eq!(reason_code, "hysteresis_suppressed");
                (hold, retry_after)
            }
            other => panic!("expected suppression, got {other:?}"),
        }
    }

    #[test]
    fn pmo_priority_hysteresis_cooldown_bounded_with_authorized_bypass() {
        let allow = HysteresisDecision::Allow { bypass: None };
        let agent = principal("principal:agent");

        // A confident one-step automatic move with no history passes.
        let step = proposed("700", 40, 30, MoveOrigin::Automatic, 80);
        assert_eq!(decide(&step, &[], None).unwrap(), allow);

        // Low confidence is held for one cooldown.
        let weak = proposed("700", 40, 30, MoveOrigin::Automatic, 50);
        assert_eq!(
            hold_of(decide(&weak, &[], None).unwrap()),
            (HysteresisHold::LowConfidence, NOW + DAY)
        );

        // Multi-step or band-targeting automatic moves are refused.
        let jump = proposed("700", 70, 40, MoveOrigin::Automatic, 95);
        assert_eq!(
            hold_of(decide(&jump, &[], None).unwrap()).0,
            HysteresisHold::OneStepOnly
        );
        let to_band = proposed("700", 30, NEXT_UP_VALUE, MoveOrigin::Automatic, 95);
        assert_eq!(
            hold_of(decide(&to_band, &[], None).unwrap()).0,
            HysteresisHold::OneStepOnly
        );

        // Cooldown after any applied move — including a manual one, so
        // automation cannot immediately undo a person's decision.
        let recent_manual = applied("700", 30, 40, MoveOrigin::Manual, NOW - DAY / 2);
        assert_eq!(
            hold_of(decide(&step, std::slice::from_ref(&recent_manual), None).unwrap()),
            (HysteresisHold::Cooldown, NOW + DAY / 2)
        );
        // Another item's history does not cool this one down.
        let other = applied("701", 30, 40, MoveOrigin::Automatic, NOW - 1);
        assert_eq!(decide(&step, &[other], None).unwrap(), allow);

        // Net drift is bounded: two automatic raises in the window
        // already, a third is held until the oldest leaves the window.
        let drift = [
            applied("700", 70, 50, MoveOrigin::Automatic, NOW - 20 * DAY),
            applied("700", 50, 40, MoveOrigin::Automatic, NOW - 10 * DAY),
        ];
        assert_eq!(
            hold_of(decide(&step, &drift, None).unwrap()),
            (HysteresisHold::DriftBound, NOW + 10 * DAY)
        );
        // Oscillation nets out: a raise then a lower is no drift.
        let wobble = [
            applied("700", 50, 40, MoveOrigin::Automatic, NOW - 20 * DAY),
            applied("700", 40, 50, MoveOrigin::Automatic, NOW - 10 * DAY),
        ];
        let lower = proposed("700", 50, 40, MoveOrigin::Automatic, 90);
        assert_eq!(decide(&lower, &wobble, None).unwrap(), allow);
        // Moves outside the window no longer count.
        let old = [
            applied("700", 70, 50, MoveOrigin::Automatic, NOW - 40 * DAY),
            applied("700", 50, 40, MoveOrigin::Automatic, NOW - 35 * DAY),
        ];
        assert_eq!(decide(&step, &old, None).unwrap(), allow);
        // Manual moves are not automatic drift.
        let manual_history = [
            applied("700", 70, 50, MoveOrigin::Manual, NOW - 20 * DAY),
            applied("700", 50, 40, MoveOrigin::Manual, NOW - 10 * DAY),
        ];
        assert_eq!(decide(&step, &manual_history, None).unwrap(), allow);

        // An authorised manual move bypasses every resistance.
        let mut manual = proposed("700", 70, 20, MoveOrigin::Manual, 0);
        assert_eq!(
            decide(&manual, &drift, Some(&agent)).unwrap(),
            HysteresisDecision::Allow {
                bypass: Some(HysteresisBypass::Manual)
            }
        );
        // … but not authorisation: no principal or no capability fails.
        assert_eq!(
            decide(&manual, &drift, None).unwrap_err().code(),
            "unauthorized_input"
        );
        let reader = AuthenticatedPrincipal {
            principal_ref: "principal:reader".to_string(),
            capabilities: vec!["priority.read".to_string()],
        };
        assert_eq!(
            decide(&manual, &drift, Some(&reader)).unwrap_err().code(),
            "unauthorized_input"
        );

        // A verified deadline or incident bypasses resistance only with
        // evidence.
        let mut deadline = proposed("700", 40, 20, MoveOrigin::VerifiedDeadline, 10);
        assert_eq!(
            decide(&deadline, &drift, None).unwrap_err().code(),
            "unauthorized_input"
        );
        deadline.evidence_ref = Some("deadline:release".to_string());
        assert_eq!(
            decide(&deadline, &drift, None).unwrap(),
            HysteresisDecision::Allow {
                bypass: Some(HysteresisBypass::Deadline)
            }
        );
        let mut incident = proposed("700", 40, 10, MoveOrigin::VerifiedIncident, 10);
        incident.evidence_ref = Some("incident:sev1".to_string());
        assert_eq!(
            decide(&incident, &[recent_manual.clone()], None).unwrap(),
            HysteresisDecision::Allow {
                bypass: Some(HysteresisBypass::Incident)
            }
        );

        // Malformed inputs and policies are refused.
        manual.to = manual.from.clone();
        assert_eq!(
            decide(&manual, &[], Some(&agent)).unwrap_err().code(),
            "invalid_profile"
        );
        let mut bad = hysteresis_policy();
        bad.cooldown_secs = 0;
        assert_eq!(bad.validate().unwrap_err().code(), "invalid_profile");
        bad = hysteresis_policy();
        bad.min_confidence_pct = 101;
        assert_eq!(bad.validate().unwrap_err().code(), "invalid_profile");
        bad = hysteresis_policy();
        bad.schema_version = "fleet.priority-hysteresis-policy/v2".to_string();
        assert_eq!(bad.validate().unwrap_err().code(), "unsupported_version");
    }

    #[test]
    fn pmo_priority_hysteresis_cannot_bypass_owner_policy() {
        let agent = principal("principal:agent");
        let drew = principal("principal:drew");

        // No bypass reaches a protected item: manual, verified deadline,
        // verified incident and automatic all deny.
        let mut deadline = proposed("3447", EXPEDITE_VALUE, 10, MoveOrigin::VerifiedDeadline, 99);
        deadline.evidence_ref = Some("deadline:x".to_string());
        let mut incident = proposed("3447", EXPEDITE_VALUE, 10, MoveOrigin::VerifiedIncident, 99);
        incident.evidence_ref = Some("incident:x".to_string());
        let manual = proposed("3447", EXPEDITE_VALUE, 40, MoveOrigin::Manual, 99);
        let automatic = proposed("3447", EXPEDITE_VALUE, 20, MoveOrigin::Automatic, 99);
        for (mv, who) in [
            (&deadline, Some(&drew)),
            (&incident, Some(&drew)),
            (&manual, Some(&drew)),
            (&manual, Some(&agent)),
            (&automatic, None),
        ] {
            assert_eq!(
                decide(mv, &[], who).unwrap_err().code(),
                "OWNER_POLICY_PROTECTED",
                "{:?} must not bypass the owner policy",
                mv.origin
            );
        }

        // Only the authenticated owner acting as owner may move it.
        let owner = proposed("3447", EXPEDITE_VALUE, 40, MoveOrigin::OwnerAction, 0);
        assert_eq!(
            decide(&owner, &[], Some(&drew)).unwrap(),
            HysteresisDecision::Allow {
                bypass: Some(HysteresisBypass::Manual)
            }
        );
        assert_eq!(
            decide(&owner, &[], Some(&agent)).unwrap_err().code(),
            "OWNER_POLICY_PROTECTED"
        );

        // Stale, missing and unverifiable policy all fail closed, even
        // for a verified bypass on ordinary work.
        let mut ordinary = proposed("700", 40, 20, MoveOrigin::VerifiedIncident, 99);
        ordinary.evidence_ref = Some("incident:y".to_string());
        ordinary.owner_policy_ref = Some("owner-policy:6".to_string());
        assert_eq!(
            decide(&ordinary, &[], None).unwrap_err().code(),
            "OWNER_POLICY_VERSION_CONFLICT"
        );
        ordinary.owner_policy_ref = None;
        assert_eq!(
            decide(&ordinary, &[], None).unwrap_err().code(),
            "owner_policy_ref_missing"
        );
        ordinary.owner_policy_ref = Some("owner-policy:7".to_string());
        assert_eq!(
            evaluate_hysteresis(
                &queue(),
                &hysteresis_policy(),
                &ordinary,
                &[],
                None,
                Some(&membership()),
                None
            )
            .unwrap_err()
            .code(),
            "owner_policy_unavailable"
        );
    }
}
