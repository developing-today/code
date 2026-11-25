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
}
