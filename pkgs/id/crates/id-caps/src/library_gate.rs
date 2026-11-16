//! The import gate: the three subset checks of `#5956` §3 with the
//! diagnostic contract of §5 (step 3 of the §7 follow-on shape for
//! `#10027`).
//!
//! The gate consumes the resolution record produced by
//! [`crate::library_resolution::resolve_import`] and decides whether
//! the import may compile into the importing world. All three checks
//! must hold:
//!
//! ```text
//! transitive requirements ⊆ selected platform surface
//! transitive requirements ⊆ world requested set
//! discovered-used (whole import closure) ⊆ world requested set
//! ```
//!
//! A failure is a compile-time refusal of the import — never a degraded
//! library, a stubbed capability, or a warning. Every refusal carries
//! the §5 diagnostic: the code `import_requirement_unmet`, one subreason
//! per failed check, the excess set **exhaustively** (every failing
//! requirement by name, never just the first), what the import was
//! checked against, and the remedy in the same terms.
//!
//! The gate is a compile-time satisfiability decision only. It mints
//! no grants, ceilings, or consent: sensor (`loc.*`) requirements keep
//! their runtime rules in `world_caps.rs` regardless of the verdict
//! here, and compile success is never presented as user consent (§3).

use std::collections::BTreeSet;
use std::fmt;

use crate::library_resolution::{ResolutionOutcome, ResolutionRecord, ResolutionRefusal};
use crate::world_caps::ProviderRegistry;

/// The one diagnostic code every import-gate refusal carries (`#5956` §5).
pub const IMPORT_REQUIREMENT_UNMET: &str = "import_requirement_unmet";

/// Why one check of the gate refused the import (`#5956` §5 subreasons).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateSubreason {
    /// A requirement is absent from the selected platform surface.
    NotOnPlatform,
    /// A requirement (declared or discovered-used) is absent from the
    /// world's requested set.
    NotRequested,
    /// A requirement names no catalog entry.
    UnknownRequirement,
    /// A requirement's version pin disagrees with the compiling catalog.
    VersionMismatch,
    /// The closure uses a capability no closure manifest declares
    /// (discovered-used ⊄ declared).
    DeclarationMismatch,
    /// A URL import's bytes are not yet fetched/verified against the
    /// pin, and the importing world requires checked requirements.
    UnverifiedProvenance,
}

impl GateSubreason {
    /// The subreason's wire name, as printed in the §5 diagnostic.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::NotOnPlatform => "not_on_platform",
            Self::NotRequested => "not_requested",
            Self::UnknownRequirement => "unknown_requirement",
            Self::VersionMismatch => "version_mismatch",
            Self::DeclarationMismatch => "declaration_mismatch",
            Self::UnverifiedProvenance => "unverified_provenance",
        }
    }
}

/// One failing requirement inside a [`GateFailure`]: the catalog name
/// plus the per-name explanation the §5 diagnostic prints.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExcessEntry {
    /// The requirement's catalog name.
    pub name: String,
    /// Why this name failed the check, in the diagnostic's terms.
    pub detail: String,
}

/// One failed check: its subreason, the **exhaustive** excess set
/// (sorted by name), and the remedy in the same terms.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GateFailure {
    /// Which check failed.
    pub subreason: GateSubreason,
    /// Every requirement that failed this check — never just the first.
    pub excess: Vec<ExcessEntry>,
    /// What to do about it, in the diagnostic's terms.
    pub remedy: String,
}

/// The gate's decision on one resolved import.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateOutcome {
    /// All three §3 checks hold; the import may compile.
    Accepted,
    /// Resolution itself refused (cycle, unknown manifest, pin
    /// conflict); the gate does not re-decide it.
    ResolutionRefused(ResolutionRefusal),
    /// The import is refused at compile time, with one failure per
    /// failed check.
    Refused(Vec<GateFailure>),
}

/// The §5 diagnostic for a refused import: everything an author needs
/// to act on the refusal, and everything an auditor needs to re-derive
/// the verdict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportDiagnostic {
    /// The imported library's name.
    pub library_name: String,
    /// The manifest hash resolution settled on.
    pub manifest_hash: String,
    /// The importing world/app.
    pub importing_world: String,
    /// The diagnostic code — always [`IMPORT_REQUIREMENT_UNMET`].
    pub code: &'static str,
    /// The selected platform's identity, when the record carries the
    /// checked surface.
    pub platform: Option<String>,
    /// The catalog version the check ran against, when recorded.
    pub catalog_version: Option<String>,
    /// The world's requested set, sorted (what the import was checked
    /// against).
    pub world_requested: Vec<String>,
    /// One failure per failed check, in check order.
    pub failures: Vec<GateFailure>,
}

impl fmt::Display for ImportDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "import refused: library {:?} (manifest {}) into world {:?}",
            self.library_name, self.manifest_hash, self.importing_world
        )?;
        let platform = self.platform.as_deref().unwrap_or("<unspecified platform>");
        let catalog = self
            .catalog_version
            .as_deref()
            .unwrap_or("<unspecified catalog>");
        writeln!(
            f,
            "  checked against: platform {platform} (catalog {catalog}); world requested: {}",
            if self.world_requested.is_empty() {
                "<none>".to_string()
            } else {
                self.world_requested.join(", ")
            }
        )?;
        for failure in &self.failures {
            writeln!(f, "  {} / {}:", self.code, failure.subreason.as_str())?;
            for entry in &failure.excess {
                writeln!(f, "    {} — {}", entry.name, entry.detail)?;
            }
            writeln!(f, "  remedy: {}", failure.remedy)?;
        }
        Ok(())
    }
}

/// The gate's full verdict on one import: the outcome, plus the §5
/// diagnostic when the import was refused by a check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GateVerdict {
    /// The decision.
    pub outcome: GateOutcome,
    /// The diagnostic, present exactly when `outcome` is
    /// [`GateOutcome::Refused`].
    pub diagnostic: Option<ImportDiagnostic>,
}

/// The compile-path inputs the gate checks a resolution record
/// against. The platform surface and the requested set are name sets:
/// the subset checks of §3 are name-level, with version pins handled
/// against the catalog itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GateInputs {
    /// Catalog names the selected platform exposes.
    pub platform_surface: BTreeSet<String>,
    /// Catalog names the importing world requested — its ceiling
    /// request, which a library may not widen transitively (§3).
    pub world_requested: BTreeSet<String>,
    /// Catalog names the whole import closure was discovered to use
    /// (the `#5958` manifest invariant, consumed unchanged): what the
    /// code actually reaches for, as opposed to what manifests declare.
    pub discovered_used: BTreeSet<String>,
    /// The importing world's policy on declared-only imports: when
    /// true, a URL import whose bytes are not yet fetched and verified
    /// against the pin is refused as [`GateSubreason::UnverifiedProvenance`]
    /// (§4); when false, its declared requirements participate as
    /// declared-not-checked and the record says so.
    pub require_checked: bool,
}

/// Run the import gate over one resolution record.
///
/// Never fails: every import attempt leaves a verdict, and a refusal
/// names its excess exhaustively so it can be audited and acted on.
#[must_use]
pub fn check_import(
    record: &ResolutionRecord,
    catalog: &ProviderRegistry,
    inputs: &GateInputs,
) -> GateVerdict {
    if let ResolutionOutcome::Refused(refusal) = &record.outcome {
        return GateVerdict {
            outcome: GateOutcome::ResolutionRefused(refusal.clone()),
            diagnostic: None,
        };
    }

    let platform = record.checked_surface.as_ref().map(|s| s.platform.clone());
    let platform_label = platform.as_deref().unwrap_or("<unspecified platform>");
    let mut failures: Vec<GateFailure> = Vec::new();

    // Partition the transitive requirements: names the catalog does not
    // carry, and pins the compiling catalog disagrees with, are their
    // own subreasons and are not re-reported under the subset checks.
    let mut unknown: Vec<ExcessEntry> = Vec::new();
    let mut mismatched: Vec<ExcessEntry> = Vec::new();
    let mut eligible: Vec<String> = Vec::new();
    for req in &record.transitive_requirements {
        match catalog.get(&req.name) {
            None => unknown.push(ExcessEntry {
                name: req.name.clone(),
                detail: "names no entry in the catalog the world is compiled against".to_string(),
            }),
            Some(entry) => {
                if let Some(pin) = req.version_pin {
                    if pin != entry.version {
                        mismatched.push(ExcessEntry {
                            name: req.name.clone(),
                            detail: format!(
                                "pinned to catalog version {pin}, the compiling catalog carries version {}",
                                entry.version
                            ),
                        });
                        continue;
                    }
                }
                eligible.push(req.name.clone());
            }
        }
    }
    if !unknown.is_empty() {
        unknown.sort_by(|a, b| a.name.cmp(&b.name));
        let names = names_of(&unknown);
        failures.push(GateFailure {
            subreason: GateSubreason::UnknownRequirement,
            excess: unknown,
            remedy: format!(
                "fix the library manifest: {names} name no catalog entry; requirements are exact catalog names."
            ),
        });
    }
    if !mismatched.is_empty() {
        mismatched.sort_by(|a, b| a.name.cmp(&b.name));
        failures.push(GateFailure {
            subreason: GateSubreason::VersionMismatch,
            excess: mismatched,
            remedy: "pin a different library version, or build against the catalog version the library pins."
                .to_string(),
        });
    }

    // Check 1 (§3): transitive requirements ⊆ selected platform surface.
    let mut not_on_platform: Vec<ExcessEntry> = eligible
        .iter()
        .filter(|name| !inputs.platform_surface.contains(*name))
        .map(|name| ExcessEntry {
            name: name.clone(),
            detail: format!("not exposed by the selected platform ({platform_label})"),
        })
        .collect();
    not_on_platform.sort_by(|a, b| a.name.cmp(&b.name));
    if !not_on_platform.is_empty() {
        let names = names_of(&not_on_platform);
        failures.push(GateFailure {
            subreason: GateSubreason::NotOnPlatform,
            excess: not_on_platform,
            remedy: format!(
                "select a platform exposing {names}, or use a library version whose requirements the selected platform covers."
            ),
        });
    }

    // Checks 2+3 (§3): transitive requirements ⊆ world requested set,
    // and discovered-used over the whole closure ⊆ world requested set.
    // Both are the same ceiling question, so their excess merges under
    // the one subreason.
    let mut not_requested: BTreeSet<String> = BTreeSet::new();
    for name in &eligible {
        if !inputs.world_requested.contains(name) {
            not_requested.insert(name.clone());
        }
    }
    for name in &inputs.discovered_used {
        if !inputs.world_requested.contains(name) {
            not_requested.insert(name.clone());
        }
    }
    if !not_requested.is_empty() {
        let excess: Vec<ExcessEntry> = not_requested
            .iter()
            .map(|name| ExcessEntry {
                name: name.clone(),
                detail: "not in the world requested set".to_string(),
            })
            .collect();
        let names = names_of(&excess);
        failures.push(GateFailure {
            subreason: GateSubreason::NotRequested,
            excess,
            remedy: format!(
                "extend the world's requested set to include {names}, or use a library version whose requirements the world already requests."
            ),
        });
    }

    // The declaration half of check 3: discovered-used must be covered
    // by what the closure's manifests declare (#5958's invariant).
    let declared: BTreeSet<&str> = record
        .transitive_requirements
        .iter()
        .map(|r| r.name.as_str())
        .collect();
    let undeclared: Vec<ExcessEntry> = inputs
        .discovered_used
        .iter()
        .filter(|name| !declared.contains(name.as_str()))
        .map(|name| ExcessEntry {
            name: name.clone(),
            detail: "used by the import closure but declared in no closure manifest".to_string(),
        })
        .collect();
    if !undeclared.is_empty() {
        let names = names_of(&undeclared);
        failures.push(GateFailure {
            subreason: GateSubreason::DeclarationMismatch,
            excess: undeclared,
            remedy: format!("declare the missing uses in the library's manifest: {names}."),
        });
    }

    // Provenance (§4): a declared-only (unverified URL) import is
    // refused by a world whose policy requires checked requirements;
    // the refusal names the state, not a capability.
    if record.declared_only && inputs.require_checked {
        let mut excess: Vec<ExcessEntry> = record
            .transitive_requirements
            .iter()
            .map(|r| ExcessEntry {
                name: r.name.clone(),
                detail: "declared-not-checked: the URL bytes are not yet fetched and verified against the pinned hash".to_string(),
            })
            .collect();
        excess.sort_by(|a, b| a.name.cmp(&b.name));
        failures.push(GateFailure {
            subreason: GateSubreason::UnverifiedProvenance,
            excess,
            remedy: "fetch the library bytes and verify them against the pinned hash before import, or import the library from the registry."
                .to_string(),
        });
    }

    if failures.is_empty() {
        GateVerdict {
            outcome: GateOutcome::Accepted,
            diagnostic: None,
        }
    } else {
        GateVerdict {
            outcome: GateOutcome::Refused(failures.clone()),
            diagnostic: Some(ImportDiagnostic {
                library_name: record.library_name.clone(),
                manifest_hash: record.manifest_hash.clone(),
                importing_world: record.importing_world.clone(),
                code: IMPORT_REQUIREMENT_UNMET,
                platform,
                catalog_version: record
                    .checked_surface
                    .as_ref()
                    .map(|s| s.catalog_version.clone()),
                world_requested: inputs.world_requested.iter().cloned().collect(),
                failures,
            }),
        }
    }
}

fn names_of(excess: &[ExcessEntry]) -> String {
    excess
        .iter()
        .map(|e| e.name.as_str())
        .collect::<Vec<_>>()
        .join(" + ")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::library_manifest::PlatformRequirement;
    use crate::library_resolution::{CheckedSurface, ImportSource};
    use crate::world_caps::PROVIDER_REGISTRY;

    fn req(name: &str, pin: Option<u32>) -> PlatformRequirement {
        PlatformRequirement {
            name: name.to_string(),
            version_pin: pin,
        }
    }

    fn record(requirements: Vec<PlatformRequirement>) -> ResolutionRecord {
        ResolutionRecord {
            importing_world: "chess-room".to_string(),
            library_name: "greeter".to_string(),
            library_version: "1.0.0".to_string(),
            manifest_hash: "blake3:9f2".to_string(),
            source: ImportSource::Registry,
            declared_only: false,
            closure: Vec::new(),
            transitive_requirements: requirements,
            checked_surface: Some(CheckedSurface {
                platform: "roc-world/platform-no-caps".to_string(),
                catalog_version: "2026-11".to_string(),
            }),
            outcome: ResolutionOutcome::Resolved,
        }
    }

    fn inputs(platform: &[&str], requested: &[&str], discovered: &[&str]) -> GateInputs {
        GateInputs {
            platform_surface: platform.iter().map(|s| (*s).to_string()).collect(),
            world_requested: requested.iter().map(|s| (*s).to_string()).collect(),
            discovered_used: discovered.iter().map(|s| (*s).to_string()).collect(),
            require_checked: false,
        }
    }

    fn failure(verdict: &GateVerdict, subreason: GateSubreason) -> &GateFailure {
        match &verdict.outcome {
            GateOutcome::Refused(failures) => failures
                .iter()
                .find(|f| f.subreason == subreason)
                .expect("the named subreason refused the import"),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    fn excess_names(failure: &GateFailure) -> Vec<&str> {
        failure.excess.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn design_example_not_on_platform() {
        // Oracle: #5956 §5's worked example — greeter into chess-room on
        // roc-world/platform-no-caps; the diagnostic names both missing
        // requirements, exhaustively, under not_on_platform.
        let rec = record(vec![req("RocCap", None), req("records.read", None)]);
        let verdict = check_import(
            &rec,
            &PROVIDER_REGISTRY,
            &inputs(&[], &["RocCap", "records.read"], &[]),
        );
        let f = failure(&verdict, GateSubreason::NotOnPlatform);
        assert_eq!(excess_names(f), vec!["RocCap", "records.read"]);
        let text = verdict.diagnostic.unwrap().to_string();
        assert!(
            text.contains("import_requirement_unmet / not_on_platform:"),
            "{text}"
        );
        assert!(
            text.contains(
                "RocCap — not exposed by the selected platform (roc-world/platform-no-caps)"
            ),
            "{text}"
        );
        assert!(
            text.contains("remedy: select a platform exposing RocCap + records.read"),
            "{text}"
        );
    }

    #[test]
    fn pure_library_is_accepted_anywhere() {
        // Oracle: #5994 §4 pure positive control — an empty requirement
        // set passes every profile.
        let rec = record(Vec::new());
        let verdict = check_import(&rec, &PROVIDER_REGISTRY, &inputs(&[], &[], &[]));
        assert_eq!(verdict.outcome, GateOutcome::Accepted);
        assert!(verdict.diagnostic.is_none());
    }

    #[test]
    fn covered_requirements_are_accepted() {
        let rec = record(vec![req("records.read", None), req("time.now", Some(1))]);
        let verdict = check_import(
            &rec,
            &PROVIDER_REGISTRY,
            &inputs(
                &["records.read", "time.now"],
                &["records.read", "time.now"],
                &["records.read"],
            ),
        );
        assert_eq!(verdict.outcome, GateOutcome::Accepted);
    }

    #[test]
    fn not_requested_names_the_world_ceiling_excess() {
        // Oracle: §3 check 2 — a world does not acquire a tier
        // transitively through a library it did not request.
        let rec = record(vec![req("records.read", None), req("chat.say", None)]);
        let verdict = check_import(
            &rec,
            &PROVIDER_REGISTRY,
            &inputs(&["records.read", "chat.say"], &["records.read"], &[]),
        );
        let f = failure(&verdict, GateSubreason::NotRequested);
        assert_eq!(excess_names(f), vec!["chat.say"]);
    }

    #[test]
    fn unknown_requirement_is_its_own_subreason() {
        let rec = record(vec![req("fs.read", None)]);
        let verdict = check_import(
            &rec,
            &PROVIDER_REGISTRY,
            &inputs(&["fs.read"], &["fs.read"], &[]),
        );
        let f = failure(&verdict, GateSubreason::UnknownRequirement);
        assert_eq!(excess_names(f), vec!["fs.read"]);
    }

    #[test]
    fn version_mismatch_names_pin_and_catalog() {
        let rec = record(vec![req("time.now", Some(99))]);
        let verdict = check_import(
            &rec,
            &PROVIDER_REGISTRY,
            &inputs(&["time.now"], &["time.now"], &[]),
        );
        let f = failure(&verdict, GateSubreason::VersionMismatch);
        assert_eq!(excess_names(f), vec!["time.now"]);
        assert!(f.excess[0].detail.contains("99"), "{}", f.excess[0].detail);
    }

    #[test]
    fn discovered_undeclared_use_is_a_declaration_mismatch() {
        // Oracle: §3 check 3 / §5 declaration_mismatch — discovered-used
        // beyond the declared set refuses, naming the undeclared use.
        let rec = record(vec![req("records.read", None)]);
        let verdict = check_import(
            &rec,
            &PROVIDER_REGISTRY,
            &inputs(
                &["records.read", "time.now"],
                &["records.read", "time.now"],
                &["records.read", "time.now"],
            ),
        );
        let f = failure(&verdict, GateSubreason::DeclarationMismatch);
        assert_eq!(excess_names(f), vec!["time.now"]);
    }

    #[test]
    fn discovered_use_widens_not_requested() {
        // Oracle: §3 check 3 — discovered-used ⊆ requested is the same
        // ceiling as check 2, reported under not_requested.
        let rec = record(vec![req("records.read", None)]);
        let verdict = check_import(
            &rec,
            &PROVIDER_REGISTRY,
            &inputs(
                &["records.read", "chat.say"],
                &["records.read"],
                &["records.read", "chat.say"],
            ),
        );
        let f = failure(&verdict, GateSubreason::NotRequested);
        assert_eq!(excess_names(f), vec!["chat.say"]);
    }

    #[test]
    fn declared_only_import_refused_only_when_checked_required() {
        // Oracle: §4 — declared-not-checked participates unless the
        // world's policy requires checked requirements.
        let mut rec = record(vec![req("records.read", None)]);
        rec.source = ImportSource::Url {
            url: "https://example.invalid/greeter".to_string(),
        };
        rec.declared_only = true;
        let mut gate = inputs(&["records.read"], &["records.read"], &[]);
        gate.require_checked = true;
        let verdict = check_import(&rec, &PROVIDER_REGISTRY, &gate);
        let f = failure(&verdict, GateSubreason::UnverifiedProvenance);
        assert_eq!(excess_names(f), vec!["records.read"]);

        gate.require_checked = false;
        let verdict = check_import(&rec, &PROVIDER_REGISTRY, &gate);
        assert_eq!(verdict.outcome, GateOutcome::Accepted);
    }

    #[test]
    fn resolution_refusal_passes_through() {
        let mut rec = record(Vec::new());
        rec.outcome = ResolutionOutcome::Refused(ResolutionRefusal::Cycle {
            cycle: vec!["greeter (blake3:9f2)".to_string()],
        });
        let verdict = check_import(&rec, &PROVIDER_REGISTRY, &inputs(&[], &[], &[]));
        assert!(matches!(
            verdict.outcome,
            GateOutcome::ResolutionRefused(ResolutionRefusal::Cycle { .. })
        ));
        assert!(verdict.diagnostic.is_none());
    }

    #[test]
    fn excess_is_exhaustive_and_sorted() {
        // Oracle: §5 — every failing requirement by name, never just
        // the first, never a count.
        let rec = record(vec![
            req("time.now", None),
            req("chat.say", None),
            req("records.read", None),
        ]);
        let verdict = check_import(
            &rec,
            &PROVIDER_REGISTRY,
            &inputs(&[], &["time.now", "chat.say", "records.read"], &[]),
        );
        let f = failure(&verdict, GateSubreason::NotOnPlatform);
        assert_eq!(
            excess_names(f),
            vec!["chat.say", "records.read", "time.now"]
        );
    }
}
