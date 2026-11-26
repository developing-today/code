//! Program update discovery and the update envelope decision (#8546,
//! first slice; design of record
//! `doc/2026-11-11T05-24-00Z_design_id_program_ecosystem_v2/` §§4–5, §8).
//!
//! An installed program bundle follows its author's version line: the
//! (author key, name) pair of its D1 identity. This module answers the
//! questions the update flow asks before anything is applied:
//!
//! - [`discover`]: which offered records are verified, newer releases of
//!   the installed line. Discovery reads; it never changes an install, so
//!   a pinned install is only ever reported on.
//! - [`decide_update`]: whether a verified candidate may apply without the
//!   owner's approval. Only a `tracking` install, offered a candidate that
//!   keeps the ABI descriptor, widens no required capability and whose
//!   state migration passed its F12.9 probe, applies without approval.
//!   Every other candidate is `update_approval_required`, carrying the one
//!   fixed summary the owner approves from.
//! - [`updated_install_record`]: the install record after an applied
//!   update. It names the new D1 identity and keeps the owner's policy.
//!
//! Verification comes first and policy second: both entry points take a
//! manifest that already verified ([`BundleRecord::verify`] or
//! [`crate::program_bundle::ProgramBundle::decode`]), so policy is never
//! consulted for an unverified bundle. The decision depends only on the
//! verified manifest, so a candidate fetched from a registry and the same
//! bundle read from a local file get the same answer.
//!
//! What this slice leaves to the apply act (later slices of #8546): running
//! the migration probe ([`crate::world::upgrade_program`] on a
//! copy of the running program), the apply-time quota re-check, writing the
//! new install event, and the per-install update history. Bundles carry no
//! signed changelog yet, so every summary shows that none was provided, and
//! they state no required capabilities, so the host supplies both sides of
//! the capability comparison.

use std::collections::BTreeSet;

use crate::program_bundle::{
    AbiDescriptor, BundleError, BundleManifest, BundleRecord, ProgramBundle, ProgramIdentity,
};
use crate::world::{BundleProvenance, InstallPolicy};

/// What an update summary shows when the author provided no changelog.
/// It is never filled in from anything else (design v2 §6).
pub const NO_CHANGELOG: &str = "no changelog provided by the author";

/// Why an offered bundle is not an update to an install at all.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum UpdateRefusal {
    /// `update_wrong_line`: the bundle belongs to another (author, name)
    /// line. A fork is a different line. It may still be installed as a new
    /// program on a fresh decision, but never as an update.
    #[error(
        "update_wrong_line: {offered_author}/{offered_name} is not the installed line {installed_author}/{installed_name}"
    )]
    WrongLine {
        /// The installed program's author key.
        installed_author: String,
        /// The installed program's name.
        installed_name: String,
        /// The offered bundle's author key.
        offered_author: String,
        /// The offered bundle's name.
        offered_name: String,
    },
    /// The bundle is on the installed line but is not a later release.
    #[error("version {offered} is not newer than the installed version {installed}")]
    NotNewer {
        /// The installed version.
        installed: u64,
        /// The offered version.
        offered: u64,
    },
}

/// One way a candidate falls outside the update envelope (design D4, v2 §5
/// step 2). Each names the envelope term it breaches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnvelopeBreach {
    /// The candidate targets a different guest ABI.
    AbiChanged {
        /// The installed ABI.
        from: AbiDescriptor,
        /// The candidate's ABI.
        to: AbiDescriptor,
    },
    /// The candidate requires capabilities the installed version does not.
    CapabilitiesWidened {
        /// The capabilities added, in name order.
        added: BTreeSet<String>,
    },
    /// The state migration probe failed, so applying would roll back.
    MigrationFailed {
        /// The probe's failure, as reported.
        reason: String,
    },
}

impl EnvelopeBreach {
    /// The envelope term this breach names.
    #[must_use]
    pub const fn term(&self) -> &'static str {
        match self {
            Self::AbiChanged { .. } => "abi_change",
            Self::CapabilitiesWidened { .. } => "capability_widening",
            Self::MigrationFailed { .. } => "migration_failure",
        }
    }
}

/// The outcome of the F12.9 migration probe run on a copy of the running
/// program, before any decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MigrationProbe {
    /// The migrated state round-tripped; the upgrade would commit.
    Passed,
    /// The probe failed; applying would roll back.
    Failed(String),
}

/// The required capabilities of the installed version and the candidate,
/// as the host knows them. Bundles do not state capabilities yet.
#[derive(Clone, Copy, Debug)]
pub struct RequiredCapabilities<'a> {
    /// What the installed version requires.
    pub installed: &'a BTreeSet<String>,
    /// What the candidate requires.
    pub candidate: &'a BTreeSet<String>,
}

/// The one fixed summary an update is decided on (v2 §5 step 3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateSummary {
    /// The installed identity.
    pub from: ProgramIdentity,
    /// The candidate identity.
    pub to: ProgramIdentity,
    /// The author's changelog for the candidate, or [`NO_CHANGELOG`].
    pub changelog: String,
    /// The install's policy, which an applied update keeps.
    pub policy: InstallPolicy,
    /// Every envelope term the candidate breaches; empty inside it.
    pub breaches: Vec<EnvelopeBreach>,
}

/// Whether a candidate applies without the owner's approval.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateDecision {
    /// A tracking install and an in-envelope candidate: the update may
    /// apply without an approval act.
    Applies(UpdateSummary),
    /// `update_approval_required`: a pinned install, or a candidate outside
    /// the envelope. This is a state, not a failure; nothing applies until
    /// the owner approves this summary.
    ApprovalRequired(UpdateSummary),
}

impl UpdateDecision {
    /// The summary the decision was made on.
    #[must_use]
    pub const fn summary(&self) -> &UpdateSummary {
        match self {
            Self::Applies(summary) | Self::ApprovalRequired(summary) => summary,
        }
    }
}

/// Whether a verified candidate is a later release on the installed line.
///
/// # Errors
///
/// [`UpdateRefusal::WrongLine`] for another (author, name) line, and
/// [`UpdateRefusal::NotNewer`] for the same or an earlier version.
pub fn check_line(
    installed: &BundleProvenance,
    candidate: &BundleManifest,
) -> Result<(), UpdateRefusal> {
    let offered = candidate.identity();
    if !installed.program.same_channel(&offered) {
        return Err(UpdateRefusal::WrongLine {
            installed_author: installed.program.author.clone(),
            installed_name: installed.program.name.clone(),
            offered_author: offered.author,
            offered_name: offered.name,
        });
    }
    if offered.version <= installed.program.version {
        return Err(UpdateRefusal::NotNewer {
            installed: installed.program.version,
            offered: offered.version,
        });
    }
    Ok(())
}

/// Decide whether a verified candidate may apply to `installed` without the
/// owner's approval (v2 §5 step 2).
///
/// # Errors
///
/// [`UpdateRefusal`] when the candidate is not an update to this install.
pub fn decide_update(
    installed: &BundleProvenance,
    candidate: &BundleManifest,
    capabilities: RequiredCapabilities<'_>,
    migration: &MigrationProbe,
) -> Result<UpdateDecision, UpdateRefusal> {
    check_line(installed, candidate)?;
    let mut breaches = Vec::new();
    if candidate.abi != installed.abi {
        breaches.push(EnvelopeBreach::AbiChanged {
            from: installed.abi.clone(),
            to: candidate.abi.clone(),
        });
    }
    let added: BTreeSet<String> = capabilities
        .candidate
        .difference(capabilities.installed)
        .cloned()
        .collect();
    if !added.is_empty() {
        breaches.push(EnvelopeBreach::CapabilitiesWidened { added });
    }
    if let MigrationProbe::Failed(reason) = migration {
        breaches.push(EnvelopeBreach::MigrationFailed {
            reason: reason.clone(),
        });
    }
    let summary = UpdateSummary {
        from: installed.program.clone(),
        to: candidate.identity(),
        changelog: NO_CHANGELOG.to_owned(),
        policy: installed.policy,
        breaches,
    };
    if installed.policy == InstallPolicy::Tracking && summary.breaches.is_empty() {
        Ok(UpdateDecision::Applies(summary))
    } else {
        Ok(UpdateDecision::ApprovalRequired(summary))
    }
}

/// What a discovery check found among the offered records.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Discovery {
    /// Verified later releases of the installed line, oldest first, each
    /// identity once.
    pub candidates: Vec<ProgramIdentity>,
    /// Records that failed verification, by their position in the offer,
    /// with the failing check. Policy was never consulted for them.
    pub unverified: Vec<(usize, BundleError)>,
}

/// Check offered records for later releases of the installed line (v2 §4).
///
/// Every record is verified first. Records on other lines, and releases
/// that are not newer, are not candidates. Discovery is the same for both
/// policies and changes nothing: whether a candidate may apply is
/// [`decide_update`]'s question.
#[must_use]
pub fn discover<'a>(
    installed: &BundleProvenance,
    offered: impl IntoIterator<Item = &'a BundleRecord>,
) -> Discovery {
    let mut discovery = Discovery::default();
    for (position, record) in offered.into_iter().enumerate() {
        match record.verify() {
            Ok(manifest) => {
                let identity = manifest.identity();
                if check_line(installed, &manifest).is_ok()
                    && !discovery.candidates.contains(&identity)
                {
                    discovery.candidates.push(identity);
                }
            }
            Err(error) => discovery.unverified.push((position, error)),
        }
    }
    discovery.candidates.sort_by(|a, b| {
        a.version
            .cmp(&b.version)
            .then_with(|| a.content_hash.cmp(&b.content_hash))
    });
    discovery
}

/// The install record after `candidate` is applied over `installed`: the
/// candidate's identity, ABI, lineage, licence and signature, with the
/// owner's policy kept (v2 §5 step 4).
#[must_use]
pub fn updated_install_record(
    installed: &BundleProvenance,
    candidate: &ProgramBundle,
) -> BundleProvenance {
    BundleProvenance {
        policy: installed.policy,
        ..candidate.provenance()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::program_bundle::{ProgramSpec, export, fork};
    use ed25519_dalek::SigningKey;

    const PAYLOAD: &[u8] = b"\0asm\x01\0\0\0 program payload";

    fn alice() -> SigningKey {
        SigningKey::from_bytes(&[1; 32])
    }

    fn bob() -> SigningKey {
        SigningKey::from_bytes(&[2; 32])
    }

    fn release(name: &str, version: u64, author: &SigningKey) -> ProgramBundle {
        let mut payload = PAYLOAD.to_vec();
        payload.extend_from_slice(&version.to_le_bytes());
        export(
            payload,
            ProgramSpec {
                name,
                version,
                licence: Some("MIT"),
                description: None,
                categories: &[],
            },
            author,
        )
        .unwrap()
    }

    fn installed(policy: InstallPolicy) -> BundleProvenance {
        let mut record = release("chess", 3, &alice()).provenance();
        record.policy = policy;
        record
    }

    fn none() -> BTreeSet<String> {
        BTreeSet::new()
    }

    fn decide(
        install: &BundleProvenance,
        candidate: &ProgramBundle,
        caps: (&BTreeSet<String>, &BTreeSet<String>),
        migration: &MigrationProbe,
    ) -> Result<UpdateDecision, UpdateRefusal> {
        decide_update(
            install,
            candidate.manifest(),
            RequiredCapabilities {
                installed: caps.0,
                candidate: caps.1,
            },
            migration,
        )
    }

    #[test]
    fn an_in_envelope_update_applies_under_tracking_and_keeps_the_policy() {
        let install = installed(InstallPolicy::Tracking);
        let next = release("chess", 4, &alice());
        let decision =
            decide(&install, &next, (&none(), &none()), &MigrationProbe::Passed).unwrap();
        let summary = match decision {
            UpdateDecision::Applies(summary) => summary,
            other @ UpdateDecision::ApprovalRequired(_) => {
                panic!("expected Applies, got {other:?}")
            }
        };
        assert_eq!(summary.from, install.program);
        assert_eq!(summary.to, next.identity());
        assert_eq!(summary.changelog, NO_CHANGELOG);
        assert!(summary.breaches.is_empty());

        let after = updated_install_record(&install, &next);
        assert_eq!(after.program, next.identity());
        assert_eq!(after.policy, InstallPolicy::Tracking);
        assert_eq!(after.bundle_signature, next.bundle_signature());
    }

    #[test]
    fn a_pinned_install_needs_approval_even_inside_the_envelope() {
        let install = installed(InstallPolicy::Pinned);
        let next = release("chess", 4, &alice());
        let decision =
            decide(&install, &next, (&none(), &none()), &MigrationProbe::Passed).unwrap();
        let summary = match decision {
            UpdateDecision::ApprovalRequired(summary) => summary,
            other @ UpdateDecision::Applies(_) => {
                panic!("expected ApprovalRequired, got {other:?}")
            }
        };
        assert!(summary.breaches.is_empty());
        assert_eq!(summary.policy, InstallPolicy::Pinned);
        assert_eq!(
            updated_install_record(&install, &next).policy,
            InstallPolicy::Pinned
        );
    }

    #[test]
    fn each_out_of_envelope_variant_requires_approval_and_names_its_term() {
        let install = installed(InstallPolicy::Tracking);
        let next = release("chess", 4, &alice());

        let mut older_abi = install.clone();
        older_abi.abi = AbiDescriptor {
            family: "plaza".to_owned(),
            version: 0,
        };
        let storage = BTreeSet::from(["storage".to_owned()]);
        let empty = none();
        let cases = [
            (
                older_abi,
                (&empty, &empty),
                MigrationProbe::Passed,
                "abi_change",
            ),
            (
                install.clone(),
                (&empty, &storage),
                MigrationProbe::Passed,
                "capability_widening",
            ),
            (
                install.clone(),
                (&empty, &empty),
                MigrationProbe::Failed("guest migrate export failed".to_owned()),
                "migration_failure",
            ),
        ];
        for (install, caps, migration, term) in cases {
            let decision = decide(&install, &next, caps, &migration).unwrap();
            let summary = match decision {
                UpdateDecision::ApprovalRequired(summary) => summary,
                other @ UpdateDecision::Applies(_) => {
                    panic!("{term}: expected ApprovalRequired, got {other:?}")
                }
            };
            let terms: Vec<_> = summary.breaches.iter().map(EnvelopeBreach::term).collect();
            assert_eq!(terms, [term]);
        }

        // Narrowing capabilities is not a widening.
        let decision = decide(
            &install,
            &next,
            (&storage, &none()),
            &MigrationProbe::Passed,
        )
        .unwrap();
        assert!(matches!(decision, UpdateDecision::Applies(_)));
    }

    #[test]
    fn a_fork_or_another_name_is_the_wrong_line_and_never_an_update() {
        let install = installed(InstallPolicy::Tracking);
        let parent = release("chess", 3, &alice());
        let forked = fork(
            &parent,
            PAYLOAD.to_vec(),
            ProgramSpec {
                name: "chess",
                version: 9,
                licence: None,
                description: None,
                categories: &[],
            },
            &bob(),
        )
        .unwrap();
        let renamed = release("checkers", 9, &alice());
        for candidate in [&forked, &renamed] {
            assert!(matches!(
                decide(
                    &install,
                    candidate,
                    (&none(), &none()),
                    &MigrationProbe::Passed
                ),
                Err(UpdateRefusal::WrongLine { .. })
            ));
        }
        let same = release("chess", 3, &alice());
        let older = release("chess", 2, &alice());
        for candidate in [&same, &older] {
            assert!(matches!(
                decide(
                    &install,
                    candidate,
                    (&none(), &none()),
                    &MigrationProbe::Passed
                ),
                Err(UpdateRefusal::NotNewer { installed: 3, .. })
            ));
        }
    }

    #[test]
    fn discovery_reports_only_verified_newer_releases_of_the_line() {
        let install = installed(InstallPolicy::Pinned);
        let before = install.clone();
        let v5 = release("chess", 5, &alice()).record();
        let v4 = release("chess", 4, &alice()).record();
        let mut tampered = release("chess", 6, &alice()).record();
        tampered.manifest = tampered.manifest.replace("\"chess\"", "\"chesz\"");
        let offered = [
            v5.clone(),
            release("chess", 3, &alice()).record(),
            release("checkers", 7, &alice()).record(),
            tampered,
            v4.clone(),
            v5.clone(),
        ];
        let discovery = discover(&install, &offered);
        let versions: Vec<_> = discovery.candidates.iter().map(|c| c.version).collect();
        assert_eq!(versions, [4, 5]);
        assert_eq!(discovery.unverified.len(), 1);
        assert_eq!(discovery.unverified[0].0, 3);
        // Discovery reads only: the pinned install is unchanged.
        assert_eq!(install, before);
    }

    #[test]
    fn a_registry_record_and_the_local_file_get_the_same_decision() {
        let install = installed(InstallPolicy::Tracking);
        let next = release("chess", 4, &alice());
        let from_file = ProgramBundle::decode(&next.to_bytes()).unwrap();
        let from_registry = next.record().verify().unwrap();
        let empty = none();
        let caps = RequiredCapabilities {
            installed: &empty,
            candidate: &empty,
        };
        let file_decision = decide_update(
            &install,
            from_file.manifest(),
            caps,
            &MigrationProbe::Passed,
        )
        .unwrap();
        let registry_decision =
            decide_update(&install, &from_registry, caps, &MigrationProbe::Passed).unwrap();
        assert_eq!(file_decision, registry_decision);
    }
}
