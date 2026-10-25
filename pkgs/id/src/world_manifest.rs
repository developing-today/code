//! `world.toml`: a world described as data.
//!
//! A manifest names the module (by hash or source), the seed, the granted
//! capability set, the capability ceiling, the requested execution profile and
//! the pending schedules. `id world apply` reconciles a live world to the
//! manifest through the existing admin operations, `--dry-run` prints the plan
//! without applying it, and `id world diff` shows drift between the two.
//!
//! Everything here is pure: [`plan`] turns a [`WorldManifest`] and a current
//! [`WorldState`] into a list of [`ApplyStep`]s, [`apply`] folds the steps into
//! the state, and [`diff`] renders the steps as human-readable drift. The
//! server-side command wiring executes the steps one by one, journaling each.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use crate::world_caps::{MAX_NAME_BYTES, validate_ceiling_names, validate_grant_names};
use crate::world_limits::WorldExecutionProfile;

/// A declarative description of a world. Empty fields are left alone by
/// [`plan`], so a minimal manifest drifts nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct WorldManifest {
    /// A content hash of an already-installed module (`blake3:<hex>`). Mutually
    /// exclusive with `module_source`.
    pub module_hash: Option<String>,
    /// A source file or directory to compile on the host before install.
    /// Mutually exclusive with `module_hash`.
    pub module_source: Option<PathBuf>,
    /// The deterministic seed for the program's initializer.
    pub seed: Option<u64>,
    /// Capabilities to grant. `*` grants every capability the ceiling allows.
    pub grants: Vec<String>,
    /// The capability ceiling. `None` means the whole catalog; an empty list
    /// means no capability is allowed.
    pub ceiling: Option<Vec<String>>,
    /// Requested execution profile bounds. Omitted fields inherit the server.
    pub profile: WorldExecutionProfile,
    /// Pending schedules: id to fire time, in unix milliseconds.
    pub schedules: BTreeMap<String, u64>,
}

/// The relevant current state of a live world, as `id world stats` would see
/// it. This is the projection a manifest is compared against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldState {
    /// The active module's content hash, if a program is installed.
    pub module_hash: Option<String>,
    /// The seed the active program was installed with.
    pub seed: u64,
    /// The granted capability set (raw, `*` allowed).
    pub grants: Vec<String>,
    /// The capability ceiling (`None` = whole catalog, empty = none).
    pub ceiling: Option<Vec<String>>,
    /// The requested execution profile.
    pub profile: WorldExecutionProfile,
    /// Pending schedules: id to fire time, in unix milliseconds.
    pub schedules: BTreeMap<String, u64>,
}

impl Default for WorldState {
    fn default() -> Self {
        Self {
            module_hash: None,
            seed: 0,
            grants: Vec::new(),
            ceiling: None,
            profile: WorldExecutionProfile::default(),
            schedules: BTreeMap::new(),
        }
    }
}

/// One reconciliation step, in the order a server applies it. Module install
/// comes first because a changed seed or profile rides on the next install.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApplyStep {
    /// Install a different module: a known hash to activate, or a source path
    /// to compile and install.
    InstallModule {
        /// Desired content hash, if the module is already on the host.
        hash: Option<String>,
        /// Source path to compile, if the module is not yet installed.
        source: Option<PathBuf>,
        /// The seed to initialize the program with.
        seed: u64,
    },
    /// Replace the granted capability set.
    SetGrants {
        /// Desired grants.
        grants: Vec<String>,
    },
    /// Replace the capability ceiling.
    SetCeiling {
        /// Desired ceiling (`None` = whole catalog, empty = none).
        ceiling: Option<Vec<String>>,
    },
    /// Replace the requested execution profile.
    SetProfile {
        /// The requested bounds.
        profile: WorldExecutionProfile,
    },
    /// Schedule a fire.
    ScheduleAt {
        /// The schedule's id.
        id: String,
        /// When it fires, in unix milliseconds.
        at_ms: u64,
    },
    /// Cancel a schedule the manifest does not declare.
    CancelSchedule {
        /// The schedule's id.
        id: String,
    },
}

impl ApplyStep {
    /// A stable, human-readable name for the step.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::InstallModule { .. } => "install_module",
            Self::SetGrants { .. } => "set_grants",
            Self::SetCeiling { .. } => "set_ceiling",
            Self::SetProfile { .. } => "set_profile",
            Self::ScheduleAt { .. } => "schedule_at",
            Self::CancelSchedule { .. } => "cancel_schedule",
        }
    }
}

/// Parse a `world.toml` document.
///
/// # Errors
///
/// Invalid TOML, or fields that name an unknown capability.
pub fn parse(text: &str) -> Result<WorldManifest> {
    let manifest: WorldManifest = toml::from_str(text).map_err(anyhow::Error::new)?;
    manifest.validate()?;
    Ok(manifest)
}

impl WorldManifest {
    /// Validate the manifest: grants and ceiling name only known capabilities,
    /// schedule ids are bounded printable ASCII, and fire times are positive.
    ///
    /// # Errors
    ///
    /// Names the first offending field.
    pub fn validate(&self) -> Result<()> {
        validate_grant_names(&self.grants)?;
        if let Some(ceiling) = &self.ceiling {
            validate_ceiling_names(ceiling)?;
        }
        for (id, at_ms) in &self.schedules {
            crate::world_caps::validate_schedule_id(id)?;
            ensure!(*at_ms > 0, "schedule {id:?} must fire in the future");
        }
        ensure!(
            self.module_hash.is_none() || self.module_source.is_none(),
            "module_hash and module_source are mutually exclusive"
        );
        if let Some(hash) = &self.module_hash {
            let valid = hash.len() <= MAX_NAME_BYTES && hash.bytes().all(|b| b.is_ascii_hexdigit());
            ensure!(
                valid,
                "module hash must be 1 to {MAX_NAME_BYTES} hex digits"
            );
        }
        Ok(())
    }

    /// Serialize the manifest back to TOML.
    ///
    /// # Errors
    ///
    /// Only if the manifest cannot be represented (it always can).
    pub fn to_toml(&self) -> Result<String> {
        toml::to_string_pretty(self).map_err(anyhow::Error::new)
    }
}

/// Compute the steps that reconcile `current` to `desired`. When `current`
/// already matches, the plan is empty, so applying twice is a no-op.
#[must_use]
pub fn plan(desired: &WorldManifest, current: &WorldState) -> Vec<ApplyStep> {
    let mut steps = Vec::new();

    // Module and seed: a manifest hash equal to the live hash means no install.
    let module_changed = match (&desired.module_hash, &desired.module_source) {
        (Some(hash), _) => current.module_hash.as_deref() != Some(hash),
        (None, Some(_)) => true, // a source always recompiles
        (None, None) => false,
    };
    if module_changed {
        steps.push(ApplyStep::InstallModule {
            hash: desired.module_hash.clone(),
            source: desired.module_source.clone(),
            seed: desired.seed.unwrap_or(current.seed),
        });
    }

    // Grants: replace when the set differs.
    let desired_grants: std::collections::BTreeSet<String> =
        desired.grants.iter().cloned().collect();
    let current_grants: std::collections::BTreeSet<String> =
        current.grants.iter().cloned().collect();
    if desired_grants != current_grants {
        steps.push(ApplyStep::SetGrants {
            grants: desired.grants.clone(),
        });
    }

    // Ceiling: replace when it differs.
    if desired.ceiling != current.ceiling {
        steps.push(ApplyStep::SetCeiling {
            ceiling: desired.ceiling.clone(),
        });
    }

    // Profile: replace when it differs.
    if desired.profile != current.profile {
        steps.push(ApplyStep::SetProfile {
            profile: desired.profile,
        });
    }

    // Schedules: add missing, cancel extra.
    for (id, at_ms) in &desired.schedules {
        if current.schedules.get(id) != Some(at_ms) {
            steps.push(ApplyStep::ScheduleAt {
                id: id.clone(),
                at_ms: *at_ms,
            });
        }
    }
    for id in current.schedules.keys() {
        if !desired.schedules.contains_key(id) {
            steps.push(ApplyStep::CancelSchedule { id: id.clone() });
        }
    }

    steps
}

/// Apply `steps` to `state`, returning the number of steps that changed it.
///
/// # Errors
///
/// A step that names an unknown capability (a manifest should be validated
/// first; this is a defensive re-check).
pub fn apply(steps: &[ApplyStep], state: &mut WorldState) -> Result<usize> {
    let mut changed = 0;
    for step in steps {
        match step {
            ApplyStep::InstallModule { hash, source, seed } => {
                if let Some(hash) = hash {
                    state.module_hash = Some(hash.clone());
                }
                state.seed = *seed;
                let _ = source;
                changed += 1;
            }
            ApplyStep::SetGrants { grants } => {
                validate_grant_names(grants)?;
                if state.grants != *grants {
                    state.grants = grants.clone();
                    changed += 1;
                }
            }
            ApplyStep::SetCeiling { ceiling } => {
                if let Some(names) = ceiling {
                    validate_ceiling_names(names)?;
                }
                if state.ceiling != *ceiling {
                    state.ceiling = ceiling.clone();
                    changed += 1;
                }
            }
            ApplyStep::SetProfile { profile } => {
                if state.profile != *profile {
                    state.profile = *profile;
                    changed += 1;
                }
            }
            ApplyStep::ScheduleAt { id, at_ms } => {
                crate::world_caps::validate_schedule_id(id)?;
                if state.schedules.get(id) != Some(at_ms) {
                    state.schedules.insert(id.clone(), *at_ms);
                    changed += 1;
                }
            }
            ApplyStep::CancelSchedule { id } => {
                if state.schedules.remove(id).is_some() {
                    changed += 1;
                }
            }
        }
    }
    Ok(changed)
}

/// Render the steps as `id world diff` drift lines.
#[must_use]
pub fn diff(desired: &WorldManifest, current: &WorldState) -> String {
    let steps = plan(desired, current);
    if steps.is_empty() {
        return "no drift".to_owned();
    }
    let mut out = String::new();
    for step in steps {
        match step {
            ApplyStep::InstallModule { hash, source, seed } => {
                if let Some(hash) = hash {
                    out.push_str(&format!("- module {hash} (seed {seed})\n"));
                }
                if let Some(source) = source {
                    out.push_str(&format!(
                        "+ module source {} (seed {seed})\n",
                        source.display()
                    ));
                }
            }
            ApplyStep::SetGrants { grants } => {
                out.push_str(&format!("~ grants [{}]\n", grants.join(", ")))
            }
            ApplyStep::SetCeiling { ceiling } => {
                let names = ceiling.as_deref().unwrap_or(&[]).join(", ");
                out.push_str(&format!("~ ceiling [{names}]\n"));
            }
            ApplyStep::SetProfile { profile } => {
                out.push_str(&format!("~ profile {profile:?}\n"));
            }
            ApplyStep::ScheduleAt { id, at_ms } => {
                out.push_str(&format!("+ schedule {id} at {at_ms}\n"));
            }
            ApplyStep::CancelSchedule { id } => {
                out.push_str(&format!("- schedule {id}\n"));
            }
        }
    }
    out
}

/// Round-trip a manifest through TOML and back.
///
/// # Errors
///
/// Serialization or parsing failures.
pub fn round_trip(manifest: &WorldManifest) -> Result<WorldManifest> {
    parse(&manifest.to_toml()?)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn manifest_toml() -> &'static str {
        r#"
module_hash = "abc123"
seed = 42
grants = ["chat.say", "time.now"]
ceiling = ["chat.say", "time.now", "records.read"]
schedules = { "round" = 1800000000000 }

[profile]
wasm_fuel = 5_000_000
"#
    }

    #[test]
    fn parses_and_round_trips() {
        let manifest = parse(manifest_toml()).unwrap();
        assert_eq!(manifest.module_hash, Some("abc123".to_owned()));
        assert_eq!(manifest.seed, Some(42));
        assert_eq!(manifest.grants, vec!["chat.say", "time.now"]);
        assert_eq!(
            manifest.ceiling,
            Some(vec![
                "chat.say".to_owned(),
                "time.now".to_owned(),
                "records.read".to_owned()
            ])
        );
        assert_eq!(manifest.profile.wasm_fuel, Some(5_000_000));
        assert_eq!(manifest.schedules.get("round"), Some(&1_800_000_000_000));
        assert_eq!(round_trip(&manifest).unwrap(), manifest);
    }

    #[test]
    fn an_empty_manifest_drifts_nothing() {
        let manifest = WorldManifest::default();
        // A default (empty) manifest leaves a default (empty) world alone.
        assert!(plan(&manifest, &WorldState::default()).is_empty());
        // And a partially-populated world only drifts where it differs from the
        // empty manifest: the grants and ceiling are removed.
        let state = WorldState {
            module_hash: Some("abc".to_owned()),
            seed: 7,
            grants: vec!["chat.say".to_owned()],
            ceiling: Some(vec!["chat.say".to_owned()]),
            ..WorldState::default()
        };
        let steps = plan(&manifest, &state);
        assert_eq!(steps.len(), 2, "{steps:?}");
        assert!(steps.iter().all(|s| matches!(
            s,
            ApplyStep::SetGrants { .. } | ApplyStep::SetCeiling { .. }
        )));
    }

    #[test]
    fn apply_is_idempotent() {
        let manifest = parse(manifest_toml()).unwrap();
        let mut state = WorldState::default();
        let steps = plan(&manifest, &state);
        assert!(!steps.is_empty());
        let changed = apply(&steps, &mut state).unwrap();
        assert_eq!(changed, steps.len());
        // A second plan against the reconciled state is empty.
        assert!(plan(&manifest, &state).is_empty());
    }

    #[test]
    fn drift_is_detected_after_a_manual_change() {
        let manifest = parse(manifest_toml()).unwrap();
        let mut state = WorldState::default();
        apply(&plan(&manifest, &state), &mut state).unwrap();
        // Simulate an admin narrowing the ceiling by hand.
        state.ceiling = Some(vec!["chat.say".to_owned()]);
        let steps = plan(&manifest, &state);
        assert!(
            steps
                .iter()
                .any(|step| matches!(step, ApplyStep::SetCeiling { ceiling } if ceiling == &manifest.ceiling)),
            "drift was not detected: {steps:?}"
        );
    }

    #[test]
    fn schedules_are_added_and_removed() {
        let manifest = WorldManifest {
            schedules: BTreeMap::from([("keep".to_owned(), 5), ("new".to_owned(), 6)]),
            ..WorldManifest::default()
        };
        let state = WorldState {
            schedules: BTreeMap::from([("keep".to_owned(), 5), ("stale".to_owned(), 4)]),
            ..WorldState::default()
        };
        let steps = plan(&manifest, &state);
        assert!(steps.iter().any(
            |s| matches!(s, ApplyStep::ScheduleAt { id, at_ms } if id == "new" && *at_ms == 6)
        ));
        assert!(
            steps
                .iter()
                .any(|s| matches!(s, ApplyStep::CancelSchedule { id } if id == "stale"))
        );
        assert!(
            !steps
                .iter()
                .any(|s| matches!(s, ApplyStep::ScheduleAt { id, .. } | ApplyStep::CancelSchedule { id } if id == "keep"))
        );
    }

    #[test]
    fn validation_rejects_unknown_names_and_bad_hashes() {
        assert!(parse("grants = [\"time.nwo\"]").is_err());
        assert!(parse("ceiling = [\"*\"]").is_err());
        assert!(parse("ceiling = [\"time.nwo\"]").is_err());
        assert!(parse("module_hash = \"not-hex!\"").is_err());
        assert!(parse("module_hash = \"abc\"\nmodule_source = \"main.roc\"").is_err());
        assert!(parse("schedules = { \"bad\nid\" = 1 }").is_err());
        assert!(parse("schedules = { \"a\" = 0 }").is_err());
    }

    #[test]
    fn a_source_module_always_recompiles() {
        let manifest = WorldManifest {
            module_source: Some(PathBuf::from("main.roc")),
            ..WorldManifest::default()
        };
        let state = WorldState {
            module_hash: Some("abc".to_owned()),
            ..WorldState::default()
        };
        let steps = plan(&manifest, &state);
        assert!(steps.iter().any(|s| matches!(
            s,
            ApplyStep::InstallModule {
                source: Some(_),
                ..
            }
        )));
    }

    #[test]
    fn diff_renders_steps_and_empty_for_no_drift() {
        let manifest = parse(manifest_toml()).unwrap();
        let state = WorldState::default();
        let d = diff(&manifest, &state);
        assert!(d.contains("grants"), "{d}");
        assert!(d.contains("ceiling"), "{d}");
        assert!(d.contains("schedule"), "{d}");

        // After applying, the world matches the manifest, so diff is empty.
        let mut reconciled = WorldState::default();
        apply(&plan(&manifest, &reconciled), &mut reconciled).unwrap();
        assert_eq!(diff(&manifest, &reconciled), "no drift");
        // And diffing an empty manifest against the default state is empty too.
        assert_eq!(
            diff(&WorldManifest::default(), &WorldState::default()),
            "no drift"
        );
    }
}
