//! Managed body section capability policy — AUTH-04
//! (developing-today/code#9431; design parent #9208, program #9196).
//!
//! A Fleet-managed section inside a GitHub issue/PR body may be
//! mirrored only under an honest write guarantee. The GitHub body
//! update contract documents PATCH with a replacement body, but no
//! conditional compare-and-swap: a read/compare/PATCH sequence
//! cannot prevent a human edit landing in the gap. This module is
//! the capability policy that follows from that fact:
//!
//! * **Default: unsupported.** Absent a *verified* conditional-write
//!   capability, a managed body section update is denied — not
//!   attempted best-effort, and never described as optimistic
//!   locking (a pre-read check is not a lock).
//! * **Opt-in best-effort is a separate gate.** A weaker
//!   marked-section update may be enabled only by an explicit
//!   approval that acknowledges the human-clobber risk. Its
//!   guarantee is labelled best-effort/no-CAS, with the race
//!   disclosed, never optimistic locking.
//! * **Refusals are typed and non-mutating.** Malformed or duplicate
//!   markers, outside-section bytes altered since the baseline
//!   snapshot, and a concurrent edit inside the section each refuse
//!   the update with a distinct reason; nothing is planned.
//! * **A timeout after PATCH is unknown, not failure or success.**
//!   The write may have applied. The only sound next step is to
//!   re-read and reconcile against the baseline and the intended
//!   section before any new plan.
//!
//! This is a pure policy: it parses, compares, and plans. It does
//! no I/O, holds no state, and touches no GitHub adapter — an
//! adapter may act on a [`PlannedUpdate`] only, and only a verified
//! capability (proven against the real API contract, recorded as
//! evidence) changes the default.

/// The exact start marker of a Fleet-managed body section.
pub const SECTION_START: &str = "<!-- fleet:managed-section:start -->";
/// The exact end marker of a Fleet-managed body section.
pub const SECTION_END: &str = "<!-- fleet:managed-section:end -->";

/// Whether the target API/adapter has a *verified* conditional
/// write (compare-and-swap) for body updates. Verification means
/// evidence against the documented API contract, not a pre-read
/// comparison performed by the caller.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ConditionalWriteCapability {
    /// No conditional-write capability has been proven. This is
    /// the state of the documented GitHub body update contract
    /// today, and the default for every new target.
    #[default]
    Unverified,
    /// A conditional write was proven; `evidence` names the proof
    /// (contract citation or capability test run) so the claim is
    /// auditable rather than asserted.
    Verified {
        /// What established the capability.
        evidence: String,
    },
}

impl ConditionalWriteCapability {
    /// True only for a proven capability with recorded evidence.
    pub fn is_verified(&self) -> bool {
        match self {
            ConditionalWriteCapability::Verified { evidence } => !evidence.trim().is_empty(),
            ConditionalWriteCapability::Unverified => false,
        }
    }
}

/// The separate, explicit approval that alone can enable the
/// weaker best-effort mode. It is not a capability and never
/// upgrades the guarantee: it only acknowledges the risk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BestEffortOptIn {
    /// Who approved best-effort mode (an owner identity, recorded
    /// so the approval is attributable).
    pub approved_by: String,
    /// The approval must explicitly acknowledge that a human edit
    /// in the read/PATCH gap can be clobbered. Anything less is
    /// not an opt-in.
    pub acknowledges_human_clobber_risk: bool,
}

impl BestEffortOptIn {
    /// An opt-in counts only with an attributable approver and an
    /// explicit human-clobber-risk acknowledgment.
    pub fn is_valid(&self) -> bool {
        self.acknowledges_human_clobber_risk && !self.approved_by.trim().is_empty()
    }
}

/// The honest guarantee a planned update carries. Callers must
/// surface this label as-is; in particular the no-CAS variants
/// must never be relabelled as optimistic locking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Guarantee {
    /// No update may be attempted at all.
    Unsupported,
    /// Best-effort marked-section update under an explicit opt-in:
    /// outside-section bytes are preserved exactly and the section
    /// is baseline-checked, but a human edit in the read/PATCH gap
    /// can still be clobbered. The race is disclosed, not hidden.
    BestEffortNoCas,
    /// The API/adapter proved a conditional write; the update is
    /// applied only if the remote value still matches the
    /// baseline the plan was made against.
    VerifiedConditionalWrite,
}

impl Guarantee {
    /// The user-facing label for this guarantee.
    pub fn label(&self) -> &'static str {
        match self {
            Guarantee::Unsupported => "unsupported",
            Guarantee::BestEffortNoCas => "best-effort (no CAS; human-clobber race disclosed)",
            Guarantee::VerifiedConditionalWrite => "verified conditional write",
        }
    }
}

/// A body split at its managed-section markers. All three parts
/// are exact slices of the original body: concatenating
/// `outside_prefix + SECTION_START + section + SECTION_END +
/// outside_suffix` reproduces the body byte for byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedBody {
    /// Bytes before the start marker, preserved exactly.
    pub outside_prefix: String,
    /// Bytes between the markers (the managed section itself).
    pub section: String,
    /// Bytes after the end marker, preserved exactly.
    pub outside_suffix: String,
}

impl ParsedBody {
    /// Reassemble a body with `new_section` in place of the
    /// current section, preserving outside bytes exactly.
    pub fn with_section(&self, new_section: &str) -> String {
        let mut out = String::with_capacity(
            self.outside_prefix.len()
                + SECTION_START.len()
                + new_section.len()
                + SECTION_END.len()
                + self.outside_suffix.len(),
        );
        out.push_str(&self.outside_prefix);
        out.push_str(SECTION_START);
        out.push_str(new_section);
        out.push_str(SECTION_END);
        out.push_str(&self.outside_suffix);
        out
    }
}

/// Why a body has no well-formed managed section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MalformedMarkers {
    /// No start marker.
    MissingStart,
    /// No end marker.
    MissingEnd,
    /// More than one start or end marker: the managed section is
    /// ambiguous and must not be guessed at.
    DuplicateMarkers,
    /// The end marker precedes the start marker.
    EndBeforeStart,
}

/// Split `body` at its managed-section markers. Refuses anything
/// but exactly one start marker followed by exactly one end
/// marker: a malformed section is a conflict to surface, never a
/// shape to repair silently.
pub fn parse_managed_section(body: &str) -> Result<ParsedBody, MalformedMarkers> {
    let starts: Vec<usize> = body.match_indices(SECTION_START).map(|(i, _)| i).collect();
    let ends: Vec<usize> = body.match_indices(SECTION_END).map(|(i, _)| i).collect();
    if starts.len() > 1 || ends.len() > 1 {
        return Err(MalformedMarkers::DuplicateMarkers);
    }
    let start = starts
        .first()
        .copied()
        .ok_or(MalformedMarkers::MissingStart)?;
    let end = ends.first().copied().ok_or(MalformedMarkers::MissingEnd)?;
    if end < start {
        return Err(MalformedMarkers::EndBeforeStart);
    }
    Ok(ParsedBody {
        outside_prefix: body[..start].to_string(),
        section: body[start + SECTION_START.len()..end].to_string(),
        outside_suffix: body[end + SECTION_END.len()..].to_string(),
    })
}

/// Why an update was not planned. Every variant is non-mutating:
/// the remote body stays exactly as it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Default state: no verified conditional-write capability
    /// and no valid best-effort opt-in. The update is unsupported,
    /// not merely risky.
    DeniedUnsupported,
    /// The baseline or current body has no well-formed managed
    /// section.
    MalformedMarkers(MalformedMarkers),
    /// Bytes outside the managed section differ from the baseline
    /// snapshot. The update would have to reason about content it
    /// does not own, so it refuses.
    OutsideBytesAltered,
    /// The section itself changed since the baseline snapshot —
    /// a concurrent edit (human or Fleet) landed in the gap.
    ConcurrentEdit,
}

/// An update an adapter may apply: the full replacement body
/// (outside bytes preserved exactly from the *current* body,
/// which was verified equal to the baseline outside the section)
/// and the honest guarantee it carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedUpdate {
    /// Full body to send, with the new section in place.
    pub body: String,
    /// The guarantee this update actually has.
    pub guarantee: Guarantee,
}

/// Decide whether a managed body section update may proceed, and
/// plan it if so.
///
/// Order is deliberate: the capability gate comes first, so an
/// unsupported target is denied even against a raced or malformed
/// body — the denial is about the guarantee, not the input. Only
/// a verified capability or a valid best-effort opt-in reaches
/// the structural checks (markers, outside bytes, concurrent
/// edit), each of which refuses without planning a mutation.
pub fn plan_update(
    capability: &ConditionalWriteCapability,
    best_effort_opt_in: Option<&BestEffortOptIn>,
    baseline_body: &str,
    current_body: &str,
    new_section: &str,
) -> Result<PlannedUpdate, Refusal> {
    let guarantee = if capability.is_verified() {
        Guarantee::VerifiedConditionalWrite
    } else if best_effort_opt_in.is_some_and(BestEffortOptIn::is_valid) {
        Guarantee::BestEffortNoCas
    } else {
        return Err(Refusal::DeniedUnsupported);
    };
    let baseline = parse_managed_section(baseline_body).map_err(Refusal::MalformedMarkers)?;
    let current = parse_managed_section(current_body).map_err(Refusal::MalformedMarkers)?;
    if current.outside_prefix != baseline.outside_prefix
        || current.outside_suffix != baseline.outside_suffix
    {
        return Err(Refusal::OutsideBytesAltered);
    }
    if current.section != baseline.section {
        return Err(Refusal::ConcurrentEdit);
    }
    Ok(PlannedUpdate {
        body: current.with_section(new_section),
        guarantee,
    })
}

/// What a PATCH whose acknowledgment was lost to a timeout means.
/// The write may or may not have applied; claiming either without
/// a re-read would be inventing an outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeoutResolution {
    /// The re-read body carries the intended section and the
    /// baseline's outside bytes: the PATCH did apply.
    Applied,
    /// The re-read body is still exactly the baseline: the PATCH
    /// did not apply, and a fresh plan may be made against it.
    NotApplied,
    /// The re-read body differs from both: someone else edited
    /// the body in the gap. Surface a conflict; do not retry over
    /// their edit.
    Diverged,
    /// No re-read is available yet: the outcome is still unknown
    /// and no retry is sound.
    StillUnknown,
}

/// Reconcile a timed-out PATCH by re-reading. `current_body` is
/// the freshly read body, or `None` if the re-read has not
/// happened — in which case the resolution stays unknown.
pub fn reconcile_after_patch_timeout(
    baseline_body: &str,
    intended_section: &str,
    current_body: Option<&str>,
) -> TimeoutResolution {
    let Some(current_body) = current_body else {
        return TimeoutResolution::StillUnknown;
    };
    let Ok(baseline) = parse_managed_section(baseline_body) else {
        return TimeoutResolution::Diverged;
    };
    let Ok(current) = parse_managed_section(current_body) else {
        return TimeoutResolution::Diverged;
    };
    if current.outside_prefix != baseline.outside_prefix
        || current.outside_suffix != baseline.outside_suffix
    {
        return TimeoutResolution::Diverged;
    }
    if current.section == intended_section {
        TimeoutResolution::Applied
    } else if current.section == baseline.section {
        TimeoutResolution::NotApplied
    } else {
        TimeoutResolution::Diverged
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn baseline_body() -> String {
        format!("human intro\n{SECTION_START}\nmanaged v1\n{SECTION_END}\nhuman outro\n")
    }

    fn verified() -> ConditionalWriteCapability {
        ConditionalWriteCapability::Verified {
            evidence: "capability test against the documented API contract".to_string(),
        }
    }

    /// Oracle for AUTH-04 (code#9431): a concurrent human edit
    /// lands between the GET and the PATCH. Without a verified
    /// conditional-write capability the update is unsupported /
    /// default-denied — the race is exactly the case the default
    /// exists for. With a verified capability the same race is a
    /// typed concurrent-edit conflict, and without the race the
    /// verified update plans cleanly.
    #[test]
    fn pmo_auth_managed_body_requires_verified_conditional_write_capability() {
        let baseline = baseline_body();
        // The concurrent human GET/PATCH race: the section changed
        // after our baseline snapshot was taken.
        let raced = baseline.replace("managed v1", "human edit in the gap");

        let denied = plan_update(
            &ConditionalWriteCapability::Unverified,
            None,
            &baseline,
            &raced,
            "\nmanaged v2\n",
        );
        assert_eq!(denied, Err(Refusal::DeniedUnsupported));
        assert_eq!(Guarantee::Unsupported.label(), "unsupported");

        // Even an unraced update is denied by default: unsupported
        // is the absence of a guarantee, not a property of the
        // input body.
        let denied_clean = plan_update(
            &ConditionalWriteCapability::Unverified,
            None,
            &baseline,
            &baseline,
            "\nmanaged v2\n",
        );
        assert_eq!(denied_clean, Err(Refusal::DeniedUnsupported));

        // Verified capability: the race surfaces as a conflict,
        // and the clean update plans with the verified guarantee.
        let conflict = plan_update(&verified(), None, &baseline, &raced, "\nmanaged v2\n");
        assert_eq!(conflict, Err(Refusal::ConcurrentEdit));
        let planned = plan_update(&verified(), None, &baseline, &baseline, "\nmanaged v2\n")
            .expect("verified capability with a clean baseline must plan");
        assert_eq!(planned.guarantee, Guarantee::VerifiedConditionalWrite);
        assert!(planned.body.contains("managed v2"));
    }

    #[test]
    fn outside_bytes_altered_refuses_even_with_verified_capability() {
        let baseline = baseline_body();
        let altered = baseline.replace("human outro", "human rewrote the outro");
        let refused = plan_update(&verified(), None, &baseline, &altered, "\nmanaged v2\n");
        assert_eq!(refused, Err(Refusal::OutsideBytesAltered));
    }

    #[test]
    fn malformed_markers_refuse_the_update() {
        let baseline = baseline_body();
        for body in [
            "no markers at all".to_string(),
            format!("only start {SECTION_START} section"),
            format!("only end section {SECTION_END}"),
            format!("{SECTION_END} backwards {SECTION_START}"),
            format!("{SECTION_START} one {SECTION_START} two {SECTION_END}"),
            format!("{SECTION_START} one {SECTION_END} two {SECTION_END}"),
        ] {
            let refused = plan_update(&verified(), None, &baseline, &body, "\nmanaged v2\n");
            assert!(
                matches!(refused, Err(Refusal::MalformedMarkers(_))),
                "body {body:?} must refuse as malformed, got {refused:?}"
            );
        }
    }

    #[test]
    fn best_effort_requires_explicit_opt_in_and_is_never_called_optimistic_locking() {
        let baseline = baseline_body();
        // An opt-in that does not acknowledge the human-clobber
        // risk, or names no approver, is not an opt-in.
        for opt_in in [
            BestEffortOptIn {
                approved_by: "owner".to_string(),
                acknowledges_human_clobber_risk: false,
            },
            BestEffortOptIn {
                approved_by: String::new(),
                acknowledges_human_clobber_risk: true,
            },
        ] {
            let denied = plan_update(
                &ConditionalWriteCapability::Unverified,
                Some(&opt_in),
                &baseline,
                &baseline,
                "\nmanaged v2\n",
            );
            assert_eq!(denied, Err(Refusal::DeniedUnsupported));
        }
        // A valid opt-in plans the update, labelled honestly.
        let opt_in = BestEffortOptIn {
            approved_by: "owner".to_string(),
            acknowledges_human_clobber_risk: true,
        };
        let planned = plan_update(
            &ConditionalWriteCapability::Unverified,
            Some(&opt_in),
            &baseline,
            &baseline,
            "\nmanaged v2\n",
        )
        .expect("valid opt-in must plan a best-effort update");
        assert_eq!(planned.guarantee, Guarantee::BestEffortNoCas);
        for guarantee in [
            Guarantee::Unsupported,
            Guarantee::BestEffortNoCas,
            Guarantee::VerifiedConditionalWrite,
        ] {
            assert!(
                !guarantee.label().contains("optimistic"),
                "no-CAS guarantees must never be labelled optimistic locking"
            );
        }
    }

    #[test]
    fn planned_update_preserves_outside_bytes_exactly() {
        let baseline = baseline_body();
        let planned = plan_update(&verified(), None, &baseline, &baseline, "\nmanaged v2\n")
            .expect("clean verified update must plan");
        assert_eq!(
            planned.body,
            format!("human intro\n{SECTION_START}\nmanaged v2\n{SECTION_END}\nhuman outro\n")
        );
    }

    #[test]
    fn timeout_after_patch_is_unknown_until_reconciled() {
        let baseline = baseline_body();
        // No re-read yet: still unknown, never assumed either way.
        assert_eq!(
            reconcile_after_patch_timeout(&baseline, "\nmanaged v2\n", None),
            TimeoutResolution::StillUnknown
        );
        // Re-read shows the intended section: it applied.
        let applied = baseline.replace("managed v1", "managed v2");
        assert_eq!(
            reconcile_after_patch_timeout(&baseline, "\nmanaged v2\n", Some(&applied)),
            TimeoutResolution::Applied
        );
        // Re-read shows the untouched baseline: it did not apply.
        assert_eq!(
            reconcile_after_patch_timeout(&baseline, "\nmanaged v2\n", Some(&baseline)),
            TimeoutResolution::NotApplied
        );
        // Re-read shows a third state: someone else edited; the
        // timeout must not become a retry over their edit.
        let diverged = baseline.replace("managed v1", "someone else was here");
        assert_eq!(
            reconcile_after_patch_timeout(&baseline, "\nmanaged v2\n", Some(&diverged)),
            TimeoutResolution::Diverged
        );
    }
}
