//! Sanctions boundary data model (#5683 step 4, #7785; design
//! `doc/2026-11-08_design_id5683.md` §5, decision record for #5878 in
//! `doc/2026-11-10T19-52-00Z_design_id_sanctions_boundary/`).
//!
//! Sanctions boundaries are **data, not code**: a boundary reads a
//! versioned list ([`ListVersion`]) that an operator supplied or
//! activated from the shipped starting point. Nothing in this module
//! fetches an external feed (design §11 defers that), and a list that
//! exists enforces nothing until a [`SanctionsBoundary`] declaration
//! references it — capabilities, never defaults.
//!
//! Evaluation is deliberately small. The framework's evidence model
//! (see [`crate::boundary`]) decides what a subject's evidence is;
//! this module decides what a list says about a subject and what a
//! boundary does with that: a match on list-native identifiers is
//! *strong*, a name-only resemblance is *weak* and can only flag for
//! review, and a subject who already has access is never cut off by
//! a list entry appearing — they are flagged for review instead
//! (grandfathering, §5.3). Every deny names its appeal path (§5.4),
//! and every decision records the exact list version applied (§5.1)
//! so a past decision is reproducible.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::boundary::{EnforcementPoint, ListVersionRef};

/// Where a list's content came from (§5.1, decision D1 of #5878).
/// Either way the list is inert until an operator activates it for a
/// boundary; the framework asserts no derivation is complete or
/// legally sufficient.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ListSource {
    /// Supplied by the operator who owns the boundary, with the
    /// derivation recorded (what external list it was derived from,
    /// and that list's publication date, as free text).
    OperatorSupplied {
        /// How this list was derived, recorded verbatim.
        derivation: String,
    },
    /// Shipped with the software as a starting point. The label is
    /// part of the data so no consumer can mistake it for advice.
    ShippedStartingPoint {
        /// The standing label: a starting point, not legal advice.
        label: String,
    },
}

impl ListSource {
    /// The label a shipped starting-point list must carry (§5.1).
    pub const SHIPPED_LABEL: &'static str = "starting point — not legal advice";

    /// A shipped starting-point source carrying the required label.
    #[must_use]
    pub fn shipped() -> Self {
        Self::ShippedStartingPoint {
            label: Self::SHIPPED_LABEL.to_string(),
        }
    }
}

/// One entry of a sanctions list. Entries match on list-native keys:
/// a country entry carries its ISO 3166-1 alpha-2 code; entity and
/// person entries carry the source list's own entry id plus the
/// identifiers the source publishes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum SanctionsEntry {
    /// A country, by ISO 3166-1 alpha-2 code (e.g. `"CU"`).
    Country {
        /// ISO 3166-1 alpha-2 country code.
        code: String,
    },
    /// An organisation or vessel on the source list.
    Entity {
        /// The source list's native id for this entry.
        native_id: String,
        /// Names the source publishes for the entity.
        names: BTreeSet<String>,
    },
    /// A named individual on the source list.
    Person {
        /// The source list's native id for this entry.
        native_id: String,
        /// Names the source publishes for the person.
        names: BTreeSet<String>,
        /// Date of birth as published, when the source carries one.
        date_of_birth: Option<String>,
    },
}

impl SanctionsEntry {
    /// The stable match key of this entry within its list: the
    /// country code, or the source list's native id. Keys are how
    /// decisions and review items name the entry that matched.
    #[must_use]
    pub fn key(&self) -> &str {
        match self {
            Self::Country { code } => code,
            Self::Entity { native_id, .. } | Self::Person { native_id, .. } => native_id,
        }
    }

    /// The names this entry publishes, for weak (name-only) matching.
    /// Country entries publish no names.
    #[must_use]
    pub fn names(&self) -> Option<&BTreeSet<String>> {
        match self {
            Self::Country { .. } => None,
            Self::Entity { names, .. } | Self::Person { names, .. } => Some(names),
        }
    }
}

/// One immutable version of a sanctions list (§5.1). Any change — an
/// entry appearing, disappearing, or being corrected — is a new
/// version; versions are never edited in place, so a decision taken
/// under version *n* stays reproducible against version *n*.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListVersion {
    /// The list, named by its supplier.
    pub id: String,
    /// Monotone version within the list, starting at 1.
    pub version: u64,
    /// Where the content came from.
    pub source: ListSource,
    /// Unix seconds from which this version may be enforced.
    pub effective_from: u64,
    /// Unix seconds at which this version stopped being current, if
    /// it has been superseded.
    pub effective_to: Option<u64>,
    /// The entries in force under this version.
    pub entries: Vec<SanctionsEntry>,
    /// Signature over the list content as supplied. Carried as data;
    /// verifying it against a supplier key is the activation path's
    /// job, not evaluation's.
    pub signature: String,
}

impl ListVersion {
    /// The reference a decision records for this version (§5.1).
    #[must_use]
    pub fn reference(&self) -> ListVersionRef {
        ListVersionRef {
            id: self.id.clone(),
            version: self.version,
        }
    }

    /// Whether this version is the one in force at `now`: at or
    /// after its effective date and not yet superseded.
    #[must_use]
    pub fn is_active_at(&self, now: u64) -> bool {
        if now < self.effective_from {
            return false;
        }
        match self.effective_to {
            Some(to) => now < to,
            None => true,
        }
    }

    /// The entry with match key `key`, if this version carries one.
    #[must_use]
    pub fn entry(&self, key: &str) -> Option<&SanctionsEntry> {
        self.entries.iter().find(|e| e.key() == key)
    }
}

/// How strongly a subject matched a list entry. The distinction is
/// the false-positive defence (§5.4): only a strong match can ever
/// deny, and even it cannot cut off existing access (see
/// [`evaluate`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchBasis {
    /// Name-only resemblance. Expected false-positive territory:
    /// name collisions, dual citizens, travellers (§5.4).
    WeakNameOnly,
    /// A list-native identifier matched: a country entry against the
    /// subject's evidenced jurisdiction, or an entity/person native
    /// id the subject presented.
    StrongIdentifier,
}

/// A subject's match against one entry of the list being evaluated.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubjectMatch {
    /// Match key of the entry that matched ([`SanctionsEntry::key`]).
    pub entry_key: String,
    /// How the match was established.
    pub basis: MatchBasis,
}

/// Find a subject's matches in a list version. Country entries match
/// strongly when the subject's evidenced country code equals the
/// entry's code; entity/person entries match strongly on a shared
/// native id and weakly on a shared name. Evidence itself — what the
/// subject's country, ids and names *are* — belongs to the
/// framework's evidence model; this function only compares.
#[must_use]
pub fn find_matches(
    list: &ListVersion,
    subject_country: Option<&str>,
    subject_native_ids: &BTreeSet<String>,
    subject_names: &BTreeSet<String>,
) -> Vec<SubjectMatch> {
    let mut matches = Vec::new();
    for entry in &list.entries {
        let basis = match entry {
            SanctionsEntry::Country { code } => subject_country
                .filter(|c| *c == code)
                .map(|_| MatchBasis::StrongIdentifier),
            SanctionsEntry::Entity { native_id, names }
            | SanctionsEntry::Person {
                native_id, names, ..
            } => {
                if subject_native_ids.contains(native_id) {
                    Some(MatchBasis::StrongIdentifier)
                } else if names.iter().any(|n| subject_names.contains(n)) {
                    Some(MatchBasis::WeakNameOnly)
                } else {
                    None
                }
            }
        };
        if let Some(basis) = basis {
            matches.push(SubjectMatch {
                entry_key: entry.key().to_string(),
                basis,
            });
        }
    }
    matches
}

/// A sanctions boundary: the declaration-side half of the model.
/// The list is reusable across boundaries; everything specific to
/// one boundary — which interactions it covers, how a denied
/// subject appeals — lives here, never on the list (#5878 D4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SanctionsBoundary {
    /// The list this boundary reads, by id. Evaluation refuses a
    /// [`ListVersion`] whose id differs, or that is not in force at
    /// the decision time ([`SanctionsEvaluationError`]); the decision
    /// records the exact version applied.
    pub list_id: String,
    /// Interactions this boundary covers (§5.2). An omitted
    /// interaction is not covered; the set may not be empty.
    pub scope: BTreeSet<EnforcementPoint>,
    /// How a denied or flagged subject requests review. Named in
    /// every deny (§5.4); may not be empty.
    pub appeal_path: String,
}

/// Why a sanctions boundary was refused at declaration time.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SanctionsError {
    /// A boundary covering no interaction enforces nothing: a false
    /// promise, rejected like the declaration's own
    /// enabled-without-enforcement rule (§2.3, §5.2).
    #[error("sanctions boundary names no enforcement point in its scope")]
    EmptyScope,
    /// A boundary whose denies cannot name an appeal path must not
    /// be declared (§5.4).
    #[error("sanctions boundary names no appeal path")]
    EmptyAppealPath,
    /// The list identity is empty.
    #[error("sanctions boundary list id is empty")]
    EmptyListId,
}

impl SanctionsBoundary {
    /// Build a boundary, refusing the shapes the design forbids: no
    /// list, no scope, or no appeal path.
    ///
    /// # Errors
    /// Returns [`SanctionsError`] naming the empty field.
    pub fn new(
        list_id: String,
        scope: BTreeSet<EnforcementPoint>,
        appeal_path: String,
    ) -> Result<Self, SanctionsError> {
        if list_id.is_empty() {
            return Err(SanctionsError::EmptyListId);
        }
        if scope.is_empty() {
            return Err(SanctionsError::EmptyScope);
        }
        if appeal_path.is_empty() {
            return Err(SanctionsError::EmptyAppealPath);
        }
        Ok(Self {
            list_id,
            scope,
            appeal_path,
        })
    }
}

/// Why a sanctions evaluation was refused before any decision was
/// recorded (#8376). Evaluation binds to the boundary's declared
/// list: a decision computed against another list, or against a
/// version outside its effective window, would enforce the wrong
/// operator policy — or clear a restriction incorrectly — under an
/// audit record that cannot reproduce the asserted match.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SanctionsEvaluationError {
    /// The list evaluated against is not the list the boundary
    /// declares.
    #[error("sanctions boundary reads list {expected}, but evaluation was given list {actual}")]
    WrongList {
        /// The boundary's declared list id.
        expected: String,
        /// The id of the list actually passed.
        actual: String,
    },
    /// The list version is not in force at the decision time.
    #[error("list {id} version {version} is not in force at {now}")]
    ListNotActive {
        /// The list id.
        id: String,
        /// The list version.
        version: u64,
        /// The decision time the version was checked against.
        now: u64,
    },
}

/// The binding every evaluation enforces (#8376): the list must be
/// the boundary's declared list, and the version in force at the
/// decision time.
fn check_binding(
    boundary: &SanctionsBoundary,
    list: &ListVersion,
    now: u64,
) -> Result<(), SanctionsEvaluationError> {
    if list.id != boundary.list_id {
        return Err(SanctionsEvaluationError::WrongList {
            expected: boundary.list_id.clone(),
            actual: list.id.clone(),
        });
    }
    if !list.is_active_at(now) {
        return Err(SanctionsEvaluationError::ListNotActive {
            id: list.id.clone(),
            version: list.version,
            now,
        });
    }
    Ok(())
}

/// What a sanctions evaluation decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionOutcome {
    /// No covered match: the interaction proceeds.
    Allow,
    /// A strong match with no existing access to grandfather: the
    /// interaction is refused, naming the appeal path.
    Deny,
    /// The subject goes to the review queue: a weak match, or a
    /// strong match against access the subject already has
    /// (grandfathering, §5.3). Nothing is cut off.
    FlagForReview,
    /// The subject matched under a previous list version and no
    /// longer matches: any flag or restriction from that match is
    /// cleared (§5.3, entry removed).
    Cleared,
}

/// One recorded sanctions decision. The `list` field is the exact
/// version applied, so the decision is reproducible (§5.1); the
/// decision log is a sequence of these.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SanctionsDecision {
    /// The exact list version applied.
    pub list: ListVersionRef,
    /// The interaction that was evaluated.
    pub point: EnforcementPoint,
    /// What was decided.
    pub outcome: DecisionOutcome,
    /// The entry that matched, when one did.
    pub matched_entry: Option<String>,
    /// The appeal path, named on every deny and flag (§5.4). `None`
    /// only on allow/cleared outcomes, where no recourse is needed.
    pub appeal_path: Option<String>,
}

/// Evaluate one interaction for one subject against a boundary.
///
/// The evaluation is bound to the boundary's declared list (#8376):
/// `list` must be that list, and the version in force at `now`, and
/// the subject's matches are computed from that exact list inside
/// this call — a caller cannot supply matches minted against another
/// list or version and have them recorded against this one.
///
/// Rules, in order:
/// 0. A list that is not the boundary's, or not in force at `now`,
///    is refused ([`SanctionsEvaluationError`]); no decision is
///    recorded.
/// 1. An interaction outside the boundary's scope is not covered:
///    allow (§5.2).
/// 2. No match: allow.
/// 3. A weak (name-only) match never denies: flag for review (§5.4).
/// 4. A strong match against a subject who already has access never
///    cuts that access off — an entry appearing flags the subject
///    for review instead (grandfathering, §5.3; acceptance of #7785).
/// 5. A strong match with no existing access denies, naming the
///    boundary's appeal path (§5.4).
///
/// # Errors
/// [`SanctionsEvaluationError`] when the binding (rule 0) fails.
pub fn evaluate(
    boundary: &SanctionsBoundary,
    list: &ListVersion,
    point: EnforcementPoint,
    subject_country: Option<&str>,
    subject_native_ids: &BTreeSet<String>,
    subject_names: &BTreeSet<String>,
    subject_has_existing_access: bool,
    now: u64,
) -> Result<SanctionsDecision, SanctionsEvaluationError> {
    check_binding(boundary, list, now)?;
    let record = |outcome, matched_entry, appeal_path| SanctionsDecision {
        list: list.reference(),
        point,
        outcome,
        matched_entry,
        appeal_path,
    };
    if !boundary.scope.contains(&point) {
        return Ok(record(DecisionOutcome::Allow, None, None));
    }
    let matches = find_matches(list, subject_country, subject_native_ids, subject_names);
    // The strongest match decides; entry keys are compared so the
    // recorded entry is deterministic for a given match set.
    let strongest = matches.iter().max_by(|a, b| {
        a.basis
            .cmp(&b.basis)
            .then_with(|| a.entry_key.cmp(&b.entry_key))
    });
    let Some(m) = strongest else {
        return Ok(record(DecisionOutcome::Allow, None, None));
    };
    let entry = Some(m.entry_key.clone());
    let appeal = Some(boundary.appeal_path.clone());
    Ok(match m.basis {
        MatchBasis::WeakNameOnly => record(DecisionOutcome::FlagForReview, entry, appeal),
        MatchBasis::StrongIdentifier => {
            if subject_has_existing_access {
                record(DecisionOutcome::FlagForReview, entry, appeal)
            } else {
                record(DecisionOutcome::Deny, entry, appeal)
            }
        }
    })
}

/// The removal side of a list change (§5.3): a subject who matched
/// under the previous version and does not match under the current
/// one is cleared, and the clearing is itself a recorded decision
/// against the current version. Returns `None` when the subject did
/// not match before either (nothing to clear).
///
/// Bound like [`evaluate`] (#8376): `current` must be the boundary's
/// declared list and in force at `now`, and the subject's current
/// matches are computed from `current` inside this call — a clearing
/// is never recorded against an unrelated or inactive list.
///
/// # Errors
/// [`SanctionsEvaluationError`] when the binding fails.
pub fn cleared_by_removal(
    boundary: &SanctionsBoundary,
    current: &ListVersion,
    point: EnforcementPoint,
    matched_under_previous: bool,
    subject_country: Option<&str>,
    subject_native_ids: &BTreeSet<String>,
    subject_names: &BTreeSet<String>,
    now: u64,
) -> Result<Option<SanctionsDecision>, SanctionsEvaluationError> {
    check_binding(boundary, current, now)?;
    let matches_now = find_matches(current, subject_country, subject_native_ids, subject_names);
    if !matched_under_previous || !matches_now.is_empty() || !boundary.scope.contains(&point) {
        return Ok(None);
    }
    Ok(Some(SanctionsDecision {
        list: current.reference(),
        point,
        outcome: DecisionOutcome::Cleared,
        matched_entry: None,
        appeal_path: None,
    }))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn list(version: u64, entries: Vec<SanctionsEntry>) -> ListVersion {
        ListVersion {
            id: "operator-list".to_string(),
            version,
            source: ListSource::OperatorSupplied {
                derivation: "derived from the operator's chosen published list".to_string(),
            },
            effective_from: 100,
            effective_to: None,
            entries,
            signature: "sig".to_string(),
        }
    }

    fn boundary() -> SanctionsBoundary {
        SanctionsBoundary::new(
            "operator-list".to_string(),
            BTreeSet::from([EnforcementPoint::AccountCreation, EnforcementPoint::Login]),
            "request review from the server operator".to_string(),
        )
        .unwrap()
    }

    #[test]
    fn boundary_refuses_empty_scope_appeal_path_and_list() {
        // Oracle: design §5.2/§5.4 — a boundary that covers nothing
        // or whose denies cannot name an appeal path is invalid.
        assert_eq!(
            SanctionsBoundary::new("l".into(), BTreeSet::new(), "appeal".into()),
            Err(SanctionsError::EmptyScope)
        );
        assert_eq!(
            SanctionsBoundary::new(
                "l".into(),
                BTreeSet::from([EnforcementPoint::Login]),
                String::new()
            ),
            Err(SanctionsError::EmptyAppealPath)
        );
        assert_eq!(
            SanctionsBoundary::new(
                String::new(),
                BTreeSet::from([EnforcementPoint::Login]),
                "appeal".into()
            ),
            Err(SanctionsError::EmptyListId)
        );
    }

    #[test]
    fn out_of_scope_interaction_is_not_covered() {
        // Oracle: design §5.2 — an omitted interaction is not covered.
        let l = list(1, vec![SanctionsEntry::Country { code: "CU".into() }]);
        let m = find_matches(&l, Some("CU"), &BTreeSet::new(), &BTreeSet::new());
        assert_eq!(m.len(), 1);
        let d = evaluate(
            &boundary(),
            &l,
            EnforcementPoint::BlobStorage,
            Some("CU"),
            &BTreeSet::new(),
            &BTreeSet::new(),
            false,
            100,
        )
        .unwrap();
        assert_eq!(d.outcome, DecisionOutcome::Allow);
        let _ = &m;
    }

    #[test]
    fn strong_match_without_existing_access_denies_and_names_the_appeal_path() {
        // Oracle: #7785 acceptance — a deny on a sanctions boundary
        // always names an appeal path; and §5.1 — the decision
        // references the exact list version applied.
        let l = list(7, vec![SanctionsEntry::Country { code: "CU".into() }]);
        let m = find_matches(&l, Some("CU"), &BTreeSet::new(), &BTreeSet::new());
        let d = evaluate(
            &boundary(),
            &l,
            EnforcementPoint::AccountCreation,
            Some("CU"),
            &BTreeSet::new(),
            &BTreeSet::new(),
            false,
            100,
        )
        .unwrap();
        let _ = &m;
        assert_eq!(d.outcome, DecisionOutcome::Deny);
        assert_eq!(
            d.appeal_path.as_deref(),
            Some("request review from the server operator")
        );
        assert_eq!(
            d.list,
            ListVersionRef {
                id: "operator-list".into(),
                version: 7
            }
        );
        assert_eq!(d.matched_entry.as_deref(), Some("CU"));
    }

    #[test]
    fn weak_name_only_match_flags_and_never_denies() {
        // Oracle: design §5.4 — name collisions are expected false
        // positives; a name-only match is review input, not a deny.
        let l = list(
            1,
            vec![SanctionsEntry::Person {
                native_id: "p-1".into(),
                names: BTreeSet::from(["Alex Example".to_string()]),
                date_of_birth: None,
            }],
        );
        let m = find_matches(
            &l,
            None,
            &BTreeSet::new(),
            &BTreeSet::from(["Alex Example".to_string()]),
        );
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].basis, MatchBasis::WeakNameOnly);
        let d = evaluate(
            &boundary(),
            &l,
            EnforcementPoint::Login,
            None,
            &BTreeSet::new(),
            &BTreeSet::from(["Alex Example".to_string()]),
            false,
            100,
        )
        .unwrap();
        let _ = &m;
        assert_eq!(d.outcome, DecisionOutcome::FlagForReview);
    }

    #[test]
    fn entry_appearing_flags_existing_access_instead_of_cutting_it_off() {
        // Oracle: #7785 acceptance — adding a list entry never
        // removes existing access without a recorded review flag.
        let v1 = list(1, vec![]);
        let v2 = list(2, vec![SanctionsEntry::Country { code: "CU".into() }]);
        let before = find_matches(&v1, Some("CU"), &BTreeSet::new(), &BTreeSet::new());
        assert!(before.is_empty());
        let after = find_matches(&v2, Some("CU"), &BTreeSet::new(), &BTreeSet::new());
        let d = evaluate(
            &boundary(),
            &v2,
            EnforcementPoint::Login,
            Some("CU"),
            &BTreeSet::new(),
            &BTreeSet::new(),
            true,
            100,
        )
        .unwrap();
        let _ = &after;
        assert_eq!(d.outcome, DecisionOutcome::FlagForReview);
        assert_eq!(d.list.version, 2);
    }

    #[test]
    fn entry_removed_clears_a_subject_who_no_longer_matches() {
        // Oracle: design §5.3 — an entry removed clears the subjects
        // it had flagged, and the clearing is logged.
        let current = list(3, vec![]);
        let d = cleared_by_removal(
            &boundary(),
            &current,
            EnforcementPoint::Login,
            true,
            Some("CU"),
            &BTreeSet::new(),
            &BTreeSet::new(),
            100,
        )
        .unwrap()
        .expect("a previously matched subject with no current match is cleared");
        assert_eq!(d.outcome, DecisionOutcome::Cleared);
        assert_eq!(d.list.version, 3);
        // No prior match: nothing to clear.
        assert!(
            cleared_by_removal(
                &boundary(),
                &current,
                EnforcementPoint::Login,
                false,
                Some("CU"),
                &BTreeSet::new(),
                &BTreeSet::new(),
                100,
            )
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn list_version_activity_window_and_reference() {
        let mut l = list(5, vec![]);
        assert!(!l.is_active_at(99));
        assert!(l.is_active_at(100));
        l.effective_to = Some(200);
        assert!(l.is_active_at(199));
        assert!(!l.is_active_at(200));
        assert_eq!(
            l.reference(),
            ListVersionRef {
                id: "operator-list".into(),
                version: 5
            }
        );
    }

    #[test]
    fn evaluation_refuses_a_list_the_boundary_does_not_declare() {
        // Regression for #8376: a boundary for list A evaluated with
        // list B and a strong match from B returned Deny and recorded
        // B. The binding now refuses before any decision.
        let mut other = list(1, vec![SanctionsEntry::Country { code: "CU".into() }]);
        other.id = "someone-elses-list".to_string();
        assert_eq!(
            evaluate(
                &boundary(),
                &other,
                EnforcementPoint::Login,
                Some("CU"),
                &BTreeSet::new(),
                &BTreeSet::new(),
                false,
                100,
            ),
            Err(SanctionsEvaluationError::WrongList {
                expected: "operator-list".to_string(),
                actual: "someone-elses-list".to_string(),
            })
        );
        // And the declared list, which carries no CU entry, allows
        // the same subject: matches are computed from the evaluated
        // list, never imported from another one.
        let declared = list(1, vec![]);
        let d = evaluate(
            &boundary(),
            &declared,
            EnforcementPoint::Login,
            Some("CU"),
            &BTreeSet::new(),
            &BTreeSet::new(),
            false,
            100,
        )
        .unwrap();
        assert_eq!(d.outcome, DecisionOutcome::Allow);
    }

    #[test]
    fn evaluation_refuses_a_version_outside_its_effective_window() {
        // Regression for #8376: neither entry point received an
        // evaluation time, so an inactive version decided as if in
        // force.
        let l = list(1, vec![SanctionsEntry::Country { code: "CU".into() }]);
        assert_eq!(
            evaluate(
                &boundary(),
                &l,
                EnforcementPoint::Login,
                Some("CU"),
                &BTreeSet::new(),
                &BTreeSet::new(),
                false,
                99,
            ),
            Err(SanctionsEvaluationError::ListNotActive {
                id: "operator-list".to_string(),
                version: 1,
                now: 99,
            })
        );
        let mut ended = list(1, vec![SanctionsEntry::Country { code: "CU".into() }]);
        ended.effective_to = Some(200);
        assert!(matches!(
            evaluate(
                &boundary(),
                &ended,
                EnforcementPoint::Login,
                Some("CU"),
                &BTreeSet::new(),
                &BTreeSet::new(),
                false,
                200,
            ),
            Err(SanctionsEvaluationError::ListNotActive { .. })
        ));
    }

    #[test]
    fn clearing_refuses_an_unrelated_or_inactive_list() {
        // Regression for #8376: cleared_by_removal accepted an
        // unrelated or inactive list and recorded a Cleared decision
        // against it.
        let mut other = list(3, vec![]);
        other.id = "unrelated-list".to_string();
        assert!(matches!(
            cleared_by_removal(
                &boundary(),
                &other,
                EnforcementPoint::Login,
                true,
                Some("CU"),
                &BTreeSet::new(),
                &BTreeSet::new(),
                100,
            ),
            Err(SanctionsEvaluationError::WrongList { .. })
        ));
        let current = list(3, vec![]);
        assert!(matches!(
            cleared_by_removal(
                &boundary(),
                &current,
                EnforcementPoint::Login,
                true,
                Some("CU"),
                &BTreeSet::new(),
                &BTreeSet::new(),
                99,
            ),
            Err(SanctionsEvaluationError::ListNotActive { .. })
        ));
    }

    mod property {
        use super::*;
        use proptest::prelude::*;

        fn entry_strategy() -> impl Strategy<Value = SanctionsEntry> {
            prop_oneof![
                "[A-Z]{2}".prop_map(|code| SanctionsEntry::Country { code }),
                "[a-z]{1,3}".prop_map(|native_id| SanctionsEntry::Entity {
                    native_id,
                    names: BTreeSet::new(),
                }),
            ]
        }

        proptest! {
            /// Oracle: #7785 acceptance, as a property — for any list
            /// version pair where the newer version only *adds*
            /// entries, a subject with existing access whose strong
            /// match comes from the added entries is flagged for
            /// review under the new version, never denied.
            #[test]
            fn added_entries_never_deny_existing_access(
                base in prop::collection::vec(entry_strategy(), 0..6),
                added in prop::collection::vec(entry_strategy(), 1..6),
                country in "[A-Z]{2}",
            ) {
                let mut v2_entries = base.clone();
                v2_entries.extend(added.iter().cloned());
                v2_entries.push(SanctionsEntry::Country { code: country.clone() });
                let v2 = list(2, v2_entries);
                // The pushed country entry guarantees a strong match
                // computed from v2 itself; no assumption needed.
                let d = evaluate(
                    &boundary(),
                    &v2,
                    EnforcementPoint::Login,
                    Some(&country),
                    &BTreeSet::new(),
                    &BTreeSet::new(),
                    true,
                    100,
                )
                .unwrap();
                prop_assert_eq!(d.outcome, DecisionOutcome::FlagForReview);
                prop_assert_eq!(d.list.version, 2);
            }

            /// Oracle: design §5.4, as a property — a weak match
            /// never produces a deny, whatever the list contents.
            #[test]
            fn weak_matches_never_deny(
                entries in prop::collection::vec(entry_strategy(), 0..6),
                has_access in any::<bool>(),
            ) {
                let mut entries = entries;
                entries.push(SanctionsEntry::Person {
                    native_id: "p-weak".to_string(),
                    names: BTreeSet::from(["Alex Example".to_string()]),
                    date_of_birth: None,
                });
                let l = list(1, entries);
                // Country is None and native ids are empty, so the
                // only possible match is the weak name-only one.
                let d = evaluate(
                    &boundary(),
                    &l,
                    EnforcementPoint::Login,
                    None,
                    &BTreeSet::new(),
                    &BTreeSet::from(["Alex Example".to_string()]),
                    has_access,
                    100,
                )
                .unwrap();
                prop_assert_eq!(d.outcome, DecisionOutcome::FlagForReview);
            }
        }
    }
}
